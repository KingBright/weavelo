//! One screen/window capture per paired network. Consumers own their pacing,
//! encoders, pause state and stop receipts; captured IOSurfaces remain native.
use crate::{capture::{MacVideoCapturer as Owner,MacVideoFrame},lifecycle::{NativeStopCompletion,NativeStopReceipt,Resources}};
use async_trait::async_trait;
use protocol::session::CaptureSource;
use remote_core::{VideoCapturer,frame_pacer::FramePacer,shared_files::{ShareScope,current_share_scope}};
use std::{collections::HashMap,error::Error,sync::{Arc,Mutex,OnceLock,Weak,atomic::Ordering}};
use tokio::sync::watch;
type Failure=Box<dyn Error+Send+Sync>;
type Key=(ShareScope,CaptureSource);
static ROOTS:OnceLock<Mutex<HashMap<Key,Weak<Root>>>>=OnceLock::new();
struct Viewer {fps:u32,active:bool}
#[derive(Default)]
struct State {
    viewers:HashMap<u64,Viewer>,next_id:u64,started:bool,ending:bool,
    stopped:Option<Result<(),String>>,error:Option<String>,sequence:u64,latest:Option<MacVideoFrame>,
}
struct Root {scope:ShareScope,state:Mutex<State>,control:watch::Sender<()>,progress:watch::Sender<()>,resources:Arc<Resources>,health:crate::capture::StreamHealth}
fn canonical_source(source: CaptureSource) -> CaptureSource {
    match source {
        CaptureSource::MainDisplay => {
            CaptureSource::Display(core_graphics::display::CGDisplay::main().id)
        }
        other => other,
    }
}

