//! Bounded, in-memory directed intent. Wire identity always comes from the
//! authenticated host connection; a ticket is not a node-ID assertion.
use protocol::session::{CaptureSource, DeliveryState, SessionCommand, validate_delivery_sources};
use crate::shared_files::ShareScope;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use protocol::session::DeliveryClaimProof;
use crate::workspace_session::{WorkspaceConnection,WorkspaceEvents};
type Error=Box<dyn std::error::Error+Send+Sync>;

/// Scalar failure evidence. Queue admission is not a claim of wire delivery.
#[derive(Clone, Debug, Serialize)]
pub struct ConfirmationTimeout {
    pub connection_id: u32,
    pub target: std::net::SocketAddr,
    pub request_id: u64,
    pub revision: u64,
    pub queued_commands: u32,
    pub control_queue_depth: usize,
    pub peer_response_age_ms: u64,
    pub peer_responsive: bool,
}
impl std::fmt::Display for ConfirmationTimeout {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(if self.peer_responsive {
            "Device response timed out"
        } else {
            "Connection lost"
        })
    }
}
impl std::error::Error for ConfirmationTimeout {}
fn record_confirmation_timeout(timeout: &ConfirmationTimeout) {
    let message = serde_json::to_string(timeout).expect("Scalar timeout diagnostics");
    #[cfg(target_os = "android")]
    {
        // Native stderr is normally discarded in an APK. Use Android's log so
        // a failed real-device run remains inspectable without another build.
        #[link(name = "log")]
        unsafe extern "C" {
            fn __android_log_write(priority: i32, tag: *const std::ffi::c_char,
                message: *const std::ffi::c_char) -> i32;
        }
        let message = std::ffi::CString::new(message).expect("Scalar JSON has no NUL");
        unsafe { __android_log_write(6, c"WeaveloDelivery".as_ptr(), message.as_ptr()); }
    }
    #[cfg(not(target_os = "android"))]
    eprintln!("WEAVELO_DELIVERY_CONFIRMATION_TIMEOUT {message}");
}

/// Retry the exact command/revision; newer revisions never substitute for a
/// lost first offer. Only this connection's matching receipt completes it.
pub async fn exchange(
    conn: &WorkspaceConnection,
    events: &mut WorkspaceEvents,
    command: SessionCommand,
    request: u64,
    revision: u64,
) -> Result<DeliveryState, Error> {
    // Match ordinary session confirmation tolerance. Four lost UDP receipts
    // used to end an otherwise live 15-second delivery lease after only 3s.
    let mut queued_commands = 0;
    let confirmation = async {
        loop {
            conn.control(command.clone()).await?;
            queued_commands += 1;
            let deadline = tokio::time::Instant::now() + Duration::from_millis(750);
            loop {
                match tokio::time::timeout_at(deadline, events.control.recv()).await {
                    Ok(Some(SessionCommand::DeliveryReceipt {
                        request_id,
                        revision: got,
                        state,
                        reason,
                    })) if request_id == request
                        && (got == revision
                            || matches!(state, DeliveryState::Cancelled | DeliveryState::Rejected)) =>
                    {
                        if state == DeliveryState::Rejected {
                            return Err(format!("Target rejected delivery: {reason}").into());
                        }
                        return Ok(state);
                    }
                    Ok(Some(SessionCommand::Closed { .. })) | Ok(None) => {
                        return Err("Control connection closed".into());
                    }
                    Ok(Some(_)) => continue,
                    Err(_) => break,
                }
            }
        }
    };
    let confirmed = tokio::time::timeout(Duration::from_secs(8), confirmation).await;
    confirmed.map_err(|_| -> Error {
        let timeout = ConfirmationTimeout { connection_id: conn.id, target: conn.target,
            request_id: request, revision, queued_commands,
            control_queue_depth: conn.control_queue_depth(),
            peer_response_age_ms: conn.peer_response_age_ms(), peer_responsive: conn.peer_is_responsive() };
        record_confirmation_timeout(&timeout);
        Box::new(timeout)
    })?
}
pub async fn claim(
    conn: &WorkspaceConnection,
    events: &mut WorkspaceEvents,
    proof: &DeliveryClaimProof,
) -> Result<(), Error> {
    validate_delivery_sources(&proof.sources)?;
    let state = exchange(
        conn,
        events,
        SessionCommand::DeliveryClaim {
            request_id: proof.request_id,
            target_device_id: proof.target_device_id.clone(),
            ticket: proof.ticket,
            sources: proof.sources.clone(),
        },
        proof.request_id,
        0,
    )
    .await?;
    if state != DeliveryState::Claimed {
        return Err("Source did not accept this sending intention".into());
    }
    Ok(())
}

pub struct OutgoingOffer {
    pub source: String,
    pub target: String,
    pub ticket: [u8; 32],
    pub request: u64,
    pub sources: Vec<CaptureSource>,
    pub scope: ShareScope,
}

