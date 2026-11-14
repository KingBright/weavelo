use protocol::gamepad::button as gp_btn;
use protocol::session::{
    CaptureSource, CaptureSourceInfo, ConsoleSummary, PairConsoleRequest, PairConsoleResponse,
};
use protocol::{ControlMessage, GamepadFeedback, InputEvent};
use ps_remote::ddp::{DDP_PORT_PS5, ConsolePowerState, DiscoveredConsole, discover_ps5, probe_ps5};
use ps_remote::feedback::{Ps5ControllerState, TouchPoint as Ps5TouchPoint, button as ps_btn};
use ps_remote::psn_auth::resolve_psn_account;
use ps_remote::regist::pair_with_ps5;
use ps_remote::session::{Ps5MediaEvent, Ps5SessionConfig, Ps5SessionHandle, start_ps5_session};
use ps_remote::store::{ConsoleStore, PairedConsole, console_id_from_host_id};
use remote_core::InputInjector;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::Write as _;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
use std::sync::atomic::Ordering::Relaxed;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
use tokio::sync::broadcast;

static ACTIVE_CONSOLES: OnceLock<Mutex<HashMap<u64, Ps5SessionHandle>>> = OnceLock::new();

fn active_consoles() -> &'static Mutex<HashMap<u64, Ps5SessionHandle>> {
    ACTIVE_CONSOLES.get_or_init(|| Mutex::new(HashMap::new()))
}

struct ActiveConsoleGuard(u64);

impl Drop for ActiveConsoleGuard {
    fn drop(&mut self) {
        if let Ok(mut map) = active_consoles().lock() {
            map.remove(&self.0);
        }
    }
}

pub(crate) fn store_path() -> PathBuf {
    if let Some(override_path) = std::env::var_os("WEAVELO_PS5_STORE_PATH").filter(|s| !s.is_empty())
    {
        return PathBuf::from(override_path);
    }
    remote_core::mesh::default_app_private_mesh_dir().join("ps5_consoles.json")
}

/// Enumerates paired PS5 consoles from a store path as selectable `CaptureSource::Console` entries.
pub(crate) fn list_capture_sources_from(path: &std::path::Path) -> Vec<CaptureSourceInfo> {
    let Ok(store) = ConsoleStore::load(path) else {
        return Vec::new();
    };
    store
        .list()
        .iter()
        .map(|console| CaptureSourceInfo {
            source: CaptureSource::Console(console.console_id),
            title: format!("PS5 · {}", console.nickname),
            application: "PlayStation 5".to_string(),
            process_id: None,
            width: 1920,
            height: 1080,
            supports_input: true,
            camera: None,
        })
        .collect()
}

/// Enumerates paired PS5 consoles as selectable `CaptureSource::Console` entries.
pub fn list_capture_sources_blocking() -> Vec<CaptureSourceInfo> {
    list_capture_sources_from(&store_path())
}

fn ddp_state_label(state: ConsolePowerState) -> &'static str {
    match state {
        ConsolePowerState::Ready => "ready",
        ConsolePowerState::Standby => "standby",
        ConsolePowerState::Unknown(_) => "unknown",
    }
}

