use async_trait::async_trait;
use remote_core::{VideoCapturer, VideoFrame, VideoFrameHandleKind};
use screencapturekit::prelude::*;
use std::error::Error;
use std::sync::Arc;

use screencapturekit::cm::CMSampleBuffer;

#[derive(Default)]
struct StreamFailure {error:std::sync::Mutex<Option<String>>,changed:tokio::sync::Notify}
impl StreamFailure {
    fn stop(&self,reason:String){let mut error=self.error.lock().unwrap_or_else(|e|e.into_inner());
        if error.is_none(){*error=Some(reason);}drop(error);self.changed.notify_one();}
    fn reason(&self)->Option<String>{self.error.lock().unwrap_or_else(|e|e.into_inner()).clone()}
}
struct StreamDelegate(Arc<StreamFailure>);
impl screencapturekit::stream::delegate_trait::SCStreamDelegateTrait for StreamDelegate {
    fn did_stop_with_error(&self,error:screencapturekit::error::SCError){self.0.stop(error.to_string());}
    fn stream_did_stop(&self,error:Option<String>){self.0.stop(error.unwrap_or_else(||"Native screen capture ended".into()));}
}
#[derive(Clone,Default)]
pub(crate) struct StreamHealth(Arc<std::sync::Mutex<Arc<StreamFailure>>>);
impl StreamHealth {
    fn current(&self)->Arc<StreamFailure>{self.0.lock().unwrap_or_else(|e|e.into_inner()).clone()}
    fn restart(&self)->Arc<StreamFailure>{let next=Arc::new(StreamFailure::default());*self.0.lock().unwrap_or_else(|e|e.into_inner())=next.clone();next}
    pub(crate) fn reason(&self)->Option<String>{self.current().reason()}
}

#[derive(Clone)]
pub struct MacVideoFrame {
    pub width: u32,
    pub height: u32,
    pub window_crop: Option<protocol::viewport::PixelRect>,
    pub sample_buffer: Arc<CMSampleBuffer>,
    pub capture_time_ms: u32,
    pub timing: protocol::FrameTimingCheckpoints,
    /// Source timeline for pacing; callback delivery can arrive in bursts.
    pub pacing_ts_us: u64,
}

impl VideoFrame for MacVideoFrame {
    fn width(&self) -> u32 {
        self.window_crop.map_or(self.width, |c| c.width)
    }
    fn height(&self) -> u32 {
        self.window_crop.map_or(self.height, |c| c.height)
    }

    fn handle_kind(&self) -> VideoFrameHandleKind {
        VideoFrameHandleKind::MacosCvPixelBuffer
    }
}

struct StreamOutput {
    slot: Arc<remote_core::LatestFrameSlot<MacVideoFrame>>,
    source: protocol::session::CaptureSource,
    resources: Option<Arc<crate::lifecycle::Resources>>,
    window_crop: Arc<std::sync::Mutex<Option<protocol::viewport::PixelRect>>>,
    last_format: std::sync::atomic::AtomicU64,
}

