use crate::relay_status::{
    RelayFailure, RelayStage, RelayTunnelDiagnostics, StopObservation, io_failure, tls_io_failure,
    websocket_failure,
};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as SyncMutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UdpSocket;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio::sync::{Mutex, Semaphore, mpsc, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::{WebSocketStream, accept_async_with_config};

const RELAY_MAGIC: &[u8; 4] = b"RPR1";
const REGISTER_PACKET: u8 = 1;
const DATA_PACKET: u8 = 2;
const MAX_ID_LEN: usize = 255;
const MAX_PACKET_LEN: usize = 65_535;
const MAX_TCP_FRAME_LEN: usize = RELAY_MAGIC.len() + 3 + (2 * MAX_ID_LEN) + MAX_PACKET_LEN;
const DEFAULT_MAX_TCP_CONNECTIONS: usize = 128;
const DEFAULT_MAX_PEERS_PER_GROUP: usize = 8;
const DEFAULT_TCP_WRITER_QUEUE_CAPACITY: usize = 512;
const TCP_WRITER_STALL_TIMEOUT: Duration = Duration::from_secs(8);
const DEFAULT_TCP_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const WEBSOCKET_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const LAN_FAST_CONNECT_TIMEOUT: Duration = Duration::from_millis(800);
const UDP_PUBLIC_HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(1_500);
const UDP_HANDSHAKE_RETRY_INTERVAL: Duration = Duration::from_millis(250);
const UDP_DEFAULT_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(12);
const UDP_PEER_PRUNE_INTERVAL: Duration = Duration::from_secs(1);
const DEFAULT_WEBSOCKET_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const DEFAULT_WEBSOCKET_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(25);
const WEBSOCKET_SEND_BATCH_LIMIT: usize = 32;
const WEBSOCKET_RECV_YIELD_INTERVAL: usize = 16;

const RELAY_SCALE_FULL_PERMILLE: u32 = 1_000;
const RELAY_SCALE_MIN_PERMILLE: u32 = 250;
const RELAY_PRESSURE_IDLE_RESET_US: u64 = 3_000_000;

static RELAY_OUTBOUND_SCALE_PERMILLE: AtomicU32 = AtomicU32::new(RELAY_SCALE_FULL_PERMILLE);
static RELAY_OUTBOUND_LAST_UPDATE_US: AtomicU64 = AtomicU64::new(0);

/// Instant upstream backpressure signal observed by the local WebSocket relay tunnel.
/// Returns `1000` (100%) when healthy/idle and smoothly scales down to `250` (25%)
/// when TCP `flush()` stalls indicate outbound relay congestion, preventing the
/// 2 MB local UDP tunnel buffer from overflowing and dropping video fragments.
pub fn relay_outbound_congestion_scale_permille() -> u32 {
    let last_us = RELAY_OUTBOUND_LAST_UPDATE_US.load(Ordering::Relaxed);
    if last_us == 0 {
        return RELAY_SCALE_FULL_PERMILLE;
    }
    let now_us = crate::timing::quanta_now_us();
    if now_us.saturating_sub(last_us) >= RELAY_PRESSURE_IDLE_RESET_US {
        RELAY_OUTBOUND_SCALE_PERMILLE.store(RELAY_SCALE_FULL_PERMILLE, Ordering::Relaxed);
        return RELAY_SCALE_FULL_PERMILLE;
    }
    RELAY_OUTBOUND_SCALE_PERMILLE
        .load(Ordering::Relaxed)
        .clamp(RELAY_SCALE_MIN_PERMILLE, RELAY_SCALE_FULL_PERMILLE)
}

fn record_relay_outbound_flush(elapsed: Duration, _batched: usize) {
    let now_us = crate::timing::quanta_now_us();
    let current = RELAY_OUTBOUND_SCALE_PERMILLE.load(Ordering::Relaxed);
    if elapsed >= Duration::from_millis(80) {
        let next = ((u64::from(current) * 72) / 100).max(u64::from(RELAY_SCALE_MIN_PERMILLE)) as u32;
        RELAY_OUTBOUND_SCALE_PERMILLE.store(next, Ordering::Relaxed);
        RELAY_OUTBOUND_LAST_UPDATE_US.store(now_us, Ordering::Relaxed);
    } else if elapsed >= Duration::from_millis(35) {
        let next = ((u64::from(current) * 85) / 100).max(u64::from(RELAY_SCALE_MIN_PERMILLE)) as u32;
        RELAY_OUTBOUND_SCALE_PERMILLE.store(next, Ordering::Relaxed);
        RELAY_OUTBOUND_LAST_UPDATE_US.store(now_us, Ordering::Relaxed);
    } else if elapsed <= Duration::from_millis(12) && current < RELAY_SCALE_FULL_PERMILLE {
        let last_us = RELAY_OUTBOUND_LAST_UPDATE_US.load(Ordering::Relaxed);
        if now_us.saturating_sub(last_us) >= 600_000 {
            let next = current.saturating_add(60).min(RELAY_SCALE_FULL_PERMILLE);
            RELAY_OUTBOUND_SCALE_PERMILLE.store(next, Ordering::Relaxed);
            RELAY_OUTBOUND_LAST_UPDATE_US.store(now_us, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
pub(crate) fn reset_relay_outbound_congestion_for_tests() {
    RELAY_OUTBOUND_SCALE_PERMILLE.store(RELAY_SCALE_FULL_PERMILLE, Ordering::Relaxed);
    RELAY_OUTBOUND_LAST_UPDATE_US.store(0, Ordering::Relaxed);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdpRelayServerConfig {
    pub bind_addr: SocketAddr,
    pub peer_ttl: Duration,
    pub log_events: bool,
}

impl UdpRelayServerConfig {
    pub fn new(bind_addr: SocketAddr) -> Self {
        Self {
            bind_addr,
            peer_ttl: Duration::from_secs(15),
            log_events: false,
        }
    }

    pub fn with_event_logging(mut self, log_events: bool) -> Self {
        self.log_events = log_events;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdpRelayTunnelConfig {
    pub bind_addr: SocketAddr,
    pub relay_addr: SocketAddr,
    pub group_id: String,
    pub peer_id: String,
    pub local_target_addr: Option<SocketAddr>,
    pub register_interval: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TcpRelayServerConfig {
    pub bind_addr: SocketAddr,
    pub log_events: bool,
    pub max_connections: usize,
    pub max_peers_per_group: usize,
    pub writer_queue_capacity: usize,
    pub idle_timeout: Duration,
}

impl TcpRelayServerConfig {
    pub fn new(bind_addr: SocketAddr) -> Self {
        Self {
            bind_addr,
            log_events: false,
            max_connections: DEFAULT_MAX_TCP_CONNECTIONS,
            max_peers_per_group: DEFAULT_MAX_PEERS_PER_GROUP,
            writer_queue_capacity: DEFAULT_TCP_WRITER_QUEUE_CAPACITY,
            idle_timeout: DEFAULT_TCP_IDLE_TIMEOUT,
        }
    }

    pub fn with_event_logging(mut self, log_events: bool) -> Self {
        self.log_events = log_events;
        self
    }

    pub fn with_limits(
        mut self,
        max_connections: usize,
        max_peers_per_group: usize,
        writer_queue_capacity: usize,
    ) -> Self {
        self.max_connections = max_connections.max(1);
        self.max_peers_per_group = max_peers_per_group.max(1);
        self.writer_queue_capacity = writer_queue_capacity.max(1);
        self
    }

    pub fn with_idle_timeout(mut self, idle_timeout: Duration) -> Self {
        self.idle_timeout = idle_timeout.max(Duration::from_secs(1));
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TcpRelayTunnelConfig {
    pub bind_addr: SocketAddr,
    pub relay_addr: SocketAddr,
    pub group_id: String,
    pub peer_id: String,
    pub local_target_addr: Option<SocketAddr>,
    pub register_interval: Duration,
    pub log_events: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSocketRelayTunnelConfig {
    pub bind_addr: SocketAddr,
    pub relay_url: String,
    pub fallback_relay_urls: Vec<String>,
    pub group_id: String,
    pub peer_id: String,
    pub local_target_addr: Option<SocketAddr>,
    pub register_interval: Duration,
    pub heartbeat_interval: Duration,
    pub heartbeat_timeout: Duration,
    pub log_events: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HybridRelayCandidate {
    Udp(String),
    WebSocket(String),
}

impl HybridRelayCandidate {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Udp(url) | Self::WebSocket(url) => url.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HybridRelayTunnelConfig {
    pub bind_addr: SocketAddr,
    pub candidates: Vec<HybridRelayCandidate>,
    pub group_id: String,
    pub peer_id: String,
    pub local_target_addr: Option<SocketAddr>,
    pub register_interval: Duration,
    pub heartbeat_interval: Duration,
    pub heartbeat_timeout: Duration,
    pub log_events: bool,
}

impl TcpRelayTunnelConfig {
    pub fn new(
        bind_addr: SocketAddr,
        relay_addr: SocketAddr,
        group_id: impl Into<String>,
        peer_id: impl Into<String>,
    ) -> Result<Self, RelayConfigError> {
        let group_id = validate_relay_id("group_id", group_id.into())?;
        let peer_id = validate_relay_id("peer_id", peer_id.into())?;
        Ok(Self {
            bind_addr,
            relay_addr,
            group_id,
            peer_id,
            local_target_addr: None,
            register_interval: Duration::from_secs(1),
            log_events: false,
        })
    }

    pub fn with_local_target_addr(mut self, local_target_addr: SocketAddr) -> Self {
        self.local_target_addr = Some(local_target_addr);
        self
    }

    pub fn with_event_logging(mut self, log_events: bool) -> Self {
        self.log_events = log_events;
        self
    }
}

impl WebSocketRelayTunnelConfig {
    pub fn new(
        bind_addr: SocketAddr,
        relay_url: impl Into<String>,
        group_id: impl Into<String>,
        peer_id: impl Into<String>,
    ) -> Result<Self, RelayConfigError> {
        let raw = relay_url.into();
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(RelayConfigError::EmptyId("relay_url"));
        }
        let mut urls = Vec::new();
        for part in trimmed.split(',') {
            let validated = validate_websocket_relay_url(part.to_string())?;
            if !urls.contains(&validated) {
                urls.push(validated);
            }
        }
        let relay_url = urls.remove(0);
        let group_id = validate_relay_id("group_id", group_id.into())?;
        let peer_id = validate_relay_id("peer_id", peer_id.into())?;
        Ok(Self {
            bind_addr,
            relay_url,
            fallback_relay_urls: urls,
            group_id,
            peer_id,
            local_target_addr: None,
            register_interval: Duration::from_secs(1),
            heartbeat_interval: DEFAULT_WEBSOCKET_HEARTBEAT_INTERVAL,
            heartbeat_timeout: DEFAULT_WEBSOCKET_HEARTBEAT_TIMEOUT,
            log_events: false,
        })
    }

    pub fn with_fallback_relay_urls<I, S>(mut self, urls: I) -> Result<Self, RelayConfigError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for raw in urls {
            let validated = validate_websocket_relay_url(raw.into())?;
            if validated != self.relay_url && !self.fallback_relay_urls.contains(&validated) {
                self.fallback_relay_urls.push(validated);
            }
        }
        Ok(self)
    }

    pub fn candidate_relay_urls(&self) -> Vec<&str> {
        let mut urls = Vec::with_capacity(1 + self.fallback_relay_urls.len());
        urls.push(self.relay_url.as_str());
        for url in &self.fallback_relay_urls {
            urls.push(url.as_str());
        }
        urls
    }

    pub fn with_local_target_addr(mut self, local_target_addr: SocketAddr) -> Self {
        self.local_target_addr = Some(local_target_addr);
        self
    }

    pub fn with_event_logging(mut self, log_events: bool) -> Self {
        self.log_events = log_events;
        self
    }

    pub fn with_heartbeat(
        mut self,
        heartbeat_interval: Duration,
        heartbeat_timeout: Duration,
    ) -> Self {
        self.heartbeat_interval = heartbeat_interval.max(Duration::from_millis(10));
        self.heartbeat_timeout = heartbeat_timeout.max(self.heartbeat_interval);
        self
    }
}

impl HybridRelayTunnelConfig {
    pub fn new(
        bind_addr: SocketAddr,
        relay_urls: impl Into<String>,
        group_id: impl Into<String>,
        peer_id: impl Into<String>,
    ) -> Result<Self, RelayConfigError> {
        let raw = relay_urls.into();
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(RelayConfigError::EmptyId("relay_url"));
        }
        let mut candidates = Vec::new();
        for part in trimmed.split(',') {
            let part_trimmed = part.trim();
            let candidate = if part_trimmed.starts_with("udp://") {
                HybridRelayCandidate::Udp(validate_udp_relay_url(part_trimmed.to_string())?)
            } else if part_trimmed.starts_with("ws://") || part_trimmed.starts_with("wss://") {
                HybridRelayCandidate::WebSocket(validate_websocket_relay_url(
                    part_trimmed.to_string(),
                )?)
            } else {
                return Err(RelayConfigError::InvalidRelayUrl(part_trimmed.to_string()));
            };
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
        if candidates.is_empty() {
            return Err(RelayConfigError::EmptyId("relay_url"));
        }
        let group_id = validate_relay_id("group_id", group_id.into())?;
        let peer_id = validate_relay_id("peer_id", peer_id.into())?;
        Ok(Self {
            bind_addr,
            candidates,
            group_id,
            peer_id,
            local_target_addr: None,
            register_interval: Duration::from_secs(1),
            heartbeat_interval: DEFAULT_WEBSOCKET_HEARTBEAT_INTERVAL,
            heartbeat_timeout: DEFAULT_WEBSOCKET_HEARTBEAT_TIMEOUT,
            log_events: false,
        })
    }

    pub fn with_local_target_addr(mut self, local_target_addr: SocketAddr) -> Self {
        self.local_target_addr = Some(local_target_addr);
        self
    }

    pub fn with_event_logging(mut self, log_events: bool) -> Self {
        self.log_events = log_events;
        self
    }

    pub fn with_heartbeat(
        mut self,
        heartbeat_interval: Duration,
        heartbeat_timeout: Duration,
    ) -> Self {
        self.heartbeat_interval = heartbeat_interval.max(Duration::from_millis(10));
        self.heartbeat_timeout = heartbeat_timeout.max(self.heartbeat_interval);
        self
    }
}

impl UdpRelayTunnelConfig {
    pub fn new(
        bind_addr: SocketAddr,
        relay_addr: SocketAddr,
        group_id: impl Into<String>,
        peer_id: impl Into<String>,
    ) -> Result<Self, RelayConfigError> {
        let group_id = validate_relay_id("group_id", group_id.into())?;
        let peer_id = validate_relay_id("peer_id", peer_id.into())?;
        Ok(Self {
            bind_addr,
            relay_addr,
            group_id,
            peer_id,
            local_target_addr: None,
            register_interval: Duration::from_secs(1),
        })
    }

    pub fn with_local_target_addr(mut self, local_target_addr: SocketAddr) -> Self {
        self.local_target_addr = Some(local_target_addr);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayConfigError {
    EmptyId(&'static str),
    IdTooLong { field: &'static str, len: usize },
    InvalidWebSocketUrl(String),
    InvalidRelayUrl(String),
}

impl std::fmt::Display for RelayConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyId(field) => write!(f, "{field} is empty"),
            Self::IdTooLong { field, len } => {
                write!(f, "{field} is too long: {len} bytes, max {MAX_ID_LEN}")
            }
            Self::InvalidWebSocketUrl(value) => {
                write!(f, "invalid relay WebSocket URL {value:?}")
            }
            Self::InvalidRelayUrl(value) => {
                write!(f, "invalid relay URL {value:?}")
            }
        }
    }
}

impl std::error::Error for RelayConfigError {}

pub fn derive_relay_group_id(
    network_name: &str,
    network_secret: &str,
    channel: &str,
) -> Result<String, RelayConfigError> {
    let network_name = validate_relay_id("network_name", network_name.to_string())?;
    let channel = validate_relay_id("channel", channel.to_string())?;
    let network_secret = network_secret.trim();
    if network_secret.is_empty() {
        return Err(RelayConfigError::EmptyId("network_secret"));
    }

    let mut mac = Hmac::<Sha256>::new_from_slice(network_secret.as_bytes())
        .expect("HMAC accepts keys of any size");
    mac.update(b"remote-play-relay-group-v1\0");
    mac.update(network_name.as_bytes());
    mac.update(b"\0");
    mac.update(channel.as_bytes());
    let bytes = mac.finalize().into_bytes();
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(encoded)
}

pub struct BoundUdpRelayServer {
    socket: UdpSocket,
    config: UdpRelayServerConfig,
}

impl BoundUdpRelayServer {
    pub async fn bind(config: UdpRelayServerConfig) -> io::Result<Self> {
        let socket = UdpSocket::bind(config.bind_addr).await?;
        crate::net::configure_udp_buffers(&socket);
        Ok(Self { socket, config })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    pub async fn run(self, mut cancel_rx: broadcast::Receiver<()>) -> io::Result<()> {
        let mut peers = RelayPeerTable::new(self.config.peer_ttl);
        let mut buf = vec![0u8; MAX_PACKET_LEN];
        let mut last_prune = Instant::now();

        loop {
            tokio::select! {
                _ = cancel_rx.recv() => return Ok(()),
                received = self.socket.recv_from(&mut buf) => {
                    let (len, addr) = match received {
                        Ok(pair) => pair,
                        Err(err) if is_transient_udp_recv_error(&err) => continue,
                        Err(err) => return Err(err),
                    };
                    if last_prune.elapsed() >= UDP_PEER_PRUNE_INTERVAL {
                        peers.prune_expired();
                        last_prune = Instant::now();
                    }
                    let Some(packet) = RelayPacketView::decode(&buf[..len]) else {
                        continue;
                    };
                    match packet {
                        RelayPacketView::Register { group_id, peer_id } => {
                            if self.config.log_events {
                                println!("Relay register group={group_id} peer={peer_id} addr={addr}");
                            }
                            peers.touch_or_register(group_id, peer_id, addr);
                            // Echo the registration packet so UDP clients can verify reachability
                            // and fail over quickly when UDP is blocked or unreachable.
                            let _ = self.socket.send_to(&buf[..len], addr).await;
                        }
                        RelayPacketView::Data { group_id, peer_id, payload } => {
                            peers.touch_or_register(group_id, peer_id, addr);
                            let Ok(target_peer) = crate::routed_relay::relay_target(payload, peer_id) else {
                                continue;
                            };
                            let targets = peers.forward_targets_for(
                                group_id,
                                peer_id,
                                addr,
                                target_peer,
                            );
                            if self.config.log_events {
                                println!(
                                    "Relay data group={group_id} peer={peer_id} bytes={} targets={}",
                                    payload.len(),
                                    targets.len()
                                );
                            }
                            for target in targets {
                                let _ = self.socket.send_to(&buf[..len], target).await;
                            }
                        }
                    }
                }
            }
        }
    }
}

pub struct BoundUdpRelayTunnel {
    socket: UdpSocket,
    remote_socket: UdpSocket,
    config: UdpRelayTunnelConfig,
}

pub struct BoundTcpRelayServer {
    listener: TcpListener,
    config: TcpRelayServerConfig,
}

impl BoundTcpRelayServer {
    pub async fn bind(config: TcpRelayServerConfig) -> io::Result<Self> {
        let listener = TcpListener::bind(config.bind_addr).await?;
        Ok(Self { listener, config })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub async fn run(self, mut cancel_rx: broadcast::Receiver<()>) -> io::Result<()> {
        let state = Arc::new(Mutex::new(TcpRelayServerState::default()));
        let connection_slots = Arc::new(Semaphore::new(self.config.max_connections));

        loop {
            tokio::select! {
                _ = cancel_rx.recv() => return Ok(()),
                accepted = self.listener.accept() => {
                    let (stream, addr) = accepted?;
                    let Ok(connection_slot) = connection_slots.clone().try_acquire_owned() else {
                        if self.config.log_events {
                            eprintln!("TCP relay rejected connection from {addr}: connection limit reached");
                        }
                        drop(stream);
                        continue;
                    };
                    let state = state.clone();
                    let config = self.config.clone();
                    tokio::spawn(async move {
                        let _connection_slot = connection_slot;
                        if let Err(err) = run_tcp_relay_connection(stream, addr, state, &config).await {
                            eprintln!("TCP relay connection ended: {err}");
                        }
                    });
                }
            }
        }
    }

    pub async fn run_websocket(self, cancel_rx: broadcast::Receiver<()>) -> io::Result<()> {
        self.run_websocket_inner(None, cancel_rx).await
    }

    /// Run WebSocket and unicast UDP relay servers sharing a single unified peer table so
    /// LAN/WAN UDP peers and WSS peers in the same group communicate seamlessly.
    pub async fn run_websocket_with_udp(
        self,
        udp_server: BoundUdpRelayServer,
        cancel_rx: broadcast::Receiver<()>,
    ) -> io::Result<()> {
        self.run_websocket_inner(Some(udp_server), cancel_rx).await
    }

    async fn run_websocket_inner(
        self,
        udp_server: Option<BoundUdpRelayServer>,
        mut cancel_rx: broadcast::Receiver<()>,
    ) -> io::Result<()> {
        let mut initial_state = TcpRelayServerState::default();
        let udp_task = if let Some(udp_server) = udp_server {
            initial_state.udp_peer_ttl = udp_server.config.peer_ttl;
            let state = Arc::new(Mutex::new(initial_state));
            let udp_socket = Arc::new(udp_server.socket);
            let udp_cancel = cancel_rx.resubscribe();
            let udp_state = state.clone();
            let udp_sock_clone = udp_socket.clone();
            let udp_config = udp_server.config;
            let max_peers_per_group = self.config.max_peers_per_group;
            let task = tokio::spawn(async move {
                run_bridged_udp_relay_loop(
                    udp_sock_clone,
                    udp_state,
                    udp_config,
                    max_peers_per_group,
                    udp_cancel,
                )
                .await
            });
            (state, Some(udp_socket), Some(task))
        } else {
            (Arc::new(Mutex::new(initial_state)), None, None)
        };
        let (state, udp_socket, udp_task) = udp_task;
        let connection_slots = Arc::new(Semaphore::new(self.config.max_connections));

        let accept_result: io::Result<()> = loop {
            tokio::select! {
                _ = cancel_rx.recv() => break Ok(()),
                accepted = self.listener.accept() => {
                    let (stream, addr) = match accepted {
                        Ok(pair) => pair,
                        Err(err) => break Err(err),
                    };
                    let Ok(connection_slot) = connection_slots.clone().try_acquire_owned() else {
                        if self.config.log_events {
                            eprintln!("WebSocket relay rejected connection from {addr}: connection limit reached");
                        }
                        drop(stream);
                        continue;
                    };
                    let state = state.clone();
                    let config = self.config.clone();
                    let udp_socket = udp_socket.clone();
                    tokio::spawn(async move {
                        let _connection_slot = connection_slot;
                        crate::net::configure_relay_tcp_stream(&stream);
                        let handshake = tokio::time::timeout(
                            WEBSOCKET_HANDSHAKE_TIMEOUT,
                            accept_async_with_config(stream, Some(relay_websocket_config())),
                        )
                        .await;
                        let relay_stream = match handshake {
                            Ok(Ok(stream)) => stream,
                            Ok(Err(err)) => {
                                if config.log_events {
                                    eprintln!("WebSocket relay handshake from {addr} failed: {err}");
                                }
                                return;
                            }
                            Err(_) => {
                                if config.log_events {
                                    eprintln!("WebSocket relay handshake from {addr} timed out");
                                }
                                return;
                            }
                        };
                        if let Err(err) = run_websocket_relay_connection_with_udp(
                            relay_stream,
                            addr,
                            state,
                            udp_socket,
                            &config,
                        )
                        .await
                        {
                            // Abnormal connection ends need a bounded reason
                            // without enabling packet logs or exposing routes.
                            eprintln!("WebSocket relay connection ended kind={:?} retirement={:?}",
                                err.kind(), err.get_ref().and_then(|e|e.downcast_ref::<RelayRetirement>()));
                        }
                    });
                }
            }
        };

        if let Some(task) = udp_task {
            let _ = task.await;
        }
        accept_result
    }
}

pub struct BoundHybridRelayTunnel {
    socket: UdpSocket,
    config: HybridRelayTunnelConfig,
    diagnostics: Arc<RelayTunnelDiagnostics>,
}

pub struct BoundTcpRelayTunnel {
    socket: UdpSocket,
    config: TcpRelayTunnelConfig,
}

pub struct BoundWebSocketRelayTunnel {
    socket: UdpSocket,
    config: WebSocketRelayTunnelConfig,
    diagnostics: Arc<RelayTunnelDiagnostics>,
}

impl BoundTcpRelayTunnel {
    pub async fn bind(config: TcpRelayTunnelConfig) -> io::Result<Self> {
        let socket = UdpSocket::bind(config.bind_addr).await?;
        crate::net::configure_relay_tunnel_udp_buffers(&socket);
        Ok(Self { socket, config })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    pub async fn run(self, mut cancel_rx: broadcast::Receiver<()>) -> io::Result<()> {
        loop {
            tokio::select! {
                _ = cancel_rx.recv() => return Ok(()),
                connected = TcpStream::connect(self.config.relay_addr) => {
                    match connected {
                        Ok(stream) => {
                            stream.set_nodelay(true)?;
                            crate::net::configure_relay_tcp_stream(&stream);
                            if self.config.log_events {
                                println!("TCP relay tunnel connected to {}", self.config.relay_addr);
                            }
                            match self.run_connected(stream, &mut cancel_rx).await {
                                Ok(()) => return Ok(()),
                                Err(err) => {
                                    if self.config.log_events {
                                        eprintln!("TCP relay tunnel disconnected from {}: {err}", self.config.relay_addr);
                                    }
                                }
                            }
                        }
                        Err(err) => {
                            if self.config.log_events {
                                eprintln!("TCP relay tunnel connect to {} failed: {err}", self.config.relay_addr);
                            }
                        }
                    }
                }
            }

            tokio::select! {
                _ = cancel_rx.recv() => return Ok(()),
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
        }
    }

    async fn run_connected(
        &self,
        stream: TcpStream,
        cancel_rx: &mut broadcast::Receiver<()>,
    ) -> io::Result<()> {
        let (mut reader, mut writer) = stream.into_split();
        let mut buf = vec![0u8; MAX_PACKET_LEN];
        let mut local_target_addr = self.config.local_target_addr;
        let register_packet = RelayPacket::Register {
            group_id: self.config.group_id.clone(),
            peer_id: self.config.peer_id.clone(),
        }
        .encode();
        write_tcp_frame(&mut writer, &register_packet).await?;
        let mut register_interval = tokio::time::interval(self.config.register_interval);

        loop {
            tokio::select! {
                _ = cancel_rx.recv() => return Ok(()),
                _ = register_interval.tick() => {
                    write_tcp_frame(&mut writer, &register_packet).await?;
                }
                frame = read_tcp_frame(&mut reader) => {
                    let frame = frame?;
                    if let Some(RelayPacket::Data { payload, .. }) = RelayPacket::decode(&frame)
                        && let Some(target) = local_target_addr
                    {
                        if self.config.log_events {
                            println!("TCP relay tunnel inbound bytes={} target={target}", payload.len());
                        }
                        let _ = self.socket.send_to(&payload, target).await;
                    }
                }
                received = self.socket.recv_from(&mut buf) => {
                    let (len, addr) = received?;
                    local_target_addr = Some(addr);
                    if self.config.log_events {
                        println!("TCP relay tunnel outbound bytes={len} source={addr}");
                    }
                    let encoded = RelayPacket::Data {
                        group_id: self.config.group_id.clone(),
                        peer_id: self.config.peer_id.clone(),
                        payload: buf[..len].to_vec(),
                    }
                    .encode();
                    write_tcp_frame(&mut writer, &encoded).await?;
                }
            }
        }
    }
}

struct ObservedRelayTransport {
    socket: TcpStream,
    diagnostics: Arc<RelayTunnelDiagnostics>,
}
impl tokio::io::AsyncRead for ObservedRelayTransport {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let before = buffer.filled().len();
        let result = std::pin::Pin::new(&mut self.socket).poll_read(cx, buffer);
        self.diagnostics
            .transport_bytes(false, buffer.filled().len().saturating_sub(before));
        result
    }
}
impl tokio::io::AsyncWrite for ObservedRelayTransport {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        let result = std::pin::Pin::new(&mut self.socket).poll_write(cx, buffer);
        if let std::task::Poll::Ready(Ok(count)) = &result {
            self.diagnostics.transport_bytes(true, *count);
        }
        result
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.socket).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.socket).poll_shutdown(cx)
    }
}

async fn connect_observed_websocket(
    url: &str,
    diagnostics: &Arc<RelayTunnelDiagnostics>,
) -> Result<
    (
        WebSocketStream<tokio_tungstenite::MaybeTlsStream<ObservedRelayTransport>>,
        tokio_tungstenite::tungstenite::handshake::client::Response,
    ),
    RelayFailure,
> {
    use tokio_tungstenite::MaybeTlsStream;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let request = url
        .into_client_request()
        .map_err(|_| RelayFailure::Protocol)?;
    let uri = request.uri();
    let host = uri
        .host()
        .ok_or(RelayFailure::Protocol)?
        .trim_matches(['[', ']'])
        .to_owned();
    let tls = uri.scheme_str() == Some("wss");
    let port = uri.port_u16().unwrap_or(if tls { 443 } else { 80 });
    diagnostics.stage(RelayStage::Dns);
    let addresses = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|error| {
            diagnostics.connection_error(error.raw_os_error(), None);
            io_failure(&error)
        })?
        .collect::<Vec<_>>();
    diagnostics.dns_resolved(&addresses);
    if addresses.is_empty() {
        return Err(RelayFailure::Connection);
    }
    diagnostics.stage(RelayStage::Tcp);
    let socket = TcpStream::connect(addresses.as_slice())
        .await
        .map_err(|error| {
            diagnostics.connection_error(error.raw_os_error(), None);
            io_failure(&error)
        })?;
    socket.set_nodelay(true).map_err(|error| {
        diagnostics.connection_error(error.raw_os_error(), None);
        io_failure(&error)
    })?;
    crate::net::configure_relay_tcp_stream(&socket);
    diagnostics.tcp_connected();
    let socket = ObservedRelayTransport {
        socket,
        diagnostics: diagnostics.clone(),
    };
    let stream = if tls {
        diagnostics.stage(RelayStage::Tls);
        // Identical root set and builder to locked tokio-tungstenite 0.28's default.
        // Split only so EOF during TLS cannot be reported as a WebSocket failure.
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let name =
            rustls::pki_types::ServerName::try_from(host).map_err(|_| RelayFailure::Protocol)?;
        let stream = tokio_rustls::TlsConnector::from(Arc::new(config))
            .connect(name, socket)
            .await
            .map_err(|error| {
                diagnostics.connection_error(error.raw_os_error(), None);
                tls_io_failure(&error)
            })?;
        diagnostics.tls_connected();
        MaybeTlsStream::Rustls(stream)
    } else {
        MaybeTlsStream::Plain(socket)
    };
    diagnostics.stage(RelayStage::Websocket);
    tokio_tungstenite::client_async_with_config(request, stream, Some(relay_websocket_config()))
        .await
        .map_err(|error| {
            let status = if let tokio_tungstenite::tungstenite::Error::Http(response) = &error {
                Some(response.status().as_u16())
            } else {
                None
            };
            let errno = if let tokio_tungstenite::tungstenite::Error::Io(io) = &error {
                io.raw_os_error()
            } else {
                None
            };
            diagnostics.connection_error(errno, status);
            websocket_failure(&error)
        })
}

impl BoundWebSocketRelayTunnel {
    pub async fn bind(config: WebSocketRelayTunnelConfig) -> io::Result<Self> {
        // JNI/native consumers do not run the desktop entrypoint. Preserve any
        // process-selected provider and use the existing ring bootstrap for WSS.
        if config
            .candidate_relay_urls()
            .iter()
            .any(|url| url.starts_with("wss://"))
        {
            crate::init_crypto_provider();
        }
        let socket = UdpSocket::bind(config.bind_addr).await?;
        crate::net::configure_relay_tunnel_udp_buffers(&socket);
        Ok(Self {
            socket,
            config,
            diagnostics: Arc::new(RelayTunnelDiagnostics::default()),
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    pub fn diagnostics(&self) -> Arc<RelayTunnelDiagnostics> {
        self.diagnostics.clone()
    }

    pub async fn run(self, mut cancel_rx: broadcast::Receiver<()>) -> io::Result<()> {
        let _stopped = StopObservation(self.diagnostics.clone());
        let candidates: Vec<String> = self
            .config
            .candidate_relay_urls()
            .into_iter()
            .map(str::to_owned)
            .collect();
        let mut candidate_index = 0usize;
        let mut retry_delay = Duration::from_millis(200);
        loop {
            let url = &candidates[candidate_index];
            let has_fallback = candidates.len() > 1 && candidate_index + 1 < candidates.len();
            let connect_timeout = if has_fallback && is_private_or_local_url(url) {
                LAN_FAST_CONNECT_TIMEOUT
            } else {
                WEBSOCKET_HANDSHAKE_TIMEOUT
            };
            self.diagnostics.attempt();
            let attempt_started = Instant::now();
            let mut connected_session = false;
            let mut healthy_drop = false;
            tokio::select! {
                biased;
                _ = cancel_rx.recv() => return Ok(()),
                connected = tokio::time::timeout(connect_timeout,
                    connect_observed_websocket(url.as_str(), &self.diagnostics)) => {
                    match connected {
                        Ok(Ok((stream, _response))) => {
                            connected_session = true;
                            self.diagnostics.connected();
                            if self.config.log_events {
                                eprintln!("WebSocket relay connected url={url}");
                            }
                            match self.run_connected(stream, &mut cancel_rx).await {
                                Ok(()) => return Ok(()),
                                Err(err) => {
                                    if attempt_started.elapsed() >= Duration::from_secs(5) {
                                        retry_delay = Duration::from_millis(200);
                                        healthy_drop = true;
                                    }
                                    self.diagnostics.retry(io_failure(&err));
                                }
                            }
                        }
                        Ok(Err(failure)) => self.diagnostics.retry(failure),
                        Err(_) => self.diagnostics.retry(RelayFailure::Timeout),
                    }
                }
            }
            if self.config.log_events {
                eprintln!(
                    "WebSocket relay retry url={url}: {:?}",
                    self.diagnostics.snapshot().last_error
                );
            }
            if candidates.len() > 1 {
                if healthy_drop {
                    // After a healthy session drops (e.g. Wi-Fi roam back home), snap back
                    // to index 0 so LAN-first endpoints are probed first.
                    candidate_index = 0;
                } else {
                    candidate_index = (candidate_index + 1) % candidates.len();
                    if !connected_session && candidate_index != 0 {
                        // Fail over immediately to the next candidate in the same round.
                        continue;
                    }
                }
            }
            tokio::select! {
                _ = cancel_rx.recv() => return Ok(()),
                _ = tokio::time::sleep(retry_delay) => {}
            }
            retry_delay = (retry_delay * 2).min(Duration::from_secs(1));
        }
    }

    async fn run_connected<S>(
        &self,
        stream: WebSocketStream<S>,
        cancel_rx: &mut broadcast::Receiver<()>,
    ) -> io::Result<()>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        run_connected_websocket_stream(
            &self.socket,
            &self.config.group_id,
            &self.config.peer_id,
            self.config.local_target_addr,
            self.config.register_interval,
            self.config.heartbeat_interval,
            self.config.heartbeat_timeout,
            &self.diagnostics,
            stream,
            cancel_rx,
        )
        .await
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_connected_websocket_stream<S>(
    socket: &UdpSocket,
    group_id: &str,
    peer_id: &str,
    initial_local_target: Option<SocketAddr>,
    register_interval_duration: Duration,
    heartbeat_interval_duration: Duration,
    heartbeat_timeout: Duration,
    diagnostics: &Arc<RelayTunnelDiagnostics>,
    stream: WebSocketStream<S>,
    cancel_rx: &mut broadcast::Receiver<()>,
) -> io::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut writer, mut reader) = stream.split();
    let local_target_addr = SyncMutex::new(initial_local_target);
    let last_server_message = SyncMutex::new(Instant::now());
    let pending_ping = SyncMutex::new(None::<Bytes>);
    let flush = tokio::sync::Notify::new();
    // Keep both halves progressing in this connection's existing task. A
    // blocked write must not stop reads and deadlock a full-duplex peer.
    // No unbounded queue, extra worker or packet reordering is introduced.
    let receive = async {
        let mut forwarded_since_yield = 0usize;
        loop {
            let message = reader.next().await;
            let Some(message) = message else {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "WebSocket relay closed",
                ));
            };
            let message = message.map_err(websocket_io_error)?;
            *last_server_message
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Instant::now();
            match message {
                Message::Binary(frame) => {
                    if let Some(RelayPacketView::Data { payload, .. }) =
                        RelayPacketView::decode(&frame)
                    {
                        diagnostics.data_received();
                        let target = *local_target_addr.lock().unwrap_or_else(|e| e.into_inner());
                        if let Some(target) = target {
                            let _ = socket.send_to(payload, target).await;
                        }
                        forwarded_since_yield += 1;
                        if forwarded_since_yield >= WEBSOCKET_RECV_YIELD_INTERVAL {
                            forwarded_since_yield = 0;
                            // Ready fragment buffers must still give the other
                            // half and capture/input tasks a scheduling turn.
                            tokio::task::yield_now().await;
                        }
                    }
                }
                Message::Close(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionReset,
                        "WebSocket relay closed",
                    ));
                }
                Message::Pong(payload) => {
                    // Only an echo of this connection's outstanding ping is
                    // a heartbeat observation. It is not a registration ACK
                    // or evidence that another device received a data frame.
                    let mut pending = pending_ping.lock().unwrap_or_else(|e| e.into_inner());
                    if pending.as_ref() == Some(&payload) {
                        *pending = None;
                        diagnostics.pong_received();
                    }
                }
                // Tungstenite queues the protocol Pong while reading.
                // Wake the writer even when there is no local payload.
                Message::Ping(_) => flush.notify_one(),
                Message::Text(_) | Message::Frame(_) => {}
            }
        }
    };
    let send = async {
        let mut buf = vec![0u8; MAX_PACKET_LEN];
        let register_packet = RelayPacket::Register {
            group_id: group_id.to_owned(),
            peer_id: peer_id.to_owned(),
        }
        .encode();
        send_websocket_binary(&mut writer, register_packet.clone()).await?;
        diagnostics.registration_sent();
        let mut register_interval = tokio::time::interval(register_interval_duration);
        let mut heartbeat_interval = tokio::time::interval(heartbeat_interval_duration);
        heartbeat_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut heartbeat_sequence = 0u64;
        loop {
            tokio::select! {
                _=flush.notified()=>writer.flush().await.map_err(websocket_io_error)?,
                _=register_interval.tick()=> {
                    send_websocket_binary(&mut writer,register_packet.clone()).await?;
                    diagnostics.registration_sent();
                }
                _=heartbeat_interval.tick()=> {
                    let age=last_server_message.lock().unwrap_or_else(|e|e.into_inner()).elapsed();
                    if age>heartbeat_timeout {
                        return Err(io::Error::new(io::ErrorKind::TimedOut,"WebSocket relay heartbeat timed out"));
                    }
                    heartbeat_sequence=heartbeat_sequence.wrapping_add(1);
                    let payload=Bytes::copy_from_slice(&heartbeat_sequence.to_be_bytes());
                    // Install before the write: the concurrent reader may see
                    // the echoed Pong before send() finishes flushing.
                    *pending_ping.lock().unwrap_or_else(|e|e.into_inner())=Some(payload.clone());
                    writer.send(Message::Ping(payload)).await.map_err(websocket_io_error)?;
                    diagnostics.ping_sent();
                }
                received = socket.recv_from(&mut buf) => {
                    let (mut len, mut addr) = received?;
                    let mut batched = 0usize;
                    let flush_started = Instant::now();
                    loop {
                        *local_target_addr.lock().unwrap_or_else(|e|e.into_inner())=Some(addr);
                        let encoded = RelayPacket::encode_data_frame(
                            group_id,
                            peer_id,
                            &buf[..len],
                        );
                        writer
                            .feed(Message::Binary(Bytes::from(encoded)))
                            .await
                            .map_err(websocket_io_error)?;
                        diagnostics.data_sent();
                        batched += 1;
                        if batched >= WEBSOCKET_SEND_BATCH_LIMIT {
                            break;
                        }
                        match socket.try_recv_from(&mut buf) {
                            Ok((next_len, next_addr)) => {
                                len = next_len;
                                addr = next_addr;
                            }
                            Err(_) => break,
                        }
                    }
                    writer.flush().await.map_err(websocket_io_error)?;
                    record_relay_outbound_flush(flush_started.elapsed(), batched);
                    tokio::task::yield_now().await;
                }
            }
        }
    };
    tokio::select! {
        biased;
        _=cancel_rx.recv()=>Ok(()),
        result=receive=>result,
        result=send=>result,
    }
}

async fn connect_observed_udp_relay(
    url: &str,
    group_id: &str,
    peer_id: &str,
    handshake_timeout: Duration,
    diagnostics: &Arc<RelayTunnelDiagnostics>,
) -> Result<(UdpSocket, SocketAddr), RelayFailure> {
    let (host, port) = parse_udp_relay_host_port(url).ok_or(RelayFailure::Protocol)?;
    diagnostics.stage(RelayStage::Dns);
    let mut addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| {
            diagnostics.connection_error(error.raw_os_error(), None);
            io_failure(&error)
        })?
        .collect::<Vec<_>>();
    // Prefer IPv4 when both A and AAAA records exist because home router UDP port
    // forwards are predominantly IPv4.
    addresses.sort_by_key(|addr| if addr.is_ipv4() { 0u8 } else { 1u8 });
    diagnostics.dns_resolved(&addresses);
    let Some(relay_addr) = addresses.first().copied() else {
        return Err(RelayFailure::Connection);
    };
    let remote_bind = match relay_addr {
        SocketAddr::V4(_) => SocketAddr::from(([0, 0, 0, 0], 0)),
        SocketAddr::V6(_) => SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, 0)),
    };
    let remote_socket = UdpSocket::bind(remote_bind).await.map_err(|error| {
        diagnostics.connection_error(error.raw_os_error(), None);
        io_failure(&error)
    })?;
    crate::net::configure_relay_tunnel_udp_buffers(&remote_socket);
    diagnostics.stage(RelayStage::Registration);

    let register_packet = RelayPacket::Register {
        group_id: group_id.to_owned(),
        peer_id: peer_id.to_owned(),
    }
    .encode();
    let deadline = tokio::time::Instant::now() + handshake_timeout;
    let mut resend = tokio::time::interval(UDP_HANDSHAKE_RETRY_INTERVAL);
    resend.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut buf = vec![0u8; MAX_PACKET_LEN];

    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Err(RelayFailure::Timeout);
        }
        tokio::select! {
            biased;
            _ = tokio::time::sleep_until(deadline) => return Err(RelayFailure::Timeout),
            _ = resend.tick() => {
                if let Err(error) = remote_socket.send_to(&register_packet, relay_addr).await {
                    diagnostics.connection_error(error.raw_os_error(), None);
                    return Err(io_failure(&error));
                }
            }
            received = remote_socket.recv_from(&mut buf) => {
                match received {
                    Ok((len, addr)) if addr == relay_addr => {
                        if let Some(RelayPacketView::Register { group_id: ack_group, peer_id: ack_peer }) =
                            RelayPacketView::decode(&buf[..len])
                            && ack_group == group_id
                            && ack_peer == peer_id
                        {
                            return Ok((remote_socket, relay_addr));
                        }
                    }
                    Ok(_) => {}
                    Err(error) if is_transient_udp_recv_error(&error) => {}
                    Err(error) => {
                        diagnostics.connection_error(error.raw_os_error(), None);
                        return Err(io_failure(&error));
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_connected_udp_stream(
    local_socket: &UdpSocket,
    remote_socket: UdpSocket,
    relay_addr: SocketAddr,
    group_id: &str,
    peer_id: &str,
    initial_local_target: Option<SocketAddr>,
    register_interval_duration: Duration,
    heartbeat_timeout: Duration,
    diagnostics: &Arc<RelayTunnelDiagnostics>,
    cancel_rx: &mut broadcast::Receiver<()>,
) -> io::Result<()> {
    let mut local_buf = vec![0u8; MAX_PACKET_LEN];
    let mut remote_buf = vec![0u8; MAX_PACKET_LEN];
    let mut local_target_addr = initial_local_target;
    let mut last_server_message = Instant::now();
    let register_packet = RelayPacket::Register {
        group_id: group_id.to_owned(),
        peer_id: peer_id.to_owned(),
    }
    .encode();
    let mut register_interval = tokio::time::interval(register_interval_duration);
    register_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let effective_timeout = heartbeat_timeout.min(UDP_DEFAULT_HEARTBEAT_TIMEOUT);

    loop {
        tokio::select! {
            biased;
            _ = cancel_rx.recv() => return Ok(()),
            _ = register_interval.tick() => {
                if last_server_message.elapsed() > effective_timeout {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "UDP relay heartbeat timed out",
                    ));
                }
                remote_socket.send_to(&register_packet, relay_addr).await?;
                diagnostics.registration_sent();
            }
            received = remote_socket.recv_from(&mut remote_buf) => {
                let (mut len, mut addr) = match received {
                    Ok(pair) => pair,
                    Err(err) if is_transient_udp_recv_error(&err) => continue,
                    Err(err) => return Err(err),
                };
                let mut processed = 0usize;
                loop {
                    if addr == relay_addr {
                        match RelayPacketView::decode(&remote_buf[..len]) {
                            Some(RelayPacketView::Data { payload, .. }) => {
                                last_server_message = Instant::now();
                                diagnostics.data_received();
                                if let Some(target) = local_target_addr {
                                    let _ = local_socket.send_to(payload, target).await;
                                }
                            }
                            Some(RelayPacketView::Register { .. }) => {
                                last_server_message = Instant::now();
                                diagnostics.pong_received();
                            }
                            None => {}
                        }
                    }
                    processed += 1;
                    if processed >= WEBSOCKET_SEND_BATCH_LIMIT {
                        tokio::task::yield_now().await;
                        break;
                    }
                    match remote_socket.try_recv_from(&mut remote_buf) {
                        Ok((next_len, next_addr)) => {
                            len = next_len;
                            addr = next_addr;
                        }
                        Err(_) => break,
                    }
                }
            }
            received = local_socket.recv_from(&mut local_buf) => {
                let (mut len, mut addr) = received?;
                let mut batched = 0usize;
                loop {
                    local_target_addr = Some(addr);
                    let encoded = RelayPacket::encode_data_frame(
                        group_id,
                        peer_id,
                        &local_buf[..len],
                    );
                    remote_socket.send_to(&encoded, relay_addr).await?;
                    diagnostics.data_sent();
                    batched += 1;
                    if batched >= WEBSOCKET_SEND_BATCH_LIMIT {
                        tokio::task::yield_now().await;
                        break;
                    }
                    match local_socket.try_recv_from(&mut local_buf) {
                        Ok((next_len, next_addr)) => {
                            len = next_len;
                            addr = next_addr;
                        }
                        Err(_) => break,
                    }
                }
            }
        }
    }
}

impl BoundHybridRelayTunnel {
    pub async fn bind(config: HybridRelayTunnelConfig) -> io::Result<Self> {
        if config.candidates.iter().any(|candidate| {
            matches!(candidate, HybridRelayCandidate::WebSocket(url) if url.starts_with("wss://"))
        }) {
            crate::init_crypto_provider();
        }
        let socket = UdpSocket::bind(config.bind_addr).await?;
        crate::net::configure_relay_tunnel_udp_buffers(&socket);
        Ok(Self {
            socket,
            config,
            diagnostics: Arc::new(RelayTunnelDiagnostics::default()),
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    pub fn diagnostics(&self) -> Arc<RelayTunnelDiagnostics> {
        self.diagnostics.clone()
    }

    pub async fn run(self, mut cancel_rx: broadcast::Receiver<()>) -> io::Result<()> {
        let _stopped = StopObservation(self.diagnostics.clone());
        let candidates = self.config.candidates.clone();
        let mut candidate_index = 0usize;
        let mut retry_delay = Duration::from_millis(200);
        loop {
            let candidate = &candidates[candidate_index];
            let has_fallback = candidates.len() > 1 && candidate_index + 1 < candidates.len();
            self.diagnostics.attempt();
            let attempt_started = Instant::now();
            let mut connected_session = false;
            let mut healthy_drop = false;

            match candidate {
                HybridRelayCandidate::WebSocket(url) => {
                    let connect_timeout = if has_fallback && is_private_or_local_url(url) {
                        LAN_FAST_CONNECT_TIMEOUT
                    } else {
                        WEBSOCKET_HANDSHAKE_TIMEOUT
                    };
                    tokio::select! {
                        biased;
                        _ = cancel_rx.recv() => return Ok(()),
                        connected = tokio::time::timeout(
                            connect_timeout,
                            connect_observed_websocket(url.as_str(), &self.diagnostics),
                        ) => {
                            match connected {
                                Ok(Ok((stream, _response))) => {
                                    connected_session = true;
                                    self.diagnostics.connected();
                                    if self.config.log_events {
                                        eprintln!("Hybrid relay connected (WS) url={url}");
                                    }
                                    match run_connected_websocket_stream(
                                        &self.socket,
                                        &self.config.group_id,
                                        &self.config.peer_id,
                                        self.config.local_target_addr,
                                        self.config.register_interval,
                                        self.config.heartbeat_interval,
                                        self.config.heartbeat_timeout,
                                        &self.diagnostics,
                                        stream,
                                        &mut cancel_rx,
                                    )
                                    .await
                                    {
                                        Ok(()) => return Ok(()),
                                        Err(err) => {
                                            if attempt_started.elapsed() >= Duration::from_secs(5) {
                                                retry_delay = Duration::from_millis(200);
                                                healthy_drop = true;
                                            }
                                            self.diagnostics.retry(io_failure(&err));
                                        }
                                    }
                                }
                                Ok(Err(failure)) => self.diagnostics.retry(failure),
                                Err(_) => self.diagnostics.retry(RelayFailure::Timeout),
                            }
                        }
                    }
                }
                HybridRelayCandidate::Udp(url) => {
                    let connect_timeout = if has_fallback && is_private_or_local_url(url) {
                        LAN_FAST_CONNECT_TIMEOUT
                    } else {
                        UDP_PUBLIC_HANDSHAKE_TIMEOUT
                    };
                    tokio::select! {
                        biased;
                        _ = cancel_rx.recv() => return Ok(()),
                        connected = connect_observed_udp_relay(
                            url.as_str(),
                            &self.config.group_id,
                            &self.config.peer_id,
                            connect_timeout,
                            &self.diagnostics,
                        ) => {
                            match connected {
                                Ok((remote_socket, relay_addr)) => {
                                    connected_session = true;
                                    self.diagnostics.connected_udp();
                                    self.diagnostics.registration_sent();
                                    self.diagnostics.pong_received();
                                    if self.config.log_events {
                                        eprintln!("Hybrid relay connected (UDP) url={url} addr={relay_addr}");
                                    }
                                    match run_connected_udp_stream(
                                        &self.socket,
                                        remote_socket,
                                        relay_addr,
                                        &self.config.group_id,
                                        &self.config.peer_id,
                                        self.config.local_target_addr,
                                        self.config.register_interval,
                                        self.config.heartbeat_timeout,
                                        &self.diagnostics,
                                        &mut cancel_rx,
                                    )
                                    .await
                                    {
                                        Ok(()) => return Ok(()),
                                        Err(err) => {
                                            if attempt_started.elapsed() >= Duration::from_secs(5) {
                                                retry_delay = Duration::from_millis(200);
                                                healthy_drop = true;
                                            }
                                            self.diagnostics.retry(io_failure(&err));
                                        }
                                    }
                                }
                                Err(failure) => self.diagnostics.retry(failure),
                            }
                        }
                    }
                }
            }

            if self.config.log_events {
                eprintln!(
                    "Hybrid relay retry url={}: {:?}",
                    candidate.as_str(),
                    self.diagnostics.snapshot().last_error
                );
            }
            if candidates.len() > 1 {
                if healthy_drop {
                    candidate_index = 0;
                } else {
                    candidate_index = (candidate_index + 1) % candidates.len();
                    if !connected_session && candidate_index != 0 {
                        continue;
                    }
                }
            }
            tokio::select! {
                _ = cancel_rx.recv() => return Ok(()),
                _ = tokio::time::sleep(retry_delay) => {}
            }
            retry_delay = (retry_delay * 2).min(Duration::from_secs(1));
        }
    }
}

impl BoundUdpRelayTunnel {
    pub async fn bind(config: UdpRelayTunnelConfig) -> io::Result<Self> {
        let socket = UdpSocket::bind(config.bind_addr).await?;
        crate::net::configure_relay_tunnel_udp_buffers(&socket);
        let remote_bind = match config.relay_addr {
            SocketAddr::V4(_) => SocketAddr::from(([0, 0, 0, 0], 0)),
            SocketAddr::V6(_) => SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, 0)),
        };
        let remote_socket = UdpSocket::bind(remote_bind).await?;
        crate::net::configure_relay_tunnel_udp_buffers(&remote_socket);
        Ok(Self {
            socket,
            remote_socket,
            config,
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    pub async fn run(self, mut cancel_rx: broadcast::Receiver<()>) -> io::Result<()> {
        let mut local_buf = vec![0u8; MAX_PACKET_LEN];
        let mut remote_buf = vec![0u8; MAX_PACKET_LEN];
        let mut local_target_addr = self.config.local_target_addr;
        let register_packet = RelayPacket::Register {
            group_id: self.config.group_id.clone(),
            peer_id: self.config.peer_id.clone(),
        }
        .encode();
        let mut register_interval = tokio::time::interval(self.config.register_interval);
        let _ = self
            .remote_socket
            .send_to(&register_packet, self.config.relay_addr)
            .await;

        loop {
            tokio::select! {
                _ = cancel_rx.recv() => return Ok(()),
                _ = register_interval.tick() => {
                    let _ = self.remote_socket.send_to(&register_packet, self.config.relay_addr).await;
                }
                received = self.remote_socket.recv_from(&mut remote_buf) => {
                    let (len, addr) = match received {
                        Ok(pair) => pair,
                        Err(err) if is_transient_udp_recv_error(&err) => continue,
                        Err(err) => return Err(err),
                    };
                    if addr == self.config.relay_addr
                        && let Some(RelayPacketView::Data { payload, .. }) =
                            RelayPacketView::decode(&remote_buf[..len])
                        && let Some(target) = local_target_addr
                    {
                        let _ = self.socket.send_to(payload, target).await;
                    }
                }
                received = self.socket.recv_from(&mut local_buf) => {
                    let (len, addr) = received?;
                    local_target_addr = Some(addr);
                    let encoded = RelayPacket::encode_data_frame(
                        &self.config.group_id,
                        &self.config.peer_id,
                        &local_buf[..len],
                    );
                    let _ = self.remote_socket.send_to(&encoded, self.config.relay_addr).await;
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RelayPeerKey {
    group_id: String,
    peer_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RelayPeerState {
    addr: SocketAddr,
    last_seen: Instant,
}

struct RelayPeerTable {
    peer_ttl: Duration,
    peers: HashMap<RelayPeerKey, RelayPeerState>,
}

struct TcpRelayServerState {
    next_connection_id: u64,
    peers: HashMap<RelayPeerKey, TcpRelayPeerState>,
    connection_identities: HashMap<u64, RelayPeerKey>,
    udp_peers: HashMap<RelayPeerKey, RelayPeerState>,
    udp_peer_ttl: Duration,
    last_udp_prune: Option<Instant>,
}

impl Default for TcpRelayServerState {
    fn default() -> Self {
        Self {
            next_connection_id: 0,
            peers: HashMap::new(),
            connection_identities: HashMap::new(),
            udp_peers: HashMap::new(),
            udp_peer_ttl: Duration::from_secs(15),
            last_udp_prune: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelayRetirement {
    Replaced,
    SlowPeer,
}
impl std::fmt::Display for RelayRetirement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Replaced => "relay connection replaced",
            Self::SlowPeer => "relay peer is too slow",
        })
    }
}
impl std::error::Error for RelayRetirement {}
fn relay_retirement_error(reason: Option<RelayRetirement>) -> io::Error {
    match reason {
        Some(reason) => io::Error::new(io::ErrorKind::ConnectionAborted, reason),
        None => io::Error::new(
            io::ErrorKind::BrokenPipe,
            "relay ownership notification ended",
        ),
    }
}

#[derive(Clone)]
struct TcpRelayPeerState {
    connection_id: u64,
    writer_tx: mpsc::Sender<Arc<[u8]>>,
    disconnect_tx: watch::Sender<Option<RelayRetirement>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelayRegistrationError {
    ConnectionRetired,
    ConnectionIdentityChanged,
    GroupPeerLimitReached,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RelayForwardResult {
    forwarded: usize,
    disconnected_slow_peers: usize,
}

impl TcpRelayServerState {
    fn next_connection_id(&mut self) -> u64 {
        self.next_connection_id = self.next_connection_id.wrapping_add(1).max(1);
        self.next_connection_id
    }

    fn prune_expired_udp_peers_if_due(&mut self, force: bool) {
        if self.udp_peers.is_empty() {
            return;
        }
        let now = Instant::now();
        if !force
            && self
                .last_udp_prune
                .is_some_and(|last| now.duration_since(last) < UDP_PEER_PRUNE_INTERVAL)
        {
            return;
        }
        let ttl = self.udp_peer_ttl;
        self.udp_peers
            .retain(|_, peer| now.duration_since(peer.last_seen) <= ttl);
        self.last_udp_prune = Some(now);
    }

    fn group_peer_count_excluding(&self, group_id: &str, peer_id: &str) -> usize {
        let tcp_count = self
            .peers
            .keys()
            .filter(|peer| peer.group_id == group_id && peer.peer_id != peer_id)
            .count();
        let udp_count = self
            .udp_peers
            .keys()
            .filter(|peer| {
                peer.group_id == group_id
                    && peer.peer_id != peer_id
                    && !self.peers.contains_key(*peer)
            })
            .count();
        tcp_count + udp_count
    }

    fn register(
        &mut self,
        group_id: String,
        peer_id: String,
        connection_id: u64,
        writer_tx: mpsc::Sender<Arc<[u8]>>,
        disconnect_tx: watch::Sender<Option<RelayRetirement>>,
        max_peers_per_group: usize,
    ) -> Result<(), RelayRegistrationError> {
        if disconnect_tx.borrow().is_some() {
            return Err(RelayRegistrationError::ConnectionRetired);
        }
        let key = RelayPeerKey { group_id, peer_id };
        if let Some(existing) = self.connection_identities.get(&connection_id) {
            return if existing == &key {
                Ok(())
            } else {
                Err(RelayRegistrationError::ConnectionIdentityChanged)
            };
        }

        self.prune_expired_udp_peers_if_due(true);
        let existing_peer_connection = self.peers.get(&key).map(|peer| peer.connection_id);
        let other_group_peers = self.group_peer_count_excluding(&key.group_id, &key.peer_id);
        if existing_peer_connection.is_none() && other_group_peers >= max_peers_per_group {
            return Err(RelayRegistrationError::GroupPeerLimitReached);
        }

        // If this peer previously registered over UDP, remove the UDP route so
        // it never receives duplicate copies over both transports.
        self.udp_peers.remove(&key);
        if let Some(existing) = self.peers.remove(&key) {
            let _ = existing.disconnect_tx.send(Some(RelayRetirement::Replaced));
            self.connection_identities.remove(&existing.connection_id);
        }
        self.connection_identities
            .insert(connection_id, key.clone());
        self.peers.insert(
            key,
            TcpRelayPeerState {
                connection_id,
                writer_tx,
                disconnect_tx,
            },
        );
        Ok(())
    }

    fn register_udp(
        &mut self,
        group_id: &str,
        peer_id: &str,
        addr: SocketAddr,
        max_peers_per_group: usize,
    ) -> bool {
        self.prune_expired_udp_peers_if_due(false);
        let now = Instant::now();
        // Hot path: update existing UDP peer without allocating Strings.
        if let Some((_, existing)) = self
            .udp_peers
            .iter_mut()
            .find(|(key, _)| key.group_id == group_id && key.peer_id == peer_id)
        {
            existing.addr = addr;
            existing.last_seen = now;
            return true;
        }
        self.prune_expired_udp_peers_if_due(true);
        if self.group_peer_count_excluding(group_id, peer_id) >= max_peers_per_group {
            return false;
        }
        let key = RelayPeerKey {
            group_id: group_id.to_owned(),
            peer_id: peer_id.to_owned(),
        };
        if let Some(existing_ws) = self.peers.remove(&key) {
            let _ = existing_ws
                .disconnect_tx
                .send(Some(RelayRetirement::Replaced));
            self.connection_identities
                .remove(&existing_ws.connection_id);
        }
        self.udp_peers.insert(
            key,
            RelayPeerState {
                addr,
                last_seen: now,
            },
        );
        true
    }

    fn identity(&self, connection_id: u64) -> Option<&RelayPeerKey> {
        self.connection_identities.get(&connection_id)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn forward_frame(
        &mut self,
        group_id: &str,
        source_connection_id: u64,
        frame: Arc<[u8]>,
        payload: &[u8],
    ) -> (RelayForwardResult, Vec<TcpRelayPeerState>) {
        let (result, deferred, _udp_targets) =
            self.forward_frame_with_udp(group_id, source_connection_id, frame, payload);
        (result, deferred)
    }

    fn forward_frame_with_udp(
        &mut self,
        group_id: &str,
        source_connection_id: u64,
        frame: Arc<[u8]>,
        payload: &[u8],
    ) -> (RelayForwardResult, Vec<TcpRelayPeerState>, Vec<SocketAddr>) {
        let mut result = RelayForwardResult {
            forwarded: 0,
            disconnected_slow_peers: 0,
        };
        let mut deferred = Vec::new();
        let Some(source) = self
            .identity(source_connection_id)
            .filter(|identity| identity.group_id == group_id)
            .map(|identity| identity.peer_id.as_str())
        else {
            return (result, deferred, Vec::new());
        };
        let Ok(target) = crate::routed_relay::relay_target(payload, source) else {
            return (result, deferred, Vec::new());
        };
        let targets = self
            .peers
            .iter()
            .filter(|(key, peer)| {
                key.group_id == group_id
                    && peer.connection_id != source_connection_id
                    && target.is_none_or(|target| key.peer_id == target)
            })
            .map(|(_, peer)| peer.clone())
            .collect::<Vec<_>>();
        let now = Instant::now();
        let udp_ttl = self.udp_peer_ttl;
        let udp_targets = self
            .udp_peers
            .iter()
            .filter(|(key, peer)| {
                key.group_id == group_id
                    && key.peer_id != source
                    && target.is_none_or(|target| key.peer_id == target)
                    && now.duration_since(peer.last_seen) <= udp_ttl
            })
            .map(|(_, peer)| peer.addr)
            .collect::<Vec<_>>();
        let mut failed_connections = Vec::new();
        for peer in targets {
            match peer.writer_tx.try_send(frame.clone()) {
                Ok(()) => result.forwarded += 1,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    deferred.push(peer);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    failed_connections.push(peer.connection_id);
                }
            }
        }
        for connection_id in failed_connections {
            self.remove_connection(connection_id);
        }
        (result, deferred, udp_targets)
    }

    fn collect_udp_forward_targets(
        &self,
        group_id: &str,
        source_peer_id: &str,
        source_addr: SocketAddr,
        payload: &[u8],
    ) -> Option<(Vec<SocketAddr>, Vec<mpsc::Sender<Arc<[u8]>>>)> {
        let Ok(target) = crate::routed_relay::relay_target(payload, source_peer_id) else {
            return None;
        };
        let now = Instant::now();
        let udp_ttl = self.udp_peer_ttl;
        let udp_targets = self
            .udp_peers
            .iter()
            .filter(|(key, peer)| {
                key.group_id == group_id
                    && key.peer_id != source_peer_id
                    && peer.addr != source_addr
                    && target.is_none_or(|target| key.peer_id == target)
                    && now.duration_since(peer.last_seen) <= udp_ttl
            })
            .map(|(_, peer)| peer.addr)
            .collect::<Vec<_>>();
        let ws_targets = self
            .peers
            .iter()
            .filter(|(key, _)| {
                key.group_id == group_id
                    && key.peer_id != source_peer_id
                    && target.is_none_or(|target| key.peer_id == target)
            })
            .map(|(_, peer)| peer.writer_tx.clone())
            .collect::<Vec<_>>();
        Some((udp_targets, ws_targets))
    }

    fn remove_connection(&mut self, connection_id: u64) {
        self.connection_identities.remove(&connection_id);
        self.peers
            .retain(|_, peer| peer.connection_id != connection_id);
    }
}

async fn run_bridged_udp_relay_loop(
    udp_socket: Arc<UdpSocket>,
    state: Arc<Mutex<TcpRelayServerState>>,
    config: UdpRelayServerConfig,
    max_peers_per_group: usize,
    mut cancel_rx: broadcast::Receiver<()>,
) -> io::Result<()> {
    let mut buf = vec![0u8; MAX_PACKET_LEN];
    loop {
        tokio::select! {
            _ = cancel_rx.recv() => return Ok(()),
            received = udp_socket.recv_from(&mut buf) => {
                let (len, addr) = match received {
                    Ok(pair) => pair,
                    Err(err) if is_transient_udp_recv_error(&err) => continue,
                    Err(err) => return Err(err),
                };
                let Some(packet) = RelayPacketView::decode(&buf[..len]) else {
                    continue;
                };
                match packet {
                    RelayPacketView::Register { group_id, peer_id } => {
                        let registered = state.lock().await.register_udp(
                            group_id,
                            peer_id,
                            addr,
                            max_peers_per_group,
                        );
                        if registered {
                            if config.log_events {
                                println!("UDP relay peer registered addr={addr}");
                            }
                            let _ = udp_socket.send_to(&buf[..len], addr).await;
                        }
                    }
                    RelayPacketView::Data { group_id, peer_id, payload } => {
                        let targets = {
                            let mut guard = state.lock().await;
                            if !guard.register_udp(group_id, peer_id, addr, max_peers_per_group) {
                                None
                            } else {
                                guard.collect_udp_forward_targets(group_id, peer_id, addr, payload)
                            }
                        };
                        let Some((udp_targets, ws_targets)) = targets else {
                            continue;
                        };
                        for target in udp_targets {
                            let _ = udp_socket.send_to(&buf[..len], target).await;
                        }
                        if !ws_targets.is_empty() {
                            let frame = Arc::<[u8]>::from(&buf[..len]);
                            // Never block the shared UDP receive loop on a slow WS peer.
                            for writer_tx in ws_targets {
                                let _ = writer_tx.try_send(frame.clone());
                            }
                        }
                    }
                }
            }
        }
    }
}

async fn forward_relay_frame(
    state: &Arc<Mutex<TcpRelayServerState>>,
    group_id: &str,
    source_connection_id: u64,
    frame: Arc<[u8]>,
    payload: &[u8],
) -> RelayForwardResult {
    forward_relay_frame_with_udp(state, group_id, source_connection_id, frame, payload, None).await
}

async fn forward_relay_frame_with_udp(
    state: &Arc<Mutex<TcpRelayServerState>>,
    group_id: &str,
    source_connection_id: u64,
    frame: Arc<[u8]>,
    payload: &[u8],
    udp_socket: Option<&Arc<UdpSocket>>,
) -> RelayForwardResult {
    forward_relay_frame_with_timeout_and_udp(
        state,
        group_id,
        source_connection_id,
        frame,
        payload,
        TCP_WRITER_STALL_TIMEOUT,
        udp_socket,
    )
    .await
}

#[cfg_attr(not(test), allow(dead_code))]
async fn forward_relay_frame_with_timeout(
    state: &Arc<Mutex<TcpRelayServerState>>,
    group_id: &str,
    source_connection_id: u64,
    frame: Arc<[u8]>,
    payload: &[u8],
    stall_timeout: Duration,
) -> RelayForwardResult {
    forward_relay_frame_with_timeout_and_udp(
        state,
        group_id,
        source_connection_id,
        frame,
        payload,
        stall_timeout,
        None,
    )
    .await
}

async fn forward_relay_frame_with_timeout_and_udp(
    state: &Arc<Mutex<TcpRelayServerState>>,
    group_id: &str,
    source_connection_id: u64,
    frame: Arc<[u8]>,
    payload: &[u8],
    stall_timeout: Duration,
    udp_socket: Option<&Arc<UdpSocket>>,
) -> RelayForwardResult {
    let (mut result, deferred, udp_targets) = state.lock().await.forward_frame_with_udp(
        group_id,
        source_connection_id,
        frame.clone(),
        payload,
    );
    if let Some(udp_socket) = udp_socket {
        for addr in udp_targets {
            if udp_socket.send_to(&frame, addr).await.is_ok() {
                result.forwarded += 1;
            }
        }
    }
    let deadline = tokio::time::Instant::now() + stall_timeout;
    for peer in deferred {
        // Keep one current frame and stop reading this producer while the
        // bounded destination drains. A short socket stall is not a failed peer.
        match tokio::time::timeout_at(deadline, peer.writer_tx.send(frame.clone())).await {
            Ok(Ok(())) => {
                result.forwarded += 1;
                continue;
            }
            Ok(Err(_)) => {}
            Err(_) => {
                let retired = peer.disconnect_tx.borrow().is_some();
                if !retired {
                    let _ = peer.disconnect_tx.send(Some(RelayRetirement::SlowPeer));
                    result.disconnected_slow_peers += 1;
                }
            }
        }
        state.lock().await.remove_connection(peer.connection_id);
    }
    result
}

async fn run_tcp_relay_connection(
    stream: TcpStream,
    addr: SocketAddr,
    state: Arc<Mutex<TcpRelayServerState>>,
    config: &TcpRelayServerConfig,
) -> io::Result<()> {
    stream.set_nodelay(true)?;
    crate::net::configure_relay_tcp_stream(&stream);
    let (mut reader, writer) = stream.into_split();
    let (writer_tx, writer_rx) = mpsc::channel(config.writer_queue_capacity);
    let (disconnect_tx, mut disconnect_rx) = watch::channel(None);
    let connection_id = {
        let mut state = state.lock().await;
        state.next_connection_id()
    };
    let writer_task = tokio::spawn(run_tcp_relay_writer(writer, writer_rx));

    let result = async {
        loop {
            let frame = tokio::select! {
                _ = disconnect_rx.changed() => {
                    return Err(relay_retirement_error(*disconnect_rx.borrow()));
                }
                frame = tokio::time::timeout(config.idle_timeout, read_tcp_frame(&mut reader)) => {
                    frame.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "relay connection idle timeout"))??
                }
            };
            if let Some(reason)=*disconnect_tx.borrow() {
                return Err(relay_retirement_error(Some(reason)));
            }
            let Some(packet) = RelayPacket::decode(&frame) else {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid relay packet"));
            };
            match packet {
                RelayPacket::Register { group_id, peer_id } => {
                    if config.log_events {
                        println!("TCP relay peer registered addr={addr}");
                    }
                    state
                        .lock()
                        .await
                        .register(
                            group_id,
                            peer_id,
                            connection_id,
                            writer_tx.clone(),
                            disconnect_tx.clone(),
                            config.max_peers_per_group,
                        )
                        .map_err(|err| {
                            io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                format!("relay registration rejected: {err:?}"),
                            )
                        })?;
                }
                RelayPacket::Data {
                    group_id,
                    peer_id,
                    payload,
                } => {
                    let relay_state = state.lock().await;
                    let Some(identity) = relay_state.identity(connection_id) else {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "relay data received before registration",
                        ));
                    };
                    if identity.group_id != group_id || identity.peer_id != peer_id {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "relay data identity does not match registration",
                        ));
                    }
                    let encoded = Arc::<[u8]>::from(frame.as_slice());
                    drop(relay_state);
                    let forward = forward_relay_frame(&state, &group_id, connection_id, encoded, &payload).await;
                    // Buffered fragments can stay ready without an I/O yield.
                    // Let healthy peer writers drain before admitting the next burst.
                    tokio::task::yield_now().await;
                    if config.log_events {
                        println!(
                            "TCP relay data forwarded targets={} slow_peers={}",
                            forward.forwarded,
                            forward.disconnected_slow_peers,
                        );
                    }
                }
            }
        }
    }
    .await;

    state.lock().await.remove_connection(connection_id);
    writer_task.abort();
    result
}

async fn run_tcp_relay_writer(
    mut writer: OwnedWriteHalf,
    mut writer_rx: mpsc::Receiver<Arc<[u8]>>,
) -> io::Result<()> {
    while let Some(frame) = writer_rx.recv().await {
        write_tcp_frame(&mut writer, &frame).await?;
    }
    Ok(())
}

#[cfg_attr(not(test), allow(dead_code))]
async fn run_websocket_relay_connection<S>(
    stream: WebSocketStream<S>,
    addr: SocketAddr,
    state: Arc<Mutex<TcpRelayServerState>>,
    config: &TcpRelayServerConfig,
) -> io::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    run_websocket_relay_connection_with_udp(stream, addr, state, None, config).await
}

async fn run_websocket_relay_connection_with_udp<S>(
    stream: WebSocketStream<S>,
    addr: SocketAddr,
    state: Arc<Mutex<TcpRelayServerState>>,
    udp_socket: Option<Arc<UdpSocket>>,
    config: &TcpRelayServerConfig,
) -> io::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut writer, mut reader) = stream.split();
    let (writer_tx, mut writer_rx) = mpsc::channel::<Arc<[u8]>>(config.writer_queue_capacity);
    let (disconnect_tx, mut disconnect_rx) = watch::channel(None);
    let connection_id = state.lock().await.next_connection_id();
    let flush = tokio::sync::Notify::new();
    // A blocked peer write must not stop registration, input or heartbeat
    // reads. Poll both halves in this task, retaining the existing bound.
    let receive = async {
        let mut forwarded_since_yield = 0usize;
        loop {
            let inbound = tokio::time::timeout(config.idle_timeout, reader.next())
                .await
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::TimedOut, "relay connection idle timeout")
                })?;
            let Some(message) = inbound else {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "WebSocket relay closed",
                ));
            };
            match message.map_err(websocket_io_error)? {
                Message::Binary(frame) => {
                    handle_tcp_relay_packet_with_udp(
                        &frame,
                        addr,
                        connection_id,
                        &writer_tx,
                        &disconnect_tx,
                        &state,
                        udp_socket.as_ref(),
                        config,
                    )
                    .await?;
                    forwarded_since_yield += 1;
                    if forwarded_since_yield >= WEBSOCKET_RECV_YIELD_INTERVAL {
                        forwarded_since_yield = 0;
                        tokio::task::yield_now().await;
                    }
                }
                Message::Close(_) => return Ok(()),
                Message::Text(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "relay accepts binary WebSocket messages only",
                    ));
                }
                // Tungstenite queues the matching Pong during the read. Wake
                // the writer even when no media is queued for this peer.
                Message::Ping(_) => flush.notify_one(),
                Message::Pong(_) | Message::Frame(_) => {}
            }
        }
    };
    let send = async {
        loop {
            tokio::select! {
                _ = flush.notified() => writer.flush().await.map_err(websocket_io_error)?,
                outbound = writer_rx.recv() => {
                    let Some(first) = outbound else { return Ok(()); };
                    writer
                        .feed(Message::Binary(Bytes::from_owner(first)))
                        .await
                        .map_err(websocket_io_error)?;
                    let mut batched = 1usize;
                    while batched < WEBSOCKET_SEND_BATCH_LIMIT {
                        match writer_rx.try_recv() {
                            Ok(next) => {
                                writer
                                    .feed(Message::Binary(Bytes::from_owner(next)))
                                    .await
                                    .map_err(websocket_io_error)?;
                                batched += 1;
                            }
                            Err(_) => break,
                        }
                    }
                    writer.flush().await.map_err(websocket_io_error)?;
                }
            }
        }
    };
    let result = tokio::select! {
        _ = disconnect_rx.changed() => Err(relay_retirement_error(*disconnect_rx.borrow())),
        result = receive => result,
        result = send => result,
    };

    state.lock().await.remove_connection(connection_id);
    result
}

#[allow(clippy::too_many_arguments)]
async fn handle_tcp_relay_packet_with_udp(
    frame: &[u8],
    addr: SocketAddr,
    connection_id: u64,
    writer_tx: &mpsc::Sender<Arc<[u8]>>,
    disconnect_tx: &watch::Sender<Option<RelayRetirement>>,
    state: &Arc<Mutex<TcpRelayServerState>>,
    udp_socket: Option<&Arc<UdpSocket>>,
    config: &TcpRelayServerConfig,
) -> io::Result<()> {
    if let Some(reason) = *disconnect_tx.borrow() {
        return Err(relay_retirement_error(Some(reason)));
    }
    let Some(packet) = RelayPacketView::decode(frame) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid relay packet",
        ));
    };
    match packet {
        RelayPacketView::Register { group_id, peer_id } => {
            if config.log_events {
                println!("WebSocket relay peer registered addr={addr}");
            }
            state
                .lock()
                .await
                .register(
                    group_id.to_owned(),
                    peer_id.to_owned(),
                    connection_id,
                    writer_tx.clone(),
                    disconnect_tx.clone(),
                    config.max_peers_per_group,
                )
                .map_err(|err| {
                    io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        format!("relay registration rejected: {err:?}"),
                    )
                })
        }
        RelayPacketView::Data {
            group_id,
            peer_id,
            payload,
        } => {
            let relay_state = state.lock().await;
            let Some(identity) = relay_state.identity(connection_id) else {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "relay data received before registration",
                ));
            };
            if identity.group_id != group_id || identity.peer_id != peer_id {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "relay data identity does not match registration",
                ));
            }
            let encoded = Arc::<[u8]>::from(frame);
            drop(relay_state);
            let forward = forward_relay_frame_with_udp(
                state,
                group_id,
                connection_id,
                encoded,
                payload,
                udp_socket,
            )
            .await;
            if config.log_events {
                println!(
                    "WebSocket relay data forwarded targets={} slow_peers={}",
                    forward.forwarded, forward.disconnected_slow_peers,
                );
            }
            Ok(())
        }
    }
}

