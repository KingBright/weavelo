use std::net::IpAddr;
use std::time::Duration;

use rand::Rng;
use thiserror::Error;
use tokio::sync::mpsc;

use crate::ctrl::{connect_ctrl, CtrlConfig, CtrlError, CtrlHandle};
use crate::ddp::{discover_ps5, wake_and_wait_ready, DdpError};
use crate::feedback::{Ps5ControllerState, RumbleEvent, TriggerEffectsEvent};
use crate::launchspec::LaunchSpecConfig;
use crate::senkusha::{probe_senkusha, SENKUSHA_PORT};
use crate::store::PairedConsole;
use crate::takion::{
    connect_takion, TakionAudioConfig, TakionConfig, TakionError, TakionEvent, TakionHandle,
    TAKION_PORT,
};

#[derive(Debug, Error)]
pub enum Ps5SessionError {
    #[error("PS5 console {0} could not be located on LAN")]
    ConsoleNotFound(String),
    #[error("PS5 discovery/wakeup failed: {0}")]
    Ddp(#[from] DdpError),
    #[error("PS5 ctrl connection failed: {0}")]
    Ctrl(#[from] CtrlError),
    #[error("PS5 Takion stream failed: {0}")]
    Takion(#[from] TakionError),
}

/// Video & streaming profile used to start a PS5 Remote Play session.
#[derive(Debug, Clone)]
pub struct Ps5SessionConfig {
    pub console: PairedConsole,
    pub host_override: Option<IpAddr>,
    pub width: u32,
    pub height: u32,
    pub max_fps: u32,
    pub bw_kbps_sent: u32,
    pub hdr: bool,
    pub wake_if_standby: bool,
    pub wake_timeout: Duration,
}

impl Ps5SessionConfig {
    pub fn from_console(console: PairedConsole) -> Self {
        Self {
            console,
            host_override: None,
            width: 1920,
            height: 1080,
            max_fps: 60,
            bw_kbps_sent: 15_000,
            hdr: false,
            wake_if_standby: true,
            wake_timeout: Duration::from_secs(20),
        }
    }
}

/// Unified events delivered by an active PS5 session.
#[derive(Debug, Clone)]
pub enum Ps5MediaEvent {
    AudioConfig(TakionAudioConfig),
    VideoHeaders(Vec<Vec<u8>>),
    VideoFrame {
        frame_index: u16,
        adaptive_stream_index: u8,
        data: Vec<u8>,
    },
    AudioFrame(Vec<u8>),
    AudioHaptics(Vec<u8>),
    Rumble(RumbleEvent),
    TriggerEffects(TriggerEffectsEvent),
    Disconnected(String),
}

/// Active PS5 Remote Play session orchestrating `/ctrl` (TCP 9295) and `Takion` (UDP 9296).
#[derive(Clone)]
pub struct Ps5SessionHandle {
    ctrl: CtrlHandle,
    takion: TakionHandle,
    pub host: IpAddr,
    pub audio_config: TakionAudioConfig,
    pub video_headers: Vec<Vec<u8>>,
}

impl Ps5SessionHandle {
    pub fn send_controller_state(&self, state: Ps5ControllerState) {
        self.takion.send_controller_state(state);
    }

    pub fn request_idr(&self) {
        self.takion.request_idr();
    }

    pub async fn go_home(&self) -> Result<(), CtrlError> {
        self.ctrl.send_go_home().await
    }

    pub async fn set_microphone(&self, unmuted: bool) -> Result<(), CtrlError> {
        self.ctrl.set_microphone(unmuted).await
    }

    pub async fn standby_and_close(&self) -> Result<(), CtrlError> {
        self.ctrl.send_standby().await?;
        self.close().await;
        Ok(())
    }

    pub async fn close(&self) {
        self.takion.shutdown().await;
        self.ctrl.shutdown().await;
    }
}

async fn resolve_target_ip(config: &Ps5SessionConfig) -> Result<IpAddr, Ps5SessionError> {
    if let Some(ip) = config.host_override.or(config.console.host_addr) {
        return Ok(ip);
    }
    let found = discover_ps5(Duration::from_millis(1200), &[]).await?;
    found
        .into_iter()
        .find(|c| c.host_id == config.console.host_id)
        .map(|c| c.ip)
        .ok_or_else(|| Ps5SessionError::ConsoleNotFound(config.console.nickname.clone()))
}

/// Start a full PS5 Remote Play session:
/// 1. Resolve console IP (or discover on LAN via DDP 9302) and wake from standby
/// 2. Authenticate over TCP 9295 (`/sess/init` + `/sess/ctrl`, enforcing PS5)
/// 3. Probe UDP 9297 Senkusha RTT/MTU (falling back gracefully to 1454 MTU / 2ms)
/// 4. Connect UDP 9296 Takion v12 & exchange ECDH + LaunchSpec
pub async fn start_ps5_session(
    config: Ps5SessionConfig,
) -> Result<(Ps5SessionHandle, mpsc::Receiver<Ps5MediaEvent>), Ps5SessionError> {
    let target_host = resolve_target_ip(&config).await?;

    let active_host = if config.wake_if_standby {
        let info = wake_and_wait_ready(
            target_host,
            &config.console.rp_regist_key,
            config.wake_timeout,
        )
        .await?;
        info.ip
    } else {
        target_host
    };

    let ctrl_cfg = CtrlConfig::new(
        active_host,
        config.console.rp_regist_key,
        config.console.rp_key,
        config.hdr,
    );
    let ctrl_session = connect_ctrl(ctrl_cfg).await?;

    let senkusha = probe_senkusha(active_host, SENKUSHA_PORT, Duration::from_millis(120)).await;

    let mut handshake_key = [0u8; 16];
    rand::rng().fill_bytes(&mut handshake_key);

    let rtt_ms = (senkusha.rtt_us / 1000).max(1) as u32;
    let launch_spec = LaunchSpecConfig {
        width: config.width,
        height: config.height,
        max_fps: config.max_fps,
        bw_kbps_sent: config.bw_kbps_sent,
        mtu: senkusha.mtu_out,
        rtt_ms,
        hdr: config.hdr,
        handshake_key,
    };

    let takion_cfg = TakionConfig {
        host: active_host,
        port: TAKION_PORT,
        session_id: ctrl_session.session_id,
        rpcrypt: ctrl_session.rpcrypt,
        launch_spec,
        senkusha,
    };

    let mut takion_session = match connect_takion(takion_cfg).await {
        Ok(s) => s,
        Err(e) => {
            ctrl_session.handle.shutdown().await;
            return Err(Ps5SessionError::Takion(e));
        }
    };

    let audio_config = takion_session.audio_config;
    let video_headers = takion_session.video_headers.clone();
    let handle = Ps5SessionHandle {
        ctrl: ctrl_session.handle.clone(),
        takion: takion_session.handle.clone(),
        host: active_host,
        audio_config,
        video_headers,
    };

    let (tx, rx) = mpsc::channel(256);
    let mut ctrl_watch = ctrl_session.handle;
    let takion_handle = takion_session.handle;

    tokio::spawn(async move {
        loop {
            tokio::select! {
                ev_opt = takion_session.events.recv() => {
                    match ev_opt {
                        Some(TakionEvent::AudioConfig(c)) => {
                            let _ = tx.send(Ps5MediaEvent::AudioConfig(c)).await;
                        }
                        Some(TakionEvent::VideoHeaders(h)) => {
                            let _ = tx.send(Ps5MediaEvent::VideoHeaders(h)).await;
                        }
                        Some(TakionEvent::VideoFrame { frame_index, adaptive_stream_index, data }) => {
                            let _ = tx.try_send(Ps5MediaEvent::VideoFrame {
                                frame_index,
                                adaptive_stream_index,
                                data,
                            });
                        }
                        Some(TakionEvent::AudioFrame(a)) => {
                            let _ = tx.try_send(Ps5MediaEvent::AudioFrame(a));
                        }
                        Some(TakionEvent::AudioHaptics(h)) => {
                            let _ = tx.try_send(Ps5MediaEvent::AudioHaptics(h));
                        }
                        Some(TakionEvent::Rumble(r)) => {
                            let _ = tx.try_send(Ps5MediaEvent::Rumble(r));
                        }
                        Some(TakionEvent::TriggerEffects(t)) => {
                            let _ = tx.try_send(Ps5MediaEvent::TriggerEffects(t));
                        }
                        Some(TakionEvent::Disconnected(reason)) => {
                            let _ = tx.send(Ps5MediaEvent::Disconnected(reason)).await;
                            break;
                        }
                        None => {
                            let _ = tx.send(Ps5MediaEvent::Disconnected("takion channel closed".into())).await;
                            break;
                        }
                    }
                }
                reason_opt = ctrl_watch.wait_closed() => {
                    let msg = reason_opt.unwrap_or_else(|| "ctrl channel closed".into());
                    takion_handle.shutdown().await;
                    let _ = tx.send(Ps5MediaEvent::Disconnected(msg)).await;
                    break;
                }
            }
        }
    });

    Ok((handle, rx))
}