/// Discovers LAN PS5 consoles via DDP (`9302`) and merges live power state with paired credentials.
pub async fn list_consoles() -> Vec<ConsoleSummary> {
    let path = store_path();
    let mut store = ConsoleStore::load(&path).unwrap_or_else(|_| {
        ConsoleStore::load(std::env::temp_dir().join("weavelo_ps5_empty.json"))
            .expect("missing store path still creates empty store")
    });

    let extra_targets: Vec<SocketAddr> = store
        .list()
        .iter()
        .filter_map(|c| c.host_addr.map(|ip| SocketAddr::new(ip, DDP_PORT_PS5)))
        .collect();

    let discovered = discover_ps5(Duration::from_millis(450), &extra_targets)
        .await
        .unwrap_or_default();

    let mut by_host_id: HashMap<String, DiscoveredConsole> = HashMap::new();
    for item in discovered {
        by_host_id.insert(item.host_id.to_ascii_uppercase(), item);
    }

    let mut store_dirty = false;
    let mut summaries = Vec::with_capacity(store.list().len() + by_host_id.len());
    let paired_ids: Vec<u64> = store.list().iter().map(|c| c.console_id).collect();

    for id in paired_ids {
        let Some(mut paired) = store.get(id).cloned() else {
            continue;
        };
        let key = paired.host_id.to_ascii_uppercase();
        let status = if let Some(live) = by_host_id.remove(&key) {
            if paired.host_addr != Some(live.ip) {
                paired.host_addr = Some(live.ip);
                store_dirty = true;
            }
            if !live.host_name.is_empty() && paired.nickname == "PlayStation 5" {
                paired.nickname = live.host_name.clone();
                store_dirty = true;
            }
            store.upsert(paired.clone());
            ddp_state_label(live.state).to_string()
        } else {
            "offline".to_string()
        };

        summaries.push(ConsoleSummary {
            console_id: paired.console_id,
            host_id: paired.host_id,
            nickname: paired.nickname,
            host_addr: paired.host_addr.map(|ip| ip.to_string()),
            status,
            paired: true,
        });
    }

    if store_dirty {
        let _ = store.save();
    }

    let mut unpaired: Vec<ConsoleSummary> = by_host_id
        .into_values()
        .map(|live| ConsoleSummary {
            console_id: console_id_from_host_id(&live.host_id),
            host_id: live.host_id,
            nickname: if live.host_name.is_empty() {
                "PlayStation 5".to_string()
            } else {
                live.host_name
            },
            host_addr: Some(live.ip.to_string()),
            status: ddp_state_label(live.state).to_string(),
            paired: false,
        })
        .collect();
    unpaired.sort_by(|a, b| a.nickname.cmp(&b.nickname).then(a.host_id.cmp(&b.host_id)));
    summaries.extend(unpaired);
    summaries
}

fn format_mac_host_id(mac: &[u8; 6]) -> String {
    let mut out = String::with_capacity(12);
    for b in mac {
        let _ = write!(&mut out, "{b:02X}");
    }
    out
}

/// Pairs a PS5 console using Sony OAuth sign-in (`redirect URL` / `code` / Base64 `AccountID`) + 8-digit PIN.
pub async fn pair_console(req: PairConsoleRequest) -> PairConsoleResponse {
    match pair_console_inner(req).await {
        Ok(console) => PairConsoleResponse {
            ok: true,
            console: Some(console),
            error: None,
        },
        Err(err) => PairConsoleResponse {
            ok: false,
            console: None,
            error: Some(err),
        },
    }
}

async fn pair_console_inner(req: PairConsoleRequest) -> Result<ConsoleSummary, String> {
    let pin_digits: String = req.pin.chars().filter(|c| c.is_ascii_digit()).collect();
    if pin_digits.len() != 8 {
        return Err("Enter the 8-digit Remote Play Link Device PIN from your PS5".to_string());
    }
    let pin: u32 = pin_digits
        .parse()
        .map_err(|_| "PS5 PIN must contain 8 digits".to_string())?;

    let account = resolve_psn_account(&req.psn_redirect_or_account)
        .await
        .map_err(|e| format!("PlayStation OAuth account resolution failed: {e}"))?;

    let requested_ip: Option<IpAddr> = match req
        .host_addr
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(raw) => Some(
            raw.parse::<IpAddr>()
                .map_err(|_| format!("Invalid PS5 IP address: {raw}"))?,
        ),
        None => None,
    };

    let extra_targets: Vec<SocketAddr> = requested_ip
        .into_iter()
        .map(|ip| SocketAddr::new(ip, DDP_PORT_PS5))
        .collect();
    let discovered = discover_ps5(Duration::from_millis(1200), &extra_targets)
        .await
        .unwrap_or_default();

    let matched_discovery = match requested_ip {
        Some(ip) => match discovered.into_iter().find(|c| c.ip == ip) {
            Some(found) => Some(found),
            None => probe_ps5(ip, Duration::from_millis(650))
                .await
                .ok()
                .flatten(),
        },
        None => discovered.into_iter().next(),
    };

    let target_ip = requested_ip
        .or_else(|| matched_discovery.as_ref().map(|d| d.ip))
        .ok_or_else(|| {
            "No PS5 found on LAN; enter its IP address manually or verify Remote Play is enabled"
                .to_string()
        })?;

    let regist = pair_with_ps5(target_ip, account.account_id, pin)
        .await
        .map_err(|e| format!("PS5 pairing failed: {e}"))?;

    let host_id = matched_discovery
        .as_ref()
        .map(|d| d.host_id.clone())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format_mac_host_id(&regist.ps5_mac));
    let discovery_status = matched_discovery
        .as_ref()
        .map(|d| ddp_state_label(d.state).to_string())
        .unwrap_or_else(|| "ready".to_string());

    let mut paired = regist.into_paired_console(host_id, account.account_id);
    if let Some(custom_name) = req
        .nickname_override
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        paired.nickname = custom_name.to_string();
    }

    let path = store_path();
    let mut store = ConsoleStore::load(&path)
        .map_err(|e| format!("Failed to open PS5 console store: {e}"))?;
    store.upsert(paired.clone());
    store
        .save()
        .map_err(|e| format!("Failed to save paired PS5 console: {e}"))?;

    Ok(ConsoleSummary {
        console_id: paired.console_id,
        host_id: paired.host_id,
        nickname: paired.nickname,
        host_addr: paired.host_addr.map(|ip| ip.to_string()),
        status: discovery_status,
        paired: true,
    })
}

