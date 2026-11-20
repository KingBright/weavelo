//! Wire protocol types for virtual displays, wireless secondary displays, and super-screen presets.
//!
//! All types support compact `serde` + `bincode` serialization and enforce strict
//! geometry, refresh-rate, DPI-scale, and name bounds before crossing host/viewer boundaries.

use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

/// Minimum supported virtual display width in pixels.
pub const MIN_VIRTUAL_DISPLAY_WIDTH: u32 = 320;
/// Maximum supported virtual display width in pixels (8K UHD).
pub const MAX_VIRTUAL_DISPLAY_WIDTH: u32 = 7680;
/// Minimum supported virtual display height in pixels.
pub const MIN_VIRTUAL_DISPLAY_HEIGHT: u32 = 240;
/// Maximum supported virtual display height in pixels (8K UHD).
pub const MAX_VIRTUAL_DISPLAY_HEIGHT: u32 = 4320;

/// Minimum supported virtual display refresh rate in Hz.
pub const MIN_VIRTUAL_DISPLAY_REFRESH_HZ: u16 = 24;
/// Maximum supported virtual display refresh rate in Hz.
pub const MAX_VIRTUAL_DISPLAY_REFRESH_HZ: u16 = 240;
/// Default refresh rate used when a client reports 0 Hz.
pub const DEFAULT_VIRTUAL_DISPLAY_REFRESH_HZ: u16 = 60;

/// Minimum supported HiDPI scale factor.
pub const MIN_VIRTUAL_DISPLAY_DPI_SCALE: f32 = 0.5;
/// Maximum supported HiDPI scale factor.
pub const MAX_VIRTUAL_DISPLAY_DPI_SCALE: f32 = 4.0;

/// Maximum UTF-8 byte length for a virtual display name.
pub const MAX_VIRTUAL_DISPLAY_NAME_BYTES: usize = 64;

/// Baseline DPI (160 dpi = 1.0x) used to convert client physical DPI into an OS HiDPI scale factor.
pub const WIRELESS_EXTEND_BASELINE_DPI: f32 = 160.0;

/// Maximum number of displays permitted in a single [`VirtualDisplayCommand::Snapshot`] payload.
pub const MAX_SNAPSHOT_DISPLAYS: usize = 32;

/// Relative placement of a virtual display with respect to the host's primary display.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum DisplayPlacement {
    /// Positioned immediately to the right of the primary display.
    #[default]
    RightOf,
    /// Positioned immediately to the left of the primary display.
    LeftOf,
    /// Positioned immediately above the primary display.
    Above,
    /// Positioned immediately below the primary display.
    Below,
    /// Mirrors the primary display content.
    Mirror,
    /// Standalone headless display with no relative anchor.
    Standalone,
}

impl fmt::Display for DisplayPlacement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RightOf => f.write_str("right_of"),
            Self::LeftOf => f.write_str("left_of"),
            Self::Above => f.write_str("above"),
            Self::Below => f.write_str("below"),
            Self::Mirror => f.write_str("mirror"),
            Self::Standalone => f.write_str("standalone"),
        }
    }
}

/// Built-in resolution presets for virtual displays and super-screens.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum VirtualDisplayPreset {
    /// Standard 16:9 Full HD (1920x1080).
    #[default]
    Fhd1080p,
    /// Standard 16:9 Quad HD / 2K (2560x1440).
    Qhd1440p,
    /// Standard 16:9 Ultra HD / 4K (3840x2160).
    Uhd4k,
    /// 21:9 Ultrawide QHD (3440x1440).
    Ultrawide21x9,
    /// 32:9 Super-Ultrawide DQHD (5120x1440).
    SuperUltrawide32x9,
    /// Dynamically matches the connecting client's display geometry and DPI.
    MatchClient,
}

