//! Shared desktop session state. Network and transfer implementations stay in remote_core.
use super::requests::{FileRequests, RequestKind};
use client::{ClientMediaRuntime, TransferCenterState};
use protocol::{
    InputEvent,
    audio_output::{AudioOutputMode, AudioOutputRequest, AudioOutputStatus, LocalPlayback},
    session::{CaptureSource, CaptureSourceInfo, SessionCommand, SubscriptionRequest},
    shared_files::{SharedFileInfo, SharedFileRequest, SharedFileResponse},
};
use remote_core::{
    SharedHostStats, Statistics,
    file_transfer_runtime::{FileTransferCommand, FileTransferEvent},
    workspace_session::{WorkspaceConnection, WorkspaceEvents},
};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::{Duration, Instant},
};
use tokio::task::JoinHandle;

/// One protocol/lifecycle implementation for both UI renderers. The type parameters
/// own presentation handles only; network identity and input safety are not duplicated.
pub struct SessionState<Texture, Pan: Default> {
    pub key: String,
    pub name: String,
    pub route: String,
    pub conn: Arc<WorkspaceConnection>,
    pub events: WorkspaceEvents,
    pub media: Option<ClientMediaRuntime>,
    #[cfg(any(all(target_os = "windows", feature = "native-windows-video"), all(target_os = "linux", feature = "native-linux-video")))]
    pub prefer_native_video: bool,
    pub stats: Arc<Statistics>,
    pub host_stats: Arc<SharedHostStats>,
    pub sources: Vec<CaptureSourceInfo>,
    pub source_catalog_revision: u64,
    pub source_catalog_error: Option<String>,
    pub source: CaptureSource,
    pub video_id: u32,
    source_revision: u32,
    pub confirmed: bool,
    pub supports_input: bool,
    pub locked: bool,
    pub paused: bool,
    pub pending: Option<(u32, Instant)>,
    camera_replacement: Option<CaptureSource>,
    camera_switch: Option<protocol::session::CameraSwitchRequest>,
    pub delivery_request_id: Option<u64>,
    pub activity_revision: u64,
    focus_sequence: u64,
    pub status: String,
    pub error: String,
    input_error: Option<String>,
    pub video_error: Option<String>,
    pub connected: bool,
    pub files: Vec<SharedFileInfo>,
    pub next_file_page: Option<u64>,
    pub file_pending: bool,
    pub transfers: Arc<Mutex<TransferCenterState>>,
    file_scope: remote_core::shared_files::ShareScope,
    file_notices: tokio::sync::mpsc::Receiver<FileTransferEvent>,
    file_observers: tokio::sync::broadcast::Sender<FileTransferEvent>,
    file_task: Option<JoinHandle<()>>,
    clipboard_file_events: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<FileTransferEvent>>>>,
    clipboard_file_cancel: Option<tokio::sync::broadcast::Sender<()>>,
    clipboard_file_state: Arc<Mutex<remote_core::clipboard_file_runtime::ClipboardFileSyncState>>,
    clipboard_commands: tokio::sync::mpsc::UnboundedSender<(bool, Option<tokio::sync::oneshot::Sender<()>>)>,
    clipboard_active: bool,
    clipboard_gate: Option<Arc<AtomicBool>>,
    pub volume: u8,
    pub muted: bool,
    host_audio_request: Option<AudioOutputRequest>,
    host_audio_status: Option<(u64, AudioOutputStatus)>,
    pub window_resize_request: Option<u32>,
    pub window_resize_notice: Option<Result<(u32, u32), String>>,
    pub zoom: f32,
    pub pan: Pan,
    pub fit: bool,
    pub texture: Option<Texture>,
    pub uploaded_at: Option<Instant>,
    pub uploaded_epoch: Option<remote_core::video_recovery::VideoEpoch>,
    pub keys: BTreeSet<u16>,
    pub mouse: BTreeSet<u8>,
    pub modifiers: u8,
    pub clipboard: client::ClipboardRuntimeControl,
    pub clipboard_enabled: bool,
    pub request: SubscriptionRequest,
    reader_task: Option<JoinHandle<()>>,
    audio_task: Option<JoinHandle<()>>,
    pub presentation_hidden: bool,
    view_selected: bool,
    pub input_focused: bool,
    consumers: remote_core::media_demand::MediaConsumers,
    view_consumer: remote_core::media_demand::ConsumerToken,
    last_activity: Option<remote_core::media_demand::MediaDemand>,
    activity_pending: Option<u64>,
    pub first_frame_after: Instant,
    file_requests: FileRequests,
    pub file_page_after: u64,
    pub file_history: Vec<u64>,
    next_request: u32,
}
#[cfg(feature = "gpui-restoration")]
pub(crate) type OriginalGuiSession =
    SessionState<Arc<client::MacDecodedVideoFrame>, gpui::Point<f32>>;

fn clipboard_should_be_active(eligible: bool, owns: bool, focused: bool) -> bool {
    eligible && (owns || focused)
}

