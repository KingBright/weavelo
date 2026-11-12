use client::TransferCenterState;
use client::audio_player::{AudioPlayer, AudioPlayerEvent, forward_audio_event};
use client::{ClientMediaRuntime, ClipboardRuntimeControl};
use gpui::prelude::FluentBuilder;
use gpui::*;
use protocol::session::{CaptureSourceInfo, SessionCommand, SubscriptionRequest};
use remote_core::workspace_session::{WorkspaceConnection, WorkspaceEvents};
use remote_core::{SharedHostStats, Statistics};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
static CLIPBOARD_TARGET: Mutex<Option<std::sync::Weak<WorkspaceConnection>>> = Mutex::new(None);

#[derive(Default)]
struct Shared {
    input_sources: std::collections::HashSet<u32>,
    clipboard_enabled: Arc<std::sync::atomic::AtomicBool>,
    connection: Option<Arc<WorkspaceConnection>>,
    sources: Vec<CaptureSourceInfo>,
    bindings: HashMap<u32, Option<u32>>,
    status: String,
    audio_error: Option<String>,
    transfers: TransferCenterState,
    remote_files: Vec<protocol::shared_files::SharedFileInfo>,
    shared_next: Option<u64>,
    shared_request: u64,
}
struct Pane {
    focus: FocusHandle,
    request: SubscriptionRequest,
    title: String,
    media: ClientMediaRuntime,
    _stats: Arc<Statistics>,
    bounds: Arc<Mutex<Option<Bounds<Pixels>>>>,
    activity: Option<(bool, bool)>,
    revision: u64,
    resize_at: Instant,
    requested_size: (u32, u32),
}
struct WorkspaceView {
    shared: Arc<Mutex<Shared>>,
    panes: Vec<Pane>,
    next_id: u32,
    selected: Option<u32>,
    tiled: bool,
    keep_audio: bool,
    audio_tx: mpsc::Sender<AudioPlayerEvent>,
    _audio_player: Option<AudioPlayer>,
    volumes: HashMap<u32, u8>,
    clipboard: Option<ClipboardRuntimeControl>,
    receive_dir: std::path::PathBuf,
    last_visible: bool,
    rate_editor: Option<u32>,
    fps_edit: String,
    bitrate_edit: String,
}
impl Drop for WorkspaceView {
    fn drop(&mut self) {
        if let Some(control) = &self.clipboard {
            control.stop();
        }
        self.shared.lock().unwrap().connection.take();
    }
}

pub fn open(target: SocketAddr, name: String, cx: &mut App) {
    let result = cx.open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some(format!("Weavelo · {name}").into()),
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::Windowed(bounds(
                point(px(100.), px(80.)),
                size(px(1200.), px(780.)),
            ))),
            window_min_size: Some(size(px(720.), px(480.))),
            ..Default::default()
        },
        move |window, cx| {
            let view = cx.new(|cx| WorkspaceView::new(target, window, cx));
            let weak = view.downgrade();
            cx.spawn(async move |cx| {
                loop {
                    let Ok(delay) = weak.update(&mut *cx, |view, cx| {
                        cx.notify();
                        if view.last_visible && !view.panes.is_empty() {
                            16
                        } else {
                            250
                        }
                    }) else {
                        break;
                    };
                    Timer::after(Duration::from_millis(delay)).await;
                }
            })
            .detach();
            view
        },
    );
    if let Err(error) = result {
        eprintln!("Open workspace: {error}");
    }
}