impl Root {
    fn new(scope:ShareScope)->Arc<Self>{Arc::new(Self{scope,state:Mutex::new(State::default()),control:watch::channel(()).0,progress:watch::channel(()).0,resources:Arc::new(Resources::default()),health:Default::default()})}
    fn notify(&self){self.progress.send_replace(());}
    fn register(self:&Arc<Self>,fps:u32,resources:Option<Arc<Resources>>)->Result<ViewerLease,&'static str> {
        let mut state=self.state.lock().unwrap_or_else(|e|e.into_inner());
        if state.ending||state.error.is_some()||state.stopped.is_some(){return Err("Previous native capture is ending");}
        state.next_id=state.next_id.checked_add(1).ok_or("Native viewer identities exhausted")?;let id=state.next_id;
        state.viewers.insert(id,Viewer{fps,active:true});drop(state);self.control.send_replace(());
        if let Some(resources)=&resources {resources.native_streams.fetch_add(1,Ordering::SeqCst);}
        Ok(ViewerLease{root:self.clone(),id,resources,detached:false})
    }
    fn fail(&self,reason:String){let mut state=self.state.lock().unwrap_or_else(|e|e.into_inner());state.error=Some(reason);let previous=state.latest.take();drop(state);drop(previous);self.notify();}
    fn stopped(&self,result:Result<(),String>){let mut state=self.state.lock().unwrap_or_else(|e|e.into_inner());state.ending=true;let previous=state.latest.take();state.stopped=Some(result);drop(state);drop(previous);self.notify();}
    async fn wait_started(&self)->Result<(),Failure>{
        let mut progress=self.progress.subscribe();loop{
            if let Some(reason)=self.health.reason(){return Err(reason.into());}
            {let state=self.state.lock().unwrap_or_else(|e|e.into_inner());if let Some(reason)=&state.error{return Err(reason.clone().into());}if state.started{return Ok(());}if state.ending{return Err("Native capture ended before startup".into());}}
            progress.changed().await?;
        }
    }
    async fn wait_stopped(&self)->Result<(),String>{
        let mut progress=self.progress.subscribe();loop{
            if let Some(result)=self.state.lock().unwrap_or_else(|e|e.into_inner()).stopped.clone(){return result;}
            if progress.changed().await.is_err(){return Err("Native capture stop receipt is unavailable".into());}
        }
    }
    async fn produce(self:&Arc<Self>,owner:&mut Owner)->Result<(),Failure>{
        let mut control=self.control.subscribe();let mut running=false;let mut applied_fps=0u32;
        loop{
            if self.scope!=current_share_scope(){return Err("The paired network changed".into());}
            let (ending,fps)={let state=self.state.lock().unwrap_or_else(|e|e.into_inner());
                (state.ending,state.viewers.values().filter(|v|v.active).map(|v|v.fps).max().unwrap_or(0))};
            if ending{return Ok(());}
            if fps==0 {
                if running{owner.stop().await?;running=false;applied_fps=0;let mut state=self.state.lock().unwrap_or_else(|e|e.into_inner());let previous=state.latest.take();drop(state);drop(previous);self.notify();}
                control.changed().await?;continue;
            }
            if applied_fps!=fps {
                owner.update_capture_fps(fps)?;
                applied_fps=fps;
            }
            if !running{owner.start().await?;running=true;self.state.lock().unwrap_or_else(|e|e.into_inner()).started=true;self.notify();}
            tokio::select! {
                result=control.changed()=>{result?;},
                frame=owner.capture_frame()=>{
                    let frame=frame?;let mut state=self.state.lock().unwrap_or_else(|e|e.into_inner());
                    if state.ending{continue;}
                    state.sequence=state.sequence.checked_add(1).ok_or("Native capture sequence exhausted")?;
                    let previous=state.latest.replace(frame);drop(state);drop(previous);self.notify();
                }
            }
        }
    }
    fn run(self:Arc<Self>,mut owner:Owner){tokio::spawn(async move{
        struct Interrupted(Arc<Root>);
        impl Drop for Interrupted{fn drop(&mut self){if self.0.state.lock().unwrap_or_else(|e|e.into_inner()).stopped.is_none(){
            self.0.fail("Shared native capture worker ended unexpectedly".into());self.0.stopped(Err("Native capture stop was interrupted".into()));
        }}}
        let _interrupted=Interrupted(self.clone());
        if let Err(error)=self.produce(&mut owner).await{self.fail(error.to_string());}
        let result=owner.stop().await.map_err(|error|error.to_string());self.stopped(result);
    });}
}
struct ViewerLease {root:Arc<Root>,id:u64,resources:Option<Arc<Resources>>,detached:bool}
impl ViewerLease {
    fn observe_geometry(&self){if let Some(resources)=&self.resources{
        let geometry=self.root.resources.native_capture_geometry.lock().unwrap_or_else(|e|e.into_inner()).clone();
        *resources.native_capture_geometry.lock().unwrap_or_else(|e|e.into_inner())=geometry;
    }}
    fn configure(&self,fps:u32,active:bool){let mut state=self.root.state.lock().unwrap_or_else(|e|e.into_inner());
        if let Some(viewer)=state.viewers.get_mut(&self.id){viewer.fps=fps;viewer.active=active;}
        drop(state);self.root.control.send_replace(());
    }
    fn detach(&mut self)->Option<NativeStopReceipt>{
        if self.detached{return None;}self.detached=true;
        let (last,previous)={let mut state=self.root.state.lock().unwrap_or_else(|e|e.into_inner());state.viewers.remove(&self.id);let last=state.viewers.is_empty();let previous=if last{state.ending=true;state.latest.take()}else{None};(last,previous)};
        drop(previous);
        self.root.control.send_replace(());
        let (receipt,completion)=NativeStopCompletion::new(self.resources.clone());
        if !last{completion.complete(Ok(()));}
        else{let root=self.root.clone();match tokio::runtime::Handle::try_current(){
            Ok(runtime)=>{runtime.spawn(async move{completion.complete(root.wait_stopped().await);});},
            Err(_)=>completion.complete(Err("Native capture stop runtime is unavailable".into())),
        }}Some(receipt)
    }
}
impl Drop for ViewerLease{fn drop(&mut self){self.detach();}}