impl<Texture, Pan: Default> SessionState<Texture, Pan> {
    fn clear_transient_error(&mut self) {
        self.input_error = None;
        self.error = match self.media.as_ref().map(|media| &media.status) {
            Some(client::ClientMediaRuntimeStatus::AudioUnavailable { reason }) =>
                format!("Audio unavailable: {reason}"),
            _ => String::new(),
        };
    }
    pub fn new(
        key: String,
        name: String,
        route: String,
        conn: WorkspaceConnection,
        mut events: WorkspaceEvents,
    ) -> Self {
        let conn = Arc::new(conn);
        let clipboard = client::start_clipboard_runtime_control(conn.sender.clone());
        let (clipboard_commands, mut clipboard_requests) = tokio::sync::mpsc::unbounded_channel::<(bool, Option<tokio::sync::oneshot::Sender<()>>)>() ;
        let clipboard_conn = conn.clone();
        let clipboard_ingress = clipboard.clone();
        tokio::spawn(async move {
            while let Some((enabled, done)) = clipboard_requests.recv().await {
                let ingress = enabled.then(|| Arc::new(clipboard_ingress.clone()) as Arc<dyn remote_core::client_session::EnvelopeIngress>);
                let _ = clipboard_conn.clipboard(ingress).await;
                let _ = clipboard_conn.control(SessionCommand::SetClipboard { enabled, revision: 0 }).await;
                if let Some(done) = done {
                    let _ = clipboard_conn.flush_controls().await;
                    let _ = done.send(());
                }
            }
        });
        let transfers = Arc::new(Mutex::new(TransferCenterState::default()));
        let state = transfers.clone();
        let (_unused, empty) = tokio::sync::mpsc::unbounded_channel();
        let mut incoming = std::mem::replace(&mut events.files, empty);
        let (notice_tx, file_notices) = tokio::sync::mpsc::channel(64);
        let (file_observers, _) = tokio::sync::broadcast::channel(64);
        let observers = file_observers.clone();
        let clipboard_file_events: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<FileTransferEvent>>>> = Arc::new(Mutex::new(None));
        let bridge_events = clipboard_file_events.clone();
        let file_task = tokio::spawn(async move {
            while let Some(event) = incoming.recv().await {
                if matches!(event, FileTransferEvent::IncomingClipboardReady { .. }) {
                    if let Some(bridge) = bridge_events.lock().unwrap().as_ref() {
                        let _ = bridge.send(event.clone());
                    }
                }
                {
                    let mut state = state.lock().unwrap();
                    state.apply_event(&event);
                    state.compact_history(128);
                }
                if observers.receiver_count() > 0 { let _ = observers.send(event.clone()); }
                if matches!(
                    event,
                    FileTransferEvent::SharedResponse { .. }
                        | FileTransferEvent::Error { .. }
                        | FileTransferEvent::IncomingCompleted { .. }
                        | FileTransferEvent::OutgoingCompleted { .. }
                ) {
                    let _ = notice_tx.try_send(event);
                }
            }
        });
        let mut consumers = remote_core::media_demand::MediaConsumers::default();
        let view_consumer = consumers.attach(0, remote_core::media_demand::MediaDemand::BOTH).unwrap();
        Self {
            key,
            name,
            route,
            conn,
            events,
            media: None,
            #[cfg(any(all(target_os = "windows", feature = "native-windows-video"), all(target_os = "linux", feature = "native-linux-video")))]
            prefer_native_video: false,
            stats: Statistics::new(),
            host_stats: Arc::new(SharedHostStats::default()),
            sources: Vec::new(),
            source_catalog_revision: 0,
            source_catalog_error: None,
            source: CaptureSource::MainDisplay,
            video_id: 1,
            source_revision: 0,
            host_audio_request: None,
            host_audio_status: None,
            window_resize_request: None,
            window_resize_notice: None,
            confirmed: false,
            supports_input: false,
            // The authenticated connection authorizes control. Source, lease,
            // frame and foreground checks still decide whether an event can flow.
            locked: false,
            paused: false,
            pending: None,
            camera_replacement: None,
            camera_switch: None,
            delivery_request_id: None,
            activity_revision: 0,
            focus_sequence: 0,
            status: "Authenticated device connection".into(),
            error: String::new(),
            input_error: None,
            video_error: None,
            connected: true,
            files: Vec::new(),
            next_file_page: None,
            file_pending: false,
            transfers,
            file_notices,
            file_observers,
            file_task: Some(file_task),
            clipboard_file_events,
            clipboard_file_cancel: None,
            clipboard_file_state: Default::default(),
            clipboard_commands,
            clipboard_active: false,
            clipboard_gate: None,
            file_scope: remote_core::shared_files::current_share_scope(),
            volume: 100,
            muted: false,
            zoom: 1.,
            pan: Pan::default(),
            fit: true,
            texture: None,
            uploaded_at: None,
            uploaded_epoch: None,
            keys: BTreeSet::new(),
            mouse: BTreeSet::new(),
            modifiers: 0,
            clipboard,
            clipboard_enabled: false,
            request: SubscriptionRequest {
                id: 1,
                source: CaptureSource::MainDisplay,
                width: 1920,
                height: 1080,
                fps: 30,
                bitrate_kbps: 8000,
                audio: true,
            },
            reader_task: None,
            audio_task: None,
            presentation_hidden: false,
            view_selected: true,
            input_focused: true,
            consumers,
            view_consumer,
            last_activity: Some(remote_core::media_demand::MediaDemand::BOTH),
            activity_pending: None,
            first_frame_after: Instant::now(),
            file_requests: FileRequests::default(),
            file_page_after: 0,
            file_history: Vec::new(),
            next_request: 1000,
        }
    }
    pub fn control(&mut self, command: SessionCommand) -> bool {
        match self.conn.try_control_checked(command) {
            Ok(()) => true,
            Err(reason) => {
                self.fail_input(reason);
                false
            }
        }
    }
    pub(crate) fn host_audio(&self) -> Option<(bool, Option<AudioOutputStatus>)> {
        (self.confirmed && self.connected && self.request.audio
            && !self.paused && !self.presentation_hidden
            && self.source.category() == protocol::session::SourceCategory::Screen)
            .then(|| (self.host_audio_request.is_some() && self.host_audio_status.is_none(),
                self.host_audio_status.as_ref().map(|(_, status)| status.clone())))
    }
    pub(crate) fn set_host_audio(&mut self, mode: AudioOutputMode) {
        if !matches!(self.host_audio(), Some((false, _))) { return; }
        self.next_request = self.next_request.wrapping_add(1).max(1000);
        let request = AudioOutputRequest { id: self.video_id, source_revision: self.source_revision,
            revision: u64::from(self.next_request), mode };
        match self.conn.try_control_checked(SessionCommand::SetAudioOutput(request)) {
            Ok(()) => {
                self.host_audio_request = Some(request);
                self.host_audio_status = None;
            }
            Err(reason) => self.host_audio_status = Some((0, AudioOutputStatus { requested: mode,
                local_playback: LocalPlayback::Unknown, private_supported: true, message: Some(reason) })),
        }
    }
    pub fn start_video(&mut self) {
        if self.pending.is_some() || !self.connected {
            return;
        }
        if self.video_error.is_some() {
            // A retry gets a fresh identity; an acknowledgement from the failed
            // attempt must never confirm this request or its input source.
            let Some(id) = self.video_id.checked_add(256).filter(|id|*id < 0x1000_0000) else {
                self.error = "Reconnect before retrying this video subscription".into();
                return;
            };
            self.video_id = id;
            self.source_revision = 0;
            self.host_audio_request = None;
            self.host_audio_status = None;
            self.activity_revision = 0;
            self.activity_pending = None;
            self.last_activity = Some(remote_core::media_demand::MediaDemand::BOTH);
        }
        self.video_error = None;
        if let Some(media) = &self.media {
            media.reset_video();
            #[cfg(target_os = "macos")]
            media.set_video_payload_active(true);
        }
        if self.media.is_none() {
            let create = || {
                if self.request.audio {
                    ClientMediaRuntime::start(self.stats.clone())
                } else {
                    ClientMediaRuntime::start_video_only(self.stats.clone())
                }
            };
            #[cfg(any(all(target_os = "windows", feature = "native-windows-video"), all(target_os = "linux", feature = "native-linux-video")))]
            let result = if self.prefer_native_video {
                ClientMediaRuntime::start_native(
                    self.stats.clone(),
                    self.request.audio,
                    self.request.fps,
                )
            } else {
                create()
            };
            #[cfg(not(any(all(target_os = "windows", feature = "native-windows-video"), all(target_os = "linux", feature = "native-linux-video"))))]
            let result = create();
            match result {
                Ok(media) => self.media = Some(media),
                Err(e) => {
                    self.fail_video(e.to_string());
                    return;
                }
            }
            let media = self.media.as_ref().unwrap();
            let player = media.audio_tx.clone();
            let (_unused, dummy) = tokio::sync::mpsc::channel(1);
            let mut audio = std::mem::replace(&mut self.events.audio, dummy);
            self.audio_task = Some(tokio::spawn(async move {
                while let Some(event) = audio.recv().await {
                    let event = match event {
                        remote_core::client_session::AudioIngressEvent::PayloadActivity { stream_id, active } => client::audio_player::AudioPlayerEvent::PayloadActivity { stream_id, active },
                        remote_core::client_session::AudioIngressEvent::Packet(p) => {
                            client::audio_player::AudioPlayerEvent::Packet(p)
                        }
                        remote_core::client_session::AudioIngressEvent::StreamConfig(c) => {
                            client::audio_player::AudioPlayerEvent::StreamConfig(c)
                        }
                    };
                    if player.send(event).await.is_err() {
                        break;
                    }
                }
            }));
            if let client::ClientMediaRuntimeStatus::AudioUnavailable { reason } = &media.status {
                self.error = format!("Audio unavailable: {reason}");
            }
        }
        self.confirmed = false;
        self.clear_transient_error();
        self.status = "Requesting video; waiting for the first decoded frame".into();
        self.request.source = self.source;
        self.request.id = self.video_id;
        self.pending = Some((self.video_id, Instant::now()));
        let conn = self.conn.clone();
        let request = self.request.clone();
        let video = self.media.as_ref().unwrap().decode_tx.clone();
        let stats = self.stats.clone();
        let host = self.host_stats.clone();
        self.reader_task = Some(tokio::spawn(async move {
            if let Err(e) = conn.subscribe(request, video, stats, host).await {
                eprintln!("subscription request failed: {e}");
            }
        }));
        // The source picker requests its own catalog. Starting video must not
        // enqueue an unrelated expensive catalog ahead of Subscribe.
    }
    pub fn configure_stream(
        &mut self,
        w: u32,
        h: u32,
        fps: u32,
        bitrate_kbps: u32,
    ) -> Result<(), String> {
        self.request.width = w;
        self.request.height = h;
        self.request.fps = fps;
        self.request.bitrate_kbps = bitrate_kbps;
        if self.media.is_some() {
            self.conn.try_update_settings(&self.request)?;
        }
        Ok(())
    }
    pub fn resize_window(&mut self, target_size: Option<(u32, u32)>) -> Result<u32, String> {
        if !matches!(self.source, CaptureSource::Window(_)) {
            return Err("当前捕获源不是独立应用窗口".into());
        }
        self.next_request = self.next_request.wrapping_add(1).max(1000);
        let request_id = self.next_request;
        self.conn.try_control_checked(SessionCommand::ResizeWindow {
            id: self.video_id,
            source_revision: self.source_revision,
            request_id,
            target_size,
        })?;
        self.window_resize_request = Some(request_id);
        self.window_resize_notice = None;
        Ok(request_id)
    }
    pub fn focus_source(&mut self) -> bool {
        if self.locked
            || !self.connected
            || !self.confirmed
            || !self.supports_input
            || self.video_error.is_some()
        {
            return false;
        }
        if let Some(reason) = self.input_error.take()
            && self.error == reason
        {
            self.clear_transient_error();
        }
        self.focus_sequence = self.focus_sequence.saturating_add(1);
        self.conn.try_control(SessionCommand::FocusSource {
            id: self.video_id,
            source_revision: self.source_revision,
            sequence: self.focus_sequence,
        })
    }
    pub fn switch_source(&mut self, source: CaptureSource) {
        // A previously denied source must be explicitly selectable again after
        // the user corrects host permission. Keep the ordinary duplicate guard.
        if (source == self.source && self.supports_input) || self.pending.is_some() {
            return;
        }
        if matches!(self.source,CaptureSource::Camera(_)) || matches!(source,CaptureSource::Camera(_)) {
            if self.sources.iter().find(|info|info.source==self.source)
                .zip(self.sources.iter().find(|info|info.source==source))
                .is_some_and(|(from,target)|from.camera_switch_target(target)) {
                self.next_request=self.next_request.wrapping_add(1).max(1000);
                let request=protocol::session::CameraSwitchRequest {id:self.video_id,request_id:u64::from(self.next_request),
                    source_revision:self.source_revision,from:self.source,target:source,delivery_request_id:self.delivery_request_id};
                if self.control(SessionCommand::RequestCameraSwitch(request.clone())) {
                    self.pending=Some((self.next_request,Instant::now()));self.camera_switch=Some(request);
                    self.status="Checking camera availability".into();
                }
                return;
            }
            self.replace_camera_view(source);
            return;
        }
        self.release_input();
        self.next_request = self.next_request.wrapping_add(1).max(1000);
        let request = self.next_request;
        if self.control(SessionCommand::SwitchSource {
            id: self.video_id,
            request_id: request,
            source,
        }) {
            self.pending = Some((request, Instant::now()));
            self.confirmed = false;
            self.status = "Switching source".into();
        }
    }
    fn replace_camera_view(&mut self, source: CaptureSource) {
        self.fail_video("Updating camera view".into());
        self.camera_replacement=Some(source);
        self.pending=Some((self.video_id,Instant::now()));
        self.error.clear();
        self.status="Updating camera view".into();
    }
    pub(crate) fn pending_capture_source(&self)->Option<CaptureSource> {
        self.camera_switch.as_ref().map(|request|request.target)
            .or(self.camera_replacement).or_else(||self.pending.map(|_|self.request.source))
    }
    pub fn set_paused(&mut self, paused: bool) {
        if self.paused != paused { self.release_input(); }
        self.paused = paused;
        self.send_activity();
    }
    pub fn set_view_selected(&mut self, selected: bool) {
        if self.view_selected && !selected { self.release_input(); }
        self.view_selected = selected;
        self.reconcile_clipboard();
        self.send_activity();
    }
    pub fn set_input_focused(&mut self, focused: bool) {
        if self.input_focused && !focused { self.release_input(); }
        self.input_focused = focused;
        self.reconcile_clipboard();
    }
    pub fn set_presentation_hidden(&mut self, hidden: bool) {
        if self.presentation_hidden != hidden { self.release_input(); }
        self.presentation_hidden = hidden;
        self.reconcile_clipboard();
        self.send_activity();
    }
    pub fn attach_consumer(&mut self, slot: u64, demand: remote_core::media_demand::MediaDemand)
        -> Option<remote_core::media_demand::ConsumerToken> {
        if slot == 0 { return None; }
        let token = self.consumers.attach(slot, demand)?;
        self.send_activity();
        Some(token)
    }
    pub fn update_consumer(&mut self, token: remote_core::media_demand::ConsumerToken,
        demand: remote_core::media_demand::MediaDemand) -> bool {
        let changed = self.consumers.update(token, demand);
        if changed { self.send_activity(); }
        changed
    }
    pub fn detach_consumer(&mut self, token: remote_core::media_demand::ConsumerToken) -> bool {
        let changed = self.consumers.detach(token);
        if changed { self.send_activity(); }
        changed
    }
    pub fn payload_demand(&self) -> remote_core::media_demand::MediaDemand { self.consumers.demand() }
    fn send_activity(&mut self) {
        // An unfocused visible picture is still consumed. Audio playback has
        // its own demand, independent of whether the picture is visible.
        let local = remote_core::media_demand::MediaDemand {
            video: self.view_selected && !self.paused && !self.presentation_hidden,
            audio: self.view_selected && !self.paused && self.request.audio && !self.muted && self.volume > 0,
        };
        self.consumers.update(self.view_consumer, local);
        let active = self.consumers.demand();
        if !self.confirmed || self.last_activity == Some(active) { return; }
        let revision = self.activity_revision.wrapping_add(1);
        #[cfg(target_os = "macos")]
        if let Some(media) = &self.media { media.set_video_payload_active(active.video); }
        if self.control(SessionCommand::SetActivity {
            id: self.video_id, revision, video: active.video, audio: active.audio,
        }) {
            if self.last_activity.is_some_and(|previous| previous.video != active.video) {
                self.release_input();
                self.uploaded_at = None;
                self.uploaded_epoch = None;
                self.texture = None;
            }
            self.activity_revision = revision;
            self.activity_pending = Some(revision);
            self.last_activity = Some(active);
        } else {
            #[cfg(target_os = "macos")]
            if let Some(media) = &self.media {
                media.set_video_payload_active(self.last_activity.is_none_or(|previous| previous.video));
            }
        }
    }
    pub fn media_activity_pending(&self) -> bool {
        self.activity_pending.is_some()
    }

