//! Shared audio output request and local playback status types.
//!
//! These describe host output routing intent separately from captured media
//! frames and their playback on the viewer.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AudioOutputMode {
    /// Keep audio audible on the host while making it available to viewers.
    #[default]
    Mirror,
    /// Make selected host audio private to viewers.
    Private,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AudioOutputScope {
    System,
    Application(u32),
}

impl AudioOutputScope {
    /// Application scopes require a concrete, nonzero process identifier.
    pub const fn is_valid(self) -> bool {
        match self {
            Self::System => true,
            Self::Application(process_id) => process_id != 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalPlayback {
    Enabled,
    Disabled,
    Partial,
    Unknown,
}

/// A viewer's output choice applies only to its current source generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioOutputRequest {
    pub id: u32,
    pub source_revision: u32,
    pub revision: u64,
    pub mode: AudioOutputMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioOutputStatus {
    pub requested: AudioOutputMode,
    pub local_playback: LocalPlayback,
    pub private_supported: bool,
    pub message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_types_roundtrip_through_bincode() {
        let status = AudioOutputStatus {
            requested: AudioOutputMode::Private,
            local_playback: LocalPlayback::Partial,
            private_supported: true,
            message: Some("One application is private".into()),
        };
        let bytes = bincode::serialize(&status).unwrap();
        assert_eq!(
            bincode::deserialize::<AudioOutputStatus>(&bytes).unwrap(),
            status
        );
        assert_eq!(AudioOutputMode::default(), AudioOutputMode::Mirror);
        assert!(AudioOutputScope::Application(42).is_valid());
        assert!(!AudioOutputScope::Application(0).is_valid());
    }
}
