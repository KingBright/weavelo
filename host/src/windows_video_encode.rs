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
    Desktop(crate::windows_screen::ScreenStream),
    Camera(crate::cameras::CameraStream),
}
pub struct WindowsVideoEncoder {
    settings:crate::StreamSettings,
    selection:Option<crate::windows_sources::Selection>,
    camera:Option<protocol::session::CaptureSource>,
    resources:std::sync::Arc<crate::lifecycle::Resources>,
    source:Option<Source>,
    pending_stop:Option<crate::lifecycle::NativeStopReceipt>,
    paused:bool,force_keyframe:bool,awaiting_initial_keyframe:bool,
}
impl WindowsVideoEncoder {
    pub fn new(settings:crate::StreamSettings,source:protocol::session::CaptureSource,
        resources:std::sync::Arc<crate::lifecycle::Resources>)->Result<Self,Box<dyn Error+Send+Sync>> {
        let camera=matches!(source,protocol::session::CaptureSource::Camera(_)).then_some(source);
        let selection=if camera.is_none(){Some(crate::windows_sources::resolve(source)?)}else{None};
        let mut encoder=Self {settings,selection,camera,resources,
            source:None,pending_stop:None,paused:false,force_keyframe:false,awaiting_initial_keyframe:true};
        encoder.start_source()?;Ok(encoder)
    }
    fn start_source(&mut self)->Result<(),Box<dyn Error+Send+Sync>> {
        let settings=&self.settings;
        let (w,h,fps,kbps)=(settings.width,settings.height,settings.fps,settings.bitrate_kbps);
        self.source=Some(if let Some(camera)=self.camera {
            Source::Camera(crate::cameras::CameraStream::start(camera,w,h,fps,kbps,self.resources.clone())?)
        }else{Source::Desktop(crate::windows_screen::ScreenStream::start(self.selection.as_ref().ok_or("Native source unavailable")?,self.settings.clone(),self.resources.clone())?)});
        self.awaiting_initial_keyframe=true;self.force_keyframe=false;Ok(())
    }
    fn stop_source(&mut self) {
        if let Some(source)=self.source.take(){self.pending_stop=Some(match source{Source::Camera(camera)=>camera.stop(),Source::Desktop(screen)=>screen.stop()});}
    }
    pub fn request_keyframe(&mut self){if !self.awaiting_initial_keyframe{self.force_keyframe=true;}}
    pub fn update_settings(&mut self,settings:crate::StreamSettings)->Result<(),Box<dyn Error+Send+Sync>> {
        protocol::validate_video_settings(settings.width,settings.height,settings.fps,settings.bitrate_kbps)?;
        if let Some(Source::Desktop(screen))=&self.source{screen.update(settings.clone())?;}
        else if self.settings!=settings{self.stop_source();}
        self.settings=settings;Ok(())
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
            else if let Some(Source::Desktop(screen))=&self.source{screen.request_keyframe();}
            self.force_keyframe=false;
        }
        loop {
            let chunk=match self.source.as_mut().ok_or("Native source unavailable")? {
                Source::Camera(camera)=>camera.pull().await?,
                Source::Desktop(source)=>{
                    if let Some(selection)=&self.selection{let _=crate::windows_sources::resolve(selection.source)?;}
                    source.pull().await?
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
    fn paused_settings_update_never_starts_native_capture() {
        let mut encoder = WindowsVideoEncoder {
            settings:crate::StreamSettings{width:1920,height:1080,fps:60,bitrate_kbps:8000,paused:true,source_revision:3,viewport:None},
            selection:None,
            camera:None,
            resources:Default::default(),
            pending_stop:None,
            paused:true,
            source: None,
            force_keyframe: false,
            awaiting_initial_keyframe:true,
        };
        encoder.update_settings(crate::StreamSettings{width:1280,height:720,fps:30,bitrate_kbps:4000,..encoder.settings.clone()}).unwrap();
        assert!(encoder.source.is_none());
        assert_eq!((encoder.settings.width,encoder.settings.height,encoder.settings.fps,encoder.settings.bitrate_kbps),(1280,720,30,4000));
    }
}