/// Desktop and mobile use the same retry, revision, expiry and receipt rules.
/// The source adapter owns registration/revocation and its native capture grant.
pub async fn maintain_offer<R, F, A, S>(
    conn: &WorkspaceConnection, events: &mut WorkspaceEvents, offer: OutgoingOffer,
    cancelled: &mut tokio::sync::watch::Receiver<bool>, alive: A, mut renew: R, mut status: S,
) -> Result<(), Error>
where
    R: FnMut(u64) -> F,
    F: std::future::Future<Output = Result<(), Error>>,
    A: Fn() -> bool,
    S: FnMut(DeliveryState),
{
    validate_delivery_sources(&offer.sources)?;
    let cancellation = cancelled.clone();
    let sending = async {
        let mut revision = 1;
        loop {
            let next_offer = tokio::time::Instant::now() + Duration::from_secs(5);
            if *cancellation.borrow() { return Ok(()); }
            if !alive() || offer.scope != crate::shared_files::current_share_scope() {
                return Err("Screen sharing or device network changed".into());
            }
            if revision > 1 { renew(revision).await?; }
            let state = exchange(conn, events, SessionCommand::DeliveryOffer {
                request_id: offer.request, source_device_id: offer.source.clone(), target_device_id: offer.target.clone(),
                ticket: offer.ticket, revision, sources: offer.sources.clone(),
            }, offer.request, revision).await?;
            match state {
                DeliveryState::Pending | DeliveryState::Accepted | DeliveryState::Receiving => status(state),
                DeliveryState::Cancelled | DeliveryState::Rejected => return Ok(()),
                _ => return Err("Unexpected delivery receipt".into()),
            }
            // Receipt latency consumes this interval; never add another five
            // seconds after a delayed confirmation and drift past the lease.
            tokio::time::sleep_until(next_offer).await;
            revision = revision.checked_add(1).ok_or("Sending revision exhausted")?;
        }
    };
    tokio::select! {biased; _=cancelled.changed()=>Ok(()), result=sending=>result}
}

pub fn sending_status(state: DeliveryState) -> &'static str {
    match state {
        DeliveryState::Pending => "Opening receiver…",
        DeliveryState::Accepted => "Connecting…",
        DeliveryState::Receiving => "Sending",
        _ => "Sending stopped",
    }
}