async fn send_websocket_binary<S>(stream: &mut S, frame: Vec<u8>) -> io::Result<()>
where
    S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    stream
        .send(Message::Binary(Bytes::from(frame)))
        .await
        .map_err(websocket_io_error)
}

fn relay_websocket_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .read_buffer_size(128 * 1024)
        .write_buffer_size(64 * 1024)
        .max_write_buffer_size((512 * 1024).max(2 * MAX_TCP_FRAME_LEN))
        .max_message_size(Some(MAX_TCP_FRAME_LEN))
        .max_frame_size(Some(MAX_TCP_FRAME_LEN))
}

fn websocket_io_error(err: tokio_tungstenite::tungstenite::Error) -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionAborted, err)
}

impl RelayPeerTable {
    fn new(peer_ttl: Duration) -> Self {
        Self {
            peer_ttl,
            peers: HashMap::new(),
        }
    }

    fn touch_or_register(&mut self, group_id: &str, peer_id: &str, addr: SocketAddr) {
        let now = Instant::now();
        if let Some((_, peer)) = self
            .peers
            .iter_mut()
            .find(|(key, _)| key.group_id == group_id && key.peer_id == peer_id)
        {
            peer.addr = addr;
            peer.last_seen = now;
            return;
        }
        let key = RelayPeerKey {
            group_id: group_id.to_owned(),
            peer_id: peer_id.to_owned(),
        };
        self.peers.insert(
            key,
            RelayPeerState {
                addr,
                last_seen: now,
            },
        );
    }