impl WorkspaceView {
    fn new(target: SocketAddr, window: &mut Window, cx: &mut Context<Self>) -> Self {
        crate::product_components::theme::sync_appearance(window.appearance(), cx);
        cx.observe_window_appearance(window, |_, window, cx| {
            crate::product_components::theme::sync_appearance(window.appearance(), cx);
        })
        .detach();
        cx.observe_window_activation(window, |this, window, _| {
            if !window.is_window_active() {
                this.release_input();
            }
        })
        .detach();
        let shared = Arc::new(Mutex::new(Shared {
            status: "Connecting…".into(),
            ..Default::default()
        }));
        let receive_dir = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Downloads/Weavelo");
        let (audio_tx, audio_rx) = mpsc::channel(64);
        let audio_player = match AudioPlayer::new(audio_rx) {
            Ok(player) => Some(player),
            Err(error) => {
                shared.lock().unwrap().audio_error =
                    Some(format!("Audio output unavailable: {error}"));
                None
            }
        };
        let weak = Arc::downgrade(&shared);
        let directory = receive_dir.clone();
        let audio_events = audio_tx.clone();
        tokio::spawn(async move {
            match WorkspaceConnection::connect(target, directory).await {
                Ok((connection, events)) => {
                    let connection = Arc::new(connection);
                    if let Some(shared) = weak.upgrade() {
                        let mut state = shared.lock().unwrap();
                        state.connection = Some(connection.clone());
                        state.status = "Connected · choose windows or transfer files".into();
                    } else {
                        return;
                    }
                    let _ = connection
                        .control(SessionCommand::ListSources { request_id: 1, known_icons: Vec::new() })
                        .await;
                    drop(connection);
                    let WorkspaceEvents {
                        mut control,
                        mut audio,
                        mut files,
                    } = events;
                    let (forward_tx, forward_rx) = mpsc::unbounded_channel();
                    let (commands, enabled) = {
                        let Some(shared) = weak.upgrade() else {
                            return;
                        };
                        let state = shared.lock().unwrap();
                        (
                            state.connection.as_ref().unwrap().file_commands.clone(),
                            state.clipboard_enabled.clone(),
                        )
                    };
                    tokio::spawn(async move {
                        let (cancel, cancel_rx) = tokio::sync::broadcast::channel(1);
                        let _cancel = cancel;
                        let observation = remote_core::clipboard_file_runtime::process_clipboard_observation();
                        let _ = remote_core::clipboard_file_runtime::run_clipboard_file_sync(
                            remote_core::clipboard_file_runtime::OwnedClipboardProvider::new(remote_platform::MacClipboardProvider::new(), enabled.clone()).with_observation(observation.clone()),
                            commands,
                            files,
                            Some(forward_tx),
                            cancel_rx,
                            remote_core::clipboard_file_runtime::ClipboardFileSyncConfig {
                                enabled: Some(enabled),
                                observation: Some(observation),
                                ..Default::default()
                            },
                        )
                        .await;
                    });
                    files = forward_rx;
                    loop {
                        tokio::select! {
                            message = control.recv() => {
                                let Some(message) = message else { break; };
                                let Some(shared) = weak.upgrade() else { break; };
                                let mut state = shared.lock().unwrap();
                                match message {
                                    SessionCommand::Sources { sources, .. } => state.sources = sources,
                                    SessionCommand::Subscribed { id, audio_owner, supports_input, .. } => { state.bindings.insert(id, audio_owner); if supports_input { state.input_sources.insert(id); } }
                                    SessionCommand::Error { reason, .. } => state.status = reason,
                                    SessionCommand::Closed { reason, .. } => state.status = format!("Disconnected: {reason}"),
                                    _ => {}
                                }
                            }
                            event = audio.recv() => {
                                let Some(event) = event else { break; };
                                forward_audio_event(&audio_events, event).await;
                            }
                            event = files.recv() => {
                                let Some(event) = event else { break; };
                                if let Some(shared) = weak.upgrade() {
                                    let mut s=shared.lock().unwrap();
                                    if let remote_core::file_transfer_runtime::FileTransferEvent::SharedResponse {request_id,response}=&event {
                                        match response {
                                            protocol::shared_files::SharedFileResponse::Page {entries,next_after_id} if *request_id==s.shared_request => {s.remote_files=entries.clone();s.shared_next = *next_after_id;s.status="Shared list received; choose a file to retrieve".into();}
                                            protocol::shared_files::SharedFileResponse::Queued {..} => s.status="File request accepted; waiting for delivery".into(),
                                            protocol::shared_files::SharedFileResponse::Rejected {message} => s.status=message.clone(),
                                            _ => {}
                                        }
                                    }
                                    s.transfers.apply_event(&event);
                                } else { break; }
                            }
                        }
                    }
                }
                Err(error) => {
                    if let Some(shared) = weak.upgrade() {
                        shared.lock().unwrap().status = error;
                    }
                }
            }
        });
        Self {
            shared,
            panes: Vec::new(),
            next_id: 256,
            selected: None,
            tiled: true,
            keep_audio: false,
            audio_tx,
            _audio_player: audio_player,
            volumes: HashMap::new(),
            clipboard: None,
            receive_dir,
            last_visible: true,
            rate_editor: None,
            fps_edit: String::new(),
            bitrate_edit: String::new(),
        }
    }
    fn request_shared_files(&self, after_id: u64) {
        if let Some(connection) = self.connection() {
            let request_id = {
                let mut s = self.shared.lock().unwrap();
                s.shared_request += 1;
                s.status = "Requesting remote shared list".into();
                s.shared_request
            };
            tokio::spawn(async move {
                let _ = connection
                    .file_commands
                    .send(
                        remote_core::file_transfer_runtime::FileTransferCommand::SharedRequest {
                            request_id,
                            request: protocol::shared_files::SharedFileRequest::List { after_id },
                        },
                    )
                    .await;
            });
        }
    }
    fn connection(&self) -> Option<Arc<WorkspaceConnection>> {
        self.shared.lock().unwrap().connection.clone()
    }
    fn apply_rates(&mut self) {
        let Some(id) = self.rate_editor else {
            return;
        };
        let (Ok(fps), Ok(bitrate_kbps)) = (
            self.fps_edit.trim().parse::<u32>(),
            self.bitrate_edit.trim().parse::<u32>(),
        ) else {
            self.shared.lock().unwrap().status =
                "Enter positive integer FPS and bitrate (kbps)".into();
            return;
        };
        let Some(connection) = self.connection() else {
            return;
        };
        let Some(pane) = self.panes.iter_mut().find(|p| p.request.id == id) else {
            return;
        };
        if let Err(reason) = protocol::validate_video_settings(
            pane.request.width,
            pane.request.height,
            fps,
            bitrate_kbps,
        ) {
            self.shared.lock().unwrap().status = reason.into();
            return;
        }
        pane.request.fps = fps;
        pane.request.bitrate_kbps = bitrate_kbps;
        let request = pane.request.clone();
        tokio::spawn(async move {
            let _ = connection.update_settings(&request).await;
        });
        self.rate_editor = None;
    }
    fn input(&self, id: u32, event: protocol::InputEvent) {
        if !self.shared.lock().unwrap().input_sources.contains(&id) {
            return;
        }
        if let Some(connection) = self.connection() {
            connection.try_control(SessionCommand::Input { id, event });
        }
    }
    fn release_input(&self) {
        if let Some(connection) = self.connection() {
            for id in self.shared.lock().unwrap().input_sources.iter().copied() {
                let connection = connection.clone();
                tokio::spawn(async move {
                    let _ = connection
                        .control(SessionCommand::ReleaseInput { id })
                        .await;
                });
            }
        }
    }
    fn release_input_id(&self, id: u32) {
        if let Some(connection) = self.connection() {
            tokio::spawn(async move {
                let _ = connection
                    .control(SessionCommand::ReleaseInput { id })
                    .await;
            });
        }
    }
    fn pointer(&self, id: u32, position: Point<Pixels>) {
        let Some(pane) = self.panes.iter().find(|p| p.request.id == id) else {
            return;
        };
        let Some(rect) = *pane.bounds.lock().unwrap() else {
            return;
        };
        let frames = pane.media.shared_frame();
        let frame = frames.lock().unwrap();
        let Some(frame) = frame.as_ref() else {
            return;
        };
        use remote_core::VideoFrame;
        if let Some(event) = crate::restored_ui::absolute_pointer_event(
            (position.x.into(), position.y.into()),
            (
                rect.origin.x.into(),
                rect.origin.y.into(),
                rect.size.width.into(),
                rect.size.height.into(),
            ),
            (frame.width(), frame.height()),
            crate::restored_ui::ViewportScaleMode::AspectFit,
        ) {
            self.input(id, event);
        }
    }
    fn add(&mut self, source: CaptureSourceInfo, window: &mut Window, cx: &mut Context<Self>) {
        if self.panes.len() >= protocol::session::MAX_SUBSCRIPTIONS {
            return;
        }
        let Some(connection) = self.connection() else {
            return;
        };
        if self.panes.len() >= connection.max_subscriptions {
            self.shared.lock().unwrap().status = format!(
                "This device supports {} concurrent source(s)",
                connection.max_subscriptions
            );
            return;
        }
        let stats = Statistics::new();
        let media = match ClientMediaRuntime::start_video_only(stats.clone()) {
            Ok(media) => media,
            Err(error) => {
                self.shared.lock().unwrap().status = error.to_string();
                return;
            }
        };
        let preferences = crate::preferences::UserPreferences::load_or_default();
        let request = SubscriptionRequest {
            id: self.next_id,
            source: source.source,
            width: 1920,
            height: 1080,
            fps: preferences.stream.fps,
            bitrate_kbps: preferences.stream.bitrate_kbps,
            audio: true,
        };
        self.next_id += 256;
        self.selected = Some(request.id);
        let send_request = request.clone();
        let decode = media.decode_tx.clone();
        let decode_stats = stats.clone();
        tokio::spawn(async move {
            let _ = connection
                .subscribe(
                    send_request,
                    decode,
                    decode_stats,
                    Arc::new(SharedHostStats::default()),
                )
                .await;
        });
        let focus = cx.focus_handle();
        let id = request.id;
        cx.on_focus_out(&focus, window, move |this, _, _, _| {
            this.release_input_id(id)
        })
        .detach();
        self.panes.push(Pane {
            focus,
            request,
            title: format!("{} · {}", source.application, source.title),
            media,
            _stats: stats,
            bounds: Arc::new(Mutex::new(None)),
            activity: None,
            revision: 0,
            resize_at: Instant::now(),
            requested_size: (1920, 1080),
        });
    }
    fn remove(&mut self, id: u32) {
        self.release_input();
        self.panes.retain(|pane| pane.request.id != id);
        if self.selected == Some(id) {
            self.selected = self.panes.first().map(|p| p.request.id);
        }
        if let Some(connection) = self.connection() {
            tokio::spawn(async move {
                let _ = connection.control(SessionCommand::Unsubscribe { id }).await;
            });
        }
        let mut state = self.shared.lock().unwrap();
        if let Some(Some(owner)) = state.bindings.remove(&id)
            && !state.bindings.values().any(|value| *value == Some(owner))
        {
            let _ = self.audio_tx.try_send(AudioPlayerEvent::RemoveStream(
                protocol::remote_system_audio_stream_id(owner),
            ));
        }
    }
    fn volume(&mut self, id: u32, change: i16) {
        let owner = self
            .shared
            .lock()
            .unwrap()
            .bindings
            .get(&id)
            .copied()
            .flatten();
        if let Some(owner) = owner {
            let volume = self.volumes.entry(owner).or_insert(100);
            *volume = (i16::from(*volume) + change).clamp(0, 200) as u8;
            let _ = self.audio_tx.try_send(AudioPlayerEvent::StreamVolume {
                stream_id: protocol::remote_system_audio_stream_id(owner),
                volume_percent: *volume,
                muted: *volume == 0,
            });
        }
    }
    fn toggle_clipboard(&mut self) {
        let Some(connection) = self.connection() else {
            return;
        };
        if !connection.clipboard_available {
            self.shared.lock().unwrap().status =
                "This device does not support clipboard sharing".into();
            return;
        }
        if let Some(control) = self.clipboard.take() {
            self.shared
                .lock()
                .unwrap()
                .clipboard_enabled
                .store(false, std::sync::atomic::Ordering::Relaxed);
            control.stop();
            tokio::spawn(async move {
                let _ = connection.clipboard(None).await;
                let _ = connection
                    .control(SessionCommand::SetClipboard { enabled: false, revision: 0 })
                    .await;
            });
        } else {
            if let Some(previous) = CLIPBOARD_TARGET
                .lock()
                .unwrap()
                .replace(Arc::downgrade(&connection))
                .and_then(|weak| weak.upgrade())
            {
                tokio::spawn(async move {
                    let _ = previous
                        .control(SessionCommand::SetClipboard { enabled: false, revision: 0 })
                        .await;
                    let _ = previous.clipboard(None).await;
                });
            }
            self.shared
                .lock()
                .unwrap()
                .clipboard_enabled
                .store(true, std::sync::atomic::Ordering::Relaxed);
            let control = client::start_clipboard_runtime_control(connection.sender.clone());
            control.start_workspace(
                connection.target,
                self.shared.lock().unwrap().clipboard_enabled.clone(),
            );
            self.clipboard = Some(control.clone());
            tokio::spawn(async move {
                let _ = connection.clipboard(Some(Arc::new(control))).await;
                let _ = connection
                    .control(SessionCommand::SetClipboard { enabled: true, revision: 0 })
                    .await;
            });
        }
    }

