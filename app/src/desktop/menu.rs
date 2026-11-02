//! Same-user native menu data and fixed UI targets. No media or network owner.
use protocol::session::CaptureSource;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Section {
    Devices,
    Activity,
    Transfers,
    Preferences,
    Sharing,
    Pairing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum DeviceAction {
    View,
    Send,
    Files,
    Details,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum StreamAction {
    Open,
    Pause(bool),
    End,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StreamTarget {
    pub entity: u64,
    pub device: String,
    pub source: Option<CaptureSource>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) enum Call {
    Manager(Section),
    Device {
        id: String,
        action: DeviceAction,
    },
    Stream {
        target: StreamTarget,
        action: StreamAction,
    },
    OpenSend(String),
    StopSend(String),
    StopAllSends,
}
impl Call {
    pub fn valid(&self) -> bool {
        let identifier =
            |s: &str| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control);
        match self {
            Self::Device { id, .. } => identifier(id),
            Self::Stream { target, .. } => target.entity != 0 && identifier(&target.device),
            Self::OpenSend(id) | Self::StopSend(id) => identifier(id) && id.len() <= 128,
            _ => true,
        }
    }
    pub fn can_start_gui(&self) -> bool {
        matches!(self, Self::Manager(_) | Self::Device { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Receiving {
    pub target: StreamTarget,
    pub title: String,
    pub paused: bool,
    pub can_pause: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Sending {
    pub request: String,
    pub title: String,
    pub status: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Snapshot {
    pub ok: bool,
    pub generation: String,
    pub executable_sha256: String,
    pub renderer: String,
    pub sharing: usize,
    pub receiving_count: usize,
    pub receiving: Vec<Receiving>,
    pub sending: Vec<Sending>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stream_actions_keep_entity_device_and_source() {
        let call = Call::Stream {
            target: StreamTarget {
                entity: 73,
                device: "peer-a".into(),
                source: Some(CaptureSource::Window(7)),
            },
            action: StreamAction::End,
        };
        let encoded = serde_json::to_string(&call).unwrap();
        assert_eq!(serde_json::from_str::<Call>(&encoded).unwrap(), call);
        assert!(call.valid());
        assert!(!call.can_start_gui());
        assert!(
            !Call::Device {
                id: "\n".into(),
                action: DeviceAction::Files
            }
            .valid()
        );
        assert!(Call::Manager(Section::Transfers).can_start_gui());
    }
}