    fn forward_targets_for(
        &self,
        group_id: &str,
        source_peer_id: &str,
        source_addr: SocketAddr,
        target_peer_id: Option<&str>,
    ) -> Vec<SocketAddr> {
        let now = Instant::now();
        let peer_ttl = self.peer_ttl;
        self.peers
            .iter()
            .filter(|(key, peer)| {
                key.group_id == group_id
                    && key.peer_id != source_peer_id
                    && peer.addr != source_addr
                    && target_peer_id.is_none_or(|target| key.peer_id == target)
                    && now.duration_since(peer.last_seen) <= peer_ttl
            })
            .map(|(_, peer)| peer.addr)
            .collect()
    }

    fn prune_expired(&mut self) {
        let now = Instant::now();
        let peer_ttl = self.peer_ttl;
        self.peers
            .retain(|_, peer| now.duration_since(peer.last_seen) <= peer_ttl);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelayPacketView<'a> {
    Register {
        group_id: &'a str,
        peer_id: &'a str,
    },
    Data {
        group_id: &'a str,
        peer_id: &'a str,
        payload: &'a [u8],
    },
}

impl<'a> RelayPacketView<'a> {
    fn decode(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < RELAY_MAGIC.len() + 3
            || bytes.len() > MAX_TCP_FRAME_LEN
            || &bytes[..RELAY_MAGIC.len()] != RELAY_MAGIC
        {
            return None;
        }
        let packet_type = bytes[4];
        let group_len = bytes[5] as usize;
        let peer_len = bytes[6] as usize;
        let header_len = RELAY_MAGIC.len() + 3;
        let group_start = header_len;
        let peer_start = group_start.checked_add(group_len)?;
        let payload_start = peer_start.checked_add(peer_len)?;
        if payload_start > bytes.len() {
            return None;
        }

        let group_id = std::str::from_utf8(&bytes[group_start..peer_start]).ok()?;
        let peer_id = std::str::from_utf8(&bytes[peer_start..payload_start]).ok()?;
        if group_id.is_empty() || peer_id.is_empty() {
            return None;
        }

        match packet_type {
            REGISTER_PACKET => {
                if payload_start != bytes.len() {
                    return None;
                }
                Some(Self::Register { group_id, peer_id })
            }
            DATA_PACKET => Some(Self::Data {
                group_id,
                peer_id,
                payload: &bytes[payload_start..],
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RelayPacket {
    Register {
        group_id: String,
        peer_id: String,
    },
    Data {
        group_id: String,
        peer_id: String,
        payload: Vec<u8>,
    },
}

impl RelayPacket {
    fn encode_data_frame(group_id: &str, peer_id: &str, payload: &[u8]) -> Vec<u8> {
        let group = group_id.as_bytes();
        let peer = peer_id.as_bytes();
        let mut out =
            Vec::with_capacity(RELAY_MAGIC.len() + 5 + group.len() + peer.len() + payload.len());
        out.extend_from_slice(RELAY_MAGIC);
        out.push(DATA_PACKET);
        out.push(group.len() as u8);
        out.push(peer.len() as u8);
        out.extend_from_slice(group);
        out.extend_from_slice(peer);
        out.extend_from_slice(payload);
        out
    }

    fn encode(&self) -> Vec<u8> {
        let (packet_type, group_id, peer_id, payload) = match self {
            Self::Register { group_id, peer_id } => (
                REGISTER_PACKET,
                group_id.as_str(),
                peer_id.as_str(),
                &[][..],
            ),
            Self::Data {
                group_id,
                peer_id,
                payload,
            } => (
                DATA_PACKET,
                group_id.as_str(),
                peer_id.as_str(),
                payload.as_slice(),
            ),
        };

        let group = group_id.as_bytes();
        let peer = peer_id.as_bytes();
        let mut out =
            Vec::with_capacity(RELAY_MAGIC.len() + 5 + group.len() + peer.len() + payload.len());
        out.extend_from_slice(RELAY_MAGIC);
        out.push(packet_type);
        out.push(group.len() as u8);
        out.push(peer.len() as u8);
        out.extend_from_slice(group);
        out.extend_from_slice(peer);
        out.extend_from_slice(payload);
        out
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < RELAY_MAGIC.len() + 3
            || bytes.len() > MAX_TCP_FRAME_LEN
            || &bytes[..RELAY_MAGIC.len()] != RELAY_MAGIC
        {
            return None;
        }
        let packet_type = bytes[4];
        let group_len = bytes[5] as usize;
        let peer_len = bytes[6] as usize;
        let header_len = RELAY_MAGIC.len() + 3;
        let group_start = header_len;
        let peer_start = group_start.checked_add(group_len)?;
        let payload_start = peer_start.checked_add(peer_len)?;
        if payload_start > bytes.len() {
            return None;
        }

        let group_id = std::str::from_utf8(&bytes[group_start..peer_start])
            .ok()?
            .to_string();
        let peer_id = std::str::from_utf8(&bytes[peer_start..payload_start])
            .ok()?
            .to_string();
        if group_id.is_empty() || peer_id.is_empty() {
            return None;
        }

        match packet_type {
            REGISTER_PACKET => {
                if payload_start != bytes.len() {
                    return None;
                }
                Some(Self::Register { group_id, peer_id })
            }
            DATA_PACKET => Some(Self::Data {
                group_id,
                peer_id,
                payload: bytes[payload_start..].to_vec(),
            }),
            _ => None,
        }
    }
}

async fn read_tcp_frame(reader: &mut OwnedReadHalf) -> io::Result<Vec<u8>> {
    let len = reader.read_u32().await? as usize;
    if len > MAX_TCP_FRAME_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("relay frame too large: {len} bytes"),
        ));
    }
    let mut frame = vec![0u8; len];
    reader.read_exact(&mut frame).await?;
    Ok(frame)
}

async fn write_tcp_frame(writer: &mut OwnedWriteHalf, frame: &[u8]) -> io::Result<()> {
    if frame.len() > MAX_TCP_FRAME_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("relay frame too large: {} bytes", frame.len()),
        ));
    }
    writer.write_u32(frame.len() as u32).await?;
    writer.write_all(frame).await?;
    writer.flush().await
}

