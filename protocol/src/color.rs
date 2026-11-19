//! Video color profile definitions (YUV 4:4:4 true color, 10-bit depth, and HDR10/HLG metadata).
//!
//! All wire types use integer representations for mastering metadata to guarantee
//! deterministic `bincode`/JSON roundtrips without floating-point drift or NaN hazards.

use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

/// Maximum luminance representable by SMPTE ST 2084 (PQ) in nits (cd/m²).
pub const MAX_HDR_LUMINANCE_NITS: u32 = 10_000;

/// Scale factor for `HdrMasteringMetadata::min_mastering_nits_0001` (0.0001 cd/m² per unit).
pub const MIN_MASTERING_LUMINANCE_SCALE: u32 = 10_000;

/// Chroma subsampling layout of the encoded video stream.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ChromaSubsampling {
    /// 4:2:0 chroma subsampling (half horizontal and vertical chroma resolution).
    #[default]
    Yuv420,
    /// 4:4:4 full-resolution chroma (true color; sharp subpixel text and UI edges).
    Yuv444,
}

impl ChromaSubsampling {
    pub const fn is_444(self) -> bool {
        matches!(self, Self::Yuv444)
    }

    /// Horizontal and vertical chroma subsampling divisors `(h_div, v_div)`.
    pub const fn chroma_divisors(self) -> (u32, u32) {
        match self {
            Self::Yuv420 => (2, 2),
            Self::Yuv444 => (1, 1),
        }
    }
}

impl fmt::Display for ChromaSubsampling {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Yuv420 => f.write_str("4:2:0"),
            Self::Yuv444 => f.write_str("4:4:4"),
        }
    }
}

/// Color component bit depth per channel.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ColorBitDepth {
    #[default]
    Bit8,
    Bit10,
}

impl ColorBitDepth {
    pub const fn bits_per_channel(self) -> u8 {
        match self {
            Self::Bit8 => 8,
            Self::Bit10 => 10,
        }
    }

    pub const fn is_10bit(self) -> bool {
        matches!(self, Self::Bit10)
    }
}

impl fmt::Display for ColorBitDepth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bit8 => f.write_str("8-bit"),
            Self::Bit10 => f.write_str("10-bit"),
        }
    }
}

/// Electro-optical transfer function (EOTF/OETF) for the video stream.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum HdrTransfer {
    /// Standard Dynamic Range (ITU-R BT.709 / sRGB).
    #[default]
    Sdr,
    /// Perceptual Quantizer (SMPTE ST 2084 / HDR10).
    PqSt2084,
    /// Hybrid Log-Gamma (ARIB STD-B67 / ITU-R BT.2100 HLG).
    Hlg,
}

impl HdrTransfer {
    pub const fn is_hdr(self) -> bool {
        matches!(self, Self::PqSt2084 | Self::Hlg)
    }

    /// ITU-T H.273 / CICP transfer characteristics code point.
    pub const fn cicp_transfer_characteristics(self) -> u8 {
        match self {
            Self::Sdr => 1,      // BT.709
            Self::PqSt2084 => 16, // SMPTE ST 2084
            Self::Hlg => 18,     // ARIB STD-B67
        }
    }
}

impl fmt::Display for HdrTransfer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sdr => f.write_str("SDR"),
            Self::PqSt2084 => f.write_str("PQ (ST 2084)"),
            Self::Hlg => f.write_str("HLG"),
        }
    }
}

/// Color gamut primaries of the video stream.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ColorPrimaries {
    /// ITU-R BT.709 / sRGB color gamut.
    #[default]
    Bt709,
    /// ITU-R BT.2020 wide color gamut (required for HDR10/HLG).
    Bt2020,
}

impl ColorPrimaries {
    /// ITU-T H.273 / CICP color primaries code point.
    pub const fn cicp_color_primaries(self) -> u8 {
        match self {
            Self::Bt709 => 1,
            Self::Bt2020 => 9,
        }
    }

    /// ITU-T H.273 / CICP YCbCr matrix coefficients code point (non-constant luminance).
    pub const fn cicp_matrix_coefficients(self) -> u8 {
        match self {
            Self::Bt709 => 1,
            Self::Bt2020 => 9,
        }
    }
}

impl fmt::Display for ColorPrimaries {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bt709 => f.write_str("BT.709"),
            Self::Bt2020 => f.write_str("BT.2020"),
        }
    }
}