    pub fn audio_settings(&mut self) {
        self.send_activity();
        if let Some(media) = &self.media {
            media
                .audio_playback
                .set_settings(client::audio_player::AudioPlayerSettings {
                    volume_percent: self.volume,
                    remote_system_muted: self.muted,
                    remote_microphone_muted: true,
                });
        }
    }
    pub fn set_clipboard(&mut self, enabled: bool) {
        self.clipboard_enabled = enabled;
        self.reconcile_clipboard();
    }
    fn reconcile_clipboard(&mut self) {
        let eligible = self.clipboard_enabled && self.conn.clipboard_available && self.connected
            && self.view_selected && !self.presentation_hidden;
        let owns = self.clipboard_gate.as_ref().is_some_and(|gate| gate.load(std::sync::atomic::Ordering::Acquire));
        // Losing focus to a local application retains the last remote owner.
        // Only a focused session may claim ownership after another remote takes it.
        let active = clipboard_should_be_active(eligible, owns, self.input_focused);
        if self.clipboard_active == active && (!active || owns) { return; }
        self.clipboard_active = active;
        if let Some(cancel) = self.clipboard_file_cancel.take() { let _ = cancel.send(()); }
        if active {
            let gate = Arc::new(AtomicBool::new(false));
            self.clipboard.start_workspace(self.conn.target, gate.clone());
            self.clipboard_gate = Some(gate.clone());
            *self.conn.clipboard_file_owner.lock().unwrap() = Some(remote_core::clipboard_file_runtime::ClipboardOwnership(gate.clone()));
            #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
            if self.conn.files_available {
                #[cfg(target_os = "macos")]
                let provider = remote_platform::MacClipboardProvider::new();
                #[cfg(target_os = "linux")]
                let provider = remote_platform::LinuxClipboardProvider::new();
                #[cfg(target_os = "windows")]
                let provider = remote_platform::WindowsClipboardProvider::new();
                let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
                let (cancel, cancel_rx) = tokio::sync::broadcast::channel(1);
                *self.clipboard_file_events.lock().unwrap() = Some(event_tx);
                self.clipboard_file_cancel = Some(cancel);
                let commands = self.conn.file_commands.clone();
                let state = self.clipboard_file_state.clone();
                tokio::spawn(async move {
                    let observation = remote_core::clipboard_file_runtime::process_clipboard_observation();
                    if let Err(error) = remote_core::clipboard_file_runtime::run_clipboard_file_sync(
                        remote_core::clipboard_file_runtime::OwnedClipboardProvider::new(provider, gate.clone()).with_observation(observation.clone()),
                        commands, event_rx, None, cancel_rx,
                        remote_core::clipboard_file_runtime::ClipboardFileSyncConfig { enabled: Some(gate), state: Some(state), observation: Some(observation), ..Default::default() },
                    ).await { eprintln!("File clipboard sync error: {error}"); }
                });
            }
        } else {
            self.clipboard.stop();
            self.clipboard_gate = None;
            *self.conn.clipboard_file_owner.lock().unwrap() = None;
            *self.clipboard_file_events.lock().unwrap() = None;
            if let Some(cancel) = self.clipboard_file_cancel.take() { let _ = cancel.send(()); }
        }
        let _ = self.clipboard_commands.send((active, None));
    }
    pub(crate) fn input_failed(&self) -> bool {
        self.input_error.is_some()
    }
    pub fn set_input_locked(&mut self, locked: bool) {
        if locked {
            self.release_input();
        } else if self.input_error.take().is_some_and(|error| self.error == error) {
            self.clear_transient_error();
        }
        self.locked = locked;
    }
    fn fail_input(&mut self, reason: String) {
        self.release_input();
        self.input_error = Some(reason.clone());
        self.error = reason;
        self.status = "Control paused; click the video to retry when the host is ready.".into();
    }
    pub(crate) fn retry_input(&mut self) -> bool {
        if self.locked { return false; }
        let Some(reason) = self.input_error.take() else { return false; };
        if !self.can_input() {
            self.input_error = Some(reason);
            return false;
        }
        if self.error == reason { self.clear_transient_error(); }
        self.status = "Remote control resumed".into();
        true
    }
    pub fn release_input(&mut self) {
        if (!self.keys.is_empty() || !self.mouse.is_empty() || self.modifiers != 0)
            && !self
                .conn
                .try_control(SessionCommand::ReleaseInput { id: self.video_id })
        {
            let conn = self.conn.clone();
            let id = self.video_id;
            tokio::spawn(async move {
                let _ = conn.control(SessionCommand::ReleaseInput { id }).await;
            });
        }
        self.keys.clear();
        self.mouse.clear();
        self.modifiers = 0;
    }
    fn fail_video(&mut self, reason: String) {
        self.camera_switch=None;
        self.camera_replacement=None;
        self.release_input();
        if let Some(task) = self.reader_task.take() { task.abort(); }
        // Keep the file/device connection, but end the failed media request.
        // Core removes ingress immediately and retries the remote unsubscribe.
        if !self.conn.try_control(SessionCommand::Unsubscribe { id:self.video_id }) {
            let conn=self.conn.clone(); let id=self.video_id;
            tokio::spawn(async move { let _=conn.control(SessionCommand::Unsubscribe{id}).await; });
        }
        if let Some(media) = &self.media {
            #[cfg(target_os = "macos")]
            media.set_video_payload_active(false);
            media.reset_video();
        }
        self.pending = None;
        self.confirmed = false;
        self.locked = true;
        self.texture = None;
        self.uploaded_at = None;
        self.uploaded_epoch = None;
        self.video_error = Some(reason.clone());
        self.error = reason;
        self.status = "Video request failed; file connection remains available".into();
    }
    pub fn input_status(&self) -> &'static str {
        let recovery = self.stats.video_recovery.snapshot();
        if self.video_error.is_some() || !self.connected {
            "Video/connection unavailable: control disabled"
        } else if self.input_error.is_some() {
            "Control paused; click the video to retry when the host is ready"
        } else if !self.confirmed || self.pending.is_some() {
            "Waiting for the host to confirm this source"
        } else if !self.supports_input {
            match self.source {
                CaptureSource::Window(_) => "This host cannot control the selected window. Check Weavelo Accessibility on that host or choose an input-enabled source",
                CaptureSource::Camera(_) => "Camera viewing does not control the remote desktop",
                CaptureSource::MainDisplay | CaptureSource::Display(_) => "Display control unavailable. Read the host reason below; after correcting permission or source availability, reselect this source before enabling control",
            }
        } else if !self.conn.input_ready(self.video_id,self.source_revision) {
            "Waiting for this host to bind safe input to the current source"
        } else if !self.view_selected {
            "This view is not selected; remote control is paused"
        } else if !self.input_focused {
            "Weavelo window is inactive; remote control is paused"
        } else if self.paused || self.presentation_hidden || self.activity_pending.is_some() {
            "Remote control paused. Resume video and wait for a frame from this source"
        } else if recovery.waiting {
            "Waiting for a video keyframe; remote control is temporarily paused"
        } else if !self.conn.peer_is_responsive() {
            "Host heartbeat is stale: control disabled"
        } else if self.uploaded_epoch != Some(recovery.epoch)
            || !current_input_frame(
                self.uploaded_at,
                self.first_frame_after.max(recovery.began_at),
                true,
            )
        {
            "Waiting for a frame from the current source"
        } else if self.locked {
            "View only: click Enable control, then click the remote picture"
        } else {
            "Mouse and keyboard enabled; Ctrl+Alt+Esc locks control"
        }
    }
    pub fn can_input(&self) -> bool {
        self.can_computer_use_input() && self.input_focused
    }
    pub fn can_prepare_text_input(&self) -> bool {
        self.can_input_ignoring_transient_error() && self.input_focused
    }
    pub fn can_computer_use_input(&self) -> bool {
        self.input_error.is_none() && self.can_input_ignoring_transient_error()
    }
    fn can_input_ignoring_transient_error(&self) -> bool {
        let recovery = self.stats.video_recovery.snapshot();
        self.video_error.is_none()
            && !recovery.waiting
            && self.connected
            && self.confirmed
            && self.pending.is_none()
            && self.supports_input
            && self.conn.input_ready(self.video_id,self.source_revision)
            && !self.locked
            && self.view_selected
            && !self.paused
            && !self.presentation_hidden
            && self.activity_pending.is_none()
            && self.uploaded_epoch == Some(recovery.epoch)
            && current_input_frame(
                self.uploaded_at,
                self.first_frame_after.max(recovery.began_at),
                self.conn.peer_is_responsive(),
            )
    }
    pub fn current_video_frame(
        &self,
        epoch: remote_core::video_recovery::VideoEpoch,
        decoded_at: Instant,
    ) -> bool {
        let recovery = self.stats.video_recovery.snapshot();
        epoch == recovery.epoch
            && !recovery.waiting
            && decoded_at >= self.first_frame_after.max(recovery.began_at)
    }
    pub(crate) fn source_revision(&self) -> u32 { self.source_revision }

    pub fn input_command(&self, event: InputEvent) -> SessionCommand {
        SessionCommand::SourceInput {id:self.video_id,source_revision:self.source_revision,event}
    }
    pub fn input(&mut self, event: InputEvent) {
        if self.can_input() {
            self.control(self.input_command(event));
        }
    }
    pub fn files_command(&mut self, command: FileTransferCommand) -> bool {
        if !self.conn.files_available {
            self.error = "This peer does not support files".into();
            return false;
        }
        if self.conn.file_commands.try_send(command).is_err() {
            self.error = "File queue is busy".into();
            return false;
        }
        true
    }
    fn issue_shared_request(&mut self, kind: RequestKind, request: SharedFileRequest) -> Result<u64, String> {
        if !self.connected || !self.conn.files_available {
            return Err("This connection cannot transfer files".into());
        }
        let id = self.file_requests.begin(kind, Instant::now()).map_err(str::to_owned)?;
        if !self.files_command(FileTransferCommand::SharedRequest {
            request_id: id,
            request,
        }) {
            self.file_requests.cancel(id);
            return Err(self.error.clone());
        }
        self.file_pending = self.file_requests.list_pending();
        Ok(id)
    }
    fn shared_request(&mut self, kind: RequestKind, request: SharedFileRequest) {
        if let Err(reason) = self.issue_shared_request(kind, request) { self.error = reason; }
    }
    pub(crate) fn cli_shared_request(&mut self, request: SharedFileRequest)
        -> Result<(u64, tokio::sync::broadcast::Receiver<FileTransferEvent>), String> {
        let kind = match request {
            SharedFileRequest::List { after_id } => RequestKind::List { after: after_id },
            SharedFileRequest::Fetch { file_id } => RequestKind::Fetch { file: file_id },
        };
        // Subscribe before admission so a fast response/completion cannot be lost.
        let events = self.file_observers.subscribe();
        Ok((self.issue_shared_request(kind, request)?, events))
    }
    pub fn list_files(&mut self, after: u64) {
        self.shared_request(
            RequestKind::List { after },
            SharedFileRequest::List { after_id: after },
        );
    }
    pub(crate) fn list_files_on_connect(&mut self, automatic: bool) {
        if automatic { self.list_files(0); }
    }
    pub fn fetch_file(&mut self, file: u64) {
        if !self.files.iter().any(|f| f.id == file) {
            self.error = "Refresh the shared list before pulling this file".into();
            return;
        }
        self.shared_request(
            RequestKind::Fetch { file },
            SharedFileRequest::Fetch { file_id: file },
        );
    }
    pub fn poll(&mut self) {
        if self.can_input() && (!self.keys.is_empty()||!self.mouse.is_empty()||self.modifiers!=0) {
            let healthy=self.conn.input_binding(self.video_id,self.source_revision).is_some_and(|binding|self.conn.refresh_bound_input(binding));
            if !healthy {
                if self.conn.peer_is_responsive() {
                    self.release_input();
                } else {
                    self.fail_input("Input paused after the window stopped responding; enable control to continue".into());
                }
            }
        }
        let recovery = self.stats.video_recovery.snapshot();
        self.first_frame_after = self.first_frame_after.max(recovery.began_at);
        if self
            .uploaded_epoch
            .is_some_and(|epoch| epoch != recovery.epoch)
        {
            self.release_input();
            self.uploaded_at = None;
            self.uploaded_epoch = None;
        }
        if !self.can_input()
            && (!self.keys.is_empty() || !self.mouse.is_empty() || self.modifiers != 0)
        {
            self.release_input();
        }
        if self.file_requests.expire(Instant::now()) != 0 {
            self.file_pending = self.file_requests.list_pending();
            self.error = "A shared-file request timed out; retry or update the peer".into();
        }
        for _ in 0..128 {
            let Ok(event) = self.events.control.try_recv() else {
                break;
            };
            remote_core::workspace_session::trace_control_event(self.conn.id,"gui_received",&event,
                self.pending.map(|p|p.1.elapsed()),Some(self.pending.is_some()));
            match event {
                SessionCommand::CameraSwitchResult {request,ready}
                    if self.camera_switch.as_ref()==Some(&request) && self.connected
                        && request.id==self.video_id && request.from==self.source && request.source_revision==self.source_revision => {
                    self.camera_switch=None;self.pending=None;
                    match ready {
                        Ok(())=>self.replace_camera_view(request.target),
                        Err(reason)=>{self.error=reason;self.status="Camera switch was not available".into();}
                    }
                }
                SessionCommand::Sources { sources, .. } => {
                    self.source_catalog_revision=self.source_catalog_revision.wrapping_add(1);
                    self.source_catalog_error=None;
                    let before=self.sources.iter().find(|info|info.source==self.source).and_then(|info|info.camera.as_ref());
                    let after=sources.iter().find(|info|info.source==self.source).and_then(|info|info.camera.as_ref());
                    let rotated=self.confirmed && before.zip(after).is_some_and(|(a,b)|a.rotation!=b.rotation||a.mirrored!=b.mirrored);
                    self.sources=sources;
                    if rotated && self.camera_replacement.is_none(){self.replace_camera_view(self.source);}
                }
                SessionCommand::Subscribed {
                    id, supports_input, ..
                } if id == self.video_id
                    && self.pending.is_some_and(|p| p.0 == self.video_id)
                    && self.video_error.is_none() =>
                {
                    self.confirmed = true;
                    self.supports_input = supports_input;
                    self.pending = None;
                    self.status = "Video accepted; waiting for its first decoded frame".into();
                    self.last_activity = Some(remote_core::media_demand::MediaDemand { video: true, audio: self.request.audio });
                    self.send_activity();
                    // Incoming app streams also need real titles/icons. Fetch after
                    // Subscribe is accepted, preserving the selected source and the
                    // first-frame path; metadata never starts a desktop subscription.
                    if self.sources.is_empty()
                        || (matches!(self.source, CaptureSource::Camera(_) | CaptureSource::Window(_))
                            && !self.sources.iter().any(|source| source.source == self.source))
                    {
                        let _ = self.conn.try_control_checked(SessionCommand::ListSources {
                            request_id: 900_001,
                            known_icons: Vec::new(),
                        });
                    }
                }
                SessionCommand::SourceSwitched {
                    id,
                    request_id,
                    source,
                    supports_input, .. } if id == self.video_id && self.pending.is_some_and(|p| p.0 == request_id) => {
                    self.source = source;
                    self.source_revision = request_id;
                    self.host_audio_request = None;
                    self.host_audio_status = None;
                    self.window_resize_request = None;
                    self.window_resize_notice = None;
                    self.request.source = source;
                    self.supports_input = supports_input;
                    self.clear_transient_error();
                    self.pending = None;
                    self.confirmed = true;
                    self.uploaded_at = None;
                    self.uploaded_epoch = None;
                    self.texture = None;
                    self.status = "Source accepted; waiting for its first frame".into();
                }
                SessionCommand::Activity {
                    id,
                    revision,
                    video,
                    ..
                } if id == self.video_id
                    && revision >= self.activity_revision
                    && self.activity_pending.is_some_and(|r| revision >= r) =>
                {
                    self.activity_pending = None;
                    if !video && self.payload_demand().video {
                        self.locked = true;
                        self.error =
                            "The host did not resume video; retry before enabling input".into();
                    }
                }
                SessionCommand::AudioOutputState { request, generation, status } if self.host_audio_request == Some(request)
                    && request.id == self.video_id && request.source_revision == self.source_revision
                    && self.host_audio_status.as_ref().is_none_or(|(previous, _)| generation > *previous) => {
                    self.host_audio_status = Some((generation, status));
                }
                SessionCommand::WindowResized {
                    id,
                    source_revision,
                    request_id,
                    applied_size,
                } if id == self.video_id
                    && source_revision == self.source_revision
                    && self.window_resize_request == Some(request_id) =>
                {
                    self.window_resize_request = None;
                    self.window_resize_notice = Some(applied_size);
                }
                SessionCommand::Closed { reason, .. } => {
                    self.camera_switch=None;self.pending=None;
                    self.window_resize_request = None;
                    self.window_resize_notice = None;
                    self.release_input();
                    self.connected = false;
                    self.reconcile_clipboard();
                    self.confirmed = false;
                    self.error = reason;
                }
                SessionCommand::Unsubscribed { id } if id==self.video_id && self.camera_replacement.is_some() => {
                    self.source=self.camera_replacement.take().unwrap();
                    self.pending=None;
                    self.start_video();
                }
                SessionCommand::Unsubscribed { id } if id == self.video_id && self.video_error.is_none() => {
                    self.fail_video("Screen sharing ended".into());
                }
                SessionCommand::TextRejected {binding,reason,..} if binding.connection_id==self.conn.id && binding.subscription_id==self.video_id && binding.source_revision==self.source_revision() => {
                    self.release_input();
                    let fatal_source = !self.confirmed
                        || !self.supports_input
                        || !self.connected
                        || self.video_error.is_some()
                        || reason.contains("source is unavailable")
                        || reason.contains("source changed");
                    if fatal_source {
                        self.fail_input(reason);
                    } else {
                        self.status = "Text input resynchronized; continue typing".into();
                    }
                }
                SessionCommand::Error { request_id, reason }
                    if request_id == self.video_id && reason.starts_with("INPUT_UNAVAILABLE:") =>
                {
                    let reason = reason
                        .trim_start_matches("INPUT_UNAVAILABLE:")
                        .trim()
                        .to_owned();
                    self.fail_input(reason);
                }
                SessionCommand::Error {request_id,reason} if self.camera_switch.as_ref().is_some_and(|request|request.request_id==u64::from(request_id)) => {
                    self.camera_switch=None;self.pending=None;self.error=reason;
                }
                SessionCommand::Error { request_id, reason } => {
                    if request_id==900_001 {
                        self.source_catalog_revision=self.source_catalog_revision.wrapping_add(1);
                        self.source_catalog_error=Some(reason.clone());
                    }
                    // Cleanup uses the same numeric ID as the video request. Once
                    // fail_video has queued Unsubscribe, its late error (for
                    // example, "subscription is unavailable") must not erase the
                    // capture/subscribe failure that caused cleanup.
                    if request_id == self.video_id && self.video_error.is_some() {
                        continue;
                    }
                    let source_permission_failed = request_id == 900_001
                        && self.media.is_some()
                        && self.uploaded_at.is_none()
                        && capture_permission_failure(&reason);
                    let media_failure = remote_core::workspace_session::confirmation_timeout_affects_media(&reason) != Some(false);
                    if media_failure && (request_id == self.video_id
                        || self.pending.is_some_and(|p| p.0 == request_id)
                        || source_permission_failed)
                    {
                        self.fail_video(reason);
                    } else {
                        self.error = reason;
                    }
                }
                _ => {}
            }
        }
        if self
            .pending
            .is_some_and(|p| p.1.elapsed() > Duration::from_secs(10))
        {
            if self.camera_switch.take().is_some() {
                self.pending=None;self.error="Camera switch was not confirmed; try again".into();
            } else {
                self.fail_video("Host did not confirm the video request; reconnect or update the peer".into());
            }
        }
        for _ in 0..256 {
            let Ok(event) = self.events.audio.try_recv() else {
                break;
            };
            if let Some(media) = &self.media {
                let e = match event {
                    remote_core::client_session::AudioIngressEvent::PayloadActivity { stream_id, active } => client::audio_player::AudioPlayerEvent::PayloadActivity { stream_id, active },
                        remote_core::client_session::AudioIngressEvent::Packet(p) => {
                        client::audio_player::AudioPlayerEvent::Packet(p)
                    }
                    remote_core::client_session::AudioIngressEvent::StreamConfig(c) => {
                        client::audio_player::AudioPlayerEvent::StreamConfig(c)
                    }
                };
                let _ = media.audio_tx.try_send(e);
            }
        }
        for _ in 0..256 {
            let Ok(event) = self.file_notices.try_recv() else {
                break;
            };
            match event {
                FileTransferEvent::SharedResponse {
                    request_id,
                    response,
                } => {
                    let Some(kind) = self.file_requests.resolve(request_id, &response) else {
                        continue;
                    };
                    self.file_pending = self.file_requests.list_pending();
                    match response {
                        SharedFileResponse::Page {
                            entries,
                            next_after_id,
                        } => {
                            if let RequestKind::List { after } = kind {
                                if after == 0 {
                                    self.file_history.clear();
                                } else if self.file_history.last() == Some(&after) {
                                    self.file_history.pop();
                                } else if after != self.file_page_after {
                                    self.file_history.push(self.file_page_after);
                                }
                                self.file_page_after = after;
                            }
                            self.files = entries;
                            self.next_file_page = next_after_id;
                            self.clear_transient_error();
                        }
                        SharedFileResponse::Queued { .. } => {
                            self.status = "Pull accepted; awaiting verified file delivery".into()
                        }
                        SharedFileResponse::Rejected { message } => self.error = message,
                    }
                }
                FileTransferEvent::IncomingCompleted { size_bytes, .. } => {
                    self.status = format!("File received, verified and saved ({size_bytes} bytes)");
                }
                FileTransferEvent::OutgoingCompleted { .. } => {
                    self.status = "File delivered; receiver confirmed completion".into();
                }
                FileTransferEvent::Error { message, .. } => {
                    self.error = message;
                }
                _ => {}
            }
        }
    }
}
impl<Texture, Pan: Default> Drop for SessionState<Texture, Pan> {
    fn drop(&mut self) {
        self.release_input();
        self.clipboard.stop();
        *self.conn.clipboard_file_owner.lock().unwrap() = None;
        *self.clipboard_file_events.lock().unwrap() = None;
        if let Some(cancel) = self.clipboard_file_cancel.take() { let _ = cancel.send(()); }
        let (revoke_done, revoke_wait) = tokio::sync::oneshot::channel();
        let _ = self.clipboard_commands.send((false, Some(revoke_done)));
        if let Some(task) = self.file_task.take() {
            if self.conn.files_available && self.file_scope == remote_core::shared_files::current_share_scope() {
                let conn = self.conn.clone();
                let scope = self.file_scope;
                let mut events = self.file_observers.subscribe();
                // The runtime owns the drain decision, including commands it
                // has dequeued but not yet started. No idle-time guess can
                // safely replace that barrier.
                tokio::spawn(async move {
                    let _ = revoke_wait.await;
                    let drain = async {
                        if conn.file_commands.send(FileTransferCommand::Drain).await.is_ok() {
                            loop {
                                match events.recv().await {
                                    Ok(FileTransferEvent::Drained) | Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                                }
                            }
                        }
                    };
                    tokio::pin!(drain);
                    let mut network = tokio::time::interval(Duration::from_secs(1));
                    loop {
                        tokio::select! {
                            _ = &mut drain => break,
                            _ = network.tick() => if scope != remote_core::shared_files::current_share_scope() {
                                conn.close().await;
                                break;
                            }
                        }
                    }
                    task.abort();
                });
            } else {
                task.abort();
                tokio::spawn(async move { let _ = revoke_wait.await; });
            }
        }
        if let Some(task) = self.audio_task.take() {
            task.abort();
        }
        if let Some(task) = self.reader_task.take() {
            task.abort();
        }
        self.conn
            .try_control(SessionCommand::Unsubscribe { id: self.video_id });
    }
}

