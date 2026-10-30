//! Android's user-authorized MediaProjection feeds encoded frames into the shared
//! transport. A projection exposes one selected source; it cannot enumerate or
//! silently switch to arbitrary third-party application windows.
use protocol::{ControlMessage, RtpHeader, RtpPacket, session::*};
use remote_core::{
    directed_delivery::{Broker, Incoming, Peer as DeliveryPeer},
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

// Bound authenticated transports, including clients which have not opened a file
// session yet. One MediaProjection encoder may still have only one viewer.
const MAX_PUBLISHER_PEERS: usize = 8;

#[derive(Clone, serde::Serialize)]
pub struct CaptureDemand {
    pub active: bool,
    pub audio: bool,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub keyframe: u64,
}
impl Default for CaptureDemand {
    fn default() -> Self {
        Self {
            active: false,
            audio: false,
            width: 1280,
            height: 720,
            fps: 30,
            bitrate_kbps: 5000,
            keyframe: 0,
        }
    }
}
#[derive(Clone, Copy)]
struct MediaPermission {
    enabled: bool,
    audio: bool,
    epoch: u64,
}
struct Frame {
    epoch: u64,
    bytes: Vec<u8>,
    timestamp: u32,
    audio: bool,
}
pub struct MobilePublisher {
    pub address: SocketAddr,
    demand: Arc<Mutex<CaptureDemand>>,
    source: Arc<Mutex<(u32, u32)>>,
    frames: mpsc::Sender<Frame>,
    errors: mpsc::Sender<String>,
    permission: watch::Sender<MediaPermission>,
    input: Arc<Mutex<crate::publisher_input::PublisherInput>>,
    capabilities: watch::Sender<remote_core::discovery::DiscoveryCapabilities>,
    task: tokio::task::JoinHandle<()>,
    file_reception: Arc<AtomicBool>,
    delivery: Arc<Broker>,
    incoming: Mutex<mpsc::Receiver<Arc<Incoming>>>,
    deliveries: Mutex<HashMap<String,Arc<Incoming>>>,
}
impl Drop for MobilePublisher {
    fn drop(&mut self) {
        self.capabilities.send_replace(crate::inbound_capabilities(0, false));
        self.input.lock().unwrap().set_capture(false);
        self.task.abort();
    }
}

// The actual serving task owns the advertised lifetime. A failed or aborted
// worker must not leave a stale screen/file claim in discovery.
struct AdvertisedPublisher(watch::Sender<remote_core::discovery::DiscoveryCapabilities>, Arc<Mutex<crate::publisher_input::PublisherInput>>);
impl Drop for AdvertisedPublisher {
    fn drop(&mut self) {
        self.0.send_replace(crate::inbound_capabilities(0, false));
        self.1.lock().unwrap().set_capture(false);
    }
}
struct Peer {
    addr: SocketAddr,
    id: u32,
    seen: Instant,
    subscription: Option<SubscriptionRequest>,
    retired: std::collections::HashSet<u32>,
    activity_revision: u64,
    settings_revision: u64,
    video_sequence: u16,
    audio_sequence: u16,
    scheduled: ScheduledDataSender,
    files: Option<mpsc::Sender<protocol::DataEnvelope>>,
    cancel: broadcast::Sender<()>,
    tasks: Vec<tokio::task::AbortHandle>,
    delivery_owner: Option<(DeliveryPeer,[u8;32])>,
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
    pub fn set_device_identity(&self, identity:String) { self.delivery.set_identity(identity); }
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
                "claim":DeliveryClaimProof {ticket:offer.ticket,target_device_id:offer.target.clone(),request_id:offer.request,sources:offer.sources.clone()}});
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
        let demand = Arc::new(Mutex::new(CaptureDemand::default()));
        let source = Arc::new(Mutex::new((1080, 1920)));
        let shared = demand.clone();
        let geometry = source.clone();
        let (frames, mut frame_rx) = mpsc::channel::<Frame>(4);
        let (errors, mut error_rx) = mpsc::channel::<String>(1);
        let (permission, mut permission_rx) = watch::channel(MediaPermission {
            enabled: capture_enabled,
            audio: capture_enabled && allow_audio,
            epoch: 1,
        });
        let mut advertised = crate::inbound_capabilities(address.port(), capture_enabled);
        advertised.file_transfer = allow_files;
        let (capabilities, _) = watch::channel(advertised);
        let file_reception = Arc::new(AtomicBool::new(allow_files));
        let peer_files = file_reception.clone();
        let input = Arc::new(Mutex::new(crate::publisher_input::PublisherInput::default()));
        let shared_input = input.clone();
        let (delivery,incoming)=Broker::new();
        let peer_delivery=delivery.clone();
        let scope=Some(remote_core::shared_files::share_scope_for_key(&key));
        let advertised_lifetime = AdvertisedPublisher(capabilities.clone(), input.clone());
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
                    changed = permission_rx.changed() => {
                        if changed.is_err() { break; }
                        let permission = *permission_rx.borrow_and_update();
                        shared_input.lock().unwrap().revoke();
                        // A new authorization must never inherit subscriptions or queued media.
                        while frame_rx.try_recv().is_ok() {}
                        *shared.lock().unwrap() = CaptureDemand::default();
                        for p in peers.values_mut() {
                            if let Some(subscription) = p.subscription.take() {
                                p.retired.insert(subscription.id);
                                p.activity_revision = 0;
                                p.settings_revision = 0;
                                reply(&sender, p.addr, SessionCommand::Unsubscribed { id: subscription.id }).await;
                            }
                            let (width, height) = *geometry.lock().unwrap();
                            let supports_input = shared_input.lock().unwrap().ready();
                            reply(&sender, p.addr, SessionCommand::Sources { source_icons: Vec::new(), icons: Vec::new(),
                                request_id: 0,
                                sources: available_sources(permission, width, height, supports_input),
                            }).await;
                        }
                    }
                    Some(reason) = error_rx.recv() => {
                        for p in peers.values() { reply(&sender,p.addr,SessionCommand::Closed {connection_id:p.id,reason:reason.clone()}).await; }
                        break;
                    }
                    _ = input_watchdog.tick(), if input_ready || shared_input.lock().unwrap().ready() => {
                        let (ready, handled) = {
                            let mut input = shared_input.lock().unwrap();
                            (input.ready(), input.take_handled())
                        };
                        if ready != input_ready {
                            input_ready = ready;
                            for p in peers.values() {
                                if let Some(subscription) = &p.subscription {
                                    let audio = permission_rx.borrow().audio;
                                    reply(&sender,p.addr,SessionCommand::Subscribed { id:subscription.id, audio_owner:audio.then_some(subscription.id), supports_input:ready }).await;
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
                            if let Some(id) = peers.get_mut(&addr).and_then(|peer|peer.retire_media(&shared_input, &shared)) {
                                reply(&sender, addr, SessionCommand::Unsubscribed { id }).await;
                            }
                        }
                        handshakes.expire(remote_core::session_auth::AuthTime::now());
                        authenticated.retain(|addr, (_, _, at)| {
                            let expired = peers.get(addr).map_or_else(
                                || at.elapsed() > Duration::from_secs(30),
                                |p| p.seen.elapsed() > Duration::from_secs(30),
                            );
                            if expired {
                                shared_input.lock().unwrap().revoke_peer(*addr);
                                if peers.remove(addr).is_some_and(|p| p.subscription.is_some()) {
                                    *shared.lock().unwrap() = CaptureDemand::default();
                                }
                                sender.remove_peer_crypto(*addr);
                                handshakes.forget(*addr);
                            }
                            !expired
                        });
                        if let Some(p) = peers.values().find(|p| p.subscription.is_some()) && shared.lock().unwrap().audio && let Some(subscription) = &p.subscription {
                            let config = protocol::AudioStreamConfig::remote_system(protocol::remote_system_audio_stream_id(subscription.id), 48000, 2, 20);
                            if let Ok(envelope) = remote_core::media_plane::audio_stream_config_to_envelope(&config, 0, now_unix_ms()) { let _ = p.scheduled.send(envelope).await; }
                        }
                    }
                    Some(frame) = frame_rx.recv() => {
                        let permission = *permission_rx.borrow();
                        if !permission.enabled || frame.epoch != permission.epoch || (frame.audio && !permission.audio) { continue; }
                        let Some(p) = peers.values_mut().find(|p| p.subscription.is_some()) else { continue; };
                        if p.delivery_owner.is_some_and(|(owner,ticket)|!peer_delivery.claim_alive(owner,ticket)) {continue;}
                        let Some(subscription) = &p.subscription else { continue; };
                        let wanted = shared.lock().unwrap().clone();
                        if (frame.audio && !wanted.audio) || (!frame.audio && !wanted.active) { continue; }
                        let (id, payload_type, sequence) = if frame.audio {
                            (protocol::remote_system_audio_stream_id(subscription.id), protocol::PayloadType::AudioOpus as u8, &mut p.audio_sequence)
                        } else { (subscription.id, 96, &mut p.video_sequence) };
                        let packet = RtpPacket { header:RtpHeader { version:2, payload_type, sequence_number:*sequence, timestamp:frame.timestamp, ssrc:id }, payload:frame.bytes };
                        *sequence = sequence.wrapping_add(1);
                        if let Ok(envelope) = remote_core::media_plane::rtp_to_realtime_data(&packet) { let _ = p.scheduled.send(envelope).await; }
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
                                    if peers.remove(&addr).is_some_and(|peer|peer.subscription.is_some()) {*shared.lock().unwrap()=CaptureDemand::default();}
                                    authenticated.remove(&addr);sender.remove_peer_crypto(addr);continue;
                                }
                                if let Some(peer) = peers.get_mut(&addr)
                                    && peer.delivery_owner.is_some_and(|(owner,ticket)|!peer_delivery.claim_alive(owner,ticket))
                                    && let Some(id) = peer.retire_media(&shared_input, &shared) {
                                    reply(&sender, addr, SessionCommand::Unsubscribed { id }).await;
                                }
                                if let Some(p) = peers.get_mut(&addr) { p.seen = Instant::now(); }
                                match message {
                                    ControlMessage::Session(command) => match *command {
                                        SessionCommand::Open { connection_id, version:SESSION_VERSION } => {
                                            if peers.get(&addr).is_some_and(|p| p.id != connection_id) { reply(&sender, addr, SessionCommand::Error { request_id:connection_id, reason:"close the previous connection first".into() }).await; continue; }
                                            peers.entry(addr).or_insert_with(|| Peer::new(addr, connection_id, &sender, directory.clone(), peer_files.load(Ordering::Relaxed)));
                                            reply(&sender, addr, SessionCommand::Opened { connection_id, version:SESSION_VERSION, max_subscriptions:1, files:peers[&addr].files.is_some(), clipboard:false, window_capture:false }).await;
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
                                                || (peer.delivery_owner.is_none() && peer.subscription.is_some()) {
                                                SessionCommand::DeliveryReceipt {request_id,revision:0,state:DeliveryState::Rejected,reason:"Connection belongs to another activity".into()}
                                            } else {peer_delivery.claim(owner,request_id,&target_device_id,ticket,sources)};
                                            if matches!(response,SessionCommand::DeliveryReceipt {state:DeliveryState::Claimed,..}) {peer.delivery_owner=Some((owner,ticket));}
                                            reply(&sender,addr,response).await;
                                        }
                                        SessionCommand::DeliveryCancel {request_id} if peers.contains_key(&addr) => {
                                            let peer=&peers[&addr];let (_,nonce,_)=authenticated[&addr];
                                            reply(&sender,addr,peer_delivery.cancel(DeliveryPeer {addr,connection:peer.id,nonce,scope},request_id)).await;
                                        }
                                        SessionCommand::ListSources { request_id, .. } if peers.contains_key(&addr) => {
                                            let (width, height) = *geometry.lock().unwrap();
                                            let permission = *permission_rx.borrow();
                                            let supports_input = shared_input.lock().unwrap().ready();
                                            reply(&sender, addr, SessionCommand::Sources { source_icons: Vec::new(), icons: Vec::new(),  request_id, sources:available_sources(permission, width, height, supports_input) }).await;
                                        }
                                        SessionCommand::Subscribe(request) if peers.contains_key(&addr) => {
                                            let peer=&peers[&addr];
                                            if let Some((owner,ticket))=peer.delivery_owner
                                                && (!peer_delivery.allows_source(owner,ticket,request.source)
                                                    || authenticated.get(&addr).is_none_or(|(_,nonce,_)|*nonce!=owner.nonce)) {
                                                reply(&sender,addr,SessionCommand::Error {request_id:request.id,reason:"Sending intent ended or source changed".into()}).await;continue;
                                            }
                                            let permission = *permission_rx.borrow();
                                            if !permission.enabled {
                                                reply(&sender, addr, SessionCommand::Error { request_id: request.id, reason: "Screen sharing is not authorized on this device; files remain available".into() }).await;
                                                continue;
                                            }
                                            if peers.values().any(|p| p.addr != addr && p.subscription.is_some()) {
                                                reply(&sender, addr, SessionCommand::Error { request_id:request.id, reason:"Android screen sharing already has an active viewer; files remain available".into() }).await;
                                                continue;
                                            }
                                            let p = peers.get_mut(&addr).unwrap();
                                            if request.source != CaptureSource::MainDisplay || !valid(&request) || p.retired.contains(&request.id) || p.retired.len() >= 4096 || p.subscription.as_ref().is_some_and(|s| s.id != request.id) {
                                                reply(&sender, addr, SessionCommand::Error { request_id:request.id, reason:"Android projection supports one authorized source".into() }).await; continue;
                                            }
                                            if p.subscription.is_none() { let mut d = shared.lock().unwrap(); set_rates(&mut d, &request); d.active=true; d.audio=request.audio && permission.audio; d.keyframe+=1; }
                                            let id=request.id; if p.subscription.is_none() { p.subscription=Some(request); }
                                            let supports_input = shared_input.lock().unwrap().ready();
                                            reply(&sender, addr, SessionCommand::Subscribed { id, audio_owner:permission.audio.then_some(id), supports_input }).await;
                                        }
                                        SessionCommand::SetActivity { id, revision, video, audio } if peers.contains_key(&addr) => {
                                            let p=peers.get_mut(&addr).unwrap(); if p.subscription.as_ref().is_none_or(|s| s.id != id) { continue; }
                                            let (video, audio) = { let mut d=shared.lock().unwrap(); if revision > p.activity_revision { if !video { shared_input.lock().unwrap().revoke_peer(addr); } p.activity_revision=revision; if video && !d.active { d.keyframe+=1; } d.active=video; d.audio=audio && permission_rx.borrow().audio; } (d.active,d.audio) };
                                            reply(&sender, addr, SessionCommand::Activity { id, revision:p.activity_revision, video, audio }).await;
                                        }
                                        SessionCommand::Configure { id, revision, width, height, fps, bitrate_kbps } if peers.contains_key(&addr) => {
                                            let p=peers.get_mut(&addr).unwrap();
                                            if let Some(s)=p.subscription.as_mut().filter(|s| s.id==id) {
                                                let updated=SubscriptionRequest { width,height,fps,bitrate_kbps,..s.clone() };
                                                if !valid(&updated) { reply(&sender,addr,SessionCommand::Error {request_id:id,reason:"unsupported stream settings".into()}).await; continue; }
                                                if revision > p.settings_revision { p.settings_revision=revision; *s=updated; set_rates(&mut shared.lock().unwrap(),s); }
                                                reply(&sender,addr,SessionCommand::Configured {id,revision:p.settings_revision}).await;
                                            }
                                        }
                                        SessionCommand::Unsubscribe { id } if peers.contains_key(&addr) => {
                                            let p=peers.get_mut(&addr).unwrap();
                                            // Reserve a tombstone for the live subscription on permission revocation.
                                            let reserved = usize::from(p.subscription.as_ref().is_some_and(|s| s.id != id));
                                            if p.retired.len() + reserved >= 4096 && !p.retired.contains(&id) {
                                                reply(&sender,addr,SessionCommand::Error {request_id:id,reason:"subscription ID capacity reached; reconnect".into()}).await;
                                                continue;
                                            }
                                            p.retired.insert(id); if p.subscription.as_ref().is_some_and(|s| s.id==id) { shared_input.lock().unwrap().revoke_peer(addr); p.subscription=None; p.activity_revision=0; p.settings_revision=0; shared.lock().unwrap().active=false; shared.lock().unwrap().audio=false; }
                                            reply(&sender,addr,SessionCommand::Unsubscribed {id}).await;
                                        }
                                        SessionCommand::RequestInputBinding {connection_id,id,source_revision:0,request}
                                            if request != 0 && shared.lock().unwrap().active && peers.get(&addr).is_some_and(|p| p.id==connection_id && p.subscription.as_ref().is_some_and(|s| s.id==id)) => {
                                            let binding = shared_input.lock().unwrap().grant(addr,connection_id,id);
                                            if let Some(binding) = binding {
                                                reply(&sender,addr,SessionCommand::InputBinding {binding,request,host_time_ms:crate::publisher_input::now_ms()}).await;
                                            }
                                        }
                                        SessionCommand::CommitText(request) if shared.lock().unwrap().active && peers.get(&addr).is_some_and(|p| p.id==request.binding.connection_id && p.subscription.as_ref().is_some_and(|s| s.id==request.binding.subscription_id)) => {
                                            let response=shared_input.lock().unwrap().admit_text(addr,&request);
                                            if let Some(response)=response {reply(&sender,addr,response).await;}
                                        }
                                        SessionCommand::RealtimeInput(packet) if shared.lock().unwrap().active && peers.get(&addr).is_some_and(|p| p.id==packet.binding.connection_id && p.subscription.as_ref().is_some_and(|s| s.id==packet.binding.subscription_id)) => {
                                            shared_input.lock().unwrap().admit(addr,&packet);
                                        }
                                        SessionCommand::ReleaseInput {id} if peers.get(&addr).is_some_and(|p| p.subscription.as_ref().is_some_and(|s| s.id==id)) => {
                                            shared_input.lock().unwrap().revoke_peer(addr);
                                            reply(&sender,addr,SessionCommand::InputReleased {id}).await;
                                        }
                                        SessionCommand::Close { connection_id } if peers.get(&addr).is_some_and(|p| p.id==connection_id) => {
                                            shared_input.lock().unwrap().revoke_peer(addr);
                                            if peers.remove(&addr).is_some_and(|p| p.subscription.is_some()) { *shared.lock().unwrap()=CaptureDemand::default(); }
                                            authenticated.remove(&addr);
                                            sender.remove_peer_crypto(addr);
                                            handshakes.forget(addr);
                                        }
                                        _ => {}
                                    },
                                    ControlMessage::Ping { client_send_ts } if peers.contains_key(&addr) => { let now=now_unix_ms(); let _=sender.send_control(&ControlMessage::Pong { client_send_ts,host_recv_ts:now,host_send_ts:now },addr).await; }
                                    ControlMessage::RequestKeyframe { .. } if peers.get(&addr).is_some_and(|p| p.subscription.is_some()) => { shared.lock().unwrap().keyframe+=1; }
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
            demand,
            source,
            frames,
            errors,
            permission,
            input,
            capabilities,
            task,file_reception,delivery,incoming:Mutex::new(incoming),deliveries:Mutex::new(HashMap::new()),
        })
    }
    /// Detach or attach media without replacing the authenticated file connection.
    pub fn set_capture_available(&self, enabled: bool, audio: bool) {
        self.permission.send_if_modified(|permission| {
            let audio = enabled && audio;
            if permission.enabled == enabled && permission.audio == audio {
                return false;
            }
            permission.enabled = enabled;
            permission.audio = audio;
            permission.epoch = permission.epoch.wrapping_add(1);
            true
        });
        self.capabilities.send_modify(|capabilities| {
            // Once the task has retired its file endpoint, a late permission
            // callback cannot resurrect the capture announcement.
            // Read the current permission while publishing, rather than replaying
            // a stale callback argument after a concurrent revoke.
            capabilities.can_stream = !self.task.is_finished() && self.permission.borrow().enabled;
        });
        if !enabled {
            self.input.lock().unwrap().set_capture(false);
            *self.demand.lock().unwrap() = CaptureDemand::default();
        }
    }
    pub fn set_file_reception(&self, enabled: bool) {
        self.file_reception.store(enabled, Ordering::Relaxed);
        self.capabilities.send_modify(|capabilities| capabilities.file_transfer = enabled && !self.task.is_finished());
    }
    pub fn set_input_service_available(&self, available: bool) { self.input.lock().unwrap().set_service(available); }
    pub fn set_input_capture_allowed(&self, allowed: bool) { self.input.lock().unwrap().set_capture(allowed && self.permission.borrow().enabled); }
    pub fn input_dispatch(&self) -> Result<String,String> {
        serde_json::to_string(&self.input.lock().unwrap().poll()).map_err(|e| e.to_string())
    }
    pub fn input_execution_valid(&self, generation: u64, token: u64) -> bool { self.input.lock().unwrap().execution_valid(generation,token) }
    pub fn complete_input(&self, generation: u64, token: u64, accepted: bool) { self.input.lock().unwrap().complete(generation,token,accepted); }
    pub fn capabilities(&self) -> watch::Receiver<remote_core::discovery::DiscoveryCapabilities> {
        self.capabilities.subscribe()
    }
    pub fn demand(&self) -> CaptureDemand {
        let mut demand = self.demand.lock().unwrap().clone();
        let permission = *self.permission.borrow();
        demand.active &= permission.enabled;
        demand.audio &= permission.enabled && permission.audio;
        demand
    }
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
    pub fn set_source_size(&self, width: u32, height: u32) {
        let mut source = self.source.lock().unwrap();
        if *source != (width,height) { self.input.lock().unwrap().revoke(); *source = (width,height); }
    }
    pub fn frame(&self, bytes: Vec<u8>, _timestamp_us: i64, audio: bool) {
        let permission = *self.permission.borrow();
        if !permission.enabled || (audio && !permission.audio) || bytes.len() > 8 * 1024 * 1024 {
            return;
        }
        if self
            .frames
            .try_send(Frame {
                epoch: permission.epoch,
                bytes,
                timestamp: now_unix_ms() as u32,
                audio,
            })
            .is_err()
            && !audio
        {
            self.demand.lock().unwrap().keyframe += 1;
        }
    }
    pub fn fail(&self, reason: String) {
        self.set_capture_available(false, false);
        let _ = self.errors.try_send(reason);
    }
}
fn available_sources(
    permission: MediaPermission,
    width: u32,
    height: u32,
    supports_input: bool,
) -> Vec<CaptureSourceInfo> {
    if !permission.enabled {
        return Vec::new();
    }
    vec![CaptureSourceInfo {
        source: CaptureSource::MainDisplay,
        title: "Android selected screen or app".into(),
        application: "MediaProjection".into(),
        process_id: None,
        width,
        height,
        supports_input,
    }]
}
fn valid(s: &SubscriptionRequest) -> bool {
    protocol::validate_video_settings(s.width, s.height, s.fps, s.bitrate_kbps).is_ok()
        && s.width <= 4096
        && s.height <= 4096
        && u64::from(s.width) * u64::from(s.height) <= 8_388_608
}
fn set_rates(d: &mut CaptureDemand, s: &SubscriptionRequest) {
    d.width = s.width;
    d.height = s.height;
    d.fps = s.fps;
    d.bitrate_kbps = s.bitrate_kbps;
}
async fn reply(sender: &UdpSender, addr: SocketAddr, command: SessionCommand) {
    let _ = sender
        .send_control(&ControlMessage::Session(Box::new(command)), addr)
        .await;
}
impl Peer {
    /// End media and input without dropping authenticated file workers. The
    /// delivery_owner tombstone still prevents resubscription after revocation.
    fn retire_media(&mut self, input: &Mutex<crate::publisher_input::PublisherInput>, demand: &Mutex<CaptureDemand>) -> Option<u32> {
        input.lock().unwrap().revoke_peer(self.addr);
        let subscription = self.subscription.take()?;
        self.retired.insert(subscription.id);
        *demand.lock().unwrap() = CaptureDemand::default();
        Some(subscription.id)
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
            subscription: None,
            retired: Default::default(),
            activity_revision: 0,
            settings_revision: 0,
            video_sequence: 0,
            audio_sequence: 0,
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
        peer.subscription = Some(SubscriptionRequest { id: 256, source: CaptureSource::MainDisplay,
            width: 720, height: 1280, fps: 30, bitrate_kbps: 4000, audio: false });
        let owner = DeliveryPeer { addr, connection: 9, nonce: [1;16], scope: Some([2;32]) };
        peer.delivery_owner = Some((owner,[3;32]));
        let files = peer.files.clone().unwrap();
        let input = Mutex::new(crate::publisher_input::PublisherInput::default());
        let demand = Mutex::new(CaptureDemand {active:true,..Default::default()});
        assert_eq!(peer.retire_media(&input, &demand), Some(256));
        assert_eq!(peer.retire_media(&input, &demand), None);
        assert!(peer.subscription.is_none());
        assert!(peer.retired.contains(&256));
        assert_eq!(peer.delivery_owner, Some((owner,[3;32])));
        assert!(files.same_channel(peer.files.as_ref().unwrap()) && !files.is_closed());
        assert!(peer.tasks.iter().all(|task|!task.is_finished()));
        assert!(!demand.lock().unwrap().active);
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
                max_subscriptions: 1,
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
        publisher.frame(vec![0, 0, 0, 1, 0x26, 0x01, 1, 2, 3], 0, false);
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
        publisher.set_source_size(1920,1080);
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
        publisher.frame(vec![1, 2, 3], 0, false);
        publisher.frame(vec![1, 2, 3], 0, true);
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
        assert!(matches!(reply(&clients[1].1).await,SessionCommand::Error {request_id:256,..}));
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
        publisher.frame(vec![0,0,0,1,0x26,1,9],0,false);
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
        let old_epoch = publisher.permission.borrow().epoch;
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
                epoch: old_epoch,
                bytes: vec![9, 9, 9],
                timestamp: 0,
                audio: false,
            })
            .await
            .unwrap();
        publisher.frame(vec![0, 0, 0, 1, 0x26, 1, 7], 0, false);
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
