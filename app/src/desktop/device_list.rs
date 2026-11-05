//! Framework-independent device list state and projection for desktop adapters.
use crate::AppDevice;
use remote_core::{discovery::DiscoveryScope, role::RoleState};
use std::net::SocketAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum DeviceFilterKind {
    #[default]
    All,
    Lan,
    P2p,
    Relay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeviceConnectionStatus {
    Connecting,
    Viewing,
    Serving,
    Available,
    Standby,
}

impl DeviceConnectionStatus {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Connecting => "Connecting…",
            Self::Viewing => "Session connected",
            Self::Serving => "Serving",
            Self::Available => "Available",
            Self::Standby => "Standby",
        }
    }

    const fn is_active(self) -> bool {
        matches!(self, Self::Connecting | Self::Viewing | Self::Serving)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeviceRowViewModel {
    pub device_id: String,
    pub display_name: String,
    pub endpoint: SocketAddr,
    pub scope: DiscoveryScope,
    pub online: bool,
    pub connection: DeviceConnectionStatus,
    pub can_connect: bool,
    pub can_open_files: bool,
    pub can_open_workspace: bool,
}

impl DeviceRowViewModel {
    pub(crate) const fn is_active(&self) -> bool {
        self.connection.is_active()
    }
}

/// A projection of existing owners, without a second connection lifecycle.
#[derive(Clone,Default,serde::Serialize)]
pub(crate) struct DeviceActivity {
    pub receiving:usize,
    pub sharing:usize,
    pub connecting:bool,
    pub error:Option<String>,
    pub transfer:Option<(u64,u64)>,
    pub can_stop_sending:bool,
}

/// Small, frontend-only history: closing a failed receive must not erase its reason.
#[derive(Default)]
pub(crate) struct StreamFailures {
    scope: remote_core::shared_files::ShareScope,
    entries: Vec<(String, protocol::session::CaptureSource, String)>,
}
impl StreamFailures {
    pub fn remember(&mut self, scope:remote_core::shared_files::ShareScope, device:String,
        source:protocol::session::CaptureSource, reason:String) {
        if self.scope!=scope {self.entries.clear();self.scope=scope;}
        self.clear(&device,source);
        if self.entries.len()==32 {self.entries.remove(0);}
        self.entries.push((device,source,reason));
    }
    pub fn clear(&mut self, device:&str, source:protocol::session::CaptureSource) {
        self.entries.retain(|(old,capture,_)|old!=device || *capture!=source);
    }
    pub fn retry_device(&mut self, device:&str) {
        self.entries.retain(|(old,_,_)|old!=device);
    }
    pub fn current(&self, scope:remote_core::shared_files::ShareScope) -> impl Iterator<Item=&(String,protocol::session::CaptureSource,String)> {
        self.entries.iter().filter(move |_|self.scope==scope)
    }
}
impl DeviceActivity {
    pub fn label(&self)->String {
        let mut parts=Vec::new();
        if self.connecting {parts.push("Connecting…".into());}
        if self.receiving>0 {parts.push(format!("Receiving · {}",self.receiving));}
        if self.sharing>0 {parts.push(format!("Sharing · {}",self.sharing));}
        if let Some((done,total))=self.transfer {
            parts.push(if total>0 {format!("Files · {}%",done.saturating_mul(100).checked_div(total).unwrap_or(0).min(100))} else {"Transferring files…".into()});
        }
        if self.error.is_some() {parts.push("Needs attention".into());}
        parts.join("  ·  ")
    }
}

pub(crate) struct DeviceListModel<'a> {
    pub devices: &'a [AppDevice],
    pub role: &'a RoleState,
}
impl DeviceListModel<'_> {
    pub(crate) fn project(&self, filter: DeviceFilterKind) -> DeviceListViewModel {
        DeviceListViewModel::project(self.devices, self.role, filter)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct DeviceListViewModel {
    pub rows: Vec<DeviceRowViewModel>,
}

impl DeviceListViewModel {
    pub(crate) fn project(
        devices: &[AppDevice],
        role: &RoleState,
        filter: DeviceFilterKind,
    ) -> Self {
        let active = role.session();
        let rows = devices
            .iter()
            .filter(|device| matches_filter(filter, device.scope))
            .map(|device| {
                let connection = match (role, active) {
                    (RoleState::Connecting(_), Some(session))
                        if session.peer.device_id == device.device_id =>
                    {
                        DeviceConnectionStatus::Connecting
                    }
                    (RoleState::Viewing(_), Some(session))
                        if session.peer.device_id == device.device_id =>
                    {
                        DeviceConnectionStatus::Viewing
                    }
                    (RoleState::Serving(_), Some(session))
                        if session.peer.device_id == device.device_id =>
                    {
                        DeviceConnectionStatus::Serving
                    }
                    _ if device.online => DeviceConnectionStatus::Available,
                    _ => DeviceConnectionStatus::Standby,
                };
                DeviceRowViewModel {
                    device_id: device.device_id.clone(),
                    display_name: device.display_name.clone(),
                    endpoint: device.endpoint,
                    scope: device.scope,
                    online: device.online,
                    connection,
                    can_connect: device.is_streamable()
                        && connection != DeviceConnectionStatus::Connecting,
                    can_open_files: device.online,
                    can_open_workspace: device.online,
                }
            })
            .collect();
        Self { rows }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct DeviceListState {
    pub filter: DeviceFilterKind,
    pub more: Option<String>,
    pub details: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DeviceListAction {
    EndStreams(String),
    StopSending(String),
    ShowStreams(String),
    SelectFilter(DeviceFilterKind),
    Connect(String),
    OpenFiles(String),
    OpenWorkspace(String),
    CopyReceiveCommand(String),
    SendScreen(String),
    More(String),
    Details(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DeviceListEffect {
    EndStreams(String),
    StopSending(String),
    ShowStreams(String),
    ConnectStream(String),
    ConnectFiles(String),
    OpenWorkspace(String),
    CopyReceiveCommand(String),
    SendScreen(String),
}

impl DeviceListState {
    /// Reduce an adapter action to state or an effect. The adapter executes effects
    /// through the existing OriginalOwner; this module starts no tasks or runtimes.
    pub(crate) fn reduce(&mut self, action: DeviceListAction) -> Option<DeviceListEffect> {
        match action {
            DeviceListAction::EndStreams(device)=>Some(DeviceListEffect::EndStreams(device)),
            DeviceListAction::StopSending(device)=>Some(DeviceListEffect::StopSending(device)),
            DeviceListAction::ShowStreams(device)=>Some(DeviceListEffect::ShowStreams(device)),
            DeviceListAction::More(device) => {
                self.more = (self.more.as_ref() != Some(&device)).then_some(device);
                self.details = None;
                None
            }
            DeviceListAction::Details(device) => {
                self.details = (self.details.as_ref() != Some(&device)).then_some(device);
                self.more = None;
                None
            }
            DeviceListAction::SelectFilter(filter) => {
                self.filter = filter;
                None
            }
            DeviceListAction::Connect(device_id) => {
                Some(DeviceListEffect::ConnectStream(device_id))
            }
            DeviceListAction::OpenFiles(device_id) => {
                Some(DeviceListEffect::ConnectFiles(device_id))
            }
            DeviceListAction::SendScreen(device_id) => {
                Some(DeviceListEffect::SendScreen(device_id))
            }
            DeviceListAction::CopyReceiveCommand(device_id) => {
                Some(DeviceListEffect::CopyReceiveCommand(device_id))
            }
            DeviceListAction::OpenWorkspace(device_id) => {
                Some(DeviceListEffect::OpenWorkspace(device_id))
            }
        }
    }
}

/// This prepares a command for the existing remote-host channel; it does not
/// claim to execute a remote action. IDs are data, never shell fragments.
pub(crate) fn receive_command(source: &str, receiver: &str) -> Result<String, &'static str> {
    if [source, receiver].iter().any(|id| {
        id.is_empty()
            || id.len() > 256
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    }) {
        return Err("Device identity cannot be represented in a receive command");
    }
    Ok(format!(
        "\"$HOME/Applications/Weavelo/Weavelo.app/Contents/MacOS/weavelo\" --gui-receive '{source}' 60 --receiver '{receiver}'"
    ))
}

fn matches_filter(filter: DeviceFilterKind, scope: DiscoveryScope) -> bool {
    match filter {
        DeviceFilterKind::All => true,
        DeviceFilterKind::Lan => scope == DiscoveryScope::Lan,
        DeviceFilterKind::P2p => scope == DiscoveryScope::P2p,
        DeviceFilterKind::Relay => scope == DiscoveryScope::Relay,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn failed_receives_survive_close_until_the_same_source_is_retried() {
        use protocol::session::CaptureSource::{MainDisplay,Window};
        let mut history=super::StreamFailures::default();
        let scope=Some([7;32]);
        history.remember(scope,"phone".into(),Window(1),"App launch refused".into());
        history.remember(scope,"phone".into(),MainDisplay,"Capture ended".into());
        history.clear("other-phone",Window(1));
        assert_eq!(history.current(scope).count(),2);
        history.clear("phone",Window(1));
        assert_eq!(history.current(scope).map(|(_,source,_)|*source).collect::<Vec<_>>(),vec![MainDisplay]);
        history.remember(scope,"other-phone".into(),Window(2),"Other device failed".into());
        history.retry_device("phone");
        assert_eq!(history.current(scope).map(|(device,_,_)|device.as_str()).collect::<Vec<_>>(),vec!["other-phone"]);
        assert_eq!(history.current(Some([8;32])).count(),0,"another paired network must not inherit errors");
        history.remember(Some([8;32]),"phone".into(),Window(2),"New network".into());
        assert_eq!(history.current(scope).count(),0);
    }
    use super::*;
    use remote_core::{discovery::DiscoveryScope, role::RoleSession};

    fn device(id: &str, scope: DiscoveryScope, online: bool) -> AppDevice {
        AppDevice {
            device_id: id.into(),
            display_name: format!("Device {id}"),
            endpoint: "127.0.0.1:9000".parse().unwrap(),
            scope,
            can_stream: true,
            can_view: true,
            online,
            last_seen_ms: 0,
        }
    }

    #[test]
    fn names_remain_distinct_and_follow_peer_updates_without_changing_device_identity() {
        let mut devices = [
            device("studio-id", DiscoveryScope::Lan, true),
            device("cube-id", DiscoveryScope::Relay, true),
        ];
        devices[0].display_name = "梁的 Studio".into();
        devices[1].display_name = "cube".into();
        let first = DeviceListViewModel::project(&devices, &RoleState::Idle, DeviceFilterKind::All);
        assert_eq!(first.rows[0].display_name, "梁的 Studio");
        assert_eq!(first.rows[1].display_name, "cube");
        devices[1].online = false;
        devices[1].display_name = "Office cube".into();
        let reconnect =
            DeviceListViewModel::project(&devices, &RoleState::Idle, DeviceFilterKind::All);
        assert_eq!(reconnect.rows[1].device_id, first.rows[1].device_id);
        assert_eq!(reconnect.rows[1].display_name, "Office cube");
        assert_eq!(reconnect.rows[0].display_name, first.rows[0].display_name);
    }

    #[test]
    fn projection_keeps_device_and_connection_status_separate_from_the_adapter() {
        let devices = [
            device("a", DiscoveryScope::Lan, true),
            device("b", DiscoveryScope::Relay, true),
            device("c", DiscoveryScope::Lan, false),
        ];
        let role = RoleState::Connecting(RoleSession::new(devices[0].role_peer(), 7, 0));

        let model = DeviceListViewModel::project(&devices, &role, DeviceFilterKind::Lan);

        assert_eq!(model.rows.len(), 2);
        assert_eq!(model.rows[0].connection, DeviceConnectionStatus::Connecting);
        assert!(!model.rows[0].can_connect);
        assert!(model.rows[0].can_open_files);
        assert_eq!(model.rows[1].connection, DeviceConnectionStatus::Standby);
        assert!(!model.rows[1].can_connect);
        assert!(!model.rows[1].can_open_files);
    }

    #[test]
    fn state_actions_return_effects_without_starting_runtime_work() {
        let mut state = DeviceListState::default();
        assert_eq!(state.reduce(DeviceListAction::More("peer-a".into())), None);
        assert_eq!(state.more.as_deref(), Some("peer-a"));
        assert_eq!(
            state.reduce(DeviceListAction::Details("peer-b".into())),
            None
        );
        assert_eq!(state.details.as_deref(), Some("peer-b"));
        assert!(state.more.is_none());
        assert_eq!(
            state.reduce(DeviceListAction::Details("peer-b".into())),
            None
        );
        assert!(state.details.is_none());
        assert_eq!(
            state.reduce(DeviceListAction::SelectFilter(DeviceFilterKind::Relay)),
            None
        );
        assert_eq!(state.filter, DeviceFilterKind::Relay);
        assert_eq!(
            state.reduce(DeviceListAction::Connect("peer-1".into())),
            Some(DeviceListEffect::ConnectStream("peer-1".into()))
        );
        assert_eq!(
            state.reduce(DeviceListAction::OpenFiles("peer-2".into())),
            Some(DeviceListEffect::ConnectFiles("peer-2".into()))
        );
        assert_eq!(
            state.reduce(DeviceListAction::OpenWorkspace("peer-3".into())),
            Some(DeviceListEffect::OpenWorkspace("peer-3".into()))
        );
    }

    #[test]
    fn switching_device_moves_active_state_without_rebinding_row_identity() {
        let devices = [
            device("a", DiscoveryScope::Lan, true),
            device("b", DiscoveryScope::Relay, true),
        ];
        let viewing_a = RoleState::Viewing(RoleSession::new(devices[0].role_peer(), 7, 0));
        let connecting_b = RoleState::Connecting(RoleSession::new(devices[1].role_peer(), 8, 1));
        let before = DeviceListModel {
            devices: &devices,
            role: &viewing_a,
        }
        .project(DeviceFilterKind::All);
        let after = DeviceListModel {
            devices: &devices,
            role: &connecting_b,
        }
        .project(DeviceFilterKind::All);
        assert_eq!(before.rows[0].connection.label(), "Session connected");
        assert!(
            before.rows[0].can_connect,
            "an active view does not prevent opening its source picker"
        );
        assert!(after.rows[0].can_connect);
        assert!(!after.rows[1].can_connect);
        assert_eq!(after.rows[1].device_id, "b");
        assert_eq!(after.rows[1].endpoint, devices[1].endpoint);
        let filtered = DeviceListModel {
            devices: &devices,
            role: &connecting_b,
        }
        .project(DeviceFilterKind::Lan);
        assert_eq!(filtered.rows.len(), 1);
        assert_eq!(
            filtered.rows[0].connection,
            DeviceConnectionStatus::Available
        );
    }
    #[test]
    fn receive_command_preserves_target_and_rejects_shell_fragments() {
        assert!(receive_command("source; touch /tmp/file", "target").is_err());
        assert!(receive_command("source", "$(other)").is_err());
        assert!(receive_command("source", "").is_err());
        let command = receive_command("source-1", "receiver-2").unwrap();
        assert_eq!(
            command,
            "\"$HOME/Applications/Weavelo/Weavelo.app/Contents/MacOS/weavelo\" --gui-receive 'source-1' 60 --receiver 'receiver-2'"
        );
    }
}
