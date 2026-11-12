//! Adapter between the unchanged original product controls and current safe sessions.
//! This owns no discovery/host runtime of its own and does not use legacy StartStream.
use super::model::{OriginalGuiSession, candidate_routes};
use crate::{AppDevice, StreamStartOptions, UnifiedServiceOwner};
use client::{MacDecodedVideoFrame, TransferEntrySnapshot};
use protocol::{
    InputEvent,
    session::{CaptureSource, CaptureSourceInfo, SessionCommand},
};
use remote_core::{
    file_transfer_runtime::FileTransferCommand,
    role::{RolePeer, RoleSession, RoleState},
    session_tabs::{
        Completion, OpenDecision, SessionCandidate, SessionFacts, SessionHealth, SessionPeer,
        SessionTabsState, SessionTabsViewModel,
    },
    shared_files::ShareScope,
    stream_settings::{PendingStreamSettingsState, StreamSettingsValues},
    workspace_session::WorkspaceConnection,
    VideoFrame,
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
    time::{Instant,Duration},
};

type Error = Box<dyn std::error::Error + Send + Sync>;

// Expose relative stage offsets, not the host's absolute capture timestamp.
fn diagnostic_checkpoints(timing: Option<&protocol::FrameTimingCheckpoints>) -> serde_json::Value {
    timing.map_or(serde_json::Value::Null, |t| serde_json::json!({
        "capture_present": t.capture_ts_us != 0,
        "encode_queue_us": t.encode_queue_ts_us,
        "encode_done_us": t.encode_done_ts_us,
        "packetize_us": t.packetize_ts_us,
        "send_us": t.send_ts_us,
        "receive_us": t.recv_ts_us,
        "jitter_enter_us": t.jitter_enter_ts_us,
        "jitter_exit_us": t.jitter_exit_ts_us,
        "decode_enter_us": t.decode_enter_ts_us,
        "decode_done_us": t.decode_done_ts_us,
        "ui_submit_us": t.render_submit_ts_us,
        "render_done_us": t.render_done_ts_us,
    }))
}

fn diagnostic_telemetry(
    stats: &remote_core::SharedHostStats,
    frame: Option<(&protocol::FrameTimingCheckpoints, Instant)>,
) -> serde_json::Value {
    let clock = stats.clock_estimate();
    let now = remote_core::timing::quanta_now_us();
    let host = stats.latest_host_timing();
    let submitted = stats.latest_ui_submission();
    serde_json::json!({
        "clock": clock.map(|c| serde_json::json!({
            "state": format!("{:?}", c.state),
            "sample_count": c.sample_count,
            "selected_sample_count": c.selected_sample_count,
            "query_before_estimate": now < c.client_time_us,
            "selected_sample_age_us": c.sample_age_us.saturating_add(now.saturating_sub(c.client_time_us)),
            "selected_span_us": c.selected_span_us,
            "latest_probe_age_us": now.saturating_sub(c.latest_sample.client_recv_us),
            "latest_probe_rtt_us": c.latest_sample.network_rtt_us,
        })),
        "host_timing": diagnostic_checkpoints(host.as_ref()),
        "texture_timing": diagnostic_checkpoints(frame.map(|(timing, _)| timing)),
        "texture_age_ms": frame.map(|(_, decoded_at)| decoded_at.elapsed().as_millis() as u64),
        "latest_ui_submission": diagnostic_checkpoints(submitted.as_ref()),
    })
}

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum ConnectionPurpose { View, Files, Sources }

pub(crate) const PHONE_NOT_SHARING: &str = "Screen not shared. Start sharing on the remote device.";

#[derive(Clone,Copy)]
struct RetryBinding {
    attempt: u64,
    scope: ShareScope,
    purpose: ConnectionPurpose,
    audio: bool,
    auto_list: bool,
    capture: Option<CaptureSource>,
}
#[derive(Debug)]
struct ConnectionFailure {
    device: String,
    attempt: u64,
    scope: ShareScope,
    message: String,
}
impl std::fmt::Display for ConnectionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.message) }
}
impl std::error::Error for ConnectionFailure {}
fn connection_failure(device: &str, attempt: u64, scope: ShareScope, message: String) -> Error {
    Box::new(ConnectionFailure { device: device.into(), attempt, scope, message })
}

pub(crate) type FrameSlot = Arc<Mutex<Option<Arc<MacDecodedVideoFrame>>>>;
pub(crate) type SourceViewBinding = remote_core::view_commands::ViewConnectionBinding<WorkspaceConnection, ShareScope>;

#[derive(Clone)]
pub(crate) struct ViewportTarget {
    pub binding:SourceViewBinding,
    pub id:u32,
    pub source_revision:u32,
}
impl ViewportTarget {
    pub(crate) fn same_target(&self, other: &Self) -> bool {
        self.id == other.id && self.source_revision == other.source_revision
            && self.binding.same_connection(&other.binding)
    }
}

#[derive(Clone)]
pub(crate) struct FileViewBinding {
    connection: Weak<WorkspaceConnection>,
    scope: ShareScope,
}
fn view_binding_matches<T>(
    connection: &Weak<T>,
    active: &Arc<T>,
    scope: ShareScope,
    current: ShareScope,
) -> bool {
    scope == current
        && connection
            .upgrade()
            .is_some_and(|previous| Arc::ptr_eq(&previous, active))
}