/// Removes a paired PS5 console from storage and returns the refreshed console list.
pub async fn remove_console(console_id: u64) -> Vec<ConsoleSummary> {
    let path = store_path();
    if let Ok(mut store) = ConsoleStore::load(&path)
        && store.remove(console_id).is_some()
    {
        let _ = store.save();
    }
    list_consoles().await
}

#[derive(Default)]
struct ControllerFallbackState {
    gamepad: Option<Ps5ControllerState>,
    held_keys: HashSet<u16>,
    held_scancodes: HashSet<u32>,
    held_mouse: u8,
    mouse_right_stick: (i16, i16),
    last_mouse_at: Option<Instant>,
}

impl ControllerFallbackState {
    fn recompute(&mut self) -> Ps5ControllerState {
        let mut out = self.gamepad.clone().unwrap_or_default();

        // Virtual key fallback (GPUI / macOS keycodes + Linux evdev scancodes)
        let has_key = |mac_code: u16, evdev_code: u32| {
            self.held_keys.contains(&mac_code) || self.held_scancodes.contains(&evdev_code)
        };

        let mut lx = i32::from(out.left_x);
        let mut ly = i32::from(out.left_y);
        if has_key(0x00, 30) {
            lx -= 32767;
        }
        if has_key(0x02, 32) {
            lx += 32767;
        }
        if has_key(0x0D, 17) {
            ly -= 32767;
        }
        if has_key(0x01, 31) {
            ly += 32767;
        }
        out.left_x = lx.clamp(-32767, 32767) as i16;
        out.left_y = ly.clamp(-32767, 32767) as i16;

        let mut rx = i32::from(out.right_x);
        let mut ry = i32::from(out.right_y);
        if has_key(0x7B, 105) {
            rx -= 32767;
        }
        if has_key(0x7C, 106) {
            rx += 32767;
        }
        if has_key(0x7E, 103) {
            ry -= 32767;
        }
        if has_key(0x7D, 108) {
            ry += 32767;
        }
        if let Some(ts) = self.last_mouse_at {
            if ts.elapsed() <= Duration::from_millis(80) {
                rx += i32::from(self.mouse_right_stick.0);
                ry += i32::from(self.mouse_right_stick.1);
            } else {
                self.mouse_right_stick = (0, 0);
                self.last_mouse_at = None;
            }
        }
        out.right_x = rx.clamp(-32767, 32767) as i16;
        out.right_y = ry.clamp(-32767, 32767) as i16;

        if has_key(0x31, 57) || has_key(0x24, 28) {
            out.buttons |= ps_btn::CROSS;
        }
        if has_key(0x35, 1) || has_key(0x33, 14) {
            out.buttons |= ps_btn::MOON;
        }
        if has_key(0x0E, 18) {
            out.buttons |= ps_btn::BOX;
        }
        if has_key(0x0C, 16) {
            out.buttons |= ps_btn::PYRAMID;
        }
        if has_key(0x38, 42) || has_key(0x3C, 54) {
            out.buttons |= ps_btn::L1;
        }
        if has_key(0x0F, 19) || (self.held_mouse & (1 << 2)) != 0 {
            out.buttons |= ps_btn::R1;
        }
        if has_key(0x12, 2) || (self.held_mouse & (1 << 1)) != 0 {
            out.l2_state = 255;
        }
        if has_key(0x14, 4) || (self.held_mouse & (1 << 0)) != 0 {
            out.r2_state = 255;
        }
        if has_key(0x30, 15) {
            out.buttons |= ps_btn::TOUCHPAD;
        }
        if has_key(0x04, 35) {
            out.buttons |= ps_btn::PS;
        }
        if has_key(0x1F, 24) {
            out.buttons |= ps_btn::OPTIONS;
        }
        if has_key(0x08, 46) {
            out.buttons |= ps_btn::SHARE;
        }
        out
    }
}