impl VirtualDisplayPreset {
    /// Returns the fixed `(width, height)` in pixels for static presets,
    /// or `None` for [`VirtualDisplayPreset::MatchClient`].
    pub const fn dimensions(self) -> Option<(u32, u32)> {
        match self {
            Self::Fhd1080p => Some((1920, 1080)),
            Self::Qhd1440p => Some((2560, 1440)),
            Self::Uhd4k => Some((3840, 2160)),
            Self::Ultrawide21x9 => Some((3440, 1440)),
            Self::SuperUltrawide32x9 => Some((5120, 1440)),
            Self::MatchClient => None,
        }
    }

    /// Recommended default HiDPI scale factor for this preset.
    pub const fn default_dpi_scale(self) -> f32 {
        match self {
            Self::Uhd4k => 2.0,
            Self::Fhd1080p
            | Self::Qhd1440p
            | Self::Ultrawide21x9
            | Self::SuperUltrawide32x9
            | Self::MatchClient => 1.0,
        }
    }

    /// Default human-readable display name for this preset.
    pub const fn default_name(self) -> &'static str {
        match self {
            Self::Fhd1080p => "Weavelo 1080p",
            Self::Qhd1440p => "Weavelo 2K",
            Self::Uhd4k => "Weavelo 4K",
            Self::Ultrawide21x9 => "Weavelo 21:9",
            Self::SuperUltrawide32x9 => "Weavelo 32:9",
            Self::MatchClient => "Weavelo Wireless Display",
        }
    }

    /// Builds a [`VirtualDisplaySpec`] for any fixed-dimension preset.
    ///
    /// Returns `None` for [`VirtualDisplayPreset::MatchClient`], which should be
    /// resolved via [`WirelessExtendRequest::to_virtual_display_spec`].
    pub fn to_spec(
        self,
        refresh_hz: u16,
        placement: DisplayPlacement,
    ) -> Option<VirtualDisplaySpec> {
        let (width, height) = self.dimensions()?;
        Some(VirtualDisplaySpec {
            name: self.default_name().to_owned(),
            width,
            height,
            refresh_hz,
            dpi_scale: self.default_dpi_scale(),
            hdr: false,
            placement,
        })
    }
}

/// Validation errors for virtual display specifications and wireless extend requests.
#[derive(Debug, Clone, PartialEq)]
pub enum VirtualDisplayError {
    /// Display name is empty or whitespace-only.
    EmptyName,
    /// Display name exceeds [`MAX_VIRTUAL_DISPLAY_NAME_BYTES`].
    NameTooLong { len: usize, max: usize },
    /// Display width is outside `MIN_VIRTUAL_DISPLAY_WIDTH..=MAX_VIRTUAL_DISPLAY_WIDTH`.
    WidthOutOfRange { width: u32, min: u32, max: u32 },
    /// Display height is outside `MIN_VIRTUAL_DISPLAY_HEIGHT..=MAX_VIRTUAL_DISPLAY_HEIGHT`.
    HeightOutOfRange { height: u32, min: u32, max: u32 },
    /// Display dimensions must be even for YUV/hardware video encoders.
    OddDimensions { width: u32, height: u32 },
    /// Refresh rate is outside `MIN_VIRTUAL_DISPLAY_REFRESH_HZ..=MAX_VIRTUAL_DISPLAY_REFRESH_HZ`.
    RefreshRateOutOfRange { refresh_hz: u16, min: u16, max: u16 },
    /// DPI scale is non-finite or outside `MIN_VIRTUAL_DISPLAY_DPI_SCALE..=MAX_VIRTUAL_DISPLAY_DPI_SCALE`.
    InvalidDpiScale { dpi_scale: f32, min: f32, max: f32 },
    /// Client reported zero width or height in a wireless extend request.
    InvalidClientDimensions { width: u32, height: u32 },
    /// Client reported zero DPI in a wireless extend request.
    InvalidClientDpi { dpi: u16 },
    /// Snapshot contained more than [`MAX_SNAPSHOT_DISPLAYS`] entries.
    TooManySnapshotDisplays { count: usize, max: usize },
}