pub const LEASE: Duration = Duration::from_secs(15);
const LIMIT: usize = 16;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Peer {
    pub addr: SocketAddr,
    pub connection: u32,
    pub nonce: [u8; 16],
    pub scope: ShareScope,
}
#[derive(Clone, Serialize, Deserialize)]
pub enum LocalIntent {
    Register {
        ticket: [u8; 32],
        target: String,
        generation: String,
        connection: u32,
        sequence: u64,
        sources: Vec<CaptureSource>,
    },
    Renew {
        ticket: [u8; 32],
        generation: String,
        revision: u64,
    },
    Revoke {
        ticket: [u8; 32],
        generation: String,
    },
}
struct Ticket {
    target: String,
    generation: String,
    scope: ShareScope,
    _connection: u32,
    revision: u64,
    expires: Instant,
    sources: Vec<CaptureSource>,
    claimed: HashMap<Peer, Vec<CaptureSource>>,
    cancel: tokio::sync::watch::Sender<bool>,
}
impl Drop for Ticket {
    fn drop(&mut self) {
        self.cancel.send_replace(true);
    }
}
struct State {
    identity: Option<String>,
    tickets: HashMap<[u8; 32], Ticket>,
    incoming: HashMap<(Peer, u64), Arc<Incoming>>,
    source_generation: Option<String>,
    registration_high_water: u64,
}
pub struct Broker {
    state: Mutex<State>,
    tx: mpsc::Sender<Arc<Incoming>>,
}
pub struct Incoming {
    pub peer: Peer,
    pub request: u64,
    pub source: String,
    pub target: String,
    pub ticket: [u8; 32],
    pub sources: Vec<CaptureSource>,
    inner: Mutex<IncomingState>,
}
struct IncomingState {
    revision: u64,
    expires: Instant,
    state: DeliveryState,
    reason: String,
    stage: &'static str,
}
impl Incoming {
    pub fn mark_receiver_stage(&self, stage: &'static str) {
        self.inner.lock().unwrap().stage=stage;
    }
    pub fn snapshot(&self) -> (u64, Instant, DeliveryState) {
        let s = self.inner.lock().unwrap();
        (s.revision, s.expires, s.state)
    }
    pub fn alive(&self) -> bool {
        let s = self.inner.lock().unwrap();
        Instant::now() < s.expires
            && matches!(
                s.state,
                DeliveryState::Pending | DeliveryState::Accepted | DeliveryState::Receiving
            )
    }
    pub fn finish(&self, state: DeliveryState, reason: &str) {
        let mut s = self.inner.lock().unwrap();
        if matches!(
            s.state,
            DeliveryState::Pending | DeliveryState::Accepted | DeliveryState::Receiving
        ) {
            s.state = state;
            s.reason = reason.into();
            if state == DeliveryState::Receiving { s.stage = "receiving"; }
        }
    }
    fn receipt(&self) -> SessionCommand {
        let s = self.inner.lock().unwrap();
        receipt(self.request, s.revision, s.state, &s.reason)
    }
}
fn receipt(request_id: u64, revision: u64, state: DeliveryState, reason: &str) -> SessionCommand {
    SessionCommand::DeliveryReceipt {
        request_id,
        revision,
        state,
        reason: reason.into(),
    }
}
impl Broker {
    pub fn new() -> (Arc<Self>, mpsc::Receiver<Arc<Incoming>>) {
        let (tx, rx) = mpsc::channel(8);
        let broker = Arc::new(Self {
            state: Mutex::new(State {
                identity: None,
                tickets: HashMap::new(),
                incoming: HashMap::new(),
                source_generation: None,
                registration_high_water: 0,
            }),
            tx,
        });
        let weak = Arc::downgrade(&broker);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let Some(broker) = weak.upgrade() else {
                    break;
                };
                broker.expire_tickets(
                    Instant::now(),
                    crate::shared_files::current_share_scope(),
                );
            }
        });
        (broker, rx)
    }
    fn expire_tickets(&self, now: Instant, scope: ShareScope) {
        let mut s = self.state.lock().unwrap();
        let expired = s
            .tickets
            .iter()
            .filter(|(_, t)| now >= t.expires || t.scope != scope)
            .map(|(k, _)| *k)
            .collect::<Vec<_>>();
        for k in expired {
            s.tickets.remove(&k);
        }
    }
    pub fn upgrade_busy(&self) -> bool {
        let state=self.state.lock().unwrap();
        !state.tickets.is_empty() || state.incoming.values().any(|offer|offer.alive())
    }
    /// Scalar same-user diagnostics. Tickets and private source paths stay private.
    pub fn diagnostics(&self) -> serde_json::Value {
        let state=self.state.lock().unwrap();
        let incoming=state.incoming.values().map(|offer| {
            let current=offer.inner.lock().unwrap();
            serde_json::json!({"connection":offer.peer.connection,"request":offer.request,"revision":current.revision,
                "state":current.state,"stage":current.stage,"reason":current.reason,
                "remaining_ms":current.expires.saturating_duration_since(Instant::now()).as_millis()})
        }).collect::<Vec<_>>();
        serde_json::json!({"incoming":incoming,"outgoing_tickets":state.tickets.len()})
    }
    pub fn set_identity(&self, id: String) {
        self.state.lock().unwrap().identity = Some(id);
    }
    /// The source adapter first verifies its live native capture authorization.
    /// Desktop additionally verifies the GUI generation/binary under its
    /// serialized registration lock; Android owns the MediaProjection epoch.
    pub fn verified_source_generation(&self, generation: &str) {
        let mut s = self.state.lock().unwrap();
        if s.source_generation.as_deref() != Some(generation) {
            s.tickets.clear(); // Drop cancels old generation's exact resources first.
            s.source_generation = Some(generation.to_owned());
            s.registration_high_water = 0;
        }
    }
    pub fn local(&self, command: LocalIntent) -> Result<(), String> {
        let mut s = self.state.lock().unwrap();
        let now = Instant::now();
        let scope = crate::shared_files::current_share_scope();
        match command {
            LocalIntent::Register {
                ticket,
                target,
                generation,
                connection,
                sequence,
                sources,
            } => {
                register(
                    &mut s, ticket, target, generation, connection, sequence, sources, scope, now,
                )?;
            }
            LocalIntent::Renew {
                ticket,
                generation,
                revision,
            } => {
                let t = s.tickets.get_mut(&ticket).ok_or("Directed intent ended")?;
                renew(t, &generation, revision, now, scope)?;
            }
            LocalIntent::Revoke { ticket, generation } => {
                if s.tickets
                    .get(&ticket)
                    .is_some_and(|t| t.generation == generation)
                {
                    s.tickets.remove(&ticket);
                }
            }
        }
        Ok(())
    }
    pub fn offer(
        &self,
        peer: Peer,
        request: u64,
        source: String,
        target: String,
        ticket: [u8; 32],
        revision: u64,
        sources: Vec<CaptureSource>,
    ) -> SessionCommand {
        let mut s = self.state.lock().unwrap();
        let key = (peer, request);
        if let Some(offer) = s.incoming.get(&key) {
            if offer.source != source || offer.target != target || offer.ticket != ticket || offer.sources != sources {
                return receipt(
                    request,
                    revision,
                    DeliveryState::Rejected,
                    "Request belongs to another intent",
                );
            }
            let mut state = offer.inner.lock().unwrap();
            if Instant::now() >= state.expires {
                state.state = DeliveryState::Cancelled;
            }
            if revision > state.revision
                && matches!(
                    state.state,
                    DeliveryState::Pending | DeliveryState::Accepted | DeliveryState::Receiving
                )
            {
                state.revision = revision;
                state.expires = Instant::now() + LEASE;
            }
            drop(state);
            return offer.receipt();
        }
        if peer.scope.is_none()
            || validate_delivery_sources(&sources).is_err()
            || s.identity.as_deref() != Some(&target)
            || source.is_empty()
            || source.len() > 256
            || ticket == [0; 32]
            || request == 0
            || revision != 1
            || s.incoming.len() >= LIMIT
            || s.incoming.values().any(|o| o.alive())
        {
            return receipt(
                request,
                revision,
                DeliveryState::Rejected,
                "Directed delivery unavailable or invalid target",
            );
        }
        let offer = Arc::new(Incoming {
            peer,
            request,
            source,
            target,
            ticket,
            sources,
            inner: Mutex::new(IncomingState {
                revision,
                expires: Instant::now() + LEASE,
                state: DeliveryState::Pending,
                reason: String::new(),
                stage:"queued",
            }),
        });
        if self.tx.try_send(offer.clone()).is_err() {
            return receipt(request, revision, DeliveryState::Rejected, "Receiver busy");
        }
        s.incoming.insert(key, offer);
        receipt(request, revision, DeliveryState::Pending, "")
    }
    pub fn claim(
        &self,
        peer: Peer,
        request: u64,
        target: &str,
        ticket: [u8; 32],
        sources: Vec<CaptureSource>,
    ) -> SessionCommand {
        self.claim_in_scope(
            peer,
            request,
            target,
            ticket,
            sources,
            crate::shared_files::current_share_scope(),
        )
    }
    fn claim_in_scope(
        &self,
        peer: Peer,
        request: u64,
        target: &str,
        ticket: [u8; 32],
        sources: Vec<CaptureSource>,
        scope: ShareScope,
    ) -> SessionCommand {
        let mut s = self.state.lock().unwrap();
        let accepted = peer.scope.is_some()
            && peer.scope == scope
            && validate_delivery_sources(&sources).is_ok()
            && s.tickets.get_mut(&ticket).is_some_and(|t| {
                if t.target != target
                    || t.scope != peer.scope
                    || Instant::now() >= t.expires
                    || sources.iter().any(|source|!t.sources.contains(source))
                    || t.claimed.get(&peer).is_some_and(|selected| *selected != sources)
                    || t.claimed.iter().any(|(p, selected)| *p != peer && selected.iter().any(|source|sources.contains(source)))
                {
                    return false;
                }
                t.claimed.insert(peer, sources);
                true
            });
        receipt(
            request,
            0,
            if accepted {
                DeliveryState::Claimed
            } else {
                DeliveryState::Rejected
            },
            if accepted {
                ""
            } else {
                "Sending intent is unavailable"
            },
        )
    }
    pub fn cancel(&self, peer: Peer, request: u64) -> SessionCommand {
        if let Some(o) = self.state.lock().unwrap().incoming.get(&(peer, request)) {
            o.finish(DeliveryState::Cancelled, "Sender stopped");
        }
        receipt(request, 0, DeliveryState::Cancelled, "")
    }
    /// A claimed connection stays bound even after its ticket is removed. The
    /// host stores that binding, so a revoked ticket cannot become an ordinary pull.
    pub fn claim_cancel(
        &self,
        peer: Peer,
        ticket: [u8; 32],
    ) -> Option<tokio::sync::watch::Receiver<bool>> {
        self.state
            .lock()
            .unwrap()
            .tickets
            .get(&ticket)
            .filter(|t| t.claimed.contains_key(&peer) && Instant::now() < t.expires)
            .map(|t| t.cancel.subscribe())
    }
    pub fn claim_alive(&self, peer: Peer, ticket: [u8; 32]) -> bool {
        self.state
            .lock()
            .unwrap()
            .tickets
            .get(&ticket)
            .is_some_and(|t| {
                t.claimed.contains_key(&peer)
                    && t.scope == crate::shared_files::current_share_scope()
                    && Instant::now() < t.expires
            })
    }
    pub fn allows_source(&self, peer: Peer, ticket: [u8; 32], source: CaptureSource) -> bool {
        self.allows_source_in_scope(peer, ticket, source, crate::shared_files::current_share_scope())
    }
    fn allows_source_in_scope(&self, peer: Peer, ticket: [u8; 32], source: CaptureSource, scope: ShareScope) -> bool {
        self.state.lock().unwrap().tickets.get(&ticket).is_some_and(|t| {
            t.claimed.get(&peer).is_some_and(|sources|sources.contains(&source))
                && t.scope == scope
                && Instant::now() < t.expires
        })
    }
    pub fn reconcile(&self, live: impl Fn(Peer) -> bool) {
        self.reconcile_in_scope(live, crate::shared_files::current_share_scope());
    }
    fn reconcile_in_scope(&self, live: impl Fn(Peer) -> bool, scope: ShareScope) {
        let mut s = self.state.lock().unwrap();
        let now = Instant::now();
        for o in s.incoming.values() {
            if !live(o.peer) || o.peer.scope != scope || !o.alive() {
                o.finish(
                    DeliveryState::Cancelled,
                    "Control connection or lease ended",
                );
            }
        }
        // Keep tombstones until the authenticated control connection ends. A
        // repeated Offer cannot resurrect a GUI lease after Stop or rejection.
        s.incoming.retain(|(p, _), _| live(*p));
        let expired = s
            .tickets
            .iter()
            .filter(|(_, t)| {
                now >= t.expires || t.scope != scope
            })
            .map(|(k, _)| *k)
            .collect::<Vec<_>>();
        for k in expired {
            s.tickets.remove(&k);
        }
    }
}