impl SCStreamOutputTrait for StreamOutput {
    fn did_output_sample_buffer(
        &self,
        sample: screencapturekit::cm::CMSampleBuffer,
        _of_type: SCStreamOutputType,
    ) {
        let capture_ts_us = remote_core::timing::quanta_now_us();
        let now_ms = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            & 0xFFFFFFFF) as u32;
        let Some(buffer) = sample.image_buffer() else {
            return;
        };
        if let Some(resources) = &self.resources {
            use std::sync::atomic::Ordering::Relaxed;
            let format = ((buffer.width() as u64) << 32) | buffer.height() as u64;
            if self.last_format.load(Relaxed) != format {
                let rect = sample.content_rect().map(|r| [r.origin().x, r.origin().y, r.size().width, r.size().height]);
                *resources.native_capture_geometry.lock().unwrap() = Some(crate::lifecycle::NativeCaptureGeometry {
                    source: self.source, buffer: [buffer.width() as u32, buffer.height() as u32],
                    content_rect: rect,
                    bounding_rect: sample.bounding_rect().map(|r| [r.origin().x, r.origin().y, r.size().width, r.size().height]),
                    screen_rect: sample.screen_rect().map(|r| [r.origin().x, r.origin().y, r.size().width, r.size().height]),
                    content_scale: sample.content_scale(), scale_factor: sample.scale_factor(),
                });
                self.last_format.store(format, Relaxed);
            }
        }
        let active_crop = *self.window_crop.lock().unwrap_or_else(|e| e.into_inner());
        let frame = MacVideoFrame {
            width: buffer.width() as u32,
            height: buffer.height() as u32,
            window_crop: active_crop,
            sample_buffer: Arc::new(sample),
            capture_time_ms: now_ms,
            timing: protocol::FrameTimingCheckpoints::new(capture_ts_us),
            pacing_ts_us: capture_ts_us,
        };

        self.slot.push(frame);
    }
}

pub struct MacVideoCapturer {
    slot: Arc<remote_core::LatestFrameSlot<MacVideoFrame>>,
    stream: Option<SCStream>,
    stream_health:StreamHealth,
    camera: Option<crate::cameras::CameraHandle>,
    camera_started: bool,
    stream_running: bool,
    pending_stop: Option<crate::lifecycle::NativeStopReceipt>,
    resources: Option<Arc<crate::lifecycle::Resources>>,
    minimum_capture_ts_us: u64,
    target_width: u32,
    source: protocol::session::CaptureSource,
    source_size: (u32, u32),
    includes_children: bool,
    family_display_mode: bool,
    window_crop: Arc<std::sync::Mutex<Option<protocol::viewport::PixelRect>>>,
    geometry_checked: std::time::Instant,
    target_height: u32,
    target_fps: u32,
    pacer: remote_core::frame_pacer::FramePacer,
}

impl MacVideoCapturer {
    pub fn new(target_width: u32, target_height: u32, target_fps: u32) -> Self {
        Self {
            slot: Arc::new(remote_core::LatestFrameSlot::new()),
            stream: None,
            stream_health:StreamHealth::default(),
            camera: None,
            camera_started: false,
            stream_running: false,
            pending_stop: None,
            resources: None,
            minimum_capture_ts_us: 0,
            target_width,
            source: protocol::session::CaptureSource::MainDisplay,
            source_size: (0, 0),
            includes_children: false,
            family_display_mode: false,
            window_crop: Arc::new(std::sync::Mutex::new(None)),
            geometry_checked: std::time::Instant::now(),
            target_height,
            target_fps,
            pacer: remote_core::frame_pacer::FramePacer::new(target_fps),
        }
    }

    pub fn with_source(mut self, source: protocol::session::CaptureSource) -> Self {
        self.source = source;
        self
    }
    pub(crate) fn with_resources(mut self, resources: Arc<crate::lifecycle::Resources>) -> Self {
        resources.capture_target_fps.store(u64::from(self.target_fps), std::sync::atomic::Ordering::Relaxed);
        self.resources = Some(resources);
        self
    }
    pub(crate) fn with_stream_health(mut self,health:StreamHealth)->Self{self.stream_health=health;self}
    fn stop_owned_stream(&mut self) {
        self.camera_started=false;
        if let Some(camera)=self.camera.take() {
            let (receipt,completion)=crate::lifecycle::NativeStopCompletion::new(self.resources.clone());
            self.pending_stop=Some(receipt);
            camera.stop_owned(completion);
        }
        if let Some(stream) = self.stream.take() {
            let (receipt, completion) =
                crate::lifecycle::NativeStopCompletion::new(self.resources.clone());
            self.pending_stop = Some(receipt);
            self.stream_running = false;
            stream.stop_capture_owned(move |result| {
                completion.complete(result.map_err(|error| error.to_string()))
            });
        }
    }
    fn start_camera(&mut self)->Result<(),Box<dyn Error+Send+Sync>> {
        drop(self.slot.take());
        self.minimum_capture_ts_us=remote_core::timing::quanta_now_us();
        self.camera=Some(crate::cameras::CameraHandle::start(self.source,self.target_width,self.target_height,self.target_fps,self.slot.clone(),self.resources.clone())?);
        self.camera_started=true;
        if let Some(resources)=&self.resources {
            resources.native_streams.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
        }
        Ok(())
    }
    async fn await_owned_stop(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        if let Some(receipt) = &self.pending_stop {
            receipt.wait().await?;
            self.pending_stop = None;
        }
        Ok(())
    }