/// Static HDR mastering display and content light level metadata (SMPTE ST 2086 + CTA-861.3).
///
/// Luminance values use integer units to keep serialization exact:
/// - `max_cll`, `max_fall`, and `max_mastering_nits` are in nits (cd/m²).
/// - `min_mastering_nits_0001` is in units of 0.0001 cd/m² (e.g. `50` = `0.005` nits).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HdrMasteringMetadata {
    /// Maximum Content Light Level in nits (0 = unspecified per CTA-861.3).
    pub max_cll: u16,
    /// Maximum Frame-Average Light Level in nits (0 = unspecified per CTA-861.3).
    pub max_fall: u16,
    /// Minimum mastering display luminance in 0.0001 cd/m² units (SMPTE ST 2086).
    pub min_mastering_nits_0001: u32,
    /// Maximum mastering display luminance in cd/m² (nits).
    pub max_mastering_nits: u32,
}

impl HdrMasteringMetadata {
    /// Conventional HDR10 mastering defaults (1000 nit peak, 0.005 nit floor, 400 nit MaxFALL).
    pub const fn hdr10_default() -> Self {
        Self {
            max_cll: 1000,
            max_fall: 400,
            min_mastering_nits_0001: 50, // 0.005 nits
            max_mastering_nits: 1000,
        }
    }

    /// Construct metadata from floating-point minimum nits (rounded to 0.0001 cd/m²).
    pub fn from_nits(
        max_cll: u16,
        max_fall: u16,
        min_mastering_nits: f32,
        max_mastering_nits: u32,
    ) -> Result<Self, ColorProfileError> {
        if !min_mastering_nits.is_finite() || min_mastering_nits < 0.0 {
            return Err(ColorProfileError::InvalidMasteringLuminance {
                min_0001: u32::MAX,
                max_nits: max_mastering_nits,
            });
        }
        let scaled = (min_mastering_nits * MIN_MASTERING_LUMINANCE_SCALE as f32).round();
        if scaled > (MAX_HDR_LUMINANCE_NITS as f32 * MIN_MASTERING_LUMINANCE_SCALE as f32) {
            return Err(ColorProfileError::LuminanceOutOfRange {
                field: "min_mastering_nits_0001",
                value: u32::MAX,
                max: MAX_HDR_LUMINANCE_NITS * MIN_MASTERING_LUMINANCE_SCALE,
            });
        }
        let meta = Self {
            max_cll,
            max_fall,
            min_mastering_nits_0001: scaled as u32,
            max_mastering_nits,
        };
        meta.validate()?;
        Ok(meta)
    }

    /// Minimum mastering luminance in nits (cd/m²).
    pub fn min_mastering_nits(&self) -> f32 {
        self.min_mastering_nits_0001 as f32 / MIN_MASTERING_LUMINANCE_SCALE as f32
    }

    /// Maximum mastering luminance in nits (cd/m²).
    pub const fn max_mastering_nits(&self) -> u32 {
        self.max_mastering_nits
    }

    pub fn validate(&self) -> Result<(), ColorProfileError> {
        if u32::from(self.max_cll) > MAX_HDR_LUMINANCE_NITS {
            return Err(ColorProfileError::LuminanceOutOfRange {
                field: "max_cll",
                value: u32::from(self.max_cll),
                max: MAX_HDR_LUMINANCE_NITS,
            });
        }
        if u32::from(self.max_fall) > MAX_HDR_LUMINANCE_NITS {
            return Err(ColorProfileError::LuminanceOutOfRange {
                field: "max_fall",
                value: u32::from(self.max_fall),
                max: MAX_HDR_LUMINANCE_NITS,
            });
        }
        if self.max_cll > 0 && self.max_fall > self.max_cll {
            return Err(ColorProfileError::MaxFallExceedsMaxCll {
                max_fall: self.max_fall,
                max_cll: self.max_cll,
            });
        }
        if self.max_mastering_nits == 0 || self.max_mastering_nits > MAX_HDR_LUMINANCE_NITS {
            return Err(ColorProfileError::LuminanceOutOfRange {
                field: "max_mastering_nits",
                value: self.max_mastering_nits,
                max: MAX_HDR_LUMINANCE_NITS,
            });
        }
        let max_0001 = self
            .max_mastering_nits
            .saturating_mul(MIN_MASTERING_LUMINANCE_SCALE);
        if self.min_mastering_nits_0001 >= max_0001 {
            return Err(ColorProfileError::InvalidMasteringLuminance {
                min_0001: self.min_mastering_nits_0001,
                max_nits: self.max_mastering_nits,
            });
        }
        Ok(())
    }
}