pub fn candidate_routes(
    snapshot: &remote_core::discovery::DiscoveryPeerSnapshot,
    device_id: &str,
) -> Vec<(String, std::net::SocketAddr)> {
    use remote_core::discovery::DiscoveryScope;
    let routes: Vec<_> = snapshot
        .peers()
        .iter()
        .filter(|p| p.announcement.device_id == device_id && p.announcement.control_port != 0)
        .map(|p| {
            (
                match p.scope {
                    DiscoveryScope::Lan => 0,
                    DiscoveryScope::P2p => 1,
                    DiscoveryScope::Relay => 2,
                    _ => 3,
                },
                format!("{:?}", p.scope),
                p.endpoint,
            )
        })
        .collect();
    order_unique_routes(routes)
}

/// Preserve preferred routes while trying each endpoint at most once. Sorting by
/// priority does not group equal endpoints, so adjacent-only deduplication is not
/// sufficient (LAN A, P2P B, Relay A).
fn order_unique_routes(
    mut routes: Vec<(u8, String, std::net::SocketAddr)>,
) -> Vec<(String, std::net::SocketAddr)> {
    routes.sort_by_key(|p| p.0);
    let mut endpoints = BTreeSet::new();
    routes.retain(|p| endpoints.insert(p.2));
    routes.into_iter().map(|p| (p.1, p.2)).collect()
}