    fn configuration(&self, size: (u32, u32), includes_children: bool, family_display_mode: bool) -> SCStreamConfiguration {
        // Preserve the selected source's native pixels. Each independent viewer
        // crops/scales its IOSurface on the GPU, so zoom never magnifies a
        // previously downscaled 1080p capture of a Retina window/display.
        let (width, height) = (size.0.max(2),size.1.max(2));
        let mut config = SCStreamConfiguration::new();
        config.set_width(width);
        config.set_height(height);
        config.set_minimum_frame_interval(&CMTime::new(
            1,
            self.target_fps.saturating_mul(2).min(i32::MAX as u32) as i32,
        ));
        config.set_queue_depth(3);
        config.set_pixel_format(screencapturekit::stream::configuration::PixelFormat::YCbCr_420v);
        config.set_color_matrix(&crate::video_color::capture_yuv_matrix());
        config.set_shows_cursor(false);
        if matches!(self.source, protocol::session::CaptureSource::Window(_)) && !family_display_mode {
            // Shadows are decorations outside the AX/CG window bounds.
            config.set_ignores_shadows_single_window(true);
            // Include associated child popovers/menus/sheets unless AppKit fullscreen
            // parked a hidden 32pt titlebar above the content window.
            config.set_includes_child_windows(includes_children);
            config.set_scales_to_fit(true);
            config.set_ignore_global_clip_single_window(true);
            config.set_capture_resolution_type(
                screencapturekit::stream::configuration::SCCaptureResolutionType::Best,
            );
        }
        config
    }

    pub(crate) fn invalidate_geometry_check(&mut self) {
        self.geometry_checked = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(2))
            .unwrap_or_else(std::time::Instant::now);
    }

    pub fn update_resolution_and_fps(
        &mut self,
        width: u32,
        height: u32,
        fps: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        if !matches!(self.source, protocol::session::CaptureSource::Camera(_)) {
            self.invalidate_geometry_check();
            let _ = self.refresh_geometry();
        }
        if (self.target_width, self.target_height, self.target_fps) == (width, height, fps) {
            return Ok(());
        }
        self.target_width = width;
        self.target_height = height;
        self.target_fps = fps;
        if let Some(resources)=&self.resources { resources.capture_target_fps.store(u64::from(fps),std::sync::atomic::Ordering::Relaxed); }
        self.pacer.reset(fps);
        if matches!(self.source,protocol::session::CaptureSource::Camera(_)) {
            let started=self.camera_started;
            self.stop_owned_stream();self.camera_started=started;
            return Ok(());
        }
        if let Some(stream) = &self.stream {
            stream.update_configuration(&self.configuration(self.source_size, self.includes_children, self.family_display_mode))?;
        }
        Ok(())
    }
    fn refresh_geometry(&mut self) -> Result<bool, Box<dyn Error + Send + Sync>> {
        let interval = if matches!(self.source, protocol::session::CaptureSource::Window(_)) {
            std::time::Duration::from_millis(250)
        } else {
            std::time::Duration::from_secs(1)
        };
        if self.geometry_checked.elapsed() < interval {
            return Ok(false);
        }
        self.geometry_checked = std::time::Instant::now();
        if matches!(self.source,protocol::session::CaptureSource::Camera(_)) {
            return if self.camera.as_ref().is_some_and(|camera|camera.is_running()) {Ok(false)}
                else {Err("Camera stopped or its permission was revoked".into())};
        }
        let plan = crate::capture_sources::capture_plan(self.source)?;
        if plan.family_display_mode != self.family_display_mode {
            return Ok(true);
        }
        *self.window_crop.lock().unwrap_or_else(|e| e.into_inner()) = plan.window_crop;
        if (plan.stream_size, plan.includes_children) != (self.source_size, self.includes_children) {
            self.source_size = plan.stream_size;
            self.includes_children = plan.includes_children;
            if let Some(stream) = &self.stream {
                stream.update_configuration(&self.configuration(plan.stream_size, plan.includes_children, plan.family_display_mode))?;
            }
        }
        Ok(false)
    }
    pub(crate) fn update_capture_fps(&mut self,fps:u32)->Result<(),Box<dyn Error+Send+Sync>> {
        self.update_resolution_and_fps(self.target_width,self.target_height,fps)
    }
}

