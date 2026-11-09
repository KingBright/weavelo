use crate::audio_capture::{MacAudioFrame, MacSystemAudioCapturer};
use crate::lifecycle::Resources;
use crate::macos_audio_tap::{MacAudioTapCapturer, private_mode_available, private_mode_claimed};
use async_trait::async_trait;
use protocol::ControlMessage;
use protocol::audio_output::{
    AudioOutputMode, AudioOutputRequest, AudioOutputStatus, LocalPlayback,
};
use protocol::session::{CaptureSource, SessionCommand};
use remote_core::{AudioCapturer, net::UdpSender};
use std::{
    error::Error,
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::watch;

type CaptureError = Box<dyn Error + Send + Sync>;
type ResultCache = Option<(AudioOutputRequest, u64, AudioOutputStatus)>;

#[derive(Clone)]
pub(crate) struct OutputControl {
    request: watch::Sender<Option<AudioOutputRequest>>,
    result: Arc<Mutex<ResultCache>>,
    exclusive: Arc<AtomicBool>,
}

impl OutputControl {
    pub(crate) fn new() -> Self {
        Self {
            request: watch::channel(None).0,
            result: Arc::default(),
            exclusive: Arc::default(),
        }
    }

    pub(crate) fn submitted(&self, request: AudioOutputRequest) -> bool {
        self.request.borrow().is_some_and(|current| {
            current.id == request.id
                && current.source_revision == request.source_revision
                && current.revision >= request.revision
        })
    }

    pub(crate) fn submit(&self, request: AudioOutputRequest) -> Option<(u64, AudioOutputStatus)> {
        if self.submitted(request) {
            let current = *self.request.borrow();
            return self.result.lock().unwrap().as_ref().and_then(
                |(accepted, generation, status)| {
                    (*accepted == request && Some(*accepted) == current)
                        .then(|| (*generation, status.clone()))
                },
            );
        }
        self.request.send_modify(|current| {
            if request.mode == AudioOutputMode::Private {
                self.exclusive.store(true, Ordering::Release);
            }
            *current = Some(request);
        });
        None
    }

    pub(crate) fn has_private_request(&self) -> bool {
        self.exclusive.load(Ordering::Acquire)
    }

    fn record_result(
        &self,
        request: AudioOutputRequest,
        status: AudioOutputStatus,
        tap_released: bool,
    ) -> Option<u64> {
        let current = self.request.borrow();
        if *current != Some(request) {
            return None;
        }
        if tap_released {
            self.exclusive.store(false, Ordering::Release);
        }
        let mut result = self.result.lock().unwrap();
        let generation = match result.as_ref() {
            Some((previous, _, previous_status))
                if *previous == request && *previous_status == status =>
            {
                return None;
            }
            Some((previous, generation, _)) if *previous == request => generation + 1,
            _ => 1,
        };
        *result = Some((request, generation, status));
        Some(generation)
    }
}

pub(crate) struct SystemAudio {
    mirror: MacSystemAudioCapturer,
    tap: Option<MacAudioTapCapturer>,
    control: Option<OutputControl>,
    request: Option<watch::Receiver<Option<AudioOutputRequest>>>,
    active_request: Option<AudioOutputRequest>,
    rejected: Option<AudioOutputRequest>,
    tap_failure: Option<String>,
    resources: Arc<Resources>,
    udp: UdpSender,
    addr: SocketAddr,
}

impl SystemAudio {
    pub(crate) fn new(
        source: CaptureSource,
        resources: Arc<Resources>,
        control: Option<OutputControl>,
        udp: UdpSender,
        addr: SocketAddr,
    ) -> Self {
        Self {
            mirror: MacSystemAudioCapturer::new()
                .for_source(source)
                .with_resources(resources.clone()),
            tap: None,
            request: control.as_ref().map(|control| control.request.subscribe()),
            control,
            active_request: None,
            rejected: None,
            tap_failure: None,
            resources,
            udp,
            addr,
        }
    }

    async fn report(
        &mut self,
        request: AudioOutputRequest,
        playback: LocalPlayback,
        message: Option<String>,
        tap_released: bool,
    ) {
        let Some(control) = &self.control else {
            return;
        };
        let status = AudioOutputStatus {
            requested: request.mode,
            local_playback: playback,
            private_supported: private_mode_available(),
            message,
        };
        let Some(generation) = control.record_result(request, status.clone(), tap_released) else {
            return;
        };
        let command = SessionCommand::AudioOutputState {
            request,
            generation,
            status,
        };
        let _ = self
            .udp
            .send_control(&ControlMessage::Session(Box::new(command)), self.addr)
            .await;
    }

    async fn close_tap(&mut self) -> Result<(), CaptureError> {
        if let Some(tap) = &mut self.tap {
            tap.stop().await?;
            self.tap = None;
        }
        Ok(())
    }

    async fn report_restored(
        &mut self,
        request: AudioOutputRequest,
        message: Option<String>,
        tap_released: bool,
    ) {
        let (playback, message) = if private_mode_claimed() {
            (
                LocalPlayback::Unknown,
                Some("Another viewing session owns private host audio".into()),
            )
        } else {
            (LocalPlayback::Enabled, message)
        };
        self.report(request, playback, message, tap_released).await;
    }
}

#[async_trait]
impl AudioCapturer for SystemAudio {
    type Frame = MacAudioFrame;

    async fn start(&mut self) -> Result<(), CaptureError> {
        self.active_request = self
            .request
            .as_mut()
            .and_then(|request| *request.borrow_and_update());
        if let Some(request) = self.active_request.filter(|request| {
            request.mode == AudioOutputMode::Private && self.rejected != Some(*request)
        }) {
            // Store the native owner before awaiting start so cancellation
            // still runs its destruction and connection lifecycle receipt.
            self.tap = Some(MacAudioTapCapturer::new(self.resources.clone()));
            match self.tap.as_mut().unwrap().start().await {
                Ok(()) => {
                    self.report(request, LocalPlayback::Disabled, None, false)
                        .await;
                    return Ok(());
                }
                Err(error) => {
                    self.rejected = Some(request);
                    if let Err(stop_error) = self.close_tap().await {
                        self.report(
                            request,
                            LocalPlayback::Unknown,
                            Some(stop_error.to_string()),
                            false,
                        )
                        .await;
                        return Err(stop_error);
                    }
                    self.report(
                        request,
                        LocalPlayback::Unknown,
                        Some(error.to_string()),
                        true,
                    )
                    .await;
                }
            }
        } else if let Some(request) = self
            .active_request
            .filter(|request| request.mode == AudioOutputMode::Mirror)
        {
            self.report_restored(request, None, true).await;
        }
        self.mirror.start().await
    }

    async fn stop(&mut self) -> Result<(), CaptureError> {
        let owned = self.tap.is_some();
        let stopped = self.close_tap().await;
        if owned && let Some(request) = self.active_request {
            if let Err(error) = &stopped {
                self.report(
                    request,
                    LocalPlayback::Unknown,
                    Some(error.to_string()),
                    false,
                )
                .await;
            } else {
                let failure = self.tap_failure.take();
                if failure.is_some() {
                    self.rejected = Some(request);
                }
                self.report_restored(request, failure, self.rejected == Some(request))
                    .await;
            }
        }
        stopped?;
        self.mirror.stop().await
    }

    async fn pause(&mut self) -> Result<(), CaptureError> {
        if self.tap.is_some() {
            self.stop().await
        } else {
            self.mirror.pause().await
        }
    }

    async fn resume(&mut self) -> Result<(), CaptureError> {
        if self.active_request.is_some_and(|request| {
            request.mode == AudioOutputMode::Private && self.rejected != Some(request)
        }) {
            return self.start().await;
        }
        self.mirror.resume().await
    }

    async fn capture_frame(&mut self) -> Result<Self::Frame, CaptureError> {
        tokio::select! {
            biased;
            // Do not consume the revision here. The audio loop stops this
            // capture and restarts it with the latest request and same RTP ID.
            _ = async {
                if let Some(request) = &self.request {
                    let mut request = request.clone();
                    let _ = request.changed().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => Err("host audio output changed".into()),
            frame = async {
                if let Some(tap) = &mut self.tap { tap.capture_frame().await }
                else { self.mirror.capture_frame().await }
            } => {
                if self.tap.is_some() && let Err(error) = &frame { self.tap_failure = Some(error.to_string()); }
                frame
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_result_cannot_clear_exclusive_ownership_or_replace_current_receipt() {
        let control = OutputControl::new();
        let mirror = AudioOutputRequest {
            id: 7,
            source_revision: 42,
            revision: 1,
            mode: AudioOutputMode::Mirror,
        };
        let private = AudioOutputRequest {
            revision: 2,
            mode: AudioOutputMode::Private,
            ..mirror
        };
        let enabled = AudioOutputStatus {
            requested: mirror.mode,
            local_playback: LocalPlayback::Enabled,
            private_supported: true,
            message: None,
        };
        let disabled = AudioOutputStatus {
            requested: private.mode,
            local_playback: LocalPlayback::Disabled,
            private_supported: true,
            message: None,
        };
        assert_eq!(control.submit(mirror), None);
        assert_eq!(control.submit(private), None);
        assert!(control.has_private_request());
        assert_eq!(control.record_result(mirror, enabled.clone(), true), None);
        assert_eq!(
            control.record_result(private, disabled.clone(), false),
            Some(1)
        );
        assert_eq!(control.submit(mirror), None);
        assert_eq!(control.submit(private), Some((1, disabled)));
        let mirror = AudioOutputRequest {
            revision: 3,
            ..mirror
        };
        assert_eq!(control.submit(mirror), None);
        assert!(control.has_private_request());
        assert_eq!(
            control.record_result(mirror, enabled.clone(), true),
            Some(1)
        );
        assert_eq!(control.submit(mirror), Some((1, enabled)));
        assert!(!control.has_private_request());
    }
}