#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ComputerUseFence {
    device: String,
    scope: ShareScope,
    service_epoch: Option<u64>,
    owner_epoch: u64,
    subscription: u32,
    source_revision: u32,
    source: CaptureSource,
    source_started: Instant,
    video_epoch: remote_core::video_recovery::VideoEpoch,
    input: Option<protocol::realtime_input::InputBinding>,
}
pub(crate) type ComputerUseObservation = super::computer_use::Observation<
    WorkspaceConnection, ComputerUseFence, MacDecodedVideoFrame>;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ComputerUseExportError {
    Session(super::computer_use::Rejection),
    Pixels(client::observation_pixels::PixelError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ComputerUseMode {
    Observing,
    Controlling,
    PausedByHuman,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ComputerUseVisualStatus {
    pub mode: Option<ComputerUseMode>,
    pub background_allowed: bool,
    pub cursor_norm: Option<(u16, u16)>,
    pub cursor_geometry: Option<protocol::viewport::FrameGeometry>,
    pub last_action_style: Option<client::observation_pixels::ReticleStyle>,
    pub status_label: Option<String>,
}

struct ComputerUseInput<'a> {
    connection: &'a WorkspaceConnection,
    binding: protocol::realtime_input::InputBinding,
    text_owner: Option<remote_core::workspace_session::TextInputOwner>,
    geometry: Option<protocol::viewport::FrameGeometry>,
    admit: Option<&'a dyn Fn(bool,&mut dyn FnMut()->bool)->bool>,
}
impl super::computer_use::InputPort for ComputerUseInput<'_> {
    fn send(&mut self, event: super::computer_use::Event) -> bool {
        use super::computer_use::Event;
        let event=match (event,self.geometry) {
            (Event::Move{x,y},Some(geometry))=>{let (x,y)=geometry.source_point(x,y);
                let (x,y)=geometry.clamp_source_point((f64::from(x)/65535.,f64::from(y)/65535.));
                Event::Move{x:(x*65535.).round() as u16,y:(y*65535.).round() as u16}},
            (event,_)=>event,
        };
        let mut enqueue=|| match event.clone() {
            Event::Move { x, y } => self
                .connection
                .try_bound_input(self.binding, Some(InputEvent::MouseMoveAbsolute { x, y })),
            Event::Button { button, pressed: true } => self
                .connection
                .try_bound_input(self.binding, Some(InputEvent::MouseDown(button))),
            Event::Button { button, pressed: false } => self
                .connection
                .try_bound_input(self.binding, Some(InputEvent::MouseUp(button))),
            Event::Key { key_code, pressed, modifiers } => self
                .connection
                .try_bound_input(self.binding, Some(InputEvent::Key { key_code, pressed, modifiers })),
            Event::Modifiers { modifiers } => self
                .connection
                .try_bound_input(self.binding, Some(InputEvent::ModifiersChanged(modifiers))),
            Event::Text(text) => {
                let Some(owner) = self.text_owner else {
                    return false;
                };
                self.connection
                    .try_commit_text_for_owner(owner, &text, std::time::Duration::ZERO)
                    .is_ok()
            }
            Event::Scroll { delta_x, delta_y } => self
                .connection
                .try_bound_input(self.binding, Some(InputEvent::scroll(delta_x, delta_y))),
        };
        if let Some(admit)=self.admit{admit(true,&mut enqueue)}else{enqueue()}
    }
    fn release(&mut self) -> bool {
        let mut enqueue=||self.connection.try_bound_input(self.binding,None);
        if let Some(admit)=self.admit{admit(false,&mut enqueue)}else{enqueue()}
    }
}

struct ViewSession {
    purpose: ConnectionPurpose,
    session: OriginalGuiSession,
    frame: FrameSlot,
    source_seen: CaptureSource,
    frame_observer: Option<tokio::task::JoinHandle<()>>,
    sources: Arc<Vec<CaptureSourceInfo>>,
}
impl Drop for ViewSession {
    fn drop(&mut self) {
        if let Some(task) = self.frame_observer.take() {
            task.abort();
        }
        *self.frame.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}
struct DirectedReceiveBinding {
    request_id: String,
    scope: ShareScope,
    source: String,
    expires: Instant,
    cancel: tokio::sync::watch::Sender<()>,
    attempt: Option<u64>,
    connection: Option<u32>,
    claim: Option<protocol::session::DeliveryClaimProof>,
    capture: CaptureSource,
}
struct Pool {
    sessions: Vec<ViewSession>,
    tabs: SessionTabsState,
    pending_settings: PendingStreamSettingsState,
    retry_binding: Option<RetryBinding>,
    message: String,
    receive_dir: PathBuf,
    talkback: Option<(u32, client::TalkbackRuntimeControl)>,
    clipboard_preference: bool,
    directed_receive: Option<DirectedReceiveBinding>,
}
impl Pool {
    fn retire_fileless_connection(&mut self, key: &str) -> Result<(), Error> {
        let Some(index) = self.sessions.iter().position(|entry|
            entry.session.key == key && !entry.session.conn.files_available) else { return Ok(()); };
        if self.sessions[index].session.media.is_some() {
            return Err("File reception changed after this stream connected; reconnect the stream to enable files".into());
        }
        let count = self.sessions.len();
        self.tabs.close_index(index, count);
        self.sessions.remove(index);
        Ok(())
    }

    fn receive_matches(&self, source: &str, request: Option<&str>) -> bool {
        match (&self.directed_receive, request) {
            (None, None) => true,
            (Some(binding), Some(request)) => binding.request_id == request && binding.source == source && binding.scope == remote_core::shared_files::current_share_scope() && Instant::now() < binding.expires,
            _ => false,
        }
    }
    fn receive_failed(&self)->bool {
        self.directed_receive.as_ref().is_some_and(|binding|binding.scope==remote_core::shared_files::current_share_scope()&&Instant::now()<binding.expires&&binding.attempt.is_some_and(|attempt|
            self.tabs.failed().is_some_and(|failed|failed.device_id==binding.source&&failed.attempt==attempt)))
    }
    fn receive_current(&self,request:&str)->bool {
        self.directed_receive.as_ref().is_some_and(|binding|
            binding.request_id==request && binding.scope==remote_core::shared_files::current_share_scope()
            && Instant::now()<binding.expires && !self.receive_failed()
            && binding.connection.is_none_or(|connection|self.sessions.iter().any(|entry|
                entry.session.conn.id==connection && entry.session.connected && entry.session.video_error.is_none())))
    }
    fn cancel_receive(&mut self, request_id: &str) {
        if !self.directed_receive.as_ref().is_some_and(|b| b.request_id == request_id) { return; }
        let binding = self.directed_receive.take().unwrap();
        if let Some(attempt) = binding.attempt {
            self.pending_settings.take(&binding.source, attempt);
            let count = self.sessions.len();
            // complete(false) consumes only this exact pending generation. A
            // newer manual request, even to the same device, remains untouched.
            self.tabs.complete(&binding.source, attempt, false, count);
            if self.tabs.failed().is_some_and(|f| f.device_id == binding.source && f.attempt == attempt) {
                self.tabs.dismiss_failure();
            }
            if self.retry_binding.is_some_and(|r| r.attempt == attempt) { self.retry_binding = None; }
        }
        if let Some(connection) = binding.connection {
            if let Some(index) = self.sessions.iter().position(|s| s.session.conn.id == connection) {
                let active = self.active().is_some_and(|s| s.session.conn.id == connection);
                let count = self.sessions.len();
                self.tabs.close_index(index, count);
                // Session drop sends release-input and closes the exact peer.
                self.sessions.remove(index);
                if active { if let Some((_, talkback)) = self.talkback.take() { talkback.stop(); } }
            }
        }
        if self.sessions.is_empty() && self.tabs.connecting().is_none() && self.tabs.failed().is_none() {
            self.message = "Ready".into();
        }
    }
    fn retry(&self, scope: ShareScope) -> Option<(&remote_core::session_tabs::FailedSession, RetryBinding)> {
        let failed = self.tabs.failed()?;
        let binding = self.retry_binding?;
        (binding.attempt == failed.attempt && binding.scope == scope).then_some((failed, binding))
    }
    fn expire_retry(&mut self, scope: ShareScope) {
        if self.tabs.failed().is_some() && self.retry(scope).is_none() {
            self.tabs.dismiss_failure();
            self.retry_binding = None;
            self.message = "Ready".into();
        }
    }
    fn reconnect_target(&self, scope: ShareScope) -> Option<(String, Option<RetryBinding>)> {
        if let Some((failed, binding)) = self.retry(scope) {
            return Some((failed.device_id.clone(), Some(binding)));
        }
        // Two clicks can be accepted before the first scheduled task starts.
        // Reuse its pending generation rather than reporting No selected device.
        if let Some(connecting) = self.tabs.connecting()
            && let Some(binding) = self.retry_binding
            && binding.attempt == connecting.attempt && binding.scope == scope {
            return Some((connecting.peer.device_id.clone(), Some(binding)));
        }
        self.active().map(|entry| (entry.session.key.clone(), None))
    }
    fn active(&self) -> Option<&ViewSession> {
        let index = self.tabs.active_index(self.sessions.len())?;
        self.sessions.get(index)
    }
    fn active_mut(&mut self) -> Option<&mut ViewSession> {
        let index = self.tabs.active_index(self.sessions.len())?;
        self.sessions.get_mut(index)
    }
    /// Called when the form commits, before yielding to async execution. A handshake
    /// cannot attach with old settings between taking its binding and scheduling work.
    fn prepare_stream_settings_update(
        &mut self,
        values: StreamSettingsValues,
    ) -> Result<Option<Weak<WorkspaceConnection>>, Error> {
        protocol::validate_video_settings(values.width, values.height, values.fps, values.bitrate_kbps)?;
        if let Some(connecting) = self.tabs.connecting() {
            if !self.pending_settings.update(&connecting.peer.device_id, connecting.attempt, values) {
                return Err("Connection settings expired; retry the selected device".into());
            }
            return Ok(None);
        }
        Ok(self.active().map(|entry| Arc::downgrade(&entry.session.conn)))
    }
}

pub(crate) struct OriginalOwner {
    publication_status: Mutex<String>,
    backend: Arc<UnifiedServiceOwner>,
    #[cfg(any(unix, windows))]
    observed_service_epoch: std::sync::atomic::AtomicU64,
    pool: Mutex<Pool>,
    computer_use_epoch: std::sync::atomic::AtomicU64,
    computer_use_background_allowed: std::sync::atomic::AtomicBool,
    computer_use_status: Mutex<ComputerUseVisualStatus>,
    pending_exit_connections: Mutex<Vec<Arc<WorkspaceConnection>>>,
    updates: tokio::sync::watch::Sender<u64>,
    presentation_visible: std::sync::atomic::AtomicBool,
    input_focused: std::sync::atomic::AtomicBool,
}
#[derive(Clone)]
pub(crate) struct ViewSnapshot {
    pub role: RoleState,
    pub route: Option<String>,
    pub devices: Vec<AppDevice>,
    pub sources: Arc<Vec<CaptureSourceInfo>>,
    pub source_catalog_revision: u64,
    pub source_catalog_error: Option<String>,
    pub source_icons: remote_core::application_icons::SourceIconAssets,
    pub source_appearances: remote_core::application_icons::SourceAppearances,
    pub source_binding: Option<SourceViewBinding>,
    pub active_source: CaptureSource,
    pub pending_source: Option<CaptureSource>,
    pub supports_input: bool,
    pub error: Option<String>,
    pub input_failed: bool,
    pub locked: bool,
    pub connection_id: Option<u32>,
    pub source_epoch: Option<Instant>,
    pub message: String,
    pub input_status: String,
    pub input_eligible: bool,
    pub computer_use_status: ComputerUseVisualStatus,
    pub window_resize_notice: Option<Result<(u32, u32), String>>,
}
impl OriginalOwner {
    pub fn new(backend: Arc<UnifiedServiceOwner>) -> Arc<Self> {
        // Retain legacy receive_dir in the migrated JSON, but never adopt its
        // shared destination for a new independent instance.
        let prefs = super::Preferences::default();
        let (updates, _) = tokio::sync::watch::channel(0);
        Arc::new(Self {
            publication_status: Mutex::new(String::new()),
            #[cfg(any(unix, windows))]
            observed_service_epoch: std::sync::atomic::AtomicU64::new(backend.frontend_epoch().map(|e| e.load(std::sync::atomic::Ordering::Acquire)).unwrap_or(0)),
            backend,
            pending_exit_connections: Mutex::new(Vec::new()),
            updates,
            presentation_visible: std::sync::atomic::AtomicBool::new(true),
            input_focused: std::sync::atomic::AtomicBool::new(true),
            computer_use_epoch: std::sync::atomic::AtomicU64::new(0),
            computer_use_background_allowed: std::sync::atomic::AtomicBool::new(false),
            computer_use_status: Mutex::new(ComputerUseVisualStatus::default()),
            pool: Mutex::new(Pool {
                sessions: Vec::new(),
                tabs: SessionTabsState::default(),
                pending_settings: PendingStreamSettingsState::default(),
                retry_binding: None,
                message: "Ready".into(),
                receive_dir: prefs.receive_dir,
                talkback: None,
                clipboard_preference: false,
                directed_receive: None,
            }),
        })
    }
    pub fn publication_status(&self) -> String { self.publication_status.lock().unwrap().clone() }
    pub fn set_publication_status(&self, status: String) {
        *self.publication_status.lock().unwrap() = status;
        self.updates.send_modify(|revision| *revision = revision.wrapping_add(1));
    }
    pub fn subscribe_updates(&self) -> tokio::sync::watch::Receiver<u64> {
        self.updates.subscribe()
    }
    pub fn poll(&self) {
        #[cfg(any(unix, windows))]
        if let Some(epoch) = self.backend.frontend_epoch() {
            let next = epoch.load(std::sync::atomic::Ordering::Acquire);
            if self.observed_service_epoch.swap(next, std::sync::atomic::Ordering::AcqRel) != next { self.close_all(); }
        }
        let Ok(mut pool) = self.pool.try_lock() else {
            return;
        };
        let selected = pool.tabs.selected_index();
        let mut retired: [Option<Arc<MacDecodedVideoFrame>>; 8] = std::array::from_fn(|_| None);
        let transitioning = pool.tabs.connecting().is_some();
        // Revoke the old owner before any new selected session can claim it.
        for (index, entry) in pool.sessions.iter_mut().enumerate() {
            if transitioning || index != selected { entry.session.set_view_selected(false); }
        }
        for (index, entry) in pool.sessions.iter_mut().enumerate() {
            entry
                .session
                .set_presentation_hidden(transitioning || index != selected
                    || !self.presentation_visible.load(std::sync::atomic::Ordering::Acquire));
            entry.session.set_input_focused(!transitioning && index == selected
                && self.input_focused.load(std::sync::atomic::Ordering::Acquire));
            entry.session.set_view_selected(!transitioning && index == selected);
            entry.session.poll();
            let s = &mut entry.session;
            if entry.sources.as_ref() != &s.sources {
                entry.sources = Arc::new(s.sources.clone());
            }
            if entry.frame_observer.is_none()
                && let Some(media) = &s.media
            {
                let signal = media.frame_updates();
                let updates = self.updates.clone();
                entry.frame_observer = Some(tokio::spawn(async move {
                    loop {
                        signal.notified().await;
                        updates.send_modify(|v| *v = v.wrapping_add(1));
                    }
                }));
            }
            if !s.confirmed
                || s.video_error.is_some()
                || !s.connected
                || s.source != entry.source_seen
            {
                *entry.frame.lock().unwrap_or_else(|e| e.into_inner()) = None;
                entry.source_seen = s.source;
            }
            // Observe the exact prior native frame's submission before replacing
            // it with a newer frame. Continuous video must not starve the input
            // gate, nor may stream-wide counters unlock an unpainted new source.
            #[cfg(any(all(target_os = "windows", feature = "native-windows-video"), all(target_os = "linux", feature = "native-linux-video")))]
            if let Some(frame) = s.texture.clone() {
                super::original_presenter::acknowledge_paint(s, &frame);
            }
            if s.confirmed
                && !s.paused
                && !s.presentation_hidden
                && s.video_error.is_none()
                && let Some(media) = &s.media
            {
                let shared = media.shared_frame();
                let next = shared.try_lock().ok().and_then(|mut f| f.take());
                if let Some(mut frame) = next
                    && s.current_video_frame(frame.video_epoch, frame.decoded_at)
                {
                    frame.display_rotation=s.sources.iter().find(|info|info.source==s.source).and_then(|info|info.camera.as_ref()).map_or(0,|camera|camera.rotation);
                    let frame = Arc::new(frame);
                    s.texture = Some(frame.clone());
                    let previous = entry
                        .frame
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .replace(frame);
                    retired[index] = previous;
                }
            }
        }
        let received_source = pool.directed_receive.as_ref().and_then(|binding| {
            pool.sessions.iter().find(|entry| binding.connection == Some(entry.session.conn.id))
                .map(|entry| entry.session.source)
        });
        if let Some(source) = received_source
            && let Some(binding) = pool.directed_receive.as_mut()
        {
            binding.capture = source;
        }
        let selected = pool.tabs.selected_index();
        if pool.sessions.get(selected).is_some_and(|s| {
            !s.session.connected || s.session.paused || s.session.presentation_hidden
        }) && let Some((_, control)) = pool.talkback.take()
        {
            control.stop();
        }
        drop(pool);
        drop(retired);
    }
    pub fn snapshot(&self) -> ViewSnapshot {
        let devices = self.backend.runtime().lock().unwrap().devices();
        let mut computer_use_status = self
            .computer_use_status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        computer_use_status.background_allowed = self
            .computer_use_background_allowed
            .load(std::sync::atomic::Ordering::Acquire);
        let mut p = self.pool.lock().unwrap_or_else(|e| e.into_inner());
        p.expire_retry(remote_core::shared_files::current_share_scope());
        if let Some(connecting) = p.tabs.connecting() {
            return ViewSnapshot {
                role: RoleState::Connecting(RoleSession::new(
                    RolePeer::new(&connecting.peer.device_id, &connecting.peer.name, connecting.peer.endpoint),
                    connecting.attempt as u32, 0,
                )),
                route: None,
                devices,
                sources: Arc::new(Vec::new()),
                source_catalog_revision: 0,
                source_catalog_error: None,
                source_icons: Default::default(),
                source_appearances: Default::default(),
                source_binding: None,
                active_source: CaptureSource::MainDisplay,
                pending_source: None,
                supports_input: false,
                error: None,
                input_failed: false,
                locked: true,
                connection_id: None,
                source_epoch: None,
                message: p.message.clone(),
                input_status: "Connecting to selected device".into(),
                input_eligible: false,
                computer_use_status,
                window_resize_notice: None,
            };
        }
        if let Some(e) = p.active() {
            let s = &e.session;
            let role_session =
                RoleSession::new(RolePeer::new(&s.key, &s.name, s.conn.target), s.conn.id, 0);
            let role = if !s.connected || (s.media.is_none() && s.pending.is_none()
                && s.video_error.is_none() && s.uploaded_at.is_none()) {
                // File-only and closed connections keep Connect Stream
                // available; a new attempt still uses current route/identity checks.
                RoleState::Idle
            } else if s.confirmed && s.media.is_some() {
                RoleState::Viewing(role_session)
            } else {
                RoleState::Connecting(role_session)
            };
            ViewSnapshot {
                role,
                route: s.connected.then(|| s.route.clone()).filter(|r| !r.trim().is_empty()),
                devices,
                sources: e.sources.clone(),
                source_catalog_revision: s.source_catalog_revision,
                source_catalog_error: s.source_catalog_error.clone(),
                source_icons: s.conn.source_icons(),
                source_appearances: s.conn.source_appearances(),
                source_binding: Some(SourceViewBinding::new(&s.conn, remote_core::shared_files::current_share_scope())),
                active_source: s.source,
                pending_source: s.pending_capture_source(),
                supports_input: s.supports_input && s.connected,
                error: active_video_error(s.video_error.as_deref(), s.media.as_ref().and_then(|m|m.decode_status.try_lock().ok().map(|v|v.clone())).as_deref()),
                input_failed: s.input_failed(),
                locked: s.locked,
                connection_id: Some(s.conn.id),
                source_epoch: Some(s.first_frame_after),
                message: s.status.clone(),
                input_status: if s.error.is_empty() {
                    s.input_status().into()
                } else {
                    s.error.clone()
                },
                input_eligible: s.can_input(),
                computer_use_status,
                window_resize_notice: s.window_resize_notice.clone(),
            }
        } else {
            let role = self.backend.runtime().lock().unwrap().role_state().clone();
            ViewSnapshot {
                role,
                route: None,
                devices,
                sources: Arc::new(Vec::new()),
                source_catalog_revision: 0,
                source_catalog_error: None,
                source_icons: Default::default(),
                source_appearances: Default::default(),
                source_binding: None,
                active_source: CaptureSource::MainDisplay,
                pending_source: None,
                supports_input: false,
                error: None,
                input_failed: false,
                locked: true,
                connection_id: None,
                source_epoch: None,
                message: p.message.clone(),
                input_status: "Choose a device".into(),
                input_eligible: false,
                computer_use_status,
                window_resize_notice: None,
            }
        }
    }

    pub(crate) fn network_snapshot(&self) -> crate::network_diagnostics::NetworkSnapshot {
        let mut snapshot = self.backend.network_snapshot();
        if snapshot.gui_projection.is_none() {
            // A standalone GUI projects the owner's runtime directly.
            snapshot.gui_projection = Some(snapshot.owner_projection.clone());
        }
        snapshot
    }
    pub fn can_change_network(&self) -> bool {
        let p = self.pool.lock().unwrap();
        p.tabs.can_change_network(p.sessions.len())
    }
    pub fn session_tabs(&self) -> SessionTabsViewModel {
        let mut p = self.pool.lock().unwrap();
        p.expire_retry(remote_core::shared_files::current_share_scope());
        p.tabs.project(p.sessions.iter().map(|e| SessionFacts {
            connection_id: e.session.conn.id,
            device_id: &e.session.key,
            name: &e.session.name,
            connected: e.session.connected,
            video_confirmed: e.session.confirmed,
            has_media: e.session.media.is_some(),
        }))
    }
    pub(crate) fn device_activity(&self)->std::collections::BTreeMap<String,super::device_list::DeviceActivity> {
        use super::device_list::DeviceActivity;
        let p=self.pool.lock().unwrap();
        let mut result=std::collections::BTreeMap::<String,DeviceActivity>::new();
        for entry in &p.sessions {
            let s=&entry.session;
            let activity=result.entry(s.key.clone()).or_default();
            if s.connected && s.media.is_some() && s.video_error.is_none() {
                if s.confirmed {activity.receiving+=1;} else {activity.connecting=true;}
            }
            if let Some(error)=&s.video_error {activity.error=Some(error.clone());}
            let transfers=s.transfers.lock().unwrap().snapshots();
            for file in transfers.iter().filter(|file|file.is_running()) {
                let progress=activity.transfer.get_or_insert((0u64,0u64));
                progress.0=progress.0.saturating_add(file.transferred_bytes);
                progress.1=progress.1.saturating_add(file.total_bytes);
            }
        }
        if let Some(connecting)=p.tabs.connecting() {result.entry(connecting.peer.device_id.clone()).or_default().connecting=true;}
        if let Some(failed)=p.tabs.failed() {result.entry(failed.device_id.clone()).or_default().error=Some(p.message.clone());}
        result
    }

    pub(crate) fn receiving_streams(&self) -> usize {
        self.pool.lock().unwrap().sessions.iter()
            .filter(|entry| entry.session.connected && entry.session.media.is_some() && entry.session.video_error.is_none())
            .count()
    }
    #[cfg(test)]
    pub fn sessions(&self) -> Vec<(u32, String, bool)> {
        self.session_tabs().tabs.into_iter()
            .map(|tab| (tab.connection_id, tab.label, tab.selected)).collect()
    }
    pub fn select(&self, id: u32) {
        let mut p = self.pool.lock().unwrap();
        self.revoke_computer_use();
        if let Some(index) = p.sessions.iter().position(|e| e.session.conn.id == id) {
            if p.tabs.connecting().is_none() && p.tabs.selected_index() == index { return; }
            if let Some(e) = p.active_mut() {
                e.session.release_input();
                e.session.set_view_selected(false);
            }
            if let Some((_, c)) = p.talkback.take() {
                c.stop();
            }
            let key = p.sessions[index].session.key.clone();
            let count = p.sessions.len();
            p.tabs.select(index, &key, count);
            let visible = self.presentation_visible.load(std::sync::atomic::Ordering::Acquire);
            let focused = self.input_focused.load(std::sync::atomic::Ordering::Acquire);
            let session = &mut p.sessions[index].session;
            session.set_presentation_hidden(!visible);
            session.set_input_focused(focused);
            session.set_view_selected(true);
        }
    }
    pub fn close_session(&self, id: u32) {
        let mut p = self.pool.lock().unwrap();
        self.revoke_computer_use();
        let Some(index) = p.sessions.iter().position(|e| e.session.conn.id == id) else { return; };
        let was_active = p.active().is_some_and(|e| e.session.conn.id == id);
        let count = p.sessions.len();
        if p.tabs.close_index(index, count) {
            // OriginalGuiSession::drop releases input and its media/file resources.
            p.sessions.remove(index);
            if was_active && let Some((_, control)) = p.talkback.take() { control.stop(); }
            if p.sessions.is_empty() && p.tabs.connecting().is_none() && p.tabs.failed().is_none() {
                p.message = "Ready".into();
            }
        }
    }
    pub fn active_connection(&self) -> Option<Arc<WorkspaceConnection>> {
        let p = self.pool.lock().unwrap();
        p.active().map(|e| e.session.conn.clone())
    }
    pub fn frame_binding(&self) -> Option<(u32, FrameSlot)> {
        let p = self.pool.lock().unwrap();
        p.active().map(|e| (e.session.conn.id, e.frame.clone()))
    }
    pub fn stats(&self) -> Option<Arc<remote_core::SharedHostStats>> {
        let p = self.pool.lock().unwrap();
        p.active().map(|e| e.session.host_stats.clone())
    }
    pub fn acknowledge_paint(&self, connection: u32, frame: &Arc<MacDecodedVideoFrame>) {
        let mut p = self.pool.lock().unwrap();
        let Some(e) = p
            .sessions
            .iter_mut()
            .find(|e| e.session.conn.id == connection)
        else {
            return;
        };
        super::original_presenter::acknowledge_paint(&mut e.session, frame);
    }
    pub(crate) fn begin_directed_receive(self: &Arc<Self>, request_id: &str, source: &str, capture: CaptureSource, duration: std::time::Duration) -> Result<tokio::task::JoinHandle<()>, Error> {
        let expires = Instant::now() + duration;
        let mut pool = self.pool.lock().unwrap();
        if pool.directed_receive.is_some() || !pool.tabs.can_change_network(pool.sessions.len()) {
            return Err("Close this window's current sessions before receiving".into());
        }
        pool.directed_receive = Some(DirectedReceiveBinding { request_id: request_id.into(), scope: remote_core::shared_files::current_share_scope(), source: source.into(), expires, cancel: tokio::sync::watch::channel(()).0, attempt: None, connection: None, claim:None, capture });
        drop(pool);
        let owner = Arc::downgrade(self);
        let request = request_id.to_string();
        Ok(tokio::spawn(async move {
            let mut deadline=expires;
            loop {
                tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
                let Some(owner)=owner.upgrade() else {break;};
                let mut p=owner.pool.lock().unwrap();
                let Some(binding)=p.directed_receive.as_ref().filter(|b|b.request_id==request) else {break;};
                if Instant::now()<binding.expires {deadline=binding.expires;continue;}
                p.cancel_receive(&request);break;
            }
        }))
    }
    pub(crate) fn set_receive_claim(&self,request:&str,claim:Option<protocol::session::DeliveryClaimProof>) {
        if let Some(b)=self.pool.lock().unwrap().directed_receive.as_mut().filter(|b|b.request_id==request){b.claim=claim;}
    }
    pub(crate) fn renew_directed_receive(&self,request:&str,duration:std::time::Duration)->bool {
        let mut p=self.pool.lock().unwrap();
        if !p.receive_current(request) {return false;}
        let Some(b)=p.directed_receive.as_mut().filter(|b|b.request_id==request&&b.claim.is_some()&&Instant::now()<b.expires) else{return false;};
        b.expires=Instant::now()+duration;true
    }
    pub(crate) fn cancel_directed_receive(&self, request: &str) {
        self.pool.lock().unwrap().cancel_receive(request);
        let _ = self.updates.send_modify(|revision| *revision = revision.wrapping_add(1));
    }
    pub(crate) fn close_received_window(&self) {
        let mut pool = self.pool.lock().unwrap();
        if let Some(request) = pool.directed_receive.as_ref().map(|binding| binding.request_id.clone()) {
            pool.cancel_receive(&request);
        }
    }
    pub(crate) fn directed_receive_current(&self, request: &str) -> bool {
        self.pool.lock().unwrap().receive_current(request)
    }
    pub(crate) fn directed_receive_error(&self, request: &str) -> Option<String> {
        let pool=self.pool.lock().unwrap();
        let binding=pool.directed_receive.as_ref().filter(|binding|binding.request_id==request
            && binding.scope==remote_core::shared_files::current_share_scope())?;
        if pool.receive_failed() { return Some(pool.message.clone()); }
        let session=&pool.sessions.iter().find(|entry|Some(entry.session.conn.id)==binding.connection)?.session;
        session.video_error.clone().or_else(||(!session.connected && !session.error.is_empty()).then(||session.error.clone()))
    }
    pub(crate) fn directed_receive_source(&self, request: &str) -> Option<CaptureSource> {
        self.pool
            .lock()
            .unwrap()
            .directed_receive
            .as_ref()
            .filter(|b| b.request_id == request)
            .map(|b| b.capture)
    }
    pub(crate) async fn connect_received_device(&self, key: &str, options: StreamStartOptions, request: &str) -> Result<(), Error> {
        self.connect_mode(key, options, ConnectionPurpose::View, true, None, Some(request), true, None).await
    }

    pub async fn connect_device(
        &self,
        key: &str,
        options: StreamStartOptions,
        _now: u64,
    ) -> Result<(), Error> {
        self.connect_mode(key, options, ConnectionPurpose::View, true, None, None, true, None).await
    }
    pub async fn connect_files(&self, key: &str) -> Result<(), Error> {
        self.connect_mode(key, StreamStartOptions::default(), ConnectionPurpose::Files, true, None, None, true, None)
            .await
    }

    pub async fn connect_silent_files(&self, key: &str) -> Result<(), Error> {
        // Command callers admit their own correlated List/Fetch after connecting.
        self.connect_mode(key, StreamStartOptions::default(), ConnectionPurpose::Files, false, None, None, false, None)
            .await
    }
    pub(crate) async fn connect_source_browser(&self,key:&str)->Result<(),Error> {
        self.connect_mode(key,StreamStartOptions::default(),ConnectionPurpose::Sources,false,None,None,false,None).await
    }
    pub(crate) async fn connect_capture_device(&self, key: &str, source: CaptureSource, options: StreamStartOptions) -> Result<(),Error> {
        self.connect_mode(key,options,ConnectionPurpose::View,true,None,None,true,Some(source)).await
    }
    async fn connect_mode(
        &self,
        key: &str,
        options: StreamStartOptions,
        purpose: ConnectionPurpose,
        audio: bool,
        expected_scope: Option<ShareScope>,
        receive: Option<&str>,
        auto_list: bool,
        capture: Option<CaptureSource>,
    ) -> Result<(), Error> {
        protocol::validate_video_settings(
            options.width,
            options.height,
            options.fps,
            options.bitrate_kbps,
        )?;
        let configuration=crate::UnifiedRuntimeConfig::from_env()?;
        let allow_files=self.native_file_transfer_enabled(&configuration)
            && remote_core::session_crypto::paired_file_session_key().is_some();
        let video=purpose==ConnectionPurpose::View;
        if purpose==ConnectionPurpose::Files && !allow_files {return Err("File transfers require the existing enabled preference and an authenticated group".into());}
        let initial_settings = StreamSettingsValues {
            width: options.width, height: options.height,
            fps: options.fps, bitrate_kbps: options.bitrate_kbps,
        };
        let scope = remote_core::shared_files::current_share_scope();
        if expected_scope.is_some_and(|expected| expected != scope) {
            return Err("Device network changed; choose the device again".into());
        }
        let device = self
            .backend
            .runtime()
            .lock()
            .unwrap()
            .devices()
            .into_iter()
            .find(|d| d.device_id == key);
        let routes = self
            .backend
            .discovery_snapshot_rx()
            .map(|rx| candidate_routes(&rx.borrow(), key))
            .unwrap_or_default();
        let rejected = if device.as_ref().is_none_or(|d| !d.online) {
            Some("Device is not currently online")
        } else if routes.is_empty() {
            if purpose == ConnectionPurpose::Sources && device.as_ref().is_some_and(|d|
                !d.can_stream && d.badge == protocol::appearance::DeviceBadgePreset::Phone) {
                Some(PHONE_NOT_SHARING)
            } else {
                Some("No current route to the selected device")
            }
        } else { None };
        if let Some(message) = rejected {
            let mut p = self.pool.lock().unwrap();
            if !p.receive_matches(key, receive) { return Err("Receive lease expired or was superseded".into()); }
            self.revoke_computer_use();
            if let Some(e) = p.active_mut() {
                e.session.release_input(); e.session.set_presentation_hidden(true);
            }
            if let Some((_, control)) = p.talkback.take() { control.stop(); }
            if let Some(old) = p.tabs.pending_attempt(key) { p.pending_settings.take(key, old); }
            let count = p.sessions.len();
            let name = device.as_ref().map(|d| d.display_name.as_str()).unwrap_or(key);
            let attempt = p.tabs.reject_open(key, name, count).ok_or("Connection generation exhausted")?;
            if let Some(binding) = p.directed_receive.as_mut() { binding.attempt = Some(attempt); }
            p.retry_binding = Some(RetryBinding { attempt, scope, purpose, audio, auto_list, capture });
            p.message = message.into();
            return Err(connection_failure(key, attempt, scope, message.into()));
        }
        let device = device.unwrap();
        let (attempt, receive_dir, mut receive_cancel, receive_claim) = {
            let mut p = self.pool.lock().unwrap();
            if !p.receive_matches(key, receive) { return Err("Receive lease expired or was superseded".into()); }
            if purpose == ConnectionPurpose::Files { p.retire_fileless_connection(key)?; }
            self.revoke_computer_use();
            let existing = p.sessions.iter().position(|e| e.session.key == key).map(|index| {
                let s = &p.sessions[index].session;
                SessionCandidate { index, health: SessionHealth {
                    connected: s.connected,
                    peer_responsive: s.conn.peer_is_responsive(),
                    video_failed: s.video_error.is_some(),
                }}
            });
            if existing.is_some_and(|candidate| candidate.health.reusable())
                && let Some(e) = p.active_mut() {
                e.session.release_input();
            }
            let peer = device.role_peer();
            let count = p.sessions.len();
            let plan = p.tabs.plan_open(SessionPeer {
                device_id: peer.device_id, name: peer.display_name, endpoint: peer.endpoint,
            }, existing, count);
            if let Some(index) = plan.retire_index { p.sessions.remove(index); }
            let attempt = match plan.decision {
                OpenDecision::Reuse(index) => {
                    p.retry_binding = None;
                    if let Some((_, c)) = p.talkback.take() {
                        c.stop();
                    }
                    if video && p.sessions[index].session.media.is_none() {
                        p.sessions[index].purpose=ConnectionPurpose::View;
                        let session = &mut p.sessions[index].session;
                        if let Some(source)=capture {session.source=source;session.request.source=source;}
                        session.request.width = options.width;
                        session.request.height = options.height;
                        session.request.fps = options.fps;
                        session.request.bitrate_kbps = options.bitrate_kbps;
                        session.request.audio = audio;
                        session.start_video();
                    }
                    if !video {
                        p.sessions[index].session.list_files_on_connect(auto_list);
                    }
                    if p.tabs.connecting().is_none() {
                        let route = p.sessions[index].session.route.clone();
                        p.message = format!("Connected · {route}");
                    }
                    return Ok(());
                }
                OpenDecision::AlreadyPending => {
                    if let Some(connecting) = p.tabs.connecting().cloned() {
                        p.pending_settings.update(key, connecting.attempt, initial_settings);
                        p.retry_binding = Some(RetryBinding { attempt: connecting.attempt, scope, purpose, audio, auto_list, capture });
                    }
                    let previous = p.tabs.selected_index();
                    if let Some(e) = p.sessions.get_mut(previous) {
                        e.session.release_input();
                        e.session.set_presentation_hidden(true);
                    }
                    p.message = format!("Connecting to {}", device.display_name);
                    return Ok(());
                }
                OpenDecision::AtCapacity => return Err("Close an unused connection before opening another (maximum 8)".into()),
                OpenDecision::GenerationExhausted => return Err("Connection generation exhausted".into()),
                OpenDecision::Start(attempt) => attempt,
            };
            if let Some(binding) = p.directed_receive.as_mut() { binding.attempt = Some(attempt); }
            p.retry_binding = Some(RetryBinding { attempt, scope, purpose, audio, auto_list, capture });
            p.pending_settings.begin(key, attempt, initial_settings);
            let previous = p.tabs.selected_index();
            if let Some(e) = p.sessions.get_mut(previous) {
                e.session.release_input();
                e.session.set_presentation_hidden(true);
            }
            p.message = format!("Connecting to {}", device.display_name);
            let cancel = p.directed_receive.as_ref().map(|binding| (binding.cancel.subscribe(), binding.expires));
            (attempt, p.receive_dir.clone(), cancel, p.directed_receive.as_ref().and_then(|b|b.claim.clone()))
        };
        let mut failures = Vec::new();
        let mut connected = None;
        for (route, endpoint) in routes {
            let connect = async {
                let (conn,mut events)=if allow_files {
                    let profile=remote_core::mesh::default_app_private_mesh_dir();
                    let receive_dir=remote_core::desktop_storage::receive_directory_at(&profile,Some(&receive_dir))?;
                    WorkspaceConnection::connect(endpoint,receive_dir).await?
                } else {WorkspaceConnection::connect_media(endpoint).await?};
                if purpose == ConnectionPurpose::Files && !conn.files_available {
                    return Err("Enable Receive files on the target device, then try again".into());
                }
                if let Some(proof)=&receive_claim {
                    crate::desktop::directed_send::claim(&conn,&mut events,proof).await?;
                }
                Ok::<_,Error>((conn,events))
            };
            let result = if let Some((cancel, deadline)) = &mut receive_cancel {
                tokio::pin!(connect);
                loop {
                    tokio::select! {
                        biased;
                        _ = cancel.changed() => return Err("Receive lease was cancelled".into()),
                        _ = tokio::time::sleep_until(tokio::time::Instant::from_std(*deadline)) => {
                            let p=self.pool.lock().unwrap();
                            if let Some(binding)=p.directed_receive.as_ref().filter(|b|Some(b.request_id.as_str())==receive&&b.expires>Instant::now()) {*deadline=binding.expires;}
                            else {return Err("Receive lease expired".into());}
                        },
                        result = &mut connect => break result,
                    }
                }
            } else { connect.await };
            match result {
                Ok(c) => {
                    connected = Some((route, c));
                    break;
                }
                Err(e) => failures.push(format!("{route}: {e}")),
            }
        }
        if remote_core::shared_files::current_share_scope() != scope {
            connected = None;
            failures.push("Device network changed during connection".into());
        }
        let mut p = self.pool.lock().unwrap();
        if !p.receive_matches(key, receive) || (receive.is_some() && p.directed_receive.as_ref().and_then(|b| b.attempt) != Some(attempt)) {
            if let Some(request) = receive { p.cancel_receive(request); }
            drop(connected);
            return Err("Receive lease expired or was superseded".into());
        }
        let count = p.sessions.len();
        let settings = p.pending_settings.take(key, attempt);
        let completion = p.tabs.complete(key, attempt, connected.is_some() && settings.is_some(), count);
        if completion == Completion::Cancelled {
            drop(connected);
            return Err(connection_failure(key, attempt, scope, "Connection request was cancelled or superseded".into()));
        }
        let selected = completion == Completion::Selected;
        let Some((route, (conn, events))) = connected else {
            let message = format!(
                "Could not connect to {}: {}",
                device.display_name,
                failures.join("; ")
            );
            if selected { p.message = message.clone(); }
            return Err(connection_failure(key, attempt, scope, message));
        };
        let Some(settings) = settings else {
            return Err(connection_failure(key, attempt, scope, "Connection settings expired; retry the selected device".into()));
        };
        let mut session =
            OriginalGuiSession::new(key.into(), device.display_name, route, conn, events);
        session.set_view_selected(false);
        if let Some(proof) = &receive_claim {
            session.source = *proof.sources.first().ok_or("Delivery has no selected source")?;
            session.request.source = session.source;
            session.delivery_request_id = Some(proof.request_id);
        } else if let Some(binding) = p.directed_receive.as_ref().filter(|binding| Some(binding.request_id.as_str()) == receive) {
            session.source = binding.capture;
            session.request.source = binding.capture;
        } else if let Some(source)=capture {
            session.source=source;session.request.source=source;
        }
        #[cfg(any(all(target_os = "windows", feature = "native-windows-video"), all(target_os = "linux", feature = "native-linux-video")))]
        {
            session.prefer_native_video = true;
        }
        session.request.audio = audio;
        session.request.width = settings.width;
        session.request.height = settings.height;
        session.request.fps = settings.fps;
        session.request.bitrate_kbps = settings.bitrate_kbps;
        session.set_clipboard(p.clipboard_preference);
        if receive.is_some() {
            session.attach_consumer(1, remote_core::media_demand::MediaDemand { video, audio });
        }
        if video {
            session.start_video();
        } else {
            session.list_files_on_connect(auto_list);
        }
        let previous = p.tabs.selected_index();
        if selected && let Some(e) = p.sessions.get_mut(previous) {
            e.session.release_input();
            e.session.set_view_selected(false);
        }
        if let Some(binding) = p.directed_receive.as_mut() { binding.connection = Some(session.conn.id); }
        let source_seen = session.source;
        p.sessions.push(ViewSession {
            purpose,
            session,
            frame: Arc::new(Mutex::new(None)),
            source_seen,
            frame_observer: None,
            sources: Arc::new(Vec::new()),
        });
        let count = p.sessions.len();
        p.tabs.attached(completion, count);
        if selected {
            let visible = self.presentation_visible.load(std::sync::atomic::Ordering::Acquire);
            let focused = self.input_focused.load(std::sync::atomic::Ordering::Acquire);
            let mut connected_route = None;
            if let Some(entry) = p.active_mut() {
                entry.session.set_presentation_hidden(!visible);
                entry.session.set_input_focused(focused);
                entry.session.set_view_selected(true);
                connected_route = Some(entry.session.route.clone());
            }
            if let Some(route) = connected_route {
                p.message = format!("Connected · {route}");
            }
        }
        Ok(())
    }
    pub async fn disconnect_active(&self) -> Result<(), Error> {
        let mut p = self.pool.lock().unwrap();
        self.revoke_computer_use();
        if let Some(connecting) = p.tabs.connecting().cloned() {
            p.pending_settings.take(&connecting.peer.device_id, connecting.attempt);
        }
        let count = p.sessions.len();
        if let Some(index) = p.tabs.disconnect(count) {
            p.sessions.remove(index);
        }
        p.retry_binding = None;
        p.message = "Ready".into();
        if let Some((_, c)) = p.talkback.take() {
            c.stop();
        }
        Ok(())
    }
    pub async fn reconnect_active(&self, options: StreamStartOptions) -> Result<(), Error> {
        let (key, mode, capture, purpose) = {
            let p = self.pool.lock().unwrap();
            let (key,mode)=p.reconnect_target(remote_core::shared_files::current_share_scope()).ok_or("No selected device")?;
            let purpose=p.active().map(|entry|if entry.session.media.is_some(){ConnectionPurpose::View}else{entry.purpose}).unwrap_or(ConnectionPurpose::View);
            let capture=p.active().filter(|_|purpose==ConnectionPurpose::View).map(|entry|entry.session.source);
            (key,mode,capture,purpose)
        };
        if let Some(mode) = mode {
            return self.connect_mode(&key, options, mode.purpose, mode.audio, Some(mode.scope), None, mode.auto_list, mode.capture).await;
        }
        self.disconnect_active().await?;
        match purpose {
            ConnectionPurpose::Sources=>self.connect_source_browser(&key).await,
            ConnectionPurpose::Files=>self.connect_files(&key).await,
            ConnectionPurpose::View=>match capture {
                Some(source)=>self.connect_capture_device(&key,source,options).await,
                None=>self.connect_device(&key,options,0).await,
            },
        }
    }
    pub fn clear_failed_connection(&self) {
        let mut p = self.pool.lock().unwrap();
        p.tabs.dismiss_failure(); p.retry_binding = None; p.message = "Ready".into();
    }
    pub fn connection_error_is_current(&self, error: &Error) -> bool {
        let Some(error) = error.downcast_ref::<ConnectionFailure>() else { return true; };
        let p = self.pool.lock().unwrap();
        p.retry(remote_core::shared_files::current_share_scope())
            .is_some_and(|(failed, _)| failed.device_id == error.device
                && failed.attempt == error.attempt && error.scope == remote_core::shared_files::current_share_scope())
    }
    pub async fn request_capture_sources(&self) -> Result<(), Error> {
        let binding = self.source_view_binding().ok_or("No active connection")?;
        self.request_bound_capture_sources(&binding).await
    }
    pub fn source_view_binding(&self) -> Option<SourceViewBinding> {
        let p = self.pool.lock().unwrap();
        p.active().map(|e| SourceViewBinding::new(&e.session.conn, remote_core::shared_files::current_share_scope()))
    }
    pub(crate) fn viewport_target(&self)->Option<ViewportTarget> {
        let pool=self.pool.lock().unwrap();let session=&pool.active()?.session;
        (session.connected&&!matches!(session.source,CaptureSource::Camera(_))).then(||ViewportTarget{
            binding:SourceViewBinding::new(&session.conn,remote_core::shared_files::current_share_scope()),
            id:session.video_id,source_revision:session.source_revision(),
        })
    }
    /// Bind a geometry-free displayed frame only while its slot still belongs
    /// to the active session and the frame belongs to its current video epoch.
    pub(crate) fn viewport_target_for_frame(&self, slot:&FrameSlot, frame:&Arc<MacDecodedVideoFrame>)->Option<ViewportTarget> {
        let pool=self.pool.lock().unwrap();
        let entry=pool.active()?;
        let session=&entry.session;
        if !Arc::ptr_eq(&entry.frame,slot) || !session.current_video_frame(frame.video_epoch,frame.decoded_at)
            || !session.connected || matches!(session.source,CaptureSource::Camera(_)) {return None;}
        Some(ViewportTarget { binding:SourceViewBinding::new(&session.conn,remote_core::shared_files::current_share_scope()),
            id:session.video_id,source_revision:session.source_revision() })
    }
    /// None retires a stale target; false leaves the latest demand queued for retry.
    pub(crate) fn queue_viewport(&self,target:&ViewportTarget,demand:protocol::viewport::ViewportDemand)->Option<bool> {
        if demand.validate().is_err()||demand.id!=target.id||demand.source_revision!=target.source_revision{return None;}
        let mut pool=self.pool.lock().unwrap();let session=&mut pool.active_mut()?.session;
        Self::viewport_matches(session,target).then(||session.control(SessionCommand::Viewport(demand)))
    }
    fn viewport_matches(session:&OriginalGuiSession,target:&ViewportTarget)->bool {
        session.connected&&session.video_id==target.id&&session.source_revision()==target.source_revision
            &&!matches!(session.source,CaptureSource::Camera(_))
            &&target.binding.matches(&session.conn,remote_core::shared_files::current_share_scope())
    }
    pub(crate) fn viewport_target_is_current(&self,target:&ViewportTarget)->bool {
        self.pool.lock().unwrap().active().is_some_and(|entry|Self::viewport_matches(&entry.session,target))
    }
    pub(crate) fn source_view_binding_for_device(&self,key:&str)->Option<SourceViewBinding> {
        let pool=self.pool.lock().unwrap();let entry=pool.active()?;
        (entry.session.key==key && entry.session.connected)
            .then(||SourceViewBinding::new(&entry.session.conn,remote_core::shared_files::current_share_scope()))
    }
    pub(crate) fn validate_source_selection(&self,binding:&SourceViewBinding,sources:&[CaptureSourceInfo])->Result<String,String> {
        protocol::session::validate_delivery_sources(&sources.iter().map(|source|source.source).collect::<Vec<_>>())?;
        let pool=self.pool.lock().unwrap();let entry=pool.active().ok_or("No active connection")?;
        for source in sources {
            remote_core::view_commands::apply_enumerated_source(binding,&entry.session.conn,
                remote_core::shared_files::current_share_scope(),source,&entry.session.sources,|_|())
                .ok_or("The connection or source list changed; choose the sources again")?;
        }
        Ok(entry.session.key.clone())
    }
    pub fn source_binding_is_current(&self, binding: &SourceViewBinding) -> bool {
        let p = self.pool.lock().unwrap();
        p.active().is_some_and(|e| binding.matches(&e.session.conn, remote_core::shared_files::current_share_scope()))
    }
    pub(crate) fn bound_catalog_revision(&self,binding:&SourceViewBinding)->Option<u64> {
        let pool=self.pool.lock().unwrap();let entry=pool.active()?;
        (entry.session.connected && binding.matches(&entry.session.conn,remote_core::shared_files::current_share_scope()))
            .then_some(entry.session.source_catalog_revision)
    }
    pub(crate) fn computer_use_target_binding(&self, device: &str, source: CaptureSource, connection: u32) -> Option<SourceViewBinding> {
        let p=self.pool.lock().ok()?;
        let s=&p.active()?.session;
        (s.key==device && s.source==source && s.conn.id==connection)
            .then(||SourceViewBinding::new(&s.conn,remote_core::shared_files::current_share_scope()))
    }
    pub(crate) fn computer_use_source_revision(observation: &ComputerUseObservation) -> u64 {
        // Wire revisions are nonzero; the session's u32 revision may start at zero.
        u64::from(observation.fence().source_revision)+1
    }
    pub(crate) fn computer_use_snapshot_context(&self) -> Option<super::computer_use_mcp::SessionSnapshotContext> {
        let p = self.pool.lock().ok()?;
        let s = &p.active()?.session;
        let frame = s.texture.as_ref()?;
        let source_title = s
            .sources
            .iter()
            .find(|info| info.source == s.source)
            .map(|info| info.title.clone())
            .unwrap_or_else(|| format!("{:?}", s.source));
        Some(super::computer_use_mcp::SessionSnapshotContext {
            peer_device_id: s.key.clone(),
            peer_name: s.name.clone(),
            connection_id: s.conn.id,
            source: s.source,
            source_title,
            source_revision: u64::from(s.source_revision()) + 1,
            frame_width: frame.width(),
            frame_height: frame.height(),
            can_input: self.computer_use_can_input(s),
            cursor_norm: self
                .computer_use_status
                .lock()
                .ok()
                .and_then(|st| st.cursor_norm),
        })
    }
    pub async fn request_bound_capture_sources(&self, binding: &SourceViewBinding) -> Result<(), Error> {
        let mut p = self.pool.lock().unwrap();
        let e = p.active_mut().ok_or("No active connection")?;
        let connection = e.session.conn.clone();
        binding.apply(&connection, remote_core::shared_files::current_share_scope(), || {
            if e.session.control(SessionCommand::ListSources { request_id: 900_001, known_icons: Vec::new() }) { Ok(()) }
            else { Err("Control queue is busy".into()) }
        }).ok_or("Selected connection changed; source discovery was not sent")?
    }
    pub async fn switch_capture_source(
        &self,
        source: CaptureSource,
        w: u32,
        h: u32,
        fps: u32,
        bitrate: u32,
    ) -> Result<(), Error> {
        let binding = self.source_view_binding().ok_or("No active connection")?;
        self.switch_bound_capture_source(&binding, source, w, h, fps, bitrate).await
    }
    pub async fn switch_bound_capture_source(
        &self, binding: &SourceViewBinding, source: CaptureSource,
        w: u32, h: u32, fps: u32, bitrate: u32,
    ) -> Result<(), Error> {
        protocol::validate_video_settings(w, h, fps, bitrate)?;
        let mut p = self.pool.lock().unwrap();
        let e = p.active_mut().ok_or("No active connection")?;
        let expected = remote_core::view_commands::enumerated_source(&e.session.sources, source)
            .cloned().ok_or("Selected source is missing or ambiguous in the current catalog")?;
        Self::switch_bound_session_source(e, binding, &expected, w, h, fps, bitrate)
    }
    pub async fn switch_bound_enumerated_source(
        &self, binding: &SourceViewBinding, expected: &CaptureSourceInfo,
        w: u32, h: u32, fps: u32, bitrate: u32,
    ) -> Result<(), Error> {
        protocol::validate_video_settings(w, h, fps, bitrate)?;
        let mut p = self.pool.lock().unwrap();
        let e = p.active_mut().ok_or("No active connection")?;
        Self::switch_bound_session_source(e, binding, expected, w, h, fps, bitrate)
    }
    fn switch_bound_session_source(
        e: &mut ViewSession, binding: &SourceViewBinding, expected: &CaptureSourceInfo,
        w: u32, h: u32, fps: u32, bitrate: u32,
    ) -> Result<(), Error> {
        if !e.session.connected { return Err("Selected connection is disconnected".into()); }
        let connection = e.session.conn.clone();
        // The pool lock covers catalog/allocation validation and the mutation.
        let source = remote_core::view_commands::apply_enumerated_source(
            binding, &connection, remote_core::shared_files::current_share_scope(),
            expected, &e.session.sources, |source| source,
        ).ok_or("Selected connection or source catalog changed; source was not switched")?;
        Self::switch_session_source(e, source, w, h, fps, bitrate)
    }
    fn switch_session_source(
        e: &mut ViewSession,
        source: CaptureSource,
        w: u32,
        h: u32,
        fps: u32,
        bitrate: u32,
    ) -> Result<(), Error> {
        if !matches!(source, CaptureSource::MainDisplay)
            && !e.session.sources.iter().any(|s| s.source == source)
        {
            return Err("Selected source is no longer available".into());
        }
        e.session.request.width = w;
        e.session.request.height = h;
        e.session.request.fps = fps;
        e.session.request.bitrate_kbps = bitrate;
        if e.session.media.is_none() {
            // A files-only connection has no video subscription to switch yet.
            // Subscribe directly to the requested window; never flash the desktop.
            e.session.source = source;
            e.session.request.source = source;
            e.session.start_video();
        } else {
            e.session.switch_source(source);
        }
        if !e.session.confirmed {*e.frame.lock().unwrap() = None;}
        Ok(())
    }
    /// None means committed for the pending handshake, or preferences-only while idle.
    /// An established connection returns its exact allocation identity for async work.
    pub fn prepare_stream_settings_update(
        &self,
        values: StreamSettingsValues,
    ) -> Result<Option<Weak<WorkspaceConnection>>, Error> {
        self.pool.lock().unwrap().prepare_stream_settings_update(values)
    }
    pub fn stream_settings_binding_is_current(&self, binding: &Weak<WorkspaceConnection>) -> bool {
        self.pool.lock().unwrap().active()
            .is_some_and(|entry| super::same_connection(binding, &entry.session.conn))
    }
    pub async fn update_stream_settings(
        &self,
        binding: &Weak<WorkspaceConnection>,
        w: u32,
        h: u32,
        fps: u32,
        bitrate: u32,
    ) -> Result<(), Error> {
        protocol::validate_video_settings(w, h, fps, bitrate)?;
        let mut p = self.pool.lock().unwrap();
        let e = p.active_mut()
            .filter(|entry| super::same_connection(binding, &entry.session.conn))
            .ok_or("Selected connection changed; settings were not sent")?;
        // Identity check and source mutation share the same lock. A switch between
        // the UI's check and execution must never send settings to another device.
        if e.session.media.is_some() {
            e.session.configure_stream(w, h, fps, bitrate)?;
            Ok(())
        } else {
            let source = e.session.source;
            Self::switch_session_source(e, source, w, h, fps, bitrate)
        }
    }

    pub fn resize_active_window(
        &self,
        binding: &SourceViewBinding,
        target_size: Option<(u32, u32)>,
    ) -> Result<u32, Error> {
        let mut p = self.pool.lock().unwrap();
        let e = p.active_mut().ok_or("No active connection")?;
        if !e.session.connected {
            return Err("Selected connection is disconnected".into());
        }
        let connection = e.session.conn.clone();
        binding
            .apply(
                &connection,
                remote_core::shared_files::current_share_scope(),
                || e.session.resize_window(target_size),
            )
            .ok_or("Selected connection changed; window resize was not sent")?
            .map_err(Into::into)
    }

    // Call only while holding pool: explicit local user intent cannot race a
    // receipt minted from the state immediately before that intent is applied.
    pub(super) fn revoke_computer_use(&self) {
        use std::sync::atomic::Ordering;
        let _=self.computer_use_epoch.fetch_update(Ordering::AcqRel,Ordering::Acquire,|epoch|Some(epoch.saturating_add(1)));
    }

    pub(crate) fn set_computer_use_background_allowed(&self, allowed: bool) {
        self.computer_use_background_allowed
            .store(allowed, std::sync::atomic::Ordering::Release);
        let _ = self.updates.send_modify(|revision| *revision = revision.wrapping_add(1));
    }

    pub(crate) fn computer_use_background_allowed(&self) -> bool {
        self.computer_use_background_allowed
            .load(std::sync::atomic::Ordering::Acquire)
    }

    pub(crate) fn set_computer_use_active(&self, mode: Option<ComputerUseMode>, label: Option<String>) {
        if let Ok(mut status) = self.computer_use_status.lock() {
            status.mode = mode;
            status.status_label = label;
            if mode.is_none() {
                status.cursor_norm = None;
                status.cursor_geometry = None;
                status.last_action_style = None;
            }
        }
        let _ = self.updates.send_modify(|revision| *revision = revision.wrapping_add(1));
    }

    fn record_computer_use_visual_feedback(&self, action: &super::computer_use::Action,geometry:Option<protocol::viewport::FrameGeometry>) {
        if let Some((coords, style, label)) = action.visual_feedback() {
            if let Ok(mut status) = self.computer_use_status.lock() {
                status.mode = Some(ComputerUseMode::Controlling);
                status.cursor_norm = Some(coords);
                status.cursor_geometry = geometry;
                status.last_action_style = Some(style);
                status.status_label = Some(format!(
                    "AI Controlling: {label} ({}, {}) — Move mouse or press Esc to take over",
                    coords.0, coords.1
                ));
            }
            let _ = self.updates.send_modify(|revision| *revision = revision.wrapping_add(1));
        }
    }

    fn computer_use_can_input(&self, s: &OriginalGuiSession) -> bool {
        if self.computer_use_background_allowed() {
            s.can_computer_use_input()
        } else {
            s.can_input()
        }
    }

    fn computer_use_fence(&self, s: &OriginalGuiSession)
        -> Result<ComputerUseFence, super::computer_use::Rejection> {
        use super::computer_use::Rejection;
        use std::sync::atomic::Ordering;
        let scope = remote_core::shared_files::current_share_scope();
        if scope.is_none() { return Err(Rejection::Unpaired); }
        let owner_epoch = self.computer_use_epoch.load(Ordering::Acquire);
        if owner_epoch == u64::MAX { return Err(Rejection::StaleSession); }
        let mut service_epoch = None;
        #[cfg(any(unix, windows))]
        if let Some(epoch) = self.backend.frontend_epoch() {
            let epoch = epoch.load(Ordering::Acquire);
            if epoch != self.observed_service_epoch.load(Ordering::Acquire) {
                return Err(Rejection::StaleSession);
            }
            service_epoch = Some(epoch);
        }
        if !s.connected || !s.confirmed || s.pending.is_some()
            || s.video_error.is_some() || s.paused || s.presentation_hidden
            || !s.conn.peer_is_responsive() {
            return Err(Rejection::FrameUnavailable);
        }
        let frame = s.texture.as_ref().ok_or(Rejection::FrameUnavailable)?;
        if !s.current_video_frame(frame.video_epoch, frame.decoded_at) {
            return Err(Rejection::FrameUnavailable);
        }
        Ok(ComputerUseFence {
            device: s.key.clone(), scope, service_epoch, owner_epoch,
            subscription: s.video_id, source_revision: s.source_revision(),
            source: s.source, source_started: s.first_frame_after,
            video_epoch: frame.video_epoch,
            input: s.conn.input_binding(s.video_id, s.source_revision()),
        })
    }

    /// Trusted in-process adapter only. The caller must already be authorized
    /// for this device and possess its selected-source binding. Access is an
    /// upper bound, not a permission grant; Control also requires can_input()
    /// (or can_computer_use_input() when background Computer Use is enabled).
    /// Native frame handles are returned unchanged: no CPU readback, new capture
    /// or claim of physical presentation. A wire/export adapter is still needed.
    pub(crate) fn observe_bound_computer_use(
        &self, binding: &SourceViewBinding, expected_device: &str,
        expected_source: CaptureSource, access: super::computer_use::Access,
    ) -> Result<ComputerUseObservation, super::computer_use::Rejection> {
        use super::computer_use::{Access, Observation, Rejection};
        let p = self.pool.lock().unwrap();
        let s = &p.active().ok_or(Rejection::NoSession)?.session;
        let fence = self.computer_use_fence(s)?;
        if s.key != expected_device || s.source != expected_source
            || !binding.matches(&s.conn, fence.scope) {
            return Err(Rejection::StaleSession);
        }
        if access == Access::Control && (!self.computer_use_can_input(s) || fence.input.is_none()) {
            return Err(Rejection::InputNotAllowed);
        }
        if access == Access::Control && (!s.keys.is_empty() || !s.mouse.is_empty() || s.modifiers != 0) {
            return Err(Rejection::InputBusy);
        }
        Ok(Observation::new(&s.conn, fence, s.texture.as_ref().unwrap().clone(),
            access, Instant::now()))
    }

    /// Revalidate the original observation before returning any pixels. This
    /// Native completion runs outside the pool lock so local revoke/selection
    /// stays responsive. Recheck the original receipt after completion and before
    /// returning pixels; a retained frame never preserves authority.
    pub(crate) fn validate_bound_computer_use(
        &self, observation: &mut ComputerUseObservation,
    ) -> Result<(), ComputerUseExportError> {
        use super::computer_use::Rejection;
            let p = self.pool.lock().unwrap();
            let s = &p.active().ok_or_else(|| {
                observation.invalidate();
                ComputerUseExportError::Session(Rejection::NoSession)
            })?.session;
            let fence = self.computer_use_fence(s).map_err(|error| {
                observation.invalidate();
                ComputerUseExportError::Session(error)
            })?;
            observation.validate_current(&s.conn, &fence, Instant::now())
                .map_err(ComputerUseExportError::Session)
    }
    pub(crate) fn export_bound_computer_use(
        &self, observation: &mut ComputerUseObservation, maximum: usize,
    ) -> Result<client::observation_pixels::CpuRgba, ComputerUseExportError> {
        self.validate_bound_computer_use(observation)?;
        let mut pixels = client::desktop_frame::observation_rgba(observation.frame(), maximum)
            .map_err(ComputerUseExportError::Pixels)?;
        self.validate_bound_computer_use(observation)?;
        if let Ok(status) = self.computer_use_status.lock() {
            if let (Some((cx, cy)), Some(style)) = (status.cursor_norm, status.last_action_style) {
                if status.cursor_geometry!=observation.frame().frame_geometry{return Ok(pixels);}
                pixels = pixels.with_reticle(cx, cy, style);
            }
        }
        Ok(pixels)
    }

    /// Crop a full-resolution sub-rectangle `[x0, y0, x1, y1]` (on the `[0, 1000]`
    /// normalized coordinate grid) from the validated observation frame.
    pub(crate) fn zoom_bound_computer_use(
        &self, observation: &mut ComputerUseObservation, region: [u16; 4], maximum: usize,
    ) -> Result<client::observation_pixels::CpuRgba, ComputerUseExportError> {
        self.validate_bound_computer_use(observation)?;
        let full_budget = maximum.max(16 * 1024 * 1024);
        let full = client::desktop_frame::observation_rgba(observation.frame(), full_budget)
            .map_err(ComputerUseExportError::Pixels)?;
        let cropped = full
            .zoom_crop(region, maximum)
            .map_err(ComputerUseExportError::Pixels)?;
        self.validate_bound_computer_use(observation)?;
        Ok(cropped)
    }

    /// Dispatch to the exact observed host grant, under the same session lock.
    /// This does not refocus, unlock, reconnect, start video or refresh a lease.
    /// Success means queued locally, not an acknowledgement of remote effects.
    pub(crate) fn act_bound_computer_use(
        &self, observation: &mut ComputerUseObservation, action: super::computer_use::Action,
        deadline: Instant, cancelled: &impl Fn() -> bool,
    ) -> Result<super::computer_use::Queued, super::computer_use::Rejection> {
        use super::computer_use::Rejection;
        let mut p = self.pool.lock().unwrap();
        // Queue admission may wait behind another owner operation. Enforce the
        // original task/lease deadline after that wait, before the first effect.
        if Instant::now() >= deadline || cancelled() {
            observation.invalidate();
            return Err(Rejection::Expired);
        }
        let s = &mut p.active_mut().ok_or_else(|| {
            observation.invalidate(); Rejection::NoSession
        })?.session;
        // A lease follows the selected source across ROI updates. Effects still
        // require the exact geometry of their own immutable observed frame.
        let geometry=observation.frame().frame_geometry;
        if s.texture.as_ref().is_none_or(|frame|frame.frame_geometry!=geometry) {
            observation.invalidate();return Err(Rejection::StaleSession);
        }
        let fence = self.computer_use_fence(s).inspect_err(|_| observation.invalidate())?;
        if !s.keys.is_empty() || !s.mouse.is_empty() || s.modifiers != 0 {
            observation.invalidate();
            return Err(Rejection::InputBusy);
        }
        let binding = fence.input.ok_or_else(|| {
            observation.invalidate(); Rejection::InputNotAllowed
        })?;
        let text_owner = s.conn.text_input_owner(s.video_id, s.source_revision());
        let can_input = self.computer_use_can_input(s);
        let visual_action = action.clone();
        let connection = s.conn.clone();
        let mut port = ComputerUseInput { connection: &connection, binding, text_owner,geometry,admit:None };
        let result = observation.act(&connection, &fence, Instant::now(), can_input, action, &mut port);
        if result.is_ok() {
            self.record_computer_use_visual_feedback(&visual_action,geometry);
        }
        if result.is_ok() || matches!(result, Err(Rejection::QueueRejected { .. })) {
            // Require a new observation after every attempted effect, including
            // for other receipts captured concurrently from the same picture.
            self.revoke_computer_use();
        }
        if matches!(result, Err(Rejection::QueueRejected { .. })) {
            s.locked = true;
            s.error = "Computer-use input queue rejected; exact-grant release attempted, remote result unknown".into();
        }
        result
    }

    /// Execute a multi-action batch on a single observed frame receipt with
    /// Anthropic `computer_toolset_20260801` cascading abort semantics.
    pub(crate) fn act_batch_bound_computer_use(
        &self,
        observation: &mut ComputerUseObservation,
        actions: &[super::computer_use::Action],
        deadline: Instant,
        cancelled: &impl Fn() -> bool,
    ) -> Result<super::computer_use::BatchOutcome, super::computer_use::Rejection> {
        use super::computer_use::Rejection;
        let mut p = self.pool.lock().unwrap();
        if Instant::now() >= deadline || cancelled() {
            observation.invalidate();
            return Err(Rejection::Expired);
        }
        let s = &mut p.active_mut().ok_or_else(|| {
            observation.invalidate();
            Rejection::NoSession
        })?.session;
        // A lease follows the selected source across ROI updates. Effects still
        // require the exact geometry of their own immutable observed frame.
        let geometry=observation.frame().frame_geometry;
        if s.texture.as_ref().is_none_or(|frame|frame.frame_geometry!=geometry) {
            observation.invalidate();return Err(Rejection::StaleSession);
        }
        let fence = self.computer_use_fence(s).inspect_err(|_| observation.invalidate())?;
        if !s.keys.is_empty() || !s.mouse.is_empty() || s.modifiers != 0 {
            observation.invalidate();
            return Err(Rejection::InputBusy);
        }
        let binding = fence.input.ok_or_else(|| {
            observation.invalidate();
            Rejection::InputNotAllowed
        })?;
        let text_owner = s.conn.text_input_owner(s.video_id, s.source_revision());
        let can_input = self.computer_use_can_input(s);
        let connection = s.conn.clone();
        let view_binding=SourceViewBinding::new(&connection,fence.scope);
        // The input grant remains exact, but waits must not own the session lock:
        // decoding, source changes and human takeover must continue progressing.
        drop(p);
        let valid=|| {
            if Instant::now()>=deadline||cancelled(){return false;}
            let pool=self.pool.lock().unwrap();
            pool.active().is_some_and(|entry|Arc::ptr_eq(&entry.session.conn,&connection)
                &&self.computer_use_can_input(&entry.session)
                &&entry.session.texture.as_ref().is_some_and(|frame|frame.frame_geometry==geometry)
                &&self.computer_use_fence(&entry.session).is_ok_and(|current|current==fence))
        };
        let admit=|effect:bool,enqueue:&mut dyn FnMut()->bool| {
            // Validation and dispatch share this short lock, including after waits.
            let pool=self.pool.lock().unwrap();
            let Some(entry)=pool.active()else{return false;};let session=&entry.session;
            let current=self.computer_use_epoch.load(std::sync::atomic::Ordering::Acquire)==fence.owner_epoch
                &&Arc::ptr_eq(&session.conn,&connection)&&remote_core::shared_files::current_share_scope()==fence.scope
                &&session.conn.input_binding(session.video_id,session.source_revision())==Some(binding);
            if !current{return false;}
            if effect&&(Instant::now()>=deadline||cancelled()||!self.computer_use_can_input(session)
                ||session.texture.as_ref().is_none_or(|frame|frame.frame_geometry!=geometry)
                ||!self.computer_use_fence(session).is_ok_and(|current|current==fence)){return false;}
            enqueue()
        };
        let mut port=ComputerUseInput{connection:&connection,binding,text_owner,geometry,admit:Some(&admit)};
        let mut wait_fn=|action:&super::computer_use::Action|->bool {
            if !valid(){return false;}
            if let super::computer_use::Action::WaitForStability{timeout_ms}=action {
                let budget=(*timeout_ms).min(deadline.saturating_duration_since(Instant::now()).as_millis() as u32);
                return self.wait_for_visual_stability(&view_binding,&fence.device,fence.source,budget,0.002,&||!valid()).1;
            }
            let super::computer_use::Action::Wait{duration_ms}=action else{return false;};
            let target=Instant::now()+Duration::from_millis(u64::from(*duration_ms));
            while Instant::now()<target {
                if !valid(){return false;}
                std::thread::sleep(target.saturating_duration_since(Instant::now()).min(Duration::from_millis(20)));
            }
            valid()
        };
        let outcome = observation.act_batch_with_wait(
            &connection,
            &fence,
            Instant::now(),
            can_input,
            actions,
            &mut port,
            &mut wait_fn,
        )?;
        for (idx, step) in outcome.results.iter().enumerate() {
            if !step.is_error {
                if let Some(act) = actions.get(idx) {
                    self.record_computer_use_visual_feedback(act,geometry);
                }
            }
        }
        self.revoke_computer_use();
        if outcome
            .results
            .iter()
            .any(|step| step.error.as_deref() == Some("outcome_unknown"))
        {
            let mut pool=self.pool.lock().unwrap();
            if let Some(entry)=pool.active_mut().filter(|entry|Arc::ptr_eq(&entry.session.conn,&connection)
                &&entry.session.video_id==fence.subscription&&entry.session.source_revision()==fence.source_revision) {
                entry.session.locked=true;
                entry.session.error="Computer-use batch input queue rejected; exact-grant release attempted, remote result unknown".into();
            }
        }
        Ok(outcome)
    }

    /// Adaptive visual convergence check: samples lightweight 256x256 thumbnail
    /// observations at 40ms intervals outside the pool lock and stops as soon as
    /// consecutive frames differ by `<= threshold_ratio` (e.g. 0.002 = 0.2%).
    pub(crate) fn wait_for_visual_stability(
        &self,
        binding: &SourceViewBinding,
        expected_device: &str,
        expected_source: CaptureSource,
        timeout_ms: u32,
        threshold_ratio: f32,
        cancelled: &impl Fn() -> bool,
    ) -> (u32, bool) {
        use super::computer_use::Access;
        let began = Instant::now();
        let budget = std::time::Duration::from_millis(u64::from(timeout_ms.clamp(40, 5_000)));
        let mut prev_pixels: Option<client::observation_pixels::CpuRgba> = None;
        while began.elapsed() < budget && !cancelled() {
            let current=self.observe_bound_computer_use(binding,expected_device,expected_source,Access::Observe).ok()
                .and_then(|mut observation| {
                    let pixels=client::desktop_frame::observation_thumbnail(observation.frame(),256).ok()?;
                    self.validate_bound_computer_use(&mut observation).ok()?;Some(pixels)
                });
            if let Some(curr) = current {
                if let Some(prev) = &prev_pixels {
                    if let Some(diff) = prev.frame_diff_ratio(&curr) {
                        if diff <= threshold_ratio {
                            return (began.elapsed().as_millis() as u32, true);
                        }
                    }
                }
                prev_pixels = Some(curr);
            }
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        (began.elapsed().as_millis() as u32, false)
    }

    pub(crate) fn text_input_owner(&self) -> Option<(SourceViewBinding,remote_core::workspace_session::TextInputOwner)> {
        let pool=self.pool.lock().unwrap();
        let session=&pool.active()?.session;
        if !session.can_prepare_text_input() {return None;}
        Some((SourceViewBinding::new(&session.conn,remote_core::shared_files::current_share_scope()),
            session.conn.text_input_owner(session.video_id,session.source_revision())?))
    }
    pub(crate) fn commit_viewing_text(&self,view:&SourceViewBinding,binding:remote_core::workspace_session::TextInputOwner,text:&str) -> Result<(),String> {
        let mut pool=self.pool.lock().unwrap();
        let session=&mut pool.active_mut().ok_or("remote connection closed")?.session;
        if !session.can_input() {
            let _ = session.retry_input();
        }
        if !session.can_input() || !view.matches(&session.conn,remote_core::shared_files::current_share_scope())
            || session.conn.text_input_owner(session.video_id,session.source_revision())!=Some(binding) {
            return Err("text input source changed; focus the current remote picture".into());
        }
        let result=session.conn.try_commit_text_for_owner(binding,text,std::time::Duration::ZERO);
        if let Err(error)=&result {session.error=error.clone();}
        result
    }

    pub(crate) fn retry_viewing_input(&self) -> bool {
        let mut pool = self.pool.lock().unwrap();
        self.revoke_computer_use();
        let Some(entry) = pool.active_mut() else { return false; };
        entry.session.retry_input()
    }
    pub(crate) fn focus_viewing_source(&self) -> bool {
        let mut pool = self.pool.lock().unwrap();
        self.revoke_computer_use();
        let Some(entry) = pool.active_mut() else { return false; };
        entry.session.focus_source()
    }
    pub fn queue_viewing_input(&self, event: InputEvent) -> bool {
        self.queue_input_for(None,event)
    }
    pub(crate) fn queue_viewport_input(&self,target:&ViewportTarget,event:InputEvent)->bool {
        self.queue_input_for(Some(target),event)
    }
    pub(crate) fn queue_viewport_button_tap(&self,target:&ViewportTarget,button:u8)->Result<(),String> {
        let mut pool=self.pool.lock().unwrap();
        if pool.active().is_none_or(|entry|!Self::viewport_matches(&entry.session,target)) {
            return Err("Remote navigation source changed".into());
        }
        let session=&pool.active().unwrap().session;
        if !session.mouse.is_empty() || !session.keys.is_empty() || session.modifiers!=0 {
            return Err("Release held keys or pointer before using remote navigation".into());
        }
        self.revoke_computer_use();
        let taking_over=if let Ok(mut status)=self.computer_use_status.lock() {
            if status.mode==Some(ComputerUseMode::Controlling) {
                status.mode=Some(ComputerUseMode::PausedByHuman);
                status.status_label=Some("AI Paused — Human took over input".into());true
            } else {false}
        } else {false};
        let session=&mut pool.active_mut().unwrap().session;
        if taking_over {
            session.release_input();
            return Err("Control returned to you; try navigation again".into());
        }
        if !session.can_input() {return Err("Remote navigation is waiting for current input control".into());}
        let binding=session.conn.input_binding(session.video_id,session.source_revision())
            .ok_or("Remote navigation is waiting for a current input grant")?;
        let result=session.conn.try_bound_button_tap(binding,button,std::time::Duration::ZERO);
        if let Err(error)=&result {session.error=error.clone();}
        result
    }
    fn queue_input_for(&self,target:Option<&ViewportTarget>,event:InputEvent)->bool {
        let mut p = self.pool.lock().unwrap();
        if target.is_some_and(|target|p.active().is_none_or(|entry|!Self::viewport_matches(&entry.session,target))){return false;}
        self.revoke_computer_use();
        let mut taking_over=false;
        if let Ok(mut status) = self.computer_use_status.lock() {
            if status.mode == Some(ComputerUseMode::Controlling) {
                taking_over=true;
                status.mode = Some(ComputerUseMode::PausedByHuman);
                status.status_label = Some("AI Paused — Human took over input".into());
            }
        }
        let Some(e) = p.active_mut() else {
            return false;
        };
        let s = &mut e.session;
        if taking_over{s.release_input();}
        if !s.can_input() {
            return false;
        }
        match &event {
            InputEvent::MouseDown(id) if s.mouse.contains(id) => return false,
            InputEvent::MouseUp(id) if !s.mouse.contains(id) => return false,
            InputEvent::Key { key_code, pressed: true, .. } if s.keys.contains(key_code) => return false,
            InputEvent::Key {
                key_code,
                pressed: false,
                ..
            } if !s.keys.contains(key_code) => return false,
            // Unchanged modifier snapshots must not split adjacent committed
            // text into separate round trips. Real transitions remain ordered.
            InputEvent::ModifiersChanged(modifiers) if s.modifiers == *modifiers => return false,
            _ => {}
        }
        let command = s.input_command(event.clone());
        if !s.control(command) {
            s.release_input();
            return false;
        }
        match event {
            InputEvent::MouseDown(b) => {
                s.mouse.insert(b);
            }
            InputEvent::MouseUp(b) => {
                s.mouse.remove(&b);
            }
            InputEvent::Key {
                key_code, pressed, ..
            } => {
                if pressed {
                    s.keys.insert(key_code);
                } else {
                    s.keys.remove(&key_code);
                }
            }
            InputEvent::ModifiersChanged(m) => s.modifiers = m,
            _ => {}
        }
        true
    }

    pub(crate) fn accepts_frame_source_revision(&self,revision:u32)->bool {
        self.pool.lock().unwrap().active().is_some_and(|entry|entry.session.source_revision()==revision)
    }
    pub fn set_input_locked(&self, locked: bool) {
        let mut p = self.pool.lock().unwrap();
        self.revoke_computer_use();

        if let Some(e) = p.active_mut() {
            e.session.set_input_locked(locked);
        }
    }
    pub fn release_input(&self) {
        let mut p = self.pool.lock().unwrap();
        self.revoke_computer_use();

        if let Some(e) = p.active_mut() {
            e.session.release_input();
        }
    }
    pub fn set_view_demand(&self, visible: bool, focused: bool) {
        let mut p = self.pool.lock().unwrap();
        let was_visible=self.presentation_visible.swap(visible,std::sync::atomic::Ordering::AcqRel);
        let was_focused=self.input_focused.swap(focused,std::sync::atomic::Ordering::AcqRel);
        if (was_visible&&!visible)||(was_focused&&!focused&&!self.computer_use_background_allowed()) {
            self.revoke_computer_use();
        }
        if let Some(e) = p.active_mut() {
            if !visible || !focused { e.session.set_input_focused(false); }
            e.session.set_presentation_hidden(!visible);
            e.session.set_input_focused(focused);
        }
    }
    pub fn set_media_paused(&self, paused: bool) {
        let mut p = self.pool.lock().unwrap();
        if let Some(e) = p.active_mut() && e.session.paused != paused {
            self.revoke_computer_use();
            e.session.set_paused(paused);
        }
    }
    pub fn supports_clipboard_sync(&self) -> bool {
        let p = self.pool.lock().unwrap();
        p.active()
            .is_some_and(|e| e.session.conn.clipboard_available)
    }
    pub fn clipboard_sync_enabled(&self) -> bool {
        let p = self.pool.lock().unwrap();
        p.active().is_some_and(|e| e.session.clipboard_enabled)
    }
    pub fn set_clipboard_sync_enabled(&self, enabled: bool) -> bool {
        let mut p = self.pool.lock().unwrap();
        p.clipboard_preference = enabled;
        for e in &mut p.sessions { e.session.set_clipboard(enabled); }
        p.active().is_some_and(|e| e.session.conn.clipboard_available)
    }
    pub fn force_resync_clipboard(&self) {
        let mut p = self.pool.lock().unwrap();
        p.clipboard_preference = true;
        if let Some(e) = p.active_mut() {
            e.session.set_clipboard(false);
            e.session.set_clipboard(true);
        }
    }
    pub fn supports_file_transfer(&self) -> bool {
        let p = self.pool.lock().unwrap();
        p.active().is_some_and(|e| e.session.conn.files_available)
    }
    pub fn file_transfer_enabled(&self) -> bool {
        self.backend.file_transfer_enabled()
    }
    pub(crate) fn native_file_transfer_enabled(&self, configuration: &crate::UnifiedRuntimeConfig) -> bool {
        // An attached frontend owns outgoing native sessions, not the legacy
        // side-service controller. Its absence is not a user preference.
        // Keep the effective persisted/environment preference and the original
        // peer connection's files_available check; never create another owner.
        #[cfg(any(unix, windows))]
        if self.backend.frontend_epoch().is_some() {
            return configuration.enable_file_transfer;
        }
        self.backend.file_transfer_enabled()
    }
    pub fn set_file_transfer_enabled(&self, enabled: bool) -> bool {
        self.backend.set_file_transfer_enabled(enabled)
    }
    pub fn supports_talkback(&self) -> bool {
        cfg!(target_os = "macos") && self.active_connection().is_some()
    }
    pub fn talkback_enabled(&self) -> bool {
        self.pool.lock().unwrap().talkback.is_some()
    }
    pub fn set_talkback_enabled(&self, enabled: bool) -> bool {
        let mut p = self.pool.lock().unwrap();
        if let Some((_, c)) = p.talkback.take() {
            c.stop();
        }
        if !enabled {
            return true;
        }
        if !cfg!(target_os = "macos") {
            return false;
        }
        let Some(e) = p.active() else {
            return false;
        };
        if !e.session.confirmed || e.session.paused {
            return false;
        }
        let c = client::start_talkback_runtime_control(e.session.conn.sender.clone());
        c.start(e.session.conn.target, e.session.video_id);
        p.talkback = Some((e.session.conn.id, c));
        true
    }
    pub fn set_audio(&self, volume: u8, muted: bool) {
        let mut p = self.pool.lock().unwrap();

        if let Some(e) = p.active_mut() {
            e.session.volume = volume.min(100);
            e.session.muted = muted;
            e.session.audio_settings();
        }
    }
    pub fn media_state(&self) -> (bool, bool) {
        let p = self.pool.lock().unwrap();
        p.active()
            .map(|e| {
                (
                    e.session.paused || e.session.presentation_hidden,
                    e.session.media_activity_pending(),
                )
            })
            .unwrap_or((false, false))
    }
    pub fn audio(&self) -> (u8, bool) {
        let p = self.pool.lock().unwrap();
        p.active()
            .map(|e| (e.session.volume, e.session.muted))
            .unwrap_or((100, false))
    }
    pub(crate) fn host_audio(&self) -> Option<(bool, Option<protocol::audio_output::AudioOutputStatus>)> {
        self.pool.lock().unwrap().active().and_then(|entry| entry.session.host_audio())
    }
    pub(crate) fn set_host_audio(&self, mode: protocol::audio_output::AudioOutputMode) {
        if let Some(entry) = self.pool.lock().unwrap().active_mut() {
            entry.session.set_host_audio(mode);
        }
    }
    pub fn file_transfer_snapshot(&self) -> Vec<TransferEntrySnapshot> {
        let p = self.pool.lock().unwrap();
        p.active()
            .map(|e| e.session.transfers.lock().unwrap().snapshots())
            .unwrap_or_default()
    }
    pub fn clear_finished_transfers(&self) {
        let p = self.pool.lock().unwrap();
        if let Some(e) = p.active() {
            e.session.transfers.lock().unwrap().compact_history(0);
        }
    }
    pub fn recent_received_files(&self, limit: usize) -> Vec<(String, PathBuf, u64)> {
        let dir = self.receive_dir();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut items: Vec<(std::time::SystemTime, String, PathBuf, u64)> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    return None;
                }
                let meta = std::fs::symlink_metadata(&path).ok()?;
                if !meta.is_file() {
                    return None;
                }
                let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                Some((modified, name, path, meta.len()))
            })
            .collect();
        items.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        items
            .into_iter()
            .take(limit)
            .map(|(_, name, path, size)| (name, path, size))
            .collect()
    }
    pub fn receive_dir(&self) -> PathBuf {
        self.pool.lock().unwrap().receive_dir.clone()
    }
    pub fn set_file_receive_dir(&self, path: PathBuf) {
        let path=match remote_core::desktop_storage::receive_directory_at(&remote_core::mesh::default_app_private_mesh_dir(),Some(&path)) {
            Ok(path)=>path,
            Err(error)=>{self.pool.lock().unwrap().message=format!("Receive directory unchanged: {error}");return;}
        };
        let mut p = self.pool.lock().unwrap();
        p.receive_dir = path.clone();
        for e in &mut p.sessions {
            e.session
                .files_command(FileTransferCommand::SetReceiveConfig {
                    receive_dir: path.clone(),
                    receive_policy: remote_core::file_transfer::FileReceivePolicy::default(),
                });
        }
    }
    pub fn file_snapshot(
        &self,
    ) -> (
        Vec<protocol::shared_files::SharedFileInfo>,
        Option<u64>,
        Option<u64>,
        bool,
    ) {
        let p = self.pool.lock().unwrap();
        p.active()
            .map(|e| {
                (
                    e.session.files.clone(),
                    e.session.file_history.last().copied(),
                    e.session.next_file_page,
                    e.session.file_pending,
                )
            })
            .unwrap_or_default()
    }
    pub(crate) fn file_view_binding(&self) -> Option<FileViewBinding> {
        self.active_connection().map(|connection| FileViewBinding {
            connection: Arc::downgrade(&connection),
            scope: remote_core::shared_files::current_share_scope(),
        })
    }
    pub(crate) fn list_bound_files(&self, binding: &Option<FileViewBinding>, after: u64) {
        self.with_bound_file_session(binding, |s| {
            s.release_input();
            s.list_files(after);
        });
    }
    pub(crate) fn fetch_bound_file(&self, binding: &Option<FileViewBinding>, id: u64) {
        self.with_bound_file_session(binding, |s| s.fetch_file(id));
    }
    pub(crate) fn cancel_bound_transfer(
        &self,
        binding: &Option<FileViewBinding>,
        target: client::TransferCancelTarget,
    ) {
        self.with_bound_file_session(binding, |s| {
            let command = match target {
                client::TransferCancelTarget::Transfer(transfer_id) => {
                    FileTransferCommand::CancelTransfer { transfer_id }
                }
                client::TransferCancelTarget::Group(group_id) => {
                    FileTransferCommand::CancelGroup { group_id }
                }
            };
            s.files_command(command);
        });
    }
    fn with_bound_file_session(
        &self,
        binding: &Option<FileViewBinding>,
        action: impl FnOnce(&mut OriginalGuiSession),
    ) {
        let Some(binding) = binding else {
            return;
        };
        let mut pool = self.pool.lock().unwrap();
        if let Some(entry) = pool.active_mut()
            && entry.session.connected
            && view_binding_matches(
                &binding.connection,
                &entry.session.conn,
                binding.scope,
                remote_core::shared_files::current_share_scope(),
            )
        {
            action(&mut entry.session);
        }
    }

    pub(crate) fn cli_file_connection(
        &self, key: &str, scope: ShareScope,
    ) -> Result<(Weak<WorkspaceConnection>, Arc<Mutex<client::TransferCenterState>>), Error> {
        if scope != remote_core::shared_files::current_share_scope() {
            return Err("Device network changed before sending files".into());
        }
        let p = self.pool.lock().unwrap();
        let session = &p.sessions.iter().find(|e| e.session.key == key && e.session.connected
            && e.session.conn.files_available).ok_or("Original file connection is unavailable")?.session;
        Ok((Arc::downgrade(&session.conn), session.transfers.clone()))
    }

    pub(crate) fn cli_shared_request(
        &self, target: &Weak<WorkspaceConnection>, scope: ShareScope,
        request: protocol::shared_files::SharedFileRequest,
    ) -> Result<(u64, tokio::sync::broadcast::Receiver<remote_core::file_transfer_runtime::FileTransferEvent>), Error> {
        if scope.is_none() || scope != remote_core::shared_files::current_share_scope()
            || !self.native_file_transfer_enabled(&crate::UnifiedRuntimeConfig::from_env()?) {
            return Err("Files require the existing enabled setting and paired device group".into());
        }
        let mut pool = self.pool.lock().unwrap();
        let session = &mut pool.sessions.iter_mut()
            .find(|e| e.session.connected && super::same_connection(target, &e.session.conn))
            .ok_or("Original file connection changed")?.session;
        Ok(session.cli_shared_request(request)?)
    }

    pub fn send_files_to(
        &self,
        target: &Weak<WorkspaceConnection>,
        scope: ShareScope,
        paths: Vec<PathBuf>,
    ) {
        if scope.is_none() || scope != remote_core::shared_files::current_share_scope()
            || !crate::UnifiedRuntimeConfig::from_env().is_ok_and(|c|self.native_file_transfer_enabled(&c)) {
            return;
        }
        let mut p = self.pool.lock().unwrap();
        if let Some(e) = p
            .sessions
            .iter_mut()
            .find(|e| super::same_connection(target, &e.session.conn) && e.session.connected)
        {
            if paths.len() == 1 {
                e.session.files_command(FileTransferCommand::SendFile {
                    path: paths.into_iter().next().unwrap(),
                    mime_type: None,
                });
            } else if !paths.is_empty() {
                e.session.files_command(FileTransferCommand::SendFileGroup {
                    files: paths
                        .into_iter()
                        .map(
                            |path| remote_core::file_transfer_runtime::FileTransferGroupFile {
                                path,
                                mime_type: None,
                            },
                        )
                        .collect(),
                });
            }
        }
    }
    pub(crate) fn receive_snapshot(&self, request: &str) -> serde_json::Value {
        let pool=self.pool.lock().unwrap();
        let binding=pool.directed_receive.as_ref().filter(|binding|binding.request_id==request);
        serde_json::json!({"pending":pool.tabs.pending_count(),"directed_receive_failed":binding.is_none()||pool.receive_failed(),
            "sessions":pool.sessions.iter().filter(|entry|binding.is_some_and(|binding|
                binding.connection==Some(entry.session.conn.id) && binding.source==entry.session.key))
                .map(|entry| {let session=&entry.session;serde_json::json!({
                    "device_id":session.key,"source_id":session.source,"connected":session.connected,
                    "decoded_frames":session.stats.video_frames_decoded.load(std::sync::atomic::Ordering::Relaxed),
                    "scene_submitted_frame":session.uploaded_at.is_some(),"video_error":session.video_error,
                })}).collect::<Vec<_>>()})
    }
    pub fn diagnostic_snapshot(&self) -> serde_json::Value {
        let p = self.pool.lock().unwrap();
        serde_json::json!({"pending":p.tabs.pending_count(),"message":p.message,"directed_receive_failed":p.receive_failed(),"sessions":p.sessions.iter().map(|e|{
            let s=&e.session;
            let recovery=s.stats.video_recovery.snapshot();
            let video_diagnostics=serde_json::json!({
                "source":recovery.epoch.source,"attempt":recovery.epoch.recovery,
                "waiting":recovery.waiting,"age_ms":recovery.began_at.elapsed().as_millis() as u64,
                "reference_gaps":s.stats.video_decoder_reference_gaps.load(std::sync::atomic::Ordering::Relaxed),
                "reference_skipped":s.stats.video_decoder_reference_skipped.load(std::sync::atomic::Ordering::Relaxed),
                "queue_dropped":s.stats.video_decode_queue_dropped.load(std::sync::atomic::Ordering::Relaxed),
                "presented_epoch":s.uploaded_epoch.map(|epoch|serde_json::json!({"source":epoch.source,"attempt":epoch.recovery})),
                "presented_age_ms":s.uploaded_at.map(|at|at.elapsed().as_millis() as u64),
            });
            let telemetry_diagnostics = diagnostic_telemetry(
                &s.host_stats,
                s.texture.as_ref().map(|frame| (&frame.timing, frame.decoded_at)),
            );
            let native=serde_json::Value::Null;
            #[cfg(all(target_os="windows",feature="native-windows-video"))]
            let native=s.texture.as_ref().and_then(|frame|frame.native.as_ref().map(|n|serde_json::json!({
                "generation":n.generation,"coded":n.ready.frame.geometry().coded,"visible":n.ready.frame.geometry().visible,
                "cpu_pixel_bytes":frame.rgba.len(),"exact_frame_submitted":n.ready.frame.was_submitted(),
                "surface_counters":n.ready.frame.stats().snapshot(),"surface_error":n.ready.frame.stats().last_error.lock().unwrap().clone()
            }))).unwrap_or(native);
            #[cfg(all(target_os="linux",feature="native-linux-video"))]
            let native=s.texture.as_ref().and_then(|frame|frame.native_linux.as_ref().map(|n|serde_json::json!({
                "generation":n.generation,"coded":n.frame.geometry().coded,"visible":n.frame.geometry().visible,
                "cpu_pixel_bytes":frame.rgba.len(),"exact_frame_submitted":n.frame.was_submitted(),
                "surface_counters":n.frame.stats().snapshot(),"surface_error":n.frame.stats().last_error.lock().unwrap().clone()
            }))).unwrap_or(native);
            let mut result=serde_json::json!({"connected":s.connected,"peer_responsive":s.conn.peer_is_responsive(),
                "supports_input":s.supports_input,"input_eligible":s.can_input(),
                "request_audio":s.request.audio,"volume":s.volume,"muted":s.muted,
                "audio_output_status":s.media.as_ref().map(|m|format!("{:?}",m.status)),
                "audio_playback_verified":false,
                "audio_output_callback_evidence": {
                    "callbacks":s.stats.audio_output_callbacks.load(std::sync::atomic::Ordering::Relaxed),
                    "nonzero_samples":s.stats.audio_output_nonzero_samples.load(std::sync::atomic::Ordering::Relaxed),
                    "errors":s.stats.audio_output_errors.load(std::sync::atomic::Ordering::Relaxed),
                },
                "paused":s.paused,"presentation_hidden":s.presentation_hidden,
                "activity_pending":s.media_activity_pending(),
                "decoder_status":s.media.as_ref().and_then(|m|m.decode_status.try_lock().ok().map(|v|v.clone())),
                "decoder_needs_keyframe":s.stats.video_recovery.is_waiting(),
                "native_frame":native,"device_id":s.key,"name":s.name,"connection_id":s.conn.id,"subscription_id":s.video_id,"activity_revision":s.activity_revision,"target":s.conn.target.to_string(),"route":s.route,
                "source":format!("{:?}",s.source),"source_id":s.source,"decoded_frames":s.stats.video_frames_decoded.load(std::sync::atomic::Ordering::Relaxed),
                "decode_errors":s.stats.video_decode_errors.load(std::sync::atomic::Ordering::Relaxed),"confirmed":s.confirmed,
                "scene_submitted_frame":s.uploaded_at.is_some(),"input_locked":s.locked,"error":s.error,"video_error":s.video_error,
                "clipboard_sync_supported":s.conn.clipboard_available,"clipboard_sync_enabled":s.clipboard_enabled,
                "files_listed":s.files.len(),"transfers":s.transfers.lock().unwrap().snapshots().len()});
            result["computer_use_eligible"]=self.computer_use_can_input(s).into();
            result["background_computer_use_allowed"]=self.computer_use_background_allowed().into();
            result["input_status"]=s.input_status().into();
            result["video_diagnostics"]=video_diagnostics;
            result["telemetry_diagnostics"]=telemetry_diagnostics;
            result
        }).collect::<Vec<_>>()})
    }
    #[cfg(all(test, any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    pub(crate) fn attach_recorder_session(&self, mut session: OriginalGuiSession) {
        // Test-only presentation precondition: real GUI input routing with a local
        // packet recorder. No native decoder image or OS injector is exercised.
        session.confirmed = true;
        session.supports_input = true;
        session.locked = false;
        session.first_frame_after = Instant::now() - std::time::Duration::from_secs(1);
        session.uploaded_at = Some(Instant::now());
        session.uploaded_epoch = Some(session.stats.video_recovery.snapshot().epoch);
        let mut p = self.pool.lock().unwrap();
        let key = session.key.clone();
        p.sessions.push(ViewSession {
            purpose: ConnectionPurpose::View,
            source_seen: session.source,
            session,
            frame: Arc::new(Mutex::new(None)),
            frame_observer: None,
            sources: Arc::new(Vec::new()),
        });
        let count = p.sessions.len();
        p.tabs.select(0, &key, count);
    }

    fn exit_connections(&self, allow_connecting: bool) -> Result<Vec<Arc<remote_core::workspace_session::WorkspaceConnection>>, String> {
        let connections = {
            let mut pool = self.pool.lock().unwrap();
            if !allow_connecting && pool.tabs.connecting().is_some() { return Err("A connection is still being established; cancel it before full exit".into()); }
            for view in &mut pool.sessions { view.session.release_input(); }
            let mut retained=self.pending_exit_connections.lock().unwrap();
            for view in &pool.sessions {
                if !retained.iter().any(|old|Arc::ptr_eq(old,&view.session.conn)) { retained.push(view.session.conn.clone()); }
            }
            retained.clone()
        };
        self.close_all();
        Ok(connections)
    }

    pub fn prepare_gui_exit(&self) -> Result<(), String> {
        self.exit_connections(true).map(|_| ())
    }

    /// Normal GUI exit releases local input and waits only a bounded time for
    /// the peer. Missing remote receipts remain explicit, not a permanent Quit gate.
    pub async fn close_all_for_gui_exit(&self) -> Result<bool, String> {
        let connections = self.exit_connections(true)?;
        let mut pending = tokio::task::JoinSet::new();
        for connection in connections {
            pending.spawn(async move {
                connection.close().await;
                connection.remote_cleanup_confirmed()
            });
        }
        let mut confirmed = true;
        while let Some(result) = pending.join_next().await {
            confirmed &= result.map_err(|_| "A frontend cleanup worker failed".to_string())?;
        }
        self.pending_exit_connections.lock().unwrap().clear();
        Ok(confirmed)
    }

    pub async fn close_all_verified(&self) -> Result<(), String> {
        let connections = self.exit_connections(false)?;
        let mut pending = tokio::task::JoinSet::new();
        for connection in connections { pending.spawn(async move { connection.close_verified().await }); }
        let mut failure = false;
        while let Some(result) = pending.join_next().await { failure |= !matches!(result, Ok(Ok(()))); }
        if failure { return Err("Some outgoing connections did not confirm input/media cleanup; original connections retained, full exit not confirmed".into()); }
        self.pending_exit_connections.lock().unwrap().clear();
        Ok(())
    }

    pub fn close_all(&self) {
        let mut p = self.pool.lock().unwrap();
        self.revoke_computer_use();
        p.directed_receive = None;
        p.tabs.clear();
        p.pending_settings.clear();
        p.retry_binding = None;
        p.sessions.clear();
        p.message = "Ready".into();
        if let Some((_, c)) = p.talkback.take() {
            c.stop();
        }
    }
}
impl Drop for OriginalOwner {
    fn drop(&mut self) {
        self.close_all();
    }
}

