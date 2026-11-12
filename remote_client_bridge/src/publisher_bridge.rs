//! Android's user-authorized MediaProjection feeds encoded frames into the shared
//! transport. A projection exposes one selected source; it cannot enumerate or
//! silently switch to arbitrary third-party application windows.
use protocol::{ControlMessage, RtpHeader, RtpPacket, session::*};
use remote_core::{
    directed_delivery::{Broker, Incoming, Peer as DeliveryPeer, LocalIntent, OutgoingOffer},
    file_transfer_runtime::*,
    net::{MultiplexedPacket, UdpMultiplexer, UdpSender},
    scheduled_sender::ScheduledDataSender,
    session_crypto::*,
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}},
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, mpsc, watch};
use crate::publisher_media::{ActiveRequest, MediaSources, screen_source};
pub use crate::publisher_media::CaptureDemand;

// Bound authenticated transports, including clients which have not opened a file
// session yet. Screen views share capture, with independent GPU crop/encoders.
const MAX_PUBLISHER_PEERS: usize = 8;

struct Frame {
    source: CaptureSource,
    epoch: u64,
    bytes: Vec<u8>,
    timestamp: u32,
    audio: bool,
    screen_stream: Option<u64>,
    timing: Option<protocol::FrameTimingCheckpoints>,
    geometry_revision: u64,
}
#[derive(Clone,serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenFrameMetadata {
    pub stream:u64,
    pub grant:u64,
    pub geometry:protocol::viewport::FrameGeometry,
    pub capture_ts_us:u64,
    pub admitted_at_us:u64,
}
enum CaptureFailure {
    Source(CaptureSource,u64,String),
    ScreenView{stream:u64,grant:u64,reason:String},
}
pub struct MobilePublisher {
    device_badge: Arc<Mutex<protocol::appearance::DeviceBadge>>,
    pub address: SocketAddr,
    media: Arc<Mutex<MediaSources>>,
    media_changed: watch::Sender<u64>,
    artwork_changed: watch::Sender<()>,
    generation: String,
    alive: Arc<AtomicBool>,
    frames: mpsc::Sender<Frame>,
    errors: mpsc::Sender<CaptureFailure>,
    input: Arc<Mutex<crate::publisher_input::PublisherInput>>,
    capabilities: watch::Sender<remote_core::discovery::DiscoveryCapabilities>,
    task: tokio::task::JoinHandle<()>,
    file_reception: Arc<AtomicBool>,
    identity: Mutex<Option<String>>,
    outgoing: Mutex<Option<Outgoing>>,
    delivery: Arc<Broker>,
    incoming: Mutex<mpsc::Receiver<Arc<Incoming>>>,
    deliveries: Mutex<HashMap<String,Arc<Incoming>>>,
}
struct Outgoing {
    id: u64,
    target: String,
    ticket: [u8;32],
    screen_grant: Option<u64>,
    quality_alive: Arc<AtomicBool>,
    stop: watch::Sender<bool>,
    status: Arc<Mutex<String>>,
    confirmation_timeout: Arc<Mutex<Option<remote_core::directed_delivery::ConfirmationTimeout>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Outgoing {
    fn drop(&mut self) { self.stop.send_replace(true); }
}
impl Drop for MobilePublisher {
    fn drop(&mut self) {
        self.alive.store(false,Ordering::Release);
        self.capabilities.send_replace(crate::inbound_capabilities(0, false));
        self.input.lock().unwrap().set_capture(false);
        self.task.abort();
    }
}

// The actual serving task owns the advertised lifetime. A failed or aborted
// worker must not leave a stale screen/file claim in discovery.
struct AdvertisedPublisher(watch::Sender<remote_core::discovery::DiscoveryCapabilities>, Arc<Mutex<crate::publisher_input::PublisherInput>>,Arc<AtomicBool>);
impl Drop for AdvertisedPublisher {
    fn drop(&mut self) {
        self.2.store(false,Ordering::Release);
        self.0.send_replace(crate::inbound_capabilities(0, false));
        self.1.lock().unwrap().set_capture(false);
    }
}
struct Peer {
    addr: SocketAddr,
    id: u32,
    seen: Instant,
    subscriptions: HashMap<u32, MobileSubscription>,
    retired: std::collections::HashSet<u32>,
    scheduled: ScheduledDataSender,
    files: Option<mpsc::Sender<protocol::DataEnvelope>>,
    cancel: broadcast::Sender<()>,
    tasks: Vec<tokio::task::AbortHandle>,
    delivery_owner: Option<(DeliveryPeer,[u8;32])>,
}
struct MobileSubscription {
    request: SubscriptionRequest,
    grant: u64,
    video: bool,
    audio: bool,
    activity_revision: u64,
    settings_revision: u64,
    video_sequence: u16,
    audio_sequence: u16,
    screen_stream: u64,
}
struct PendingArtwork {
    request_id:u32,
    connection:u32,
    nonce:[u8;16],
    source:CaptureSource,
    grant:u64,
    deadline:Instant,
}
async fn finish_artwork(pending:&mut HashMap<(SocketAddr,u32),PendingArtwork>,
    peers:&HashMap<SocketAddr,Peer>,authenticated:&HashMap<SocketAddr,([u8;16],[u8;16],Instant)>,
    media:&Mutex<MediaSources>,badge:&Mutex<protocol::appearance::DeviceBadge>,sender:&UdpSender) {
    let mut replies=Vec::new();
    {
        let media=media.lock().unwrap();
        pending.retain(|&(addr,id),request| {
            let Some(peer)=peers.get(&addr).filter(|p|p.id==request.connection) else{return false;};
            if !authenticated.get(&addr).is_some_and(|(_,nonce,_)|*nonce==request.nonce) {return false;}
            let valid=peer.subscriptions.get(&id).is_some_and(|sub|sub.request.source==request.source&&sub.grant==request.grant);
            let entry=media.sources.get(&request.source).filter(|entry|entry.enabled&&entry.grant==request.grant);
            let value=if !valid || entry.is_none() {Some(Err("Artwork source is no longer selected".to_owned()))}
                else if let Some(icon)=&entry.unwrap().artwork {Some(Ok(icon.clone()))}
                else if Instant::now()>=request.deadline {Some(Ok(None))} else {None};
            let Some(value)=value else{return true;};
            replies.push((addr,SessionCommand::SourceAppearance {request_id:request.request_id,id,source:request.source,
                appearance:Box::new(value.map(|application|protocol::appearance::SourceAppearance {
                    application,device:badge.lock().unwrap().clone()}))}));
            false
        });
    }
    for (addr,response) in replies {reply(sender,addr,response).await;}
}
impl MobileSubscription {
    fn new(request:SubscriptionRequest,grant:u64,audio:bool)->Self {
        static NEXT:std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(1);
        let screen_stream=NEXT.fetch_update(Ordering::Relaxed,Ordering::Relaxed,|id|(id<i64::MAX as u64).then_some(id+1))
            .expect("Screen subscription identity exhausted");
        Self{request,grant,video:true,audio,activity_revision:0,settings_revision:0,video_sequence:0,audio_sequence:0,screen_stream}
    }
    fn input_allowed(&self)->bool {screen_source(self.request.source)&&self.video}
}
fn recompute(peers:&HashMap<SocketAddr,Peer>,media:&Mutex<MediaSources>) {
    let mut media=media.lock().unwrap();
    media.recompute(peers.values().flat_map(|peer|peer.subscriptions.values())
        .map(|sub|ActiveRequest{stream:sub.screen_stream,grant:sub.grant,request:&sub.request,video:sub.video,audio:sub.audio}));
    media.set_view_tickets(peers.values().filter_map(|peer|peer.delivery_owner.map(|(_,ticket)|(peer,ticket)))
        .flat_map(|(peer,ticket)|peer.subscriptions.values().map(move |sub|(sub.screen_stream,ticket))).collect());
    let mut streams=std::collections::BTreeMap::new();
    for peer in peers.values() {
        let count=peer.subscriptions.values().filter(|sub|media.sources.get(&sub.request.source)
            .is_some_and(|source|source.enabled&&source.grant==sub.grant)).count();
        if count>0 {*streams.entry(remote_core::routed_relay::incoming_device(peer.addr)).or_insert(0)+=count;}
    }
    media.peer_streams=streams.into_iter().collect();
}
fn retire_quality(media:&Mutex<MediaSources>,alive:&AtomicBool,ticket:[u8;32]) {
    let mut media=media.lock().unwrap();alive.store(false,Ordering::Release);media.clear_quality(ticket);
}
async fn refresh_sources(peers:&mut HashMap<SocketAddr,Peer>,media:&Mutex<MediaSources>,
    input:&Mutex<crate::publisher_input::PublisherInput>,sender:&UdpSender,
    capabilities:&watch::Sender<remote_core::discovery::DiscoveryCapabilities>,alive:&AtomicBool) {
    let grants=media.lock().unwrap().sources.iter().filter(|(_,source)|source.enabled)
        .map(|(id,source)|(*id,source.grant)).collect::<HashMap<_,_>>();
    for peer in peers.values_mut() {
        let ended=peer.subscriptions.iter().filter(|(_,sub)|grants.get(&sub.request.source)!=Some(&sub.grant))
            .map(|(id,_)|*id).collect::<Vec<_>>();
        for id in ended {
            if let Some(sub)=peer.subscriptions.remove(&id) {
                if screen_source(sub.request.source){input.lock().unwrap().revoke_peer(peer.addr);}
                peer.retired.insert(id);
                reply(sender,peer.addr,SessionCommand::Unsubscribed{id}).await;
            }
        }
    }
    recompute(peers,media);
    let ready=input.lock().unwrap().ready();
    let catalog=media.lock().unwrap().catalog(ready);
    capabilities.send_modify(|caps|caps.can_stream=!catalog.is_empty()&&alive.load(Ordering::Acquire));
    let directory=media.lock().unwrap().catalog_reply(0,ready,&[]);
    for peer in peers.values(){reply(sender,peer.addr,directory.clone()).await;}
}
impl Drop for Peer {
    fn drop(&mut self) {
        let _ = self.cancel.send(());
        for task in &self.tasks {
            task.abort();
        }
    }
}
impl MobilePublisher {
    pub fn set_device_badge(&self, badge: protocol::appearance::DeviceBadge) -> Result<(),String> {
        let badge=match badge {
            protocol::appearance::DeviceBadge::Automatic=>protocol::appearance::DeviceBadge::Preset(protocol::appearance::DeviceBadgePreset::Phone),
            other=>remote_core::device_artwork::resolve(other)?,
        };
        let preset=match &badge {
            protocol::appearance::DeviceBadge::Preset(preset)=>*preset,
            protocol::appearance::DeviceBadge::Automatic | protocol::appearance::DeviceBadge::Custom(_)=>protocol::appearance::DeviceBadgePreset::Phone,
        };
        *self.device_badge.lock().unwrap()=badge;
        self.capabilities.send_modify(|capabilities|capabilities.device_badge=preset);
        Ok(())
    }
    pub fn set_device_identity(&self, identity:String) {
        self.delivery.set_identity(identity.clone());
        *self.identity.lock().unwrap()=Some(identity);
    }
    pub fn outgoing_status(&self) -> serde_json::Value {
        let mut status=self.outgoing.lock().unwrap().as_ref().map_or_else(
            ||serde_json::json!({"id":0,"active":false,"target":"","status":""}),
            |send|serde_json::json!({"id":send.id,"active":!send.task.is_finished(),"target":send.target,"status":*send.status.lock().unwrap(),
                "confirmation_timeout":*send.confirmation_timeout.lock().unwrap()}),
        );
        status["peers"]=if self.task.is_finished() {serde_json::json!([])}
            else {serde_json::json!(self.media.lock().unwrap().peer_streams)};
        status
    }
    pub fn stop_sending(&self) { if let Some(send)=self.outgoing.lock().unwrap().take() {self.media.lock().unwrap().clear_quality(send.ticket);} }
    pub fn stop_sending_id(&self, id:u64)->Result<(),String> {
        let mut outgoing=self.outgoing.lock().unwrap();
        if !outgoing.as_ref().is_some_and(|send|send.id==id) {return Err("Sending intention changed".into());}
        if let Some(send)=outgoing.take() {self.media.lock().unwrap().clear_quality(send.ticket);} Ok(())
    }
    pub(crate) fn set_outgoing_quality(&self, settings:crate::outgoing_quality::QualityRequest)->Result<Option<(u64,std::sync::mpsc::Receiver<Result<(),String>>)>,String> {
        settings.validate()?;
        let outgoing=self.outgoing.lock().unwrap();
        let send=outgoing.as_ref().filter(|send|send.id==settings.id && !send.task.is_finished())
            .ok_or("Sending intention changed")?;
        let grant=send.screen_grant.ok_or("Current delivery is not a screen")?;
        let mut media=self.media.lock().unwrap();
        if !send.quality_alive.load(Ordering::Acquire) {return Err("Sending intention changed".into());}
        if !media.sources.get(&CaptureSource::MainDisplay).is_some_and(|source|source.enabled&&source.grant==grant) {
            return Err("Screen sharing authorization changed".into());
        }
        media.begin_quality(crate::outgoing_quality::DeliveryQuality{ticket:send.ticket,source:CaptureSource::MainDisplay,grant,settings,revision:0})
    }
    pub(crate) fn complete_outgoing_quality(&self,id:u64,stream:u64,grant:u64,revision:u64,accepted:bool)->bool {
        self.media.lock().unwrap().complete_quality(id,stream,grant,revision,accepted)
    }
    pub(crate) fn cancel_outgoing_quality(&self,id:u64,revision:u64)->bool {self.media.lock().unwrap().cancel_quality(id,revision)}
    pub async fn send_screen(&self,target:String,endpoint:SocketAddr)->Result<(),String> {
        self.send_sources(target,endpoint,vec![CaptureSource::MainDisplay]).await
    }
    pub(crate) async fn send_screen_quality(&self,target:String,endpoint:SocketAddr,quality:crate::outgoing_quality::InitialQuality)->Result<u64,String> {
        quality.for_id(0).validate()?;
        self.send_sources_with_quality(target,endpoint,vec![CaptureSource::MainDisplay],Some(quality)).await
    }
    pub async fn send_sources(&self,target:String,endpoint:SocketAddr,sources:Vec<CaptureSource>)->Result<(),String> {
        self.send_sources_with_quality(target,endpoint,sources,None).await.map(|_|())
    }
    async fn send_sources_with_quality(&self,target:String,endpoint:SocketAddr,sources:Vec<CaptureSource>,initial:Option<crate::outgoing_quality::InitialQuality>)->Result<u64,String> {
        validate_delivery_sources(&sources).map_err(str::to_owned)?;
        if self.is_finished(){return Err("Camera and screen service is unavailable".into());}
        let grants={let media=self.media.lock().unwrap();sources.iter().map(|id|
            media.sources.get(id).filter(|source|source.enabled).map(|source|(*id,source.grant))
                .ok_or("Enable the selected camera or screen on this phone first"))
            .collect::<Result<Vec<_>,_>>()?};
        let screen_grant=grants.iter().find_map(|(source,grant)|(*source==CaptureSource::MainDisplay).then_some(*grant));
        let source=self.identity.lock().unwrap().clone().ok_or("Missing device identity")?;
        if target.is_empty() || target.len()>128 || target==source {return Err("Choose a paired receiving device".into());}
        let mut outgoing=self.outgoing.lock().unwrap();
        if outgoing.as_ref().is_some_and(|send|!send.task.is_finished()) {return Err("Stop the current delivery before choosing another target".into());}
        static SEQUENCE:std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(1);
        let sequence=SEQUENCE.fetch_update(Ordering::Relaxed,Ordering::Relaxed,|value|value.checked_add(1))
            .map_err(|_|"Sending sequence exhausted")?;
        let media=self.media.clone();
        let generation=self.generation.clone();
        let broker=self.delivery.clone();
        let scope=remote_core::shared_files::current_share_scope();
        let (stop,mut cancelled)=watch::channel(false);
        let status=Arc::new(Mutex::new("Contacting target".to_string()));
        let confirmation_timeout=Arc::new(Mutex::new(None));
        let timeout_evidence=confirmation_timeout.clone();
        let progress=status.clone();let target_id=target.clone();
        let mut ticket=[0;32];ticket[..16].copy_from_slice(&random_bytes_16());ticket[16..].copy_from_slice(&random_bytes_16());
        let quality_alive=Arc::new(AtomicBool::new(true));let task_quality_alive=quality_alive.clone();
        if let Some(initial)=initial {
            let grant=screen_grant.ok_or("Current delivery is not a screen")?;
            let mut media=self.media.lock().unwrap();
            if !media.sources.get(&CaptureSource::MainDisplay).is_some_and(|source|source.enabled&&source.grant==grant) {
                return Err("Screen sharing authorization changed".into());
            }
            media.set_quality(crate::outgoing_quality::DeliveryQuality{ticket,source:CaptureSource::MainDisplay,grant,settings:initial.for_id(sequence),revision:0});
        }
        let task=tokio::spawn(async move {
            type Error=Box<dyn std::error::Error+Send+Sync>;
            let run=async {
                let (conn,mut events)=tokio::select! {biased;_=cancelled.changed()=>return Ok::<(),Error>(()),
                    result=tokio::time::timeout(Duration::from_secs(5),remote_core::workspace_session::WorkspaceConnection::connect_media(endpoint))=>result??};
                let request=u64::from_le_bytes(ticket[..8].try_into().unwrap()).max(1);
                let live=||{let media=media.lock().unwrap();grants.iter().any(|(id,grant)|
                    media.sources.get(id).is_some_and(|source|source.enabled&&source.grant==*grant))};
                if scope!=remote_core::shared_files::current_share_scope() || !live() {
                    conn.close().await;return Err("Camera, screen sharing or device network changed".into());
                }
                let registration=broker.local(LocalIntent::Register {ticket,target:target_id.clone(),generation:generation.clone(),connection:conn.id,sequence,sources:sources.clone()});
                if let Err(error)=registration {conn.close().await;return Err(error.into());}
                let renewing=broker.clone();let renewing_generation=generation.clone();
                let result=remote_core::directed_delivery::maintain_offer(&conn,&mut events,
                    OutgoingOffer {source,target:target_id,ticket,request,sources,scope}, &mut cancelled,
                    live,
                    move |revision| {let broker=renewing.clone();let generation=renewing_generation.clone();
                        async move {broker.local(LocalIntent::Renew {ticket,generation,revision}).map_err(Error::from)}},
                    |state| *progress.lock().unwrap()=remote_core::directed_delivery::sending_status(state).into(),
                ).await;
                retire_quality(&media,&task_quality_alive,ticket);
                let revoked=broker.local(LocalIntent::Revoke {ticket,generation});
                let _=conn.control(SessionCommand::DeliveryCancel {request_id:request}).await;
                conn.close().await;revoked?;result
            }.await;
            retire_quality(&media,&task_quality_alive,ticket);
            *timeout_evidence.lock().unwrap()=run.as_ref().err().and_then(|error|
                error.downcast_ref::<remote_core::directed_delivery::ConfirmationTimeout>()).cloned();
            *progress.lock().unwrap()=match run {Ok(())=>"Sending stopped".into(),Err(error)=>format!("Delivery stopped: {error}")};
        });
        *outgoing=Some(Outgoing {id:sequence,target,ticket,screen_grant,quality_alive,stop,status,confirmation_timeout,task});Ok(sequence)
    }
    pub fn next_delivery(&self) -> Option<serde_json::Value> {
        let scope=remote_core::shared_files::current_share_scope();
        let mut deliveries=self.deliveries.lock().unwrap();
        deliveries.retain(|_,offer|offer.alive() && offer.peer.scope==scope);
        let mut incoming=self.incoming.lock().unwrap();
        while let Ok(offer)=incoming.try_recv() {
            if !offer.alive() || offer.peer.scope!=scope {continue;}
            let nonce=offer.peer.nonce.iter().map(|byte|format!("{byte:02x}")).collect::<String>();
            let id=format!("{}-{}-{nonce}",offer.peer.connection,offer.request);
            let result=serde_json::json!({"id":id,"source_device_id":offer.source,"target_device_id":offer.target,
                "sources":offer.sources,"remaining_ms":offer.snapshot().1.saturating_duration_since(Instant::now()).as_millis(),
                "claim":crate::delivery_claim_json::encode(DeliveryClaimProof {ticket:offer.ticket,target_device_id:offer.target.clone(),request_id:offer.request,sources:offer.sources.clone()})});
            deliveries.insert(id,offer);return Some(result);
        }
        None
    }
    pub fn delivery_alive(&self,id:&str)->bool {
        self.deliveries.lock().unwrap().get(id).is_some_and(|offer|
            offer.alive() && offer.peer.scope==remote_core::shared_files::current_share_scope())
    }
    pub fn delivery_state(&self,id:&str,state:DeliveryState,reason:&str)->Result<(),String> {
        if !matches!(state,DeliveryState::Accepted|DeliveryState::Receiving|DeliveryState::Rejected|DeliveryState::Cancelled) {
            return Err("Invalid receiving state".into());
        }
        let deliveries=self.deliveries.lock().unwrap();
        let offer=deliveries.get(id).filter(|offer|offer.alive() && offer.peer.scope==remote_core::shared_files::current_share_scope())
            .ok_or("Sending intent ended")?;
        offer.finish(state,&reason.chars().take(256).collect::<String>());Ok(())
    }
    pub async fn start(
        bind: SocketAddr,
        directory: PathBuf,
        allow_audio: bool,
    ) -> Result<Self, String> {
        let key = load_session_psk().ok_or("Pair this device before starting screen sharing")?;
        Self::start_with_key(bind, directory, key, allow_audio).await
    }
    async fn start_with_key(
        bind: SocketAddr,
        directory: PathBuf,
        key: Vec<u8>,
        allow_audio: bool,
    ) -> Result<Self, String> {
        Self::start_with_permissions(bind, directory, key, true, allow_audio).await
    }
    /// Accept paired file sessions without requesting or exposing screen capture.
    pub async fn start_receiver(bind: SocketAddr, directory: PathBuf) -> Result<Self, String> {
        let key = load_session_psk().ok_or("Join a device network before receiving files")?;
        Self::start_with_permissions(bind, directory, key, false, false).await
    }
    /// Foreground incoming-screen listener grants neither capture nor file access.
    pub async fn start_delivery_listener(bind: SocketAddr, directory: PathBuf) -> Result<Self, String> {
        let key = load_session_psk().ok_or("Join a device network before receiving screens")?;
        Self::start_with_access(bind, directory, key, false, false, false).await
    }
    async fn start_with_permissions(
        bind: SocketAddr, directory: PathBuf, key: Vec<u8>, capture_enabled: bool, allow_audio: bool,
    ) -> Result<Self, String> {
        Self::start_with_access(bind, directory, key, capture_enabled, allow_audio, true).await
    }
    async fn start_with_access(
        bind: SocketAddr,
        directory: PathBuf,
        key: Vec<u8>,
        capture_enabled: bool,
        allow_audio: bool,
        allow_files: bool,
    ) -> Result<Self, String> {
        let mux = UdpMultiplexer::bind(&bind.to_string())
            .await
            .map_err(|e| e.to_string())?;
        let address = mux.local_addr().map_err(|e| e.to_string())?;
        let (sender, receiver) = mux.split();
        let media=Arc::new(Mutex::new(MediaSources::new(capture_enabled,allow_audio)));
        let shared=media.clone();
        let device_badge=Arc::new(Mutex::new(protocol::appearance::DeviceBadge::Preset(protocol::appearance::DeviceBadgePreset::Phone)));
        let shared_badge=device_badge.clone();
        let (media_changed,mut changed_rx)=watch::channel(1u64);
        let (artwork_changed,mut artwork_rx)=watch::channel(());
        let (frames,mut frame_rx)=mpsc::channel::<Frame>(4);
        let (errors,mut error_rx)=mpsc::channel::<CaptureFailure>(8);
        let mut advertised = crate::inbound_capabilities(address.port(), capture_enabled);
        advertised.file_transfer = allow_files;
        let (capabilities, _) = watch::channel(advertised);
        let file_reception = Arc::new(AtomicBool::new(allow_files));
        let peer_files = file_reception.clone();
        let source_capabilities=capabilities.clone();
        let alive=Arc::new(AtomicBool::new(true));let task_alive=alive.clone();
        let input = Arc::new(Mutex::new(crate::publisher_input::PublisherInput::default()));
        let shared_input = input.clone();
        let (delivery,incoming)=Broker::new();
        let generation=format!("android-runtime-{:032x}",u128::from_le_bytes(random_bytes_16()));
        delivery.verified_source_generation(&generation);
        let peer_delivery=delivery.clone();
        let scope=Some(remote_core::shared_files::share_scope_for_key(&key));
        let advertised_lifetime = AdvertisedPublisher(capabilities.clone(), input.clone(),alive.clone());
        // Allocate the large publisher future before handing it to the executor.
        // This avoids extra stack copies on Android-sized worker/test stacks.
        let task = tokio::spawn(Box::pin(async move {
            let _advertised_lifetime = advertised_lifetime;
            let mut peers: HashMap<SocketAddr, Peer> = HashMap::new();
            let mut authenticated: HashMap<SocketAddr, ([u8;16], [u8;16], Instant)> = HashMap::new();
            let mut artwork_requests=HashMap::new();
            let mut handshakes=remote_core::session_auth::ServerHandshake::default();
            let mut maintenance = tokio::time::interval(Duration::from_secs(1));
            let mut input_watchdog = tokio::time::interval(Duration::from_millis(protocol::realtime_input::INPUT_WATCHDOG_INTERVAL_MS));
            input_watchdog.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut input_ready = false;
            loop {
                tokio::select! {
                    biased;
                    changed=artwork_rx.changed()=>{
                        if changed.is_err(){break;}
                        artwork_rx.borrow_and_update();
                        finish_artwork(&mut artwork_requests,&peers,&authenticated,&shared,&shared_badge,&sender).await;
                    }
                    changed=changed_rx.changed()=>{
                        if changed.is_err(){break;}
                        changed_rx.borrow_and_update();
                        refresh_sources(&mut peers,&shared,&shared_input,&sender,&source_capabilities,&task_alive).await;
                        finish_artwork(&mut artwork_requests,&peers,&authenticated,&shared,&shared_badge,&sender).await;
                    }
                    Some(failure)=error_rx.recv()=>{match failure {
                      CaptureFailure::Source(source,epoch,reason)=>{
                        if shared.lock().unwrap().sources.get(&source).is_none_or(|entry|entry.enabled||entry.demand.epoch!=epoch){continue;}
                        for peer in peers.values() {
                            for sub in peer.subscriptions.values().filter(|sub|sub.request.source==source) {
                                reply(&sender,peer.addr,SessionCommand::Error{request_id:sub.request.id,reason:reason.clone()}).await;
                            }
                        }
                        refresh_sources(&mut peers,&shared,&shared_input,&sender,&source_capabilities,&task_alive).await;
                    }
                      CaptureFailure::ScreenView{stream,grant,reason}=>{
                        for peer in peers.values_mut() {
                            let ended=peer.subscriptions.iter().find(|(_,sub)|sub.screen_stream==stream&&sub.grant==grant).map(|(id,_)|*id);
                            if let Some(id)=ended {
                                shared_input.lock().unwrap().revoke_peer(peer.addr);
                                peer.subscriptions.remove(&id);peer.retired.insert(id);
                                reply(&sender,peer.addr,SessionCommand::Error{request_id:id,reason:reason.clone()}).await;
                                reply(&sender,peer.addr,SessionCommand::Unsubscribed{id}).await;
                            }
                        }
                        recompute(&peers,&shared);
                    }}}
                    _ = input_watchdog.tick(), if input_ready || shared_input.lock().unwrap().ready() => {
                        let (ready, handled) = {
                            let mut input = shared_input.lock().unwrap();
                            (input.ready(), input.take_handled())
                        };
                        if ready != input_ready {
                            input_ready = ready;
                            for p in peers.values() {
                                for sub in p.subscriptions.values() {
                                    let audio=sub.audio;
                                    reply(&sender,p.addr,SessionCommand::Subscribed{id:sub.request.id,
                                        audio_owner:audio.then_some(sub.request.id),supports_input:ready&&sub.input_allowed(), max_touch_points: if ready&&sub.input_allowed() {protocol::touch::MAX_CONTACTS as u8} else {0} }).await;
                                }
                            }
                        }
                        for ack in handled {
                            let command=match ack.text {
                                Some((request_id,true))=>SessionCommand::TextCommitted {binding:ack.binding,request_id},
                                Some((request_id,false))=>SessionCommand::TextRejected {binding:ack.binding,request_id,reason:"Android could not commit text through the authorized input service".into()},
                                None=>SessionCommand::InputHandled {binding:ack.binding,sequence:ack.sequence},
                            };
                            reply(&sender,ack.addr,command).await;
                        }
                    }
                    _ = maintenance.tick() => {
                        finish_artwork(&mut artwork_requests,&peers,&authenticated,&shared,&shared_badge,&sender).await;
                        peer_delivery.reconcile(|owner|peers.get(&owner.addr).is_some_and(|peer|peer.id==owner.connection)
                            && authenticated.get(&owner.addr).is_some_and(|(_,nonce,_)|*nonce==owner.nonce));
                        let ended=peers.iter().filter(|(_,peer)|peer.delivery_owner.is_some_and(|(owner,ticket)|!peer_delivery.claim_alive(owner,ticket)))
                            .map(|(addr,_)|*addr).collect::<Vec<_>>();
                        for addr in ended {
                            if let Some(peer)=peers.get_mut(&addr) {
                                for id in peer.retire_media(&shared_input) {
                                    reply(&sender,addr,SessionCommand::Unsubscribed{id}).await;
                                }
                            }
                        }
                        recompute(&peers,&shared);
                        handshakes.expire(remote_core::session_auth::AuthTime::now());
                        authenticated.retain(|addr, (_, _, at)| {
                            let expired = peers.get(addr).map_or_else(
                                || at.elapsed() > Duration::from_secs(30),
                                |p| p.seen.elapsed() > Duration::from_secs(30),
                            );
                            if expired {
                                shared_input.lock().unwrap().revoke_peer(*addr);
                                peers.remove(addr);
                                sender.remove_peer_crypto(*addr);
                                handshakes.forget(*addr);
                            }
                            !expired
                        });
                        recompute(&peers,&shared);
                        for peer in peers.values() {
                            for sub in peer.subscriptions.values().filter(|sub|sub.audio) {
                                let config=protocol::AudioStreamConfig::remote_system(protocol::remote_system_audio_stream_id(sub.request.id),48000,2,20);
                                if let Ok(envelope)=remote_core::media_plane::audio_stream_config_to_envelope(&config,0,now_unix_ms()) {
                                    let _=peer.scheduled.send(envelope).await;
                                }
                            }
                        }
                    }
                    Some(frame)=frame_rx.recv()=>{
                        let grant={let media=shared.lock().unwrap();
                            let accepted=frame.screen_stream.map_or_else(||media.accepts_frame(frame.source,frame.epoch,frame.audio),
                                |stream|media.accepts_screen_geometry(stream,frame.epoch,frame.geometry_revision));
                            if !accepted{continue;}
                            media.sources[&frame.source].grant
                        };
                        for peer in peers.values_mut() {
                            if peer.delivery_owner.is_some_and(|(owner,ticket)|!peer_delivery.claim_alive(owner,ticket)){continue;}
                            for sub in peer.subscriptions.values_mut().filter(|sub|sub.request.source==frame.source) {
                                if sub.grant!=grant || frame.screen_stream.is_some_and(|stream|stream!=sub.screen_stream)
                                    || (frame.audio&&!sub.audio)||(!frame.audio&&!sub.video){continue;}
                                let (id,payload_type,sequence)=if frame.audio {
                                    (protocol::remote_system_audio_stream_id(sub.request.id),protocol::PayloadType::AudioOpus as u8,&mut sub.audio_sequence)
                                } else {(sub.request.id,96,&mut sub.video_sequence)};
                                let packet=RtpPacket{header:RtpHeader{version:2,payload_type,sequence_number:*sequence,timestamp:frame.timestamp,ssrc:id},payload:frame.bytes.clone()};
                                *sequence=sequence.wrapping_add(1);
                                if let Ok(mut envelope)=remote_core::media_plane::rtp_to_realtime_data(&packet){
                                    envelope.transport_timing=frame.timing.map(|mut timing|{
                                        timing.packetize_ts_us=remote_core::timing::quanta_now_us().saturating_sub(timing.capture_ts_us).min(u64::from(u32::MAX)) as u32;timing
                                    });
                                    let _=peer.scheduled.send(envelope).await;
                                }
                            }
                        }
                    }
                    packet = receiver.recv() => {
                        let Ok(packet) = packet else { continue; };
                        match packet {
                            MultiplexedPacket::Control(ControlMessage::SessionHandshake(message),addr)=>{
                                let allow_new=!authenticated.contains_key(&addr) && authenticated.len()<MAX_PUBLISHER_PEERS;
                                if let Ok(response)=handshakes.receive(&key,&message,addr,remote_core::session_auth::AuthTime::now(),allow_new){
                                    if let Some(established)=response.established{
                                        sender.install_peer_crypto(addr,established.crypto);
                                        authenticated.insert(addr,(established.session_id,established.client_nonce,Instant::now()));
                                    }
                                    let _=sender.send_control(&ControlMessage::SessionHandshake(Box::new(response.message)),addr).await;
                                }
                            }
                            MultiplexedPacket::Control(ControlMessage::SessionHello{..}|ControlMessage::SessionAccept{..},addr)=>{
                                let _=sender.send_control(&ControlMessage::SessionReject{reason:"Weavelo authentication protocol mismatch: version 3 required".into()},addr).await;
                            }
                            MultiplexedPacket::Control(ControlMessage::SessionReject{..},_)=>{}
                            MultiplexedPacket::ClockProbe(probe,host_recv_us,addr)=>{
                                if authenticated.get(&addr).is_some_and(|(id,..)|*id==probe.session_id) && let Some(p)=peers.get_mut(&addr){
                                    p.seen=Instant::now();
                                    let reply=protocol::session_auth::ClockReply{probe,host_recv_us,host_send_us:remote_core::timing::quanta_now_us()};
                                    let _=sender.send_control(&ControlMessage::ClockReply(reply),addr).await;
                                }
                            }
                            MultiplexedPacket::ClockReply(..)=>{}
                            MultiplexedPacket::Control(message, addr) => {
                                if !authenticated.contains_key(&addr) { continue; }
                                if let Some(p) = peers.get(&addr)
                                    && p.delivery_owner.is_some_and(|(owner,_)| authenticated.get(&addr).is_none_or(|(_,nonce,_)|*nonce!=owner.nonce)) {
                                    shared_input.lock().unwrap().revoke_peer(addr);
                                    peers.remove(&addr);recompute(&peers,&shared);
                                    authenticated.remove(&addr);sender.remove_peer_crypto(addr);continue;
                                }
                                if let Some(peer)=peers.get_mut(&addr)
                                    && peer.delivery_owner.is_some_and(|(owner,ticket)|!peer_delivery.claim_alive(owner,ticket)) {
                                    for id in peer.retire_media(&shared_input){reply(&sender,addr,SessionCommand::Unsubscribed{id}).await;}
                                    recompute(&peers,&shared);
                                }
                                if let Some(p) = peers.get_mut(&addr) { p.seen = Instant::now(); }
                                match message {
                                    ControlMessage::Session(command) => match *command {
                                        SessionCommand::Open { connection_id, version:SESSION_VERSION } => {
                                            if peers.get(&addr).is_some_and(|p| p.id != connection_id) { reply(&sender, addr, SessionCommand::Error { request_id:connection_id, reason:"close the previous connection first".into() }).await; continue; }
                                            peers.entry(addr).or_insert_with(|| Peer::new(addr, connection_id, &sender, directory.clone(), peer_files.load(Ordering::Relaxed)));
                                            let window_capture=shared.lock().unwrap().sources.values().any(|source|source.enabled&&matches!(source.info.source,CaptureSource::Window(_)));
                                            reply(&sender, addr, SessionCommand::Opened { connection_id, version:SESSION_VERSION, max_subscriptions:MAX_SUBSCRIPTIONS as u32, files:peers[&addr].files.is_some(), clipboard:false, window_capture }).await;
                                        }
                                        SessionCommand::Open { connection_id, .. } => {
                                            reply(&sender,addr,SessionCommand::Error {request_id:connection_id,reason:"unsupported connection version".into()}).await;
                                        }
                                        SessionCommand::DeliveryOffer {request_id,source_device_id,target_device_id,ticket,revision,sources} if peers.contains_key(&addr) => {
                                            let peer=&peers[&addr];let (_,nonce,_)=authenticated[&addr];
                                            let owner=DeliveryPeer {addr,connection:peer.id,nonce,scope};
                                            reply(&sender,addr,peer_delivery.offer(owner,request_id,source_device_id,target_device_id,ticket,revision,sources)).await;
                                        }
                                        SessionCommand::DeliveryClaim {request_id,target_device_id,ticket,sources} if peers.contains_key(&addr) => {
                                            let peer=peers.get_mut(&addr).unwrap();let (_,nonce,_)=authenticated[&addr];
                                            let owner=DeliveryPeer {addr,connection:peer.id,nonce,scope};
                                            let response=if peer.delivery_owner.is_some_and(|binding|binding!=(owner,ticket))
                                                || (peer.delivery_owner.is_none() && !peer.subscriptions.is_empty()) {
                                                SessionCommand::DeliveryReceipt {request_id,revision:0,state:DeliveryState::Rejected,reason:"Connection belongs to another activity".into()}
                                            } else {peer_delivery.claim(owner,request_id,&target_device_id,ticket,sources)};
                                            if matches!(response,SessionCommand::DeliveryReceipt {state:DeliveryState::Claimed,..}) {peer.delivery_owner=Some((owner,ticket));}
                                            reply(&sender,addr,response).await;
                                        }
                                        SessionCommand::DeliveryCancel {request_id} if peers.contains_key(&addr) => {
                                            let peer=&peers[&addr];let (_,nonce,_)=authenticated[&addr];
                                            reply(&sender,addr,peer_delivery.cancel(DeliveryPeer {addr,connection:peer.id,nonce,scope},request_id)).await;
                                        }
                                        SessionCommand::GetSourceAppearance{request_id,id,source} if peers.contains_key(&addr)=>{
                                            finish_artwork(&mut artwork_requests,&peers,&authenticated,&shared,&shared_badge,&sender).await;
                                            if let Some(sub)=peers[&addr].subscriptions.get(&id).filter(|sub|sub.request.source==source)
                                                && matches!(source,CaptureSource::Window(_)) {
                                                if artwork_requests.get(&(addr,id)).is_some_and(|pending|pending.request_id==request_id) {continue;}
                                                if artwork_requests.len()>=64 || artwork_requests.contains_key(&(addr,id)) {
                                                    reply(&sender,addr,SessionCommand::SourceAppearance{request_id,id,source,appearance:Box::new(Err("Application artwork request is already pending".into()))}).await;
                                                } else {
                                                    artwork_requests.insert((addr,id),PendingArtwork{request_id,connection:peers[&addr].id,
                                                        nonce:authenticated[&addr].1,source,grant:sub.grant,deadline:Instant::now()+Duration::from_secs(5)});
                                                    finish_artwork(&mut artwork_requests,&peers,&authenticated,&shared,&shared_badge,&sender).await;
                                                }
                                                continue;
                                            }
                                            let response=if peers[&addr].subscriptions.get(&id).is_some_and(|sub|sub.request.source==source) {
                                                SessionCommand::SourceAppearance{request_id,id,source,appearance:Box::new(Ok(protocol::appearance::SourceAppearance{application:None,device:shared_badge.lock().unwrap().clone()}))}
                                            } else {SessionCommand::SourceAppearance{request_id,id,source,appearance:Box::new(Err("Artwork source is no longer selected".into()))}};
                                            reply(&sender,addr,response).await;
                                        }
                                        SessionCommand::ListSources{request_id,known_icons} if peers.contains_key(&addr)=>{
                                            let ready=shared_input.lock().unwrap().ready();
                                            let catalog=shared.lock().unwrap().catalog_reply(request_id,ready,&known_icons);
                                            reply(&sender,addr,catalog).await;
                                        }
                                        SessionCommand::RequestCameraSwitch(request) if peers.contains_key(&addr) => {
                                            let peer=&peers[&addr];
                                            let ready=(|| -> Result<(),String> {
                                                let sub=peer.subscriptions.get(&request.id).ok_or("The camera subscription ended")?;
                                                let delivery_request = match peer.delivery_owner {
                                                    Some((owner, ticket)) => {
                                                        if authenticated.get(&addr).is_none_or(|(_, nonce, _)| *nonce != owner.nonce) {
                                                            return Err("Sending intent ended or source changed".into());
                                                        }
                                                        Some(
                                                            peer_delivery
                                                                .claimed_request_id(owner, ticket)
                                                                .ok_or("Sending intent ended")?,
                                                        )
                                                    }
                                                    None => None,
                                                };
                                                let catalog=shared.lock().unwrap().catalog(false);
                                                remote_core::camera_switch::validate(&request,remote_core::camera_switch::CameraView {
                                                    source:sub.request.source,revision:0,delivery_request,
                                                },&catalog).map_err(str::to_owned)?;
                                                let others=peers.values().flat_map(|other|other.subscriptions.values()
                                                    .filter(move |sub|other.addr!=addr||sub.request.id!=request.id))
                                                    .filter(|sub|sub.video).map(|sub|sub.request.source);
                                                if remote_core::camera_switch::exclusive_camera_busy(request.target,others) {
                                                    return Err("Another viewer is using this camera".into());
                                                }
                                                if let (Some((owner, ticket)), Some(request_id)) = (peer.delivery_owner, delivery_request) {
                                                    peer_delivery.switch_camera_source(owner, ticket, request_id, request.from, request.target).map_err(str::to_owned)?;
                                                }
                                                Ok(())
                                            })();
                                            reply(&sender,addr,SessionCommand::CameraSwitchResult {request,ready}).await;
                                        }
                                        SessionCommand::Subscribe(request) if peers.contains_key(&addr) => {
                                            let peer=&peers[&addr];
                                            if let Some((owner,ticket))=peer.delivery_owner
                                                && (!peer_delivery.allows_source(owner,ticket,request.source)
                                                    || authenticated.get(&addr).is_none_or(|(_,nonce,_)|*nonce!=owner.nonce)) {
                                                reply(&sender,addr,SessionCommand::Error {request_id:request.id,reason:"Sending intent ended or source changed".into()}).await;continue;
                                            }
                                            let source=shared.lock().unwrap().sources.get(&request.source).filter(|source|source.enabled).cloned();
                                            let Some(source)=source else {
                                                reply(&sender,addr,SessionCommand::Error{request_id:request.id,reason:"Selected source is unavailable; refresh this phone's sources".into()}).await;continue;
                                            };
                                            let existing=peer.subscriptions.get(&request.id);
                                            let invalid=!valid(&request)||peer.retired.contains(&request.id)
                                                ||existing.is_some_and(|sub|sub.request.source!=request.source||sub.request.audio!=request.audio)
                                                ||(existing.is_none()&&(peer.subscriptions.len()>=MAX_SUBSCRIPTIONS||peer.retired.len()+peer.subscriptions.len()>=4096));
                                            if invalid {
                                                reply(&sender,addr,SessionCommand::Error{request_id:request.id,reason:"Invalid or closed subscription; use a fresh source subscription".into()}).await;continue;
                                            }
                                            if matches!(request.source,CaptureSource::Camera(_))&&peers.values().flat_map(|peer|peer.subscriptions.values())
                                                .any(|sub|sub.video&&matches!(sub.request.source,CaptureSource::Camera(_))&&sub.request.source!=request.source) {
                                                reply(&sender,addr,SessionCommand::Error{request_id:request.id,reason:"Another camera is in use; stop that camera before switching".into()}).await;continue;
                                            }
                                            let id=request.id;let audio=request.audio&&source.audio;
                                            let supports_input=screen_source(request.source)&&shared_input.lock().unwrap().ready();
                                            peers.get_mut(&addr).unwrap().subscriptions.entry(id).or_insert_with(||MobileSubscription::new(request,source.grant,audio));
                                            recompute(&peers,&shared);
                                            reply(&sender,addr,SessionCommand::Subscribed{id,audio_owner:audio.then_some(id),supports_input, max_touch_points: if supports_input {protocol::touch::MAX_CONTACTS as u8} else {0} }).await;
                                        }
                                        SessionCommand::SetActivity{id,revision,video,audio} if peers.contains_key(&addr)=>{
                                            let Some(sub)=peers[&addr].subscriptions.get(&id)else{continue;};
                                            let source=sub.request.source;
                                            if video&&matches!(source,CaptureSource::Camera(_))&&peers.values().flat_map(|peer|peer.subscriptions.values())
                                                .any(|sub|sub.video&&matches!(sub.request.source,CaptureSource::Camera(_))&&sub.request.source!=source) {
                                                reply(&sender,addr,SessionCommand::Error{request_id:id,reason:"Another camera is in use".into()}).await;continue;
                                            }
                                            let allowed_audio=shared.lock().unwrap().sources.get(&source).is_some_and(|source|source.audio);
                                            let sub=peers.get_mut(&addr).unwrap().subscriptions.get_mut(&id).unwrap();
                                            if revision>sub.activity_revision {
                                                if !video&&sub.input_allowed(){shared_input.lock().unwrap().revoke_peer(addr);}
                                                sub.activity_revision=revision;sub.video=video;sub.audio=audio&&sub.request.audio&&allowed_audio;
                                            }
                                            let response=SessionCommand::Activity{id,revision:sub.activity_revision,video:sub.video,audio:sub.audio};
                                            recompute(&peers,&shared);reply(&sender,addr,response).await;
                                        }
                                        SessionCommand::Viewport(demand) if peers.contains_key(&addr)=>{
                                            let result=peers[&addr].subscriptions.get(&demand.id)
                                                .filter(|sub|screen_source(sub.request.source))
                                                .ok_or_else(||"Screen viewport subscription is unavailable".to_owned())
                                                .and_then(|sub|shared.lock().unwrap().viewport(sub.screen_stream,demand));
                                            let response=match result {
                                                Ok(revision)=>SessionCommand::ViewportAccepted{id:demand.id,source_revision:demand.source_revision,revision},
                                                Err(reason)=>SessionCommand::Error{request_id:demand.id,reason},
                                            };
                                            reply(&sender,addr,response).await;
                                        }
                                        SessionCommand::Configure{id,revision,width,height,fps,bitrate_kbps} if peers.contains_key(&addr)=>{
                                            let Some(sub)=peers.get_mut(&addr).unwrap().subscriptions.get_mut(&id)else{continue;};
                                            let updated=SubscriptionRequest{width,height,fps,bitrate_kbps,..sub.request.clone()};
                                            if !valid(&updated){reply(&sender,addr,SessionCommand::Error{request_id:id,reason:"Unsupported stream settings".into()}).await;continue;}
                                            if revision>sub.settings_revision{sub.settings_revision=revision;sub.request=updated;}
                                            let stream=sub.screen_stream;
                                            let response=SessionCommand::Configured{id,revision:sub.settings_revision};
                                            recompute(&peers,&shared);
                                            shared.lock().unwrap().request_view_keyframe(stream);
                                            reply(&sender,addr,response).await;
                                        }
                                        SessionCommand::ResizeWindow{id,source_revision,request_id,target_size} if peers.contains_key(&addr)=>{
                                            let response=match peers.get(&addr).and_then(|p|p.subscriptions.get(&id)) {
                                                None=>SessionCommand::WindowResized{id,source_revision,request_id,applied_size:Err("独立应用订阅不存在或已关闭".into())},
                                                Some(_) if source_revision!=0=>SessionCommand::WindowResized{id,source_revision,request_id,applied_size:Err("独立应用会话源版本已过期".into())},
                                                Some(sub)=>{
                                                    let source=sub.request.source;
                                                    let applied=shared.lock().unwrap().set_window_target_size(source,target_size);
                                                    if applied.is_ok() {shared_input.lock().unwrap().revoke_peer(addr);}
                                                    SessionCommand::WindowResized{id,source_revision,request_id,applied_size:applied}
                                                }
                                            };
                                            reply(&sender,addr,response).await;
                                        }
                                        SessionCommand::Unsubscribe{id} if peers.contains_key(&addr)=>{
                                            let peer=peers.get_mut(&addr).unwrap();
                                            let reserved=peer.subscriptions.len()-usize::from(peer.subscriptions.contains_key(&id));
                                            if !peer.retired.contains(&id)&&peer.retired.len()+reserved>=4096 {
                                                reply(&sender,addr,SessionCommand::Error{request_id:id,reason:"Subscription ID capacity reached; reconnect".into()}).await;continue;
                                            }
                                            peer.retired.insert(id);
                                            if peer.subscriptions.remove(&id).is_some_and(|sub|screen_source(sub.request.source)){shared_input.lock().unwrap().revoke_peer(addr);}
                                            recompute(&peers,&shared);reply(&sender,addr,SessionCommand::Unsubscribed{id}).await;
                                        }
                                        SessionCommand::RequestInputBinding {connection_id,id,source_revision:0,presented_geometry_revision,request}
                                            if request != 0 && peers.get(&addr).is_some_and(|p|p.id==connection_id&&p.subscriptions.get(&id).is_some_and(MobileSubscription::input_allowed)) => {
                                            let sub=&peers[&addr].subscriptions[&id];
                                            let source=sub.request.source;
                                            // Hold geometry through grant: a capture reconfiguration must
                                            // not slip between the floor check and lease creation.
                                            let binding = {
                                                let media=shared.lock().unwrap();
                                                if !media.accepts_screen_geometry(sub.screen_stream,sub.grant,presented_geometry_revision) {continue;}
                                                shared_input.lock().unwrap().grant_source(addr,connection_id,id,source)
                                            };
                                            if let Some(binding) = binding {
                                                reply(&sender,addr,SessionCommand::InputBinding {binding,request,host_time_ms:crate::publisher_input::now_ms()}).await;
                                            }
                                        }
                                        SessionCommand::CommitText(request) if peers.get(&addr).is_some_and(|p|p.id==request.binding.connection_id&&p.subscriptions.get(&request.binding.subscription_id).is_some_and(MobileSubscription::input_allowed)) => {
                                            let response=shared_input.lock().unwrap().admit_text(addr,&request);
                                            if let Some(response)=response {reply(&sender,addr,response).await;}
                                        }
                                        SessionCommand::RealtimeInput(packet) if peers.get(&addr).is_some_and(|p|p.id==packet.binding.connection_id&&p.subscriptions.get(&packet.binding.subscription_id).is_some_and(MobileSubscription::input_allowed)) => {
                                            shared_input.lock().unwrap().admit(addr,&packet);
                                        }
                                        SessionCommand::ReleaseInput {id} if peers.get(&addr).is_some_and(|p|p.subscriptions.get(&id).is_some_and(|sub|screen_source(sub.request.source))) => {
                                            shared_input.lock().unwrap().revoke_peer(addr);
                                            reply(&sender,addr,SessionCommand::InputReleased {id}).await;
                                        }
                                        SessionCommand::Close { connection_id } if peers.get(&addr).is_some_and(|p| p.id==connection_id) => {
                                            shared_input.lock().unwrap().revoke_peer(addr);
                                            let peer=peers.remove(&addr).unwrap();
                                            recompute(&peers,&shared);
                                            let response=match peer.shutdown().await {
                                                Ok(())=>SessionCommand::Closed {connection_id,reason:"Subscriptions ended, input lease revoked and connection workers stopped".into()},
                                                Err(reason)=>SessionCommand::Error {request_id:connection_id,reason},
                                            };
                                            // Send the authenticated receipt before retiring its key.
                                            reply(&sender,addr,response).await;
                                            authenticated.remove(&addr);
                                            sender.remove_peer_crypto(addr);
                                            handshakes.forget(addr);
                                        }
                                        _ => {}
                                    },
                                    ControlMessage::Ping { client_send_ts } if peers.contains_key(&addr) => { let now=now_unix_ms(); let _=sender.send_control(&ControlMessage::Pong { client_send_ts,host_recv_ts:now,host_send_ts:now },addr).await; }
                                    ControlMessage::RequestKeyframe{session_id}=>{
                                        if let Some(sub)=peers.get(&addr).and_then(|peer|peer.subscriptions.get(&session_id)) {
                                            let mut media=shared.lock().unwrap();
                                            if screen_source(sub.request.source){media.request_view_keyframe(sub.screen_stream);}
                                            else {media.request_keyframe(sub.request.source);}
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            MultiplexedPacket::Data(envelope, addr) | MultiplexedPacket::DataWithTiming(envelope, _, addr) => {
                                if let Some(p)=peers.get_mut(&addr) { p.seen=Instant::now(); if let Some(files) = &p.files { let _=files.try_send(envelope); } }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }));
        Ok(Self {
            device_badge,
            address,
            media,
            media_changed,
            artwork_changed,
            generation,
            alive,
            frames,
            errors,
            input,
            capabilities,
            task,file_reception,identity:Mutex::new(None),outgoing:Mutex::new(None),delivery,incoming:Mutex::new(incoming),deliveries:Mutex::new(HashMap::new()),
        })
    }
    /// Detach or attach media without replacing the authenticated file connection.
    fn notify_sources(&self) {
        self.media_changed.send_modify(|revision|*revision=revision.wrapping_add(1));
        let available=self.media.lock().unwrap().sources.values().any(|source|source.enabled);
        self.capabilities.send_modify(|caps|caps.can_stream=available&&!self.is_finished());
    }
    pub fn set_capture_available(&self,enabled:bool,audio:bool) {
        if !enabled {let _=self.media.lock().unwrap().set_applications(vec![]);}
        if self.media.lock().unwrap().set_screen(enabled,audio){
            self.input.lock().unwrap().set_capture(false);
            self.notify_sources();
        }
    }
    pub fn set_applications(&self,infos:Vec<CaptureSourceInfo>)->Result<(),String> {
        self.media.lock().unwrap().set_applications(infos)?;
        self.input.lock().unwrap().revoke();
        self.notify_sources();Ok(())
    }
    pub fn set_catalog_icons(&self,icons:Vec<(CaptureSource,protocol::interaction::ApplicationIcon)>)->Result<(),String> {
        self.media.lock().unwrap().set_catalog_icons(icons)?;
        self.notify_sources();Ok(())
    }
    pub fn application_demands(&self)->serde_json::Value {
        let mut media=self.media.lock().unwrap();
        // The catalog can contain hundreds of installed apps. Only subscribed
        // apps belong in the 20 Hz JNI demand poll; retain paused subscriptions.
        let ids=media.sources.iter().filter(|(id,entry)|matches!(id,CaptureSource::Window(_))
            && entry.enabled && entry.demand.subscriptions>0).map(|(id,_)|*id).collect::<Vec<_>>();
        serde_json::Value::Array(ids.into_iter().map(|source| {
            let requested_size=media.sources[&source].target_window_size.map(|(w,h)|[w,h]);
            serde_json::json!({"source":source,"artwork_epoch":media.sources[&source].artwork_epoch,"requested_size":requested_size,"demand":media.screen_source_demand(source)})
        }).collect())
    }
    pub fn set_application_artwork(&self,source:CaptureSource,grant:u64,epoch:u64,icon:Option<protocol::interaction::ApplicationIcon>)->Result<(),String> {
        if let Some(icon)=&icon {remote_core::application_icons::decode_artwork(icon,true)?;}
        {
            let mut media=self.media.lock().unwrap();
            let entry=media.sources.get_mut(&source).filter(|entry|matches!(source,CaptureSource::Window(_))
                && entry.enabled&&entry.grant==grant&&entry.artwork_epoch==epoch&&entry.demand.subscriptions>0)
                .ok_or("Application artwork source is no longer selected")?;
            entry.artwork=Some(icon);
        }
        self.artwork_changed.send_replace(());Ok(())
    }
    pub fn set_cameras(&self,cameras:Vec<remote_core::cameras::NativeCamera>)->Result<Vec<(CaptureSource,String)>,String> {
        let bindings=self.media.lock().unwrap().set_cameras(cameras)?;self.notify_sources();Ok(bindings)
    }
    pub fn camera_demands(&self)->serde_json::Value {
        let media=self.media.lock().unwrap();
        serde_json::Value::Array(media.sources.iter().filter(|(id,source)|matches!(id,CaptureSource::Camera(_))&&source.enabled)
            .map(|(id,source)|serde_json::json!({"source":id,"camera_id":media.camera_key(*id),"demand":source.demand,"camera":source.info.camera})).collect())
    }
    pub fn set_file_reception(&self,enabled:bool) {
        self.file_reception.store(enabled,Ordering::Relaxed);
        self.capabilities.send_modify(|caps|caps.file_transfer=enabled&&!self.is_finished());
    }
    pub fn set_input_service_available(&self,available:bool){self.input.lock().unwrap().set_service(available);}
    pub fn set_input_capture_allowed(&self,allowed:bool){let enabled=self.media.lock().unwrap().enabled(CaptureSource::MainDisplay);self.input.lock().unwrap().set_capture(allowed&&enabled);}
    pub fn input_dispatch(&self)->Result<String,String>{serde_json::to_string(&self.input.lock().unwrap().poll()).map_err(|error|error.to_string())}
    pub fn input_execution_valid(&self,generation:u64,token:u64)->bool{self.input.lock().unwrap().execution_valid(generation,token)}
    pub fn complete_input(&self,generation:u64,token:u64,accepted:bool){self.input.lock().unwrap().complete(generation,token,accepted);}
    pub fn capabilities(&self)->watch::Receiver<remote_core::discovery::DiscoveryCapabilities>{self.capabilities.subscribe()}
    pub fn demand(&self)->CaptureDemand{self.media.lock().unwrap().screen_demand()}
    pub fn is_finished(&self)->bool{!self.alive.load(Ordering::Acquire)||self.task.is_finished()}
    pub fn source_grant(&self,source:CaptureSource)->Option<u64>{self.media.lock().unwrap().sources.get(&source).filter(|source|source.enabled).map(|source|source.grant)}
    pub fn configure_geometry(&self,source:CaptureSource,epoch:u64,width:u32,height:u32,rotation:u16)->Result<u64,String> {
        let mut media=self.media.lock().unwrap();
        let result=media.geometry(source,epoch,width,height,rotation)?;
        if result!=epoch && screen_source(source) {self.input.lock().unwrap().revoke();}
        drop(media);
        if result!=epoch {self.notify_sources();}
        Ok(result)
    }
    pub async fn frame_from(&self,source:CaptureSource,epoch:u64,bytes:Vec<u8>,_timestamp_us:i64,audio:bool)->bool {
        if (screen_source(source)&&!audio)||bytes.len()>8*1024*1024{return false;}
        {
            let media=self.media.lock().unwrap();
            if !media.accepts_frame(source,epoch,audio) {
                // The last listener can unsubscribe between the capture poll
                // and this PCM frame. Drop it without treating a live grant as
                // a failed encoder; a later subscriber can resume capture.
                return audio && !self.is_finished() && media.sources.get(&source).is_some_and(|s|
                    s.enabled && s.audio && s.grant==epoch && !s.demand.audio);
            }
        }
        // Backpressure the codec drain worker; never discard an encoded reference frame locally.
        self.frames.send(Frame{source,epoch,bytes,timestamp:now_unix_ms() as u32,audio,screen_stream:None,timing:None,geometry_revision:0}).await.is_ok()
    }
    pub async fn screen_frame(&self,metadata:ScreenFrameMetadata,bytes:Vec<u8>)->bool {
        if bytes.is_empty()||bytes.len()>8*1024*1024||metadata.geometry.validate().is_err()
            || metadata.geometry.source_revision!=0
            || !(bytes.starts_with(&[0,0,0,1])||bytes.starts_with(&[0,0,1]))
            || !self.media.lock().unwrap().accepts_screen_geometry(metadata.stream,metadata.grant,metadata.geometry.revision){return false;}
        let now=remote_core::timing::quanta_now_us();
        if metadata.capture_ts_us==0||metadata.capture_ts_us>metadata.admitted_at_us||metadata.admitted_at_us>now{return false;}
        let mut payload=Vec::with_capacity(protocol::viewport::FRAME_HEADER_LEN+bytes.len());
        metadata.geometry.write_header(&mut payload);payload.extend_from_slice(&bytes);
        let mut timing=protocol::FrameTimingCheckpoints::new(metadata.capture_ts_us);
        timing.encode_queue_ts_us=metadata.admitted_at_us.saturating_sub(metadata.capture_ts_us).min(u64::from(u32::MAX)) as u32;
        timing.encode_done_ts_us=now.saturating_sub(metadata.capture_ts_us).min(u64::from(u32::MAX)) as u32;
        let Some(source)=self.media.lock().unwrap().screen_frame_source(metadata.stream,metadata.grant) else {return false;};
        self.frames.send(Frame{source,epoch:metadata.grant,bytes:payload,
            timestamp:(metadata.capture_ts_us/1000) as u32,audio:false,screen_stream:Some(metadata.stream),timing:Some(timing),geometry_revision:metadata.geometry.revision}).await.is_ok()
    }
    pub async fn fail_screen_view(&self,stream:u64,grant:u64,reason:String)->Result<(),String> {
        if reason.len()>1024{return Err("Screen failure detail is too long".into());}
        if !self.media.lock().unwrap().accepts_screen_frame(stream,grant){return Ok(());}
        self.errors.send(CaptureFailure::ScreenView{stream,grant,reason}).await.map_err(|_|"Screen service stopped".into())
    }
    #[cfg(test)]
    async fn frame(&self,bytes:Vec<u8>,timestamp:i64,audio:bool)->bool {
        if audio{return self.frame_from(CaptureSource::MainDisplay,self.source_grant(CaptureSource::MainDisplay).unwrap_or(0),bytes,timestamp,true).await;}
        let mut accepted=false;
        for view in self.demand().views {
            let now=remote_core::timing::quanta_now_us();
            accepted|=self.screen_frame(ScreenFrameMetadata{stream:view.stream,grant:view.grant,geometry:view.geometry,
                capture_ts_us:now,admitted_at_us:now},bytes.clone()).await;
        }
        accepted
    }
    pub fn fail_source(&self,source:CaptureSource,epoch:u64,reason:String){
        let changed={let mut media=self.media.lock().unwrap();
            media.fail(source,epoch).then(||(media.demand(source).epoch,media.sources.values().any(|entry|entry.enabled)))};
        if let Some((epoch,available))=changed {
            self.capabilities.send_modify(|caps|caps.can_stream=available&&!self.is_finished());
            if self.errors.try_send(CaptureFailure::Source(source,epoch,reason)).is_err(){self.media_changed.send_modify(|value|*value=value.wrapping_add(1));}
        }
    }
    pub fn fail(&self,reason:String){self.fail_source(CaptureSource::MainDisplay,self.demand().epoch,reason);}
}

fn valid(s: &SubscriptionRequest) -> bool {
    protocol::validate_video_settings(s.width, s.height, s.fps, s.bitrate_kbps).is_ok()
        && s.width <= 4096
        && s.height <= 4096
        && u64::from(s.width) * u64::from(s.height) <= 8_388_608
}
async fn reply(sender: &UdpSender, addr: SocketAddr, command: SessionCommand) {
    let _ = sender
        .send_control(&ControlMessage::Session(Box::new(command)), addr)
        .await;
}
impl Peer {
    async fn shutdown(self)->Result<(),String> {
        let tasks=self.tasks.clone();
        drop(self); // Revoke the file channel and abort the owned transport tasks.
        tokio::time::timeout(Duration::from_secs(2),async {
            while tasks.iter().any(|task|!task.is_finished()) {
                tokio::task::yield_now().await;
            }
        }).await.map_err(|_|"Android connection workers did not stop in time".to_owned())
    }
    /// End media and input without dropping authenticated file workers. The
    /// delivery_owner tombstone still prevents resubscription after revocation.
    fn retire_media(&mut self,input:&Mutex<crate::publisher_input::PublisherInput>)->Vec<u32> {
        input.lock().unwrap().revoke_peer(self.addr);
        let ids=self.subscriptions.keys().copied().collect::<Vec<_>>();
        self.retired.extend(ids.iter().copied());self.subscriptions.clear();ids
    }
    fn new(addr: SocketAddr, id: u32, sender: &UdpSender, directory: PathBuf, allow_files: bool) -> Self {
        let (scheduled, worker) =
            ScheduledDataSender::spawn(sender.clone(), addr, Default::default());
        let mut tasks = vec![worker.abort_handle()];
        let (cancel, rx) = broadcast::channel(1);
        let files = if allow_files {
        let (files, inbound) = mpsc::channel(128);
        let (commands, command_rx) = mpsc::channel(16);
        let (events, mut event_rx) = mpsc::unbounded_channel();
        let file_task = tokio::spawn(run_file_transfer_runtime(
            scheduled.clone(),
            command_rx,
            inbound,
            events,
            rx,
            FileTransferRuntimeConfig {
                receive_dir: directory,
                ..Default::default()
            },
        ));
        let event_task = tokio::spawn(async move {
            let _commands = commands;
            while event_rx.recv().await.is_some() {}
        });
        tasks.extend([file_task.abort_handle(), event_task.abort_handle()]);
        Some(files)
        } else { None };
        Self {
            delivery_owner:None,
            addr,
            id,
            seen: Instant::now(),
            subscriptions:HashMap::new(),
            retired:Default::default(),
            scheduled,
            files,
            cancel,
            tasks,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn authenticated_close_confirms_cleanup_and_does_not_close_another_id() {
        let key=b"mobile-close-receipt".to_vec();
        let publisher=MobilePublisher::start_with_access("127.0.0.1:0".parse().unwrap(),
            std::env::temp_dir(),key.clone(),true,false,false).await.unwrap();
        let (sender,receiver)=UdpMultiplexer::bind("127.0.0.1:0").await.unwrap().split();
        remote_core::session_auth::authenticate_with_key(&sender,&receiver,publisher.address,&key).await.unwrap();
        command(&sender,publisher.address,SessionCommand::Open {connection_id:77,version:SESSION_VERSION}).await;
        assert!(matches!(reply(&receiver).await,SessionCommand::Opened {..}));
        command(&sender,publisher.address,SessionCommand::Close {connection_id:78}).await;
        command(&sender,publisher.address,SessionCommand::Subscribe(SubscriptionRequest {id:256,
            source:CaptureSource::MainDisplay,width:720,height:1280,fps:30,bitrate_kbps:4000,audio:false})).await;
        assert!(matches!(reply(&receiver).await,SessionCommand::Subscribed {id:256,..}));
        assert!(publisher.demand().active);
        command(&sender,publisher.address,SessionCommand::Close {connection_id:77}).await;
        assert!(matches!(reply(&receiver).await,SessionCommand::Closed {connection_id:77,..}));
        assert!(!publisher.demand().active);
        assert!(publisher.outgoing_status()["peers"].as_array().unwrap().is_empty());
    }
    #[tokio::test]
    async fn selected_mobile_source_reports_its_phone_or_custom_badge() {
        use protocol::appearance::{DeviceBadge,DeviceBadgePreset};
        let key=b"mobile-source-artwork".to_vec();
        let publisher=MobilePublisher::start_with_access("127.0.0.1:0".parse().unwrap(),std::env::temp_dir(),key.clone(),true,false,false).await.unwrap();
        let (sender,receiver)=UdpMultiplexer::bind("127.0.0.1:0").await.unwrap().split();
        remote_core::session_auth::authenticate_with_key(&sender,&receiver,publisher.address,&key).await.unwrap();
        command(&sender,publisher.address,SessionCommand::Open{connection_id:77,version:SESSION_VERSION}).await;
        assert!(matches!(reply(&receiver).await,SessionCommand::Opened{..}));
        command(&sender,publisher.address,SessionCommand::GetSourceAppearance{request_id:1,id:256,source:CaptureSource::MainDisplay}).await;
        assert!(matches!(reply(&receiver).await,SessionCommand::SourceAppearance{appearance,..} if appearance.is_err()));
        command(&sender,publisher.address,SessionCommand::Subscribe(SubscriptionRequest{id:256,source:CaptureSource::MainDisplay,width:720,height:1280,fps:30,bitrate_kbps:4000,audio:false})).await;
        assert!(matches!(reply(&receiver).await,SessionCommand::Subscribed{id:256,..}));
        command(&sender,publisher.address,SessionCommand::GetSourceAppearance{request_id:2,id:256,source:CaptureSource::MainDisplay}).await;
        assert!(matches!(reply(&receiver).await,SessionCommand::SourceAppearance{appearance,..} if matches!(appearance.as_ref(),Ok(protocol::appearance::SourceAppearance{application:None,device:DeviceBadge::Preset(DeviceBadgePreset::Phone)}))));
        publisher.set_device_badge(DeviceBadge::Preset(DeviceBadgePreset::MacStudio)).unwrap();
        let advertised=publisher.capabilities.subscribe();
        assert_eq!(advertised.borrow().device_badge,DeviceBadgePreset::MacStudio);
        let icon=remote_core::application_icons::encode_rgba(2,2,&[255;16]).unwrap();
        publisher.set_device_badge(DeviceBadge::Custom(icon.clone())).unwrap();
        assert_eq!(advertised.borrow().device_badge,DeviceBadgePreset::Phone,
            "custom stream artwork must keep advertising the Android device category");
        command(&sender,publisher.address,SessionCommand::GetSourceAppearance{request_id:3,id:256,source:CaptureSource::MainDisplay}).await;
        let SessionCommand::SourceAppearance{appearance,..}=reply(&receiver).await else {panic!("missing artwork")};
        let appearance=appearance.unwrap();
        assert_eq!(appearance.device,DeviceBadge::Custom(icon));
        command(&sender,publisher.address,SessionCommand::GetSourceAppearance{request_id:4,id:256,source:CaptureSource::Camera(9)}).await;
        assert!(matches!(reply(&receiver).await,SessionCommand::SourceAppearance{appearance,..} if appearance.is_err()));
        assert!(publisher.demand().active,"Optional artwork must not end media");
    }
    #[tokio::test]
    async fn selected_app_artwork_waits_for_native_read_and_expires_with_subscription() {
        let key=b"selected-application-artwork".to_vec();
        let publisher=MobilePublisher::start_with_access("127.0.0.1:0".parse().unwrap(),std::env::temp_dir(),key.clone(),true,false,false).await.unwrap();
        let source=CaptureSource::Window(42);
        publisher.set_applications(vec![CaptureSourceInfo{source,title:"Fixture".into(),application:"Fixture".into(),
            process_id:None,width:720,height:1280,supports_input:false,camera:None}]).unwrap();
        let (sender,receiver)=UdpMultiplexer::bind("127.0.0.1:0").await.unwrap().split();
        remote_core::session_auth::authenticate_with_key(&sender,&receiver,publisher.address,&key).await.unwrap();
        command(&sender,publisher.address,SessionCommand::Open{connection_id:77,version:SESSION_VERSION}).await;
        while !matches!(reply(&receiver).await,SessionCommand::Opened{..}) {}
        command(&sender,publisher.address,SessionCommand::Subscribe(SubscriptionRequest{id:256,source,width:720,height:1280,fps:30,bitrate_kbps:4000,audio:false})).await;
        while !matches!(reply(&receiver).await,SessionCommand::Subscribed{id:256,..}) {}
        let grant=publisher.media.lock().unwrap().sources[&source].grant;
        command(&sender,publisher.address,SessionCommand::GetSourceAppearance{request_id:8,id:256,source}).await;
        command(&sender,publisher.address,SessionCommand::GetSourceAppearance{request_id:8,id:256,source}).await;
        // This catalog response orders the preceding artwork request. No empty
        // placeholder may win the race before the native icon is available.
        command(&sender,publisher.address,SessionCommand::ListSources{request_id:9,known_icons:vec![]}).await;
        loop {match reply(&receiver).await {
            SessionCommand::Sources{request_id:9,..}=>break,
            SessionCommand::SourceAppearance{request_id:8,..}=>panic!("Artwork completed before native read"),
            _=>{}
        }}
        let icon=remote_core::application_icons::encode_artwork(256,256,&vec![255;256*256*4]).unwrap();
        assert!(publisher.set_application_artwork(source,grant+1,1,Some(icon.clone())).is_err());
        let mut corrupt=icon.clone();corrupt.id[0]^=1;
        assert!(publisher.set_application_artwork(source,grant,1,Some(corrupt)).is_err());
        publisher.set_application_artwork(source,grant,1,Some(icon.clone())).unwrap();
        loop {if let SessionCommand::SourceAppearance{request_id:8,appearance,..}=reply(&receiver).await {
            assert_eq!(appearance.unwrap().application,Some(icon.clone()));break;
        }}
        command(&sender,publisher.address,SessionCommand::Unsubscribe{id:256}).await;
        while !matches!(reply(&receiver).await,SessionCommand::Unsubscribed{id:256}) {}
        assert!(publisher.media.lock().unwrap().sources[&source].artwork.is_none());
        assert!(publisher.set_application_artwork(source,grant,1,Some(icon.clone())).is_err(),"late worker cannot retain an unselected icon");
        command(&sender,publisher.address,SessionCommand::GetSourceAppearance{request_id:10,id:256,source}).await;
        assert!(matches!(reply(&receiver).await,SessionCommand::SourceAppearance{request_id:10,appearance,..} if appearance.is_err()));
        command(&sender,publisher.address,SessionCommand::Subscribe(SubscriptionRequest{id:257,source,width:720,height:1280,fps:30,bitrate_kbps:4000,audio:false})).await;
        while !matches!(reply(&receiver).await,SessionCommand::Subscribed{id:257,..}) {}
        assert!(publisher.set_application_artwork(source,grant,1,Some(icon.clone())).is_err(),"rapid reconnect must reject the prior artwork completion");
        publisher.set_application_artwork(source,grant,2,Some(icon.clone())).unwrap();
        command(&sender,publisher.address,SessionCommand::GetSourceAppearance{request_id:11,id:257,source}).await;
        let SessionCommand::SourceAppearance{request_id:11,appearance,..}=reply(&receiver).await else {panic!("missing reconnected artwork")};
        assert_eq!(appearance.unwrap().application,Some(icon));
    }
    #[tokio::test]
    async fn idle_app_catalog_does_not_enter_the_native_demand_poll() {
        let publisher=MobilePublisher::start_with_permissions("127.0.0.1:0".parse().unwrap(),
            std::env::temp_dir(),b"app-demand-test".to_vec(),true,false).await.unwrap();
        publisher.set_applications((1..=256).map(|id|CaptureSourceInfo {
            source:CaptureSource::Window(id),title:format!("App {id}"),application:format!("app.{id}"),
            process_id:None,width:720,height:1280,supports_input:false,camera:None,
        }).collect()).unwrap();
        assert_eq!(publisher.application_demands(),serde_json::json!([]));
        let request=SubscriptionRequest {id:256,source:CaptureSource::Window(7),width:720,height:1280,
            fps:30,bitrate_kbps:4000,audio:false};
        {
            let mut media=publisher.media.lock().unwrap();let grant=media.sources[&request.source].grant;
            media.recompute([crate::publisher_media::ActiveRequest {stream:1,grant,request:&request,video:false,audio:false}]);
        }
        let paused=publisher.application_demands();
        assert_eq!(paused.as_array().unwrap().len(),1);
        assert_eq!(paused[0]["source"],serde_json::json!({"Window":7}));
        assert_eq!(paused[0]["demand"]["subscriptions"],1);
        assert_eq!(paused[0]["demand"]["active"],false);
        publisher.media.lock().unwrap().recompute([]);
        assert_eq!(publisher.application_demands(),serde_json::json!([]));
    }
    #[tokio::test]
    async fn sending_requires_capture_and_stop_cannot_cancel_a_replacement_intent() {
        let publisher = MobilePublisher::start_with_access("127.0.0.1:0".parse().unwrap(),
            std::env::temp_dir(), b"send-intent-test".to_vec(), false, false, false).await.unwrap();
        publisher.set_device_identity("phone".into());
        assert!(publisher.send_screen("desktop".into(), "127.0.0.1:9".parse().unwrap()).await.unwrap_err().contains("Enable the selected"));
        assert!(!publisher.delivery.upgrade_busy());
        let (stop, mut cancelled) = watch::channel(false);
        let (finished, done) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move { let _=cancelled.changed().await; let _=finished.send(()); });
        *publisher.outgoing.lock().unwrap()=Some(Outgoing {id:2,target:"desktop".into(),ticket:[42;32],screen_grant:None,quality_alive:Arc::new(AtomicBool::new(true)),stop,
            status:Arc::new(Mutex::new("Sending".into())),confirmation_timeout:Arc::new(Mutex::new(None)),task});
        assert!(publisher.stop_sending_id(1).is_err());
        assert_eq!(publisher.outgoing_status()["id"], 2);
        assert!(publisher.outgoing_status()["active"].as_bool().unwrap());
        publisher.stop_sending_id(2).unwrap();
        tokio::time::timeout(Duration::from_secs(1), done).await.unwrap().unwrap();
    }
    #[tokio::test]
    async fn outgoing_quality_checks_intent_and_capture_grant() {
        let publisher=MobilePublisher::start_with_access("127.0.0.1:0".parse().unwrap(),
            std::env::temp_dir(),b"quality-binding".to_vec(),true,false,false).await.unwrap();
        let grant=publisher.media.lock().unwrap().sources[&CaptureSource::MainDisplay].grant;
        let (stop,mut cancelled)=watch::channel(false);
        let task=tokio::spawn(async move {let _=cancelled.changed().await;});
        let alive=Arc::new(AtomicBool::new(true));
        *publisher.outgoing.lock().unwrap()=Some(Outgoing{id:7,target:"desktop".into(),ticket:[5;32],screen_grant:Some(grant),
            quality_alive:alive.clone(),stop,status:Arc::new(Mutex::new(String::new())),confirmation_timeout:Arc::new(Mutex::new(None)),task});
        let mut quality=crate::outgoing_quality::QualityRequest{id:6,short_edge:1080,fps:30,bitrate_kbps:3000};
        assert!(publisher.set_outgoing_quality(quality).is_err());
        publisher.media.lock().unwrap().set_quality(crate::outgoing_quality::DeliveryQuality{ticket:[5;32],source:CaptureSource::MainDisplay,grant,settings:quality,revision:0});
        quality.id=7;assert!(publisher.set_outgoing_quality(quality).is_ok());
        alive.store(false,Ordering::Release);
        assert!(publisher.set_outgoing_quality(quality).is_err());
        publisher.stop_sending_id(7).unwrap();
    }
    #[tokio::test(flavor="current_thread")]
    async fn initial_quality_binds_before_offer_and_old_cleanup_cannot_clear_successor() {
        let publisher=MobilePublisher::start_with_access("127.0.0.1:0".parse().unwrap(),
            std::env::temp_dir(),b"quality-start".to_vec(),true,false,false).await.unwrap();
        publisher.set_device_identity("phone".into());
        let initial=crate::outgoing_quality::InitialQuality{short_edge:720,fps:30,bitrate_kbps:3000};
        let endpoint="127.0.0.1:9".parse().unwrap();
        let first=publisher.send_screen_quality("desktop".into(),endpoint,initial).await.unwrap();
        assert_eq!(publisher.media.lock().unwrap().quality_id_for_test(),Some(first));
        publisher.stop_sending_id(first).unwrap();
        let second=publisher.send_screen_quality("desktop".into(),endpoint,initial).await.unwrap();
        assert_ne!(first,second);
        assert!(publisher.stop_sending_id(first).is_err());
        tokio::task::yield_now().await;
        assert_eq!(publisher.media.lock().unwrap().quality_id_for_test(),Some(second));
        publisher.stop_sending_id(second).unwrap();
    }
    #[tokio::test]
    async fn foreground_delivery_listener_does_not_enable_files_or_capture() {
        let key = b"mobile-incoming-only".to_vec();
        let publisher = MobilePublisher::start_with_access(
            "127.0.0.1:0".parse().unwrap(), std::env::temp_dir(), key.clone(), false, false, false,
        ).await.unwrap();
        assert!(!publisher.capabilities().borrow().file_transfer);
        assert!(!publisher.capabilities().borrow().can_stream);
        let client = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let (sender, receiver) = client.split();
        remote_core::session_auth::authenticate_with_key(&sender, &receiver, publisher.address, &key).await.unwrap();
        command(&sender, publisher.address, SessionCommand::Open { connection_id: 31, version: SESSION_VERSION }).await;
        assert!(matches!(reply(&receiver).await, SessionCommand::Opened { files: false, .. }));
        command(&sender, publisher.address, SessionCommand::Subscribe(SubscriptionRequest {
            id: 256, source: CaptureSource::MainDisplay, width: 720, height: 1280, fps: 30, bitrate_kbps: 4000, audio: false,
        })).await;
        assert!(matches!(reply(&receiver).await, SessionCommand::Error { request_id: 256, .. }));
        assert!(!publisher.demand().active);
        publisher.set_capture_available(true, false);
        assert!(matches!(reply(&receiver).await, SessionCommand::Sources { .. }));
        assert!(publisher.capabilities().borrow().can_stream);
        assert!(!publisher.capabilities().borrow().file_transfer);
        publisher.set_file_reception(true);
        assert!(publisher.capabilities().borrow().file_transfer);
        // An already-open control connection does not silently acquire file access.
        command(&sender, publisher.address, SessionCommand::Open { connection_id: 31, version: SESSION_VERSION }).await;
        assert!(matches!(reply(&receiver).await, SessionCommand::Opened { files: false, .. }));
        let client = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let (sender, receiver) = client.split();
        remote_core::session_auth::authenticate_with_key(&sender, &receiver, publisher.address, &key).await.unwrap();
        command(&sender, publisher.address, SessionCommand::Open { connection_id: 32, version: SESSION_VERSION }).await;
        assert!(matches!(reply(&receiver).await, SessionCommand::Opened { files: true, .. }));
    }
    #[tokio::test]
    async fn retiring_delivery_media_keeps_file_workers_and_the_claim_tombstone() {
        let mux = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let (sender, _) = mux.split();
        let addr = "127.0.0.1:39271".parse().unwrap();
        let mut peer = Peer::new(addr, 9, &sender, std::env::temp_dir(), true);
        peer.subscriptions.insert(256,MobileSubscription::new(SubscriptionRequest {id:256,source:CaptureSource::MainDisplay,
            width:720,height:1280,fps:30,bitrate_kbps:4000,audio:false},1,false));
        let owner = DeliveryPeer { addr, connection: 9, nonce: [1;16], scope: Some([2;32]) };
        peer.delivery_owner = Some((owner,[3;32]));
        let files = peer.files.clone().unwrap();
        let input = Mutex::new(crate::publisher_input::PublisherInput::default());
        assert_eq!(peer.retire_media(&input),vec![256]);
        assert!(peer.retire_media(&input).is_empty());
        assert!(peer.subscriptions.is_empty());
        assert!(peer.retired.contains(&256));
        assert_eq!(peer.delivery_owner, Some((owner,[3;32])));
        assert!(files.same_channel(peer.files.as_ref().unwrap()) && !files.is_closed());
        assert!(peer.tasks.iter().all(|task|!task.is_finished()));
    }
    #[tokio::test]
    async fn directed_offer_uses_the_authenticated_peer_and_exact_source_set() {
        let key = b"mobile-directed-offer-test".to_vec();
        let publisher = MobilePublisher::start_with_key(
            "127.0.0.1:0".parse().unwrap(),
            std::env::temp_dir().join(format!("weavelo-directed-{}", std::process::id())),
            key.clone(), false,
        ).await.unwrap();
        publisher.set_device_identity("phone".into());
        let client = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let client_addr = client.local_addr().unwrap();
        let (sender, receiver) = client.split();
        remote_core::session_auth::authenticate_with_key(&sender, &receiver, publisher.address, &key).await.unwrap();
        command(&sender, publisher.address, SessionCommand::Open { connection_id: 19, version: SESSION_VERSION }).await;
        assert!(matches!(reply(&receiver).await, SessionCommand::Opened { connection_id: 19, .. }));
        let sources = vec![CaptureSource::Window(41), CaptureSource::Window(42)];
        let offer = SessionCommand::DeliveryOffer {
            request_id: 7, source_device_id: "desktop".into(), target_device_id: "phone".into(),
            ticket: [23; 32], revision: 1, sources: sources.clone(),
        };
        command(&sender, publisher.address, offer.clone()).await;
        assert!(matches!(reply(&receiver).await, SessionCommand::DeliveryReceipt { state: DeliveryState::Pending, .. }));
        let incoming = publisher.incoming.lock().unwrap().try_recv().unwrap();
        assert_eq!(incoming.peer.addr, client_addr);
        assert_eq!(incoming.peer.connection, 19);
        assert_eq!(incoming.peer.scope, Some(remote_core::shared_files::share_scope_for_key(&key)));
        assert_eq!(incoming.sources, sources);
        incoming.finish(DeliveryState::Receiving, "");
        command(&sender, publisher.address, offer).await;
        assert!(matches!(reply(&receiver).await, SessionCommand::DeliveryReceipt { state: DeliveryState::Receiving, .. }));
        assert!(publisher.incoming.lock().unwrap().try_recv().is_err()); // renewal is not a second launch
        command(&sender, publisher.address, SessionCommand::DeliveryOffer {
            request_id: 7, source_device_id: "desktop".into(), target_device_id: "phone".into(),
            ticket: [23; 32], revision: 2, sources: vec![CaptureSource::MainDisplay],
        }).await;
        assert!(matches!(reply(&receiver).await, SessionCommand::DeliveryReceipt { state: DeliveryState::Rejected, .. }));
        command(&sender, publisher.address, SessionCommand::DeliveryCancel { request_id: 7 }).await;
        assert!(matches!(reply(&receiver).await, SessionCommand::DeliveryReceipt { state: DeliveryState::Cancelled, .. }));
        assert!(!incoming.alive());
    }
    async fn command(sender: &UdpSender, target: SocketAddr, command: SessionCommand) {
        sender
            .send_control(&ControlMessage::Session(Box::new(command)), target)
            .await
            .unwrap();
    }
    // Each receive future owns a 64 KiB datagram buffer. Keep repeated test
    // exchanges off the surrounding unoptimized test coroutine's stack.
    fn reply(receiver: &remote_core::net::UdpReceiver) -> impl std::future::Future<Output = SessionCommand> + '_ {
        Box::pin(async move { tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let MultiplexedPacket::Control(ControlMessage::Session(message), _) =
                    receiver.recv().await.unwrap()
                {
                    return *message;
                }
            }
        })
        .await
        .unwrap() })
    }
    #[tokio::test]
    async fn publisher_requires_pairing_and_preserves_pause_revision_and_closed_ids() {
        let key = b"projection-test-key".to_vec();
        let directory =
            std::env::temp_dir().join(format!("weavelo-publisher-{}", std::process::id()));
        let publisher = MobilePublisher::start_with_key(
            "127.0.0.1:0".parse().unwrap(),
            directory,
            key.clone(),
            false,
        )
        .await
        .unwrap();
        let client = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let (sender, receiver) = client.split();
        let target = publisher.address;
        command(
            &sender,
            target,
            SessionCommand::Open {
                connection_id: 9,
                version: SESSION_VERSION,
            },
        )
        .await;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), receiver.recv())
                .await
                .is_err()
        );
        remote_core::session_auth::authenticate_with_key(&sender,&receiver,target,&key).await.unwrap();
        command(
            &sender,
            target,
            SessionCommand::Open {
                connection_id: 9,
                version: SESSION_VERSION,
            },
        )
        .await;
        assert!(matches!(
            reply(&receiver).await,
            SessionCommand::Opened {
                max_subscriptions: 8,
                ..
            }
        ));
        let request = SubscriptionRequest {
            id: 256,
            source: CaptureSource::MainDisplay,
            width: 720,
            height: 1280,
            fps: 37,
            bitrate_kbps: 3500,
            audio: true,
        };
        command(&sender, target, SessionCommand::Subscribe(request.clone())).await;
        assert!(matches!(
            reply(&receiver).await,
            SessionCommand::Subscribed {
                audio_owner: None,
                supports_input: false,
                ..
            }
        ));
        assert!(publisher.demand().active);
        assert!(!publisher.demand().audio);
        publisher.frame(vec![0, 0, 0, 1, 0x26, 0x01, 1, 2, 3], 0, false).await;
        let media = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let MultiplexedPacket::DataWithTiming(envelope, _, _) = media else {
            panic!("expected encoded frame")
        };
        assert_eq!(envelope.header.stream_id, 256);
        assert_eq!(protocol::viewport::split_frame(&envelope.payload).unwrap().1, [0, 0, 0, 1, 0x26, 0x01, 1, 2, 3]);
        command(
            &sender,
            target,
            SessionCommand::SetActivity {
                id: 256,
                revision: 2,
                video: false,
                audio: false,
            },
        )
        .await;
        assert!(matches!(
            reply(&receiver).await,
            SessionCommand::Activity {
                revision: 2,
                video: false,
                ..
            }
        ));
        command(
            &sender,
            target,
            SessionCommand::SetActivity {
                id: 256,
                revision: 1,
                video: true,
                audio: true,
            },
        )
        .await;
        assert!(matches!(
            reply(&receiver).await,
            SessionCommand::Activity {
                revision: 2,
                video: false,
                ..
            }
        ));
        assert!(!publisher.demand().active);
        command(&sender, target, SessionCommand::Unsubscribe { id: 256 }).await;
        assert!(matches!(
            reply(&receiver).await,
            SessionCommand::Unsubscribed { .. }
        ));
        command(&sender, target, SessionCommand::Subscribe(request)).await;
        assert!(matches!(
            reply(&receiver).await,
            SessionCommand::Error { .. }
        ));
        assert!(!publisher.demand().active);
    }

    #[tokio::test]
    async fn audio_demand_pause_is_benign_but_revoked_grants_fail() {
        let directory = ReceiverTestDir::new();
        let publisher = MobilePublisher::start_with_permissions("127.0.0.1:0".parse().unwrap(),
            directory.0.clone(), b"audio-pause-fixture".to_vec(), false, false).await.unwrap();
        publisher.set_capture_available(true,true);
        let source=CaptureSource::MainDisplay;
        let grant=publisher.source_grant(source).unwrap();
        let request=SubscriptionRequest{id:256,source,width:720,height:1280,fps:30,bitrate_kbps:4000,audio:true};
        let demand=|on| { publisher.media.lock().unwrap().recompute(if on {
            vec![crate::publisher_media::ActiveRequest{stream:1,grant,request:&request,video:true,audio:true}]
        } else {vec![]}); };
        demand(true);
        assert!(publisher.frame_from(source,grant,vec![1],0,true).await);
        demand(false);
        assert!(publisher.frame_from(source,grant,vec![2],0,true).await);
        assert!(!publisher.frame_from(source,grant+1,vec![3],0,true).await);
        demand(true);
        assert!(publisher.frame_from(source,grant,vec![4],0,true).await);
        publisher.set_capture_available(false,false);
        assert!(!publisher.frame_from(source,grant,vec![5],0,true).await);
    }

    #[tokio::test]
    async fn capture_revocation_and_worker_exit_retract_live_capabilities() {
        let directory = ReceiverTestDir::new();
        let publisher = MobilePublisher::start_with_permissions(
            "127.0.0.1:0".parse().unwrap(), directory.0.clone(),
            b"capability-fixture-key".to_vec(), false, false,
        ).await.unwrap();
        let capabilities = publisher.capabilities();
        assert!(capabilities.borrow().file_transfer);
        assert!(!capabilities.borrow().can_stream);
        publisher.set_capture_available(true, false);
        assert!(capabilities.borrow().can_stream);
        publisher.set_capture_available(false, false);
        assert!(!capabilities.borrow().can_stream);
        assert!(capabilities.borrow().file_transfer);
        assert!(!publisher.is_finished());
        publisher.set_capture_available(true, false);
        publisher.fail("fixture capture failure".into());
        assert!(!capabilities.borrow().can_stream);
        assert!(capabilities.borrow().file_transfer);
        assert!(!publisher.is_finished());
        publisher.task.abort();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !publisher.is_finished() { tokio::task::yield_now().await; }
        }).await.unwrap();
        assert!(!capabilities.borrow().file_transfer);
        publisher.set_capture_available(true, true);
        assert!(!capabilities.borrow().can_stream);
        assert!(!capabilities.borrow().file_transfer);
    }

    struct ReceiverTestDir(PathBuf);
    impl ReceiverTestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "weavelo-receiver-{:032x}",
                u128::from_le_bytes(random_bytes_16())
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for ReceiverTestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn receiver_client<'a>(
        publisher: &'a MobilePublisher,
        key: &'a [u8],
    ) -> impl std::future::Future<Output = (UdpSender, remote_core::net::UdpReceiver)> + 'a {
        Box::pin(async move {
        let mux = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let (sender, receiver) = mux.split();
        remote_core::session_auth::authenticate_with_key(&sender,&receiver,publisher.address,key).await.unwrap();
        command(
            &sender,
            publisher.address,
            SessionCommand::Open {
                connection_id: 17,
                version: SESSION_VERSION,
            },
        )
        .await;
        assert!(matches!(
            reply(&receiver).await,
            SessionCommand::Opened {
                connection_id: 17,
                files: true,
                ..
            }
        ));
        (sender, receiver)
        })
    }
    fn receiver_subscription(id: u32) -> SubscriptionRequest {
        SubscriptionRequest {
            id,
            source: CaptureSource::MainDisplay,
            width: 720,
            height: 1280,
            fps: 30,
            bitrate_kbps: 3500,
            audio: true,
        }
    }
    struct FileTestSender {
        commands: mpsc::Sender<FileTransferCommand>,
        incoming: mpsc::Sender<protocol::DataEnvelope>,
        events: mpsc::UnboundedReceiver<FileTransferEvent>,
        target: SocketAddr,
        cancel: broadcast::Sender<()>,
        tasks: Vec<tokio::task::AbortHandle>,
    }
    impl Drop for FileTestSender {
        fn drop(&mut self) {
            let _ = self.cancel.send(());
            for task in &self.tasks {
                task.abort();
            }
        }
    }
    impl FileTestSender {
        fn new(sender: &UdpSender, target: SocketAddr) -> Self {
            let (scheduled, worker) =
                ScheduledDataSender::spawn(sender.clone(), target, Default::default());
            let (commands, command_rx) = mpsc::channel(8);
            let (incoming, inbound) = mpsc::channel(128);
            let (events, event_rx) = mpsc::unbounded_channel();
            let (cancel, cancel_rx) = broadcast::channel(1);
            let task = tokio::spawn(run_file_transfer_runtime(
                scheduled,
                command_rx,
                inbound,
                events,
                cancel_rx,
                FileTransferRuntimeConfig::default(),
            ));
            Self {
                commands,
                incoming,
                events: event_rx,
                target,
                cancel,
                tasks: vec![worker.abort_handle(), task.abort_handle()],
            }
        }
        async fn deliver(&mut self, receiver: &remote_core::net::UdpReceiver, source: PathBuf) {
            self.commands
                .send(FileTransferCommand::SendFile {
                    path: source,
                    mime_type: None,
                })
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    tokio::select! {
                        event = self.events.recv() => match event {
                            Some(FileTransferEvent::OutgoingCompleted { .. }) => break,
                            Some(FileTransferEvent::Error {message,..}) => panic!("file transfer failed: {message}"),
                            None => panic!("file worker closed before receiver confirmation"),
                            _ => {}
                        },
                        packet = receiver.recv() => match packet.unwrap() {
                            MultiplexedPacket::Data(e,a) | MultiplexedPacket::DataWithTiming(e,_,a) if a == self.target => { self.incoming.send(e).await.unwrap(); },
                            _ => {}
                        }
                    }
                }
            }).await.expect("file receipt deadline");
        }
    }

    #[tokio::test]
    async fn android_input_is_bound_to_its_viewer_and_requires_native_completion() {
        use protocol::realtime_input::{InputPayload,RealtimeInput};
        use crate::publisher_input::Dispatch;
        let directory=ReceiverTestDir::new(); let key=b"android-input-fixture".to_vec();
        let publisher=MobilePublisher::start_with_permissions("127.0.0.1:0".parse().unwrap(),directory.0.clone(),key.clone(),true,false).await.unwrap();
        publisher.set_input_service_available(true); publisher.set_input_capture_allowed(true);
        let (sender,receiver)=receiver_client(&publisher,&key).await;
        let (other,_other_receiver)=receiver_client(&publisher,&key).await;
        command(&sender,publisher.address,SessionCommand::Open{connection_id:17,version:SESSION_VERSION-1}).await;
        assert!(matches!(reply(&receiver).await,SessionCommand::Error{request_id:17,..}));
        command(&sender,publisher.address,SessionCommand::Subscribe(receiver_subscription(256))).await;
        loop { if matches!(reply(&receiver).await,SessionCommand::Subscribed {supports_input:true,..}) {break;} }
        let old_view=publisher.demand().views.remove(0);
        let presented=old_view.geometry.revision;
        command(&sender,publisher.address,SessionCommand::RequestInputBinding {connection_id:17,id:256,source_revision:0,presented_geometry_revision:presented,request:1}).await;
        let binding=loop { if let SessionCommand::InputBinding {binding,..}=reply(&receiver).await {break binding;} };
        let packet=RealtimeInput {binding,sequence:1,created_host_ms:crate::publisher_input::now_ms(),payload:InputPayload::Event(protocol::InputEvent::MouseMoveAbsolute {x:123,y:456})};
        // A file peer with the same connection ID cannot control this viewer.
        command(&other,publisher.address,SessionCommand::RealtimeInput(packet.clone())).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        while let Some(job)=publisher.input.lock().unwrap().poll() { assert!(matches!(job,Dispatch::Reset {..})); }
        command(&sender,publisher.address,SessionCommand::RealtimeInput(packet.clone())).await;
        let job=tokio::time::timeout(Duration::from_secs(1),async {
            loop {
                if let Some(Dispatch::Events {generation,token,..})=publisher.input.lock().unwrap().poll() {break (generation,token);}
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }).await.unwrap();
        assert!(publisher.input.lock().unwrap().take_handled().is_empty());
        assert!(publisher.input_execution_valid(job.0,job.1));
        publisher.complete_input(job.0,job.1,true);
        loop { if let SessionCommand::InputHandled {binding:actual,sequence}=reply(&receiver).await {assert_eq!(actual,binding);assert_eq!(sequence,1);break;} }
        command(&sender,publisher.address,SessionCommand::RealtimeInput(packet)).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(publisher.input.lock().unwrap().poll().is_none());
        publisher.configure_geometry(CaptureSource::MainDisplay,publisher.demand().epoch,1920,1080,0).unwrap();
        assert!(!publisher.input_execution_valid(job.0,job.1));
        let now=remote_core::timing::quanta_now_us();
        let stale=ScreenFrameMetadata{stream:old_view.stream,grant:old_view.grant,geometry:old_view.geometry,capture_ts_us:now,admitted_at_us:now};
        assert!(!publisher.screen_frame(stale,vec![0,0,0,1,0x26,1,43]).await,"old encoded metadata must not re-enter after rotation");
        let next=publisher.demand().views.remove(0);
        assert!(next.geometry.revision>presented);
        command(&sender,publisher.address,SessionCommand::RequestInputBinding {connection_id:17,id:256,source_revision:0,presented_geometry_revision:presented,request:2}).await;
        assert!(tokio::time::timeout(Duration::from_millis(150),async {
            loop {if matches!(reply(&receiver).await,SessionCommand::InputBinding{request:2,..}) {break;}}
        }).await.is_err(),"old frame must not acquire a replacement lease");
        command(&sender,publisher.address,SessionCommand::RequestInputBinding {connection_id:17,id:256,source_revision:0,presented_geometry_revision:next.geometry.revision,request:3}).await;
        tokio::time::timeout(Duration::from_secs(2),async {
            loop {if matches!(reply(&receiver).await,SessionCommand::InputBinding{request:3,..}) {break;}}
        }).await.expect("newly rendered frame can renew control");
        publisher.set_capture_available(false,false);
        assert!(!publisher.input.lock().unwrap().ready());
    }

    #[tokio::test]
    async fn receiver_requires_authentication_and_never_exposes_unauthorized_capture() {
        let directory = ReceiverTestDir::new();
        let key = b"file-receiver-test-key".to_vec();
        let publisher = MobilePublisher::start_with_permissions(
            "127.0.0.1:0".parse().unwrap(),
            directory.0.clone(),
            key.clone(),
            false,
            false,
        )
        .await
        .unwrap();
        let bad_mux = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let (bad_sender, bad_receiver) = bad_mux.split();
        command(
            &bad_sender,
            publisher.address,
            SessionCommand::Open {
                connection_id: 7,
                version: SESSION_VERSION,
            },
        )
        .await;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), bad_receiver.recv())
                .await
                .is_err()
        );
        let (sender, receiver) = receiver_client(&publisher, &key).await;
        command(
            &sender,
            publisher.address,
            SessionCommand::ListSources { request_id: 3, known_icons: Vec::new() },
        )
        .await;
        assert!(
            matches!(reply(&receiver).await, SessionCommand::Sources {sources,..} if sources.is_empty())
        );
        command(
            &sender,
            publisher.address,
            SessionCommand::Subscribe(receiver_subscription(256)),
        )
        .await;
        assert!(matches!(
            reply(&receiver).await,
            SessionCommand::Error {
                request_id: 256,
                ..
            }
        ));
        publisher.frame(vec![1, 2, 3], 0, false).await;
        publisher.frame(vec![1, 2, 3], 0, true).await;
        assert!(!publisher.demand().active);
        assert!(!publisher.demand().audio);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), receiver.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn concurrent_file_peers_keep_transfers_and_media_ownership_isolated() {
        let directory = ReceiverTestDir::new();
        let receive = directory.0.join("received");
        let key = b"concurrent-receiver-fixture".to_vec();
        let publisher = MobilePublisher::start_with_permissions(
            "127.0.0.1:0".parse().unwrap(), receive.clone(), key.clone(), true, false,
        ).await.unwrap();
        let address = publisher.address;
        let mut clients = Vec::new();
        // Identical connection, subscription and transfer IDs on different
        // authenticated transports must never alias another computer's state.
        for _ in 0..4 { clients.push(receiver_client(&publisher,&key).await); }
        command(&clients[0].0,address,SessionCommand::Subscribe(receiver_subscription(256))).await;
        assert!(matches!(reply(&clients[0].1).await,SessionCommand::Subscribed {id:256,..}));
        command(&clients[1].0,address,SessionCommand::Subscribe(receiver_subscription(256))).await;
        assert!(matches!(reply(&clients[1].1).await,SessionCommand::Subscribed {id:256,..}));
        command(&clients[1].0,address,SessionCommand::Unsubscribe {id:256}).await;
        assert!(matches!(reply(&clients[1].1).await,SessionCommand::Unsubscribed {id:256}));
        assert!(publisher.demand().active);
        let payloads: Vec<Vec<u8>> = (0..4).map(|n| (0..262_161).map(|i| ((i+n*17)%251) as u8).collect()).collect();
        let mut transfers = tokio::task::JoinSet::new();
        for (i,(sender,receiver)) in clients.into_iter().enumerate() {
            let path = directory.0.join(format!("computer-{i}.bin"));
            std::fs::write(&path,&payloads[i]).unwrap();
            // Each client owns its own receive loop, as on the four devices.
            transfers.spawn(async move {
                let mut transfer = FileTestSender::new(&sender,address);
                transfer.deliver(&receiver,path).await;
                (i,sender,receiver,transfer)
            });
        }
        let mut completed = Vec::new();
        while let Some(result) = transfers.join_next().await { completed.push(result.unwrap()); }
        completed.sort_by_key(|client| client.0);
        for (i, bytes) in payloads.iter().enumerate() {
            assert_eq!(&std::fs::read(receive.join(format!("computer-{i}.bin"))).unwrap(),bytes);
        }
        command(&completed[1].1,address,SessionCommand::Close {connection_id:17}).await;
        command(&completed[0].1,address,SessionCommand::SetActivity {id:256,revision:1,video:true,audio:false}).await;
        assert!(matches!(reply(&completed[0].2).await,SessionCommand::Activity {video:true,..}));
        assert!(publisher.demand().active,"closing a file peer stopped another viewer");
        publisher.frame(vec![0,0,0,1,0x26,1,9],0,false).await;
        let packet = tokio::time::timeout(Duration::from_secs(2),completed[0].2.recv()).await.unwrap().unwrap();
        assert!(matches!(packet,MultiplexedPacket::DataWithTiming(e,_,_) if e.header.stream_id==256 && protocol::viewport::split_frame(&e.payload).unwrap().1==[0,0,0,1,0x26,1,9]));
        publisher.set_capture_available(false,false);
        assert!(matches!(reply(&completed[0].2).await,SessionCommand::Unsubscribed {id:256}));
        for i in [0,2,3] {
            assert!(matches!(reply(&completed[i].2).await,SessionCommand::Sources {sources,..} if sources.is_empty()));
        }
        let after = directory.0.join("after-revocation.bin");
        std::fs::write(&after,&payloads[2]).unwrap();
        let (_,_,receiver,transfer) = &mut completed[2];
        transfer.deliver(receiver,after).await;
        assert_eq!(std::fs::read(receive.join("after-revocation.bin")).unwrap(),payloads[2]);
    }

    #[tokio::test]
    async fn authenticated_peer_capacity_is_bounded_without_displacing_file_clients() {
        let directory = ReceiverTestDir::new();
        let key = b"receiver-capacity-fixture".to_vec();
        let publisher = MobilePublisher::start_with_permissions(
            "127.0.0.1:0".parse().unwrap(),directory.0.clone(),key.clone(),false,false,
        ).await.unwrap();
        let mut clients = Vec::new();
        for _ in 0..MAX_PUBLISHER_PEERS { clients.push(receiver_client(&publisher,&key).await); }
        let extra = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let (sender,receiver) = extra.split();
        let error = remote_core::session_auth::authenticate_with_key(&sender,&receiver,publisher.address,&key).await;
        assert_eq!(error.unwrap_err(), "session authentication rejected: Busy");
        for (sender,receiver) in &clients {
            command(sender,publisher.address,SessionCommand::ListSources { request_id:3, known_icons: Vec::new() }).await;
            assert!(matches!(reply(receiver).await,SessionCommand::Sources {request_id:3,sources, ..} if sources.is_empty()));
        }
    }

    #[tokio::test]
    async fn receiver_files_survive_projection_start_and_stop() {
        let directory = ReceiverTestDir::new();
        let receive = directory.0.join("received");
        let key = b"receiver-lifecycle-test".to_vec();
        let publisher = MobilePublisher::start_with_permissions(
            "127.0.0.1:0".parse().unwrap(),
            receive.clone(),
            key.clone(),
            false,
            false,
        )
        .await
        .unwrap();
        let address = publisher.address;
        let (sender, receiver) = receiver_client(&publisher, &key).await;
        let contents: Vec<u8> = (0..131_072).map(|i| (i % 251) as u8).collect();
        let first = directory.0.join("before-sharing.bin");
        std::fs::write(&first, &contents).unwrap();
        let mut file_sender = FileTestSender::new(&sender, address);
        file_sender.deliver(&receiver, first).await;
        assert_eq!(
            std::fs::read(receive.join("before-sharing.bin")).unwrap(),
            contents
        );

        publisher.set_capture_available(true, false);
        assert!(
            matches!(reply(&receiver).await, SessionCommand::Sources {sources,..} if sources.len()==1)
        );
        command(
            &sender,
            address,
            SessionCommand::Subscribe(receiver_subscription(256)),
        )
        .await;
        assert!(matches!(
            reply(&receiver).await,
            SessionCommand::Subscribed {
                id: 256,
                audio_owner: None,
                ..
            }
        ));
        assert!(publisher.demand().active);
        publisher.set_capture_available(false, false);
        assert!(matches!(
            reply(&receiver).await,
            SessionCommand::Unsubscribed { id: 256 }
        ));
        assert!(
            matches!(reply(&receiver).await, SessionCommand::Sources {sources,..} if sources.is_empty())
        );
        assert!(!publisher.demand().active);
        assert_eq!(publisher.address, address);
        assert!(!publisher.is_finished());

        let second = directory.0.join("after-sharing.bin");
        let reversed: Vec<u8> = contents.iter().rev().copied().collect();
        std::fs::write(&second, &reversed).unwrap();
        file_sender.deliver(&receiver, second).await;
        assert_eq!(
            std::fs::read(receive.join("after-sharing.bin")).unwrap(),
            reversed
        );
        // Reopening the SAME connection id succeeds, proving the authenticated
        // connection was not replaced during the media permission transition.
        command(
            &sender,
            address,
            SessionCommand::Open {
                connection_id: 17,
                version: SESSION_VERSION,
            },
        )
        .await;
        assert!(matches!(
            reply(&receiver).await,
            SessionCommand::Opened {
                connection_id: 17,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn screen_viewport_routes_exact_geometry_only_to_its_authenticated_subscription() {
        use protocol::viewport::{NormalizedRect,ViewportDemand};
        let directory=ReceiverTestDir::new();let key=b"screen-view-transport-test".to_vec();
        let publisher=MobilePublisher::start_with_permissions("127.0.0.1:0".parse().unwrap(),directory.0.clone(),key.clone(),true,false).await.unwrap();
        let (a,ar)=receiver_client(&publisher,&key).await;let (b,br)=receiver_client(&publisher,&key).await;
        for (sender,receiver,revision,x) in [(&a,&ar,1,0),(&b,&br,2,750_000)] {
            command(sender,publisher.address,SessionCommand::Subscribe(receiver_subscription(256))).await;
            assert!(matches!(reply(receiver).await,SessionCommand::Subscribed{id:256,..}));
            command(sender,publisher.address,SessionCommand::Viewport(ViewportDemand{id:256,source_revision:0,revision,
                visible:NormalizedRect{x,y:0,width:250_000,height:250_000},width:320,height:320})).await;
            assert!(matches!(reply(receiver).await,SessionCommand::ViewportAccepted{revision:r,..} if r==revision));
        }
        let views=publisher.demand().views;
        assert_eq!(views.len(),2);
        let left=views.iter().find(|view|view.geometry.demand_revision==1).unwrap();
        let now=remote_core::timing::quanta_now_us();
        let metadata=ScreenFrameMetadata{stream:left.stream,grant:left.grant,geometry:left.geometry,
            capture_ts_us:now-10_000_000,admitted_at_us:now};
        // The image is old, while this crop/encode admission is new. Keep both clocks truthful.
        assert!(publisher.screen_frame(metadata.clone(),vec![0,0,0,1,0x26,1,42]).await);
        let packet=tokio::time::timeout(Duration::from_secs(2),ar.recv()).await.unwrap().unwrap();
        let MultiplexedPacket::DataWithTiming(envelope,timing,_)=packet else{panic!("Expected timed screen frame")};
        let timing=timing.expect("Source timing was lost in transport");
        assert_eq!(protocol::viewport::split_frame(&envelope.payload).unwrap().0,Some(left.geometry));
        assert_eq!(timing.capture_ts_us,metadata.capture_ts_us);
        assert!(timing.encode_queue_ts_us>=10_000_000);
        assert!(tokio::time::timeout(Duration::from_millis(100),br.recv()).await.is_err(),"Another viewer received private ROI pixels");
        command(&a,publisher.address,SessionCommand::Unsubscribe{id:256}).await;
        assert!(matches!(reply(&ar).await,SessionCommand::Unsubscribed{id:256}));
        assert!(!publisher.screen_frame(metadata,vec![0,0,0,1,0x26,1,43]).await);
        assert_eq!(publisher.demand().views.len(),1);
        assert!(!publisher.frame_from(CaptureSource::MainDisplay,publisher.demand().epoch,vec![0,0,0,1,0x26,1,43],0,false).await,
            "Screen frames require immutable per-view geometry, never a source broadcast");
        let right=publisher.demand().views.pop().unwrap();
        publisher.fail_screen_view(right.stream,right.grant,"Fixture encoder capacity exhausted".into()).await.unwrap();
        assert!(matches!(reply(&br).await,SessionCommand::Error{request_id:256,..}));
        assert!(matches!(reply(&br).await,SessionCommand::Unsubscribed{id:256}));
        assert!(publisher.demand().views.is_empty());
        assert_eq!(publisher.source_grant(CaptureSource::MainDisplay),Some(right.grant));
        assert!(publisher.capabilities().borrow().can_stream,"A view failure revoked OS capture");
    }

    #[tokio::test]
    async fn camera_switch_preflight_preserves_other_viewers_and_requires_fresh_subscription() {
        let directory=ReceiverTestDir::new();let key=b"camera-switch-transport-test".to_vec();
        let publisher=MobilePublisher::start_with_permissions("127.0.0.1:0".parse().unwrap(),directory.0.clone(),key.clone(),false,false).await.unwrap();
        let sources=publisher.set_cameras([("front",CameraFacing::Front),("back",CameraFacing::Back)].into_iter().map(|(name,facing)|
            remote_core::cameras::NativeCamera {key:name.into(),name:name.into(),facing,switch_group:Some("phone".into()),
                width:1280,height:720,rotation:90}).collect()).unwrap();
        let from=sources[0].0;let target=sources[1].0;
        let (a,ar)=receiver_client(&publisher,&key).await;let (b,br)=receiver_client(&publisher,&key).await;
        let mut subscription=receiver_subscription(256);subscription.source=from;
        for (sender,receiver) in [(&a,&ar),(&b,&br)] {
            command(sender,publisher.address,SessionCommand::Subscribe(subscription.clone())).await;
            assert!(matches!(reply(receiver).await,SessionCommand::Subscribed{id:256,..}));
        }
        let switch=protocol::session::CameraSwitchRequest {id:256,request_id:9,source_revision:0,from,target,delivery_request_id:None};
        command(&a,publisher.address,SessionCommand::RequestCameraSwitch(switch.clone())).await;
        assert!(matches!(reply(&ar).await,SessionCommand::CameraSwitchResult {ready:Err(_),..}));
        command(&b,publisher.address,SessionCommand::Unsubscribe{id:256}).await;
        assert!(matches!(reply(&br).await,SessionCommand::Unsubscribed{id:256}));
        command(&a,publisher.address,SessionCommand::RequestCameraSwitch(switch.clone())).await;
        assert!(matches!(reply(&ar).await,SessionCommand::CameraSwitchResult {ready:Ok(()),..}));
        // Ready is preflight, never a source mutation or a claim of presentation.
        assert!(publisher.media.lock().unwrap().demand(from).active);
        assert!(!publisher.media.lock().unwrap().demand(target).active);
        command(&a,publisher.address,SessionCommand::Unsubscribe{id:256}).await;
        assert!(matches!(reply(&ar).await,SessionCommand::Unsubscribed{id:256}));
        subscription.id=512;subscription.source=target;
        command(&a,publisher.address,SessionCommand::Subscribe(subscription)).await;
        assert!(matches!(reply(&ar).await,SessionCommand::Subscribed{id:512,..}));
        command(&a,publisher.address,SessionCommand::RequestCameraSwitch(switch)).await;
        assert!(matches!(reply(&ar).await,SessionCommand::CameraSwitchResult {ready:Err(_),..}));
        assert!(publisher.media.lock().unwrap().demand(target).active);
    }

    #[tokio::test]
    async fn camera_fanout_and_projection_stop_preserve_independent_subscriptions() {
        // Synthetic encoded bytes exercise the real authenticated transport, not camera hardware.
        let directory=ReceiverTestDir::new();let key=b"camera-fanout-test".to_vec();
        let publisher=MobilePublisher::start_with_permissions("127.0.0.1:0".parse().unwrap(),directory.0.clone(),key.clone(),true,false).await.unwrap();
        let camera=publisher.set_cameras(vec![remote_core::cameras::NativeCamera {
            key:"fixture-front".into(),name:"Fixture front".into(),facing:CameraFacing::Front,
            switch_group:Some("fixture-phone".into()),width:1280,height:720,rotation:90,
        }]).unwrap()[0].0;
        let (a,ar)=receiver_client(&publisher,&key).await;let (b,br)=receiver_client(&publisher,&key).await;
        command(&a,publisher.address,SessionCommand::Subscribe(receiver_subscription(256))).await;
        assert!(matches!(reply(&ar).await,SessionCommand::Subscribed{id:256,..}));
        let mut request=receiver_subscription(512);request.source=camera;
        command(&a,publisher.address,SessionCommand::Subscribe(request.clone())).await;
        assert!(matches!(reply(&ar).await,SessionCommand::Subscribed{id:512,audio_owner:None,supports_input:false, max_touch_points: 0 }));
        request.id=256;command(&b,publisher.address,SessionCommand::Subscribe(request)).await;
        assert!(matches!(reply(&br).await,SessionCommand::Subscribed{id:256,audio_owner:None,supports_input:false, max_touch_points: 0 }));
        let epoch=publisher.media.lock().unwrap().demand(camera).epoch;
        assert!(publisher.frame_from(camera,epoch,vec![0,0,0,1,0x26,1,7],0,false).await);
        for (receiver,id) in [(&ar,512),(&br,256)] {
            let packet=tokio::time::timeout(Duration::from_secs(1),receiver.recv()).await.unwrap().unwrap();
            let MultiplexedPacket::Data(envelope,_)=packet else{panic!("expected camera frame")};
            assert_eq!(envelope.header.stream_id,id);assert_eq!(envelope.payload,vec![0,0,0,1,0x26,1,7]);
        }
        publisher.set_capture_available(false,false);
        assert!(matches!(reply(&ar).await,SessionCommand::Unsubscribed{id:256}));
        assert!(matches!(reply(&ar).await,SessionCommand::Sources{..}));
        assert!(matches!(reply(&br).await,SessionCommand::Sources{..}));
        assert!(publisher.media.lock().unwrap().demand(camera).active);
        assert_eq!(publisher.media.lock().unwrap().demand(camera).epoch,epoch);
        command(&a,publisher.address,SessionCommand::Unsubscribe{id:512}).await;
        assert!(matches!(reply(&ar).await,SessionCommand::Unsubscribed{id:512}));
        assert!(publisher.media.lock().unwrap().demand(camera).active);
        assert!(publisher.frame_from(camera,epoch,vec![0,0,0,1,0x26,1,8],0,false).await);
        let packet=tokio::time::timeout(Duration::from_secs(1),br.recv()).await.unwrap().unwrap();
        assert!(matches!(packet,MultiplexedPacket::Data(envelope,_) if envelope.header.stream_id==256&&envelope.payload.last()==Some(&8)));
        assert!(tokio::time::timeout(Duration::from_millis(60),ar.recv()).await.is_err());
        publisher.set_cameras(vec![]).unwrap();
        assert!(matches!(reply(&ar).await,SessionCommand::Sources{..}));
        assert!(matches!(reply(&br).await,SessionCommand::Unsubscribed{id:256}));
        assert!(matches!(reply(&br).await,SessionCommand::Sources{..}));
        assert!(!publisher.frame_from(camera,epoch,vec![9],0,false).await);
        assert!(!publisher.capabilities().borrow().can_stream);
        assert!(publisher.capabilities().borrow().file_transfer);
    }
    #[tokio::test]
    async fn camera_failure_does_not_close_a_screen_or_the_authenticated_file_endpoint() {
        let directory=ReceiverTestDir::new();let key=b"camera-failure-test".to_vec();
        let publisher=MobilePublisher::start_with_permissions("127.0.0.1:0".parse().unwrap(),directory.0.clone(),key.clone(),true,false).await.unwrap();
        let camera=publisher.set_cameras(vec![remote_core::cameras::NativeCamera {
            key:"fixture-camera".into(),name:"Fixture camera".into(),facing:CameraFacing::Back,
            switch_group:Some("fixture-phone".into()),width:1280,height:720,rotation:0,
        }]).unwrap()[0].0;
        let (sender,receiver)=receiver_client(&publisher,&key).await;
        command(&sender,publisher.address,SessionCommand::Subscribe(receiver_subscription(256))).await;let _=reply(&receiver).await;
        let mut request=receiver_subscription(512);request.source=camera;
        command(&sender,publisher.address,SessionCommand::Subscribe(request)).await;let _=reply(&receiver).await;
        let epoch=publisher.media.lock().unwrap().demand(camera).epoch;
        publisher.fail_source(camera,epoch,"fixture camera unavailable".into());
        assert!(matches!(reply(&receiver).await,SessionCommand::Error{request_id:512,..}));
        assert!(matches!(reply(&receiver).await,SessionCommand::Unsubscribed{id:512}));
        assert!(matches!(reply(&receiver).await,SessionCommand::Sources{..}));
        assert!(publisher.demand().active);assert!(publisher.capabilities().borrow().can_stream);
        assert!(publisher.capabilities().borrow().file_transfer);assert!(!publisher.is_finished());
        assert!(publisher.frame(vec![0,0,0,1,0x26,1,42],0,false).await);
        let packet=tokio::time::timeout(Duration::from_secs(1),receiver.recv()).await.unwrap().unwrap();
        assert!(matches!(packet,MultiplexedPacket::DataWithTiming(envelope,_,_) if envelope.header.stream_id==256));
    }
    #[tokio::test]
    async fn old_capture_frames_cannot_cross_authorization_epochs() {
        let directory = ReceiverTestDir::new();
        let key = b"receiver-epoch-test".to_vec();
        let publisher = MobilePublisher::start_with_permissions(
            "127.0.0.1:0".parse().unwrap(),
            directory.0.clone(),
            key.clone(),
            true,
            false,
        )
        .await
        .unwrap();
        let (sender, receiver) = receiver_client(&publisher, &key).await;
        let old_epoch = publisher.demand().epoch;
        command(
            &sender,
            publisher.address,
            SessionCommand::Subscribe(receiver_subscription(256)),
        )
        .await;
        assert!(matches!(
            reply(&receiver).await,
            SessionCommand::Subscribed { .. }
        ));
        publisher.set_capture_available(false, false);
        assert!(matches!(
            reply(&receiver).await,
            SessionCommand::Unsubscribed { .. }
        ));
        let _ = reply(&receiver).await;
        publisher.set_capture_available(true, false);
        let _ = reply(&receiver).await;
        command(
            &sender,
            publisher.address,
            SessionCommand::Subscribe(receiver_subscription(512)),
        )
        .await;
        assert!(matches!(
            reply(&receiver).await,
            SessionCommand::Subscribed { id: 512, .. }
        ));
        publisher
            .frames
            .send(Frame {
                source:CaptureSource::MainDisplay,
                epoch: old_epoch,
                bytes: vec![9, 9, 9],
                timestamp: 0,
                audio: false,
                screen_stream:None,
                geometry_revision:0,
                timing:None,
            })
            .await
            .unwrap();
        publisher.frame(vec![0, 0, 0, 1, 0x26, 1, 7], 0, false).await;
        let packet = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let envelope = match packet {
            MultiplexedPacket::Data(e, _) | MultiplexedPacket::DataWithTiming(e, _, _) => e,
            _ => panic!("expected media"),
        };
        assert_eq!(envelope.header.stream_id, 512);
        assert_eq!(protocol::viewport::split_frame(&envelope.payload).unwrap().1, [0, 0, 0, 1, 0x26, 1, 7]);
    }
}
