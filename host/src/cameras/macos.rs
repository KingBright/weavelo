use crate::{capture::MacVideoFrame, lifecycle::NativeStopCompletion};
use protocol::session::{CaptureSource, CaptureSourceInfo};
use remote_core::{cameras::{CameraRegistry, NativeCamera}, LatestFrameSlot};
use std::{ffi::{c_char,c_void,CStr,CString},ptr::NonNull,sync::{Arc,Mutex,OnceLock,atomic::{AtomicU64,Ordering}}};

type FrameCallback=extern "C" fn(*mut c_void,*mut c_void,u32,u32,u64);
unsafe extern "C" {
    fn weavelo_camera_catalog()->*mut c_char;
    fn weavelo_camera_free_string(text:*mut c_char);
    fn weavelo_camera_access()->i32;
    fn weavelo_camera_request_access(context:*mut c_void,done:extern "C" fn(*mut c_void,i32));
    fn weavelo_camera_start(key:*const c_char,width:u32,height:u32,fps:u32,context:*mut c_void,frame:FrameCallback,error:*mut *mut c_char)->*mut c_void;
    fn weavelo_camera_stop(handle:*mut c_void,context:*mut c_void,done:extern "C" fn(*mut c_void));
    fn weavelo_camera_running(handle:*mut c_void)->bool;
}
fn registry()->&'static Mutex<CameraRegistry> {
    static REGISTRY:OnceLock<Mutex<CameraRegistry>>=OnceLock::new();
    REGISTRY.get_or_init(||Mutex::new(CameraRegistry::default()))
}
unsafe fn owned_string(text:*mut c_char)->Result<String,String> {
    if text.is_null(){return Err("Native camera returned no result".into());}
    let value=unsafe{CStr::from_ptr(text)}.to_string_lossy().into_owned();
    unsafe{weavelo_camera_free_string(text)};
    Ok(value)
}
pub(crate) fn list()->Result<Vec<CaptureSourceInfo>,String> {
    if access()!=3 {
        registry().lock().unwrap_or_else(|e|e.into_inner()).revoke();
        return Ok(Vec::new());
    }
    let text=unsafe{owned_string(weavelo_camera_catalog())}?;
    let cameras:Vec<NativeCamera>=serde_json::from_str(&text).map_err(|error|error.to_string())?;
    if access()!=3 {
        registry().lock().unwrap_or_else(|e|e.into_inner()).revoke();
        return Ok(Vec::new());
    }
    registry().lock().unwrap_or_else(|e|e.into_inner()).reconcile(cameras).map_err(str::to_owned)
}
pub fn access()->i32 {unsafe{weavelo_camera_access()}}
pub async fn request_access()->Result<bool,String> {
    extern "C" fn done(context:*mut c_void,status:i32) {
        let sender=unsafe{Box::from_raw(context.cast::<tokio::sync::oneshot::Sender<i32>>())};
        let _=sender.send(status);
    }
    let (tx,rx)=tokio::sync::oneshot::channel::<i32>();
    unsafe{weavelo_camera_request_access(Box::into_raw(Box::new(tx)).cast(),done)};
    rx.await.map(|status|status==3).map_err(|error|error.to_string())
}
pub(crate) struct CameraHandle {
    native:Option<NonNull<c_void>>,
    frames:Arc<CameraFrames>,
}
struct CameraFrames {
    slot:Arc<LatestFrameSlot<MacVideoFrame>>,
    resources:Option<Arc<crate::lifecycle::Resources>>,
    count:AtomicU64,
    first:AtomicU64,
    last:AtomicU64,
}
// Only owned calls move between worker threads. AVFoundation owns its serial callback/stop queues.
unsafe impl Send for CameraHandle {}
extern "C" fn frame(context:*mut c_void,sample:*mut c_void,width:u32,height:u32,pacing_ts_us:u64) {
    let Some(sample_buffer)=screencapturekit::cm::CMSampleBuffer::from_raw(sample) else{return;};
    let frames=unsafe{&*context.cast::<CameraFrames>()};
    if let Some(resources)=&frames.resources {resources.native_camera_frames.fetch_add(1,Ordering::Relaxed);}
    let capture_ts=remote_core::timing::quanta_now_us();
    if frames.count.fetch_add(1,Ordering::Relaxed)==0 {frames.first.store(capture_ts,Ordering::Relaxed);}
    frames.last.store(capture_ts,Ordering::Relaxed);
    let now=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u32;
    frames.slot.push(MacVideoFrame {width,height,sample_buffer,capture_time_ms:now,pacing_ts_us,timing:protocol::FrameTimingCheckpoints::new(capture_ts)});
}
struct StopContext {
    frames:Arc<CameraFrames>,
    completion:Option<NativeStopCompletion>,
}
extern "C" fn stopped(context:*mut c_void) {
    let mut context=unsafe{Box::from_raw(context.cast::<StopContext>())};
    let frames=&context.frames;
    let count=frames.count.load(Ordering::Relaxed);
    let elapsed=frames.last.load(Ordering::Relaxed).saturating_sub(frames.first.load(Ordering::Relaxed));
    let fps=if elapsed>0 {count.saturating_sub(1) as f64*1_000_000./elapsed as f64} else {0.};
    eprintln!("Camera native capture completed: frames={count} fps={fps:.2} elapsed_ms={}",elapsed/1000);
    if let Some(done)=context.completion.take(){done.complete(Ok(()));}
}
impl CameraHandle {
    pub(crate) fn is_running(&self)->bool {
        let running=self.native.is_some_and(|native|unsafe{weavelo_camera_running(native.as_ptr())});
        if !running&&access()!=3 {registry().lock().unwrap_or_else(|e|e.into_inner()).revoke();}
        running
    }
    pub(crate) fn start(source:CaptureSource,width:u32,height:u32,fps:u32,slot:Arc<LatestFrameSlot<MacVideoFrame>>,resources:Option<Arc<crate::lifecycle::Resources>>)->Result<Self,String> {
        list()?;
        let camera=registry().lock().unwrap_or_else(|e|e.into_inner()).resolve(source).cloned().ok_or("The selected camera is no longer available")?;
        let key=CString::new(camera.key).map_err(|_|"Invalid native camera identity")?;
        let mut error=std::ptr::null_mut();
        let frames=Arc::new(CameraFrames{slot,resources,count:AtomicU64::new(0),first:AtomicU64::new(0),last:AtomicU64::new(0)});
        let raw=unsafe{weavelo_camera_start(key.as_ptr(),width,height,fps,Arc::as_ptr(&frames).cast_mut().cast(),frame,&mut error)};
        match NonNull::new(raw) {
            Some(native)=>Ok(Self{native:Some(native),frames}),
            None=>Err(unsafe{owned_string(error)}.unwrap_or_else(|_|"Native camera failed to start".into())),
        }
    }
    fn stop(&mut self,completion:Option<NativeStopCompletion>) {
        let Some(native)=self.native.take() else{return;};
        let context=Box::into_raw(Box::new(StopContext{frames:self.frames.clone(),completion}));
        unsafe{weavelo_camera_stop(native.as_ptr(),context.cast(),stopped)};
    }
    pub(crate) fn stop_owned(mut self,completion:NativeStopCompletion) {self.stop(Some(completion));}
}
impl Drop for CameraHandle {fn drop(&mut self){self.stop(None);}}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_camera_catalog_preserves_typed_view_only_sources() {
        // Enumeration only: no session creation, permission request or camera activation.
        let sources=list().unwrap();
        assert!(sources.len()<=remote_core::cameras::MAX_CAMERAS);
        for source in sources {
            assert!(matches!(source.source,CaptureSource::Camera(_)));
            assert!(source.validate_camera().is_ok());
            assert!(registry().lock().unwrap().resolve(source.source).is_some());
        }
    }
}
