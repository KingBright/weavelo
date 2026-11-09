//! The host-wide private Core Audio owner. Setup and destruction run outside Tokio.
use crate::audio_capture::MacAudioFrame;
use crate::lifecycle::{NativeStopCompletion, NativeStopReceipt, Resources};
use async_trait::async_trait;
use protocol::AudioSource;
use remote_core::AudioCapturer;
use std::error::Error;
use std::ffi::{CStr, c_char, c_void};
use std::ptr::NonNull;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

unsafe extern "C" {
    fn weavelo_audio_tap_available() -> bool;
    fn weavelo_audio_tap_create(ready: *mut bool, error: *mut c_char, cap: usize) -> *mut c_void;
    fn weavelo_audio_tap_start(handle: *mut c_void) -> i32;
    fn weavelo_audio_tap_set_private(handle: *mut c_void, private_audio: bool) -> i32;
    fn weavelo_audio_tap_rate(handle: *mut c_void) -> f64;
    fn weavelo_audio_tap_read(
        handle: *mut c_void,
        samples: *mut f32,
        max_frames: u32,
        produced: *mut u32,
    ) -> i32;
    fn weavelo_audio_tap_destroy(handle: *mut c_void) -> i32;
}

type CaptureError = Box<dyn Error + Send + Sync>;
const READ_FRAMES: u32 = 960;
const START_WAIT: Duration = Duration::from_secs(5);
static PRIVATE_AUDIO_CLAIM: AtomicBool = AtomicBool::new(false);

pub(crate) fn private_mode_available() -> bool {
    unsafe { weavelo_audio_tap_available() }
}

pub(crate) fn private_mode_claimed() -> bool {
    PRIVATE_AUDIO_CLAIM.load(Ordering::Acquire)
}

struct PrivateClaim;
impl PrivateClaim {
    fn acquire() -> Result<Self, CaptureError> {
        PRIVATE_AUDIO_CLAIM
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|_| "Another viewing session owns private host audio".into())
    }
}
impl Drop for PrivateClaim {
    fn drop(&mut self) {
        PRIVATE_AUDIO_CLAIM.store(false, Ordering::Release);
    }
}

fn status(op: &str, code: i32) -> Result<(), CaptureError> {
    if code == 0 {
        Ok(())
    } else {
        Err(format!("{op}: CoreAudio OSStatus {code}").into())
    }
}

struct NativeTap(NonNull<c_void>);
// All handle access is serialized by NativeOwner. Its hardware callback only
// accesses the separately synchronized native ring and immutable format.
unsafe impl Send for NativeTap {}

impl NativeTap {
    fn read(&self, samples: &mut [f32]) -> Result<usize, CaptureError> {
        let mut produced = 0;
        status("Convert captured audio", unsafe {
            weavelo_audio_tap_read(
                self.0.as_ptr(),
                samples.as_mut_ptr(),
                READ_FRAMES,
                &mut produced,
            )
        })?;
        Ok(produced as usize * 2)
    }
}