#[cfg(test)]
mod candidate_route_order_tests {
    use super::order_unique_routes;
    use std::net::SocketAddr;

    fn route(rank: u8, label: &str, endpoint: &str) -> (u8, String, SocketAddr) {
        (rank, label.into(), endpoint.parse().unwrap())
    }

    #[test]
    fn separated_duplicate_does_not_consume_bounded_send_attempt() {
        let routes = order_unique_routes(vec![
            route(3, "Mesh", "127.0.0.1:9003"),
            route(2, "Relay", "127.0.0.1:9001"),
            route(1, "P2p", "127.0.0.1:9002"),
            route(0, "Lan", "127.0.0.1:9001"),
        ]);
        // Directed screen send tries at most three candidates. A duplicate
        // must not displace the third distinct endpoint.
        let attempted: Vec<_> = routes.iter().take(3).map(|p| p.0.as_str()).collect();
        assert_eq!(attempted, ["Lan", "P2p", "Mesh"]);
        assert_eq!(routes.len(), 3);
    }

    #[test]
    fn duplicate_endpoint_keeps_the_preferred_scope() {
        let routes = order_unique_routes(vec![
            route(2, "Relay", "127.0.0.1:9001"),
            route(1, "P2p", "127.0.0.1:9001"),
            route(0, "Lan", "127.0.0.1:9001"),
        ]);
        assert_eq!(routes, vec![("Lan".into(), "127.0.0.1:9001".parse().unwrap())]);
    }