fn validate_relay_id(field: &'static str, value: String) -> Result<String, RelayConfigError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(RelayConfigError::EmptyId(field));
    }
    let len = value.len();
    if len > MAX_ID_LEN {
        return Err(RelayConfigError::IdTooLong { field, len });
    }
    Ok(value)
}

fn validate_websocket_relay_url(value: String) -> Result<String, RelayConfigError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(RelayConfigError::EmptyId("relay_url"));
    }
    let scheme_valid = value.starts_with("ws://") || value.starts_with("wss://");
    let authority = value
        .split_once("://")
        .map(|(_, remainder)| remainder.split('/').next().unwrap_or_default())
        .unwrap_or_default();
    if !scheme_valid
        || authority.is_empty()
        || authority.bytes().any(|byte| byte.is_ascii_whitespace())
    {
        return Err(RelayConfigError::InvalidWebSocketUrl(value));
    }
    Ok(value)
}

fn validate_udp_relay_url(value: String) -> Result<String, RelayConfigError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(RelayConfigError::EmptyId("relay_url"));
    }
    if parse_udp_relay_host_port(&value).is_none() {
        return Err(RelayConfigError::InvalidRelayUrl(value));
    }
    Ok(value)
}

fn parse_udp_relay_host_port(url: &str) -> Option<(&str, u16)> {
    let rest = url.strip_prefix("udp://")?;
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.is_empty()
        || rest.len() != authority.len()
        || authority.bytes().any(|byte| byte.is_ascii_whitespace())
    {
        return None;
    }
    let (host, port_str) = if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, after) = bracketed.split_once("]:")?;
        (host, after)
    } else {
        authority.rsplit_once(':')?
    };
    if host.is_empty() {
        return None;
    }
    let port = port_str.parse::<u16>().ok()?;
    if port == 0 {
        return None;
    }
    Some((host, port))
}