impl fmt::Display for VirtualDisplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => f.write_str("virtual display name must not be empty"),
            Self::NameTooLong { len, max } => write!(
                f,
                "virtual display name length ({len} bytes) exceeds maximum ({max} bytes)"
            ),
            Self::WidthOutOfRange { width, min, max } => {
                write!(f, "virtual display width {width} is out of range ({min}..={max})")
            }
            Self::HeightOutOfRange { height, min, max } => {
                write!(f, "virtual display height {height} is out of range ({min}..={max})")
            }
            Self::OddDimensions { width, height } => write!(
                f,
                "virtual display dimensions ({width}x{height}) must be even numbers"
            ),
            Self::RefreshRateOutOfRange {
                refresh_hz,
                min,
                max,
            } => write!(
                f,
                "virtual display refresh rate {refresh_hz} Hz is out of range ({min}..={max} Hz)"
            ),
            Self::InvalidDpiScale {
                dpi_scale,
                min,
                max,
            } => write!(
                f,
                "virtual display DPI scale {dpi_scale} is invalid or out of range ({min}..={max})"
            ),
            Self::InvalidClientDimensions { width, height } => write!(
                f,
                "wireless extend client dimensions ({width}x{height}) must be positive"
            ),
            Self::InvalidClientDpi { dpi } => {
                write!(f, "wireless extend client DPI ({dpi}) must be positive")
            }
            Self::TooManySnapshotDisplays { count, max } => write!(
                f,
                "virtual display snapshot count ({count}) exceeds maximum ({max})"
            ),
        }
    }
}

impl Error for VirtualDisplayError {}

/// Wire-serializable specification for creating or updating a virtual display.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct VirtualDisplaySpec {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u16,
    pub dpi_scale: f32,
    pub hdr: bool,
    pub placement: DisplayPlacement,
}

