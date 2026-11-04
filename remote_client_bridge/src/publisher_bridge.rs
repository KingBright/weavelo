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
use crate::publisher_media::{ActiveRequest, MediaSources};
pub use crate::publisher_media::CaptureDemand;

// Bound authenticated transports, including clients which have not opened a file
// session yet. One MediaProjection encoder may still have only one viewer.
const MAX_PUBLISHER_PEERS: usize = 8;

struct Frame {
    source: CaptureSource,
    epoch: u64,
    bytes: Vec<u8>,
    timestamp: u32,
    audio: bool,
}
pub struct MobilePublisher {
    pub address: SocketAddr,
    media: Arc<Mutex<MediaSources>>,
    media_changed: watch::Sender<u64>,
    generation: String,
    alive: Arc<AtomicBool>,
    frames: mpsc::Sender<Frame>,
    errors: mpsc::Sender<(CaptureSource,u64,String)>,
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
}
impl MobileSubscription {
    fn new(request:SubscriptionRequest,grant:u64,audio:bool)->Self {
        Self{request,grant,video:true,audio,activity_revision:0,settings_revision:0,video_sequence:0,audio_sequence:0}
    }
    fn input_allowed(&self)->bool {self.request.source==CaptureSource::MainDisplay&&self.video}
}
fn recompute(peers:&HashMap<SocketAddr,Peer>,media:&Mutex<MediaSources>) {
    let mut media=media.lock().unwrap();
    media.recompute(peers.values().flat_map(|peer|peer.subscriptions.values())
        .map(|sub|ActiveRequest{request:&sub.request,video:sub.video,audio:sub.audio}));
    let mut streams=std::collections::BTreeMap::new();
    for peer in peers.values() {
        let count=peer.subscriptions.values().filter(|sub|media.sources.get(&sub.request.source)
            .is_some_and(|source|source.enabled&&source.grant==sub.grant)).count();
        if count>0 {*streams.entry(remote_core::routed_relay::incoming_device(peer.addr)).or_insert(0)+=count;}
    }
    media.peer_streams=streams.into_iter().collect();
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
                if sub.request.source==CaptureSource::MainDisplay{input.lock().unwrap().revoke_peer(peer.addr);}
                peer.retired.insert(id);
                reply(sender,peer.addr,SessionCommand::Unsubscribed{id}).await;
            }
        }
    }
    recompute(peers,media);
    let ready=input.lock().unwrap().ready();
    let catalog=media.lock().unwrap().catalog(ready);
    capabilities.send_modify(|caps|caps.can_stream=!catalog.is_empty()&&alive.load(Ordering::Acquire));
    for peer in peers.values(){reply(sender,peer.addr,SessionCommand::sources(0,catalog.clone())).await;}
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
    pub fn stop_sending(&self) { self.outgoing.lock().unwrap().take(); }
    pub fn stop_sending_id(&self, id:u64)->Result<(),String> {
        let mut outgoing=self.outgoing.lock().unwrap();
        if !outgoing.as_ref().is_some_and(|send|send.id==id) {return Err("Sending intention changed".into());}
        outgoing.take();Ok(())
    }
    pub async fn send_screen(&self,target:String,endpoint:SocketAddr)->Result<(),String> {
        self.send_sources(target,endpoint,vec![CaptureSource::MainDisplay]).await
    }
    pub async fn send_sources(&self,target:String,endpoint:SocketAddr,sources:Vec<CaptureSource>)->Result<(),String> {
        validate_delivery_sources(&sources).map_err(str::to_owned)?;
        if self.is_finished(){return Err("Camera and screen service is unavailable".into());}
        let grants={let media=self.media.lock().unwrap();sources.iter().map(|id|
            media.sources.get(id).filter(|source|source.enabled).map(|source|(*id,source.grant))
                .ok_or("Enable the selected camera or screen on this phone first"))
            .collect::<Result<Vec<_>,_>>()?};
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
        let task=tokio::spawn(async move {
            type Error=Box<dyn std::error::Error+Send+Sync>;
            let run=async {
                let (conn,mut events)=tokio::select! {biased;_=cancelled.changed()=>return Ok::<(),Error>(()),
                    result=tokio::time::timeout(Duration::from_secs(5),remote_core::workspace_session::WorkspaceConnection::connect_media(endpoint))=>result??};
                let mut ticket=[0;32];ticket[..16].copy_from_slice(&random_bytes_16());ticket[16..].copy_from_slice(&random_bytes_16());
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
                let revoked=broker.local(LocalIntent::Revoke {ticket,generation});
                let _=conn.control(SessionCommand::DeliveryCancel {request_id:request}).await;
                conn.close().await;revoked?;result
            }.await;
            *timeout_evidence.lock().unwrap()=run.as_ref().err().and_then(|error|
                error.downcast_ref::<remote_core::directed_delivery::ConfirmationTimeout>()).cloned();
            *progress.lock().unwrap()=match run {Ok(())=>"Sending stopped".into(),Err(error)=>format!("Delivery stopped: {error}")};
        });
        *outgoing=Some(Outgoing {id:sequence,target,stop,status,confirmation_timeout,task});Ok(())
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
        let (media_changed,mut changed_rx)=watch::channel(1u64);
        let (frames,mut frame_rx)=mpsc::channel::<Frame>(4);
        let (errors,mut error_rx)=mpsc::channel::<(CaptureSource,u64,String)>(8);
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
        let task = tokio::spawn(async move {
            let _advertised_lifetime = advertised_lifetime;
            let mut peers: HashMap<SocketAddr, Peer> = HashMap::new();
            let mut authenticated: HashMap<SocketAddr, ([u8;16], [u8;16], Instant)> = HashMap::new();
            let mut handshakes=remote_core::session_auth::ServerHandshake::default();
            let mut maintenance = tokio::time::interval(Duration::from_secs(1));
            let mut input_watchdog = tokio::time::interval(Duration::from_millis(protocol::realtime_input::INPUT_WATCHDOG_INTERVAL_MS));
            input_watchdog.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut input_ready = false;
            loop {
                tokio::select! {
                    biased;
                    changed=changed_rx.changed()=>{
                        if changed.is_err(){break;}
                        changed_rx.borrow_and_update();
                        refresh_sources(&mut peers,&shared,&shared_input,&sender,&source_capabilities,&task_alive).await;
                    }
                    Some((source,epoch,reason))=error_rx.recv()=>{
                        if shared.lock().unwrap().sources.get(&source).is_none_or(|entry|entry.enabled||entry.demand.epoch!=epoch){continue;}
                        for peer in peers.values() {
                            for sub in peer.subscriptions.values().filter(|sub|sub.request.source==source) {
                                reply(&sender,peer.addr,SessionCommand::Error{request_id:sub.request.id,reason:reason.clone()}).await;
                            }
                        }
                        refresh_sources(&mut peers,&shared,&shared_input,&sender,&source_capabilities,&task_alive).await;
                    }
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
                                        audio_owner:audio.then_some(sub.request.id),supports_input:ready&&sub.input_allowed()}).await;
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
                        if !shared.lock().unwrap().accepts_frame(frame.source,frame.epoch,frame.audio){continue;}
                        for peer in peers.values_mut() {
                            if peer.delivery_owner.is_some_and(|(owner,ticket)|!peer_delivery.claim_alive(owner,ticket)){continue;}
                            for sub in peer.subscriptions.values_mut().filter(|sub|sub.request.source==frame.source) {
                                if (frame.audio&&!sub.audio)||(!frame.audio&&!sub.video){continue;}
                                let (id,payload_type,sequence)=if frame.audio {
                                    (protocol::remote_system_audio_stream_id(sub.request.id),protocol::PayloadType::AudioOpus as u8,&mut sub.audio_sequence)
                                } else {(sub.request.id,96,&mut sub.video_sequence)};
                                let packet=RtpPacket{header:RtpHeader{version:2,payload_type,sequence_number:*sequence,timestamp:frame.timestamp,ssrc:id},payload:frame.bytes.clone()};
                                *sequence=sequence.wrapping_add(1);
                                if let Ok(envelope)=remote_core::media_plane::rtp_to_realtime_data(&packet){let _=peer.scheduled.send(envelope).await;}
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
                                            reply(&sender, addr, SessionCommand::Opened { connection_id, version:SESSION_VERSION, max_subscriptions:MAX_SUBSCRIPTIONS as u32, files:peers[&addr].files.is_some(), clipboard:false, window_capture:false }).await;
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
                                        SessionCommand::ListSources{request_id,..} if peers.contains_key(&addr)=>{
                                            let ready=shared_input.lock().unwrap().ready();
                                            let catalog=shared.lock().unwrap().catalog(ready);
                                            reply(&sender,addr,SessionCommand::sources(request_id,catalog)).await;
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
                                                reply(&sender,addr,SessionCommand::Error{request_id:request.id,reason:"Enable the selected camera or screen on this phone; files remain available".into()}).await;continue;
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
                                            let supports_input=request.source==CaptureSource::MainDisplay&&shared_input.lock().unwrap().ready();
                                            peers.get_mut(&addr).unwrap().subscriptions.entry(id).or_insert_with(||MobileSubscription::new(request,source.grant,audio));
                                            recompute(&peers,&shared);
                                            reply(&sender,addr,SessionCommand::Subscribed{id,audio_owner:audio.then_some(id),supports_input}).await;
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
                                        SessionCommand::Configure{id,revision,width,height,fps,bitrate_kbps} if peers.contains_key(&addr)=>{
                                            let Some(sub)=peers.get_mut(&addr).unwrap().subscriptions.get_mut(&id)else{continue;};
                                            let updated=SubscriptionRequest{width,height,fps,bitrate_kbps,..sub.request.clone()};
                                            if !valid(&updated){reply(&sender,addr,SessionCommand::Error{request_id:id,reason:"Unsupported stream settings".into()}).await;continue;}
                                            if revision>sub.settings_revision{sub.settings_revision=revision;sub.request=updated;}
                                            let response=SessionCommand::Configured{id,revision:sub.settings_revision};
                                            recompute(&peers,&shared);reply(&sender,addr,response).await;
                                        }
                                        SessionCommand::Unsubscribe{id} if peers.contains_key(&addr)=>{
                                            let peer=peers.get_mut(&addr).unwrap();
                                            let reserved=peer.subscriptions.len()-usize::from(peer.subscriptions.contains_key(&id));
                                            if !peer.retired.contains(&id)&&peer.retired.len()+reserved>=4096 {
                                                reply(&sender,addr,SessionCommand::Error{request_id:id,reason:"Subscription ID capacity reached; reconnect".into()}).await;continue;
                                            }
                                            peer.retired.insert(id);
                                            if peer.subscriptions.remove(&id).is_some_and(|sub|sub.request.source==CaptureSource::MainDisplay){shared_input.lock().unwrap().revoke_peer(addr);}
                                            recompute(&peers,&shared);reply(&sender,addr,SessionCommand::Unsubscribed{id}).await;
                                        }
                                        SessionCommand::RequestInputBinding {connection_id,id,source_revision:0,request}
                                            if request != 0 && peers.get(&addr).is_some_and(|p|p.id==connection_id&&p.subscriptions.get(&id).is_some_and(MobileSubscription::input_allowed)) => {
                                            let binding = shared_input.lock().unwrap().grant(addr,connection_id,id);
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
                                        SessionCommand::ReleaseInput {id} if peers.get(&addr).is_some_and(|p|p.subscriptions.get(&id).is_some_and(|sub|sub.request.source==CaptureSource::MainDisplay)) => {
                                            shared_input.lock().unwrap().revoke_peer(addr);
                                            reply(&sender,addr,SessionCommand::InputReleased {id}).await;
                                        }
                                        SessionCommand::Close { connection_id } if peers.get(&addr).is_some_and(|p| p.id==connection_id) => {
                                            shared_input.lock().unwrap().revoke_peer(addr);
                                            peers.remove(&addr);recompute(&peers,&shared);
                                            authenticated.remove(&addr);
                                            sender.remove_peer_crypto(addr);
                                            handshakes.forget(addr);
                                        }
                                        _ => {}
                                    },
                                    ControlMessage::Ping { client_send_ts } if peers.contains_key(&addr) => { let now=now_unix_ms(); let _=sender.send_control(&ControlMessage::Pong { client_send_ts,host_recv_ts:now,host_send_ts:now },addr).await; }
                                    ControlMessage::RequestKeyframe{session_id}=>{
                                        if let Some(sub)=peers.get(&addr).and_then(|peer|peer.subscriptions.get(&session_id)) {
                                            shared.lock().unwrap().request_keyframe(sub.request.source);
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
        });
        Ok(Self {
            address,
            media,
            media_changed,
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
        if self.media.lock().unwrap().set_screen(enabled,audio){
            self.input.lock().unwrap().set_capture(false);
            self.notify_sources();
        }
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
    pub fn demand(&self)->CaptureDemand{self.media.lock().unwrap().demand(CaptureSource::MainDisplay)}
    pub fn is_finished(&self)->bool{!self.alive.load(Ordering::Acquire)||self.task.is_finished()}
    pub fn source_grant(&self,source:CaptureSource)->Option<u64>{self.media.lock().unwrap().sources.get(&source).filter(|source|source.enabled).map(|source|source.grant)}
    pub fn configure_geometry(&self,source:CaptureSource,epoch:u64,width:u32,height:u32,rotation:u16)->Result<u64,String> {
        let result=self.media.lock().unwrap().geometry(source,epoch,width,height,rotation)?;
        if result!=epoch {if source==CaptureSource::MainDisplay{self.input.lock().unwrap().revoke();}self.notify_sources();}
        Ok(result)
    }
    pub async fn frame_from(&self,source:CaptureSource,epoch:u64,bytes:Vec<u8>,_timestamp_us:i64,audio:bool)->bool {
        if bytes.len()>8*1024*1024||!self.media.lock().unwrap().accepts_frame(source,epoch,audio){return false;}
        // Backpressure the codec drain worker; never discard an encoded reference frame locally.
        self.frames.send(Frame{source,epoch,bytes,timestamp:now_unix_ms() as u32,audio}).await.is_ok()
    }
    #[cfg(test)]
    async fn frame(&self,bytes:Vec<u8>,timestamp:i64,audio:bool)->bool {
        let epoch=if audio{self.source_grant(CaptureSource::MainDisplay).unwrap_or(0)}else{self.demand().epoch};
        self.frame_from(CaptureSource::MainDisplay,epoch,bytes,timestamp,audio).await
    }
    pub fn fail_source(&self,source:CaptureSource,epoch:u64,reason:String){
        let changed={let mut media=self.media.lock().unwrap();
            media.fail(source,epoch).then(||(media.demand(source).epoch,media.sources.values().any(|entry|entry.enabled)))};
        if let Some((epoch,available))=changed {
            self.capabilities.send_modify(|caps|caps.can_stream=available&&!self.is_finished());
            if self.errors.try_send((source,epoch,reason)).is_err(){self.media_changed.send_modify(|value|*value=value.wrapping_add(1));}
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
    async fn sending_requires_capture_and_stop_cannot_cancel_a_replacement_intent() {
        let publisher = MobilePublisher::start_with_access("127.0.0.1:0".parse().unwrap(),
            std::env::temp_dir(), b"send-intent-test".to_vec(), false, false, false).await.unwrap();
        publisher.set_device_identity("phone".into());
        assert!(publisher.send_screen("desktop".into(), "127.0.0.1:9".parse().unwrap()).await.unwrap_err().contains("Enable the selected"));
        assert!(!publisher.delivery.upgrade_busy());
        let (stop, mut cancelled) = watch::channel(false);
        let (finished, done) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move { let _=cancelled.changed().await; let _=finished.send(()); });
        *publisher.outgoing.lock().unwrap()=Some(Outgoing {id:2,target:"desktop".into(),stop,
            status:Arc::new(Mutex::new("Sending".into())),confirmation_timeout:Arc::new(Mutex::new(None)),task});
        assert!(publisher.stop_sending_id(1).is_err());
        assert_eq!(publisher.outgoing_status()["id"], 2);
        assert!(publisher.outgoing_status()["active"].as_bool().unwrap());
        publisher.stop_sending_id(2).unwrap();
        tokio::time::timeout(Duration::from_secs(1), done).await.unwrap().unwrap();
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
    async fn reply(receiver: &remote_core::net::UdpReceiver) -> SessionCommand {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let MultiplexedPacket::Control(ControlMessage::Session(message), _) =
                    receiver.recv().await.unwrap()
                {
                    return *message;
                }
            }
        })
        .await
        .unwrap()
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
        let MultiplexedPacket::Data(envelope, _) = media else {
            panic!("expected encoded frame")
        };
        assert_eq!(envelope.header.stream_id, 256);
        assert_eq!(envelope.payload, vec![0, 0, 0, 1, 0x26, 0x01, 1, 2, 3]);
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
    async fn receiver_client(
        publisher: &MobilePublisher,
        key: &[u8],
    ) -> (UdpSender, remote_core::net::UdpReceiver) {
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
        let (sender,receiver)=Box::pin(receiver_client(&publisher,&key)).await;
        let (other,_other_receiver)=Box::pin(receiver_client(&publisher,&key)).await;
        command(&sender,publisher.address,SessionCommand::Subscribe(receiver_subscription(256))).await;
        loop { if matches!(reply(&receiver).await,SessionCommand::Subscribed {supports_input:true,..}) {break;} }
        command(&sender,publisher.address,SessionCommand::RequestInputBinding {connection_id:17,id:256,source_revision:0,request:1}).await;
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
        for _ in 0..4 { clients.push(Box::pin(receiver_client(&publisher,&key)).await); }
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
        assert!(matches!(packet,MultiplexedPacket::Data(e,_) if e.header.stream_id==256 && e.payload==vec![0,0,0,1,0x26,1,9]));
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
        assert!(matches!(reply(&ar).await,SessionCommand::Subscribed{id:512,audio_owner:None,supports_input:false}));
        request.id=256;command(&b,publisher.address,SessionCommand::Subscribe(request)).await;
        assert!(matches!(reply(&br).await,SessionCommand::Subscribed{id:256,audio_owner:None,supports_input:false}));
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
        assert!(matches!(packet,MultiplexedPacket::Data(envelope,_) if envelope.header.stream_id==256));
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
        assert_eq!(envelope.payload, vec![0, 0, 0, 1, 0x26, 1, 7]);
    }
}