impl Default for HdrMasteringMetadata {
    fn default() -> Self {
        Self::hdr10_default()
    }
}

/// Validation errors for [`ColorProfile`] and [`HdrMasteringMetadata`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorProfileError {
    /// HDR transfer functions (PQ / HLG) require 10-bit color depth.
    HdrRequires10Bit {
        transfer: HdrTransfer,
        bit_depth: ColorBitDepth,
    },
    /// HDR transfer functions (PQ / HLG) require BT.2020 primaries.
    HdrRequiresBt2020 {
        transfer: HdrTransfer,
        primaries: ColorPrimaries,
    },
    /// SDR streams must not carry HDR mastering metadata.
    SdrDisallowsHdrMetadata,
    /// `max_fall` cannot exceed `max_cll` when `max_cll` is specified.
    MaxFallExceedsMaxCll { max_fall: u16, max_cll: u16 },
    /// A luminance field is zero where prohibited or exceeds SMPTE ST 2084 bounds (10,000 nits).
    LuminanceOutOfRange {
        field: &'static str,
        value: u32,
        max: u32,
    },
    /// Mastering minimum luminance must be strictly less than maximum luminance.
    InvalidMasteringLuminance { min_0001: u32, max_nits: u32 },
}

impl fmt::Display for ColorProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HdrRequires10Bit {
                transfer,
                bit_depth,
            } => write!(
                f,
                "HDR transfer {transfer} requires 10-bit color depth, got {bit_depth}"
            ),
            Self::HdrRequiresBt2020 {
                transfer,
                primaries,
            } => write!(
                f,
                "HDR transfer {transfer} requires BT.2020 primaries, got {primaries}"
            ),
            Self::SdrDisallowsHdrMetadata => {
                f.write_str("SDR color profile must not carry HDR mastering metadata")
            }
            Self::MaxFallExceedsMaxCll { max_fall, max_cll } => write!(
                f,
                "MaxFALL ({max_fall} nits) cannot exceed MaxCLL ({max_cll} nits)"
            ),
            Self::LuminanceOutOfRange { field, value, max } => write!(
                f,
                "HDR luminance field '{field}' value {value} is out of valid range (1..={max})"
            ),
            Self::InvalidMasteringLuminance { min_0001, max_nits } => write!(
                f,
                "Mastering min luminance ({min_0001} * 0.0001 nits) must be less than max luminance ({max_nits} nits)"
            ),
        }
    }
}

impl Error for ColorProfileError {}

/// Complete wire-serializable video color profile negotiated between host and viewer.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorProfile {
    pub chroma: ChromaSubsampling,
    pub bit_depth: ColorBitDepth,
    pub transfer: HdrTransfer,
    pub primaries: ColorPrimaries,
    pub hdr_metadata: Option<HdrMasteringMetadata>,
}

impl Default for ColorProfile {
    fn default() -> Self {
        Self::sdr_420()
    }
}

impl ColorProfile {
    /// Baseline 8-bit SDR BT.709 YUV 4:2:0 profile.
    pub const fn sdr_420() -> Self {
        Self {
            chroma: ChromaSubsampling::Yuv420,
            bit_depth: ColorBitDepth::Bit8,
            transfer: HdrTransfer::Sdr,
            primaries: ColorPrimaries::Bt709,
            hdr_metadata: None,
        }
    }

    /// 8-bit SDR BT.709 YUV 4:4:4 true-color profile for crisp text and desktop UI.
    pub const fn sdr_444() -> Self {
        Self {
            chroma: ChromaSubsampling::Yuv444,
            bit_depth: ColorBitDepth::Bit8,
            transfer: HdrTransfer::Sdr,
            primaries: ColorPrimaries::Bt709,
            hdr_metadata: None,
        }
    }

    /// 10-bit SDR BT.709 YUV 4:4:4 profile (reduced banding on gradients).
    pub const fn sdr_10bit_444() -> Self {
        Self {
            chroma: ChromaSubsampling::Yuv444,
            bit_depth: ColorBitDepth::Bit10,
            transfer: HdrTransfer::Sdr,
            primaries: ColorPrimaries::Bt709,
            hdr_metadata: None,
        }
    }

    /// 10-bit HDR10 (PQ ST 2084 + BT.2020) YUV 4:2:0 profile.
    pub const fn hdr10_420(hdr_metadata: Option<HdrMasteringMetadata>) -> Self {
        Self {
            chroma: ChromaSubsampling::Yuv420,
            bit_depth: ColorBitDepth::Bit10,
            transfer: HdrTransfer::PqSt2084,
            primaries: ColorPrimaries::Bt2020,
            hdr_metadata,
        }
    }

