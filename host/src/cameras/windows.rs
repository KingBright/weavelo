//! Native MF camera/HEVC ownership. COM and raw GPU resources never enter Rust.
use crate::{lifecycle::{NativeStopCompletion,NativeStopReceipt,Resources},windows_video_encode::EncodedChunk};
use protocol::session::{CaptureSource,CaptureSourceInfo,CameraFacing};
use remote_core::{cameras::{CameraRegistry,NativeCamera},frame_pacer::FramePacer,hevc_sequence::VideoSignal};
use std::{collections::HashSet,ffi::{c_char,c_void,CStr,CString},sync::{Arc,Mutex,OnceLock,atomic::Ordering}};
use tokio::sync::mpsc;

type Error=extern "C" fn(*mut c_void,*const c_char);
type Packet=extern "C" fn(*mut c_void,*const u8,usize,u64,u64);
type Format=extern "C" fn(*mut c_void,u32,u32,u32,u32,u32,bool);
unsafe extern "C" {
    fn weavelo_windows_camera_catalog(context:*mut c_void,item:extern "C" fn(*mut c_void,*const c_char,*const c_char),error:Error)->i32;
    fn weavelo_windows_camera_start(key:*const c_char,w:u32,h:u32,fps:u32,bitrate:u32,context:*mut c_void,
        timestamp:extern "C" fn(*mut c_void)->u64,admit:extern "C" fn(*mut c_void,u64)->bool,
        format:Format,packet:Packet,error:Error)->*mut c_void;
    fn weavelo_windows_camera_keyframe(handle:*mut c_void);
    fn weavelo_windows_camera_stop(handle:*mut c_void);
    fn weavelo_windows_camera_epoch()->u64;
    #[cfg(test)] fn weavelo_windows_camera_test_gpu(variant:u32,context:*mut c_void,packet:Packet,error:Error)->i32;
}
struct Registry {cameras:CameraRegistry,epoch:u64,busy:HashSet<String>}
fn registry()->&'static Mutex<Registry> {
    static REGISTRY:OnceLock<Mutex<Registry>>=OnceLock::new();
    REGISTRY.get_or_init(||Mutex::new(Registry{cameras:CameraRegistry::default(),epoch:0,busy:HashSet::new()}))
}
#[derive(Default)]
struct Catalog {cameras:Vec<NativeCamera>,error:Option<String>}
extern "C" fn catalog_item(context:*mut c_void,key:*const c_char,name:*const c_char) {
    let catalog=unsafe{&mut *context.cast::<Catalog>()};
    if key.is_null()||name.is_null(){catalog.error=Some("Invalid camera catalog entry".into());return;}
    catalog.cameras.push(NativeCamera {key:unsafe{CStr::from_ptr(key)}.to_string_lossy().into_owned(),
        name:unsafe{CStr::from_ptr(name)}.to_string_lossy().into_owned(),facing:CameraFacing::Unspecified,
        switch_group:None,width:0,height:0,rotation:0});
}
extern "C" fn catalog_error(context:*mut c_void,error:*const c_char) {
    unsafe{&mut *context.cast::<Catalog>()}.error=Some(error_text(error));
}
fn error_text(error:*const c_char)->String {
    if error.is_null(){"Native camera failed".into()}else{unsafe{CStr::from_ptr(error)}.to_string_lossy().into_owned()}
}
pub(crate) fn list()->Result<Vec<CaptureSourceInfo>,String> {
    let before=unsafe{weavelo_windows_camera_epoch()};
    if before==0{return Err("Native camera device notifications are unavailable".into());}
    let mut catalog=Catalog::default();
    let ok=unsafe{weavelo_windows_camera_catalog((&mut catalog as *mut Catalog).cast(),catalog_item,catalog_error)};
    let after=unsafe{weavelo_windows_camera_epoch()};
    let mut registry=registry().lock().unwrap_or_else(|e|e.into_inner());
    if before!=after||registry.epoch!=after||ok==0 {registry.cameras.revoke();registry.epoch=after;}
    if before!=after{return Err("Camera devices changed; refresh sources".into());}
    if ok==0{return Err(catalog.error.unwrap_or_else(||"Camera enumeration failed".into()));}
    registry.cameras.reconcile(catalog.cameras).map_err(str::to_owned)
}
struct Lease(String);
impl Drop for Lease {fn drop(&mut self){registry().lock().unwrap_or_else(|e|e.into_inner()).busy.remove(&self.0);}}
#[derive(Clone)]
struct Expected {size:[u32;2],signal:VideoSignal}
fn validate_keyframe(nalu:&mut Vec<u8>,expected:&Expected)->Result<(),String> {
    crate::windows_media::validate_keyframe(nalu,expected.size,expected.signal)
}