    #[test]
    fn unique_routes_keep_priority_and_equal_priority_order() {
        let routes = order_unique_routes(vec![
            route(2, "Relay", "127.0.0.1:9004"),
            route(1, "P2p first", "127.0.0.1:9002"),
            route(0, "Lan", "127.0.0.1:9001"),
            route(1, "P2p second", "127.0.0.1:9003"),
        ]);
        let labels: Vec<_> = routes.iter().map(|p| p.0.as_str()).collect();
        assert_eq!(labels, ["Lan", "P2p first", "P2p second", "Relay"]);
    }

    #[test]
    fn endpoint_identity_includes_port_address_family_and_ipv6_scope() {
        let endpoints = [
            "127.0.0.1:9001", "127.0.0.1:9002", "[::1]:9001",
            "[fe80::1%2]:9001", "[fe80::1%3]:9001",
        ];
        let routes = order_unique_routes(endpoints.iter()
            .map(|endpoint| route(0, "Lan", endpoint)).collect());
        assert_eq!(routes.len(), endpoints.len());
    }

    #[test]
    fn empty_candidate_list_stays_empty() {
        assert!(order_unique_routes(Vec::new()).is_empty());
    }
}

/// Classify the host error without claiming the user forgot to grant permission.
fn capture_permission_failure(reason: &str) -> bool {
    let reason = reason.to_lowercase();
    reason.contains("tcc")
        || reason.contains("code requirement")
        || (reason.contains("capture") && reason.contains("permission"))
}

pub fn video_failure_guidance(reason: &str) -> &'static str {
    let lower = reason.to_lowercase();
    if lower.contains("code requirement") || lower.contains("signing identity") {
        "The running host does not match its authorized signing identity. Install the correctly signed Weavelo release on that host and restart it. Do not reset unrelated privacy permissions."
    } else if capture_permission_failure(reason) {
        "The host could not use its screen-recording authorization. Permission may already be enabled: check the actual running app and its signing identity, then restart that app. Files remain available."
    } else {
        "The host did not provide a usable video stream. Check the host error below and reconnect after resolving it. Files remain available."
    }
}

#[cfg(test)]
mod capture_error_tests {
    use super::*;
    #[test]
    fn tcc_does_not_blame_missing_user_authorization() {
        let error = "No shareable content available: 用户拒绝了应用程序、窗口、显示器捕捉的TCC";
        assert!(capture_permission_failure(error));
        let hint = video_failure_guidance(error);
        assert!(hint.contains("may already be enabled"));
        assert!(hint.contains("signing identity"));
    }
    #[test]
    fn signing_mismatch_has_specific_remedy() {
        assert!(
            video_failure_guidance("Failed to match existing code requirement")
                .contains("correctly signed")
        );
    }
    #[test]
    fn network_failure_is_not_labeled_permission_failure() {
        assert!(!capture_permission_failure("Connection timed out"));
        assert!(!video_failure_guidance("Connection timed out").contains("authorization"));
    }
}

fn current_input_frame(frame: Option<Instant>, source_started: Instant, responsive: bool) -> bool {
    responsive && frame.is_some_and(|decoded| decoded >= source_started)
}
#[cfg(test)]
pub(crate) mod static_input_tests {
    use super::*;
    use protocol::ControlMessage;
    use remote_core::{
        net::{MultiplexedPacket, UdpMultiplexer},
        session_auth::{AuthTime, ServerHandshake},
    };

    // Real session commands and wire encoding, with only a loopback recorder.
    // No GUI, capture, permission request, clipboard start or OS input injection.
    pub(crate) async fn recorder_session() -> (
        SessionState<(), ()>,
        tokio::sync::mpsc::UnboundedReceiver<protocol::realtime_input::RealtimeInput>,
        JoinHandle<()>,
        tempfile::TempDir,
    ) {
        let psk = remote_core::session_crypto::load_session_psk();
        remote_core::init_crypto_provider();
        let mux = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let addr = mux.local_addr().unwrap();
        let (sender, receiver) = mux.split();
        let (recorded_tx, recorded_rx) = tokio::sync::mpsc::unbounded_channel();
        let recorder = tokio::spawn(async move {
            let mut handshake = ServerHandshake::default();
            let mut owner = None;
            while let Ok(packet) = receiver.recv().await {
                match packet {
                    MultiplexedPacket::Control(ControlMessage::SessionHandshake(message), from) => {
                        let reply = handshake
                            .receive(
                                psk.as_deref().unwrap(),
                                &message,
                                from,
                                AuthTime::now(),
                                owner.is_none(),
                            )
                            .unwrap();
                        if let Some(established) = reply.established {
                            sender.install_peer_crypto(from, established.crypto);
                            owner = Some(from);
                        }
                        sender
                            .send_control(
                                &ControlMessage::SessionHandshake(Box::new(reply.message)),
                                from,
                            )
                            .await
                            .unwrap();
                    }
                    MultiplexedPacket::Control(ControlMessage::Session(command), from)
                        if owner == Some(from)
                            || (psk.is_none()
                                && owner.is_none()
                                && matches!(command.as_ref(), SessionCommand::Open { .. })) =>
                    {
                        if let SessionCommand::RequestInputBinding {
                            connection_id,
                            id,
                            source_revision,
                            request,
                            ..
                        } = *command
                        {
                            sender
                                .send_control(
                                    &ControlMessage::Session(Box::new(
                                        SessionCommand::InputBinding {
                                            binding: protocol::realtime_input::InputBinding {
                                                connection_id,
                                                subscription_id: id,
                                                source_revision,
                                                epoch: [source_revision as u8; 16],
                                            },
                                            request,
                                            host_time_ms: remote_core::session_crypto::now_unix_ms(
                                            ),
                                        },
                                    )),
                                    from,
                                )
                                .await
                                .unwrap();
                        } else if let SessionCommand::RealtimeInput(packet) = *command {
                            if matches!(
                                packet.payload,
                                protocol::realtime_input::InputPayload::Event(_)
                            ) {
                                recorded_tx.send(packet).unwrap();
                            }
                        } else if let SessionCommand::Open { connection_id, .. } = *command {
                            // This test-only recorder also serves the existing unpaired
                            // loopback diagnostic; it is not authentication evidence.
                            owner = Some(from);
                            sender
                                .send_control(
                                    &ControlMessage::Session(Box::new(SessionCommand::Opened {
                                        connection_id,
                                        version: protocol::session::SESSION_VERSION,
                                        max_subscriptions: 8,
                                        files: false,
                                        clipboard: false,
                                        window_capture: true,
                                    })),
                                    from,
                                )
                                .await
                                .unwrap();
                        }
                    }
                    MultiplexedPacket::ClockProbe(probe, host_recv_us, from)
                        if owner == Some(from) =>
                    {
                        sender
                            .send_control(
                                &ControlMessage::ClockReply(protocol::session_auth::ClockReply {
                                    probe,
                                    host_recv_us,
                                    host_send_us: remote_core::quanta_now_us(),
                                }),
                                from,
                            )
                            .await
                            .unwrap();
                    }
                    _ => {}
                }
            }
        });
        let temp = tempfile::tempdir().unwrap();
        // This peer offers only media/input. Do not request file workers or
        // inherit a user's paired group just to exercise this loopback fixture.
        let (conn, events) = WorkspaceConnection::connect_media(addr)
            .await
            .unwrap();
        assert!(!conn.files_available);
        assert!(conn.file_commands.is_closed());
        let session = SessionState::<(), ()>::new(
            "loopback".into(),
            "Input recorder".into(),
            "Loopback".into(),
            conn,
            events,
        );
        (session, recorded_rx, recorder, temp)
    }

