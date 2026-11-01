//! Camera portal permission and restricted PipeWire camera identities. This
//! backend never opens the user's unrestricted PipeWire socket or a device path.
use futures_util::StreamExt;
use protocol::session::{CameraFacing, CaptureSource, CaptureSourceInfo};
use remote_core::cameras::{CameraRegistry, NativeCamera};
use std::{cell::{Cell,RefCell},collections::HashMap,os::fd::OwnedFd,rc::Rc,
    sync::{Mutex,OnceLock,atomic::{AtomicI32,AtomicU64,Ordering}},time::{Duration,Instant}};
use zbus::{Connection,Proxy,zvariant::{OwnedObjectPath,OwnedValue,Value}};

const DEST:&str="org.freedesktop.portal.Desktop";
const PATH:&str="/org/freedesktop/portal/desktop";
const CAMERA:&str="org.freedesktop.portal.Camera";
static ACCESS:AtomicI32=AtomicI32::new(0);
static REQUEST:AtomicU64=AtomicU64::new(1);
type Result<T>=std::result::Result<T,String>;

#[derive(Clone)]
struct Node {id:u32,serial:u64,cookie:u32,name:String,width:u32,height:u32}
impl Node {
    fn key(&self)->String {format!("{}:{}",self.cookie,self.serial)}
    fn camera(&self)->NativeCamera {
        NativeCamera {key:self.key(),name:self.name.clone(),facing:CameraFacing::Unspecified,
            switch_group:None,width:self.width,height:self.height,rotation:0}
    }
}
struct State {registry:CameraRegistry,nodes:HashMap<String,Node>,cache:Option<(Instant,Vec<CaptureSourceInfo>)>}
fn state()->&'static Mutex<State> {
    static STATE:OnceLock<Mutex<State>>=OnceLock::new();
    STATE.get_or_init(||Mutex::new(State{registry:CameraRegistry::default(),nodes:HashMap::new(),cache:None}))
}
pub fn access()->i32 {ACCESS.load(Ordering::Relaxed)}
async fn portal()->Result<(Connection,Proxy<'static>)> {
    let connection=zbus::connection::Builder::session().map_err(|e|e.to_string())?
        .method_timeout(Duration::from_secs(5)).build().await.map_err(|e|e.to_string())?;
    let proxy=Proxy::new_owned(connection.clone(),DEST,PATH,CAMERA).await.map_err(|e|e.to_string())?;
    Ok((connection,proxy))
}
async fn remote()->Result<(Connection,OwnedFd)> {
    let (connection,proxy)=portal().await?;
    let options:HashMap<&str,Value<'_>>=HashMap::new();
    let fd:zbus::zvariant::OwnedFd=proxy.call("OpenPipeWireRemote",&(options,)).await
        .map_err(|_|"Allow camera in Weavelo Sharing & permissions on this Linux device".to_string())?;
    ACCESS.store(3,Ordering::Relaxed);
    Ok((connection,fd.into()))
}
struct PendingRequest {connection:Connection,path:String,complete:bool}
impl Drop for PendingRequest {
    fn drop(&mut self) {
        if self.complete{return;}
        let connection=self.connection.clone();let path=self.path.clone();
        if let Ok(runtime)=tokio::runtime::Handle::try_current() {runtime.spawn(async move {
            if let Ok(proxy)=Proxy::new_owned(connection,DEST,path,"org.freedesktop.portal.Request").await {
                let _=tokio::time::timeout(Duration::from_secs(3),proxy.call::<_,_,()>("Close",&())).await;
            }
        });}
    }
}
/// Requests real local portal consent; it never answers the system dialog.
pub async fn request_access()->Result<bool> {
    let (connection,proxy)=portal().await?;
    let sender=connection.unique_name().ok_or("Camera portal connection has no unique name")?
        .as_str().trim_start_matches(':').replace('.' ,"_");
    let token=format!("weavelo_camera_{}_{}",std::process::id(),REQUEST.fetch_add(1,Ordering::Relaxed));
    let path=format!("{PATH}/request/{sender}/{token}");
    let mut pending=PendingRequest{connection:connection.clone(),path:path.clone(),complete:false};
    let request=Proxy::new_owned(connection,DEST,path.clone(),"org.freedesktop.portal.Request")
        .await.map_err(|e|e.to_string())?;
    // Subscribe before AccessCamera: a saved permission may respond immediately.
    let mut responses=request.receive_signal("Response").await.map_err(|e|e.to_string())?;
    let mut options=HashMap::new();options.insert("handle_token",Value::from(token.as_str()));
    let returned:OwnedObjectPath=proxy.call("AccessCamera",&(options,)).await.map_err(|e|e.to_string())?;
    if returned.as_str()!=path {return Err("Camera portal changed its request identity".into());}
    let signal=tokio::time::timeout(Duration::from_secs(120),responses.next()).await
        .map_err(|_|"Camera permission request timed out".to_string())?.ok_or("Camera permission request closed")?;
    let (code,_):(u32,HashMap<String,OwnedValue>)=signal.body().deserialize().map_err(|e|e.to_string())?;
    pending.complete=true;
    ACCESS.store(if code==0 {3}else{2},Ordering::Relaxed);
    state().lock().unwrap_or_else(|e|e.into_inner()).cache=None;
    Ok(code==0)
}