    /// 10-bit HDR10 (PQ ST 2084 + BT.2020) YUV 4:4:4 true-color profile.
    pub const fn hdr10_444(hdr_metadata: Option<HdrMasteringMetadata>) -> Self {
        Self {
            chroma: ChromaSubsampling::Yuv444,
            bit_depth: ColorBitDepth::Bit10,
            transfer: HdrTransfer::PqSt2084,
            primaries: ColorPrimaries::Bt2020,
            hdr_metadata,
        }
    }

    /// 10-bit HLG (ARIB STD-B67 + BT.2020) YUV 4:2:0 profile.
    pub const fn hlg_420(hdr_metadata: Option<HdrMasteringMetadata>) -> Self {
        Self {
            chroma: ChromaSubsampling::Yuv420,
            bit_depth: ColorBitDepth::Bit10,
            transfer: HdrTransfer::Hlg,
            primaries: ColorPrimaries::Bt2020,
            hdr_metadata,
        }
    }

    /// 10-bit HLG (ARIB STD-B67 + BT.2020) YUV 4:4:4 profile.
    pub const fn hlg_444(hdr_metadata: Option<HdrMasteringMetadata>) -> Self {
        Self {
            chroma: ChromaSubsampling::Yuv444,
            bit_depth: ColorBitDepth::Bit10,
            transfer: HdrTransfer::Hlg,
            primaries: ColorPrimaries::Bt2020,
            hdr_metadata,
        }
    }

    pub const fn is_hdr(&self) -> bool {
        self.transfer.is_hdr()
    }

    pub const fn is_yuv444(&self) -> bool {
        self.chroma.is_444()
    }