    fn camera_pair()->Vec<CaptureSourceInfo> {
        [protocol::session::CameraFacing::Back,protocol::session::CameraFacing::Front].into_iter().enumerate()
            .map(|(index,facing)|CaptureSourceInfo {source:CaptureSource::Camera(11+index as u64),title:format!("Camera {index}"),
                application:String::new(),process_id:None,width:1280,height:720,supports_input:false,
                camera:Some(protocol::session::CameraInfo {facing,switch_group:1,rotation:0,mirrored:false})}).collect()
    }
    #[tokio::test]
    async fn camera_preflight_failure_timeout_and_late_reply_preserve_the_live_source() {
        let (mut session,_received,recorder,_temp)=recorder_session().await;
        session.sources=camera_pair();session.source=session.sources[0].source;session.confirmed=true;
        let from=session.source;let target=session.sources[1].source;
        let (sender,receiver)=tokio::sync::mpsc::unbounded_channel();session.events.control=receiver;
        session.switch_source(target);let request=session.camera_switch.clone().unwrap();
        assert_eq!(session.pending_capture_source(),Some(target));assert_eq!(session.source,from);assert!(session.confirmed);
        assert_eq!((request.id,request.source_revision,request.from,request.target,request.delivery_request_id),
            (session.video_id,0,from,target,None));
        sender.send(SessionCommand::CameraSwitchResult{request:request.clone(),ready:Err("Camera is occupied".into())}).unwrap();
        session.poll();assert_eq!(session.error,"Camera is occupied");assert!(session.confirmed);
        assert_eq!(session.source,from);assert!(session.video_error.is_none());assert!(session.pending.is_none());
        sender.send(SessionCommand::CameraSwitchResult{request,ready:Ok(())}).unwrap();
        session.poll();assert_eq!(session.source,from);assert!(session.confirmed);
        session.switch_source(target);
        session.pending.as_mut().unwrap().1=Instant::now()-Duration::from_secs(11);
        session.poll();assert!(session.camera_switch.is_none());assert!(session.pending.is_none());
        assert!(session.confirmed);assert_eq!(session.source,from);assert!(session.video_error.is_none());
        recorder.abort();
    }
    #[tokio::test]
    async fn camera_preflight_retires_only_the_confirmed_subscription_before_replacement() {
        let (mut session,_received,recorder,_temp)=recorder_session().await;
        session.sources=camera_pair();session.source=session.sources[0].source;session.confirmed=true;session.request.audio=false;
        let old=session.video_id;let from=session.source;let target=session.sources[1].source;
        let (sender,receiver)=tokio::sync::mpsc::unbounded_channel();session.events.control=receiver;
        session.switch_source(target);let request=session.camera_switch.clone().unwrap();
        sender.send(SessionCommand::CameraSwitchResult{request,ready:Ok(())}).unwrap();session.poll();
        assert_eq!(session.source,from);assert_eq!(session.video_id,old);assert!(!session.confirmed);
        sender.send(SessionCommand::Unsubscribed{id:old+256}).unwrap();session.poll();assert_eq!(session.source,from);
        sender.send(SessionCommand::Unsubscribed{id:old}).unwrap();session.poll();
        assert_eq!(session.source,target);assert_eq!(session.video_id,old+256);assert!(!session.confirmed);
        recorder.abort();
    }
    #[tokio::test]
    async fn camera_rotation_waits_for_retirement_and_ignores_old_acknowledgements() {
        let (mut session,_received,recorder,_temp)=recorder_session().await;
        let camera=CaptureSourceInfo {source:CaptureSource::Camera(11),title:"Fixture camera".into(),
            application:"Camera".into(),process_id:None,width:1280,height:720,supports_input:false,
            camera:Some(protocol::session::CameraInfo {facing:protocol::session::CameraFacing::Back,switch_group:1,rotation:90,mirrored:false})};
        session.source=camera.source;session.sources=vec![camera.clone()];session.confirmed=true;
        session.request.audio=false;
        let old=session.video_id;
        let (sender,receiver)=tokio::sync::mpsc::unbounded_channel();session.events.control=receiver;
        let mut rotated=camera.clone();rotated.camera.as_mut().unwrap().rotation=180;
        sender.send(SessionCommand::Sources {request_id:0,sources:vec![rotated],source_icons:vec![],icons:vec![]}).unwrap();
        session.poll();
        assert!(!session.confirmed);assert_eq!(session.video_id,old);
        assert!(session.media.is_none());
        sender.send(SessionCommand::Subscribed {id:old,audio_owner:None,supports_input:true, max_touch_points: 0 }).unwrap();
        sender.send(SessionCommand::Unsubscribed {id:old+256}).unwrap();
        session.poll();assert_eq!(session.video_id,old);assert!(!session.confirmed);
        sender.send(SessionCommand::Unsubscribed {id:old}).unwrap();session.poll();
        assert_eq!(session.video_id,old+256);assert!(!session.confirmed);
        sender.send(SessionCommand::Subscribed {id:old,audio_owner:None,supports_input:true, max_touch_points: 0 }).unwrap();
        session.poll();assert!(!session.confirmed);
        sender.send(SessionCommand::Subscribed {id:old+256,audio_owner:None,supports_input:false, max_touch_points: 0 }).unwrap();
        session.poll();assert!(session.confirmed);assert!(!session.supports_input);
        recorder.abort();
    }

    #[tokio::test]
    async fn rejected_input_preserves_its_actual_reason_for_the_control_retry() {
        let (mut session, mut received, recorder, _temp) = recorder_session().await;
        assert!(!session.control(SessionCommand::SourceInput {
            id: 991, source_revision: 0,
            event: protocol::InputEvent::MouseMoveAbsolute { x: 10, y: 20 },
        }));
        assert!(!session.locked);
        assert!(session.input_failed());
        assert_eq!(session.error, "input binding unavailable");
        assert!(received.try_recv().is_err());
        session.set_input_locked(false);
        assert!(session.error.is_empty());
        assert!(!session.can_input());
        recorder.abort();
    }

    #[tokio::test]
    async fn retry_control_clears_only_its_previous_input_failure() {
        let (mut session, _received, recorder, _temp) = recorder_session().await;
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        session.events.control = receiver;
        sender.send(SessionCommand::Error {
            request_id: session.video_id,
            reason: "INPUT_UNAVAILABLE: selected application is inactive".into(),
        }).unwrap();
        session.poll();
        assert!(!session.locked);
        assert!(session.input_failed());
        assert_eq!(session.error, "selected application is inactive");
        session.set_input_locked(false);
        assert!(session.error.is_empty());
        assert!(!session.input_failed());
        assert!(!session.locked);
        // Retrying the UI does not invent the source/frame/input authorization.
        assert!(!session.can_input());

        session.fail_input("selected application is inactive".into());
        session.error = "File destination is unavailable".into();
        session.set_input_locked(false);
        assert_eq!(session.error, "File destination is unavailable");
        session.video_error = Some("Capture failed".into());
        session.error = "Capture failed".into();
        session.set_input_locked(true);
        session.set_input_locked(false);
        assert_eq!(session.error, "Capture failed");
        assert_eq!(session.video_error.as_deref(), Some("Capture failed"));
        recorder.abort();
    }