    fn update_activity(&mut self, visible: bool, scale: f32) {
        let Some(connection) = self.connection() else {
            return;
        };
        self.last_visible = visible;
        let visible_count = if self.tiled {
            self.panes.len().max(1)
        } else {
            1
        };
        for pane in &mut self.panes {
            let video = visible && (self.tiled || self.selected == Some(pane.request.id));
            let audio = video || self.keep_audio;
            if pane.activity != Some((video, audio)) {
                pane.activity = Some((video, audio));
                pane.revision += 1;
                let command = SessionCommand::SetActivity {
                    id: pane.request.id,
                    revision: pane.revision,
                    video,
                    audio,
                };
                let connection = connection.clone();
                tokio::spawn(async move {
                    let _ = connection.control(command).await;
                });
            }
            if video && let Some(bounds) = *pane.bounds.lock().unwrap() {
                let width = (f32::from(bounds.size.width) * scale).clamp(64., 3840.) as u32 & !1;
                let height = (f32::from(bounds.size.height) * scale).clamp(64., 2160.) as u32 & !1;
                let size = (width / 32 * 32, height / 32 * 32);
                if pane.requested_size != size {
                    pane.requested_size = size;
                    pane.resize_at = Instant::now();
                }
                if (pane.request.width, pane.request.height) != size
                    && pane.resize_at.elapsed() > Duration::from_millis(250)
                {
                    pane.request.width = size.0;
                    pane.request.height = size.1;
                    let ceiling = 120_000_000_u64
                        / (u64::from(size.0) * u64::from(size.1) * visible_count as u64).max(1);
                    let mut request = pane.request.clone();
                    request.fps = request.fps.min(ceiling.max(1) as u32);
                    let connection = connection.clone();
                    tokio::spawn(async move {
                        let _ = connection.update_settings(&request).await;
                    });
                }
            }
        }
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::product_components::theme::ActiveTheme;
        use ely_gpui_component::{
            buttons::{Button, ButtonVariant},
            theme::ControlSize,
        };
        if !self
            .shared
            .lock()
            .unwrap()
            .clipboard_enabled
            .load(std::sync::atomic::Ordering::Relaxed)
            && let Some(control) = self.clipboard.take()
        {
            control.stop();
        }
        self.update_activity(window_visible(window), window.scale_factor());
        if window_visible(window) && window.is_window_active() {
            if let Some(connection) = self.connection() {
                let ids = self
                    .shared
                    .lock()
                    .unwrap()
                    .input_sources
                    .iter()
                    .copied()
                    .collect::<Vec<_>>();
                for id in ids {
                    if let Some(binding) = connection.input_binding(id, 0) {
                        if !connection.refresh_bound_input(binding) {
                            self.release_input_id(id);
                        }
                    }
                }
            }
        }

        let state = self.shared.lock().unwrap();
        let sources = state.sources.clone();
        let status = state.status.clone();
        let audio_error = state.audio_error.clone();
        let transfers = state.transfers.snapshots();
        let bindings = state.bindings.clone();
        let remote_files = state.remote_files.clone();
        let shared_next = state.shared_next;
        drop(state);

        let theme = cx.theme().clone();
        let rose_ink = crate::design_system::theme_accent_rose(cx);
        let layout_switcher = crate::product_components::segmented_track(&theme)
            .child(
                Button::new("grid", crate::i18n::tr("Grid"))
                    .variant(if self.tiled {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Ghost
                    })
                    .control_size(ControlSize::Sm)
                    .h(px(26.))
                    .px_2p5()
                    .rounded(px(7.))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.tiled = true;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("tabs", crate::i18n::tr("Tabs"))
                    .variant(if !self.tiled {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Ghost
                    })
                    .control_size(ControlSize::Sm)
                    .h(px(26.))
                    .px_2p5()
                    .rounded(px(7.))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.tiled = false;
                        cx.notify();
                    })),
            );

        let toolbar = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .items_center()
            .p_2p5()
            .rounded(px(12.))
            .bg(theme.surface.base)
            .border_1()
            .border_color(theme.border.default)
            .child(crate::product_components::weavelo_brand_mark(26.0, cx))
            .child(layout_switcher)
            .child(
                Button::new("clipboard", crate::i18n::tr("Clipboard"))
                    .variant(if self.clipboard.is_some() {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    })
                    .control_size(ControlSize::Sm)
                    .h(px(28.))
                    .px_2p5()
                    .rounded_full()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_clipboard();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("audio-background", crate::i18n::tr("Background audio"))
                    .variant(if self.keep_audio {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    })
                    .control_size(ControlSize::Sm)
                    .h(px(28.))
                    .px_2p5()
                    .rounded_full()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.keep_audio = !this.keep_audio;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("send", crate::i18n::tr("Send files"))
                    .variant(ButtonVariant::Secondary)
                    .control_size(ControlSize::Sm)
                    .h(px(28.))
                    .px_2p5()
                    .rounded_full()
                    .on_click(cx.listener(|this, _, _, _| {
                        if let Some(connection) = this.connection() {
                            std::thread::spawn(move || {
                                if let Some(paths) = rfd::FileDialog::new().pick_files() {
                                    let _ = connection.file_commands.blocking_send(
                                        remote_core::file_transfer_runtime::FileTransferCommand::SendFileGroup {
                                            files: paths
                                                .into_iter()
                                                .map(|path| {
                                                    remote_core::file_transfer_runtime::FileTransferGroupFile {
                                                        path,
                                                        mime_type: None,
                                                    }
                                                })
                                                .collect(),
                                        },
                                    );
                                }
                            });
                        }
                    })),
            )
            .child(
                Button::new("received", crate::i18n::tr("Received files"))
                    .variant(ButtonVariant::Secondary)
                    .control_size(ControlSize::Sm)
                    .h(px(28.))
                    .px_2p5()
                    .rounded_full()
                    .on_click(cx.listener(|this, _, _, cx| cx.reveal_path(&this.receive_dir))),
            )
            .child(
                Button::new("refresh", crate::i18n::tr("Refresh sources"))
                    .variant(ButtonVariant::Subtle)
                    .control_size(ControlSize::Sm)
                    .h(px(28.))
                    .px_2p5()
                    .rounded_full()
                    .on_click(cx.listener(|this, _, _, _| {
                        if let Some(connection) = this.connection() {
                            tokio::spawn(async move {
                                let _ = connection
                                    .control(SessionCommand::ListSources {
                                        request_id: 1,
                                        known_icons: Vec::new(),
                                    })
                                    .await;
                            });
                        }
                    })),
            );

        let file_toolbar = div()
            .flex()
            .flex_col()
            .gap_1p5()
            .pt_2()
            .border_t_1()
            .border_color(theme.border.divider)
            .child(
                Button::new("publish-files", crate::i18n::tr("Share files with network"))
                    .variant(ButtonVariant::Secondary)
                    .control_size(ControlSize::Sm)
                    .full_width()
                    .h(px(28.))
                    .rounded(px(8.))
                    .on_click(cx.listener(|this, _, _, _| {
                        let shared = Arc::downgrade(&this.shared);
                        std::thread::spawn(move || {
                            let scope = remote_core::shared_files::current_share_scope();
                            if let Some(paths) = rfd::FileDialog::new().pick_files() {
                                let result = (|| -> Result<(), String> {
                                    let runtime = tokio::runtime::Builder::new_current_thread()
                                        .enable_all()
                                        .build()
                                        .map_err(|e| e.to_string())?;
                                    for path in paths {
                                        if remote_core::shared_files::current_share_scope() != scope {
                                            return Err(
                                                "Device network changed; select files again".into()
                                            );
                                        }
                                        runtime.block_on(
                                            remote_core::shared_files::shared_file_catalog()
                                                .publish(&path, scope),
                                        )?;
                                    }
                                    Ok(())
                                })();
                                if let Some(s) = shared.upgrade() {
                                    s.lock().unwrap().status = match result {
                                        Ok(()) => "Files shared with device network until cleared, network change, or app exit".into(),
                                        Err(e) => e,
                                    };
                                }
                            }
                        });
                    })),
            )
            .child(
                Button::new("clear-shared", crate::i18n::tr("Clear my shared list"))
                    .variant(ButtonVariant::Ghost)
                    .control_size(ControlSize::Sm)
                    .full_width()
                    .h(px(26.))
                    .rounded(px(8.))
                    .on_click(cx.listener(|this, _, _, _| {
                        remote_core::shared_files::shared_file_catalog().clear();
                        this.shared.lock().unwrap().status =
                            "Shared list cleared; accepted transfers may finish".into();
                    })),
            )
            .child(
                Button::new("list-shared", crate::i18n::tr("Retrieve shared files"))
                    .variant(ButtonVariant::Subtle)
                    .control_size(ControlSize::Sm)
                    .full_width()
                    .h(px(28.))
                    .rounded(px(8.))
                    .on_click(cx.listener(|this, _, _, _| {
                        this.request_shared_files(0);
                    })),
            )
            .when_some(shared_next, |view, next| {
                view.child(
                    Button::new("next-shared", crate::i18n::tr("Next shared page"))
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Sm)
                        .full_width()
                        .h(px(26.))
                        .rounded(px(8.))
                        .on_click(cx.listener(move |this, _, _, _| {
                            this.request_shared_files(next);
                        })),
                )
            });

        let mut sidebar = div()
            .id("sources")
            .w(px(252.))
            .flex_none()
            .p_3()
            .rounded(px(12.))
            .bg(theme.surface.base)
            .border_1()
            .border_color(theme.border.default)
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.content.secondary)
                    .child(crate::i18n::tr("Displays and application windows")),
            );
        for (index, source) in sources.into_iter().enumerate() {
            sidebar = sidebar.child(
                Button::new(
                    format!("source-{index}"),
                    format!("{} — {}", source.application, source.title),
                )
                .variant(ButtonVariant::Subtle)
                .control_size(ControlSize::Sm)
                .full_width()
                .justify_start()
                .h(px(30.))
                .rounded(px(8.))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.add(source.clone(), window, cx);
                    cx.notify();
                })),
            );
        }
        sidebar = sidebar.child(file_toolbar);
        for file in remote_files {
            let id = file.id;
            sidebar = sidebar.child(
                Button::new(
                    format!("get-shared-{id}"),
                    format!("Get {} ({} bytes)", file.name, file.size_bytes),
                )
                .variant(ButtonVariant::Secondary)
                .control_size(ControlSize::Sm)
                .full_width()
                .justify_start()
                .h(px(28.))
                .rounded(px(8.))
                .on_click(cx.listener(move |this, _, _, _| {
                    if let Some(connection) = this.connection() {
                        let request_id = {
                            let mut s = this.shared.lock().unwrap();
                            s.shared_request += 1;
                            s.shared_request
                        };
                        tokio::spawn(async move {
                            let _ = connection
                                .file_commands
                                .send(
                                    remote_core::file_transfer_runtime::FileTransferCommand::SharedRequest {
                                        request_id,
                                        request: protocol::shared_files::SharedFileRequest::Fetch {
                                            file_id: id,
                                        },
                                    },
                                )
                                .await;
                        });
                    }
                })),
            );
        }
        let mut tabs = div().flex().flex_wrap().gap_1p5();
        for pane in &self.panes {
            let id = pane.request.id;
            let is_selected = self.selected == Some(id);
            tabs = tabs.child(
                Button::new(format!("tab-{id}"), pane.title.clone())
                    .variant(if is_selected {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    })
                    .control_size(ControlSize::Sm)
                    .h(px(28.))
                    .px_2p5()
                    .rounded_full()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = Some(id);
                        cx.notify();
                    })),
            );
        }
        let count = if self.tiled {
            self.panes.len()
        } else {
            usize::from(!self.panes.is_empty())
        };
        let columns = if count > 1 { 2 } else { 1 };
        let mut grid = div()
            .flex_1()
            .min_h_0()
            .grid()
            .grid_cols(columns)
            .grid_rows((count as u16).div_ceil(columns).max(1))
            .gap_2p5();
        for pane in &self.panes {
            let id = pane.request.id;
            if !self.tiled && self.selected != Some(id) {
                continue;
            }
            let volume = bindings
                .get(&id)
                .copied()
                .flatten()
                .and_then(|owner| self.volumes.get(&owner).copied())
                .unwrap_or(100);
            let heading = div()
                .flex()
                .gap_1p5()
                .items_center()
                .pb_1p5()
                .border_b_1()
                .border_color(theme.border.divider)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .child(pane.title.clone()),
                )
                .child(
                    Button::new(format!("rates-{id}"), crate::i18n::tr("Rates"))
                        .variant(ButtonVariant::Subtle)
                        .control_size(ControlSize::Sm)
                        .h(px(24.))
                        .px_2()
                        .rounded_full()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(pane) = this.panes.iter().find(|p| p.request.id == id) {
                                this.rate_editor = Some(id);
                                this.fps_edit = pane.request.fps.to_string();
                                this.bitrate_edit = pane.request.bitrate_kbps.to_string();
                                cx.notify();
                            }
                        })),
                )
                .child(
                    Button::new(format!("less-{id}"), "−")
                        .variant(ButtonVariant::Subtle)
                        .control_size(ControlSize::Sm)
                        .size(px(24.))
                        .p_0()
                        .rounded(px(6.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.volume(id, -10);
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .font_family(crate::design_system::product_mono_font())
                        .text_size(px(11.))
                        .text_color(theme.content.secondary)
                        .child(format!("{volume}%")),
                )
                .child(
                    Button::new(format!("more-{id}"), "+")
                        .variant(ButtonVariant::Subtle)
                        .control_size(ControlSize::Sm)
                        .size(px(24.))
                        .p_0()
                        .rounded(px(6.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.volume(id, 10);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(format!("close-{id}"), "")
                        .variant(ButtonVariant::Subtle)
                        .control_size(ControlSize::Sm)
                        .size(px(24.))
                        .p_0()
                        .rounded(px(6.))
                        .child(
                            crate::product_components::icon(
                                crate::product_components::IconName::Close,
                            )
                            .size(px(12.)),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.remove(id);
                            cx.notify();
                        })),
                );
            let shared_frame = pane.media.shared_frame();
            let frame = shared_frame.lock().unwrap();
            let image = if let Some(frame) = frame.as_ref() {
                client::decoded_video_frame_surface_with_fit(frame, ObjectFit::Contain)
            } else {
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.content.tertiary)
                    .child(crate::i18n::tr("Waiting for source…"))
                    .into_any_element()
            };
            let bounds = pane.bounds.clone();
            grid = grid.child(
                div()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .bg(theme.surface.base)
                    .border_1()
                    .border_color(if self.selected == Some(id) {
                        theme.border.focus
                    } else {
                        theme.border.default
                    })
                    .rounded(px(12.))
                    .p_2p5()
                    .child(heading)
                    .child(
                        div()
                            .id(format!("workspace-input-{id}"))
                            .track_focus(&pane.focus)
                            .on_mouse_move(cx.listener(
                                move |this, event: &MouseMoveEvent, _, _| {
                                    this.pointer(id, event.position)
                                },
                            ))
                            .on_any_mouse_down(cx.listener(
                                move |this, event: &MouseDownEvent, window, _| {
                                    if let Some(pane) =
                                        this.panes.iter().find(|p| p.request.id == id)
                                    {
                                        window.focus(&pane.focus);
                                    }
                                    this.pointer(id, event.position);
                                    if let Some(button) =
                                        crate::restored_ui::protocol_mouse_button(event.button)
                                    {
                                        this.input(id, protocol::InputEvent::MouseDown(button));
                                    }
                                },
                            ))
                            .capture_any_mouse_up(cx.listener(
                                move |this, event: &MouseUpEvent, _, _| {
                                    if let Some(button) =
                                        crate::restored_ui::protocol_mouse_button(event.button)
                                    {
                                        this.input(id, protocol::InputEvent::MouseUp(button));
                                    }
                                },
                            ))
                            .capture_key_down(cx.listener(
                                move |this, event: &KeyDownEvent, _, _| {
                                    if let Some(event) = crate::restored_ui::fresh_key_down(event)
                                        .and_then(|key| {
                                            crate::restored_ui::protocol_key_event(key, true)
                                        })
                                    {
                                        this.input(id, event);
                                    }
                                },
                            ))
                            .capture_key_up(cx.listener(move |this, event: &KeyUpEvent, _, _| {
                                if let Some(event) =
                                    crate::restored_ui::protocol_key_event(&event.keystroke, false)
                                {
                                    this.input(id, event);
                                }
                            }))
                            .on_scroll_wheel(cx.listener(
                                move |this, event: &ScrollWheelEvent, _, _| {
                                    let delta = event.delta.pixel_delta(px(40.));
                                    this.input(
                                        id,
                                        protocol::InputEvent::scroll(
                                            f32::from(delta.x).round() as i32,
                                            f32::from(delta.y).round() as i32,
                                        ),
                                    );
                                },
                            ))
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .rounded(px(8.))
                            .bg(rgb(0x090c10))
                            .overflow_hidden()
                            .child(image)
                            .child(
                                canvas(
                                    move |rect, _, _| {
                                        *bounds.lock().unwrap() = Some(rect);
                                    },
                                    |_, (), _, _| {},
                                )
                                .absolute()
                                .size_full(),
                            ),
                    ),
            );
        }
        let mut transfer_line = div().flex().flex_wrap().gap_2();
        for transfer in transfers.iter().rev().take(4) {
            transfer_line = transfer_line.child(
                div()
                    .px_2p5()
                    .py_1()
                    .rounded_full()
                    .bg(theme.surface.sunken)
                    .border_1()
                    .border_color(theme.border.muted)
                    .text_size(px(11.))
                    .child(format!(
                        "{} · {:?} · {:.0}%",
                        transfer.label,
                        transfer.status,
                        transfer.progress() * 100.
                    )),
            );
        }
        let mut rates = div().flex().items_center().gap_2();
        if self.rate_editor.is_some() {
            rates = rates
                .p_2()
                .rounded(px(10.))
                .bg(theme.surface.base)
                .border_1()
                .border_color(theme.border.default)
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .child("FPS / kbps"),
                )
                .child(
                    crate::product_components::component::text_input("pane-fps")
                        .content(self.fps_edit.clone())
                        .on_change({
                            let view = cx.weak_entity();
                            move |text, _, cx| {
                                let _ = view.update(cx, |this, _| this.fps_edit = text.to_string());
                            }
                        }),
                )
                .child(
                    crate::product_components::component::text_input("pane-bitrate")
                        .content(self.bitrate_edit.clone())
                        .on_change({
                            let view = cx.weak_entity();
                            move |text, _, cx| {
                                let _ =
                                    view.update(cx, |this, _| this.bitrate_edit = text.to_string());
                            }
                        }),
                )
                .child(
                    Button::new("apply-pane-rates", crate::i18n::tr("Apply"))
                        .variant(ButtonVariant::Primary)
                        .control_size(ControlSize::Sm)
                        .rounded_full()
                        .h(px(28.))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.apply_rates();
                            cx.notify();
                        })),
                );
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2p5()
            .p_3()
            .bg(theme.surface.canvas)
            .text_color(theme.content.primary)
            .font_family(crate::design_system::product_ui_font())
            .text_size(px(12.))
            .child(toolbar)
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(theme.content.secondary)
                    .child(crate::i18n::message(&status)),
            )
            .children(audio_error.map(|err| {
                div()
                    .text_size(px(11.5))
                    .text_color(rose_ink)
                    .child(crate::i18n::message(&err))
            }))
            .when(self.rate_editor.is_some(), |root| root.child(rates))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .child(sidebar)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(tabs)
                            .child(grid),
                    ),
            )
            .child(transfer_line)
    }
}

/// Keyboard focus is not visibility: unfocused tiled windows keep streaming.
pub(crate) fn window_visible(window: &Window) -> bool {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return true;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return true;
    };
    unsafe {
        let view = handle.ns_view.as_ptr() as *mut objc2::runtime::AnyObject;
        let native: *mut objc2::runtime::AnyObject = objc2::msg_send![view, window];
        if native.is_null() {
            return true;
        }
        let minimized: bool = objc2::msg_send![native, isMiniaturized];
        let visible: bool = objc2::msg_send![native, isVisible];
        !minimized && visible
    }
}
