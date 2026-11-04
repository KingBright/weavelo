//! Bounded CPU snapshot bytes for observation, separate from the renderer.
//! Native macOS snapshots use Core Image color conversion, never renderer fallback.
use std::sync::Arc;
#[cfg(target_os = "macos")]
pub(crate) mod macos;

pub const MAX_OBSERVATION_BYTES: usize = 16 * 1024 * 1024;
const MAX_DIMENSION: u32 = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelError {
    NativeReadbackUnsupported,
    InvalidDimensions,
    InvalidStride,
    LengthMismatch,
    SizeLimit,
    ArithmeticOverflow,
    ColorMetadataUnavailable,
    NativeReadbackFailed,
}
impl std::fmt::Display for PixelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for PixelError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMetadata {
    /// The software decoder's RGBA buffer carries no range/matrix/transfer/
    /// primaries metadata. Never relabel it sRGB/BT.709 or invent SPA numbers.
    Unspecified,
    /// Core Image converted declared source color into sRGB RGBA8, opaque alpha.
    Srgb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReticleStyle {
    Hover,
    Click,
    Drag { from_x: u16, from_y: u16 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RgbaLayout {
    width: u32,
    height: u32,
    row_stride: usize,
    bytes: usize,
}
fn byte_length(row_stride: usize, height: usize) -> Result<usize, PixelError> {
    row_stride.checked_mul(height).ok_or(PixelError::ArithmeticOverflow)
}
impl RgbaLayout {
    /// Only tightly packed RGBA8 is supported. Reject padding/trailing bytes
    /// rather than exposing them or silently treating BGRA/YUV as RGBA.
    pub fn validate(width: u32, height: u32, row_stride: usize,
        length: usize, maximum: usize) -> Result<Self, PixelError> {
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(PixelError::InvalidDimensions);
        }
        let row = usize::try_from(width).ok().and_then(|w| w.checked_mul(4))
            .ok_or(PixelError::ArithmeticOverflow)?;
        let rows = usize::try_from(height).map_err(|_| PixelError::ArithmeticOverflow)?;
        let bytes = byte_length(row_stride, rows)?;
        if row_stride != row { return Err(PixelError::InvalidStride); }
        if bytes > maximum.min(MAX_OBSERVATION_BYTES) { return Err(PixelError::SizeLimit); }
        if length != bytes { return Err(PixelError::LengthMismatch); }
        Ok(Self { width, height, row_stride, bytes })
    }
    pub fn width(self) -> u32 { self.width }
    pub fn height(self) -> u32 { self.height }
    pub fn row_stride(self) -> usize { self.row_stride }
    pub fn bytes(self) -> usize { self.bytes }
}

/// Validated existing bytes, not an encoded image or remote artifact receipt.
/// Cloning the Arc does not allocate/copy a second full-sized frame.
#[derive(Clone)]
pub struct CpuRgba {
    layout: RgbaLayout,
    pixels: Arc<Vec<u8>>,
    color: ColorMetadata,
}
impl CpuRgba {
    pub(crate) fn from_decoded(width: u32, height: u32, pixels: Arc<Vec<u8>>,
        maximum: usize) -> Result<Self, PixelError> {
        let stride = usize::try_from(width).ok().and_then(|w| w.checked_mul(4))
            .ok_or(PixelError::ArithmeticOverflow)?;
        let layout = RgbaLayout::validate(width, height, stride, pixels.len(), maximum)?;
        Ok(Self { layout, pixels, color: ColorMetadata::Unspecified })
    }
    pub fn layout(&self) -> RgbaLayout { self.layout }
    pub fn pixels(&self) -> &[u8] { &self.pixels }
    pub fn color(&self) -> ColorMetadata { self.color }
    pub fn into_pixels(self) -> Arc<Vec<u8>> { self.pixels }

    /// Crops a 1:1 full-resolution sub-rectangle using `[x0, y0, x1, y1]` on the
    /// `[0, 1000]` normalized grid (Anthropic `zoom` / Gemini 1000x1000 grid).
    pub fn zoom_crop(&self, region: [u16; 4], maximum: usize) -> Result<Self, PixelError> {
        let [x0, y0, x1, y1] = region;
        if x0 >= x1 || y0 >= y1 || x1 > 1000 || y1 > 1000 {
            return Err(PixelError::InvalidDimensions);
        }
        let w = self.layout.width as u64;
        let h = self.layout.height as u64;
        let px0 = ((u64::from(x0) * w) / 1000) as u32;
        let py0 = ((u64::from(y0) * h) / 1000) as u32;
        let px1 = ((u64::from(x1) * w).div_ceil(1000)).min(w) as u32;
        let py1 = ((u64::from(y1) * h).div_ceil(1000)).min(h) as u32;
        let crop_w = px1.saturating_sub(px0);
        let crop_h = py1.saturating_sub(py0);
        if crop_w == 0 || crop_h == 0 {
            return Err(PixelError::InvalidDimensions);
        }
        let crop_stride = (crop_w as usize)
            .checked_mul(4)
            .ok_or(PixelError::ArithmeticOverflow)?;
        let crop_bytes = byte_length(crop_stride, crop_h as usize)?;
        let layout = RgbaLayout::validate(crop_w, crop_h, crop_stride, crop_bytes, maximum)?;
        let mut out = Vec::with_capacity(crop_bytes);
        let src_stride = self.layout.row_stride;
        for row in py0..py1 {
            let start = (row as usize) * src_stride + (px0 as usize) * 4;
            let end = start + crop_stride;
            out.extend_from_slice(&self.pixels[start..end]);
        }
        Ok(Self {
            layout,
            pixels: Arc::new(out),
            color: self.color,
        })
    }

    /// Synthesizes a center-hollow, dual-stroke symmetric crosshair reticle onto a
    /// copy of the observation buffer so the VLM can visually verify and self-correct
    /// its landing coordinates without obscuring text or small icons at `(x, y)`.
    /// Coordinates `x_norm` and `y_norm` use `[0, u16::MAX]` on both axes.
    pub fn with_reticle(&self, x_norm: u16, y_norm: u16, style: ReticleStyle) -> Self {
        let mut buf = (*self.pixels).clone();
        let w = self.layout.width as i32;
        let h = self.layout.height as i32;
        if w <= 0 || h <= 0 {
            return self.clone();
        }
        let map_coord = |val: u16, extent: i32| -> i32 {
            if extent <= 1 {
                return 0;
            }
            let scale = u64::from(u16::MAX);
            (((u64::from(val).min(scale)) * (extent as u64 - 1) + scale / 2) / scale) as i32
        };
        let cx = map_coord(x_norm, w);
        let cy = map_coord(y_norm, h);

        if let ReticleStyle::Drag { from_x, from_y } = style {
            let sx = map_coord(from_x, w);
            let sy = map_coord(from_y, h);
            draw_drag_line(&mut buf, w, h, sx, sy, cx, cy);
        }

        let accent = match style {
            ReticleStyle::Hover => [0u8, 220, 255, 255],
            ReticleStyle::Click => [255u8, 40, 200, 255],
            ReticleStyle::Drag { .. } => [32u8, 232, 136, 255],
        };
        let black = [0u8, 0, 0, 255];

        // Outer radius 12, inner hollow gap radius < 4 (center 7x7 is untouched).
        for dy in -16i32..=16 {
            for dx in -16i32..=16 {
                let px = cx + dx;
                let py = cy + dy;
                if px < 0 || px >= w || py < 0 || py >= h {
                    continue;
                }
                let r2 = dx * dx + dy * dy;
                // Center hollow region (r < 4 => r^2 < 16) is strictly preserved.
                if r2 < 16 {
                    continue;
                }
                let on_crosshair = (dx == 0 && dy.abs() >= 4 && dy.abs() <= 14)
                    || (dy == 0 && dx.abs() >= 4 && dx.abs() <= 14);
                let on_crosshair_outline = (dx.abs() == 1 && dy.abs() >= 4 && dy.abs() <= 14)
                    || (dy.abs() == 1 && dx.abs() >= 4 && dx.abs() <= 14);
                let on_inner_ring = (81..=121).contains(&r2); // r in 9..=11
                let on_outer_ring = (64..81).contains(&r2) || (121..=144).contains(&r2); // 1px black border
                let on_click_ripple = matches!(style, ReticleStyle::Click)
                    && (210..=256).contains(&r2)
                    && ((dx + dy) & 1 == 0);

                let color = if on_crosshair || on_inner_ring || on_click_ripple {
                    Some(accent)
                } else if on_outer_ring || on_crosshair_outline {
                    Some(black)
                } else {
                    None
                };
                if let Some(rgba) = color {
                    let idx = ((py as usize) * (w as usize) + (px as usize)) * 4;
                    buf[idx..idx + 4].copy_from_slice(&rgba);
                }
            }
        }
        Self {
            layout: self.layout,
            pixels: Arc::new(buf),
            color: self.color,
        }
    }

    /// Computes normalized mean absolute pixel difference (`0.0..=1.0`) for
    /// client-side visual stability / frame-diff convergence checks.
    pub fn frame_diff_ratio(&self, other: &Self) -> Option<f32> {
        if self.layout != other.layout || self.pixels.is_empty() {
            return None;
        }
        let step = (self.pixels.len() / (4 * 4096)).max(1) * 4;
        let mut diff_sum = 0u64;
        let mut samples = 0u64;
        let a = self.pixels.as_slice();
        let b = other.pixels.as_slice();
        let mut idx = 0;
        while idx + 3 < a.len() {
            diff_sum += u64::from(a[idx].abs_diff(b[idx]))
                + u64::from(a[idx + 1].abs_diff(b[idx + 1]))
                + u64::from(a[idx + 2].abs_diff(b[idx + 2]));
            samples += 3;
            idx += step;
        }
        Some(if samples == 0 {
            0.0
        } else {
            (diff_sum as f32) / ((samples * 255) as f32)
        })
    }
}

fn draw_drag_line(buf: &mut [u8], w: i32, h: i32, x0: i32, y0: i32, x1: i32, y1: i32) {
    let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
    for i in 0..=steps {
        let px = x0 + (x1 - x0) * i / steps;
        let py = y0 + (y1 - y0) * i / steps;
        if px >= 0 && px < w && py >= 0 && py < h {
            let idx = ((py as usize) * (w as usize) + (px as usize)) * 4;
            buf[idx] = ((u16::from(buf[idx]) + 32) / 2) as u8;
            buf[idx + 1] = ((u16::from(buf[idx + 1]) + 232) / 2) as u8;
            buf[idx + 2] = ((u16::from(buf[idx + 2]) + 136) / 2) as u8;
            buf[idx + 3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tightly_packed_cpu_snapshot_retains_exact_bytes_without_copying() {
        let pixels = Arc::new(vec![17, 29, 41, 255, 2, 3, 5, 255]);
        let frame = CpuRgba::from_decoded(2, 1, pixels.clone(), 8).unwrap();
        assert_eq!(Arc::strong_count(&pixels), 2);
        assert_eq!(frame.pixels(), pixels.as_slice());
        assert_eq!(frame.layout().row_stride(), 8);
        assert_eq!(frame.layout().bytes(), 8);
        assert_eq!(frame.color(), ColorMetadata::Unspecified);
    }
    #[test]
    fn empty_native_like_buffer_is_not_a_successful_image() {
        assert_eq!(CpuRgba::from_decoded(2, 2, Arc::new(vec![]), 16).err(),
            Some(PixelError::LengthMismatch));
    }
    #[test]
    fn zero_and_oversized_dimensions_are_rejected() {
        for (w,h) in [(0,1),(1,0),(8193,1),(1,8193),(u32::MAX,u32::MAX)] {
            assert_eq!(RgbaLayout::validate(w,h,4,4,16).unwrap_err(), PixelError::InvalidDimensions);
        }
    }
    #[test]
    fn padded_or_short_rows_are_never_exported() {
        for stride in [0,7,9,12] {
            assert_eq!(RgbaLayout::validate(2,2,stride,stride*2,64).unwrap_err(), PixelError::InvalidStride);
        }
    }
    #[test]
    fn truncated_and_trailing_bytes_are_rejected() {
        for length in [0,15,17,32] {
            assert_eq!(RgbaLayout::validate(2,2,8,length,64).unwrap_err(), PixelError::LengthMismatch);
        }
    }
    #[test]
    fn checked_extent_rejects_machine_word_overflow() {
        assert_eq!(byte_length(usize::MAX,2), Err(PixelError::ArithmeticOverflow));
        assert_eq!(RgbaLayout::validate(2,2,usize::MAX,0,usize::MAX).unwrap_err(), PixelError::ArithmeticOverflow);
    }
    #[test]
    fn caller_budget_cannot_expand_the_fixed_memory_limit() {
        assert_eq!(RgbaLayout::validate(4096,2160,16384,4096*2160*4,usize::MAX).unwrap_err(),
            PixelError::SizeLimit);
        assert_eq!(RgbaLayout::validate(2,2,8,16,15).unwrap_err(), PixelError::SizeLimit);
        assert_eq!(RgbaLayout::validate(1,1,4,4,0).unwrap_err(), PixelError::SizeLimit);
    }
    #[test]
    fn exact_fixed_budget_boundary_is_valid() {
        let layout = RgbaLayout::validate(2048,2048,8192,MAX_OBSERVATION_BYTES,usize::MAX).unwrap();
        assert_eq!(layout.bytes(), MAX_OBSERVATION_BYTES);
    }
    #[test]
    fn zoom_crop_extracts_exact_normalized_subregion_and_rejects_invalid_bounds() {
        let mut raw = Vec::with_capacity(4 * 4 * 4);
        for y in 0..4u8 {
            for x in 0..4u8 {
                raw.extend_from_slice(&[x * 10, y * 10, 100, 255]);
            }
        }
        let frame = CpuRgba::from_decoded(4, 4, Arc::new(raw), 1024).unwrap();
        let cropped = frame.zoom_crop([500, 500, 1000, 1000], 1024).unwrap();
        assert_eq!((cropped.layout().width(), cropped.layout().height()), (2, 2));
        assert_eq!(&cropped.pixels()[0..4], &[20, 20, 100, 255]);
        assert_eq!(&cropped.pixels()[12..16], &[30, 30, 100, 255]);
        assert!(frame.zoom_crop([600, 500, 500, 1000], 1024).is_err());
        assert!(frame.zoom_crop([0, 0, 1001, 1000], 1024).is_err());
    }
    #[test]
    fn reticle_preserves_center_hollow_pixels_and_draws_high_contrast_ring() {
        let raw = vec![120u8; 40 * 40 * 4];
        let frame = CpuRgba::from_decoded(40, 40, Arc::new(raw), 40 * 40 * 4).unwrap();
        let marked = frame.with_reticle(32768, 32768, ReticleStyle::Click);
        let cx = 20usize;
        let cy = 20usize;
        let center_idx = (cy * 40 + cx) * 4;
        // Center pixel and immediate 3px neighbors must remain untouched (hollow).
        assert_eq!(&marked.pixels()[center_idx..center_idx + 4], &[120, 120, 120, 120]);
        let ring_idx = (cy * 40 + (cx + 10)) * 4;
        assert_eq!(&marked.pixels()[ring_idx..ring_idx + 4], &[255, 40, 200, 255]);
        assert!(frame.frame_diff_ratio(&marked).unwrap() > 0.0);
        assert_eq!(frame.frame_diff_ratio(&frame), Some(0.0));
    }
}
