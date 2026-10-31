use crate::ffmpeg_hevc::{FfmpegHevcSource, replace_capture_source};
/// Pixel capture and encoding are owned by the native GPU pipeline; there is no submitted CPU frame.
pub enum WindowsVideoFrame {}
impl remote_core::VideoFrame for WindowsVideoFrame {
    fn width(&self)->u32 {match *self {}}
    fn height(&self)->u32 {match *self {}}
    fn handle_kind(&self)->remote_core::VideoFrameHandleKind {match *self {}}
    fn pixel_format(&self)->remote_core::VideoPixelFormat {match *self {}}
}
use async_trait::async_trait;
use remote_core::VideoEncoder;
use std::error::Error;

#[derive(Clone, Debug)]
pub struct EncodedChunk {
    pub nalu: Vec<u8>,
    pub capture_time_ms: u32,
    pub is_keyframe: bool,
    pub encode_cost_ms: f32,
    pub timing: protocol::FrameTimingCheckpoints,
    pub encoded_at_us:u64,
}

pub struct WindowsVideoEncoder {
    settings: (u32, u32, u32, u32),
    selection: Option<crate::windows_sources::Selection>,
    source: Option<FfmpegHevcSource>,
    force_keyframe: bool,
    awaiting_initial_keyframe:bool,
}

impl WindowsVideoEncoder {
    pub fn new(
        width: u32,
        height: u32,
        fps: u32,
        bitrate_kbps: u32,
        source: protocol::session::CaptureSource,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let selection=crate::windows_sources::resolve(source)?;
        let source = FfmpegHevcSource::start_native(&selection,width,height,fps,bitrate_kbps)?;
        Ok(Self {
            settings: (width, height, fps, bitrate_kbps),
            selection:Some(selection),
            source: Some(source),
            force_keyframe: false,
            awaiting_initial_keyframe:true,
        })
    }

    pub fn request_keyframe(&mut self) {
        if !self.awaiting_initial_keyframe {self.force_keyframe = true;}
    }

    pub fn update_settings(
        &mut self,
        width: u32,
        height: u32,
        fps: u32,
        bitrate_kbps: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let settings = (width, height, fps, bitrate_kbps);
        if self.settings == settings {
            return Ok(());
        }
        if self.source.is_some() {
            replace_capture_source(&mut self.source, || {
                FfmpegHevcSource::start_native(self.selection.as_ref().ok_or("Native capture source is unavailable")?,width,height,fps,bitrate_kbps)
            })?;
            self.force_keyframe = false;
            self.awaiting_initial_keyframe=true;
        }
        self.settings = settings;
        Ok(())
    }

    pub fn set_paused(&mut self, paused: bool) -> Result<(), Box<dyn Error + Send + Sync>> {
        if paused {
            self.source = None;
        } else if self.source.is_none() {
            let (width, height, fps, bitrate) = self.settings;
            self.source = Some(FfmpegHevcSource::start_native(self.selection.as_ref().ok_or("Native capture source is unavailable")?,width,height,fps,bitrate)?);
            self.force_keyframe = false;
            self.awaiting_initial_keyframe=true;
        }
        Ok(())
    }

    pub async fn pull_encoded_chunk(
        &mut self,
    ) -> Result<EncodedChunk, Box<dyn Error + Send + Sync>> {
        if self.force_keyframe && self.source.is_some() {
            let (width,height,fps,kbps)=self.settings;
            replace_capture_source(&mut self.source,||FfmpegHevcSource::start_native(self.selection.as_ref().ok_or("Native capture source is unavailable")?,width,height,fps,kbps))?;
            self.force_keyframe=false;self.awaiting_initial_keyframe=true;
        }
        if let Some(source) = &self.source {
            loop {
                if let Some(selection)=&self.selection {let _=crate::windows_sources::resolve(selection.source)?;}
                let (mut nalu, is_keyframe) = source.pull_access_unit().await?;
                if self.awaiting_initial_keyframe && !is_keyframe {
                    continue;
                }
                if is_keyframe {
                    if self.selection.as_ref().is_some_and(|s|s.encoder==crate::windows_capture_plan::Encoder::Amd) {
                        remote_core::hevc_sequence::set_srgb_transfer(&mut nalu)?;
                    }
                    self.awaiting_initial_keyframe = false;
                }
                let encoded_at_us = remote_core::timing::quanta_now_us();
                return Ok(EncodedChunk {
                    nalu,
                    // RTP uses a real sender output clock; capture/encode durations remain unknown.
                    capture_time_ms: (encoded_at_us / 1000) as u32,
                    is_keyframe,
                    encode_cost_ms: 0.0,
                    timing:protocol::FrameTimingCheckpoints::default(),encoded_at_us,
                });
            }
        }
        std::future::pending().await
    }
}

#[async_trait]
impl VideoEncoder for WindowsVideoEncoder {
    type Frame = WindowsVideoFrame;

    async fn submit_frame(
        &mut self,
        _frame: Self::Frame,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(())
    }

    async fn pull_encoded(&mut self) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
        Ok(self.pull_encoded_chunk().await?.nalu)
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    #[test]
    fn paused_settings_update_never_starts_ffmpeg() {
        let mut encoder = WindowsVideoEncoder {
            settings: (1920, 1080, 60, 8000),
            selection:None,
            source: None,
            force_keyframe: false,
            awaiting_initial_keyframe:true,
        };
        encoder.update_settings(1280, 720, 30, 4000).unwrap();
        assert!(encoder.source.is_none());
        assert_eq!(encoder.settings, (1280, 720, 30, 4000));
    }
}