#[cfg(test)]
mod restoration_regression_tests {
    use super::*;
    #[tokio::test]
    async fn file_capability_refresh_retires_only_the_idle_fileless_connection() {
        use remote_core::net::{UdpMultiplexer, MultiplexedPacket};
        use protocol::ControlMessage;
        remote_core::init_crypto_provider();
        assert!(remote_core::session_crypto::load_session_psk().is_none());
        let multiplexer = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let address = multiplexer.local_addr().unwrap();
        let (sender, receiver) = multiplexer.split();
        let server = tokio::spawn(async move {
            while let Ok(packet) = receiver.recv().await {
                if let MultiplexedPacket::Control(ControlMessage::Session(command), from) = packet
                    && let SessionCommand::Open { connection_id, .. } = *command {
                    sender.send_control(&ControlMessage::Session(Box::new(SessionCommand::Opened {
                        connection_id, version: protocol::session::SESSION_VERSION, max_subscriptions: 1,
                        files: false, clipboard: false, window_capture: false,
                    })), from).await.unwrap();
                }
            }
        });
        let mut pool = pending_pool();
        let (connection, events) = WorkspaceConnection::connect_media(address).await.unwrap();
        let stale = OriginalGuiSession::new("newly-enabled-receiver".into(), "Files".into(), "Loopback".into(), connection, events);
        let stale_connection = Arc::downgrade(&stale.conn);
        let (connection, events) = WorkspaceConnection::connect_media(address).await.unwrap();
        let mut ready = OriginalGuiSession::new("already-enabled-receiver".into(), "Files".into(), "Loopback".into(), connection, events);
        Arc::get_mut(&mut ready.conn).unwrap().files_available = true;
        let ready_id = ready.conn.id;
        for session in [stale, ready] {
            pool.sessions.push(ViewSession {
                purpose: ConnectionPurpose::Files, source_seen: session.source, session,
                frame: Arc::new(Mutex::new(None)), frame_observer: None, sources: Arc::new(Vec::new()),
            });
        }
        pool.tabs.select(0, "newly-enabled-receiver", 2);
        pool.sessions[0].session.start_video();
        assert!(pool.sessions[0].session.media.is_some());
        assert!(pool.retire_fileless_connection("newly-enabled-receiver").is_err());
        assert_eq!(pool.sessions.len(), 2, "a file action must preserve live video");
        pool.sessions[0].session.media.take();
        pool.retire_fileless_connection("newly-enabled-receiver").unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while stale_connection.upgrade().is_some() { tokio::task::yield_now().await; }
        }).await.expect("retired connection workers release their old binding");
        assert_eq!(pool.sessions.len(), 1);
        assert_eq!(pool.active().unwrap().session.conn.id, ready_id);
        pool.retire_fileless_connection("already-enabled-receiver").unwrap();
        assert_eq!(pool.sessions.len(), 1, "a capable connection and its transfers stay bound");
        server.abort();
    }

    #[tokio::test]
    async fn file_connection_keeps_stream_action_available_and_preserves_connection_binding() {
        use remote_core::net::{UdpMultiplexer,MultiplexedPacket};
        use protocol::{ControlMessage,session::SessionCommand};
        remote_core::init_crypto_provider();
        assert!(remote_core::session_crypto::load_session_psk().is_none());
        let mux=UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let address=mux.local_addr().unwrap();let (sender,receiver)=mux.split();
        let server=tokio::spawn(async move {
            while let Ok(packet)=receiver.recv().await {
                if let MultiplexedPacket::Control(ControlMessage::Session(command),from)=packet
                    && let SessionCommand::Open{connection_id,..}=*command {
                    sender.send_control(&ControlMessage::Session(Box::new(SessionCommand::Opened{
                        connection_id,version:protocol::session::SESSION_VERSION,max_subscriptions:1,
                        files:false,clipboard:false,window_capture:false})),from).await.unwrap();
                }
            }
        });
        let (_,peers)=tokio::sync::watch::channel(remote_core::discovery::DiscoveryPeerSnapshot::default());
        let backend=Arc::new(crate::UnifiedServiceOwner::from_background_snapshot(peers).await.unwrap());
        let owner=OriginalOwner::new(backend);
        let (conn,events)=WorkspaceConnection::connect_media(address).await.unwrap();let id=conn.id;
        owner.attach_recorder_session(OriginalGuiSession::new("file-peer".into(),"Files".into(),"Loopback".into(),conn,events));
        owner.pool.lock().unwrap().active_mut().unwrap().session.uploaded_at=None;
        let snapshot=owner.snapshot();assert!(matches!(snapshot.role,RoleState::Idle));
        assert_eq!(snapshot.connection_id,Some(id));assert_eq!(owner.active_connection().unwrap().id,id);
        let devices=[crate::AppDevice{device_id:"file-peer".into(),display_name:"Files".into(),badge:Default::default(),
            endpoint:address,scope:remote_core::discovery::DiscoveryScope::Lan,
            can_stream:true,can_view:true,online:true,last_seen_ms:0}];
        let rows=super::super::device_list::DeviceListViewModel::project(&devices,&snapshot.role,
            super::super::device_list::DeviceFilterKind::All);
        assert!(rows.rows[0].can_connect,"a completed file transfer must not disable Connect Stream");
        owner.pool.lock().unwrap().active_mut().unwrap().session.pending=Some((1,Instant::now()));
        assert!(matches!(owner.snapshot().role,RoleState::Connecting(_)),"an actual video request still shows connecting");
        {
            let mut pool=owner.pool.lock().unwrap();
            bind_receive(&mut pool,"live-media",Instant::now()+std::time::Duration::from_secs(10));
            pool.directed_receive.as_mut().unwrap().connection=Some(id);
        }
        assert!(owner.directed_receive_current("live-media"));
        owner.pool.lock().unwrap().active_mut().unwrap().session.video_error=Some("Screen sharing ended".into());
        assert!(!owner.directed_receive_current("live-media"));
        assert_eq!(owner.directed_receive_error("live-media").as_deref(),Some("Screen sharing ended"));
        assert_eq!(owner.directed_receive_error("other-request"),None);
        owner.pool.lock().unwrap().active_mut().unwrap().session.video_error=None;
        owner.pool.lock().unwrap().active_mut().unwrap().session.connected=false;
        assert!(!owner.directed_receive_current("live-media"));
        let closed=owner.snapshot();assert!(matches!(closed.role,RoleState::Idle));
        let rows=super::super::device_list::DeviceListViewModel::project(&devices,&closed.role,
            super::super::device_list::DeviceFilterKind::All);
        assert!(rows.rows[0].can_connect,"a stopped peer must remain reconnectable after an upgrade");
        server.abort();let _=server.await;
    }
    fn pending_pool() -> Pool {
        Pool {
            sessions: Vec::new(), tabs: SessionTabsState::default(),
            pending_settings: PendingStreamSettingsState::default(),
            retry_binding: None,
            message: String::new(), receive_dir: PathBuf::new(),
            talkback: None, clipboard_preference: false, directed_receive: None,
        }
    }
    fn begin(pool: &mut Pool, key: &str, values: StreamSettingsValues) -> u64 {
        let plan = pool.tabs.plan_open(SessionPeer {
            device_id: key.into(), name: key.into(), endpoint: "127.0.0.1:1".parse().unwrap(),
        }, None, 0);
        let OpenDecision::Start(attempt) = plan.decision else { panic!("new request expected") };
        pool.pending_settings.begin(key, attempt, values);
        attempt
    }
    fn values() -> StreamSettingsValues {
        StreamSettingsValues { width: 1920, height: 1080, fps: 60, bitrate_kbps: 10_000 }
    }

    fn bind_receive(pool: &mut Pool, request: &str, expires: Instant) {
        pool.directed_receive = Some(DirectedReceiveBinding { request_id: request.into(), source: "a".into(),
            scope: remote_core::shared_files::current_share_scope(), expires, cancel: tokio::sync::watch::channel(()).0, attempt: None, connection: None, claim:None, capture:CaptureSource::MainDisplay });
    }
    #[test]
    fn receiving_status_does_not_outlive_its_connection_or_failed_handshake() {
        let mut pool=pending_pool();
        bind_receive(&mut pool,"live",Instant::now()+std::time::Duration::from_secs(10));
        assert!(pool.receive_current("live"));
        assert!(!pool.receive_current("different-request"));
        pool.directed_receive.as_mut().unwrap().connection=Some(7);
        assert!(!pool.receive_current("live"));
        pool.directed_receive.as_mut().unwrap().connection=None;
        let attempt=begin(&mut pool,"a",values());
        pool.directed_receive.as_mut().unwrap().attempt=Some(attempt);
        pool.tabs.complete("a",attempt,false,0);
        assert!(!pool.receive_current("live"));
    }
    #[test]
    fn directed_receive_failure_belongs_only_to_its_exact_source_and_attempt(){
        let mut pool=pending_pool();
        bind_receive(&mut pool,"request-1",Instant::now()+std::time::Duration::from_secs(10));
        assert!(!pool.receive_failed());
        let first=begin(&mut pool,"a",values());pool.directed_receive.as_mut().unwrap().attempt=Some(first);
        assert!(!pool.receive_failed());
        pool.tabs.complete("a",first,false,0);assert!(pool.receive_failed());
        pool.directed_receive.as_mut().unwrap().attempt=Some(first+1);assert!(!pool.receive_failed());
        pool.directed_receive.as_mut().unwrap().attempt=Some(first);
        pool.directed_receive.as_mut().unwrap().source="b".into();assert!(!pool.receive_failed());
        // A new receive has no allocated attempt yet; an earlier failure must
        // never terminate it before its ordinary connection future starts.
        bind_receive(&mut pool,"request-2",Instant::now()+std::time::Duration::from_secs(10));
        assert!(!pool.receive_failed());
        let rejected=pool.tabs.reject_open("a","A",0).unwrap();pool.directed_receive.as_mut().unwrap().attempt=Some(rejected);
        assert!(pool.receive_failed());
        pool.directed_receive.as_mut().unwrap().expires=Instant::now()-std::time::Duration::from_secs(1);assert!(!pool.receive_failed());
        pool.directed_receive=None;assert!(!pool.receive_failed());
    }
    #[test]
    fn receive_expiry_before_allocation_and_commit_never_admits_late_work() {
        let mut pool = pending_pool();
        bind_receive(&mut pool, "before-start", Instant::now() - std::time::Duration::from_secs(1));
        assert!(!pool.receive_matches("a", Some("before-start")));
        pool.cancel_receive("before-start");
        bind_receive(&mut pool, "handshake", Instant::now() + std::time::Duration::from_secs(10));
        let attempt = begin(&mut pool, "a", values());
        pool.directed_receive.as_mut().unwrap().attempt = Some(attempt);
        assert!(pool.receive_matches("a", Some("handshake")));
        pool.directed_receive.as_mut().unwrap().expires = Instant::now();
        assert!(!pool.receive_matches("a", Some("handshake")));
        pool.message = "Connecting to a".into();
        pool.cancel_receive("handshake");
        assert_eq!(pool.message, "Ready");
        assert_eq!(pool.tabs.complete("a", attempt, true, 0), Completion::Cancelled);
        assert!(pool.pending_settings.take("a", attempt).is_none());
    }
    #[test]
    fn late_receive_cancel_preserves_new_manual_attempt_to_same_device() {
        let mut pool = pending_pool();
        bind_receive(&mut pool, "old", Instant::now() + std::time::Duration::from_secs(10));
        let old = begin(&mut pool, "a", values());
        pool.directed_receive.as_mut().unwrap().attempt = Some(old);
        pool.cancel_receive("old");
        let manual = begin(&mut pool, "a", values());
        pool.message = "Connecting to manual request".into();
        pool.cancel_receive("old");
        assert_eq!(pool.message, "Connecting to manual request");
        assert!(manual > old);
        assert_eq!(pool.tabs.pending_attempt("a"), Some(manual));
        assert_eq!(pool.tabs.complete("a", old, true, 0), Completion::Cancelled);
        assert_eq!(pool.pending_settings.take("a", manual), Some(values()));
        assert_eq!(pool.tabs.complete("a", manual, true, 0), Completion::Selected);
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn watchdog_cancels_without_any_gui_poll() {
        let backend = Arc::new(crate::UnifiedServiceOwner::start(crate::UnifiedServiceOwnerConfig::default()).await.unwrap());
        let owner = OriginalOwner::new(backend);
        let timer = owner.begin_directed_receive("watchdog", "a", CaptureSource::MainDisplay, std::time::Duration::from_millis(30)).unwrap();
        let attempt = {
            let mut pool = owner.pool.lock().unwrap();
            let attempt = begin(&mut pool, "a", values());
            pool.directed_receive.as_mut().unwrap().attempt = Some(attempt);
            attempt
        };
        let mut handshake_cancel = owner.pool.lock().unwrap().directed_receive.as_ref().unwrap().cancel.subscribe();
        // Simulate the GUI caller ceasing to poll while Tokio workers continue.
        std::thread::sleep(std::time::Duration::from_millis(80));
        tokio::time::timeout(std::time::Duration::from_secs(1), timer).await.unwrap().unwrap();
        assert!(handshake_cancel.changed().await.is_err());
        let mut pool = owner.pool.lock().unwrap();
        assert!(pool.directed_receive.is_none());
        assert_eq!(pool.tabs.pending_count(), 0);
        assert_eq!(pool.tabs.complete("a", attempt, true, 0), Completion::Cancelled);
        let manual = begin(&mut pool, "a", values());
        pool.cancel_receive("watchdog");
        assert_eq!(pool.tabs.pending_attempt("a"), Some(manual));
    }

    #[test]
    fn failed_retry_preserves_media_mode_and_expires_with_device_network_or_generation() {
        let mut pool = pending_pool();
        let scope = Some([7; 32]);
        let first = pool.tabs.reject_open("a", "A", 0).unwrap();
        pool.retry_binding = Some(RetryBinding { attempt: first, scope, purpose: ConnectionPurpose::Files, audio: false, auto_list: false, capture:None });
        let (failed, mode) = pool.retry(scope).unwrap();
        assert_eq!(failed.device_id, "a");
        assert!(mode.purpose==ConnectionPurpose::Files && !mode.audio);
        assert!(!mode.auto_list);
        let next = pool.tabs.reject_open("a", "A", 0).unwrap();
        assert!(next > first);
        assert!(pool.retry(scope).is_none());
        pool.retry_binding = Some(RetryBinding { attempt: next, scope, purpose: ConnectionPurpose::View, audio: true, auto_list: true, capture:None });
        assert!(pool.retry(scope).is_some());
        pool.expire_retry(Some([8; 32]));
        assert!(pool.tabs.failed().is_none());
        assert!(pool.retry_binding.is_none());
        assert_eq!(pool.message, "Ready");
        let pending = begin(&mut pool, "b", values());
        pool.retry_binding = Some(RetryBinding { attempt: pending, scope, purpose: ConnectionPurpose::Files, audio: true, auto_list: true, capture:None });
        let (key, mode) = pool.reconnect_target(scope).unwrap();
        assert_eq!(key, "b");
        assert_eq!(mode.unwrap().attempt, pending);
        assert_eq!(pool.tabs.pending_count(), 1);
        assert!(pool.reconnect_target(Some([8; 32])).is_none());
        pool.tabs.disconnect(0);
        assert!(pool.reconnect_target(scope).is_none());
    }

    #[test]
    fn connecting_settings_commit_without_an_active_connection_and_follow_selected_attempt() {
        let mut pool = pending_pool();
        let first = begin(&mut pool, "a", values());
        let selected = begin(&mut pool, "b", values());
        let latest = StreamSettingsValues { width: 3840, height: 2160, fps: 120, ..values() };
        assert!(pool.active().is_none());
        assert!(pool.prepare_stream_settings_update(latest).unwrap().is_none());
        assert_eq!(pool.pending_settings.take("a", first), Some(values()));
        assert_eq!(pool.pending_settings.take("b", selected), Some(latest));
    }
    #[test]
    fn invalid_pending_update_preserves_request_and_missing_generation_fails_closed() {
        let mut pool = pending_pool();
        let attempt = begin(&mut pool, "a", values());
        assert!(pool.prepare_stream_settings_update(StreamSettingsValues { fps: 0, ..values() }).is_err());
        assert_eq!(pool.pending_settings.take("a", attempt), Some(values()));
        assert!(pool.prepare_stream_settings_update(values()).is_err());
        pool.tabs.disconnect(0);
        assert!(pool.prepare_stream_settings_update(values()).unwrap().is_none());
    }
    #[test]
    fn stale_file_view_rejects_new_connection_and_network_even_with_same_labels() {
        let first = Arc::new("same device");
        let second = Arc::new("same device");
        let binding = Arc::downgrade(&first);
        let scope = Some([7; 32]);
        assert!(view_binding_matches(&binding, &first, scope, scope));
        assert!(!view_binding_matches(&binding, &second, scope, scope));
        assert!(!view_binding_matches(
            &binding,
            &first,
            scope,
            Some([8; 32])
        ));
        drop(first);
        assert!(!view_binding_matches(&binding, &second, scope, scope));
    }
}

