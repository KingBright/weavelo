//! Session artwork is requested only for a selected, live source.
use crate::interaction::ApplicationIcon;
use serde::{Deserialize, Serialize};

pub const MAX_ARTWORK_EDGE: u32 = 256;
pub const MAX_ARTWORK_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceBadgePreset {
    Computer,
    MacBook,
    MacStudio,
    Windows,
    Linux,
    Phone,
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
