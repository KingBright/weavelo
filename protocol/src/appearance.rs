//! Session artwork is requested only for a selected, live source.
use crate::interaction::ApplicationIcon;
use serde::{Deserialize, Serialize};

pub const MAX_ARTWORK_EDGE: u32 = 256;
pub const MAX_ARTWORK_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceBadgePreset {
    #[default]
    Computer,
    MacBook,
    MacStudio,
    Windows,
    Linux,
    Phone,
}

impl DeviceBadgePreset {
    pub const fn discovery_code(self) -> u8 {
        match self {
            Self::Computer => 0,
            Self::MacBook => 1,
            Self::MacStudio => 2,
            Self::Windows => 3,
            Self::Linux => 4,
            Self::Phone => 5,
        }
    }

    pub const fn from_discovery_code(code: u8) -> Self {
        match code {
            1 => Self::MacBook,
            2 => Self::MacStudio,
            3 => Self::Windows,
            4 => Self::Linux,
            5 => Self::Phone,
            _ => Self::Computer,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceBadge {
    #[default]
    Automatic,
    Preset(DeviceBadgePreset),
    Custom(ApplicationIcon),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceAppearance {
    pub application: Option<ApplicationIcon>,
    pub device: DeviceBadge,
}