struct NativeState {
    tap: Option<NativeTap>,
    claim: Option<PrivateClaim>,
}
struct NativeOwner {
    cancelled: AtomicBool,
    activate_before: Instant,
    state: Mutex<NativeState>,
}
impl NativeOwner {
    fn start(&self) -> Result<(f64, MacAudioFrame), CaptureError> {
        let mut state = self.state.lock().unwrap();
        self.check_cancelled()?;
        let mut ready = false;
        let mut error = [0 as c_char; 256];
        state.tap = NonNull::new(unsafe {
            weavelo_audio_tap_create(&mut ready, error.as_mut_ptr(), error.len())
        })
        .map(NativeTap);
        if !ready {
            return Err(unsafe { CStr::from_ptr(error.as_ptr()) }
                .to_string_lossy()
                .into_owned()
                .into());
        }
        let tap = state
            .tap
            .as_ref()
            .ok_or("Core Audio returned no tap owner")?;
        let handle = tap.0.as_ptr();
        let rate = unsafe { weavelo_audio_tap_rate(handle) };
        if rate == 0.0 {
            return Err("Core Audio tap format unavailable".into());
        }
        self.check_cancelled()?;
        status("Start system audio recording", unsafe {
            weavelo_audio_tap_start(handle)
        })?;
        let deadline = Instant::now() + Duration::from_millis(900);
        let mut samples = vec![0.0; READ_FRAMES as usize * 2];
        loop {
            self.check_cancelled()?;
            let count = tap.read(&mut samples)?;
            if count != 0 {
                self.check_cancelled()?;
                // Do not suppress speaker output until the native input has
                // started and supplied usable PCM for the remote stream.
                status("Enable private system audio", unsafe {
                    weavelo_audio_tap_set_private(handle, true)
                })?;
                samples.truncate(count);
                return Ok((rate, frame(samples)));
            }
            if Instant::now() >= deadline {
                return Err("Core Audio did not provide system audio samples".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn check_cancelled(&self) -> Result<(), CaptureError> {
        if self.cancelled.load(Ordering::Acquire) || Instant::now() >= self.activate_before {
            Err("Audio tap start cancelled".into())
        } else {
            Ok(())
        }
    }

    fn stop(&self) {
        let mut state = self.state.lock().unwrap();
        if let Some(tap) = state.tap.take() {
            let handle = tap.0.as_ptr();
            // Try to restore output even if stopping the HAL stream needs
            // a retry. Destruction still owns the receipt and exclusive claim.
            unsafe { weavelo_audio_tap_set_private(handle, false) };
            let mut first_failure = true;
            loop {
                let code = unsafe { weavelo_audio_tap_destroy(handle) };
                if code == 0 {
                    break;
                }
                if first_failure {
                    eprintln!(
                        "Core Audio teardown not confirmed (OSStatus {code}); retaining the native owner for retry"
                    );
                    first_failure = false;
                }
                std::thread::sleep(Duration::from_millis(150));
            }
        }
        state.claim.take();
    }
}

pub(crate) struct MacAudioTapCapturer {
    resources: Arc<Resources>,
    owner: Option<Arc<NativeOwner>>,
    pending_stop: Option<NativeStopReceipt>,
    initial_frame: Option<MacAudioFrame>,
    input_rate: f64,
    format_checked: Instant,
}

impl MacAudioTapCapturer {
    pub(crate) fn new(resources: Arc<Resources>) -> Self {
        Self {
            resources,
            owner: None,
            pending_stop: None,
            initial_frame: None,
            input_rate: 0.0,
            format_checked: Instant::now(),
        }
    }

    fn stop_owned(&mut self) {
        self.initial_frame = None;
        if let Some(owner) = self.owner.take() {
            owner.cancelled.store(true, Ordering::Release);
            let (receipt, completion) = NativeStopCompletion::new(Some(self.resources.clone()));
            self.pending_stop = Some(receipt);
            std::thread::spawn(move || {
                owner.stop();
                completion.complete(Ok(()));
            });
        }
    }
}

impl Drop for MacAudioTapCapturer {
    fn drop(&mut self) {
        self.stop_owned();
    }
}

fn frame(samples: Vec<f32>) -> MacAudioFrame {
    MacAudioFrame {
        samples,
        sample_rate: 48_000,
        channels: 2,
        source: AudioSource::RemoteSystem,
    }
}

#[async_trait]
impl AudioCapturer for MacAudioTapCapturer {
    type Frame = MacAudioFrame;

    async fn start(&mut self) -> Result<(), CaptureError> {
        if self.owner.is_some() || self.pending_stop.is_some() {
            return Err("Previous audio tap is still owned".into());
        }
        let owner = Arc::new(NativeOwner {
            cancelled: AtomicBool::new(false),
            activate_before: Instant::now() + START_WAIT,
            state: Mutex::new(NativeState {
                tap: None,
                claim: Some(PrivateClaim::acquire()?),
            }),
        });
        self.resources.native_streams.fetch_add(1, Ordering::SeqCst);
        self.owner = Some(owner.clone());
        let started = tokio::task::spawn_blocking(move || owner.start());
        let (rate, initial) = match tokio::time::timeout(START_WAIT, started).await {
            Ok(result) => result??,
            Err(_) => {
                self.stop_owned();
                return Err("Core Audio did not finish system audio recording setup".into());
            }
        };
        self.input_rate = rate;
        self.format_checked = Instant::now();
        self.initial_frame = Some(initial);
        Ok(())
    }

    async fn stop(&mut self) -> Result<(), CaptureError> {
        self.stop_owned();
        if let Some(receipt) = &self.pending_stop {
            receipt.wait().await?;
            self.pending_stop = None;
        }
        Ok(())
    }

    async fn capture_frame(&mut self) -> Result<Self::Frame, CaptureError> {
        if let Some(initial) = self.initial_frame.take() {
            return Ok(initial);
        }
        let owner = self.owner.as_ref().ok_or("Audio tap is stopped")?;
        let mut samples = vec![0.0; READ_FRAMES as usize * 2];
        loop {
            let count = {
                let state = owner.state.lock().unwrap();
                let tap = state.tap.as_ref().ok_or("Audio tap is stopped")?;
                if self.format_checked.elapsed() >= Duration::from_secs(1) {
                    self.format_checked = Instant::now();
                    if unsafe { weavelo_audio_tap_rate(tap.0.as_ptr()) } != self.input_rate {
                        return Err("Core Audio tap format changed".into());
                    }
                }
                tap.read(&mut samples)?
            };
            if count != 0 {
                samples.truncate(count);
                return Ok(frame(samples));
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}