fn register(
    s: &mut State,
    ticket: [u8; 32],
    target: String,
    generation: String,
    connection: u32,
    sequence: u64,
    sources: Vec<CaptureSource>,
    scope: ShareScope,
    now: Instant,
) -> Result<(), String> {
    validate_delivery_sources(&sources)?;
    if s.identity.is_none()
        || scope.is_none()
        || ticket == [0; 32]
        || target.is_empty()
        || target.len() > 256
        || generation.is_empty()
        || generation.len() > 128
        || connection == 0
    {
        return Err("Invalid directed intent".into());
    }
    if s.source_generation.as_deref() != Some(&generation) || sequence <= s.registration_high_water
    {
        return Err("Sending window changed or registration was replayed".into());
    }
    if s.tickets.len() >= LIMIT || s.tickets.contains_key(&ticket) {
        return Err("Too many active deliveries".into());
    }
    s.registration_high_water = sequence;
    s.tickets.insert(
        ticket,
        Ticket {
            target,
            generation,
            scope,
            _connection: connection,
            revision: 1,
            expires: now + LEASE,
            sources,
            claimed: HashMap::new(),
            cancel: tokio::sync::watch::channel(false).0,
        },
    );
    Ok(())
}

fn renew(
    t: &mut Ticket,
    generation: &str,
    revision: u64,
    now: Instant,
    scope: ShareScope,
) -> Result<(), String> {
    if t.generation != generation || t.scope != scope || now >= t.expires || revision < t.revision {
        return Err("Directed intent expired or stale".into());
    }
    if revision > t.revision {
        t.revision = revision;
        t.expires = now + LEASE;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn missing_offer_receipts_report_queue_progress_and_live_heartbeat() {
        use crate::net::{MultiplexedPacket, UdpMultiplexer};
        use protocol::ControlMessage;
        use std::sync::atomic::{AtomicU32, Ordering};
        assert!(crate::session_crypto::load_session_psk().is_none());
        let server = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let target = server.local_addr().unwrap();
        let (sender, receiver) = server.split();
        let seen = Arc::new(AtomicU32::new(0));
        let received = seen.clone();
        let request = u64::MAX - 7;
        let peer = tokio::spawn(async move {
            loop {
                let (reply, addr) = match receiver.recv().await.unwrap() {
                    MultiplexedPacket::Control(ControlMessage::Session(command), addr) => match *command {
                        SessionCommand::Open { connection_id, version } => (ControlMessage::Session(Box::new(
                            SessionCommand::Opened { connection_id, version, max_subscriptions: 8,
                                files: false, clipboard: false, window_capture: false })), addr),
                        SessionCommand::DeliveryOffer { request_id, revision, .. } => {
                            assert_eq!((request_id, revision), (request, 1));
                            received.fetch_add(1, Ordering::SeqCst);
                            continue; // Lose receipts while normal heartbeat traffic still works.
                        }
                        _ => continue,
                    },
                    MultiplexedPacket::Control(ControlMessage::Ping { client_send_ts }, addr) => (
                        ControlMessage::Pong { client_send_ts, host_recv_ts: 0, host_send_ts: 0 }, addr),
                    _ => continue,
                };
                sender.send_control(&reply, addr).await.unwrap();
            }
        });
        let (connection, mut events) = WorkspaceConnection::connect_media(target).await.unwrap();
        let error = exchange(&connection, &mut events, SessionCommand::DeliveryOffer {
            request_id: request, source_device_id: "source".into(), target_device_id: "target".into(),
            ticket: [4; 32], revision: 1, sources: vec![CaptureSource::MainDisplay],
        }, request, 1).await.unwrap_err();
        let evidence = error.downcast_ref::<ConfirmationTimeout>().expect("typed timeout evidence");
        assert_eq!((evidence.connection_id, evidence.request_id, evidence.revision), (connection.id, request, 1));
        assert_eq!(evidence.target, target);
        assert!(evidence.queued_commands >= 8 && seen.load(Ordering::SeqCst) >= 8);
        assert_eq!(evidence.control_queue_depth, 0, "the socket actor drained the admitted offers");
        assert!(evidence.peer_response_age_ms <= 6_000);
        assert!(evidence.peer_responsive, "a missing receipt must not be reported as a dead heartbeat");
        assert_eq!(error.to_string(), "Device response timed out");
        let disconnected = ConfirmationTimeout { peer_responsive: false, ..evidence.clone() };
        assert_eq!(disconnected.to_string(), "Connection lost");
        connection.close().await;
        peer.abort();
    }
    #[tokio::test]
    async fn delivery_survives_lost_receipts_without_accepting_a_stale_revision() {
        use crate::net::{MultiplexedPacket, UdpMultiplexer};
        use protocol::ControlMessage;
        assert!(crate::session_crypto::load_session_psk().is_none(), "Use only the isolated synthetic peer");
        let server = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let target = server.local_addr().unwrap();
        let (sender, receiver) = server.split();
        let request = u64::MAX - 3;
        let peer = tokio::spawn(async move {
            let mut attempts = 0;
            let mut first_offer = None;
            loop {
                let mut finished = false;
                let reply = match receiver.recv().await.unwrap() {
                    MultiplexedPacket::Control(ControlMessage::Session(command), addr) => {
                        let reply = match *command {
                            SessionCommand::Open { connection_id, version } => SessionCommand::Opened {
                                connection_id, version, max_subscriptions: 8, files: false, clipboard: false, window_capture: false,
                            },
                            SessionCommand::DeliveryOffer { request_id, revision, sources, .. } => {
                                assert_eq!(request_id, request);
                                assert_eq!(sources, vec![CaptureSource::MainDisplay]);
                                if revision == 1 {
                                    first_offer.get_or_insert_with(tokio::time::Instant::now);
                                    attempts += 1;
                                    // Four lost current receipts must not terminate
                                    // this live lease or accept a stale revision.
                                    receipt(request, if attempts <= 4 { 0 } else { revision }, DeliveryState::Receiving, "")
                                } else {
                                    assert_eq!(revision, 2);
                                    finished = true;
                                    receipt(request, revision, DeliveryState::Cancelled, "Fixture receiver stopped")
                                }
                            }
                            _ => continue,
                        };
                        (ControlMessage::Session(Box::new(reply)), addr)
                    }
                    MultiplexedPacket::Control(ControlMessage::Ping { client_send_ts }, addr) => (
                        ControlMessage::Pong { client_send_ts, host_recv_ts: 0, host_send_ts: 0 }, addr),
                    _ => continue,
                };
                sender.send_control(&reply.0, reply.1).await.unwrap();
                if finished { break first_offer.unwrap().elapsed(); }
            }
        });
        let (connection, mut events) = WorkspaceConnection::connect_media(target).await.unwrap();
        let (_stop, mut cancelled) = tokio::sync::watch::channel(false);
        let mut progress = Vec::new();
        let mut renewals = Vec::new();
        tokio::time::timeout(Duration::from_secs(9), maintain_offer(&connection, &mut events,
            OutgoingOffer { source: "fixture-source".into(), target: "fixture-target".into(), ticket: [7;32],
                request, sources: vec![CaptureSource::MainDisplay], scope: crate::shared_files::current_share_scope() },
            &mut cancelled, ||true, |revision| { renewals.push(revision); async { Ok(()) } },
            |state|progress.push(state))).await.unwrap().unwrap();
        let elapsed = peer.await.unwrap();
        // Retrying for 3s consumes the 5s cadence, rather than adding a new 5s
        // sleep. Cancel remains terminal and renews only the exact next revision.
        assert!(elapsed >= Duration::from_secs(4) && elapsed < Duration::from_secs(7), "renewal drifted: {elapsed:?}");
        assert_eq!(progress, vec![DeliveryState::Receiving]);
        assert_eq!(renewals, vec![2]);
    }

    fn peer() -> Peer {
        Peer {
            addr: "127.0.0.1:1".parse().unwrap(),
            connection: 7,
            nonce: [3; 16],
            scope: Some([4; 32]),
        }
    }
    fn ticket() -> Ticket {
        Ticket {
            target: "receiver".into(),
            generation: "gui-1".into(),
            scope: peer().scope,
            _connection: 8,
            revision: 1,
            expires: Instant::now() + LEASE,
            sources: vec![CaptureSource::MainDisplay],
            claimed: HashMap::new(),
            cancel: tokio::sync::watch::channel(false).0,
        }
    }
    #[test]
    fn duplicate_revision_does_not_extend_and_expired_intent_cannot_revive() {
        let mut t = ticket();
        let old = t.expires;
        let now = Instant::now();
        renew(&mut t, "gui-1", 1, now, peer().scope).unwrap();
        assert_eq!(old, t.expires);
        assert!(renew(&mut t, "gui-2", 2, now, peer().scope).is_err());
        assert!(renew(&mut t, "gui-1", 2, now, Some([5; 32])).is_err());
        assert!(renew(&mut t, "gui-1", 2, old, peer().scope).is_err());
        renew(&mut t, "gui-1", 2, now, peer().scope).unwrap();
        assert_eq!(t.revision, 2);
    }
    #[tokio::test]
    async fn claim_binds_exact_peer_and_revocation_signals_without_host_poll() {
        let (b, _rx) = Broker::new();
        let token = [8; 32];
        b.state.lock().unwrap().tickets.insert(token, ticket());
        assert!(matches!(
            b.claim_in_scope(peer(), 1, "receiver", token, vec![CaptureSource::MainDisplay], peer().scope),
            SessionCommand::DeliveryReceipt {
                state: DeliveryState::Claimed,
                ..
            }
        ));
        let mut cancel = b.claim_cancel(peer(), token).unwrap();
        let mut other = peer();
        other.nonce = [9; 16];
        assert!(matches!(
            b.claim_in_scope(other, 1, "receiver", token, vec![CaptureSource::MainDisplay], peer().scope),
            SessionCommand::DeliveryReceipt {
                state: DeliveryState::Rejected,
                ..
            }
        ));
        assert!(b.claim_cancel(peer(), [9; 32]).is_none());
        b.local(LocalIntent::Revoke {
            ticket: token,
            generation: "gui-1".into(),
        })
        .unwrap();
        cancel.changed().await.unwrap();
        assert!(*cancel.borrow());
        assert!(b.claim_cancel(peer(), token).is_none());
    }
    #[tokio::test]
    async fn delivery_windows_have_distinct_sources_and_close_independently() {
        let (broker, _) = Broker::new();
        let token = [8; 32]; let first = peer();
        let mut second = first; second.connection += 1; second.nonce = [9; 16];
        let mut item = ticket(); item.sources = vec![CaptureSource::Window(11), CaptureSource::Display(22)];
        broker.state.lock().unwrap().tickets.insert(token, item);
        let claim = |peer, source| matches!(broker.claim_in_scope(peer, 1, "receiver", token, vec![source], first.scope),
            SessionCommand::DeliveryReceipt { state: DeliveryState::Claimed, .. });
        assert!(claim(first, CaptureSource::Window(11)));
        assert!(claim(first, CaptureSource::Window(11))); // Retry keeps the exact binding.
        assert!(!claim(first, CaptureSource::Display(22)));
        assert!(!claim(second, CaptureSource::Window(11)));
        assert!(!claim(second, CaptureSource::Window(33)));
        assert!(claim(second, CaptureSource::Display(22)));
        let mut cancel = broker.claim_cancel(second, token).unwrap();
        broker.reconcile_in_scope(|p| p == second, first.scope);
        assert!(!*cancel.borrow());
        assert!(broker.allows_source_in_scope(second, token, CaptureSource::Display(22), first.scope));
        assert!(!broker.allows_source_in_scope(second, token, CaptureSource::Window(11), first.scope));
        // A closed window's source cannot be reclaimed by another connection.
        let mut third = second; third.connection += 1;
        assert!(!claim(third, CaptureSource::Window(11)));
        broker.reconcile_in_scope(|_| true, Some([6; 32]));
        cancel.changed().await.unwrap(); assert!(*cancel.borrow());
    }
    #[tokio::test]
    async fn one_connection_can_claim_a_bounded_source_set_without_overlapping_other_windows() {
        let (broker,_)=Broker::new();
        let token=[8;32];let mut item=ticket();
        item.sources=vec![CaptureSource::Window(11),CaptureSource::Display(22),CaptureSource::Window(33)];
        broker.state.lock().unwrap().tickets.insert(token,item);
        let first=peer();let mut second=first;second.connection+=1;
        let claim=|peer,sources|matches!(broker.claim_in_scope(peer,1,"receiver",token,sources,peer.scope),SessionCommand::DeliveryReceipt{state:DeliveryState::Claimed,..});
        let selected=vec![CaptureSource::Window(11),CaptureSource::Display(22)];
        assert!(claim(first,selected.clone()));assert!(claim(first,selected));
        assert!(broker.allows_source_in_scope(first,token,CaptureSource::Window(11),first.scope));
        assert!(broker.allows_source_in_scope(first,token,CaptureSource::Display(22),first.scope));
        assert!(!claim(first,vec![CaptureSource::Window(11)]));
        assert!(!claim(second,vec![CaptureSource::Display(22)]));
        assert!(!claim(second,vec![]));assert!(!claim(second,vec![CaptureSource::Window(33);2]));
        assert!(claim(second,vec![CaptureSource::Window(33)]));
        assert!(!broker.allows_source_in_scope(second,token,CaptureSource::Window(11),second.scope));
    }
    #[tokio::test]
    async fn offer_renewal_cannot_replace_its_source_selection() {
        let (broker, mut rx) = Broker::new(); broker.set_identity("receiver".into());
        let offer = |revision, sources| broker.offer(peer(), 1, "source".into(), "receiver".into(), [8;32], revision, sources);
        assert!(matches!(offer(1, vec![]), SessionCommand::DeliveryReceipt {state:DeliveryState::Rejected,..}));
        assert!(matches!(offer(1, vec![CaptureSource::Window(1);2]), SessionCommand::DeliveryReceipt {state:DeliveryState::Rejected,..}));
        assert!(matches!(offer(1, vec![CaptureSource::Window(1),CaptureSource::Window(2)]), SessionCommand::DeliveryReceipt {state:DeliveryState::Pending,..}));
        let incoming=rx.recv().await.unwrap(); let deadline=incoming.snapshot().1;
        assert!(matches!(offer(2, vec![CaptureSource::Window(2),CaptureSource::Window(1)]), SessionCommand::DeliveryReceipt {state:DeliveryState::Rejected,..}));
        assert_eq!(incoming.snapshot().1, deadline); assert_eq!(incoming.snapshot().0, 1);
        assert!(incoming.alive()); assert!(rx.try_recv().is_err());
    }
    #[tokio::test]
    async fn terminal_offer_is_not_resurrected_by_retry_or_new_revision() {
        let (b, mut rx) = Broker::new();
        b.set_identity("receiver".into());
        let offer = |revision| {
            b.offer(
                peer(),
                1,
                "source".into(),
                "receiver".into(),
                [8; 32],
                revision,
                vec![CaptureSource::MainDisplay],
            )
        };
        assert!(matches!(
            offer(2),
            SessionCommand::DeliveryReceipt {
                state: DeliveryState::Rejected,
                ..
            }
        ));
        assert!(matches!(
            offer(1),
            SessionCommand::DeliveryReceipt {
                state: DeliveryState::Pending,
                ..
            }
        ));
        let incoming = rx.recv().await.unwrap();
        let expires = incoming.snapshot().1;
        offer(1);
        assert_eq!(incoming.snapshot().1, expires);
        incoming.finish(DeliveryState::Cancelled, "Receiver stopped");
        assert!(matches!(
            offer(2),
            SessionCommand::DeliveryReceipt {
                state: DeliveryState::Cancelled,
                ..
            }
        ));
        assert!(!incoming.alive());
        assert!(rx.try_recv().is_err());
    }
    #[tokio::test]
    async fn source_deadline_cancels_claim_even_without_network_loop() {
        let (b, _rx) = Broker::new();
        let mut t = ticket();
        t.scope = crate::shared_files::current_share_scope();
        t.expires = Instant::now() + Duration::from_millis(10);
        let mut cancel = t.cancel.subscribe();
        b.state.lock().unwrap().tickets.insert([8; 32], t);
        tokio::time::timeout(Duration::from_secs(1), cancel.changed())
            .await
            .unwrap()
            .unwrap();
        assert!(*cancel.borrow());
    }
    #[tokio::test]
    async fn another_offer_cannot_displace_current_receiver() {
        let (b, mut rx) = Broker::new();
        b.set_identity("receiver".into());
        b.offer(peer(), 1, "source".into(), "receiver".into(), [8; 32], 1, vec![CaptureSource::MainDisplay]);
        let first = rx.recv().await.unwrap();
        assert!(matches!(
            b.offer(peer(), 2, "other".into(), "receiver".into(), [9; 32], 1, vec![CaptureSource::MainDisplay]),
            SessionCommand::DeliveryReceipt {
                state: DeliveryState::Rejected,
                ..
            }
        ));
        assert!(first.alive());
        let mut other = peer();
        other.connection += 1;
        b.cancel(other, 1);
        assert!(first.alive());
    }
    #[tokio::test]
    async fn registration_history_is_bounded_without_a_lifetime_delivery_limit() {
        let (b, _) = Broker::new();
        b.set_identity("source".into());
        b.verified_source_generation("gui-1");
        for sequence in 1u64..=1024 {
            let mut token = [8; 32];
            token[..8].copy_from_slice(&sequence.to_le_bytes());
            register(
                &mut b.state.lock().unwrap(),
                token,
                "receiver".into(),
                "gui-1".into(),
                8,
                sequence,
                vec![CaptureSource::MainDisplay],
                peer().scope,
                Instant::now(),
            )
            .unwrap();
            b.local(LocalIntent::Revoke {
                ticket: token,
                generation: "gui-1".into(),
            })
            .unwrap();
        }
        assert!(b.state.lock().unwrap().tickets.is_empty());
        assert_eq!(b.state.lock().unwrap().registration_high_water, 1024);
        assert!(
            register(
                &mut b.state.lock().unwrap(),
                [9; 32],
                "receiver".into(),
                "gui-1".into(),
                8,
                1,
                vec![CaptureSource::MainDisplay],
                peer().scope,
                Instant::now()
            )
            .is_err()
        );
        b.verified_source_generation("gui-2");
        assert!(
            register(
                &mut b.state.lock().unwrap(),
                [9; 32],
                "receiver".into(),
                "gui-1".into(),
                8,
                1025,
                vec![CaptureSource::MainDisplay],
                peer().scope,
                Instant::now()
            )
            .is_err()
        );
        register(
            &mut b.state.lock().unwrap(),
            [9; 32],
            "receiver".into(),
            "gui-2".into(),
            8,
            1,
            vec![CaptureSource::MainDisplay],
            peer().scope,
            Instant::now(),
        )
        .unwrap();
    }
}
