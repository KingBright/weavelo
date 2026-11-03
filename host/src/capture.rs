use async_trait::async_trait;
use remote_core::{VideoCapturer, VideoFrame, VideoFrameHandleKind};
use screencapturekit::prelude::*;
use std::error::Error;
use std::sync::Arc;

use screencapturekit::cm::CMSampleBuffer;

pub struct MacVideoFrame {
    pub width: u32,
    pub height: u32,
    pub sample_buffer: CMSampleBuffer,
    pub capture_time_ms: u32,
    pub timing: protocol::FrameTimingCheckpoints,
    /// Source timeline for pacing; callback delivery can arrive in bursts.
    pub pacing_ts_us: u64,
}

impl VideoFrame for MacVideoFrame {
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }

    fn handle_kind(&self) -> VideoFrameHandleKind {
        VideoFrameHandleKind::MacosCvPixelBuffer
    }
}

struct StreamOutput {
    slot: Arc<remote_core::LatestFrameSlot<MacVideoFrame>>,
    source: protocol::session::CaptureSource,
    resources: Option<Arc<crate::lifecycle::Resources>>,
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
        let frame = MacVideoFrame {
            width: buffer.width() as u32,
            height: buffer.height() as u32,
            sample_buffer: sample,
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
    camera: Option<crate::cameras::CameraHandle>,
    camera_started: bool,
    stream_running: bool,
    pending_stop: Option<crate::lifecycle::NativeStopReceipt>,
    resources: Option<Arc<crate::lifecycle::Resources>>,
    minimum_capture_ts_us: u64,
    target_width: u32,
    source: protocol::session::CaptureSource,
    source_size: (u32, u32),
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
            camera: None,
            camera_started: false,
            stream_running: false,
            pending_stop: None,
            resources: None,
            minimum_capture_ts_us: 0,
            target_width,
            source: protocol::session::CaptureSource::MainDisplay,
            source_size: (0, 0),
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

    fn configuration(&self, size: (u32, u32)) -> SCStreamConfiguration {
        let (width, height) =
            protocol::session::fit_capture_size(size, (self.target_width, self.target_height));
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
        if matches!(self.source, protocol::session::CaptureSource::Window(_)) {
            // The visual rectangle must be the same rectangle used for normalized input.
            // Shadows are decorations outside the AX/CG window bounds.
            config.set_ignores_shadows_single_window(true);
            // SCK otherwise includes associated native windows by default. In
            // fullscreen AppKit parks a hidden titlebar above the selected
            // content; that extra 32pt changes scaling without changing its
            // input bounds. A fixed-window source owns this window only.
            config.set_includes_child_windows(false);
            config.set_scales_to_fit(true);
            // A selected window owns its full content even when Stage Manager
            // places its thumbnail partly outside the desktop's visible clip.
            config.set_ignore_global_clip_single_window(true);
            config.set_capture_resolution_type(
                screencapturekit::stream::configuration::SCCaptureResolutionType::Best,
            );
        }
        config
    }

    pub fn update_resolution_and_fps(
        &mut self,
        width: u32,
        height: u32,
        fps: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
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
            stream.update_configuration(&self.configuration(self.source_size))?;
        }
        Ok(())
    }
    fn refresh_geometry(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        if self.geometry_checked.elapsed() < std::time::Duration::from_secs(1) {
            return Ok(());
        }
        self.geometry_checked = std::time::Instant::now();
        if matches!(self.source,protocol::session::CaptureSource::Camera(_)) {
            return if self.camera.as_ref().is_some_and(|camera|camera.is_running()) {Ok(())}
                else {Err("Camera stopped or its permission was revoked".into())};
        }
        let (_, size) = crate::capture_sources::filter(self.source)?;
        if size != self.source_size {
            self.source_size = size;
            if let Some(stream) = &self.stream {
                stream.update_configuration(&self.configuration(size))?;
            }
        }
        Ok(())
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
        let (filter, size) = crate::capture_sources::filter(self.source)?;
        self.source_size = size;
        let config = self.configuration(size);
        let output = StreamOutput {
            slot: self.slot.clone(),
            source: self.source,
            resources: self.resources.clone(),
            last_format: std::sync::atomic::AtomicU64::new(0),
        };

        let mut stream = SCStream::new(&filter, &config);
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
        loop {
            self.refresh_geometry()?;
            match tokio::time::timeout(
                std::time::Duration::from_millis(500),
                self.slot.take_async(),
            )
            .await
            {
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