impl VirtualDisplaySpec {
    /// Validates all bounds, even-alignment, finite DPI scale, and name constraints.
    pub fn validate(&self) -> Result<(), VirtualDisplayError> {
        if self.name.trim().is_empty() {
            return Err(VirtualDisplayError::EmptyName);
        }
        if self.name.len() > MAX_VIRTUAL_DISPLAY_NAME_BYTES {
            return Err(VirtualDisplayError::NameTooLong {
                len: self.name.len(),
                max: MAX_VIRTUAL_DISPLAY_NAME_BYTES,
            });
        }
        if !(MIN_VIRTUAL_DISPLAY_WIDTH..=MAX_VIRTUAL_DISPLAY_WIDTH).contains(&self.width) {
            return Err(VirtualDisplayError::WidthOutOfRange {
                width: self.width,
                min: MIN_VIRTUAL_DISPLAY_WIDTH,
                max: MAX_VIRTUAL_DISPLAY_WIDTH,
            });
        }
        if !(MIN_VIRTUAL_DISPLAY_HEIGHT..=MAX_VIRTUAL_DISPLAY_HEIGHT).contains(&self.height) {
            return Err(VirtualDisplayError::HeightOutOfRange {
                height: self.height,
                min: MIN_VIRTUAL_DISPLAY_HEIGHT,
                max: MAX_VIRTUAL_DISPLAY_HEIGHT,
            });
        }
        if self.width % 2 != 0 || self.height % 2 != 0 {
            return Err(VirtualDisplayError::OddDimensions {
                width: self.width,
                height: self.height,
            });
        }
        if !(MIN_VIRTUAL_DISPLAY_REFRESH_HZ..=MAX_VIRTUAL_DISPLAY_REFRESH_HZ)
            .contains(&self.refresh_hz)
        {
            return Err(VirtualDisplayError::RefreshRateOutOfRange {
                refresh_hz: self.refresh_hz,
                min: MIN_VIRTUAL_DISPLAY_REFRESH_HZ,
                max: MAX_VIRTUAL_DISPLAY_REFRESH_HZ,
            });
        }
        if !self.dpi_scale.is_finite()
            || !(MIN_VIRTUAL_DISPLAY_DPI_SCALE..=MAX_VIRTUAL_DISPLAY_DPI_SCALE)
                .contains(&self.dpi_scale)
        {
            return Err(VirtualDisplayError::InvalidDpiScale {
                dpi_scale: self.dpi_scale,
                min: MIN_VIRTUAL_DISPLAY_DPI_SCALE,
                max: MAX_VIRTUAL_DISPLAY_DPI_SCALE,
            });
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

/// Client display metrics sent when requesting a one-tap Wireless Secondary Display.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct WirelessExtendRequest {
    pub client_width: u32,
    pub client_height: u32,
    pub client_dpi: u16,
    pub client_refresh_hz: u16,
    pub landscape: bool,
}

impl WirelessExtendRequest {
    /// Computes an even-aligned, bounds-checked, HiDPI-scaled [`VirtualDisplaySpec`]
    /// matching the client's screen geometry, orientation, and refresh rate.
    pub fn to_virtual_display_spec(&self) -> Result<VirtualDisplaySpec, VirtualDisplayError> {
        if self.client_width == 0 || self.client_height == 0 {
            return Err(VirtualDisplayError::InvalidClientDimensions {
                width: self.client_width,
                height: self.client_height,
            });
        }
        if self.client_dpi == 0 {
            return Err(VirtualDisplayError::InvalidClientDpi {
                dpi: self.client_dpi,
            });
        }

        // Orient width/height according to the requested landscape/portrait flag.
        let (oriented_w, oriented_h) = if self.landscape {
            if self.client_width >= self.client_height {
                (self.client_width, self.client_height)
            } else {
                (self.client_height, self.client_width)
            }
        } else if self.client_height >= self.client_width {
            (self.client_width, self.client_height)
        } else {
            (self.client_height, self.client_width)
        };

        // Proportionally scale down if the client exceeds maximum virtual display bounds.
        let scale_w = MAX_VIRTUAL_DISPLAY_WIDTH as f64 / oriented_w as f64;
        let scale_h = MAX_VIRTUAL_DISPLAY_HEIGHT as f64 / oriented_h as f64;
        let downscale = scale_w.min(scale_h).min(1.0);

        let scaled_w = ((oriented_w as f64) * downscale).round() as u32;
        let scaled_h = ((oriented_h as f64) * downscale).round() as u32;

        // Clamp to valid range and align down to even pixels (bounds are even).
        let width = align_even_clamped(
            scaled_w,
            MIN_VIRTUAL_DISPLAY_WIDTH,
            MAX_VIRTUAL_DISPLAY_WIDTH,
        );
        let height = align_even_clamped(
            scaled_h,
            MIN_VIRTUAL_DISPLAY_HEIGHT,
            MAX_VIRTUAL_DISPLAY_HEIGHT,
        );

        // Compute HiDPI scale quantized to 0.25x steps within [0.5, 4.0].
        let raw_scale = (self.client_dpi as f32) / WIRELESS_EXTEND_BASELINE_DPI;
        let quantized_scale = ((raw_scale * 4.0).round() / 4.0)
            .clamp(MIN_VIRTUAL_DISPLAY_DPI_SCALE, MAX_VIRTUAL_DISPLAY_DPI_SCALE);

        // Clamp refresh rate; default to 60 Hz when unspecified (0).
        let refresh_hz = if self.client_refresh_hz == 0 {
            DEFAULT_VIRTUAL_DISPLAY_REFRESH_HZ
        } else {
            self.client_refresh_hz
                .clamp(MIN_VIRTUAL_DISPLAY_REFRESH_HZ, MAX_VIRTUAL_DISPLAY_REFRESH_HZ)
        };

        let spec = VirtualDisplaySpec {
            name: VirtualDisplayPreset::MatchClient.default_name().to_owned(),
            width,
            height,
            refresh_hz,
            dpi_scale: quantized_scale,
            hdr: false,
            placement: DisplayPlacement::RightOf,
        };
        spec.validate()?;
        Ok(spec)
    }

    pub fn encode(&self) -> Result<Vec<u8>, bincode::Error> {
        bincode::serialize(self)
    }

    pub fn decode(data: &[u8]) -> Result<Self, bincode::Error> {
        bincode::deserialize(data)
    }
}

#[inline]
fn align_even_clamped(value: u32, min: u32, max: u32) -> u32 {
    let clamped = value.clamp(min, max);
    let even = clamped & !1;
    even.max(min)
}

/// Host-reported status of a managed virtual display.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct VirtualDisplayInfo {
    pub id: u32,
    pub spec: VirtualDisplaySpec,
    pub active: bool,
    pub owner_session_id: Option<u64>,
}

impl VirtualDisplayInfo {
    pub fn validate(&self) -> Result<(), VirtualDisplayError> {
        self.spec.validate()
    }