fn inventory(fd:OwnedFd)->Result<Vec<Node>> {
    use pipewire as pw;
    crate::linux_pipewire::native::initialize();
    let loop_=pw::main_loop::MainLoopRc::new(None).map_err(|e|e.to_string())?;
    let context=pw::context::ContextRc::new(&loop_,None).map_err(|e|e.to_string())?;
    let core=context.connect_fd_rc(fd,None).map_err(|e|e.to_string())?;
    let registry=core.get_registry().map_err(|e|e.to_string())?;
    let nodes=Rc::new(RefCell::new(HashMap::new()));let cookie=Rc::new(Cell::new(None));
    let complete=Rc::new(Cell::new(false));let failed=Rc::new(Cell::new(false));
    let pending=core.sync(0).map_err(|e|e.to_string())?;
    let (end,done,server,error)=(loop_.clone(),complete.clone(),cookie.clone(),failed.clone());
    let error_loop=loop_.clone();
    let _core=core.add_listener_local().info(move|info|server.set(Some(info.cookie())))
        .done(move|id,seq|{if id==pw::core::PW_ID_CORE && seq==pending {done.set(true);end.quit();}})
        .error(move|_,_,_,_|{error.set(true);error_loop.quit();}).register();
    let added=nodes.clone();let removed=nodes.clone();
    let _registry=registry.add_listener_local().global(move|global| {
        if global.type_!=pw::types::ObjectType::Node {return;}
        let Some(properties)=global.props.as_ref() else{return;};
        if !properties.get("media.class").is_some_and(|value|value.starts_with("Video/Source")){return;}
        let Some(serial)=properties.get("object.serial").and_then(|s|s.parse::<u64>().ok()) else{return;};
        let name=properties.get("node.description").or_else(||properties.get("node.nick"))
            .or_else(||properties.get("node.name")).unwrap_or("Camera").chars().take(256).collect();
        let (width,height)=properties.get("video.size").and_then(|s|s.split_once('x'))
            .and_then(|(w,h)|Some((w.parse().ok()?,h.parse().ok()?))).unwrap_or((0,0));
        let mut nodes=added.borrow_mut();
        if nodes.len()<=remote_core::cameras::MAX_CAMERAS {
            nodes.insert(global.id,Node{id:global.id,serial,cookie:0,name,width,height});
        }
    }).global_remove(move|id|{removed.borrow_mut().remove(&id);}).register();
    let timer_loop=loop_.clone();let timer=loop_.loop_().add_timer(move|_|timer_loop.quit());
    timer.update_timer(Some(Duration::from_secs(3)),None).into_result().map_err(|e|e.to_string())?;
    loop_.run();
    if failed.get() || !complete.get(){return Err("Camera PipeWire enumeration did not complete".into());}
    let cookie=cookie.get().ok_or("Camera PipeWire server identity is unavailable")?;
    let mut result:Vec<_>=nodes.borrow().values().cloned().collect();
    if result.len()>remote_core::cameras::MAX_CAMERAS{return Err("Too many camera devices".into());}
    for node in &mut result {node.cookie=cookie;}
    result.sort_by_key(|node|node.serial);Ok(result)
}
fn query()->Result<Vec<Node>> {
    // Source catalogs are synchronous, including callers outside Tokio. Keep
    // native PipeWire objects and the portal connection on this bounded worker.
    std::thread::spawn(|| {
        let runtime=tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e|e.to_string())?;
        let (_connection,fd)=runtime.block_on(remote())?;
        inventory(fd)
    }).join().map_err(|_|"Camera catalog worker failed".to_string())?
}
pub(crate) fn list()->Result<Vec<CaptureSourceInfo>> {
    let mut state=state().lock().unwrap_or_else(|e|e.into_inner());
    if let Some((at,sources))=&state.cache && at.elapsed()<Duration::from_secs(1){return Ok(sources.clone());}
    let nodes=match query(){Ok(nodes)=>nodes,Err(error)=>{
        ACCESS.store(2,Ordering::Relaxed);state.nodes.clear();let _=state.registry.reconcile(Vec::new());state.cache=None;return Err(error);
    }};
    let sources=state.registry.reconcile(nodes.iter().map(Node::camera).collect()).map_err(str::to_owned)?;
    state.nodes=nodes.into_iter().map(|node|(node.key(),node)).collect();
    state.cache=Some((Instant::now(),sources.clone()));Ok(sources)
}
pub(crate) struct Capture {
    pub(crate) native:crate::linux_pipewire::native::CameraCapture,
    _portal:Connection,
}
pub(crate) async fn start(source:CaptureSource,width:u32,height:u32,fps:u32)->Result<Capture> {
    let node=tokio::task::spawn_blocking(move|| {
        list()?;
        let state=state().lock().unwrap_or_else(|e|e.into_inner());
        let camera=state.registry.resolve(source).ok_or("The selected camera is no longer available")?;
        state.nodes.get(&camera.key).cloned().ok_or("The selected camera is no longer available".to_string())
    }).await.map_err(|e|e.to_string())??;
    let (connection,fd)=remote().await?;
    let native=crate::linux_pipewire::native::prepare_camera_capture(fd,node.id,node.serial,node.cookie,width,height,fps).await?;
    Ok(Capture{native,_portal:connection})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pipewire_server_restart_retires_even_a_reused_camera_serial() {
        let node=Node{id:3,serial:90,cookie:4,name:"Camera".into(),width:0,height:0};
        let mut registry=CameraRegistry::default();let first=registry.reconcile(vec![node.camera()]).unwrap();
        let mut changed=node.clone();changed.cookie=5;
        let second=registry.reconcile(vec![changed.camera()]).unwrap();
        assert_ne!(first[0].source,second[0].source);assert!(registry.resolve(first[0].source).is_none());
        assert!(!second[0].supports_input);assert!(second[0].process_id.is_none());
    }
}