struct Context {
    tx:mpsc::Sender<Result<EncodedChunk,String>>,
    pacer:Mutex<FramePacer>,format:Mutex<Option<Expected>>,
    resources:Arc<Resources>,_lease:Lease,
    failure:Mutex<Option<String>>,
}
extern "C" fn timestamp(context:*mut c_void)->u64 {
    let context=unsafe{&*context.cast::<Context>()};
    context.resources.native_camera_frames.fetch_add(1,Ordering::Relaxed);
    remote_core::timing::quanta_now_us()
}
extern "C" fn admit(context:*mut c_void,timestamp:u64)->bool {
    let context=unsafe{&*context.cast::<Context>()};
    context.resources.raw_capture_samples.fetch_add(1,Ordering::Relaxed);
    let ready=context.pacer.lock().unwrap_or_else(|e|e.into_inner()).admit(timestamp);
    if ready {context.resources.captured_frames.fetch_add(1,Ordering::Relaxed);}
    else {context.resources.capture_pacing_skips.fetch_add(1,Ordering::Relaxed);}
    ready
}
extern "C" fn format(context:*mut c_void,w:u32,h:u32,matrix:u32,primaries:u32,transfer:u32,full_range:bool) {
    let context=unsafe{&*context.cast::<Context>()};
    *context.format.lock().unwrap_or_else(|e|e.into_inner())=Some(Expected {size:[w,h],
        signal:VideoSignal {full_range,primaries:Some(primaries as u8),transfer:Some(transfer as u8),matrix:Some(matrix as u8)}});
}
extern "C" fn packet(context:*mut c_void,data:*const u8,length:usize,captured:u64,encode_us:u64) {
    let context=unsafe{&*context.cast::<Context>()};
    if data.is_null()||length==0||length>16*1024*1024 {let _=context.tx.blocking_send(Err("Invalid native camera packet".into()));return;}
    let mut nalu=unsafe{std::slice::from_raw_parts(data,length)}.to_vec();
    let is_keyframe=crate::ffmpeg_hevc::is_hevc_keyframe(&nalu);
    if is_keyframe {
        let expected=context.format.lock().unwrap_or_else(|e|e.into_inner()).clone();
        let valid=expected.ok_or_else(||"Camera format is unavailable".into()).and_then(|expected|validate_keyframe(&mut nalu,&expected));
        if let Err(reason)=valid {let _=context.tx.blocking_send(Err(reason));return;}
    }
    let now=remote_core::timing::quanta_now_us();
    let mut timing=protocol::FrameTimingCheckpoints::new(captured);
    let elapsed=now.saturating_sub(captured).min(u64::from(u32::MAX));
    timing.encode_queue_ts_us=elapsed.saturating_sub(encode_us) as u32;
    timing.encode_done_ts_us=elapsed as u32;
    let _=context.tx.blocking_send(Ok(EncodedChunk {nalu,is_keyframe,capture_time_ms:(captured/1000) as u32,
        encode_cost_ms:encode_us as f32/1000.,timing,encoded_at_us:now}));
}
extern "C" fn error(context:*mut c_void,message:*const c_char) {
    registry().lock().unwrap_or_else(|e|e.into_inner()).cameras.revoke();
    let context=unsafe{&*context.cast::<Context>()};let reason=error_text(message);
    *context.failure.lock().unwrap_or_else(|e|e.into_inner())=Some(reason.clone());
    let _=context.tx.try_send(Err(reason));
}
// Opaque worker handle only: native calls never expose/move COM interfaces.
struct Native(usize);
impl Native {fn stop(self){unsafe{weavelo_windows_camera_stop(self.0 as *mut c_void)}}}
pub(crate) struct CameraStream {native:Option<Native>,context:Option<Box<Context>>,rx:mpsc::Receiver<Result<EncodedChunk,String>>}
impl CameraStream {
    pub(crate) fn start(source:CaptureSource,w:u32,h:u32,fps:u32,kbps:u32,resources:Arc<Resources>)->Result<Self,String> {
        list()?;
        let lease={let mut registry=registry().lock().unwrap_or_else(|e|e.into_inner());
            let key=registry.cameras.resolve(source).ok_or("The selected camera is no longer available")?.key.clone();
            if !registry.busy.insert(key.clone()){return Err("This camera is already in use".into());}Lease(key)};
        let key=CString::new(lease.0.as_str()).map_err(|_|"Invalid camera identity")?;
        let (tx,rx)=mpsc::channel(4);
        resources.capture_target_fps.store(u64::from(fps),Ordering::Relaxed);
        let mut context=Box::new(Context{tx,pacer:Mutex::new(FramePacer::new(fps)),format:Mutex::new(None),resources,_lease:lease,failure:Mutex::new(None)});
        let raw=unsafe{weavelo_windows_camera_start(key.as_ptr(),w,h,fps,kbps.checked_mul(1000).ok_or("Invalid camera bitrate")?,
            (&mut *context as *mut Context).cast(),timestamp,admit,format,packet,error)};
        if raw.is_null(){return Err("Native camera worker could not start".into());}
        context.resources.native_streams.fetch_add(1,Ordering::SeqCst);
        Ok(Self{native:Some(Native(raw as usize)),context:Some(context),rx})
    }
    pub(crate) fn request_keyframe(&self){if let Some(native)=&self.native{unsafe{weavelo_windows_camera_keyframe(native.0 as *mut c_void)}}}
    pub(crate) async fn pull(&mut self)->Result<EncodedChunk,Box<dyn std::error::Error+Send+Sync>> {
        if let Some(reason)=self.context.as_ref().and_then(|c|c.failure.lock().unwrap_or_else(|e|e.into_inner()).clone()){return Err(reason.into());}
        self.rx.recv().await.ok_or("Native camera output ended")?.map_err(Into::into)
    }
    pub(crate) fn stop(mut self)->NativeStopReceipt {self.begin_stop().expect("Native camera has an owner")}
    fn begin_stop(&mut self)->Option<NativeStopReceipt> {
        self.rx.close();
        let (Some(native),Some(context))=(self.native.take(),self.context.take())else{return None;};
        let (receipt,completion)=NativeStopCompletion::new(Some(context.resources.clone()));
        // The bounded receiver is closed before joining, so a compressed callback
        // under backpressure cannot deadlock source teardown.
        let task=Arc::new(Mutex::new(Some((native,context,completion))));let owned=task.clone();
        if let Err(error)=std::thread::Builder::new().name("weavelo-camera-stop".into()).spawn(move||{
            if let Some((native,context,completion))=owned.lock().unwrap().take(){native.stop();drop(context);completion.complete(Ok(()));}
        }) {
            // Retain the exact owner/context if the OS cannot create a teardown
            // thread. Never free a callback context while native work is alive.
            eprintln!("Native camera teardown worker unavailable: {error}");std::mem::forget(task);
        }
        Some(receipt)
    }
}
impl Drop for CameraStream {fn drop(&mut self){self.begin_stop();}}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_does_not_activate_a_camera_or_publish_device_paths() {
        for source in list().unwrap() {
            assert!(matches!(source.source,CaptureSource::Camera(_)));
            assert!(!source.supports_input);
            assert!(source.process_id.is_none());
            assert!(source.validate_camera().is_ok());
        }
    }
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    async fn missing_native_camera_reports_failure_and_releases_its_owner() {
        // A non-device symbolic identity cannot open a physical camera or
        // request consent. Exercise the real MF failure/callback/teardown path.
        let resources=Arc::new(Resources::default());let (tx,rx)=mpsc::channel(4);
        let mut context=Box::new(Context {tx,pacer:Mutex::new(FramePacer::new(30)),format:Mutex::new(None),
            resources:resources.clone(),_lease:Lease("weavelo-missing-camera-fixture".into()),failure:Mutex::new(None)});
        let key=CString::new("weavelo-missing-camera-fixture").unwrap();
        let raw=unsafe{weavelo_windows_camera_start(key.as_ptr(),320,180,30,2_000_000,
            (&mut *context as *mut Context).cast(),timestamp,admit,format,packet,error)};
        assert!(!raw.is_null());resources.native_streams.fetch_add(1,Ordering::SeqCst);
        let mut stream=CameraStream {native:Some(Native(raw as usize)),context:Some(context),rx};
        assert!(tokio::time::timeout(std::time::Duration::from_secs(5),stream.pull()).await.unwrap().is_err());
        stream.stop().wait().await.unwrap();
        assert_eq!(resources.native_streams.load(Ordering::SeqCst),0);
        assert_eq!(resources.native_stops_pending.load(Ordering::SeqCst),0);
        assert_eq!(resources.native_stops_completed.load(Ordering::SeqCst),1);
    }
    #[test]
    #[ignore="Requires the actual Windows D3D11 hardware HEVC encoder and explicit fixture decoder"]
    fn actual_d3d11_camera_encoder_preserves_frames_color_and_pixels() {
        #[derive(Default)] struct Fixture {packets:Vec<(Vec<u8>,u64)>,error:Option<String>}
        extern "C" fn output(context:*mut c_void,data:*const u8,length:usize,stamp:u64,_cost:u64) {
            let fixture=unsafe{&mut *context.cast::<Fixture>()};
            if data.is_null()||length>16*1024*1024 {fixture.error=Some("Unbounded output".into());return;}
            fixture.packets.push((unsafe{std::slice::from_raw_parts(data,length)}.to_vec(),stamp));
        }
        extern "C" fn failure(context:*mut c_void,message:*const c_char) {
            unsafe{&mut *context.cast::<Fixture>()}.error=Some(error_text(message));
        }
        for variant in 0..8 {
        let mut fixture=Fixture::default();
        let result=unsafe{weavelo_windows_camera_test_gpu(variant,(&mut fixture as *mut Fixture).cast(),output,failure)};
        assert_eq!(result,1,"{:?}",fixture.error);assert!(fixture.error.is_none());
        assert_eq!(fixture.packets.len(),36);
        let expected=VideoSignal{full_range:variant&4!=0,primaries:Some(1),transfer:Some(if variant&2!=0{13}else{1}),matrix:Some(if variant&1!=0{6}else{1})};
        let mut stream=Vec::new();let mut keys=0;
        for (index,(bytes,stamp)) in fixture.packets.iter().enumerate() {
            let mut bytes=bytes.clone();
            assert_eq!(*stamp,1_000_000+index as u64*33333,"GPU output lost frame association");
            if crate::ffmpeg_hevc::is_hevc_keyframe(&bytes) {
                validate_keyframe(&mut bytes,&Expected{size:[320,180],signal:expected}).unwrap();
                keys+=1;let sequence=remote_core::hevc_sequence::sequence_info(&bytes).unwrap().expect("HEVC keyframe needs its SPS");
                assert_eq!(sequence.visible[2..],[320,180]);assert_eq!(sequence.bit_depth,8);assert_eq!(sequence.signal,Some(expected),"native color variant {variant}");
            }
            stream.extend_from_slice(&bytes);
        }
        assert!(keys>=2,"The native keyframe request was not honored");
        let ffmpeg=std::env::var_os("WEAVELO_CAMERA_TEST_FFMPEG").expect("Explicit existing fixture decoder required");
        let mut child=std::process::Command::new(ffmpeg).args(["-v","error","-f","hevc","-i","pipe:0","-f","rawvideo","-pix_fmt","nv12","pipe:1"])
            .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
        let mut stdin=child.stdin.take().unwrap();let writer=std::thread::spawn(move||{use std::io::Write;stdin.write_all(&stream).unwrap();});
        let decoded=child.wait_with_output().unwrap();writer.join().unwrap();
        assert!(decoded.status.success(),"{}",String::from_utf8_lossy(&decoded.stderr));
        let frame_bytes=320*180*3/2;assert_eq!(decoded.stdout.len(),36*frame_bytes);
        for (index,frame) in decoded.stdout.chunks_exact(frame_bytes).enumerate() {
            for (band,x) in [40,120,200,280].into_iter().enumerate() {
                let actual=frame[90*320+x] as i32;let expected=40+band as i32*50+index as i32%2;
                assert!((actual-expected).abs()<=8,"frame {index}, band {band}: {actual}, expected {expected}");
            }
            assert!((frame[320*180+90] as i32-128).abs()<=5);
        }
        }
    }
}