    pub fn encode(&self) -> Result<Vec<u8>, bincode::Error> {
        bincode::serialize(self)
    }

    pub fn decode(data: &[u8]) -> Result<Self, bincode::Error> {
        bincode::deserialize(data)
    }
}

/// Control-plane commands and host snapshots for virtual display management.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum VirtualDisplayCommand {
    /// Request creation of a new virtual display.
    Create {
        spec: VirtualDisplaySpec,
        persist_after_disconnect: bool,
    },
    /// Update the resolution, refresh rate, DPI scale, HDR, or placement of an existing display.
    Update {
        id: u32,
        spec: VirtualDisplaySpec,
    },
    /// Destroy a virtual display by ID.
    Destroy { id: u32 },
    /// Request the current list of virtual displays from the host.
    List,
    /// Host snapshot of all currently managed virtual displays.
    Snapshot { displays: Vec<VirtualDisplayInfo> },
    /// One-tap wireless secondary display request matching the client's device metrics.
    WirelessExtend(WirelessExtendRequest),
}

impl VirtualDisplayCommand {
    /// Validates any embedded specifications or requests carried by the command.
    pub fn validate(&self) -> Result<(), VirtualDisplayError> {
        match self {
            Self::Create { spec, .. } | Self::Update { spec, .. } => spec.validate(),
            Self::Destroy { .. } | Self::List => Ok(()),
            Self::Snapshot { displays } => {
                if displays.len() > MAX_SNAPSHOT_DISPLAYS {
                    return Err(VirtualDisplayError::TooManySnapshotDisplays {
                        count: displays.len(),
                        max: MAX_SNAPSHOT_DISPLAYS,
                    });
                }
                for display in displays {
                    display.validate()?;
                }
                Ok(())
            }
            Self::WirelessExtend(request) => request.to_virtual_display_spec().map(|_| ()),
        }
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

    fn valid_spec() -> VirtualDisplaySpec {
        VirtualDisplaySpec {
            name: "Studio Secondary".to_owned(),
            width: 2560,
            height: 1440,
            refresh_hz: 120,
            dpi_scale: 1.5,
            hdr: true,
            placement: DisplayPlacement::RightOf,
        }
    }

    #[test]
    fn spec_validation_accepts_boundaries() {
        let min_spec = VirtualDisplaySpec {
            name: "A".to_owned(),
            width: MIN_VIRTUAL_DISPLAY_WIDTH,
            height: MIN_VIRTUAL_DISPLAY_HEIGHT,
            refresh_hz: MIN_VIRTUAL_DISPLAY_REFRESH_HZ,
            dpi_scale: MIN_VIRTUAL_DISPLAY_DPI_SCALE,
            hdr: false,
            placement: DisplayPlacement::Standalone,
        };
        assert!(min_spec.validate().is_ok());

        let max_spec = VirtualDisplaySpec {
            name: "x".repeat(MAX_VIRTUAL_DISPLAY_NAME_BYTES),
            width: MAX_VIRTUAL_DISPLAY_WIDTH,
            height: MAX_VIRTUAL_DISPLAY_HEIGHT,
            refresh_hz: MAX_VIRTUAL_DISPLAY_REFRESH_HZ,
            dpi_scale: MAX_VIRTUAL_DISPLAY_DPI_SCALE,
            hdr: true,
            placement: DisplayPlacement::Mirror,
        };
        assert!(max_spec.validate().is_ok());
    }

    #[test]
    fn spec_validation_rejects_invalid_names() {
        let mut spec = valid_spec();
        spec.name = "".to_owned();
        assert_eq!(spec.validate(), Err(VirtualDisplayError::EmptyName));

        spec.name = "   \t ".to_owned();
        assert_eq!(spec.validate(), Err(VirtualDisplayError::EmptyName));

        spec.name = "a".repeat(MAX_VIRTUAL_DISPLAY_NAME_BYTES + 1);
        assert_eq!(
            spec.validate(),
            Err(VirtualDisplayError::NameTooLong {
                len: MAX_VIRTUAL_DISPLAY_NAME_BYTES + 1,
                max: MAX_VIRTUAL_DISPLAY_NAME_BYTES,
            })
        );
    }

    #[test]
    fn spec_validation_rejects_out_of_range_and_odd_dimensions() {
        let mut spec = valid_spec();
        spec.width = MIN_VIRTUAL_DISPLAY_WIDTH - 2;
        assert!(matches!(
            spec.validate(),
            Err(VirtualDisplayError::WidthOutOfRange { .. })
        ));

        spec.width = MAX_VIRTUAL_DISPLAY_WIDTH + 2;
        assert!(matches!(
            spec.validate(),
            Err(VirtualDisplayError::WidthOutOfRange { .. })
        ));

        spec.width = 1920;
        spec.height = MIN_VIRTUAL_DISPLAY_HEIGHT - 2;
        assert!(matches!(
            spec.validate(),
            Err(VirtualDisplayError::HeightOutOfRange { .. })
        ));

        spec.height = MAX_VIRTUAL_DISPLAY_HEIGHT + 2;
        assert!(matches!(
            spec.validate(),
            Err(VirtualDisplayError::HeightOutOfRange { .. })
        ));

        spec.width = 1921;
        spec.height = 1080;
        assert_eq!(
            spec.validate(),
            Err(VirtualDisplayError::OddDimensions {
                width: 1921,
                height: 1080
            })
        );

        spec.width = 1920;
        spec.height = 1081;
        assert_eq!(
            spec.validate(),
            Err(VirtualDisplayError::OddDimensions {
                width: 1920,
                height: 1081
            })
        );
    }

    #[test]
    fn spec_validation_rejects_invalid_refresh_and_dpi() {
        let mut spec = valid_spec();
        spec.refresh_hz = 23;
        assert!(matches!(
            spec.validate(),
            Err(VirtualDisplayError::RefreshRateOutOfRange { .. })
        ));

        spec.refresh_hz = 241;
        assert!(matches!(
            spec.validate(),
            Err(VirtualDisplayError::RefreshRateOutOfRange { .. })
        ));

        spec.refresh_hz = 60;
        for bad_scale in [0.49, 4.01, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            spec.dpi_scale = bad_scale;
            assert!(matches!(
                spec.validate(),
                Err(VirtualDisplayError::InvalidDpiScale { .. })
            ));
        }
    }

    #[test]
    fn presets_produce_expected_valid_specs() {
        let cases = [
            (VirtualDisplayPreset::Fhd1080p, 1920, 1080, 1.0),
            (VirtualDisplayPreset::Qhd1440p, 2560, 1440, 1.0),
            (VirtualDisplayPreset::Uhd4k, 3840, 2160, 2.0),
            (VirtualDisplayPreset::Ultrawide21x9, 3440, 1440, 1.0),
            (VirtualDisplayPreset::SuperUltrawide32x9, 5120, 1440, 1.0),
        ];

        for (preset, expected_w, expected_h, expected_scale) in cases {
            assert_eq!(preset.dimensions(), Some((expected_w, expected_h)));
            let spec = preset
                .to_spec(120, DisplayPlacement::LeftOf)
                .expect("fixed preset must build a spec");
            assert_eq!(spec.width, expected_w);
            assert_eq!(spec.height, expected_h);
            assert_eq!(spec.refresh_hz, 120);
            assert_eq!(spec.dpi_scale, expected_scale);
            assert_eq!(spec.placement, DisplayPlacement::LeftOf);
            assert!(spec.validate().is_ok());
        }

        assert_eq!(VirtualDisplayPreset::MatchClient.dimensions(), None);
        assert!(
            VirtualDisplayPreset::MatchClient
                .to_spec(60, DisplayPlacement::RightOf)
                .is_none()
        );
    }

    #[test]
    fn wireless_extend_landscape_and_portrait_alignment() {
        // Portrait Android phone requested in landscape mode -> swaps and aligns even.
        let req = WirelessExtendRequest {
            client_width: 1081,
            client_height: 2401,
            client_dpi: 440,
            client_refresh_hz: 120,
            landscape: true,
        };
        let spec = req.to_virtual_display_spec().unwrap();
        assert_eq!(spec.width, 2400);
        assert_eq!(spec.height, 1080);
        assert_eq!(spec.refresh_hz, 120);
        assert_eq!(spec.dpi_scale, 2.75); // 440 / 160 = 2.75
        assert_eq!(spec.placement, DisplayPlacement::RightOf);
        assert!(spec.validate().is_ok());

        // Same metrics requested in portrait mode -> keeps portrait orientation.
        let portrait_req = WirelessExtendRequest {
            landscape: false,
            ..req
        };
        let portrait_spec = portrait_req.to_virtual_display_spec().unwrap();
        assert_eq!(portrait_spec.width, 1080);
        assert_eq!(portrait_spec.height, 2400);
        assert!(portrait_spec.validate().is_ok());
    }

    #[test]
    fn wireless_extend_clamps_extremes_and_handles_zero_refresh() {
        // Oversized 10K client scales down proportionally within 7680x4320.
        let huge = WirelessExtendRequest {
            client_width: 10_000,
            client_height: 5_000,
            client_dpi: 960,
            client_refresh_hz: 360,
            landscape: true,
        };
        let huge_spec = huge.to_virtual_display_spec().unwrap();
        assert!(huge_spec.width <= MAX_VIRTUAL_DISPLAY_WIDTH);
        assert!(huge_spec.height <= MAX_VIRTUAL_DISPLAY_HEIGHT);
        assert_eq!(huge_spec.width % 2, 0);
        assert_eq!(huge_spec.height % 2, 0);
        assert_eq!(huge_spec.refresh_hz, MAX_VIRTUAL_DISPLAY_REFRESH_HZ);
        assert_eq!(huge_spec.dpi_scale, MAX_VIRTUAL_DISPLAY_DPI_SCALE);
        assert!(huge_spec.validate().is_ok());

        // Tiny client clamps up to minimum valid dimensions and defaults 0 Hz -> 60 Hz.
        let tiny = WirelessExtendRequest {
            client_width: 100,
            client_height: 80,
            client_dpi: 72,
            client_refresh_hz: 0,
            landscape: true,
        };
        let tiny_spec = tiny.to_virtual_display_spec().unwrap();
        assert_eq!(tiny_spec.width, MIN_VIRTUAL_DISPLAY_WIDTH);
        assert_eq!(tiny_spec.height, MIN_VIRTUAL_DISPLAY_HEIGHT);
        assert_eq!(tiny_spec.refresh_hz, DEFAULT_VIRTUAL_DISPLAY_REFRESH_HZ);
        assert_eq!(tiny_spec.dpi_scale, MIN_VIRTUAL_DISPLAY_DPI_SCALE);
        assert!(tiny_spec.validate().is_ok());
    }

    #[test]
    fn wireless_extend_rejects_zero_dimensions_or_dpi() {
        let zero_w = WirelessExtendRequest {
            client_width: 0,
            client_height: 1080,
            client_dpi: 320,
            client_refresh_hz: 60,
            landscape: true,
        };
        assert_eq!(
            zero_w.to_virtual_display_spec(),
            Err(VirtualDisplayError::InvalidClientDimensions {
                width: 0,
                height: 1080
            })
        );

        let zero_dpi = WirelessExtendRequest {
            client_width: 1920,
            client_height: 1080,
            client_dpi: 0,
            client_refresh_hz: 60,
            landscape: true,
        };
        assert_eq!(
            zero_dpi.to_virtual_display_spec(),
            Err(VirtualDisplayError::InvalidClientDpi { dpi: 0 })
        );
    }

    #[test]
    fn bincode_roundtrip_all_commands_and_info() {
        let spec = valid_spec();
        let info = VirtualDisplayInfo {
            id: 7,
            spec: spec.clone(),
            active: true,
            owner_session_id: Some(42),
        };
        let wireless = WirelessExtendRequest {
            client_width: 2560,
            client_height: 1600,
            client_dpi: 320,
            client_refresh_hz: 120,
            landscape: true,
        };

        let commands = [
            VirtualDisplayCommand::Create {
                spec: spec.clone(),
                persist_after_disconnect: true,
            },
            VirtualDisplayCommand::Update {
                id: 7,
                spec: VirtualDisplayPreset::SuperUltrawide32x9
                    .to_spec(144, DisplayPlacement::Above)
                    .unwrap(),
            },
            VirtualDisplayCommand::Destroy { id: 7 },
            VirtualDisplayCommand::List,
            VirtualDisplayCommand::Snapshot {
                displays: vec![
                    info.clone(),
                    VirtualDisplayInfo {
                        id: 8,
                        spec: VirtualDisplayPreset::Uhd4k
                            .to_spec(60, DisplayPlacement::Below)
                            .unwrap(),
                        active: false,
                        owner_session_id: None,
                    },
                ],
            },
            VirtualDisplayCommand::WirelessExtend(wireless),
        ];

        for cmd in &commands {
            assert!(cmd.validate().is_ok());
            let encoded = cmd.encode().expect("bincode serialize command");
            let decoded = VirtualDisplayCommand::decode(&encoded).expect("bincode deserialize");
            assert_eq!(&decoded, cmd);
        }

        // Individual helper roundtrips.
        let spec_bytes = spec.encode().unwrap();
        assert_eq!(VirtualDisplaySpec::decode(&spec_bytes).unwrap(), spec);

        let info_bytes = info.encode().unwrap();
        assert_eq!(VirtualDisplayInfo::decode(&info_bytes).unwrap(), info);

        let wireless_bytes = wireless.encode().unwrap();
        assert_eq!(
            WirelessExtendRequest::decode(&wireless_bytes).unwrap(),
            wireless
        );
    }

    #[test]
    fn snapshot_validation_rejects_oversized_list() {
        let displays = (0..=MAX_SNAPSHOT_DISPLAYS)
            .map(|idx| VirtualDisplayInfo {
                id: idx as u32 + 1,
                spec: valid_spec(),
                active: true,
                owner_session_id: None,
            })
            .collect();
        let cmd = VirtualDisplayCommand::Snapshot { displays };
        assert_eq!(
            cmd.validate(),
            Err(VirtualDisplayError::TooManySnapshotDisplays {
                count: MAX_SNAPSHOT_DISPLAYS + 1,
                max: MAX_SNAPSHOT_DISPLAYS,
            })
        );
    }
}