    /// Validate that all fields form a standards-compliant color profile.
    pub fn validate(&self) -> Result<(), ColorProfileError> {
        if self.transfer.is_hdr() {
            if !self.bit_depth.is_10bit() {
                return Err(ColorProfileError::HdrRequires10Bit {
                    transfer: self.transfer,
                    bit_depth: self.bit_depth,
                });
            }
            if self.primaries != ColorPrimaries::Bt2020 {
                return Err(ColorProfileError::HdrRequiresBt2020 {
                    transfer: self.transfer,
                    primaries: self.primaries,
                });
            }
            if let Some(meta) = &self.hdr_metadata {
                meta.validate()?;
            }
        } else if self.hdr_metadata.is_some() {
            return Err(ColorProfileError::SdrDisallowsHdrMetadata);
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, bincode::Error> {
        bincode::serialize(self)
    }

    pub fn decode(data: &[u8]) -> Result<Self, bincode::Error> {
        bincode::deserialize(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_presets_pass_validation() {
        let presets = [
            ColorProfile::default(),
            ColorProfile::sdr_420(),
            ColorProfile::sdr_444(),
            ColorProfile::sdr_10bit_444(),
            ColorProfile::hdr10_420(None),
            ColorProfile::hdr10_420(Some(HdrMasteringMetadata::hdr10_default())),
            ColorProfile::hdr10_444(Some(HdrMasteringMetadata::hdr10_default())),
            ColorProfile::hlg_420(None),
            ColorProfile::hlg_444(Some(HdrMasteringMetadata::hdr10_default())),
        ];
        for profile in presets {
            assert_eq!(profile.validate(), Ok(()), "preset should be valid: {profile:?}");
        }
    }

    #[test]
    fn hdr_requires_10bit_and_bt2020() {
        let pq_8bit = ColorProfile {
            chroma: ChromaSubsampling::Yuv420,
            bit_depth: ColorBitDepth::Bit8,
            transfer: HdrTransfer::PqSt2084,
            primaries: ColorPrimaries::Bt2020,
            hdr_metadata: None,
        };
        assert_eq!(
            pq_8bit.validate(),
            Err(ColorProfileError::HdrRequires10Bit {
                transfer: HdrTransfer::PqSt2084,
                bit_depth: ColorBitDepth::Bit8,
            })
        );

        let hlg_bt709 = ColorProfile {
            chroma: ChromaSubsampling::Yuv444,
            bit_depth: ColorBitDepth::Bit10,
            transfer: HdrTransfer::Hlg,
            primaries: ColorPrimaries::Bt709,
            hdr_metadata: None,
        };
        assert_eq!(
            hlg_bt709.validate(),
            Err(ColorProfileError::HdrRequiresBt2020 {
                transfer: HdrTransfer::Hlg,
                primaries: ColorPrimaries::Bt709,
            })
        );
    }

    #[test]
    fn sdr_rejects_hdr_mastering_metadata() {
        let invalid = ColorProfile {
            hdr_metadata: Some(HdrMasteringMetadata::hdr10_default()),
            ..ColorProfile::sdr_444()
        };
        assert_eq!(
            invalid.validate(),
            Err(ColorProfileError::SdrDisallowsHdrMetadata)
        );
    }

    #[test]
    fn hdr_mastering_metadata_validation_rules() {
        let ok = HdrMasteringMetadata::from_nits(1000, 400, 0.005, 1000).unwrap();
        assert_eq!(ok.min_mastering_nits_0001, 50);
        assert!((ok.min_mastering_nits() - 0.005).abs() < 1e-6);

        // max_fall > max_cll when max_cll > 0 is invalid
        let bad_fall = HdrMasteringMetadata {
            max_cll: 400,
            max_fall: 600,
            min_mastering_nits_0001: 50,
            max_mastering_nits: 1000,
        };
        assert_eq!(
            bad_fall.validate(),
            Err(ColorProfileError::MaxFallExceedsMaxCll {
                max_fall: 600,
                max_cll: 400,
            })
        );

        // max_cll == 0 means unspecified, so max_fall > 0 is permitted
        let unspecified_cll = HdrMasteringMetadata {
            max_cll: 0,
            max_fall: 300,
            min_mastering_nits_0001: 10,
            max_mastering_nits: 1000,
        };
        assert_eq!(unspecified_cll.validate(), Ok(()));

        // min >= max mastering luminance is invalid
        let bad_bounds = HdrMasteringMetadata {
            max_cll: 1000,
            max_fall: 400,
            min_mastering_nits_0001: 10_000_000,
            max_mastering_nits: 1000,
        };
        assert_eq!(
            bad_bounds.validate(),
            Err(ColorProfileError::InvalidMasteringLuminance {
                min_0001: 10_000_000,
                max_nits: 1000,
            })
        );

        // max_mastering_nits > 10_000 is out of range
        let over_max = HdrMasteringMetadata {
            max_cll: 1000,
            max_fall: 400,
            min_mastering_nits_0001: 50,
            max_mastering_nits: 12_000,
        };
        assert_eq!(
            over_max.validate(),
            Err(ColorProfileError::LuminanceOutOfRange {
                field: "max_mastering_nits",
                value: 12_000,
                max: MAX_HDR_LUMINANCE_NITS,
            })
        );

        // NaN or negative min_mastering_nits rejected
        assert!(HdrMasteringMetadata::from_nits(1000, 400, f32::NAN, 1000).is_err());
        assert!(HdrMasteringMetadata::from_nits(1000, 400, -0.1, 1000).is_err());
    }

    #[test]
    fn bincode_roundtrip_all_variants() {
        let profiles = [
            ColorProfile::sdr_420(),
            ColorProfile::sdr_444(),
            ColorProfile::sdr_10bit_444(),
            ColorProfile::hdr10_420(None),
            ColorProfile::hdr10_444(Some(HdrMasteringMetadata {
                max_cll: 4000,
                max_fall: 800,
                min_mastering_nits_0001: 1,
                max_mastering_nits: 4000,
            })),
            ColorProfile::hlg_420(Some(HdrMasteringMetadata::hdr10_default())),
        ];
        for profile in profiles {
            let bytes = profile.encode().expect("bincode encode should succeed");
            let decoded = ColorProfile::decode(&bytes).expect("bincode decode should succeed");
            assert_eq!(decoded, profile);
            assert_eq!(decoded.validate(), Ok(()));
        }
    }

    #[test]
    fn cicp_code_points_match_itu_h273() {
        assert_eq!(HdrTransfer::Sdr.cicp_transfer_characteristics(), 1);
        assert_eq!(HdrTransfer::PqSt2084.cicp_transfer_characteristics(), 16);
        assert_eq!(HdrTransfer::Hlg.cicp_transfer_characteristics(), 18);
        assert_eq!(ColorPrimaries::Bt709.cicp_color_primaries(), 1);
        assert_eq!(ColorPrimaries::Bt2020.cicp_color_primaries(), 9);
        assert_eq!(ColorPrimaries::Bt709.cicp_matrix_coefficients(), 1);
        assert_eq!(ColorPrimaries::Bt2020.cicp_matrix_coefficients(), 9);
    }
}