fn gamepad_to_ps5_state(gs: &protocol::GamepadState) -> Ps5ControllerState {
    let mut buttons = 0u32;
    let map = [
        (gp_btn::CROSS, ps_btn::CROSS),
        (gp_btn::CIRCLE, ps_btn::MOON),
        (gp_btn::SQUARE, ps_btn::BOX),
        (gp_btn::TRIANGLE, ps_btn::PYRAMID),
        (gp_btn::DPAD_LEFT, ps_btn::DPAD_LEFT),
        (gp_btn::DPAD_RIGHT, ps_btn::DPAD_RIGHT),
        (gp_btn::DPAD_UP, ps_btn::DPAD_UP),
        (gp_btn::DPAD_DOWN, ps_btn::DPAD_DOWN),
        (gp_btn::L1, ps_btn::L1),
        (gp_btn::R1, ps_btn::R1),
        (gp_btn::L3, ps_btn::L3),
        (gp_btn::R3, ps_btn::R3),
        (gp_btn::OPTIONS, ps_btn::OPTIONS),
        (gp_btn::SHARE, ps_btn::SHARE),
        (gp_btn::TOUCHPAD, ps_btn::TOUCHPAD),
        (gp_btn::PS, ps_btn::PS),
    ];
    for &(src, dst) in &map {
        if (gs.buttons & src) != 0 {
            buttons |= dst;
        }
    }

    let mut touches = [Ps5TouchPoint::default(), Ps5TouchPoint::default()];
    for (dst, src) in touches.iter_mut().zip(gs.touches.iter()) {
        if src.active {
            *dst = Ps5TouchPoint {
                id: i16::from(src.id & 0x7f),
                x: src.x.min(1919),
                y: src.y.min(941),
            };
        }
    }

    Ps5ControllerState {
        buttons,
        l2_state: gs.left_trigger,
        r2_state: gs.right_trigger,
        left_x: gs.left_stick_x,
        left_y: gs.left_stick_y,
        right_x: gs.right_stick_x,
        right_y: gs.right_stick_y,
        touches,
        gyro_x: if gs.has_imu { gs.gyro[0] } else { 0.0 },
        gyro_y: if gs.has_imu { gs.gyro[1] } else { 0.0 },
        gyro_z: if gs.has_imu { gs.gyro[2] } else { 0.0 },
        accel_x: if gs.has_imu { gs.accel[0] } else { 0.0 },
        accel_y: if gs.has_imu { gs.accel[1] } else { 0.0 },
        accel_z: if gs.has_imu { gs.accel[2] } else { 0.0 },
        quat: if gs.has_imu {
            gs.quat
        } else {
            [0.0, 0.0, 0.0, 1.0]
        },
    }
}

/// Routes `weavelo` universal/DualSense gamepad events & keyboard fallback into an active PS5 session.
pub(crate) struct ConsoleInputInjector {
    console_id: u64,
    state: Mutex<ControllerFallbackState>,
}

impl ConsoleInputInjector {
    pub(crate) fn new(console_id: u64) -> Self {
        Self {
            console_id,
            state: Mutex::new(ControllerFallbackState::default()),
        }
    }

    fn apply_event(&self, event: InputEvent) -> Ps5ControllerState {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        match event {
            InputEvent::Gamepad(gs) => {
                st.gamepad = Some(gamepad_to_ps5_state(&gs));
            }
            InputEvent::Key {
                key_code, pressed, ..
            } => {
                if pressed {
                    st.held_keys.insert(key_code);
                } else {
                    st.held_keys.remove(&key_code);
                }
            }
            InputEvent::KeyDown(scancode) => {
                st.held_scancodes.insert(scancode);
            }
            InputEvent::KeyUp(scancode) => {
                st.held_scancodes.remove(&scancode);
            }
            InputEvent::MouseDown(btn) if btn < 8 => {
                st.held_mouse |= 1 << btn;
            }
            InputEvent::MouseUp(btn) if btn < 8 => {
                st.held_mouse &= !(1 << btn);
            }
            InputEvent::MouseMove { dx, dy } => {
                st.mouse_right_stick = (
                    (dx.saturating_mul(600)).clamp(-32767, 32767) as i16,
                    (dy.saturating_mul(600)).clamp(-32767, 32767) as i16,
                );
                st.last_mouse_at = Some(Instant::now());
            }
            _ => {}
        }
        st.recompute()
    }

    fn push_state(&self, ps5_state: Ps5ControllerState) {
        if let Ok(map) = active_consoles().lock()
            && let Some(handle) = map.get(&self.console_id)
        {
            handle.send_controller_state(ps5_state);
        }
    }
}

