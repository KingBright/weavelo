//! WGC keeps source textures on D3D11; only compressed HEVC crosses this FFI.
use crate::{lifecycle::{NativeStopCompletion,NativeStopReceipt,Resources},windows_video_encode::EncodedChunk};
use protocol::{session::CaptureSource,viewport::FrameGeometry};
use remote_core::{frame_pacer::FramePacer,hevc_sequence::VideoSignal,viewport::{ViewportPlanner,bitrate_for_pixels}};
use std::{ffi::{c_char,c_void,CStr},sync::{Arc,Mutex,atomic::Ordering},time::Instant};
use tokio::sync::mpsc;

#[repr(C)]
struct Plan {x:u32,y:u32,crop_width:u32,crop_height:u32,width:u32,height:u32,fps:u32,kbps:u32,geometry:[u8;60]}
unsafe extern "C" {
    fn weavelo_windows_screen_start(handle:u64,source:u32,window:bool,scoped:bool,scope:*const u8,context:*mut c_void,
        timestamp:extern "C" fn()->u64,admit:extern "C" fn(*mut c_void,u64)->bool,
        plan:extern "C" fn(*mut c_void,u32,u32,*mut Plan)->bool,
        packet:extern "C" fn(*mut c_void,*const u8,usize,u64,u64,*const u8,usize),
        error:extern "C" fn(*mut c_void,*const c_char))->*mut c_void;
    fn weavelo_windows_screen_changed(handle:*mut c_void,keyframe:bool);
    fn weavelo_windows_screen_stop(handle:*mut c_void);
}
struct Settings {value:crate::StreamSettings,planner:ViewportPlanner,pacer:FramePacer}
struct Context {tx:mpsc::Sender<Result<EncodedChunk,String>>,settings:Mutex<Settings>,resources:Arc<Resources>,failure:Mutex<Option<String>>}
impl Context {
    fn fail(&self,reason:String) {
        let mut failure=self.failure.lock().unwrap_or_else(|e|e.into_inner());
        if failure.is_none(){*failure=Some(reason.clone());let _=self.tx.try_send(Err(reason));}
    }
}
extern "C" fn timestamp()->u64 {remote_core::timing::quanta_now_us()}
extern "C" fn admit(context:*mut c_void,now:u64)->bool {
    let context=unsafe{&*context.cast::<Context>()};
    context.resources.raw_capture_samples.fetch_add(1,Ordering::Relaxed);
    let ready=context.settings.lock().unwrap_or_else(|e|e.into_inner()).pacer.admit(now);
    if ready{context.resources.captured_frames.fetch_add(1,Ordering::Relaxed);}
    else{context.resources.capture_pacing_skips.fetch_add(1,Ordering::Relaxed);}
    ready
}
extern "C" fn plan(context:*mut c_void,w:u32,h:u32,out:*mut Plan)->bool {
    let context=unsafe{&*context.cast::<Context>()};let mut state=context.settings.lock().unwrap_or_else(|e|e.into_inner());
    if out.is_null(){context.fail("Missing native viewport plan".into());return false;}
    let settings=state.value.clone();
    match state.planner.plan([w,h],[settings.width,settings.height],settings.source_revision,Instant::now()) {
        Ok(g)=>{
            let mut header=Vec::with_capacity(60);g.write_header(&mut header);
            unsafe{out.write(Plan{x:g.crop.x,y:g.crop.y,crop_width:g.crop.width,crop_height:g.crop.height,
                width:g.width,height:g.height,fps:settings.fps,kbps:bitrate_for_pixels(g.width,g.height,settings.fps,settings.bitrate_kbps),
                geometry:header.try_into().expect("Fixed viewport header")})};true
        }
        Err(reason)=>{context.fail(reason.into());false}
    }
}
fn frame_geometry(header:&[u8])->Result<FrameGeometry,String> {
    if header.len()!=60{return Err("Missing frame-bound native viewport geometry".into());}
    let mut fixture=[0u8;65];fixture[..60].copy_from_slice(header);fixture[60..].copy_from_slice(&[0,0,0,1,0x26]);
    protocol::viewport::split_frame(&fixture).map_err(str::to_owned)?.0.ok_or("Missing native viewport geometry".into())
}
extern "C" fn packet(context:*mut c_void,data:*const u8,length:usize,captured:u64,cost:u64,header:*const u8,header_len:usize) {
    let context=unsafe{&*context.cast::<Context>()};
    let result=(|| {
        if data.is_null()||length==0||length>16*1024*1024||header.is_null()||header_len!=60{return Err("Invalid native screen packet".into());}
        let header=unsafe{std::slice::from_raw_parts(header,header_len)};let geometry=frame_geometry(header)?;
        let mut nalu=unsafe{std::slice::from_raw_parts(data,length)}.to_vec();let is_keyframe=crate::ffmpeg_hevc::is_hevc_keyframe(&nalu);
        if is_keyframe{crate::windows_media::validate_keyframe(&mut nalu,[geometry.width,geometry.height],
            VideoSignal{full_range:false,primaries:Some(1),transfer:Some(13),matrix:Some(1)})?;}
        let mut bytes=Vec::with_capacity(header.len()+nalu.len());bytes.extend_from_slice(header);bytes.extend_from_slice(&nalu);
        let now=timestamp();let elapsed=now.saturating_sub(captured).min(u64::from(u32::MAX));
        let mut timing=protocol::FrameTimingCheckpoints::new(captured);
        timing.encode_queue_ts_us=elapsed.saturating_sub(cost) as u32;timing.encode_done_ts_us=elapsed as u32;
        Ok(EncodedChunk{nalu:bytes,is_keyframe,capture_time_ms:(captured/1000) as u32,encode_cost_ms:cost as f32/1000.,timing,encoded_at_us:now})
    })();
    match result {Ok(chunk)=>{let _=context.tx.blocking_send(Ok(chunk));},Err(reason)=>context.fail(reason)}
}
extern "C" fn error(context:*mut c_void,message:*const c_char) {
    let reason=if message.is_null(){"Native screen capture failed".into()}else{unsafe{CStr::from_ptr(message)}.to_string_lossy().into_owned()};
    unsafe{&*context.cast::<Context>()}.fail(reason);
}
struct Native(usize);
impl Native {fn stop(self){unsafe{weavelo_windows_screen_stop(self.0 as *mut c_void)}}}
pub(crate) struct ScreenStream {native:Option<Native>,context:Option<Box<Context>>,rx:mpsc::Receiver<Result<EncodedChunk,String>>}
impl ScreenStream {
    pub(crate) fn start(selection:&crate::windows_sources::Selection,settings:crate::StreamSettings,resources:Arc<Resources>)->Result<Self,String> {
        crate::capture_readiness::require_srgb_capture_profile(&selection.monitor)?;
        let source=match selection.source{CaptureSource::Window(id)|CaptureSource::Display(id)=>id,_=>return Err("Invalid native screen source".into())};
        let (tx,rx)=mpsc::channel(4);let fps=settings.fps;
        let mut state=Settings{value:settings,planner:ViewportPlanner::for_source(selection.source),pacer:FramePacer::new(fps)};
        if let Some(demand)=state.value.viewport{state.planner.update(demand,Instant::now())?;}
        let mut context=Box::new(Context{tx,settings:Mutex::new(state),resources,failure:Mutex::new(None)});
        context.resources.capture_target_fps.store(u64::from(fps),Ordering::Relaxed);
        let scope=remote_core::shared_files::current_share_scope();let network=scope.unwrap_or([0;32]);
        let raw=unsafe{weavelo_windows_screen_start(selection.handle,source,selection.window,scope.is_some(),network.as_ptr(),
            (&mut *context as *mut Context).cast(),timestamp,admit,plan,packet,error)};
        if raw.is_null(){return Err("Native WGC worker could not start".into());}
        context.resources.native_streams.fetch_add(1,Ordering::SeqCst);Ok(Self{native:Some(Native(raw as usize)),context:Some(context),rx})
    }
    pub(crate) fn update(&self,settings:crate::StreamSettings)->Result<(),String> {
        let context=self.context.as_ref().ok_or("Native screen ended")?;let mut state=context.settings.lock().unwrap_or_else(|e|e.into_inner());
        if state.value.source_revision!=settings.source_revision{state.planner.reset();}
        if let Some(demand)=settings.viewport{state.planner.update(demand,Instant::now())?;}
        if state.value.fps!=settings.fps{state.pacer=FramePacer::new(settings.fps);}
        context.resources.capture_target_fps.store(u64::from(settings.fps),Ordering::Relaxed);state.value=settings;drop(state);
        if let Some(native)=&self.native{unsafe{weavelo_windows_screen_changed(native.0 as *mut c_void,false)}}Ok(())
    }
    pub(crate) fn request_keyframe(&self){if let Some(native)=&self.native{unsafe{weavelo_windows_screen_changed(native.0 as *mut c_void,true)}}}
    pub(crate) async fn pull(&mut self)->Result<EncodedChunk,Box<dyn std::error::Error+Send+Sync>> {
        if let Some(reason)=self.context.as_ref().and_then(|c|c.failure.lock().unwrap_or_else(|e|e.into_inner()).clone()){return Err(reason.into());}
        self.rx.recv().await.ok_or("Native screen output ended")?.map_err(Into::into)
    }
    pub(crate) fn stop(mut self)->NativeStopReceipt{self.begin_stop().expect("Native screen has an owner")}
    fn begin_stop(&mut self)->Option<NativeStopReceipt> {
        self.rx.close();let (Some(native),Some(context))=(self.native.take(),self.context.take())else{return None;};
        let (receipt,completion)=NativeStopCompletion::new(Some(context.resources.clone()));
        let task=Arc::new(Mutex::new(Some((native,context,completion))));let owned=task.clone();
        if let Err(reason)=std::thread::Builder::new().name("weavelo-screen-stop".into()).spawn(move||{
            if let Some((native,context,completion))=owned.lock().unwrap().take(){native.stop();drop(context);completion.complete(Ok(()));}
        }){eprintln!("Native screen teardown worker unavailable: {reason}");std::mem::forget(task);}Some(receipt)
    }
}
impl Drop for ScreenStream{fn drop(&mut self){self.begin_stop();}}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::viewport::PixelRect;
    unsafe extern "C" {fn weavelo_windows_screen_test_gpu(context:*mut c_void,plan:extern "C" fn(*mut c_void,u32,u32,*mut Plan)->bool,
        packet:extern "C" fn(*mut c_void,*const u8,usize,u64,u64,*const u8,usize),error:extern "C" fn(*mut c_void,*const c_char))->i32;
        fn weavelo_windows_screen_capture_count(handle:u64,source:u32)->usize;
    }
    fn decode(stream:Vec<u8>)->Vec<u8> {
        let ffmpeg=std::env::var_os("WEAVELO_CAMERA_TEST_FFMPEG").expect("Explicit existing fixture decoder required");
        let mut child=std::process::Command::new(ffmpeg).args(["-v","error","-f","hevc","-i","pipe:0","-f","rawvideo","-pix_fmt","nv12","pipe:1"])
            .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
        let mut stdin=child.stdin.take().unwrap();let writer=std::thread::spawn(move||{use std::io::Write;stdin.write_all(&stream).unwrap();});
        let decoded=child.wait_with_output().unwrap();writer.join().unwrap();assert!(decoded.status.success(),"{}",String::from_utf8_lossy(&decoded.stderr));decoded.stdout
    }
    #[derive(Default)]
    struct Fixture {planned:u32,frames:Vec<(Vec<u8>,FrameGeometry,u64)>,costs:Vec<u64>,error:Option<String>}
    extern "C" fn make_plan(context:*mut c_void,w:u32,h:u32,out:*mut Plan)->bool {
        let fixture=unsafe{&mut *context.cast::<Fixture>()};if [w,h]!=[640,360]{return false;}
        let n=fixture.planned;fixture.planned+=1;
        let g=FrameGeometry{source_revision:3,demand_revision:u64::from(n)+1,revision:u64::from(n)+1,
            source_width:w,source_height:h,crop:PixelRect{x:n%2*320,y:n/2%2*180,width:320,height:180},width:320,height:180};
        let mut header=Vec::new();g.write_header(&mut header);
        unsafe{out.write(Plan{x:g.crop.x,y:g.crop.y,crop_width:320,crop_height:180,width:320,height:180,fps:60,kbps:4000,geometry:header.try_into().unwrap()})};true
    }
    extern "C" fn output(context:*mut c_void,data:*const u8,len:usize,captured:u64,cost:u64,header:*const u8,header_len:usize) {
        let fixture=unsafe{&mut *context.cast::<Fixture>()};
        if data.is_null()||len==0||len>16*1024*1024||header.is_null()||header_len!=60{fixture.error=Some("Invalid native packet".into());return;}
        let geometry=frame_geometry(unsafe{std::slice::from_raw_parts(header,header_len)});
        match geometry{Ok(g)=>{fixture.frames.push((unsafe{std::slice::from_raw_parts(data,len)}.to_vec(),g,captured));fixture.costs.push(cost);},Err(e)=>fixture.error=Some(e)}
    }
    extern "C" fn failure(context:*mut c_void,error:*const c_char) {
        unsafe{&mut *context.cast::<Fixture>()}.error=Some(if error.is_null(){"Native GPU test failed".into()}else{unsafe{CStr::from_ptr(error)}.to_string_lossy().into_owned()});
    }
    #[test]
    #[ignore="Requires the actual Windows D3D11 hardware HEVC encoder and explicit fixture decoder"]
    fn actual_d3d11_viewport_pan_keeps_one_codec_and_frame_bound_pixels() {
        let mut fixture=Fixture::default();
        assert_eq!(unsafe{weavelo_windows_screen_test_gpu((&mut fixture as *mut Fixture).cast(),make_plan,output,failure)},1,"{:?}",fixture.error);
        assert!(fixture.error.is_none(),"{:?}",fixture.error);assert_eq!(fixture.planned,8);assert_eq!(fixture.frames.len(),8);
        let mut stream=Vec::new();let signal=VideoSignal{full_range:false,primaries:Some(1),transfer:Some(13),matrix:Some(1)};
        for (n,(bytes,g,captured)) in fixture.frames.iter_mut().enumerate(){
            assert_eq!(*captured,n as u64+1);assert_eq!(g.demand_revision,*captured);
            assert_eq!((g.crop.x,g.crop.y),((n as u32)%2*320,(n as u32)/2%2*180));
            if crate::ffmpeg_hevc::is_hevc_keyframe(bytes){crate::windows_media::validate_keyframe(bytes,[320,180],signal).unwrap();}
            stream.extend_from_slice(bytes);
        }
        let decoded=decode(stream);let frame_bytes=320*180*3/2;assert_eq!(decoded.len(),8*frame_bytes);
        for(n,frame)in decoded.chunks_exact(frame_bytes).enumerate(){
            let actual=i32::from(frame[90*320+160]);let expected=[63,173,32,235][n%4];
            assert!((actual-expected).abs()<=8,"ROI {n}: got luma {actual}, expected {expected}");
        }
        fixture.costs.sort_unstable();eprintln!("D3D11 crop+HEVC fixture: n={}, p50_us={}, max_us={}",fixture.costs.len(),fixture.costs[fixture.costs.len()/2],fixture.costs.last().unwrap());
    }

    struct OwnedWindow {handle:usize,thread:Option<std::thread::JoinHandle<()>>}
    impl OwnedWindow {
        fn start()->Self {
            use windows_sys::Win32::{Foundation::*,Graphics::Gdi::*,UI::WindowsAndMessaging::*};
            unsafe extern "system" fn procedure(hwnd:HWND,message:u32,w:WPARAM,l:LPARAM)->LRESULT {
                unsafe {match message {
                    WM_PAINT=>{let mut paint=std::mem::zeroed();let dc=BeginPaint(hwnd,&mut paint);let mut r=std::mem::zeroed();GetClientRect(hwnd,&mut r);
                        for(y,color_row)in [[0x000000ff,0x0000ff00],[0x00ff0000,0x00ffffff]].into_iter().enumerate(){
                            for(x,color)in color_row.into_iter().enumerate(){let brush=CreateSolidBrush(color);let area=RECT{left:r.right*x as i32/2,top:r.bottom*y as i32/2,right:r.right*(x+1)as i32/2,bottom:r.bottom*(y+1)as i32/2};FillRect(dc,&area,brush);DeleteObject(brush);}
                        }EndPaint(hwnd,&paint);0},
                    _=>{if message==WM_DESTROY{PostQuitMessage(0);}
                        let original:WNDPROC=std::mem::transmute(GetClassLongPtrW(hwnd,GCLP_WNDPROC));CallWindowProcW(original,hwnd,message,w,l)}
                }}
            }
            let (tx,rx)=std::sync::mpsc::sync_channel(1);
            let thread=std::thread::spawn(move||unsafe{
                let _dpi=crate::windows_sources::PhysicalCoordinates::enter().unwrap();
                let class:Vec<u16>="STATIC\0".encode_utf16().collect();let title:Vec<u16>="Weavelo owned ROI pixel fixture\0".encode_utf16().collect();
                let hwnd=CreateWindowExW(0,class.as_ptr(),title.as_ptr(),WS_OVERLAPPEDWINDOW,60,60,661,410,std::ptr::null_mut(),std::ptr::null_mut(),std::ptr::null_mut(),std::ptr::null());
                assert!(!hwnd.is_null());SetWindowLongPtrW(hwnd,GWLP_WNDPROC,procedure as *const () as isize);
                ShowWindow(hwnd,SW_SHOWNOACTIVATE);UpdateWindow(hwnd);tx.send(hwnd as usize).unwrap();
                let mut message=std::mem::zeroed();while GetMessageW(&mut message,std::ptr::null_mut(),0,0)>0{TranslateMessage(&message);DispatchMessageW(&message);}
            });
            Self{handle:rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),thread:Some(thread)}
        }
    }
    impl Drop for OwnedWindow {fn drop(&mut self){unsafe{windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(self.handle as _,windows_sys::Win32::UI::WindowsAndMessaging::WM_CLOSE,0,0);}
        if let Some(thread)=self.thread.take(){thread.join().unwrap();}}
    }
    async fn next(stream:&mut ScreenStream,revision:u64)->EncodedChunk {
        tokio::time::timeout(std::time::Duration::from_secs(8),async {loop{
            let chunk=stream.pull().await.unwrap();if protocol::viewport::split_frame(&chunk.nalu).unwrap().0.unwrap().demand_revision==revision{return chunk;}
        }}).await.expect("Actual WGC source did not present the requested ROI")
    }
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    #[ignore="Opens only its own neutral window; requires actual WGC/D3D11 HEVC and explicit fixture decoder"]
    async fn actual_wgc_two_views_share_capture_pan_static_pixels_and_stop_independently() {
        use protocol::viewport::{ViewportDemand,NormalizedRect};
        let window=OwnedWindow::start();let sources=crate::windows_sources::list().unwrap();
        let source=sources.iter().find(|s|matches!(s.source,CaptureSource::Window(_))&&s.process_id==i32::try_from(std::process::id()).ok()
            &&crate::windows_sources::resolve(s.source).is_ok_and(|v|v.handle==window.handle as u64)).expect("Own neutral window must be selectable").source;
        let selection=crate::windows_sources::resolve(source).unwrap();let source_id=match source{CaptureSource::Window(id)=>id,_=>unreachable!()};
        let demand=|id,x,y,revision|ViewportDemand{id,source_revision:3,revision,visible:NormalizedRect{x,y,width:500_000,height:500_000},width:320,height:180};
        let settings=|viewport|crate::StreamSettings{width:320,height:180,fps:60,bitrate_kbps:4000,paused:false,source_revision:3,viewport:Some(viewport)};
        let resources=Arc::new(Resources::default());
        let mut a=ScreenStream::start(&selection,settings(demand(1,0,0,1)),resources.clone()).unwrap();
        let mut b=ScreenStream::start(&selection,settings(demand(2,500_000,500_000,1)),resources.clone()).unwrap();
        let first=next(&mut a,1).await;let second=next(&mut b,1).await;
        assert_eq!(unsafe{weavelo_windows_screen_capture_count(selection.handle,source_id)},1,"Views must share one real WGC source");
        for(chunk,expected)in[(&first,63i32),(&second,235)]{
            let(g,payload)=protocol::viewport::split_frame(&chunk.nalu).unwrap();let g=g.unwrap();assert!(chunk.is_keyframe);
            let decoded=decode(payload.to_vec());let size=g.width as usize*g.height as usize;assert_eq!(decoded.len(),size*3/2);
            let actual=i32::from(decoded[size/2+g.width as usize/2]);assert!((actual-expected).abs()<=8,"WGC client/crop/color mismatch: {actual} vs {expected}");
        }
        let mut moving_stream=protocol::viewport::split_frame(&first.nalu).unwrap().1.to_vec();
        a.update(settings(demand(1,500_000,0,2))).unwrap();
        let moved=tokio::time::timeout(std::time::Duration::from_secs(8),async{loop{
            let chunk=a.pull().await.unwrap();let(g,payload)=protocol::viewport::split_frame(&chunk.nalu).unwrap();moving_stream.extend_from_slice(payload);
            if g.unwrap().demand_revision==2{break chunk;}
        }}).await.unwrap();
        let g=protocol::viewport::split_frame(&moved.nalu).unwrap().0.unwrap();assert!(g.crop.x>0);assert_eq!(g.crop.y,0);
        let initial_geometry=protocol::viewport::split_frame(&first.nalu).unwrap().0.unwrap();
        assert_eq!(g.source_width%2,1,"Own fixture must exercise an odd native width");
        assert_eq!([g.width,g.height],[initial_geometry.width,initial_geometry.height]);
        assert!(g.crop.contains(remote_core::viewport::visible_pixels(demand(1,500_000,0,2).visible,[g.source_width,g.source_height])));
        assert!(moved.timing.capture_ts_us>=first.timing.capture_ts_us);
        let decoded=decode(moving_stream);let size=g.width as usize*g.height as usize;let last=&decoded[decoded.len()-size*3/2..];
        let actual=i32::from(last[size/2+g.width as usize/2]);assert!((actual-173).abs()<=8,"Pan must decode the actual green source region: {actual}");
        eprintln!("WGC static ROI: reused_capture_timestamp={}, first={}, moved={}",moved.timing.capture_ts_us==first.timing.capture_ts_us,first.timing.capture_ts_us,moved.timing.capture_ts_us);
        a.stop().wait().await.unwrap();assert_eq!(unsafe{weavelo_windows_screen_capture_count(selection.handle,source_id)},1);
        let requested=timestamp();b.request_keyframe();
        tokio::time::timeout(std::time::Duration::from_secs(8),async{loop{let chunk=b.pull().await.unwrap();if chunk.is_keyframe&&chunk.encoded_at_us>=requested{break;}}}).await.unwrap();
        b.stop().wait().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2),async{while unsafe{weavelo_windows_screen_capture_count(selection.handle,source_id)}!=0{tokio::time::sleep(std::time::Duration::from_millis(10)).await;}}).await.unwrap();
        assert_eq!(resources.native_streams.load(Ordering::SeqCst),0);
    }
}
