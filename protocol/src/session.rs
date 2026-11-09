//! Versioned connection and independent media subscription control.
use serde::{Deserialize, Serialize};

// Scroll gesture metadata is part of the current input payload. Reject older
// session layouts before admitting input; authentication and device keys stay unchanged.
pub const SESSION_VERSION: u32 = 11;
pub const MAX_SUBSCRIPTIONS: usize = 8;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub enum CaptureSource {
    #[default]
    MainDisplay,
    Display(u32),
    Window(u32),
    Camera(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceCategory { Screen, Window, Camera }

impl CaptureSource {
    pub fn category(self) -> SourceCategory {
        match self {
            Self::MainDisplay | Self::Display(_) => SourceCategory::Screen,
            Self::Window(_) => SourceCategory::Window,
            Self::Camera(_) => SourceCategory::Camera,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CameraFacing { Front, Back, External, Unspecified }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct CameraInfo {
    pub facing: CameraFacing,
    /// Opaque runtime-local group; zero means this camera has no switch group.
    pub switch_group: u64,
    pub rotation: u16,
    pub mirrored: bool,
}

/// The ordered source selection is immutable for the lifetime of a delivery.
pub fn validate_delivery_sources(sources: &[CaptureSource]) -> Result<(), &'static str> {
    if sources.is_empty() || sources.len() > MAX_SUBSCRIPTIONS {
        return Err("Choose between one and eight sources");
    }
    if sources.iter().enumerate().any(|(index, source)| sources[..index].contains(source)) {
        return Err("A delivery cannot contain duplicate sources");
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CaptureSourceInfo {
    pub source: CaptureSource,
    pub title: String,
    pub application: String,
    pub process_id: Option<i32>,
    pub width: u32,
    pub height: u32,
    pub supports_input: bool,
    pub camera: Option<CameraInfo>,
}

/// Android navigation reuses its native secondary/middle button adapter.
/// Touch support alone is not an operating-system or navigation capability.
pub fn android_navigation_button(sources: &[CaptureSourceInfo], source: CaptureSource, action: &str) -> Option<u8> {
    let android = sources.iter().any(|item| item.source == CaptureSource::MainDisplay && item.application == "MediaProjection");
    let selected = sources.iter().find(|item| item.source == source && item.supports_input)?;
    if !android || selected.camera.is_some() { return None; }
    match (action, source) {
        ("back", CaptureSource::MainDisplay | CaptureSource::Window(_)) => Some(1),
        ("home", CaptureSource::MainDisplay) => Some(2),
        _ => None,
    }
}

impl CaptureSourceInfo {
    /// Camera metadata is never permission to inject desktop input.
    pub fn validate_camera(&self) -> Result<(), &'static str> {
        match (self.source, self.camera) {
            (CaptureSource::Camera(id), Some(camera)) if id != 0 && id <= i64::MAX as u64
                && !self.supports_input && self.process_id.is_none()
                && camera.switch_group <= i64::MAX as u64
                && matches!(camera.rotation, 0 | 90 | 180 | 270) => Ok(()),
            (CaptureSource::Camera(_), _) => Err("Invalid camera source metadata"),
            (_, None) => Ok(()),
            (_, Some(_)) => Err("Camera metadata requires a camera source"),
        }
    }
    pub fn camera_switch_target(&self, other: &Self) -> bool {
        self.source != other.source && self.validate_camera().is_ok()
            && other.validate_camera().is_ok()
            && self.camera.zip(other.camera).is_some_and(|(a,b)|
                a.switch_group != 0 && a.switch_group == b.switch_group)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubscriptionRequest {
    pub id: u32,
    pub source: CaptureSource,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub audio: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionCommand {
    ReleaseInput {
        id: u32,
    },
    InputReleased {
        id: u32,
    },
    Configure {
        id: u32,
        revision: u64,
        width: u32,
        height: u32,
        fps: u32,
        bitrate_kbps: u32,
    },
    Configured {
        id: u32,
        revision: u64,
    },
    Closed {
        connection_id: u32,
        reason: String,
    },
    SetClipboard {
        enabled: bool,
        #[serde(default)]
        revision: u64,
    },
    ClipboardState {
        enabled: bool,
        revision: u64,
    },
    Open {
        connection_id: u32,
        version: u32,
    },
    Opened {
        connection_id: u32,
        version: u32,
        max_subscriptions: u32,
        files: bool,
        clipboard: bool,
        window_capture: bool,
    },
    ListSources {
        request_id: u32,
        known_icons: Vec<[u8; 32]>,
    },
    Sources {
        request_id: u32,
        sources: Vec<CaptureSourceInfo>,
        source_icons: Vec<crate::interaction::SourceIconBinding>,
        icons: Vec<crate::interaction::ApplicationIcon>,
    },
    Subscribe(SubscriptionRequest),
    Subscribed {
        id: u32,
        audio_owner: Option<u32>,
        supports_input: bool,
        /// Native simultaneous contacts for this accepted source; zero means mouse/key input.
        max_touch_points: u8,
    },
    Input {
        id: u32,
        event: crate::InputEvent,
    },
    SetActivity {
        id: u32,
        revision: u64,
        video: bool,
        audio: bool,
    },
    Activity {
        id: u32,
        revision: u64,
        video: bool,
        audio: bool,
    },
    Unsubscribe {
        id: u32,
    },
    Unsubscribed {
        id: u32,
    },
    Close {
        connection_id: u32,
    },
    Error {
        request_id: u32,
        reason: String,
    },
    // Appended variants preserve existing serialized enum discriminants.
    // A source switch has its own request identity, separate from media/session IDs.
    SwitchSource {
        id: u32,
        request_id: u32,
        source: CaptureSource,
    },
    SourceSwitched {
        id: u32,
        request_id: u32,
        source: CaptureSource,
        supports_input: bool,
        /// Native simultaneous contacts for this accepted source; zero means mouse/key input.
        max_touch_points: u8,
    },
    // Window input is bound to a source revision. An old datagram cannot act on
    // the newly selected window even when the media subscription ID is reused.
    SourceInput {
        id: u32,
        source_revision: u32,
        event: crate::InputEvent,
    },
    RequestInputBinding { connection_id: u32, id: u32, source_revision: u32, presented_geometry_revision: u64, request: u64 },
    InputBinding { binding: crate::realtime_input::InputBinding, request: u64, host_time_ms: u64 },
    RealtimeInput(crate::realtime_input::RealtimeInput),
    InputHandled { binding: crate::realtime_input::InputBinding, sequence: u64 },
    // Directed delivery uses this same authenticated control connection. The
    // opaque ticket is an intent capability, never a new device identity.
    DeliveryOffer { request_id: u64, source_device_id: String, target_device_id: String, ticket: [u8; 32], revision: u64, sources: Vec<CaptureSource> },
    DeliveryClaim { request_id: u64, target_device_id: String, ticket: [u8; 32], sources: Vec<CaptureSource> },
    DeliveryCancel { request_id: u64 },
    DeliveryReceipt { request_id: u64, revision: u64, state: DeliveryState, reason: String },
    CommitText(crate::interaction::TextCommit),
    TextCommitted { binding: crate::realtime_input::InputBinding, request_id: u64 },
    TextRejected { binding: crate::realtime_input::InputBinding, request_id: u64, reason: String },
    RequestCameraSwitch(CameraSwitchRequest),
    CameraSwitchResult { request: CameraSwitchRequest, ready: Result<(), String> },
    Viewport(crate::viewport::ViewportDemand),
    ViewportAccepted { id: u32, source_revision: u32, revision: u64 },
    GetSourceAppearance { request_id: u32, id: u32, source: CaptureSource },
    // Keep optional artwork off every control message's inline storage. Serde
    // encodes Box transparently, so this changes memory layout only.
    SourceAppearance { request_id: u32, id: u32, source: CaptureSource, appearance: Box<Result<crate::appearance::SourceAppearance, String>> },
    EditorFocus { binding: crate::realtime_input::InputBinding, sequence: u64, editable: bool },
    ProbeEditorFocus { binding: crate::realtime_input::InputBinding, sequence: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryClaimProof { pub ticket:[u8;32], pub target_device_id:String, pub request_id:u64, pub sources:Vec<CaptureSource> }

/// A preflight bound to the current camera view. Ready still requires retiring
/// that subscription and establishing a fresh one; it does not mean pixels changed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CameraSwitchRequest {
    pub id: u32,
    pub request_id: u64,
    pub source_revision: u32,
    pub from: CaptureSource,
    pub target: CaptureSource,
    pub delivery_request_id: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DeliveryState { Pending, Accepted, Claimed, Cancelled, Rejected, Receiving }

/// Fit inside both requested dimensions, preserve source aspect and align 4:2:0
/// output. Rounding is bounded to one pixel per dimension, never stretch to 16:9.
pub fn fit_capture_size(source: (u32, u32), target: (u32, u32)) -> (u32, u32) {
    let (sw, sh) = (source.0.max(2), source.1.max(2));
    let scale = (target.0.max(2) as f64 / sw as f64)
        .min(target.1.max(2) as f64 / sh as f64)
        .min(1.0);
    (
        ((sw as f64 * scale) as u32 & !1).max(2),
        ((sh as f64 * scale) as u32 & !1).max(2),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optional_artwork_preserves_control_size_and_wire_payload() {
        // The ordinary command footprint before optional artwork was 120 bytes
        // on 64-bit targets. Growing it multiplies every async control buffer.
        assert!(std::mem::size_of::<SessionCommand>() <= 120);
        let appearance = Ok(crate::appearance::SourceAppearance {
            application: None,
            device: crate::appearance::DeviceBadge::Preset(crate::appearance::DeviceBadgePreset::Phone),
        });
        let command = SessionCommand::SourceAppearance {
            request_id: 19, id: 7, source: CaptureSource::MainDisplay,
            appearance: Box::new(appearance.clone()),
        };
        let encoded = bincode::serialize(&command).unwrap();
        // Enum tag followed by the original unboxed struct fields, byte-for-byte.
        let old_payload = bincode::serialize(&(19u32, 7u32, CaptureSource::MainDisplay, appearance)).unwrap();
        assert_eq!(&encoded[4..], old_payload);
        let decoded: SessionCommand = bincode::deserialize(&encoded).unwrap();
        assert_eq!(bincode::serialize(&decoded).unwrap(), encoded);
    }
    #[test]
    fn camera_metadata_cannot_grant_desktop_input_or_attach_to_a_screen() {
        let camera=CameraInfo {facing:CameraFacing::Back,switch_group:11,rotation:90,mirrored:false};
        let mut source=CaptureSourceInfo {source:CaptureSource::Camera(23),title:"Back camera".into(),
            application:String::new(),process_id:None,width:1280,height:720,supports_input:false,camera:Some(camera)};
        assert!(source.validate_camera().is_ok());
        source.supports_input=true;assert!(source.validate_camera().is_err());source.supports_input=false;
        source.process_id=Some(42);assert!(source.validate_camera().is_err());source.process_id=None;
        source.camera.as_mut().unwrap().rotation=45;assert!(source.validate_camera().is_err());
        source.camera=Some(camera);source.source=CaptureSource::Display(23);assert!(source.validate_camera().is_err());
        source.source=CaptureSource::Camera(0);assert!(source.validate_camera().is_err());
        source.source=CaptureSource::Camera(23);source.camera=None;assert!(source.validate_camera().is_err());
    }
    #[test]
    fn camera_catalog_round_trip_retains_identity_facing_and_geometry() {
        let source=CaptureSourceInfo {source:CaptureSource::Camera(0x0123456789),title:"Front camera".into(),
            application:String::new(),process_id:None,width:720,height:1280,supports_input:false,
            camera:Some(CameraInfo {facing:CameraFacing::Front,switch_group:9,rotation:270,mirrored:false})};
        let encoded=bincode::serialize(&SessionCommand::sources(18,vec![source.clone()])).unwrap();
        let decoded:SessionCommand=bincode::deserialize(&encoded).unwrap();
        let SessionCommand::Sources {sources,..}=decoded else {panic!("wrong command")};
        assert_eq!(sources,vec![source]);assert_eq!(sources[0].source.category(),SourceCategory::Camera);
    }
    #[test]
    fn delivery_selection_is_bounded_unique_and_required_on_the_wire() {
        assert!(validate_delivery_sources(&[]).is_err());
        assert!(validate_delivery_sources(&vec![CaptureSource::MainDisplay; 2]).is_err());
        let sources: Vec<_> = (1..=8).map(CaptureSource::Window).collect();
        assert!(validate_delivery_sources(&sources).is_ok());
        let mut too_many = sources; too_many.push(CaptureSource::Display(3));
        assert!(validate_delivery_sources(&too_many).is_err());
        let mut old = 27u32.to_le_bytes().to_vec();
        old.extend(bincode::serialize(&(1u64, "a", "b", [1u8; 32], 1u64)).unwrap());
        assert!(bincode::deserialize::<SessionCommand>(&old).is_err());
    }
    #[test]
    fn fits_both_axes_for_portrait_landscape_and_odd_sizes() {
        assert_eq!(fit_capture_size((1080, 1920), (1920, 1080)), (606, 1080));
        assert_eq!(fit_capture_size((3440, 1440), (1280, 720)), (1280, 534));
        assert_eq!(fit_capture_size((913, 913), (1920, 1080)), (912, 912));
        assert_eq!(fit_capture_size((0, 0), (0, 0)), (2, 2));
    }
    #[test]
    fn delivery_commands_round_trip_without_changing_existing_wire_indices(){
        let existing=[(SessionCommand::ReleaseInput{id:7},0u32),(SessionCommand::Open{connection_id:9,version:SESSION_VERSION},7),
            (SessionCommand::Close{connection_id:9},18),(SessionCommand::RequestInputBinding{connection_id:9,id:7,source_revision:0,presented_geometry_revision:1,request:1},23)];
        for(command,index)in existing {let bytes=bincode::serialize(&command).unwrap();assert_eq!(&bytes[..4],&index.to_le_bytes());}
        let commands=[SessionCommand::DeliveryOffer{request_id:17,source_device_id:"source".into(),target_device_id:"target".into(),ticket:[3;32],revision:1,sources:vec![CaptureSource::Display(7),CaptureSource::Window(9)]},
            SessionCommand::DeliveryClaim{request_id:17,target_device_id:"target".into(),ticket:[3;32],sources:vec![CaptureSource::Display(7)]},
            SessionCommand::DeliveryCancel{request_id:17},
            SessionCommand::DeliveryReceipt{request_id:17,revision:2,state:DeliveryState::Receiving,reason:String::new()}];
        for(index,command)in commands.into_iter().enumerate(){
            let bytes=bincode::serialize(&command).unwrap();assert_eq!(&bytes[..4],&(27u32+index as u32).to_le_bytes());
            let decoded:SessionCommand=bincode::deserialize(&bytes).unwrap();assert_eq!(bincode::serialize(&decoded).unwrap(),bytes);
        }
    }

}

impl SessionCommand {
    /// A source directory without application artwork (e.g. a physical display).
    pub fn sources(request_id: u32, sources: Vec<CaptureSourceInfo>) -> Self {
        Self::Sources { request_id, sources, source_icons: Vec::new(), icons: Vec::new() }
    }
}

#[cfg(test)]
mod android_navigation_tests {
    use super::*;
    fn source(source:CaptureSource, application:&str)->CaptureSourceInfo {
        CaptureSourceInfo {source,title:"Source".into(),application:application.into(),process_id:None,
            width:100,height:200,supports_input:true,camera:None}
    }
    #[test]
    fn native_navigation_requires_android_catalog_and_selected_source() {
        let mut catalog=vec![source(CaptureSource::MainDisplay,"Desktop"),source(CaptureSource::Window(7),"app")];
        assert_eq!(android_navigation_button(&catalog,CaptureSource::MainDisplay,"home"),None);
        catalog[0].application="MediaProjection".into();
        assert_eq!(android_navigation_button(&catalog,CaptureSource::MainDisplay,"back"),Some(1));
        assert_eq!(android_navigation_button(&catalog,CaptureSource::MainDisplay,"home"),Some(2));
        assert_eq!(android_navigation_button(&catalog,CaptureSource::Window(7),"back"),Some(1));
        assert_eq!(android_navigation_button(&catalog,CaptureSource::Window(7),"home"),None);
        assert_eq!(android_navigation_button(&catalog,CaptureSource::Window(8),"back"),None);
        assert_eq!(android_navigation_button(&catalog,CaptureSource::MainDisplay,"rotate"),None);
        catalog[0].supports_input=false;
        assert_eq!(android_navigation_button(&catalog,CaptureSource::MainDisplay,"home"),None);
    }
}
