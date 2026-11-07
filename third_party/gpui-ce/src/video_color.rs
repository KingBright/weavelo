//! Shared GPU YUV interpretation; never reads image pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Declared SDR primaries supported by the Linux native compositor.
pub enum SdrPrimaries {
    /// BT.709/sRGB primaries, already in the compositor's signal RGB space.
    Bt709,
    /// H.273 primaries 5, with the BT.709-equivalent transfer 1 or 6.
    Bt470Bg,
}
impl SdrPrimaries {
    /// No inferred metadata, HDR reduction, or unsupported gamut substitution.
    pub fn from_cicp(primaries: i32, transfer: i32) -> Option<Self> {
        match (primaries, transfer) {
            (1, 1 | 13) => Some(Self::Bt709),
            (5, 1 | 6) => Some(Self::Bt470Bg),
            _ => None,
        }
    }
}
#[cfg(test)]
mod sdr_tests {
    use super::SdrPrimaries;
    #[test]
    fn accepts_declared_android_sdr_without_guessing_hdr_or_unknown_color() {
        assert_eq!(SdrPrimaries::from_cicp(5, 6), Some(SdrPrimaries::Bt470Bg));
        assert_eq!(SdrPrimaries::from_cicp(1, 13), Some(SdrPrimaries::Bt709));
        assert_eq!(SdrPrimaries::from_cicp(1, 1), Some(SdrPrimaries::Bt709));
        for (primaries, transfer) in [(2, 2), (5, 2), (5, 16), (9, 16), (9, 18), (12, 13)] {
            assert_eq!(SdrPrimaries::from_cicp(primaries, transfer), None);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// GPU plane representation accepted by the native-video adapters.
pub enum VideoFormat {
    /// 8-bit luma with an interleaved 4:2:0 chroma plane.
    Nv12,
    /// 10-bit 4:2:0 samples stored in the high bits of 16-bit components.
    P010,
    /// Opaque 8-bit blue/green/red/alpha resource; no YUV conversion.
    Bgra8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// SDR YCbCr-to-RGB matrix identified by the producer.
pub enum Matrix {
    /// ITU-R BT.601 luma/chroma coefficients.
    Bt601,
    /// ITU-R BT.709 luma/chroma coefficients.
    Bt709,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Quantization range of the coded signal, not its memory type.
pub enum Range {
    /// Video-range values, including headroom/footroom outside nominal limits.
    Limited,
    /// Full available signal range at the declared bit depth.
    Full,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Horizontal location of subsampled chroma relative to luma.
pub enum ChromaLocation {
    /// Chroma is centered between its corresponding luma samples.
    Center,
    /// Chroma is aligned with the left corresponding luma sample.
    Left,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Explicit SDR signal interpretation; does not imply HDR tone mapping.
pub struct VideoColor {
    /// Producer-declared YCbCr matrix.
    pub matrix: Matrix,
    /// Producer-declared code-value range.
    pub range: Range,
    /// Chroma sampling position used by the adapter.
    pub chroma: ChromaLocation,
}
impl VideoColor {
    /// Reads the decoded native NV12 buffer's actual matrix and range.
    /// Missing or unsupported metadata fails without assuming a matrix.
    #[cfg(target_os = "macos")]
    pub fn from_macos_pixel_buffer(buffer: &core_video::pixel_buffer::CVPixelBuffer) -> Result<Self, &'static str> {
        use core_foundation::{base::TCFType, string::CFString};
        use core_video::{buffer::TCVBuffer, image_buffer::*, pixel_buffer::*};
        let range = match buffer.get_pixel_format() {
            kCVPixelFormatType_420YpCbCr8BiPlanarFullRange => Range::Full,
            kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange => Range::Limited,
            _ => return Err("Native video surface requires an NV12 pixel buffer"),
        };
        let key = unsafe { CFString::wrap_under_get_rule(kCVImageBufferYCbCrMatrixKey) };
        let matrix = buffer.as_buffer().get_attachment(&key, None)
            .and_then(|value| value.downcast::<CFString>())
            .ok_or("Native video surface has no YCbCr matrix attachment")?;
        let matrix = unsafe {
            if matrix == CFString::wrap_under_get_rule(kCVImageBufferYCbCrMatrix_ITU_R_709_2) {
                Matrix::Bt709
            } else if matrix == CFString::wrap_under_get_rule(kCVImageBufferYCbCrMatrix_ITU_R_601_4)
                // VideoToolbox preserves CICP 5 from Android HEVC as this
                // numeric attachment. H.273 table 4 gives 5 and 6 the same
                // explicit BT.601 coefficients; neither is unspecified.
                || matrix == CFString::from_static_string("YCbCrMatrix#5")
                || matrix == CFString::from_static_string("YCbCrMatrix#6") {
                Matrix::Bt601
            } else {
                return Err("Native video surface has an unsupported YCbCr matrix");
            }
        };
        // The renderer samples the existing centered chroma plane. Matrix and
        // range come from this decoded buffer, which VT may have range-converted.
        Ok(Self { matrix, range, chroma: ChromaLocation::Center })
    }

    /// Converts normalized R8/RG8 or high-bit-aligned R16/RG16 samples to signal
    /// RGB. No HDR tone mapping or gamut conversion is implied by this matrix.
    pub fn rows(self, format: VideoFormat) -> [[f32; 4]; 3] {
        if format == VideoFormat::Bgra8 {
            return [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]];
        }
        let (max, shift, bits) = if format == VideoFormat::P010 {
            (65535f64, 64f64, 4f64)
        } else {
            (255., 1., 1.)
        };
        let (ys, yoff, cs, coff) = match self.range {
            Range::Limited => (
                max / (219. * bits * shift),
                16. * bits * shift / max,
                max / (224. * bits * shift),
                128. * bits * shift / max,
            ),
            Range::Full => (
                max / ((256. * bits - 1.) * shift),
                0.,
                max / ((256. * bits - 1.) * shift),
                128. * bits * shift / max,
            ),
        };
        let (kr, kb) = match self.matrix {
            Matrix::Bt601 => (0.299, 0.114),
            Matrix::Bt709 => (0.2126, 0.0722),
        };
        let kg = 1. - kr - kb;
        let coefficients = [
            [ys, 0., 2. * (1. - kr) * cs],
            [
                ys,
                -2. * kb * (1. - kb) / kg * cs,
                -2. * kr * (1. - kr) / kg * cs,
            ],
            [ys, 2. * (1. - kb) * cs, 0.],
        ];
        coefficients.map(|r| {
            [
                r[0] as f32,
                r[1] as f32,
                r[2] as f32,
                (-r[0] * yoff - (r[1] + r[2]) * coff) as f32,
            ]
        })
    }
}