impl InputInjector for ConsoleInputInjector {
    fn inject_input(&self, event: InputEvent) -> Result<(), Box<dyn Error + Send + Sync>> {
        let ps5_state = self.apply_event(event);
        self.push_state(ps5_state);
        Ok(())
    }

    fn release_all_input(&self) {
        {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            *st = ControllerFallbackState::default();
        }
        self.push_state(Ps5ControllerState::default());
    }
}

/// Computes exact 48 kHz samples-per-channel in an RFC 6716 Opus packet from its TOC byte.
pub(crate) fn opus_packet_samples_48k(packet: &[u8]) -> u32 {
    let Some(&toc) = packet.first() else {
        return 480;
    };
    let config = toc >> 3;
    let frame_samples_48k: u32 = match config {
        0 | 4 | 8 | 12 => 480,
        1 | 5 | 9 | 13 => 960,
        2 | 6 | 10 | 14 => 1920,
        3 | 7 | 11 | 15 => 2880,
        16 | 20 | 24 | 28 => 120,
        17 | 21 | 25 | 29 => 240,
        18 | 22 | 26 | 30 => 480,
        19 | 23 | 27 | 31 => 960,
        _ => 480,
    };
    let frame_count = match toc & 0x03 {
        0 => 1,
        1 | 2 => 2,
        _ => packet
            .get(1)
            .map(|b| u32::from(b & 0x3f).max(1))
            .unwrap_or(1),
    };
    frame_samples_48k.saturating_mul(frame_count)
}

