use crate::ffmpeg_hevc::FfmpegHevcSource;
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

enum Source {
    Desktop(FfmpegHevcSource),
    Camera(crate::cameras::CameraStream),
}
pub struct WindowsVideoEncoder {
    settings:(u32,u32,u32,u32),
    selection:Option<crate::windows_sources::Selection>,
    camera:Option<protocol::session::CaptureSource>,
    resources:std::sync::Arc<crate::lifecycle::Resources>,
    source:Option<Source>,
    pending_stop:Option<crate::lifecycle::NativeStopReceipt>,
    paused:bool,force_keyframe:bool,awaiting_initial_keyframe:bool,
}
impl WindowsVideoEncoder {
    pub fn new(width:u32,height:u32,fps:u32,bitrate_kbps:u32,source:protocol::session::CaptureSource,
        resources:std::sync::Arc<crate::lifecycle::Resources>)->Result<Self,Box<dyn Error+Send+Sync>> {
        let camera=matches!(source,protocol::session::CaptureSource::Camera(_)).then_some(source);
        let selection=if camera.is_none(){Some(crate::windows_sources::resolve(source)?)}else{None};
        let mut encoder=Self {settings:(width,height,fps,bitrate_kbps),selection,camera,resources,
            source:None,pending_stop:None,paused:false,force_keyframe:false,awaiting_initial_keyframe:true};
        encoder.start_source()?;Ok(encoder)
    }
    fn start_source(&mut self)->Result<(),Box<dyn Error+Send+Sync>> {
        let (w,h,fps,kbps)=self.settings;
        self.source=Some(if let Some(camera)=self.camera {
            Source::Camera(crate::cameras::CameraStream::start(camera,w,h,fps,kbps,self.resources.clone())?)
        }else{Source::Desktop(FfmpegHevcSource::start_native(self.selection.as_ref().ok_or("Native source unavailable")?,w,h,fps,kbps)?)});
        self.awaiting_initial_keyframe=true;self.force_keyframe=false;Ok(())
    }
    fn stop_source(&mut self) {
        if let Some(Source::Camera(camera))=self.source.take(){self.pending_stop=Some(camera.stop());}
    }
    pub fn request_keyframe(&mut self){if !self.awaiting_initial_keyframe{self.force_keyframe=true;}}
    pub fn update_settings(&mut self,w:u32,h:u32,fps:u32,kbps:u32)->Result<(),Box<dyn Error+Send+Sync>> {
        let settings=(w,h,fps,kbps);if self.settings!=settings{self.stop_source();self.settings=settings;}Ok(())
    }
    pub fn set_paused(&mut self,paused:bool)->Result<(),Box<dyn Error+Send+Sync>> {
        if paused{self.stop_source();}self.paused=paused;Ok(())
    }
    pub async fn pull_encoded_chunk(&mut self)->Result<EncodedChunk,Box<dyn Error+Send+Sync>> {
        if let Some(receipt)=&self.pending_stop{receipt.wait().await?;self.pending_stop=None;}
        if self.paused{return std::future::pending().await;}
        if self.source.is_none(){self.start_source()?;}
        if self.force_keyframe {
            if let Some(Source::Camera(camera))=&self.source{camera.request_keyframe();}
            else {self.stop_source();self.start_source()?;}
            self.force_keyframe=false;
        }
        loop {
            let chunk=match self.source.as_mut().ok_or("Native source unavailable")? {
                Source::Camera(camera)=>camera.pull().await?,
                Source::Desktop(source)=>{
                    if let Some(selection)=&self.selection{let _=crate::windows_sources::resolve(selection.source)?;}
                    let (mut nalu,is_keyframe)=source.pull_access_unit().await?;
                    if is_keyframe&&self.selection.as_ref().is_some_and(|s|s.encoder==crate::windows_capture_plan::Encoder::Amd){
                        remote_core::hevc_sequence::set_srgb_transfer(&mut nalu)?;
                    }
                    let encoded_at_us=remote_core::timing::quanta_now_us();
                    EncodedChunk{nalu,is_keyframe,capture_time_ms:(encoded_at_us/1000) as u32,
                        encode_cost_ms:0.,timing:protocol::FrameTimingCheckpoints::default(),encoded_at_us}
                }
            };
            if self.awaiting_initial_keyframe&&!chunk.is_keyframe{continue;}
            if chunk.is_keyframe{self.awaiting_initial_keyframe=false;}
            return Ok(chunk);
        }
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
            camera:None,
            resources:Default::default(),
            pending_stop:None,
            paused:true,
            source: None,
            force_keyframe: false,
            awaiting_initial_keyframe:true,
        };
        encoder.update_settings(1280, 720, 30, 4000).unwrap();
        assert!(encoder.source.is_none());
        assert_eq!(encoder.settings, (1280, 720, 30, 4000));
    }
}