#[async_trait]
impl VideoCapturer for MacVideoCapturer {
    type Frame = MacVideoFrame;

    async fn start(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.await_owned_stop().await?;
        if matches!(self.source,protocol::session::CaptureSource::Camera(_)) {
            return self.start_camera();
        }
        let plan = crate::capture_sources::capture_plan(self.source)?;
        self.source_size = plan.stream_size;
        self.includes_children = plan.includes_children;
        self.family_display_mode = plan.family_display_mode;
        *self.window_crop.lock().unwrap_or_else(|e| e.into_inner()) = plan.window_crop;
        let config = self.configuration(plan.stream_size, plan.includes_children, plan.family_display_mode);
        let output = StreamOutput {
            slot: self.slot.clone(),
            source: self.source,
            resources: self.resources.clone(),
            window_crop: self.window_crop.clone(),
            last_format: std::sync::atomic::AtomicU64::new(0),
        };

        // A late callback belongs to this capture generation only. A restart
        // must never inherit its old stream's stop/error signal.
        let failure=self.stream_health.restart();
        let mut stream = SCStream::new_with_delegate(&plan.filter, &config,StreamDelegate(failure));
        stream.add_output_handler(output, SCStreamOutputType::Screen);
        stream.start_capture()?;

        self.stream = Some(stream);
        self.stream_running = true;
        if let Some(x) = &self.resources {
            x.native_streams
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(())
    }

    async fn stop(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.stop_owned_stream();
        self.await_owned_stop().await
    }

    async fn pause(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        if matches!(self.source,protocol::session::CaptureSource::Camera(_)) && self.camera_started {
            self.stop_owned_stream();self.camera_started=true;
        }
        self.minimum_capture_ts_us = u64::MAX;
        drop(self.slot.take());
        Ok(())
    }

    async fn resume(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        if matches!(self.source,protocol::session::CaptureSource::Camera(_)) {
            if !self.camera_started {return Err("capture owner has ended".into());}
            self.await_owned_stop().await?;
            if self.camera.is_none(){self.start_camera()?;}
        } else {
            if self.stream.is_none() { return Err("capture owner has ended".into()); }
            if self.minimum_capture_ts_us == u64::MAX {
                // A static window need not repaint after an activity change.
                // Resume with a fresh producer, after all old callbacks stop.
                self.stop_owned_stream();
                self.await_owned_stop().await?;
                self.minimum_capture_ts_us = remote_core::timing::quanta_now_us();
                drop(self.slot.take());
                self.start().await?;
            }
        }
        // Initial activation must retain the producer's first frame. Clearing
        // it here can leave an unchanged window black until its next repaint.
        self.pacer.reset(self.target_fps);
        Ok(())
    }

    async fn capture_frame(&mut self) -> Result<Self::Frame, Box<dyn Error + Send + Sync>> {
        if matches!(self.source,protocol::session::CaptureSource::Camera(_)) && self.camera.is_none() {
            if !self.camera_started {return Err("capture owner has ended".into());}
            self.await_owned_stop().await?;self.start_camera()?;
        }
        let mut health=self.stream_health.current();
        loop {
            if let Some(reason)=health.reason(){return Err(format!("Native screen capture stopped: {reason}").into());}
            if self.refresh_geometry()? {
                self.stop_owned_stream();
                self.await_owned_stop().await?;
                self.minimum_capture_ts_us = remote_core::timing::quanta_now_us();
                drop(self.slot.take());
                self.start().await?;
                health = self.stream_health.current();
                continue;
            }
            let next=tokio::select!{
                _=health.changed.notified()=>continue,
                frame=tokio::time::timeout(
                std::time::Duration::from_millis(500),
                self.slot.take_async(),
                )=>frame,
            };
            match next {
                Ok(frame) => {
                    if let Some(resources)=&self.resources {resources.raw_capture_samples.fetch_add(1,std::sync::atomic::Ordering::Relaxed);}
                    if frame.timing.capture_ts_us < self.minimum_capture_ts_us {continue;}
                    if self.pacer.admit(frame.pacing_ts_us) {return Ok(frame);}
                    if let Some(resources)=&self.resources {resources.capture_pacing_skips.fetch_add(1,std::sync::atomic::Ordering::Relaxed);}
                }
                Err(_) => {
                    // Check if stream is still active
                    if self.stream.is_none() && self.camera.is_none() {
                        return Err("Capture stream stopped".into());
                    }
                }
            }
        }
    }
}

impl Drop for MacVideoCapturer {
    fn drop(&mut self) {
        self.stop_owned_stream();
    }
}

#[cfg(test)]
mod stream_lifecycle_tests {
    use super::*;
    use screencapturekit::stream::delegate_trait::SCStreamDelegateTrait;
    #[tokio::test]
    async fn native_stop_event_wakes_capture_without_reading_any_screen() {
        let mut capture=MacVideoCapturer::new(320,180,30);
        StreamDelegate(capture.stream_health.current()).stream_did_stop(Some("Source stopped".into()));
        let result=tokio::time::timeout(std::time::Duration::from_millis(100),capture.capture_frame()).await.unwrap();
        assert!(result.err().expect("Stopped capture must return its native error").to_string().contains("Source stopped"));
    }
    #[test]
    fn a_late_stop_from_the_previous_stream_cannot_invalidate_a_new_generation() {
        let health=StreamHealth::default();let old=StreamDelegate(health.current());let current=health.restart();
        old.stream_did_stop(Some("Old source stopped".into()));assert!(health.reason().is_none());
        StreamDelegate(current).stream_did_stop(Some("Current source stopped".into()));assert_eq!(health.reason().as_deref(),Some("Current source stopped"));
    }
}

#[cfg(test)]
mod capture_matrix_tests {
    use super::*;
    #[test]
    fn nv12_configuration_explicitly_requests_a_known_conversion_matrix() {
        let mut config = SCStreamConfiguration::new();
        config.set_pixel_format(screencapturekit::stream::configuration::PixelFormat::YCbCr_420v);
        let value = crate::video_color::capture_yuv_matrix();
        assert!(!value.is_empty());
        config.set_color_matrix(&value);
        assert_eq!(config.color_matrix().as_deref(), Some(value.as_str()));
        // Reconfigure through all documented matrices and exercise allocation
        // churn after the temporary Rust CString/Swift String have been dropped.
        for value in ["ITU_R_601_4", "ITU_R_709_2", "SMPTE_240M_1995"] {
            config.set_color_matrix(&value.to_owned());
            for index in 0..2048 {
                let _ =
                    core_foundation::string::CFString::new(&format!("color-lifetime-test-{index}"));
            }
            assert_eq!(config.clone().color_matrix().as_deref(), Some(value));
        }
    }
}