fn is_private_or_local_url(url: &str) -> bool {
    let Some((_, remainder)) = url.split_once("://") else {
        return false;
    };
    let authority = remainder.split('/').next().unwrap_or_default();
    let host = if let Some(bracketed) = authority.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".local") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        Ok(std::net::IpAddr::V6(v6)) => {
            v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local()
        }
        Err(_) => false,
    }
}

fn is_transient_udp_recv_error(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::WouldBlock
            | io::ErrorKind::Interrupted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionAborted
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{MultiplexedPacket, UdpMultiplexer};
    use protocol::ControlMessage;
    use tokio::time::timeout;

    #[test]
    fn websocket_tls_bootstraps_without_ambient_provider() {
        const CHILD: &str = "WEAVELO_TEST_FRESH_TLS_BOOTSTRAP";
        if std::env::var_os(CHILD).is_some() {
            assert!(rustls::crypto::CryptoProvider::get_default().is_none());
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let config = WebSocketRelayTunnelConfig::new(
                "127.0.0.1:0".parse().unwrap(),
                "wss://relay.invalid:443/v1/relay",
                "synthetic-group",
                "synthetic-peer",
            )
            .unwrap();
            // Binding is the production entry point; no DNS, profile or gateway is used.
            let _tunnel = runtime
                .block_on(BoundWebSocketRelayTunnel::bind(config))
                .unwrap();
            let _client = rustls::ClientConfig::builder()
                .with_root_certificates(rustls::RootCertStore::empty())
                .with_no_client_auth();
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "relay::tests::websocket_tls_bootstraps_without_ambient_provider",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "Fresh TLS bootstrap failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn retry_reason_remains_visible_until_connection_succeeds() {
        let diagnostics = RelayTunnelDiagnostics::default();
        diagnostics.retry(RelayFailure::Timeout);
        diagnostics.attempt();
        assert_eq!(
            diagnostics.snapshot().last_error,
            Some(RelayFailure::Timeout)
        );
        diagnostics.connected();
        assert_eq!(diagnostics.snapshot().last_error, None);
        let recovered = diagnostics.snapshot().last_disconnect.unwrap();
        assert_eq!(recovered.failure, RelayFailure::Timeout);
        assert_eq!(recovered.stage, RelayStage::Idle);
        diagnostics.attempt();
        assert_eq!(
            diagnostics.snapshot().last_disconnect.unwrap().failure,
            RelayFailure::Timeout
        );
    }

    #[test]
    fn active_websocket_failures_keep_the_structured_cause() {
        use tokio_tungstenite::tungstenite::{Error, error::ProtocolError};
        let wrapped =
            websocket_io_error(Error::Protocol(ProtocolError::ResetWithoutClosingHandshake));
        assert_eq!(io_failure(&wrapped), RelayFailure::ProtocolReset);
        let wrapped = websocket_io_error(Error::WriteBufferFull(Box::new(Message::Binary(
            Bytes::from_static(b"synthetic"),
        ))));
        assert_eq!(io_failure(&wrapped), RelayFailure::Backpressure);
    }

    #[tokio::test]
    async fn websocket_diagnostics_ignore_unsolicited_pong_and_observe_data_roundtrip() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sent, ready) = tokio::sync::oneshot::channel();
        let (resume, allowed) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
            ws.send(Message::Pong(Bytes::from_static(b"unsolicited")))
                .await
                .unwrap();
            sent.send(()).unwrap();
            allowed.await.unwrap();
            while let Some(Ok(message)) = ws.next().await {
                match message {
                    Message::Ping(payload) => {
                        if ws.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Message::Binary(frame)
                        if matches!(
                            RelayPacket::decode(&frame),
                            Some(RelayPacket::Data { .. })
                        ) =>
                    {
                        if ws.send(Message::Binary(frame)).await.is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        });
        let config = WebSocketRelayTunnelConfig::new(
            "127.0.0.1:0".parse().unwrap(),
            format!("ws://{address}/relay"),
            "synthetic-group",
            "synthetic-peer",
        )
        .unwrap()
        .with_heartbeat(Duration::from_millis(50), Duration::from_secs(2));
        let tunnel = BoundWebSocketRelayTunnel::bind(config).await.unwrap();
        let endpoint = tunnel.local_addr().unwrap();
        let diagnostics = tunnel.diagnostics();
        let (cancel, rx) = broadcast::channel(1);
        let task = tokio::spawn(tunnel.run(rx));
        ready.await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(diagnostics.snapshot().pongs_received, 0);
        resume.send(()).unwrap();
        let local = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        local.send_to(b"synthetic-payload", endpoint).await.unwrap();
        let mut received = [0; 64];
        let count = timeout(Duration::from_secs(2), local.recv(&mut received))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&received[..count], b"synthetic-payload");
        timeout(Duration::from_secs(2), async {
            while diagnostics.snapshot().pongs_received == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let state = diagnostics.snapshot();
        assert_eq!(state.data_frames_sent, 1);
        assert_eq!(state.data_frames_received, 1);
        cancel.send(()).unwrap();
        task.await.unwrap().unwrap();
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn websocket_diagnostics_distinguish_registration_pong_and_reconnect() {
        use crate::relay_status::RelayConnectionState;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (registered_tx, registered_rx) = tokio::sync::oneshot::channel();
        let (allow_tx, allow_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
            let Message::Binary(frame) = ws.next().await.unwrap().unwrap() else {
                panic!("registration missing")
            };
            assert!(matches!(
                RelayPacket::decode(&frame),
                Some(RelayPacket::Register { .. })
            ));
            registered_tx.send(()).unwrap();
            allow_rx.await.unwrap();
            // Reading alone must not be mistaken for receiving a Pong on the client.
            while let Some(Ok(message)) = ws.next().await {
                if let Message::Ping(payload) = message {
                    ws.send(Message::Pong(payload)).await.unwrap();
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
            drop(ws);
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
            while let Some(Ok(message)) = ws.next().await {
                if let Message::Ping(payload) = message {
                    if ws.send(Message::Pong(payload)).await.is_err() {
                        break;
                    }
                }
            }
        });
        let config = WebSocketRelayTunnelConfig::new(
            "127.0.0.1:0".parse().unwrap(),
            format!("ws://{address}/relay"),
            "synthetic-group",
            "synthetic-peer",
        )
        .unwrap()
        .with_heartbeat(Duration::from_millis(20), Duration::from_millis(200));
        let tunnel = BoundWebSocketRelayTunnel::bind(config).await.unwrap();
        let diagnostics = tunnel.diagnostics();
        let (cancel, rx) = broadcast::channel(2);
        let task = tokio::spawn(tunnel.run(rx));
        timeout(Duration::from_secs(2), registered_rx)
            .await
            .unwrap()
            .unwrap();
        let before_pong = diagnostics.snapshot();
        assert!(before_pong.registrations_sent > 0);
        assert_eq!(before_pong.pongs_received, 0);
        assert!(before_pong.last_pong_age_ms.is_none());
        allow_tx.send(()).unwrap();
        timeout(Duration::from_secs(4), async {
            loop {
                let state = diagnostics.snapshot();
                if state.attempts >= 2 && state.registrations_sent >= 3 && state.pongs_received >= 2
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        cancel.send(()).unwrap();
        timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(diagnostics.snapshot().state, RelayConnectionState::Stopped);
        assert!(diagnostics.snapshot().last_pong_age_ms.is_none());
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn connection_stage_distinguishes_silent_tls_from_silent_websocket() {
        for (scheme, expected) in [("wss", RelayStage::Tls), ("ws", RelayStage::Websocket)] {
            crate::init_crypto_provider();
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 4096];
                while socket.read(&mut buf).await.unwrap_or(0) > 0 {}
            });
            let diagnostics = Arc::new(RelayTunnelDiagnostics::default());
            diagnostics.attempt();
            assert!(
                timeout(
                    Duration::from_millis(100),
                    connect_observed_websocket(
                        &format!("{scheme}://{address}/relay"),
                        &diagnostics
                    )
                )
                .await
                .is_err()
            );
            diagnostics.retry(RelayFailure::Timeout);
            let value = diagnostics.snapshot();
            assert_eq!(value.last_failure_stage, Some(expected));
            assert_eq!(value.dns_results, 1);
            assert_eq!(value.tcp_connections, 1);
            assert_eq!(value.tls_handshakes, 0);
            assert_eq!(value.websocket_upgrades, 0);
            if scheme == "wss" {
                assert!(value.tls_bytes_sent > 0);
                assert_eq!(value.tls_bytes_received, 0);
            }
            timeout(Duration::from_secs(1), server)
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[tokio::test]
    async fn websocket_rejection_exports_status_without_headers_or_payload() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = socket.read(&mut buf).await;
            socket.write_all(b"HTTP/1.1 403 Forbidden\r\nX-Private: credential-canary\r\nContent-Length: 0\r\n\r\n").await.unwrap();
        });
        let diagnostics = Arc::new(RelayTunnelDiagnostics::default());
        diagnostics.attempt();
        let failure = match connect_observed_websocket(
            &format!("ws://{address}/relay"),
            &diagnostics,
        )
        .await
        {
            Err(failure) => failure,
            Ok(_) => panic!("unexpected upgrade"),
        };
        diagnostics.retry(failure);
        let value = diagnostics.snapshot();
        assert_eq!(value.last_failure_stage, Some(RelayStage::Websocket));
        assert_eq!(value.http_status, Some(403));
        assert!(
            !serde_json::to_string(&value)
                .unwrap()
                .contains("credential-canary")
        );
        server.await.unwrap();
    }

    #[test]
    fn dns_comparison_keeps_address_hashes_and_benchmark_count_only() {
        let diagnostics = RelayTunnelDiagnostics::default();
        let addresses = [
            "198.18.1.2:8443",
            "198.19.1.2:8443",
            "198.20.1.2:8443",
            "198.17.1.2:8443",
        ]
        .map(|address| address.parse().unwrap());
        diagnostics.dns_resolved(&addresses);
        let value = diagnostics.snapshot();
        assert_eq!(value.dns_results, 4);
        assert_eq!(value.dns_benchmark_count, 2);
        assert_eq!(value.dns_address_sha256.len(), 4);
        assert!(value.dns_address_sha256.iter().all(|hash| hash.len() == 64));
        let json = serde_json::to_string(&value).unwrap();
        assert!(!json.contains("198."));
        assert!(!json.contains("8443"));
    }

    #[test]
    fn relay_packet_roundtrips() {
        let packet = RelayPacket::Data {
            group_id: "group".to_string(),
            peer_id: "peer-a".to_string(),
            payload: vec![1, 2, 3, 4],
        };

        let encoded = packet.encode();

        assert_eq!(RelayPacket::decode(&encoded), Some(packet));
        assert_eq!(RelayPacket::decode(b"not-relay"), None);
    }

    #[test]
    fn relay_group_capability_is_secret_and_channel_scoped() {
        let control = derive_relay_group_id("network-a", "secret-a", "control").unwrap();
        let repeated = derive_relay_group_id("network-a", "secret-a", "control").unwrap();
        let discovery = derive_relay_group_id("network-a", "secret-a", "discovery").unwrap();
        let other_secret = derive_relay_group_id("network-a", "secret-b", "control").unwrap();

        assert_eq!(control, repeated);
        assert_eq!(control.len(), 64);
        assert!(control.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(!control.contains("network-a"));
        assert!(!control.contains("secret-a"));
        assert_ne!(control, discovery);
        assert_ne!(control, other_secret);
    }

    #[test]
    fn websocket_relay_url_requires_ws_or_wss() {
        assert!(
            WebSocketRelayTunnelConfig::new(
                "127.0.0.1:0".parse().unwrap(),
                "wss://relay.example.test:8443/relay",
                "group",
                "peer",
            )
            .is_ok()
        );
        assert!(matches!(
            WebSocketRelayTunnelConfig::new(
                "127.0.0.1:0".parse().unwrap(),
                "https://relay.example.test/relay",
                "group",
                "peer",
            ),
            Err(RelayConfigError::InvalidWebSocketUrl(_))
        ));
    }

    #[test]
    fn websocket_relay_heartbeat_timeout_never_precedes_the_probe_interval() {
        let config = WebSocketRelayTunnelConfig::new(
            "127.0.0.1:0".parse().unwrap(),
            "wss://relay.example.test/relay",
            "group",
            "peer",
        )
        .unwrap()
        .with_heartbeat(Duration::from_secs(5), Duration::from_secs(2));

        assert_eq!(config.heartbeat_interval, Duration::from_secs(5));
        assert_eq!(config.heartbeat_timeout, Duration::from_secs(5));
    }

    #[tokio::test]
    async fn websocket_relay_detects_a_silent_half_open_connection() {
        let config = WebSocketRelayTunnelConfig::new(
            "127.0.0.1:0".parse().unwrap(),
            "ws://relay.example.test/relay",
            "group",
            "peer",
        )
        .unwrap()
        .with_heartbeat(Duration::from_millis(20), Duration::from_millis(60));
        let tunnel = BoundWebSocketRelayTunnel::bind(config)
            .await
            .expect("tunnel should bind");
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let client_stream = WebSocketStream::from_raw_socket(
            client_io,
            tokio_tungstenite::tungstenite::protocol::Role::Client,
            None,
        )
        .await;
        let _silent_server = WebSocketStream::from_raw_socket(
            server_io,
            tokio_tungstenite::tungstenite::protocol::Role::Server,
            None,
        )
        .await;
        let (_cancel_tx, cancel_rx) = broadcast::channel(1);
        let mut cancel_rx = cancel_rx;

        let err = timeout(
            Duration::from_secs(1),
            tunnel.run_connected(client_stream, &mut cancel_rx),
        )
        .await
        .expect("heartbeat timeout should be bounded")
        .expect_err("silent connection should be rejected");

        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert_eq!(err.to_string(), "WebSocket relay heartbeat timed out");
    }

    #[test]
    fn retired_relay_connection_cannot_reclaim_a_replacement_peer() {
        let mut state = TcpRelayServerState::default();
        let (old_writer, _old_frames) = mpsc::channel(1);
        let (old_disconnect, old_retired) = watch::channel(None);
        let (new_writer, _new_frames) = mpsc::channel(1);
        let (new_disconnect, new_retired) = watch::channel(None);
        state
            .register(
                "group".into(),
                "peer".into(),
                1,
                old_writer.clone(),
                old_disconnect.clone(),
                2,
            )
            .unwrap();
        state
            .register(
                "group".into(),
                "peer".into(),
                2,
                new_writer,
                new_disconnect,
                2,
            )
            .unwrap();
        assert_eq!(*old_retired.borrow(), Some(RelayRetirement::Replaced));
        let reclaim = state.register(
            "group".into(),
            "peer".into(),
            1,
            old_writer,
            old_disconnect,
            2,
        );
        assert_eq!(
            reclaim,
            Err(RelayRegistrationError::ConnectionRetired),
            "retired connection reclaimed the replacement peer"
        );
        assert_eq!(state.identity(2).unwrap().peer_id, "peer");
        assert!(state.identity(1).is_none());
        assert!(new_retired.borrow().is_none());
    }

    #[test]
    fn tcp_connection_cannot_change_identity_after_registration() {
        let mut state = TcpRelayServerState::default();
        let connection_id = state.next_connection_id();
        let (writer_tx, _writer_rx) = mpsc::channel(4);
        let (disconnect_tx, _disconnect_rx) = tokio::sync::watch::channel(None);

        state
            .register(
                "group-a".to_string(),
                "peer-a".to_string(),
                connection_id,
                writer_tx.clone(),
                disconnect_tx.clone(),
                4,
            )
            .unwrap();
        state
            .register(
                "group-a".to_string(),
                "peer-a".to_string(),
                connection_id,
                writer_tx.clone(),
                disconnect_tx.clone(),
                4,
            )
            .unwrap();

        let err = state
            .register(
                "group-b".to_string(),
                "peer-a".to_string(),
                connection_id,
                writer_tx,
                disconnect_tx,
                4,
            )
            .unwrap_err();
        assert_eq!(err, RelayRegistrationError::ConnectionIdentityChanged);
    }

    #[test]
    fn tcp_group_peer_limit_is_enforced() {
        let mut state = TcpRelayServerState::default();
        let (writer_a, _reader_a) = mpsc::channel(4);
        let (disconnect_a, _disconnect_reader_a) = tokio::sync::watch::channel(None);
        let first_connection = state.next_connection_id();
        state
            .register(
                "group".to_string(),
                "peer-a".to_string(),
                first_connection,
                writer_a,
                disconnect_a,
                1,
            )
            .unwrap();

        let (writer_b, _reader_b) = mpsc::channel(4);
        let (disconnect_b, _disconnect_reader_b) = tokio::sync::watch::channel(None);
        let second_connection = state.next_connection_id();
        let err = state
            .register(
                "group".to_string(),
                "peer-b".to_string(),
                second_connection,
                writer_b,
                disconnect_b,
                1,
            )
            .unwrap_err();

        assert_eq!(err, RelayRegistrationError::GroupPeerLimitReached);
    }

    #[test]
    fn routed_frame_does_not_disconnect_an_unrelated_slow_peer() {
        let mut state = TcpRelayServerState::default();
        let mut register = |group: &str, peer: &str, capacity| {
            let connection = state.next_connection_id();
            let (writer, reader) = mpsc::channel(capacity);
            let (disconnect, disconnected) = watch::channel(None);
            state
                .register(
                    group.into(),
                    peer.into(),
                    connection,
                    writer.clone(),
                    disconnect,
                    8,
                )
                .unwrap();
            (connection, writer, reader, disconnected)
        };
        let (source, _, _, _) = register("control-group", "source", 2);
        let (_, _, mut target, _) = register("control-group", "target", 2);
        let (_, unrelated, _unrelated_reader, disconnected) =
            register("control-group", "unrelated", 1);
        let (_, _, mut other_group, _) = register("other-group", "target", 2);
        unrelated.try_send(Arc::<[u8]>::from([7])).unwrap();
        // Existing RPMX v1 routing metadata. The relay does not hold the end-to-end
        // MAC key; recipient selection must never stand in for receiver authentication.
        let mut payload = b"RPMX\x01\x00".to_vec();
        payload.extend_from_slice(&[0; 16]);
        payload.extend_from_slice(&[6, 6]);
        payload.extend_from_slice(b"sourcetarget");
        payload.extend_from_slice(&5u16.to_be_bytes());
        payload.extend_from_slice(b"video");
        payload.extend_from_slice(&[0; 32]);
        let frame = Arc::<[u8]>::from(
            RelayPacket::Data {
                group_id: "control-group".into(),
                peer_id: "source".into(),
                payload: payload.clone(),
            }
            .encode(),
        );
        assert_eq!(
            state
                .forward_frame("control-group", source, frame.clone(), &payload)
                .0,
            RelayForwardResult {
                forwarded: 1,
                disconnected_slow_peers: 0
            }
        );
        assert_eq!(target.try_recv().unwrap().as_ref(), frame.as_ref());
        let rejected = RelayForwardResult {
            forwarded: 0,
            disconnected_slow_peers: 0,
        };
        assert_eq!(
            state
                .forward_frame("other-group", source, frame.clone(), &payload)
                .0,
            rejected
        );
        assert_eq!(
            state
                .forward_frame("control-group", u64::MAX, frame.clone(), &payload)
                .0,
            rejected
        );
        let mut spoofed = payload.clone();
        spoofed[24] = b'x';
        assert_eq!(
            state
                .forward_frame("control-group", source, frame.clone(), &spoofed)
                .0,
            rejected
        );
        let mut unknown_target = payload.clone();
        unknown_target[30] = b'x';
        assert_eq!(
            state
                .forward_frame("control-group", source, frame.clone(), &unknown_target)
                .0,
            rejected
        );
        assert_eq!(
            state
                .forward_frame("control-group", source, frame, &payload[..4])
                .0,
            rejected
        );
        assert!(target.try_recv().is_err());
        assert!(
            disconnected.borrow().is_none(),
            "another device's stream disconnected this peer"
        );
        assert!(other_group.try_recv().is_err());
    }

    #[tokio::test]
    async fn tcp_forwarding_queue_is_bounded_and_waits_only_for_a_short_stall() {
        let mut state = TcpRelayServerState::default();
        let source_connection = state.next_connection_id();
        let target_connection = state.next_connection_id();
        let (source_writer, _source_reader) = mpsc::channel(1);
        state
            .register(
                "group".into(),
                "source".into(),
                source_connection,
                source_writer,
                watch::channel(None).0,
                2,
            )
            .unwrap();
        let (writer_tx, mut writer_rx) = mpsc::channel(1);
        let (disconnect_tx, disconnect_rx) = tokio::sync::watch::channel(None);
        state
            .register(
                "group".to_string(),
                "target".to_string(),
                target_connection,
                writer_tx,
                disconnect_tx,
                2,
            )
            .unwrap();

        let state = Arc::new(Mutex::new(state));
        let frame = std::sync::Arc::<[u8]>::from([1, 2, 3]);
        assert_eq!(
            forward_relay_frame(&state, "group", source_connection, frame.clone(), &[]).await,
            RelayForwardResult {
                forwarded: 1,
                disconnected_slow_peers: 0,
            }
        );
        let blocked_state = state.clone();
        let second = frame.clone();
        let waiting = tokio::spawn(async move {
            forward_relay_frame(&blocked_state, "group", source_connection, second, &[]).await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiting.is_finished());
        assert!(disconnect_rx.borrow().is_none());
        assert!(
            state.lock().await.identity(source_connection).is_some(),
            "a slow writer must not hold the routing mutex"
        );
        assert_eq!(writer_rx.recv().await.unwrap().as_ref(), frame.as_ref());
        assert_eq!(
            waiting.await.unwrap(),
            RelayForwardResult {
                forwarded: 1,
                disconnected_slow_peers: 0
            }
        );
        assert_eq!(writer_rx.len(), 1);
        assert_eq!(
            forward_relay_frame_with_timeout(
                &state,
                "group",
                source_connection,
                frame,
                &[],
                Duration::from_millis(250),
            )
            .await,
            RelayForwardResult {
                forwarded: 0,
                disconnected_slow_peers: 1,
            }
        );
        assert_eq!(*disconnect_rx.borrow(), Some(RelayRetirement::SlowPeer));
    }

    #[tokio::test]
    async fn relay_tunnels_forward_existing_udp_protocol_both_ways() {
        let (cancel_tx, _) = broadcast::channel(1);

        let server =
            BoundUdpRelayServer::bind(UdpRelayServerConfig::new("127.0.0.1:0".parse().unwrap()))
                .await
                .expect("server should bind");
        let relay_addr = server.local_addr().expect("server addr");
        let server_task = tokio::spawn(server.run(cancel_tx.subscribe()));

        let host_app = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("host app should bind");
        let host_app_addr = host_app.local_addr().expect("host app addr");
        let (host_sender, host_receiver) = host_app.split();

        let client_app = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("client app should bind");
        let (client_sender, client_receiver) = client_app.split();

        let host_tunnel = BoundUdpRelayTunnel::bind(
            UdpRelayTunnelConfig::new("127.0.0.1:0".parse().unwrap(), relay_addr, "group", "host")
                .unwrap()
                .with_local_target_addr(host_app_addr),
        )
        .await
        .expect("host tunnel should bind");
        let host_tunnel_addr = host_tunnel.local_addr().expect("host tunnel addr");
        let host_tunnel_task = tokio::spawn(host_tunnel.run(cancel_tx.subscribe()));

        let client_tunnel = BoundUdpRelayTunnel::bind(
            UdpRelayTunnelConfig::new(
                "127.0.0.1:0".parse().unwrap(),
                relay_addr,
                "group",
                "client",
            )
            .unwrap(),
        )
        .await
        .expect("client tunnel should bind");
        let client_tunnel_addr = client_tunnel.local_addr().expect("client tunnel addr");
        let client_tunnel_task = tokio::spawn(client_tunnel.run(cancel_tx.subscribe()));

        tokio::time::sleep(Duration::from_millis(50)).await;
        client_sender
            .send_control(&ControlMessage::Heartbeat, client_tunnel_addr)
            .await
            .expect("client send should succeed");

        let received = timeout(Duration::from_secs(2), host_receiver.recv())
            .await
            .expect("host should receive through relay")
            .expect("host packet should decode");
        let MultiplexedPacket::Control(ControlMessage::Heartbeat, host_seen_addr) = received else {
            panic!("host received unexpected packet: {received:?}");
        };
        assert_eq!(host_seen_addr, host_tunnel_addr);

        host_sender
            .send_control(&ControlMessage::StopStream, host_seen_addr)
            .await
            .expect("host reply should send");

        let received = timeout(Duration::from_secs(2), client_receiver.recv())
            .await
            .expect("client should receive through relay")
            .expect("client packet should decode");
        let MultiplexedPacket::Control(ControlMessage::StopStream, _) = received else {
            panic!("client received unexpected packet: {received:?}");
        };

        let _ = cancel_tx.send(());
        server_task.await.expect("server task should join").unwrap();
        host_tunnel_task
            .await
            .expect("host tunnel task should join")
            .unwrap();
        client_tunnel_task
            .await
            .expect("client tunnel task should join")
            .unwrap();
    }

    #[tokio::test]
    async fn tcp_relay_tunnels_forward_existing_udp_protocol_both_ways() {
        let (cancel_tx, _) = broadcast::channel(1);

        let server =
            BoundTcpRelayServer::bind(TcpRelayServerConfig::new("127.0.0.1:0".parse().unwrap()))
                .await
                .expect("server should bind");
        let relay_addr = server.local_addr().expect("server addr");
        let server_task = tokio::spawn(server.run(cancel_tx.subscribe()));

        let host_app = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("host app should bind");
        let host_app_addr = host_app.local_addr().expect("host app addr");
        let (host_sender, host_receiver) = host_app.split();

        let client_app = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("client app should bind");
        let (client_sender, client_receiver) = client_app.split();

        let host_tunnel = BoundTcpRelayTunnel::bind(
            TcpRelayTunnelConfig::new("127.0.0.1:0".parse().unwrap(), relay_addr, "group", "host")
                .unwrap()
                .with_local_target_addr(host_app_addr),
        )
        .await
        .expect("host tunnel should bind");
        let host_tunnel_addr = host_tunnel.local_addr().expect("host tunnel addr");
        let host_tunnel_task = tokio::spawn(host_tunnel.run(cancel_tx.subscribe()));

        let client_tunnel = BoundTcpRelayTunnel::bind(
            TcpRelayTunnelConfig::new(
                "127.0.0.1:0".parse().unwrap(),
                relay_addr,
                "group",
                "client",
            )
            .unwrap(),
        )
        .await
        .expect("client tunnel should bind");
        let client_tunnel_addr = client_tunnel.local_addr().expect("client tunnel addr");
        let client_tunnel_task = tokio::spawn(client_tunnel.run(cancel_tx.subscribe()));

        tokio::time::sleep(Duration::from_millis(50)).await;
        client_sender
            .send_control(&ControlMessage::Heartbeat, client_tunnel_addr)
            .await
            .expect("client send should succeed");

        let received = timeout(Duration::from_secs(2), host_receiver.recv())
            .await
            .expect("host should receive through relay")
            .expect("host packet should decode");
        let MultiplexedPacket::Control(ControlMessage::Heartbeat, host_seen_addr) = received else {
            panic!("host received unexpected packet: {received:?}");
        };
        assert_eq!(host_seen_addr, host_tunnel_addr);

        host_sender
            .send_control(&ControlMessage::StopStream, host_seen_addr)
            .await
            .expect("host reply should send");

        let received = timeout(Duration::from_secs(2), client_receiver.recv())
            .await
            .expect("client should receive through relay")
            .expect("client packet should decode");
        let MultiplexedPacket::Control(ControlMessage::StopStream, _) = received else {
            panic!("client received unexpected packet: {received:?}");
        };

        let _ = cancel_tx.send(());
        server_task.await.expect("server task should join").unwrap();
        host_tunnel_task
            .await
            .expect("host tunnel task should join")
            .unwrap();
        client_tunnel_task
            .await
            .expect("client tunnel task should join")
            .unwrap();
    }

    #[tokio::test]
    async fn websocket_reads_progress_while_a_local_datagram_write_is_backpressured() {
        use tokio_tungstenite::tungstenite::protocol::Role;
        let (local, remote) = tokio::io::duplex(4096);
        let client = WebSocketStream::from_raw_socket(local, Role::Client, None).await;
        let mut server = WebSocketStream::from_raw_socket(remote, Role::Server, None).await;
        let app = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let tunnel = BoundWebSocketRelayTunnel::bind(
            WebSocketRelayTunnelConfig::new(
                "127.0.0.1:0".parse().unwrap(),
                "ws://127.0.0.1/relay",
                "duplex-fixture",
                "client",
            )
            .unwrap()
            .with_local_target_addr(app.local_addr().unwrap()),
        )
        .await
        .unwrap();
        let address = tunnel.local_addr().unwrap();
        let (cancel, mut cancelled) = broadcast::channel(1);
        let task = tokio::spawn(async move { tunnel.run_connected(client, &mut cancelled).await });
        assert!(matches!(server.next().await, Some(Ok(Message::Binary(_)))));
        // Both legal UDP records exceed a constrained link's current writable
        // space. The peer writes before reading, as a busy full-duplex relay can.
        let outgoing = vec![41; 8 * 1024];
        let incoming = vec![73; 8 * 1024];
        app.send_to(&outgoing, address).await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        let exchange = async {
            server
                .send(Message::Binary(
                    RelayPacket::Data {
                        group_id: "duplex-fixture".into(),
                        peer_id: "server".into(),
                        payload: incoming.clone(),
                    }
                    .encode()
                    .into(),
                ))
                .await
                .unwrap();
            loop {
                let message = server.next().await.unwrap().unwrap();
                if let Message::Binary(frame) = message
                    && let Some(RelayPacket::Data { payload, .. }) = RelayPacket::decode(&frame)
                {
                    assert_eq!(payload, outgoing);
                    break;
                }
            }
            let mut bytes = vec![0; MAX_PACKET_LEN];
            let (n, _) = app.recv_from(&mut bytes).await.unwrap();
            assert_eq!(&bytes[..n], incoming);
        };
        let result = timeout(Duration::from_secs(2), exchange).await;
        let _ = cancel.send(());
        task.abort();
        let _ = task.await;
        assert!(
            result.is_ok(),
            "relay reads stopped behind an unrelated pending write"
        );
    }

    #[tokio::test]
    async fn websocket_server_reads_progress_while_its_peer_write_is_backpressured() {
        use tokio_tungstenite::tungstenite::protocol::Role;
        let (local, remote) = tokio::io::duplex(4096);
        let mut client = WebSocketStream::from_raw_socket(local, Role::Client, None).await;
        let server = WebSocketStream::from_raw_socket(remote, Role::Server, None).await;
        let state = Arc::new(Mutex::new(TcpRelayServerState::default()));
        let server_state = state.clone();
        let task = tokio::spawn(async move {
            run_websocket_relay_connection(
                server,
                "127.0.0.1:12345".parse().unwrap(),
                server_state,
                &TcpRelayServerConfig::new("127.0.0.1:0".parse().unwrap()),
            )
            .await
        });
        client
            .send(Message::Binary(
                RelayPacket::Register {
                    group_id: "server-duplex".into(),
                    peer_id: "client".into(),
                }
                .encode()
                .into(),
            ))
            .await
            .unwrap();
        let writer = timeout(Duration::from_secs(1), async {
            loop {
                if let Some(peer) = state.lock().await.peers.values().next() {
                    break peer.writer_tx.clone();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let incoming = vec![73; 8 * 1024];
        let outgoing = vec![41; 8 * 1024];
        writer
            .send(Arc::from(
                RelayPacket::Data {
                    group_id: "server-duplex".into(),
                    peer_id: "other".into(),
                    payload: incoming.clone(),
                }
                .encode(),
            ))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        let exchange = async {
            client
                .send(Message::Binary(
                    RelayPacket::Data {
                        group_id: "server-duplex".into(),
                        peer_id: "client".into(),
                        payload: outgoing,
                    }
                    .encode()
                    .into(),
                ))
                .await
                .unwrap();
            let Message::Binary(frame) = client.next().await.unwrap().unwrap() else {
                panic!("expected forwarded data")
            };
            let Some(RelayPacket::Data { payload, .. }) = RelayPacket::decode(&frame) else {
                panic!("expected relay data")
            };
            assert_eq!(payload, incoming);
            // An otherwise idle split writer must also flush automatic Pong.
            let probe = Bytes::from_static(b"server-reader-ping");
            client.send(Message::Ping(probe.clone())).await.unwrap();
            assert_eq!(client.next().await.unwrap().unwrap(), Message::Pong(probe));
        };
        let result = timeout(Duration::from_secs(2), exchange).await;
        task.abort();
        let _ = task.await;
        assert!(
            result.is_ok(),
            "server reads stopped behind a pending peer write"
        );
    }

    #[tokio::test]
    async fn websocket_split_reader_flushes_server_ping_without_local_udp_traffic() {
        use tokio_tungstenite::tungstenite::protocol::Role;
        let (local, remote) = tokio::io::duplex(4096);
        let client = WebSocketStream::from_raw_socket(local, Role::Client, None).await;
        let mut server = WebSocketStream::from_raw_socket(remote, Role::Server, None).await;
        let tunnel = BoundWebSocketRelayTunnel::bind(
            WebSocketRelayTunnelConfig::new(
                "127.0.0.1:0".parse().unwrap(),
                "ws://127.0.0.1/relay",
                "ping-fixture",
                "client",
            )
            .unwrap(),
        )
        .await
        .unwrap();
        let (cancel, mut cancelled) = broadcast::channel(1);
        let task = tokio::spawn(async move { tunnel.run_connected(client, &mut cancelled).await });
        assert!(matches!(server.next().await, Some(Ok(Message::Binary(_)))));
        let probe = Bytes::from_static(b"server-ping");
        server.send(Message::Ping(probe.clone())).await.unwrap();
        let response = timeout(Duration::from_secs(1), async {
            loop {
                match server.next().await.unwrap().unwrap() {
                    Message::Pong(payload) if payload == probe => break,
                    Message::Ping(_) => server.flush().await.unwrap(),
                    _ => {}
                }
            }
        })
        .await;
        let _ = cancel.send(());
        let stopped = timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
        assert!(
            response.is_ok(),
            "an idle UDP tunnel must flush the WebSocket Pong"
        );
        assert!(stopped.is_ok());
    }

    #[tokio::test]
    async fn websocket_cancel_interrupts_a_backpressured_local_write() {
        use tokio_tungstenite::tungstenite::protocol::Role;
        let (local, remote) = tokio::io::duplex(4096);
        let client = WebSocketStream::from_raw_socket(local, Role::Client, None).await;
        let mut server = WebSocketStream::from_raw_socket(remote, Role::Server, None).await;
        let app = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let tunnel = BoundWebSocketRelayTunnel::bind(
            WebSocketRelayTunnelConfig::new(
                "127.0.0.1:0".parse().unwrap(),
                "ws://127.0.0.1/relay",
                "cancel-fixture",
                "client",
            )
            .unwrap(),
        )
        .await
        .unwrap();
        let address = tunnel.local_addr().unwrap();
        let (cancel, mut cancelled) = broadcast::channel(1);
        let task = tokio::spawn(async move { tunnel.run_connected(client, &mut cancelled).await });
        assert!(matches!(server.next().await, Some(Ok(Message::Binary(_)))));
        app.send_to(&vec![41; 8 * 1024], address).await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        let _ = cancel.send(());
        assert!(
            timeout(Duration::from_secs(1), task)
                .await
                .expect("cancel waited for the blocked writer")
                .unwrap()
                .is_ok()
        );
    }

    #[tokio::test]
    async fn websocket_transient_receiver_stall_preserves_bounded_frame_order() {
        use tokio_tungstenite::tungstenite::protocol::Role;
        let state = Arc::new(Mutex::new(TcpRelayServerState::default()));
        let mut clients = Vec::new();
        let mut servers = Vec::new();
        for (index, peer) in ["source", "receiver"].into_iter().enumerate() {
            let (local, remote) = tokio::io::duplex(4096);
            let mut client = WebSocketStream::from_raw_socket(local, Role::Client, None).await;
            let server = WebSocketStream::from_raw_socket(remote, Role::Server, None).await;
            let server_state = state.clone();
            servers.push(tokio::spawn(async move {
                run_websocket_relay_connection(
                    server,
                    "127.0.0.1:12345".parse().unwrap(),
                    server_state,
                    &TcpRelayServerConfig::new("127.0.0.1:0".parse().unwrap()),
                )
                .await
            }));
            client
                .send(Message::Binary(
                    RelayPacket::Register {
                        group_id: "transient-stall".into(),
                        peer_id: peer.into(),
                    }
                    .encode()
                    .into(),
                ))
                .await
                .unwrap();
            timeout(Duration::from_secs(1), async {
                while state.lock().await.peers.len() != index + 1 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            clients.push(client);
        }
        let mut receiver = clients.pop().unwrap();
        let mut sender = clients.pop().unwrap();
        let sending = tokio::spawn(async move {
            for sequence in 0..256u32 {
                let mut payload = vec![73; 1400];
                payload[..4].copy_from_slice(&sequence.to_be_bytes());
                sender
                    .feed(Message::Binary(
                        RelayPacket::Data {
                            group_id: "transient-stall".into(),
                            peer_id: "source".into(),
                            payload,
                        }
                        .encode()
                        .into(),
                    ))
                    .await
                    .map_err(|error| error.to_string())?;
            }
            sender.flush().await.map_err(|error| error.to_string())?;
            Ok::<_, String>(sender)
        });
        tokio::time::sleep(Duration::from_millis(80)).await;
        let delivered = timeout(Duration::from_secs(3), async {
            for expected in 0..256u32 {
                let message = receiver
                    .next()
                    .await
                    .ok_or("receiver disconnected")?
                    .map_err(|error| error.to_string())?;
                let Message::Binary(bytes) = message else {
                    return Err("unexpected frame".to_owned());
                };
                let Some(RelayPacket::Data { payload, .. }) = RelayPacket::decode(&bytes) else {
                    return Err("invalid relay frame".to_owned());
                };
                if payload.len() != 1400
                    || u32::from_be_bytes(payload[..4].try_into().unwrap()) != expected
                {
                    return Err("compressed frame order or content changed".to_owned());
                }
            }
            Ok::<_, String>(256)
        })
        .await;
        for server in servers {
            server.abort();
        }
        sending.abort();
        assert!(
            matches!(delivered, Ok(Ok(256))),
            "short receiver stalls must retain every frame: {delivered:?}"
        );
    }

    #[tokio::test]
    async fn websocket_video_fragment_burst_keeps_a_draining_peer_connected() {
        let (cancel, _) = broadcast::channel(1);
        let server =
            BoundTcpRelayServer::bind(TcpRelayServerConfig::new("127.0.0.1:0".parse().unwrap()))
                .await
                .unwrap();
        let url = format!("ws://{}/relay", server.local_addr().unwrap());
        let server_task = tokio::spawn(server.run_websocket(cancel.subscribe()));
        let (mut sender, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        let (mut receiver, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        for (socket, peer) in [(&mut sender, "sender"), (&mut receiver, "receiver")] {
            socket
                .send(Message::Binary(
                    RelayPacket::Register {
                        group_id: "fragment-burst".into(),
                        peer_id: peer.into(),
                    }
                    .encode()
                    .into(),
                ))
                .await
                .unwrap();
        }
        // Observe actual forwarding before the burst; registration has no ACK.
        timeout(Duration::from_secs(2), async {
            loop {
                sender
                    .send(Message::Binary(
                        RelayPacket::Data {
                            group_id: "fragment-burst".into(),
                            peer_id: "sender".into(),
                            payload: b"ready".to_vec(),
                        }
                        .encode()
                        .into(),
                    ))
                    .await
                    .unwrap();
                if let Ok(Some(Ok(Message::Binary(_)))) =
                    timeout(Duration::from_millis(10), receiver.next()).await
                {
                    break;
                }
            }
        })
        .await
        .unwrap();
        const FRAGMENTS: u32 = 256;
        let receive_task = tokio::spawn(async move {
            let mut received = 0;
            while received < FRAGMENTS {
                let message = receiver
                    .next()
                    .await
                    .expect("draining peer disconnected")
                    .expect("draining peer received a transport error");
                if let Message::Binary(bytes) = message {
                    let RelayPacket::Data { payload, .. } = RelayPacket::decode(&bytes).unwrap()
                    else {
                        panic!("unexpected relay response")
                    };
                    if payload == b"ready" {
                        continue;
                    }
                    assert_eq!(payload.len(), 1400);
                    assert_eq!(
                        u32::from_be_bytes(payload[..4].try_into().unwrap()),
                        received
                    );
                    received += 1;
                }
            }
        });
        for sequence in 0..FRAGMENTS {
            let mut payload = vec![0; 1400];
            payload[..4].copy_from_slice(&sequence.to_be_bytes());
            sender
                .feed(Message::Binary(
                    RelayPacket::Data {
                        group_id: "fragment-burst".into(),
                        peer_id: "sender".into(),
                        payload,
                    }
                    .encode()
                    .into(),
                ))
                .await
                .unwrap();
        }
        sender.flush().await.unwrap();
        timeout(Duration::from_secs(3), receive_task)
            .await
            .unwrap()
            .unwrap();
        cancel.send(()).unwrap();
        server_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn websocket_relay_tunnels_forward_existing_udp_protocol_both_ways() {
        let (cancel_tx, _) = broadcast::channel(1);
        let server =
            BoundTcpRelayServer::bind(TcpRelayServerConfig::new("127.0.0.1:0".parse().unwrap()))
                .await
                .expect("server should bind");
        let relay_addr = server.local_addr().expect("server addr");
        let server_task = tokio::spawn(server.run_websocket(cancel_tx.subscribe()));
        let relay_url = format!("ws://{relay_addr}/relay");

        let host_app = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("host app should bind");
        let host_app_addr = host_app.local_addr().expect("host app addr");
        let (host_sender, host_receiver) = host_app.split();

        let client_app = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("client app should bind");
        let (client_sender, client_receiver) = client_app.split();

        let host_tunnel = BoundWebSocketRelayTunnel::bind(
            WebSocketRelayTunnelConfig::new(
                "127.0.0.1:0".parse().unwrap(),
                relay_url.clone(),
                "group",
                "host",
            )
            .unwrap()
            .with_local_target_addr(host_app_addr),
        )
        .await
        .expect("host tunnel should bind");
        let host_tunnel_addr = host_tunnel.local_addr().expect("host tunnel addr");
        let host_tunnel_task = tokio::spawn(host_tunnel.run(cancel_tx.subscribe()));

        let client_tunnel = BoundWebSocketRelayTunnel::bind(
            WebSocketRelayTunnelConfig::new(
                "127.0.0.1:0".parse().unwrap(),
                relay_url,
                "group",
                "client",
            )
            .unwrap(),
        )
        .await
        .expect("client tunnel should bind");
        let client_tunnel_addr = client_tunnel.local_addr().expect("client tunnel addr");
        let client_tunnel_task = tokio::spawn(client_tunnel.run(cancel_tx.subscribe()));

        tokio::time::sleep(Duration::from_millis(100)).await;
        client_sender
            .send_control(&ControlMessage::Heartbeat, client_tunnel_addr)
            .await
            .expect("client send should succeed");

        let received = timeout(Duration::from_secs(2), host_receiver.recv())
            .await
            .expect("host should receive through relay")
            .expect("host packet should decode");
        let MultiplexedPacket::Control(ControlMessage::Heartbeat, host_seen_addr) = received else {
            panic!("host received unexpected packet: {received:?}");
        };
        assert_eq!(host_seen_addr, host_tunnel_addr);

        host_sender
            .send_control(&ControlMessage::StopStream, host_seen_addr)
            .await
            .expect("host reply should send");
        let received = timeout(Duration::from_secs(2), client_receiver.recv())
            .await
            .expect("client should receive through relay")
            .expect("client packet should decode");
        let MultiplexedPacket::Control(ControlMessage::StopStream, _) = received else {
            panic!("client received unexpected packet: {received:?}");
        };

        let _ = cancel_tx.send(());
        server_task.await.expect("server task should join").unwrap();
        host_tunnel_task
            .await
            .expect("host tunnel task should join")
            .unwrap();
        client_tunnel_task
            .await
            .expect("client tunnel task should join")
            .unwrap();
    }

    #[test]
    fn outbound_flush_backpressure_scales_down_and_recovers() {
        reset_relay_outbound_congestion_for_tests();
        assert_eq!(relay_outbound_congestion_scale_permille(), 1_000);
        record_relay_outbound_flush(Duration::from_millis(90), 32);
        let after_severe = relay_outbound_congestion_scale_permille();
        assert!(after_severe < 800 && after_severe >= 250, "got {after_severe}");
        record_relay_outbound_flush(Duration::from_millis(45), 16);
        let after_moderate = relay_outbound_congestion_scale_permille();
        assert!(after_moderate < after_severe && after_moderate >= 250);
        reset_relay_outbound_congestion_for_tests();
        assert_eq!(relay_outbound_congestion_scale_permille(), 1_000);
    }

    #[test]
    fn encode_data_frame_matches_packet_encode() {
        let direct = RelayPacket::encode_data_frame("grp-1", "peer-a", b"hevc-chunk");
        let via_enum = RelayPacket::Data {
            group_id: "grp-1".into(),
            peer_id: "peer-a".into(),
            payload: b"hevc-chunk".to_vec(),
        }
        .encode();
        assert_eq!(direct, via_enum);
    }

    #[test]
    fn websocket_and_hybrid_configs_parse_multi_url_failover_lists() {
        let ws = WebSocketRelayTunnelConfig::new(
            "127.0.0.1:0".parse().unwrap(),
            "ws://192.168.2.200:39493/v1/relay, wss://weavelo.hackerlife.fun:8443/v1/relay",
            "group",
            "peer",
        )
        .expect("comma-separated ws/wss URLs should parse");
        assert_eq!(ws.relay_url, "ws://192.168.2.200:39493/v1/relay");
        assert_eq!(
            ws.fallback_relay_urls,
            vec!["wss://weavelo.hackerlife.fun:8443/v1/relay".to_string()]
        );
        assert!(is_private_or_local_url(&ws.relay_url));
        assert!(!is_private_or_local_url(&ws.fallback_relay_urls[0]));

        let hybrid = HybridRelayTunnelConfig::new(
            "127.0.0.1:0".parse().unwrap(),
            "udp://192.168.2.200:39494, ws://192.168.2.200:39493/v1/relay, udp://weavelo.hackerlife.fun:39494, wss://weavelo.hackerlife.fun:8443/v1/relay",
            "group",
            "peer",
        )
        .expect("hybrid URL list should parse");
        assert_eq!(hybrid.candidates.len(), 4);
        assert_eq!(
            hybrid.candidates[0],
            HybridRelayCandidate::Udp("udp://192.168.2.200:39494".to_string())
        );
        assert_eq!(
            hybrid.candidates[3],
            HybridRelayCandidate::WebSocket(
                "wss://weavelo.hackerlife.fun:8443/v1/relay".to_string()
            )
        );
        assert!(matches!(
            HybridRelayTunnelConfig::new(
                "127.0.0.1:0".parse().unwrap(),
                "udp://192.168.2.200/no-port",
                "group",
                "peer",
            ),
            Err(RelayConfigError::InvalidRelayUrl(_))
        ));
    }

    #[tokio::test]
    async fn websocket_tunnel_fails_over_from_unreachable_primary_to_fallback() {
        let (cancel_tx, _) = broadcast::channel(1);
        let dead_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead_addr = dead_listener.local_addr().unwrap();
        drop(dead_listener);

        let server =
            BoundTcpRelayServer::bind(TcpRelayServerConfig::new("127.0.0.1:0".parse().unwrap()))
                .await
                .unwrap();
        let live_addr = server.local_addr().unwrap();
        let server_task = tokio::spawn(server.run_websocket(cancel_tx.subscribe()));

        let host_app = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let host_app_addr = host_app.local_addr().unwrap();
        let (_, host_receiver) = host_app.split();

        let client_app = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let (client_sender, _) = client_app.split();

        let host_tunnel = BoundWebSocketRelayTunnel::bind(
            WebSocketRelayTunnelConfig::new(
                "127.0.0.1:0".parse().unwrap(),
                format!("ws://{dead_addr}/v1/relay,ws://{live_addr}/v1/relay"),
                "failover-group",
                "host",
            )
            .unwrap()
            .with_local_target_addr(host_app_addr),
        )
        .await
        .unwrap();
        let host_diag = host_tunnel.diagnostics();
        let host_task = tokio::spawn(host_tunnel.run(cancel_tx.subscribe()));

        let client_tunnel = BoundWebSocketRelayTunnel::bind(
            WebSocketRelayTunnelConfig::new(
                "127.0.0.1:0".parse().unwrap(),
                format!("ws://{live_addr}/v1/relay"),
                "failover-group",
                "client",
            )
            .unwrap(),
        )
        .await
        .unwrap();
        let client_tunnel_addr = client_tunnel.local_addr().unwrap();
        let client_task = tokio::spawn(client_tunnel.run(cancel_tx.subscribe()));

        timeout(Duration::from_secs(2), async {
            while host_diag.snapshot().state != crate::relay_status::RelayConnectionState::Connected
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("host tunnel should fail over to live WS fallback");

        tokio::time::sleep(Duration::from_millis(50)).await;
        client_sender
            .send_control(&ControlMessage::Heartbeat, client_tunnel_addr)
            .await
            .unwrap();
        let received = timeout(Duration::from_secs(2), host_receiver.recv())
            .await
            .expect("host should receive packet via fallback")
            .expect("packet should decode");
        assert!(matches!(
            received,
            MultiplexedPacket::Control(ControlMessage::Heartbeat, _)
        ));

        let _ = cancel_tx.send(());
        server_task.await.unwrap().unwrap();
        host_task.await.unwrap().unwrap();
        client_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn udp_relay_server_unicasts_rpmx_envelopes_only_to_addressed_peer() {
        let (cancel_tx, _) = broadcast::channel(1);
        let server =
            BoundUdpRelayServer::bind(UdpRelayServerConfig::new("127.0.0.1:0".parse().unwrap()))
                .await
                .unwrap();
        let relay_addr = server.local_addr().unwrap();
        let server_task = tokio::spawn(server.run(cancel_tx.subscribe()));

        let sock_a = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let sock_b = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let sock_c = UdpSocket::bind("127.0.0.1:0").await.unwrap();

        for (sock, peer) in [(&sock_a, "peer-a"), (&sock_b, "peer-b"), (&sock_c, "peer-c")] {
            let reg = RelayPacket::Register {
                group_id: "unicast-grp".into(),
                peer_id: peer.into(),
            }
            .encode();
            sock.send_to(&reg, relay_addr).await.unwrap();
            let mut ack = [0u8; 256];
            let (len, _) = timeout(Duration::from_secs(1), sock.recv_from(&mut ack))
                .await
                .expect("UDP register should be echoed as ACK")
                .unwrap();
            assert!(matches!(
                RelayPacketView::decode(&ack[..len]),
                Some(RelayPacketView::Register { peer_id, .. }) if peer_id == peer
            ));
        }

        // Build a well-formed RPMX v1 frame addressed from peer-a to peer-b.
        let mut rpmx = Vec::new();
        rpmx.extend_from_slice(b"RPMX");
        rpmx.push(1);
        rpmx.push(0);
        rpmx.extend_from_slice(&[7u8; 16]);
        rpmx.push(b"peer-a".len() as u8);
        rpmx.push(b"peer-b".len() as u8);
        rpmx.extend_from_slice(b"peer-a");
        rpmx.extend_from_slice(b"peer-b");
        rpmx.extend_from_slice(&(4u16).to_be_bytes());
        rpmx.extend_from_slice(b"ping");
        rpmx.extend_from_slice(&[0u8; 32]); // MAC tag placeholder (shape-valid for relay_target)

        let data = RelayPacket::encode_data_frame("unicast-grp", "peer-a", &rpmx);
        sock_a.send_to(&data, relay_addr).await.unwrap();

        let mut buf = [0u8; 512];
        let (len, _) = timeout(Duration::from_secs(1), sock_b.recv_from(&mut buf))
            .await
            .expect("peer-b should receive unicast RPMX")
            .unwrap();
        assert!(matches!(
            RelayPacketView::decode(&buf[..len]),
            Some(RelayPacketView::Data { payload, .. }) if payload == rpmx.as_slice()
        ));

        let bystander = timeout(Duration::from_millis(120), sock_c.recv_from(&mut buf)).await;
        assert!(
            bystander.is_err(),
            "peer-c must not receive unicast RPMX addressed to peer-b"
        );

        let _ = cancel_tx.send(());
        server_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn unified_hybrid_server_bridges_udp_and_websocket_peers_both_ways() {
        let (cancel_tx, _) = broadcast::channel(1);
        let ws_server =
            BoundTcpRelayServer::bind(TcpRelayServerConfig::new("127.0.0.1:0".parse().unwrap()))
                .await
                .unwrap();
        let ws_addr = ws_server.local_addr().unwrap();
        let udp_server =
            BoundUdpRelayServer::bind(UdpRelayServerConfig::new("127.0.0.1:0".parse().unwrap()))
                .await
                .unwrap();
        let udp_addr = udp_server.local_addr().unwrap();
        let server_task = tokio::spawn(
            ws_server.run_websocket_with_udp(udp_server, cancel_tx.subscribe()),
        );

        // Host connects via Hybrid tunnel using UDP (with an unreachable dead UDP first to verify failover).
        let dead_udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let dead_udp_addr = dead_udp.local_addr().unwrap();
        drop(dead_udp);

        let host_app = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let host_app_addr = host_app.local_addr().unwrap();
        let (host_sender, host_receiver) = host_app.split();

        let host_tunnel = BoundHybridRelayTunnel::bind(
            HybridRelayTunnelConfig::new(
                "127.0.0.1:0".parse().unwrap(),
                format!("udp://{dead_udp_addr},udp://{udp_addr},ws://{ws_addr}/v1/relay"),
                "bridge-group",
                "studio-host",
            )
            .unwrap()
            .with_local_target_addr(host_app_addr),
        )
        .await
        .unwrap();
        let host_tunnel_addr = host_tunnel.local_addr().unwrap();
        let host_diag = host_tunnel.diagnostics();
        let host_task = tokio::spawn(host_tunnel.run(cancel_tx.subscribe()));

        // Client connects via classic WebSocket tunnel to the SAME unified server.
        let client_app = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let (client_sender, client_receiver) = client_app.split();
        let client_tunnel = BoundWebSocketRelayTunnel::bind(
            WebSocketRelayTunnelConfig::new(
                "127.0.0.1:0".parse().unwrap(),
                format!("ws://{ws_addr}/v1/relay"),
                "bridge-group",
                "mobile-client",
            )
            .unwrap(),
        )
        .await
        .unwrap();
        let client_tunnel_addr = client_tunnel.local_addr().unwrap();
        let client_diag = client_tunnel.diagnostics();
        let client_task = tokio::spawn(client_tunnel.run(cancel_tx.subscribe()));

        timeout(Duration::from_secs(3), async {
            loop {
                let h = host_diag.snapshot();
                let c = client_diag.snapshot();
                if h.state == crate::relay_status::RelayConnectionState::Connected
                    && c.state == crate::relay_status::RelayConnectionState::Connected
                    && c.registrations_sent >= 1
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(15)).await;
            }
        })
        .await
        .expect("both UDP host and WS client should connect to unified relay");
        assert_eq!(host_diag.snapshot().websocket_upgrades, 0);
        assert_eq!(client_diag.snapshot().websocket_upgrades, 1);

        // WS client -> unified server -> UDP host
        client_sender
            .send_control(&ControlMessage::Heartbeat, client_tunnel_addr)
            .await
            .unwrap();
        let from_client = timeout(Duration::from_secs(2), host_receiver.recv())
            .await
            .expect("UDP host should receive from WS client")
            .expect("packet should decode");
        let MultiplexedPacket::Control(ControlMessage::Heartbeat, host_reply_addr) = from_client
        else {
            panic!("unexpected packet at host: {from_client:?}");
        };
        assert_eq!(host_reply_addr, host_tunnel_addr);

        // UDP host -> unified server -> WS client
        host_sender
            .send_control(&ControlMessage::StopStream, host_reply_addr)
            .await
            .unwrap();
        let from_host = timeout(Duration::from_secs(2), client_receiver.recv())
            .await
            .expect("WS client should receive from UDP host")
            .expect("packet should decode");
        assert!(matches!(
            from_host,
            MultiplexedPacket::Control(ControlMessage::StopStream, _)
        ));

        let _ = cancel_tx.send(());
        server_task.await.unwrap().unwrap();
        host_task.await.unwrap().unwrap();
        client_task.await.unwrap().unwrap();
    }
}