/// Returns true if an Annex-B HEVC access unit already contains a `VPS_NUT` (NAL type 32).
pub(crate) fn contains_hevc_vps(annex_b: &[u8]) -> bool {
    let Ok((_, data)) = protocol::viewport::split_frame(annex_b) else {
        return false;
    };
    let mut i = 0usize;
    while i + 4 <= data.len() {
        let sc_len = if data[i..].starts_with(&[0, 0, 0, 1]) {
            4
        } else if i + 3 <= data.len() && data[i..].starts_with(&[0, 0, 1]) {
            3
        } else {
            i += 1;
            continue;
        };
        if let Some(&b0) = data.get(i + sc_len)
            && ((b0 >> 1) & 0x3f) == 32
        {
            return true;
        }
        i += sc_len + 1;
    }
    false
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
pub(crate) async fn run_ps5_streaming(
    console_id: u64,
    config: crate::StreamingRunConfig,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let crate::StreamingRunConfig {
        session_id,
        client_addr,
        width,
        height,
        fps,
        bitrate_kbps,
        udp_sender,
        stats,
        resources,
        mut cancel_rx,
        scheduled_sender,
        stream_settings_rx,
        keyframe_requested,
        video_sequence,
        ..
    } = config;

    let mut stream_settings_rx = stream_settings_rx;
    let path = store_path();
    let mut store = ConsoleStore::load(&path)?;
    let paired = store
        .get(console_id)
        .cloned()
        .ok_or_else(|| format!("Paired PS5 console {console_id} not found"))?;
    let previous_ip = paired.host_addr;

    let (target_w, target_h) = if height >= 900 || width >= 1600 {
        (1920, 1080)
    } else if height >= 540 || width >= 960 {
        (1280, 720)
    } else {
        (960, 540)
    };
    let mut ps_cfg = Ps5SessionConfig::from_console(paired.clone());
    ps_cfg.width = target_w;
    ps_cfg.height = target_h;
    ps_cfg.max_fps = if fps >= 50 { 60 } else { 30 };
    ps_cfg.bw_kbps_sent = bitrate_kbps.clamp(1_000, 50_000);

    let (handle, mut events_rx) = start_ps5_session(ps_cfg).await?;
    if previous_ip != Some(handle.host) {
        let mut updated = paired;
        updated.host_addr = Some(handle.host);
        store.upsert(updated);
        let _ = store.save();
    }

    if let Ok(mut map) = active_consoles().lock() {
        map.insert(console_id, handle.clone());
    }
    let _active_guard = ActiveConsoleGuard(console_id);

    let audio_stream_id = protocol::remote_system_audio_stream_id(session_id);
    let mut sample_rate = handle.audio_config.sample_rate.max(8_000);
    let mut channels = u16::from(handle.audio_config.channels.max(1));
    let mut frame_ms = u16::try_from(
        ((u64::from(handle.audio_config.frame_size.max(1)) * 1000) / u64::from(sample_rate))
            .clamp(5, 60),
    )
    .unwrap_or(10);
    let mut audio_stream_cfg =
        protocol::AudioStreamConfig::remote_system(audio_stream_id, sample_rate, channels, frame_ms);
    if let Ok(size) = crate::send_audio_stream_config(
        &udp_sender,
        Some(&scheduled_sender),
        &audio_stream_cfg,
        client_addr,
        true,
    )
    .await
        && size > 0
    {
        stats.udp_packets_sent.fetch_add(1, Relaxed);
        stats.udp_bytes_sent.fetch_add(size, Relaxed);
    }

    let mut video_headers = handle.video_headers.clone();
    let mut sent_first_frame = false;
    let mut last_stream_index: Option<u8> = None;
    let mut paused = stream_settings_rx
        .as_ref()
        .is_some_and(|rx| rx.borrow().paused);
    let mut awaiting_resume_keyframe = false;
    let mut audio_seq: u16 = 0;
    let mut audio_ts: u32 = 0;

    let mut last_fps_update = Instant::now();
    let mut frames_since_update: u32 = 0;
    let mut bytes_since_update: u64 = 0;
    let mut telemetry_tick = tokio::time::interval(Duration::from_secs(1));
    telemetry_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut audio_cfg_repeat = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(2),
        Duration::from_secs(2),
    );
    audio_cfg_repeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let outcome: Result<(), Box<dyn Error + Send + Sync>> = loop {
        tokio::select! {
            biased;
            res = cancel_rx.recv() => {
                match res {
                    Ok(()) | Err(broadcast::error::RecvError::Closed) => break Ok(()),
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
            changed = async {
                if let Some(rx) = &mut stream_settings_rx {
                    rx.changed().await.map(|_| rx.borrow().paused)
                } else {
                    std::future::pending().await
                }
            } => {
                if let Ok(next_paused) = changed && next_paused != paused {
                    paused = next_paused;
                    if !paused {
                        awaiting_resume_keyframe = true;
                        handle.request_idr();
                    }
                    last_fps_update = Instant::now();
                    frames_since_update = 0;
                    bytes_since_update = 0;
                }
            }
            _ = audio_cfg_repeat.tick(), if !paused => {
                if let Ok(size) = crate::send_audio_stream_config(
                    &udp_sender,
                    Some(&scheduled_sender),
                    &audio_stream_cfg,
                    client_addr,
                    true,
                ).await && size > 0 {
                    stats.udp_packets_sent.fetch_add(1, Relaxed);
                    stats.udp_bytes_sent.fetch_add(size, Relaxed);
                }
            }
            _ = telemetry_tick.tick() => {
                let now = Instant::now();
                let elapsed = now.duration_since(last_fps_update).as_secs_f32();
                if elapsed >= 0.5 {
                    let _ = udp_sender.send_control(&ControlMessage::HostTelemetry {
                        session_id,
                        fps: frames_since_update as f32 / elapsed,
                        encode_latency_ms: 0.0,
                        jitter_ms: 0.0,
                        bitrate_kbps: (bytes_since_update as f64 * 8.0 / 1000.0 / f64::from(elapsed)) as u32,
                    }, client_addr).await;
                    last_fps_update = now;
                    frames_since_update = 0;
                    bytes_since_update = 0;
                }
            }
            ev = events_rx.recv() => {
                let Some(ev) = ev else {
                    break Err("PS5 session event channel closed".into());
                };
                match ev {
                    Ps5MediaEvent::AudioConfig(cfg) => {
                        sample_rate = cfg.sample_rate.max(8_000);
                        channels = u16::from(cfg.channels.max(1));
                        frame_ms = u16::try_from(
                            ((u64::from(cfg.frame_size.max(1)) * 1000) / u64::from(sample_rate))
                                .clamp(5, 60),
                        ).unwrap_or(10);
                        audio_stream_cfg = protocol::AudioStreamConfig::remote_system(
                            audio_stream_id, sample_rate, channels, frame_ms,
                        );
                        let _ = crate::send_audio_stream_config(
                            &udp_sender,
                            Some(&scheduled_sender),
                            &audio_stream_cfg,
                            client_addr,
                            true,
                        ).await;
                    }
                    Ps5MediaEvent::VideoHeaders(headers) => {
                        video_headers = headers;
                        handle.request_idr();
                    }
                    Ps5MediaEvent::VideoFrame { adaptive_stream_index, data, .. } => {
                        let evicted = scheduled_sender.take_video_evictions() > 0;
                        if keyframe_requested.swap(false, Relaxed) || evicted {
                            handle.request_idr();
                        }
                        if paused || data.is_empty() {
                            continue;
                        }
                        let is_idr = remote_core::media_plane::is_hevc_keyframe(&data);
                        if awaiting_resume_keyframe && !is_idr {
                            continue;
                        }
                        awaiting_resume_keyframe = false;

                        let needs_header = !sent_first_frame
                            || last_stream_index != Some(adaptive_stream_index)
                            || is_idr;
                        let payload = if needs_header && !contains_hevc_vps(&data) {
                            if let Some(hdr) = video_headers
                                .get(usize::from(adaptive_stream_index))
                                .or_else(|| video_headers.first())
                            {
                                let mut combined = Vec::with_capacity(hdr.len() + data.len());
                                combined.extend_from_slice(hdr);
                                combined.extend_from_slice(&data);
                                combined
                            } else {
                                data
                            }
                        } else {
                            data
                        };
                        sent_first_frame = true;
                        last_stream_index = Some(adaptive_stream_index);

                        stats.video_frames_captured.fetch_add(1, Relaxed);
                        stats.video_frames_encoded.fetch_add(1, Relaxed);
                        resources.captured_frames.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        resources.encoded_frames.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        stats.video_bytes_encoded.fetch_add(payload.len() as u64, Relaxed);
                        if is_idr {
                            stats.video_keyframes_encoded.fetch_add(1, Relaxed);
                        }
                        bytes_since_update += payload.len() as u64;
                        frames_since_update += 1;

                        let now_us = remote_core::timing::quanta_now_us();
                        let mut timing = protocol::FrameTimingCheckpoints::new(now_us);
                        timing.packetize_ts_us = 1;
                        timing.send_ts_us = 1;

                        let packet = protocol::RtpPacket {
                            header: protocol::RtpHeader {
                                version: 2,
                                payload_type: 96,
                                sequence_number: video_sequence.fetch_add(1, Relaxed),
                                timestamp: crate::now_ms() as u32,
                                ssrc: session_id,
                            },
                            payload,
                        };
                        let send_started = Instant::now();
                        let packet_size = crate::send_media_packet(
                            &udp_sender,
                            Some(&scheduled_sender),
                            &packet,
                            Some(timing),
                            client_addr,
                            true,
                        ).await?;
                        resources.media_send.record(send_started.elapsed());
                        if packet_size > 0 {
                            stats.udp_packets_sent.fetch_add(1, Relaxed);
                            stats.udp_bytes_sent.fetch_add(packet_size, Relaxed);
                        }
                    }
                    Ps5MediaEvent::AudioFrame(data) => {
                        if paused || data.is_empty() {
                            continue;
                        }
                        let step = opus_packet_samples_48k(&data);
                        stats.audio_frames_captured.fetch_add(1, Relaxed);
                        stats.audio_frames_encoded.fetch_add(1, Relaxed);
                        stats.audio_bytes_encoded.fetch_add(data.len() as u64, Relaxed);

                        let packet = protocol::RtpPacket {
                            header: protocol::RtpHeader {
                                version: 2,
                                payload_type: 97,
                                sequence_number: audio_seq,
                                timestamp: audio_ts,
                                ssrc: audio_stream_id,
                            },
                            payload: data,
                        };
                        audio_seq = audio_seq.wrapping_add(1);
                        audio_ts = audio_ts.wrapping_add(step);

                        let packet_size = crate::send_media_packet(
                            &udp_sender,
                            Some(&scheduled_sender),
                            &packet,
                            None,
                            client_addr,
                            true,
                        ).await?;
                        if packet_size > 0 {
                            stats.udp_packets_sent.fetch_add(1, Relaxed);
                            stats.udp_bytes_sent.fetch_add(packet_size, Relaxed);
                        }
                    }
                    Ps5MediaEvent::Rumble(r) => {
                        let msg = ControlMessage::GamepadFeedback(Box::new(GamepadFeedback::Rumble {
                            controller_id: 0,
                            left_large: r.left,
                            right_small: r.right,
                        }));
                        let _ = udp_sender.send_control(&msg, client_addr).await;
                    }
                    Ps5MediaEvent::TriggerEffects(t) => {
                        let msg = ControlMessage::GamepadFeedback(Box::new(
                            GamepadFeedback::TriggerEffects {
                                controller_id: 0,
                                left: t.left,
                                right: t.right,
                            },
                        ));
                        let _ = udp_sender.send_control(&msg, client_addr).await;
                    }
                    Ps5MediaEvent::AudioHaptics(payload) => {
                        if !payload.is_empty() {
                            let msg = ControlMessage::GamepadFeedback(Box::new(
                                GamepadFeedback::AudioHaptics {
                                    controller_id: 0,
                                    payload,
                                },
                            ));
                            let _ = udp_sender.send_control(&msg, client_addr).await;
                        }
                    }
                    Ps5MediaEvent::Disconnected(reason) => {
                        break Err(format!("PS5 Remote Play disconnected: {reason}").into());
                    }
                }
            }
        }
    };

    handle.close().await;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{ControllerKind, GamepadState, TouchPoint};
    use std::net::Ipv4Addr;

    #[test]
    fn opus_toc_parses_standard_48k_durations() {
        // config=2 (SILK WB 20ms=960) or config=31 (CELT FB 20ms=960), code=0 (1 frame)
        assert_eq!(opus_packet_samples_48k(&[31 << 3]), 960);
        // config=30 (CELT FB 10ms=480), code=0
        assert_eq!(opus_packet_samples_48k(&[30 << 3]), 480);
        // config=30, code=1 (2 equal frames) -> 960
        assert_eq!(opus_packet_samples_48k(&[(30 << 3) | 1]), 960);
        // empty falls back to 480 (10ms)
        assert_eq!(opus_packet_samples_48k(&[]), 480);
    }

    #[test]
    fn detects_inline_hevc_vps_across_start_codes() {
        // NAL type 32 -> (32 << 1) = 0x40
        assert!(contains_hevc_vps(&[0, 0, 0, 1, 0x40, 0x01, 0xaa]));
        assert!(contains_hevc_vps(&[0, 0, 1, 0x40, 0x01, 0xaa]));
        // IDR without VPS (NAL type 19 -> (19 << 1) = 0x26)
        assert!(!contains_hevc_vps(&[0, 0, 0, 1, 0x26, 0x01, 0xbb]));
    }

    #[test]
    fn store_catalog_lists_paired_consoles_with_console_source() {
        let path = std::env::temp_dir().join(format!("weavelo_bridge_test_{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let id = console_id_from_host_id("112233445566");
        let mut store = ConsoleStore::load(&path).unwrap();
        store.upsert(PairedConsole {
            console_id: id,
            host_id: "112233445566".to_string(),
            nickname: "LivingRoomPS5".to_string(),
            host_addr: Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 88))),
            account_id: [1; 8],
            rp_regist_key: *b"00001111\0\0\0\0\0\0\0\0",
            rp_key: [0x22; 16],
            rp_key_type: 2,
            ps5_mac: [0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
            paired_at_unix: 100,
        });
        store.save().unwrap();

        let listed = list_capture_sources_from(&path);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].source, CaptureSource::Console(id));
        assert_eq!(listed[0].title, "PS5 · LivingRoomPS5");
        assert!(listed[0].supports_input);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn injector_maps_dualsense_and_kbm_fallback_and_releases_neutral() {
        let inj = ConsoleInputInjector::new(123);
        let gs = GamepadState {
            controller_id: 0,
            kind: ControllerKind::DualSense,
            buttons: gp_btn::CROSS | gp_btn::L1 | gp_btn::PS,
            left_trigger: 128,
            right_trigger: 255,
            left_stick_x: -12345,
            left_stick_y: 23456,
            right_stick_x: 111,
            right_stick_y: -222,
            touches: [
                TouchPoint {
                    active: true,
                    id: 7,
                    x: 800,
                    y: 450,
                },
                TouchPoint::default(),
            ],
            has_imu: true,
            gyro: [0.1, -0.2, 0.3],
            accel: [0.0, 0.98, 0.0],
            quat: [0.0, 0.0, 0.0, 1.0],
        };
        let mapped = inj.apply_event(InputEvent::Gamepad(Box::new(gs)));
        assert_eq!(mapped.buttons, ps_btn::CROSS | ps_btn::L1 | ps_btn::PS);
        assert_eq!(mapped.l2_state, 128);
        assert_eq!(mapped.r2_state, 255);
        assert_eq!(mapped.touches[0].id, 7);
        assert_eq!((mapped.touches[0].x, mapped.touches[0].y), (800, 450));

        // Keyboard overlay (Space + W)
        let kbm = inj.apply_event(InputEvent::KeyDown(17)); // W -> left stick up (-32767)
        assert_eq!(kbm.left_y, -32767 + 23456);

        inj.release_all_input();
        let after = inj.apply_event(InputEvent::KeyUp(17));
        assert_eq!(after, Ps5ControllerState::default());
    }
}