pub(crate) struct MacVideoCapturer {
    source:CaptureSource,width:u32,height:u32,fps:u32,resources:Option<Arc<Resources>>,
    camera:Option<Owner>,viewer:Option<ViewerLease>,progress:Option<watch::Receiver<()>>,seen:u64,
    paused:bool,pacer:FramePacer,pending_stop:Option<NativeStopReceipt>,
}
impl MacVideoCapturer {
    pub(crate) fn new(width:u32,height:u32,fps:u32)->Self{Self{source:CaptureSource::MainDisplay,width,height,fps,resources:None,
        camera:None,viewer:None,progress:None,seen:0,paused:false,pacer:FramePacer::new(fps),pending_stop:None}}
    pub(crate) fn with_source(mut self,source:CaptureSource)->Self{self.source=source;self}
    pub(crate) fn with_resources(mut self,resources:Arc<Resources>)->Self{resources.capture_target_fps.store(u64::from(self.fps),Ordering::Relaxed);self.resources=Some(resources);self}
    pub(crate) fn update_resolution_and_fps(&mut self,width:u32,height:u32,fps:u32)->Result<(),Failure>{
        protocol::validate_video_settings(width,height,fps,500)?;
        self.width=width;self.height=height;if self.fps!=fps{self.pacer.reset(fps);}self.fps=fps;
        if let Some(resources)=&self.resources{resources.capture_target_fps.store(u64::from(fps),Ordering::Relaxed);}
        if let Some(camera)=&mut self.camera{camera.update_resolution_and_fps(width,height,fps)?;}
        if let Some(viewer)=&self.viewer{viewer.configure(fps,!self.paused);}Ok(())
    }
    pub(crate) fn snapshot(&self)->Option<(u64,MacVideoFrame)>{
        if !self.pacer.ready(remote_core::timing::quanta_now_us()) {return None;}
        if self.paused{return None;}let viewer=self.viewer.as_ref()?;let state=viewer.root.state.lock().unwrap_or_else(|e|e.into_inner());
        if state.ending||state.error.is_some()||viewer.root.health.reason().is_some(){return None;}let frame=(state.sequence,state.latest.as_ref()?.clone());drop(state);viewer.observe_geometry();Some(frame)
    }
    pub(crate) fn accepted_snapshot(&mut self,sequence:u64){
        self.seen=self.seen.max(sequence);
        self.pacer.admit(remote_core::timing::quanta_now_us());
    }
    async fn await_stop(&mut self)->Result<(),Failure>{if let Some(receipt)=&self.pending_stop{receipt.wait().await?;self.pending_stop=None;}Ok(())}
    fn detach(&mut self){if let Some(mut viewer)=self.viewer.take(){self.pending_stop=viewer.detach();}self.progress=None;}
}
#[async_trait]
impl VideoCapturer for MacVideoCapturer {
    type Frame=MacVideoFrame;
    async fn start(&mut self)->Result<(),Failure>{
        protocol::validate_video_settings(self.width,self.height,self.fps,500)?;
        self.await_stop().await?;if self.viewer.is_some()||self.camera.is_some(){return Ok(());}
        if matches!(self.source,CaptureSource::Camera(_)){
            let mut camera=Owner::new(self.width,self.height,self.fps).with_source(self.source);
            if let Some(resources)=&self.resources{camera=camera.with_resources(resources.clone());}
            camera.start().await?;self.camera=Some(camera);return Ok(());
        }
        loop{
            let scope=current_share_scope();let canonical=canonical_source(self.source);let key=(scope,canonical);
            let (root,registration)={let mut roots=ROOTS.get_or_init(Default::default).lock().unwrap_or_else(|e|e.into_inner());
                roots.retain(|_,root|root.upgrade().is_some_and(|root|root.state.lock().unwrap_or_else(|e|e.into_inner()).stopped.is_none()));
                if let Some(root)=roots.get(&key).and_then(Weak::upgrade){let lease=root.register(self.fps,self.resources.clone());(root,lease)}
                else{let root=Root::new(scope);let lease=root.register(self.fps,self.resources.clone());roots.insert(key,Arc::downgrade(&root));
                    let owner=Owner::new(self.width,self.height,self.fps).with_source(canonical).with_resources(root.resources.clone()).with_stream_health(root.health.clone());
                    root.clone().run(owner);(root,lease)}};
            let lease=match registration{
                Ok(lease)=>lease,Err(_)=>{root.wait_stopped().await?;drop(root);tokio::task::yield_now().await;continue;}
            };
            self.progress=Some(root.progress.subscribe());root.wait_started().await?;self.viewer=Some(lease);self.seen=0;self.paused=false;return Ok(());
        }
    }
    async fn stop(&mut self)->Result<(),Failure>{
        if let Some(mut camera)=self.camera.take(){camera.stop().await?;}
        self.detach();self.await_stop().await
    }
    async fn pause(&mut self)->Result<(),Failure>{
        self.paused=true;if let Some(camera)=&mut self.camera{return camera.pause().await;}
        if let Some(viewer)=&self.viewer{viewer.configure(self.fps,false);}Ok(())
    }
    async fn resume(&mut self)->Result<(),Failure>{
        self.paused=false;self.pacer.reset(self.fps);self.seen=0;
        if let Some(camera)=&mut self.camera{return camera.resume().await;}
        if let Some(viewer)=&self.viewer{viewer.configure(self.fps,true);}else{return Err("Native capture owner ended".into());}Ok(())
    }
    async fn capture_frame(&mut self)->Result<MacVideoFrame,Failure>{
        if let Some(camera)=&mut self.camera{return camera.capture_frame().await;}
        loop{
            let viewer=self.viewer.as_ref().ok_or("Native capture owner ended")?;
            if let Some(reason)=viewer.root.health.reason(){return Err(reason.into());}
            let candidate={let state=viewer.root.state.lock().unwrap_or_else(|e|e.into_inner());
                if let Some(reason)=&state.error{return Err(reason.clone().into());}if state.ending{return Err("Native capture ended".into());}
                (!self.paused&&state.sequence>self.seen).then(||state.latest.clone().map(|frame|(state.sequence,frame))).flatten()};
            if let Some((sequence,frame))=candidate{self.seen=sequence;
                viewer.observe_geometry();
                if let Some(resources)=&self.resources{resources.raw_capture_samples.fetch_add(1,Ordering::Relaxed);}
                if self.pacer.admit(frame.pacing_ts_us){return Ok(frame);}
                if let Some(resources)=&self.resources{resources.capture_pacing_skips.fetch_add(1,Ordering::Relaxed);}
            }
            self.progress.as_mut().ok_or("Native capture observer ended")?.changed().await?;
        }
    }
}
impl Drop for MacVideoCapturer{fn drop(&mut self){self.detach();}}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn ending_one_viewer_keeps_the_other_and_last_waits_for_native_stop() {
        let root=Root::new(None);let a=Arc::new(Resources::default());let b=Arc::new(Resources::default());
        let mut first=root.register(60,Some(a.clone())).unwrap();let mut second=root.register(30,Some(b.clone())).unwrap();
        second.configure(30,false);assert_eq!(root.state.lock().unwrap().viewers.values().filter(|v|v.active).count(),1);
        second.configure(30,true);
        first.detach().unwrap().wait().await.unwrap();assert!(!root.state.lock().unwrap().ending);
        assert_eq!(a.native_streams.load(Ordering::SeqCst),0);assert_eq!(b.native_streams.load(Ordering::SeqCst),1);
        let receipt=second.detach().unwrap();assert!(root.state.lock().unwrap().ending);
        assert!(root.register(60,None).is_err(),"A retiring source cannot admit another viewer");
        assert_eq!(b.native_streams.load(Ordering::SeqCst),1,"Last viewer retains ownership until the native stop receipt");
        root.stopped(Ok(()));receipt.wait().await.unwrap();assert_eq!(b.native_streams.load(Ordering::SeqCst),0);
    }
    #[tokio::test]
    async fn source_failure_clears_cached_pixels_and_reports_an_interrupted_stop() {
        let root=Root::new(None);let resources=Arc::new(Resources::default());let mut viewer=root.register(60,Some(resources.clone())).unwrap();
        root.fail("Source ended".into());assert!(root.wait_started().await.unwrap_err().to_string().contains("Source ended"));
        assert!(root.register(30,None).is_err());let receipt=viewer.detach().unwrap();root.stopped(Err("Native stop interrupted".into()));
        assert!(receipt.wait().await.is_err());assert_eq!(resources.native_stop_errors.load(Ordering::SeqCst),1);
    }
}