    #[tokio::test]
    async fn paused_input_keeps_intent_and_retries_only_with_current_frame_and_grant() {
        let (mut session, mut received, recorder, _temp) = recorder_session().await;
        let (video, _) = remote_core::video_queue::channel(1);
        session.conn.subscribe(session.request.clone(), video, session.stats.clone(), session.host_stats.clone()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !session.conn.input_ready(session.video_id, 0) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.unwrap();
        // Synthetic paint eligibility; this test never captures or injects OS input.
        session.confirmed = true;
        session.supports_input = true;
        session.first_frame_after = Instant::now() - Duration::from_secs(1);
        session.uploaded_at = Some(Instant::now());
        session.uploaded_epoch = Some(session.stats.video_recovery.snapshot().epoch);
        assert!(session.can_input());
        session.keys.insert(4);
        session.mouse.insert(0);
        session.modifiers = protocol::input_modifiers::SHIFT;
        session.fail_input("temporary host input failure".into());
        assert!(!session.locked, "a temporary failure must preserve the user's control intent");
        assert!(session.keys.is_empty() && session.mouse.is_empty() && session.modifiers == 0);
        assert!(!session.can_input());
        session.input(InputEvent::MouseMoveAbsolute { x: 1, y: 2 });
        session.input(InputEvent::MouseDown(0));
        assert!(received.try_recv().is_err(), "paused input must not continue the old gesture");
        session.uploaded_at = None;
        assert!(!session.retry_input(), "a click cannot invent a presented frame");
        assert!(session.input_failed());
        session.uploaded_at = Some(Instant::now());
        session.error = "File destination is unavailable".into();
        assert!(session.retry_input());
        assert_eq!(session.error, "File destination is unavailable");
        assert!(session.can_input());
        // The resumed click starts with its current position, with no old keys or buttons.
        for event in [InputEvent::MouseMoveAbsolute { x: 44500, y: 26000 }, InputEvent::MouseDown(0), InputEvent::MouseUp(0)] {
            session.input(event.clone());
            let packet = tokio::time::timeout(Duration::from_secs(2), received.recv()).await.unwrap().unwrap();
            assert_eq!(packet.binding.connection_id, session.conn.id);
            assert_eq!(packet.binding.subscription_id, session.video_id);
            assert_eq!(packet.binding.source_revision, session.source_revision());
            let protocol::realtime_input::InputPayload::Event(actual) = packet.payload else { panic!("expected only fresh input events"); };
            assert_eq!(actual, event);
        }
        session.fail_input("another temporary failure".into());
        session.set_input_locked(true);
        assert!(!session.retry_input(), "a fresh click must respect an explicit user lock");
        assert!(session.locked);
        recorder.abort();
    }

    #[tokio::test]
    async fn late_cleanup_error_keeps_the_first_video_failure() {
        let (mut session, _received, recorder, _temp) = recorder_session().await;
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        session.events.control = receiver;
        session.pending = Some((session.video_id, Instant::now()));
        sender.send(SessionCommand::Error {
            request_id: session.video_id,
            reason: "synthetic capture startup failed".into(),
        }).unwrap();
        session.poll();
        assert_eq!(session.video_error.as_deref(), Some("synthetic capture startup failed"));

        // The host can reject the cleanup Unsubscribe because it already
        // removed that subscription. Its reply shares the video request ID.
        sender.send(SessionCommand::Error {
            request_id: session.video_id,
            reason: "subscription is unavailable".into(),
        }).unwrap();
        session.poll();
        assert_eq!(session.video_error.as_deref(), Some("synthetic capture startup failed"));
        assert_eq!(session.error, "synthetic capture startup failed");
        recorder.abort();
    }

    #[tokio::test]
    async fn selected_display_input_reaches_the_wire_with_current_source_revision() {
        let (mut session, mut recorded_rx, recorder, _temp) = recorder_session().await;
        // Connection intent needs no extra Enable click, but cannot bypass
        // the source confirmation, presented frame or bound input grant.
        assert!(!session.locked);
        assert!(!session.can_input());
        // Synthetic presentation eligibility only; no native frame is decoded.
        session.confirmed = true;
        session.supports_input = true;
        assert!(!session.can_input());
        session.first_frame_after = Instant::now() - Duration::from_secs(1);
        session.uploaded_at = Some(Instant::now());
        session.uploaded_epoch = Some(session.stats.video_recovery.snapshot().epoch);
        let inputs = [
            InputEvent::MouseMoveAbsolute { x: 32768, y: 32768 },
            InputEvent::MouseDown(0),
            InputEvent::MouseUp(0),
            InputEvent::scroll(0, -2),
            InputEvent::Key {
                key_code: 0,
                pressed: true,
                modifiers: 0,
            },
            InputEvent::Key {
                key_code: 0,
                pressed: false,
                modifiers: 0,
            },
            InputEvent::ModifiersChanged(protocol::input_modifiers::SHIFT),
        ];
        for source in [
            CaptureSource::Display(1),
            CaptureSource::Display(77),
            CaptureSource::Window(3),
        ] {
            session.source = source;
            for revision in [0, 12] {
                session.source_revision = revision;
                let (video,_)=remote_core::video_queue::channel(1);
                session.conn.subscribe(session.request.clone(),video,session.stats.clone(),session.host_stats.clone()).await.unwrap();
                if revision!=0 {session.conn.control(SessionCommand::SwitchSource{id:session.video_id,request_id:revision,source}).await.unwrap();}
                tokio::time::timeout(Duration::from_secs(2),async {
                    while !session.conn.input_ready(session.video_id,revision) {tokio::time::sleep(Duration::from_millis(5)).await;}
                }).await.unwrap();
                for event in &inputs {
                    assert!(session.can_input());
                    session.input(event.clone());
                    let received = tokio::time::timeout(Duration::from_secs(2), recorded_rx.recv())
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(received.binding.connection_id,session.conn.id);
                    assert_eq!(received.binding.subscription_id,session.video_id);
                    assert_eq!(received.binding.source_revision,revision);
                    let protocol::realtime_input::InputPayload::Event(recorded)=received.payload else{panic!()};
                    assert_eq!(&recorded,event,"selected source {source:?} must retain exact input intent");
                }
            }
        }
        session.source = CaptureSource::MainDisplay;
        assert_eq!(
            serde_json::to_value(session.input_command(inputs[0].clone())).unwrap(),
            serde_json::to_value(SessionCommand::SourceInput {
                id: session.video_id,
                source_revision:session.source_revision,
                event: inputs[0].clone(),
            })
            .unwrap(),
            "default desktop input also retains the exact source revision"
        );
        recorder.abort();
    }

    #[test]
    fn unchanged_screen_does_not_disable_input() {
        let now = Instant::now();
        let start = now - Duration::from_secs(1200);
        let decoded = now - Duration::from_secs(600);
        assert!(current_input_frame(Some(decoded), start, true));
    }
    #[tokio::test]
    async fn deselected_view_releases_input_while_another_consumer_keeps_payload_active() {
        let (mut session, _received, recorder, _temp) = recorder_session().await;
        let (video, _packets) = remote_core::video_queue::channel(1);
        session
            .conn
            .subscribe(
                session.request.clone(),
                video,
                session.stats.clone(),
                session.host_stats.clone(),
            )
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !session.conn.input_ready(session.video_id, 0) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // Synthetic paint eligibility only. No capture, native window or OS input.
        session.confirmed = true;
        session.supports_input = true;
        session.first_frame_after = Instant::now() - Duration::from_secs(1);
        session.uploaded_at = Some(Instant::now());
        session.uploaded_epoch = Some(session.stats.video_recovery.snapshot().epoch);
        let background = session
            .attach_consumer(1, remote_core::media_demand::MediaDemand::BOTH)
            .unwrap();
        let connection_id = session.conn.id;
        let activity_revision = session.activity_revision;
        assert!(session.can_input());
        session.keys.insert(0);
        session.mouse.insert(0);
        session.modifiers = protocol::input_modifiers::SHIFT;

        session.set_view_selected(false);

        assert!(session.keys.is_empty(), "the unselected view must release held keys");
        assert!(session.mouse.is_empty(), "the unselected view must release held buttons");
        assert_eq!(session.modifiers, 0);
        assert!(!session.can_input(), "another consumer's video cannot grant GUI input");
        assert_eq!(session.input_status(), "This view is not selected; remote control is paused");
        assert_eq!(session.payload_demand(), remote_core::media_demand::MediaDemand::BOTH);
        assert_eq!(session.activity_revision, activity_revision);
        assert!(!session.media_activity_pending());
        assert_eq!(session.conn.id, connection_id);
        assert!(session.connected);

        session.set_view_selected(true);
        assert!(session.can_input());
        assert!(session.detach_consumer(background));
        recorder.abort();
    }

    #[tokio::test]
    async fn delayed_ack_and_poll_preserve_the_only_current_output_without_reusing_old_paint() {
        let (mut session, _received, recorder, _temp) = recorder_session().await;
        let (video, _packets) = remote_core::video_queue::channel(1);
        session
            .conn
            .subscribe(
                session.request.clone(),
                video,
                session.stats.clone(),
                session.host_stats.clone(),
            )
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !session.conn.input_ready(session.video_id, 0) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // The real actor's ingress/reset behavior is covered in remote_core.
        // This model regression controls GUI delivery and one decoded output;
        // paint metadata below is explicit test input, not native-display proof.
        let (events, control) = tokio::sync::mpsc::unbounded_channel();
        session.events.control = control;
        session.stats.video_recovery.track_output();
        session.stats.video_recovery.reset_source(|| ());
        let accepted = session.stats.video_recovery.snapshot();
        let decoded = session
            .stats
            .video_recovery
            .publish(accepted.epoch, Instant::now)
            .unwrap();
        session.pending = Some((session.video_id, accepted.began_at));
        events
            .send(SessionCommand::Subscribed {
                id: session.video_id,
                audio_owner: None,
                supports_input: true, max_touch_points: 0 })
            .unwrap();
        session.poll();
        assert!(session.confirmed);
        assert_eq!(session.first_frame_after, accepted.began_at);
        assert!(session.current_video_frame(accepted.epoch, decoded));
        assert!(!session.can_input(), "decoding alone is not paint");
        session.uploaded_at = Some(decoded);
        session.uploaded_epoch = Some(accepted.epoch);
        assert!(session.can_input());

        // The complete wait -> valid output transition happens between polls.
        let recovered = session
            .stats
            .video_recovery
            .require_keyframe(accepted.epoch.source)
            .unwrap();
        let decoded_again = session
            .stats
            .video_recovery
            .publish(recovered, Instant::now)
            .unwrap();
        assert!(
            !session.can_input(),
            "old paint cannot unlock even before GUI polling"
        );
        assert!(!session.current_video_frame(accepted.epoch, decoded));
        session.poll();
        assert!(session.current_video_frame(recovered, decoded_again));
        assert!(!session.can_input());
        session.uploaded_at = Some(decoded_again);
        session.uploaded_epoch = Some(recovered);
        assert!(session.can_input());

        // Audio-only/duplicate activity must leave the healthy static frame valid.
        session.activity_revision = 1;
        session.activity_pending = Some(1);
        events
            .send(SessionCommand::Activity {
                id: session.video_id,
                revision: 1,
                video: true,
                audio: false,
            })
            .unwrap();
        session.poll();
        assert!(session.can_input());
        assert_eq!(session.stats.video_recovery.snapshot().epoch, recovered);

        // Resume was already accepted and its only output decoded before GUI ACK.
        session.activity_revision = 2;
        session.activity_pending = Some(2);
        session.stats.video_recovery.reset_source(|| ());
        let resumed = session.stats.video_recovery.snapshot();
        let resumed_frame = session
            .stats
            .video_recovery
            .publish(resumed.epoch, Instant::now)
            .unwrap();
        events
            .send(SessionCommand::Activity {
                id: session.video_id,
                revision: 2,
                video: true,
                audio: false,
            })
            .unwrap();
        session.poll();
        assert_eq!(session.first_frame_after, resumed.began_at);
        assert!(session.current_video_frame(resumed.epoch, resumed_frame));
        assert!(!session.can_input());

        // A late source ACK must not reset/drop the already-current decoded frame.
        session.confirmed = false;
        session.pending = Some((100, Instant::now()));
        session.stats.video_recovery.reset_source(|| ());
        let switched = session.stats.video_recovery.snapshot();
        let switched_frame = session
            .stats
            .video_recovery
            .publish(switched.epoch, Instant::now)
            .unwrap();
        events
            .send(SessionCommand::SourceSwitched {
                id: session.video_id,
                request_id: 100,
                source: CaptureSource::Display(88),
                supports_input: true, max_touch_points: 0 })
            .unwrap();
        session.poll();
        assert_eq!(
            session.stats.video_recovery.snapshot().epoch,
            switched.epoch
        );
        assert!(session.current_video_frame(switched.epoch, switched_frame));
        assert!(!session.current_video_frame(resumed.epoch, resumed_frame));
        assert!(
            !session.can_input(),
            "new source still requires its own binding and paint"
        );
        recorder.abort();
    }
    #[test]
    fn stale_connection_or_old_source_cannot_receive_input() {
        let now = Instant::now();
        assert!(!current_input_frame(Some(now), now, false));
        assert!(!current_input_frame(
            Some(now - Duration::from_secs(1)),
            now,
            true
        ));
        assert!(!current_input_frame(None, now, true));
    }
}

#[cfg(test)]
mod clipboard_focus_tests {
    use super::clipboard_should_be_active;
    #[test]
    fn local_focus_retains_owner_but_only_focused_sessions_reclaim() {
        assert!(clipboard_should_be_active(true, true, false));
        assert!(!clipboard_should_be_active(true, false, false));
        assert!(clipboard_should_be_active(true, false, true));
        assert!(!clipboard_should_be_active(false, true, true));
    }
}