#[cfg(all(test, target_os = "macos"))]
#[path = "original_owner_live_tests.rs"]
mod original_owner_live_tests;

// Surface errors must not disappear behind a generic connected/waiting state.
// Readiness still comes from the current frame gate, never from a status label.
fn active_video_error(host:Option<&str>,decoder:Option<&str>)->Option<String> {
    host.filter(|s|!s.trim().is_empty()).map(str::to_owned)
        .or_else(||decoder.filter(|s|!s.trim().is_empty()).map(|s|format!("Video decoder: {s}")))
}
#[cfg(test)] mod decoder_diagnostic_tests {
    use super::active_video_error;
    #[test] fn host_error_wins_without_hiding_decoder_only_failure() {
        assert_eq!(active_video_error(Some("source closed"),Some("codec failed")),Some("source closed".into()));
        assert_eq!(active_video_error(None,Some("unsupported source matrix")),Some("Video decoder: unsupported source matrix".into()));
    }
    #[test] fn recovered_decoder_does_not_leave_an_old_error() {
        assert!(active_video_error(None,Some("")).is_none());assert!(active_video_error(None,None).is_none());
        assert!(active_video_error(Some("  "),Some("
 ")).is_none());
    }
}

#[cfg(test)]
mod telemetry_diagnostic_tests {
    use super::diagnostic_telemetry;
    use remote_core::{SharedHostStats, clock_sync::ClockSynchronizer};
    use std::time::Instant;

    #[test]
    fn reports_real_availability_without_exposing_capture_timestamp() {
        let stats = SharedHostStats::default();
        let empty = diagnostic_telemetry(&stats, None);
        assert!(empty["clock"].is_null());
        assert!(empty["host_timing"].is_null());
        assert!(empty["texture_timing"].is_null());
        assert!(empty["latest_ui_submission"].is_null());

        let mut frame = protocol::FrameTimingCheckpoints::new(1_234_567_890);
        frame.recv_ts_us = 10_000;
        frame.decode_done_ts_us = 12_000;
        let decoded = diagnostic_telemetry(&stats, Some((&frame, Instant::now())));
        assert_eq!(decoded["texture_timing"]["capture_present"], true);
        assert_eq!(decoded["texture_timing"]["decode_done_us"], 12_000);
        assert!(!decoded.to_string().contains("1234567890"));
        assert!(decoded["latest_ui_submission"].is_null());

        frame.render_submit_ts_us = 13_000;
        stats.set_latest_timing(frame);
        let submitted = diagnostic_telemetry(&stats, None);
        assert_eq!(submitted["latest_ui_submission"]["ui_submit_us"], 13_000);
        assert_eq!(submitted["latest_ui_submission"]["render_done_us"], 0);
    }

    #[test]
    fn reports_the_actual_clock_state_and_sample_count() {
        let stats = SharedHostStats::default();
        let mut clock = ClockSynchronizer::new();
        let now = remote_core::timing::quanta_now_us();
        let sent = now.saturating_sub(200);
        clock.begin_probe(1, sent).unwrap();
        clock.complete_probe(1, sent, sent + 50, sent + 50, sent + 100).unwrap();
        stats.set_clock_estimate(clock.estimate(now));
        let report = diagnostic_telemetry(&stats, None);
        assert_eq!(report["clock"]["state"], "Acquiring");
        assert_eq!(report["clock"]["sample_count"], 1);
        assert_eq!(report["clock"]["latest_probe_rtt_us"], 100);
        assert!(report["clock"]["selected_sample_age_us"].as_u64().is_some());
        assert!(report["clock"]["latest_probe_age_us"].as_u64().is_some());
    }
}
