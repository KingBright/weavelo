use std::collections::{HashMap, HashSet};
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bincode::Options;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{Mutex, RwLock, mpsc, oneshot, watch};
use tokio::task::JoinHandle;

/// Default maximum number of concurrently enabled forwarding rules per engine.
pub const MAX_ACTIVE_RULES: usize = 64;
/// Maximum payload bytes carried in a single [`TunnelFrame::Data`] message.
pub const MAX_FRAME_PAYLOAD_BYTES: usize = 64 * 1024;
/// Maximum serialized byte size allowed for any single [`TunnelFrame`].
pub const MAX_TUNNEL_FRAME_WIRE_BYTES: usize = MAX_FRAME_PAYLOAD_BYTES + 1024;
/// Default chunk size when reading from a TCP stream.
pub const DEFAULT_TCP_READ_CHUNK_BYTES: usize = 16 * 1024;
/// Default idle timeout before an inactive UDP forwarding session is evicted.
pub const DEFAULT_UDP_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Default sweep interval for checking idle UDP forwarding sessions.
pub const DEFAULT_UDP_EVICT_INTERVAL: Duration = Duration::from_secs(10);
/// Default timeout for establishing an outbound TCP connection on the target side.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Default maximum concurrent multiplexed streams per engine.
pub const DEFAULT_MAX_CONCURRENT_STREAMS: usize = 512;
/// Default per-stream channel capacity.
pub const DEFAULT_STREAM_CHANNEL_CAPACITY: usize = 64;

/// Transport protocol forwarded by a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TunnelProtocol {
    Tcp,
    Udp,
}

/// Direction of a port forwarding tunnel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TunnelDirection {
    /// Listen on local peer and forward to remote peer's target.
    LocalToRemote,
    /// Listen on remote peer and forward to local peer's target.
    RemoteToLocal,
}

/// Configuration for a single TCP or UDP port forwarding rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortForwardRule {
    pub id: String,
    pub name: String,
    pub protocol: TunnelProtocol,
    pub direction: TunnelDirection,
    pub bind_host: String,
    pub bind_port: u16,
    pub target_host: String,
    pub target_port: u16,
    pub enabled: bool,
}

/// Validation errors for [`PortForwardRule`] definitions and rule sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleValidationError {
    EmptyRuleId,
    InvalidBindPort,
    InvalidTargetPort,
    InvalidBindHost(String),
    InvalidTargetHost(String),
    LoopbackSelfForward {
        bind_host: String,
        target_host: String,
        port: u16,
    },
    DuplicateRuleId(String),
    DuplicateBindAddress {
        protocol: TunnelProtocol,
        bind_host: String,
        bind_port: u16,
    },
    MaxActiveRulesExceeded {
        active: usize,
        max: usize,
    },
}

impl fmt::Display for RuleValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyRuleId => write!(f, "rule id must not be empty"),
            Self::InvalidBindPort => write!(f, "bind_port must be non-zero"),
            Self::InvalidTargetPort => write!(f, "target_port must be non-zero"),
            Self::InvalidBindHost(h) => write!(f, "invalid bind_host: {h:?}"),
            Self::InvalidTargetHost(h) => write!(f, "invalid target_host: {h:?}"),
            Self::LoopbackSelfForward {
                bind_host,
                target_host,
                port,
            } => write!(
                f,
                "loop guard rejected forwarding {bind_host}:{port} -> {target_host}:{port}"
            ),
            Self::DuplicateRuleId(id) => write!(f, "duplicate rule id: {id}"),
            Self::DuplicateBindAddress {
                protocol,
                bind_host,
                bind_port,
            } => write!(
                f,
                "duplicate bind address {protocol:?} {bind_host}:{bind_port}"
            ),
            Self::MaxActiveRulesExceeded { active, max } => {
                write!(f, "active rules ({active}) exceed limit ({max})")
            }
        }
    }
}

impl std::error::Error for RuleValidationError {}

impl PortForwardRule {
    /// Validates a single rule: non-empty id, non-zero ports, valid hosts, and loop guard.
    pub fn validate(&self) -> Result<(), RuleValidationError> {
        if self.id.trim().is_empty() {
            return Err(RuleValidationError::EmptyRuleId);
        }
        if self.bind_port == 0 {
            return Err(RuleValidationError::InvalidBindPort);
        }
        if self.target_port == 0 {
            return Err(RuleValidationError::InvalidTargetPort);
        }
        if !is_valid_host(&self.bind_host, true) {
            return Err(RuleValidationError::InvalidBindHost(self.bind_host.clone()));
        }
        if !is_valid_host(&self.target_host, false) {
            return Err(RuleValidationError::InvalidTargetHost(
                self.target_host.clone(),
            ));
        }
        if self.bind_port == self.target_port
            && hosts_may_loop(&self.bind_host, &self.target_host)
        {
            return Err(RuleValidationError::LoopbackSelfForward {
                bind_host: self.bind_host.clone(),
                target_host: self.target_host.clone(),
                port: self.bind_port,
            });
        }
        Ok(())
    }
}

/// Validates a collection of rules including uniqueness and `max_active_rules` enforcement.
pub fn validate_rules(
    rules: &[PortForwardRule],
    max_active_rules: usize,
) -> Result<(), RuleValidationError> {
    let mut seen_ids = HashSet::with_capacity(rules.len());
    let mut active_binds: HashSet<(TunnelProtocol, String, u16)> = HashSet::new();
    let mut active_count = 0usize;

    for rule in rules {
        rule.validate()?;
        if !seen_ids.insert(rule.id.as_str()) {
            return Err(RuleValidationError::DuplicateRuleId(rule.id.clone()));
        }
        if rule.enabled {
            active_count += 1;
            if active_count > max_active_rules {
                return Err(RuleValidationError::MaxActiveRulesExceeded {
                    active: active_count,
                    max: max_active_rules,
                });
            }
            let bind_key = (
                rule.protocol,
                normalize_host(&rule.bind_host),
                rule.bind_port,
            );
            if !active_binds.insert(bind_key) {
                return Err(RuleValidationError::DuplicateBindAddress {
                    protocol: rule.protocol,
                    bind_host: rule.bind_host.clone(),
                    bind_port: rule.bind_port,
                });
            }
        }
    }

    Ok(())
}

fn normalize_host(host: &str) -> String {
    host.trim().to_ascii_lowercase()
}

fn is_valid_host(host: &str, allow_unspecified: bool) -> bool {
    let trimmed = host.trim();
    if trimmed.is_empty() || trimmed != host || trimmed.len() > 253 {
        return false;
    }
    if let Ok(ip) = trimmed.parse::<IpAddr>() {
        if !allow_unspecified && ip.is_unspecified() {
            return false;
        }
        return true;
    }
    // Validate RFC 1123 hostname (no colons, spaces, or control chars).
    if trimmed.starts_with('.') || trimmed.ends_with('.') {
        return false;
    }
    for label in trimmed.split('.') {
        if label.is_empty() || label.len() > 63 {
            return false;
        }
        let bytes = label.as_bytes();
        if bytes.first() == Some(&b'-') || bytes.last() == Some(&b'-') {
            return false;
        }
        if !bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
        {
            return false;
        }
    }
    true
}

fn is_loopback_host(host: &str) -> bool {
    let lower = normalize_host(host);
    if lower == "localhost" {
        return true;
    }
    lower
        .parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

fn is_unspecified_host(host: &str) -> bool {
    host.trim()
        .parse::<IpAddr>()
        .map(|ip| ip.is_unspecified())
        .unwrap_or(false)
}

fn hosts_may_loop(bind_host: &str, target_host: &str) -> bool {
    let norm_bind = normalize_host(bind_host);
    let norm_target = normalize_host(target_host);
    if norm_bind == norm_target {
        return true;
    }
    if is_loopback_host(&norm_bind) && is_loopback_host(&norm_target) {
        return true;
    }
    if is_unspecified_host(&norm_bind) && is_loopback_host(&norm_target) {
        return true;
    }
    false
}

/// Error codes carried in [`TunnelFrame::Error`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TunnelErrorCode {
    InvalidFrame,
    PayloadTooLarge,
    ConnectFailed,
    ConnectTimeout,
    StreamLimitReached,
    UnknownStream,
    IoError,
}

/// Multiplexed framed wire messages exchanged over Weavelo's encrypted tunnel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TunnelFrame {
    /// Request the remote peer to open a TCP connection or UDP session.
    Open {
        stream_id: u64,
        rule_id: String,
        protocol: TunnelProtocol,
        target_host: String,
        target_port: u16,
    },
    /// Acknowledge that `stream_id` has been established on the remote peer.
    Opened { stream_id: u64 },
    /// Payload bytes for an open stream (TCP chunk or single UDP datagram).
    Data { stream_id: u64, payload: Vec<u8> },
    /// Gracefully close `stream_id`.
    Close {
        stream_id: u64,
        reason: Option<String>,
    },
    /// Fatal error on `stream_id` (implies stream termination).
    Error {
        stream_id: u64,
        code: TunnelErrorCode,
        message: String,
    },
}

/// Serialization / deserialization errors for [`TunnelFrame`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelCodecError {
    PayloadTooLarge { size: usize, max: usize },
    WireFrameTooLarge { size: usize, max: usize },
    SerializeFailed(String),
    DeserializeFailed(String),
}

impl fmt::Display for TunnelCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PayloadTooLarge { size, max } => {
                write!(f, "tunnel frame payload too large: {size} > {max}")
            }
            Self::WireFrameTooLarge { size, max } => {
                write!(f, "serialized tunnel frame too large: {size} > {max}")
            }
            Self::SerializeFailed(e) => write!(f, "failed to serialize tunnel frame: {e}"),
            Self::DeserializeFailed(e) => write!(f, "failed to deserialize tunnel frame: {e}"),
        }
    }
}

impl std::error::Error for TunnelCodecError {}

fn bincode_options() -> impl bincode::Options {
    bincode::DefaultOptions::new()
        .with_varint_encoding()
        .with_limit(MAX_TUNNEL_FRAME_WIRE_BYTES as u64)
}

impl TunnelFrame {
    /// Returns the multiplexed stream identifier carried by this frame.
    pub fn stream_id(&self) -> u64 {
        match self {
            Self::Open { stream_id, .. }
            | Self::Opened { stream_id }
            | Self::Data { stream_id, .. }
            | Self::Close { stream_id, .. }
            | Self::Error { stream_id, .. } => *stream_id,
        }
    }

    /// Encodes the frame into compact binary bytes while enforcing payload and wire size bounds.
    pub fn encode(&self) -> Result<Vec<u8>, TunnelCodecError> {
        if let Self::Data { payload, .. } = self
            && payload.len() > MAX_FRAME_PAYLOAD_BYTES
        {
            return Err(TunnelCodecError::PayloadTooLarge {
                size: payload.len(),
                max: MAX_FRAME_PAYLOAD_BYTES,
            });
        }
        let encoded = bincode_options()
            .serialize(self)
            .map_err(|e| TunnelCodecError::SerializeFailed(e.to_string()))?;
        if encoded.len() > MAX_TUNNEL_FRAME_WIRE_BYTES {
            return Err(TunnelCodecError::WireFrameTooLarge {
                size: encoded.len(),
                max: MAX_TUNNEL_FRAME_WIRE_BYTES,
            });
        }
        Ok(encoded)
    }

    /// Decodes a compact binary frame while enforcing wire and payload size bounds.
    pub fn decode(bytes: &[u8]) -> Result<Self, TunnelCodecError> {
        if bytes.len() > MAX_TUNNEL_FRAME_WIRE_BYTES {
            return Err(TunnelCodecError::WireFrameTooLarge {
                size: bytes.len(),
                max: MAX_TUNNEL_FRAME_WIRE_BYTES,
            });
        }
        let frame: Self = bincode_options()
            .deserialize(bytes)
            .map_err(|e| TunnelCodecError::DeserializeFailed(e.to_string()))?;
        if let Self::Data { payload, .. } = &frame
            && payload.len() > MAX_FRAME_PAYLOAD_BYTES
        {
            return Err(TunnelCodecError::PayloadTooLarge {
                size: payload.len(),
                max: MAX_FRAME_PAYLOAD_BYTES,
            });
        }
        Ok(frame)
    }
}

/// Abstract framed transport pair connecting a [`PortForwardEngine`] to the underlying tunnel.
pub struct TunnelTransport {
    pub tx: mpsc::Sender<TunnelFrame>,
    pub rx: mpsc::Receiver<TunnelFrame>,
}

/// Creates a connected pair of in-memory [`TunnelTransport`] endpoints for loopback or testing.
pub fn loopback_transport_pair(buffer: usize) -> (TunnelTransport, TunnelTransport) {
    let cap = buffer.max(1);
    let (a_tx, b_rx) = mpsc::channel(cap);
    let (b_tx, a_rx) = mpsc::channel(cap);
    (
        TunnelTransport { tx: a_tx, rx: a_rx },
        TunnelTransport { tx: b_tx, rx: b_rx },
    )
}

/// Runtime configuration for [`PortForwardEngine`].
#[derive(Debug, Clone)]
pub struct PortForwardConfig {
    pub max_active_rules: usize,
    pub max_concurrent_streams: usize,
    pub tcp_read_chunk_bytes: usize,
    pub udp_idle_timeout: Duration,
    pub udp_evict_interval: Duration,
    pub connect_timeout: Duration,
    pub stream_channel_capacity: usize,
}

impl Default for PortForwardConfig {
    fn default() -> Self {
        Self {
            max_active_rules: MAX_ACTIVE_RULES,
            max_concurrent_streams: DEFAULT_MAX_CONCURRENT_STREAMS,
            tcp_read_chunk_bytes: DEFAULT_TCP_READ_CHUNK_BYTES,
            udp_idle_timeout: DEFAULT_UDP_IDLE_TIMEOUT,
            udp_evict_interval: DEFAULT_UDP_EVICT_INTERVAL,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            stream_channel_capacity: DEFAULT_STREAM_CHANNEL_CAPACITY,
        }
    }
}

/// Snapshot of runtime statistics for a [`PortForwardEngine`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PortForwardStats {
    pub active_rules: usize,
    pub active_streams: usize,
    pub active_udp_sessions: usize,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub frames_sent: u64,
    pub frames_received: u64,
}

/// Runtime errors returned by [`PortForwardEngine`] management operations.
#[derive(Debug)]
pub enum PortForwardError {
    Validation(RuleValidationError),
    BindFailed {
        bind_addr: String,
        source: std::io::Error,
    },
    RuleNotFound(String),
    EngineShutdown,
}

impl fmt::Display for PortForwardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(e) => write!(f, "rule validation failed: {e}"),
            Self::BindFailed { bind_addr, source } => {
                write!(f, "failed to bind listener on {bind_addr}: {source}")
            }
            Self::RuleNotFound(id) => write!(f, "rule not found: {id}"),
            Self::EngineShutdown => write!(f, "port forward engine is shut down"),
        }
    }
}

impl std::error::Error for PortForwardError {}

impl From<RuleValidationError> for PortForwardError {
    fn from(value: RuleValidationError) -> Self {
        Self::Validation(value)
    }
}

enum StreamCommand {
    Opened,
    Data(Vec<u8>),
    Close,
    Error,
}

struct StreamEntry {
    rule_id: String,
    protocol: TunnelProtocol,
    cmd_tx: mpsc::Sender<StreamCommand>,
}

struct RuleRuntime {
    rule: PortForwardRule,
    bound_addr: Option<SocketAddr>,
    stop_tx: Option<watch::Sender<bool>>,
    task: Option<JoinHandle<()>>,
}

struct EngineMetrics {
    bytes_sent: AtomicU64,
    bytes_received: AtomicU64,
    frames_sent: AtomicU64,
    frames_received: AtomicU64,
}

impl EngineMetrics {
    fn new() -> Self {
        Self {
            bytes_sent: AtomicU64::new(0),
            bytes_received: AtomicU64::new(0),
            frames_sent: AtomicU64::new(0),
            frames_received: AtomicU64::new(0),
        }
    }
}

struct EngineInner {
    config: PortForwardConfig,
    transport_tx: mpsc::Sender<TunnelFrame>,
    rules: RwLock<HashMap<String, RuleRuntime>>,
    streams: RwLock<HashMap<u64, StreamEntry>>,
    instance_prefix: u64,
    next_stream_seq: AtomicU64,
    shutdown_tx: watch::Sender<bool>,
    metrics: EngineMetrics,
}

impl EngineInner {
    fn alloc_stream_id(&self) -> u64 {
        let seq = self.next_stream_seq.fetch_add(1, Ordering::Relaxed) & 0xFFFF_FFFF;
        let seq = if seq == 0 {
            self.next_stream_seq.fetch_add(1, Ordering::Relaxed) & 0xFFFF_FFFF
        } else {
            seq
        };
        (self.instance_prefix << 32) | seq
    }

    async fn send_frame(&self, frame: TunnelFrame) -> bool {
        if let TunnelFrame::Data { payload, .. } = &frame {
            self.metrics
                .bytes_sent
                .fetch_add(payload.len() as u64, Ordering::Relaxed);
        }
        self.metrics.frames_sent.fetch_add(1, Ordering::Relaxed);
        self.transport_tx.send(frame).await.is_ok()
    }

    async fn register_stream(
        &self,
        stream_id: u64,
        rule_id: String,
        protocol: TunnelProtocol,
    ) -> Option<mpsc::Receiver<StreamCommand>> {
        let mut streams = self.streams.write().await;
        if streams.len() >= self.config.max_concurrent_streams || streams.contains_key(&stream_id) {
            return None;
        }
        let (cmd_tx, cmd_rx) = mpsc::channel(self.config.stream_channel_capacity.max(1));
        streams.insert(
            stream_id,
            StreamEntry {
                rule_id,
                protocol,
                cmd_tx,
            },
        );
        Some(cmd_rx)
    }

    async fn remove_stream(&self, stream_id: u64) {
        self.streams.write().await.remove(&stream_id);
    }

    async fn close_streams_for_rule(&self, rule_id: &str) {
        let matching: Vec<(u64, mpsc::Sender<StreamCommand>)> = {
            let mut streams = self.streams.write().await;
            let ids: Vec<u64> = streams
                .iter()
                .filter_map(|(id, entry)| (entry.rule_id == rule_id).then_some(*id))
                .collect();
            ids.into_iter()
                .filter_map(|id| streams.remove(&id).map(|entry| (id, entry.cmd_tx)))
                .collect()
        };

        for (stream_id, cmd_tx) in matching {
            let _ = cmd_tx.try_send(StreamCommand::Close);
            let _ = self
                .send_frame(TunnelFrame::Close {
                    stream_id,
                    reason: Some("rule stopped".to_string()),
                })
                .await;
        }
    }

    async fn close_all_streams(&self) {
        let entries: Vec<(u64, mpsc::Sender<StreamCommand>)> = {
            let mut streams = self.streams.write().await;
            streams
                .drain()
                .map(|(id, entry)| (id, entry.cmd_tx))
                .collect()
        };
        for (stream_id, cmd_tx) in entries {
            let _ = cmd_tx.try_send(StreamCommand::Close);
            let _ = self
                .transport_tx
                .try_send(TunnelFrame::Close {
                    stream_id,
                    reason: Some("engine shutdown".to_string()),
                });
        }
    }
}

/// Async multiplexed TCP/UDP port forwarding engine over a framed [`TunnelTransport`].
pub struct PortForwardEngine {
    inner: Arc<EngineInner>,
    dispatch_task: Mutex<Option<JoinHandle<()>>>,
}

impl PortForwardEngine {
    /// Spawns a new [`PortForwardEngine`] bound to `transport`.
    ///
    /// Must be called from within a Tokio runtime.
    pub fn spawn(transport: TunnelTransport, mut config: PortForwardConfig) -> Self {
        config.tcp_read_chunk_bytes = config
            .tcp_read_chunk_bytes
            .clamp(1, MAX_FRAME_PAYLOAD_BYTES);
        config.stream_channel_capacity = config.stream_channel_capacity.max(1);
        if config.udp_evict_interval.is_zero() {
            config.udp_evict_interval = Duration::from_millis(50);
        }

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let prefix = (rand::random::<u32>() as u64).max(1);

        let inner = Arc::new(EngineInner {
            config,
            transport_tx: transport.tx,
            rules: RwLock::new(HashMap::new()),
            streams: RwLock::new(HashMap::new()),
            instance_prefix: prefix,
            next_stream_seq: AtomicU64::new(1),
            shutdown_tx,
            metrics: EngineMetrics::new(),
        });

        let dispatch_inner = Arc::clone(&inner);
        let dispatch_task = tokio::spawn(async move {
            run_inbound_dispatcher(dispatch_inner, transport.rx, shutdown_rx).await;
        });

        Self {
            inner,
            dispatch_task: Mutex::new(Some(dispatch_task)),
        }
    }

    /// Adds a forwarding rule, validates it against active rules, and starts its listener if enabled.
    pub async fn add_rule(
        &self,
        rule: PortForwardRule,
    ) -> Result<Option<SocketAddr>, PortForwardError> {
        if *self.inner.shutdown_tx.borrow() {
            return Err(PortForwardError::EngineShutdown);
        }
        rule.validate()?;

        let mut rules = self.inner.rules.write().await;
        if rules.contains_key(&rule.id) {
            return Err(RuleValidationError::DuplicateRuleId(rule.id).into());
        }

        if rule.enabled {
            let active_count = rules.values().filter(|r| r.rule.enabled).count();
            if active_count + 1 > self.inner.config.max_active_rules {
                return Err(RuleValidationError::MaxActiveRulesExceeded {
                    active: active_count + 1,
                    max: self.inner.config.max_active_rules,
                }
                .into());
            }
            let norm_host = normalize_host(&rule.bind_host);
            let duplicate_bind = rules.values().any(|existing| {
                existing.rule.enabled
                    && existing.rule.protocol == rule.protocol
                    && existing.rule.bind_port == rule.bind_port
                    && normalize_host(&existing.rule.bind_host) == norm_host
            });
            if duplicate_bind {
                return Err(RuleValidationError::DuplicateBindAddress {
                    protocol: rule.protocol,
                    bind_host: rule.bind_host,
                    bind_port: rule.bind_port,
                }
                .into());
            }
        }

        let (bound_addr, stop_tx, task) = if rule.enabled {
            let (addr, tx, handle) = start_listener(Arc::clone(&self.inner), rule.clone()).await?;
            (Some(addr), Some(tx), Some(handle))
        } else {
            (None, None, None)
        };

        let id = rule.id.clone();
        rules.insert(
            id,
            RuleRuntime {
                rule,
                bound_addr,
                stop_tx,
                task,
            },
        );
        Ok(bound_addr)
    }

    /// Removes a rule, stopping its listener and closing any active streams opened by it.
    pub async fn remove_rule(&self, rule_id: &str) -> Result<bool, PortForwardError> {
        let removed = self.inner.rules.write().await.remove(rule_id);
        let Some(runtime) = removed else {
            return Ok(false);
        };
        if let Some(stop_tx) = runtime.stop_tx {
            let _ = stop_tx.send(true);
        }
        if let Some(task) = runtime.task {
            let _ = task.await;
        }
        self.inner.close_streams_for_rule(rule_id).await;
        Ok(true)
    }

    /// Enables or disables an existing rule dynamically.
    pub async fn set_rule_enabled(
        &self,
        rule_id: &str,
        enabled: bool,
    ) -> Result<Option<SocketAddr>, PortForwardError> {
        if *self.inner.shutdown_tx.borrow() {
            return Err(PortForwardError::EngineShutdown);
        }

        let mut old_stop = None;
        let mut old_task = None;

        let result = {
            let mut rules = self.inner.rules.write().await;
            let Some(current_enabled) = rules.get(rule_id).map(|r| r.rule.enabled) else {
                return Err(PortForwardError::RuleNotFound(rule_id.to_string()));
            };
            if current_enabled == enabled {
                return Ok(rules.get(rule_id).and_then(|r| r.bound_addr));
            }

            if enabled {
                let active_count = rules.values().filter(|r| r.rule.enabled).count();
                if active_count + 1 > self.inner.config.max_active_rules {
                    return Err(RuleValidationError::MaxActiveRulesExceeded {
                        active: active_count + 1,
                        max: self.inner.config.max_active_rules,
                    }
                    .into());
                }
                let candidate = rules.get(rule_id).unwrap().rule.clone();
                let norm_host = normalize_host(&candidate.bind_host);
                let duplicate_bind = rules.iter().any(|(id, existing)| {
                    id != rule_id
                        && existing.rule.enabled
                        && existing.rule.protocol == candidate.protocol
                        && existing.rule.bind_port == candidate.bind_port
                        && normalize_host(&existing.rule.bind_host) == norm_host
                });
                if duplicate_bind {
                    return Err(RuleValidationError::DuplicateBindAddress {
                        protocol: candidate.protocol,
                        bind_host: candidate.bind_host,
                        bind_port: candidate.bind_port,
                    }
                    .into());
                }

                let (addr, tx, handle) =
                    start_listener(Arc::clone(&self.inner), candidate).await?;
                let entry = rules.get_mut(rule_id).unwrap();
                entry.rule.enabled = true;
                entry.bound_addr = Some(addr);
                entry.stop_tx = Some(tx);
                entry.task = Some(handle);
                Some(addr)
            } else {
                let entry = rules.get_mut(rule_id).unwrap();
                entry.rule.enabled = false;
                entry.bound_addr = None;
                old_stop = entry.stop_tx.take();
                old_task = entry.task.take();
                None
            }
        };

        if let Some(stop_tx) = old_stop {
            let _ = stop_tx.send(true);
        }
        if let Some(task) = old_task {
            let _ = task.await;
        }
        if !enabled {
            self.inner.close_streams_for_rule(rule_id).await;
        }
        Ok(result)
    }

    /// Returns a snapshot of all configured rules.
    pub async fn list_rules(&self) -> Vec<PortForwardRule> {
        self.inner
            .rules
            .read()
            .await
            .values()
            .map(|r| r.rule.clone())
            .collect()
    }

    /// Returns a snapshot of current runtime statistics.
    pub async fn stats(&self) -> PortForwardStats {
        let active_rules = self
            .inner
            .rules
            .read()
            .await
            .values()
            .filter(|r| r.rule.enabled)
            .count();
        let (active_streams, active_udp_sessions) = {
            let streams = self.inner.streams.read().await;
            let udp = streams
                .values()
                .filter(|s| s.protocol == TunnelProtocol::Udp)
                .count();
            (streams.len(), udp)
        };
        PortForwardStats {
            active_rules,
            active_streams,
            active_udp_sessions,
            bytes_sent: self.inner.metrics.bytes_sent.load(Ordering::Relaxed),
            bytes_received: self.inner.metrics.bytes_received.load(Ordering::Relaxed),
            frames_sent: self.inner.metrics.frames_sent.load(Ordering::Relaxed),
            frames_received: self.inner.metrics.frames_received.load(Ordering::Relaxed),
        }
    }

    /// Gracefully shuts down all listeners, active streams, and the inbound dispatcher.
    pub async fn shutdown(&self) {
        let _ = self.inner.shutdown_tx.send(true);

        let runtimes: Vec<RuleRuntime> = {
            let mut rules = self.inner.rules.write().await;
            rules.drain().map(|(_, runtime)| runtime).collect()
        };
        for runtime in runtimes {
            if let Some(stop_tx) = runtime.stop_tx {
                let _ = stop_tx.send(true);
            }
            if let Some(task) = runtime.task {
                let _ = task.await;
            }
        }

        self.inner.close_all_streams().await;

        if let Some(task) = self.dispatch_task.lock().await.take() {
            let _ = task.await;
        }
    }
}

impl Drop for PortForwardEngine {
    fn drop(&mut self) {
        let _ = self.inner.shutdown_tx.send(true);
    }
}

async fn start_listener(
    inner: Arc<EngineInner>,
    rule: PortForwardRule,
) -> Result<(SocketAddr, watch::Sender<bool>, JoinHandle<()>), PortForwardError> {
    let bind_addr_str = format!("{}:{}", rule.bind_host, rule.bind_port);
    let (stop_tx, stop_rx) = watch::channel(false);
    let global_shutdown = inner.shutdown_tx.subscribe();

    match rule.protocol {
        TunnelProtocol::Tcp => {
            let listener = TcpListener::bind((rule.bind_host.as_str(), rule.bind_port))
                .await
                .map_err(|source| PortForwardError::BindFailed {
                    bind_addr: bind_addr_str,
                    source,
                })?;
            let bound = listener.local_addr().map_err(|source| {
                PortForwardError::BindFailed {
                    bind_addr: format!("{}:{}", rule.bind_host, rule.bind_port),
                    source,
                }
            })?;
            let task = tokio::spawn(run_tcp_listener(
                inner,
                rule,
                listener,
                stop_rx,
                global_shutdown,
            ));
            Ok((bound, stop_tx, task))
        }
        TunnelProtocol::Udp => {
            let socket = UdpSocket::bind((rule.bind_host.as_str(), rule.bind_port))
                .await
                .map_err(|source| PortForwardError::BindFailed {
                    bind_addr: bind_addr_str,
                    source,
                })?;
            let bound = socket.local_addr().map_err(|source| {
                PortForwardError::BindFailed {
                    bind_addr: format!("{}:{}", rule.bind_host, rule.bind_port),
                    source,
                }
            })?;
            let task = tokio::spawn(run_udp_listener(
                inner,
                rule,
                Arc::new(socket),
                stop_rx,
                global_shutdown,
            ));
            Ok((bound, stop_tx, task))
        }
    }
}

async fn run_tcp_listener(
    inner: Arc<EngineInner>,
    rule: PortForwardRule,
    listener: TcpListener,
    mut rule_stop: watch::Receiver<bool>,
    mut global_shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            biased;
            _ = rule_stop.changed() => {
                if *rule_stop.borrow() {
                    break;
                }
            }
            _ = global_shutdown.changed() => {
                if *global_shutdown.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _peer)) => {
                        let _ = stream.set_nodelay(true);
                        let inner_clone = Arc::clone(&inner);
                        let rule_clone = rule.clone();
                        let shutdown_clone = inner.shutdown_tx.subscribe();
                        tokio::spawn(async move {
                            handle_outbound_tcp_stream(inner_clone, rule_clone, stream, shutdown_clone).await;
                        });
                    }
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }
            }
        }
    }
}

async fn handle_outbound_tcp_stream(
    inner: Arc<EngineInner>,
    rule: PortForwardRule,
    stream: TcpStream,
    mut global_shutdown: watch::Receiver<bool>,
) {
    let stream_id = inner.alloc_stream_id();
    let Some(mut cmd_rx) = inner
        .register_stream(stream_id, rule.id.clone(), TunnelProtocol::Tcp)
        .await
    else {
        return;
    };

    let open_frame = TunnelFrame::Open {
        stream_id,
        rule_id: rule.id.clone(),
        protocol: TunnelProtocol::Tcp,
        target_host: rule.target_host.clone(),
        target_port: rule.target_port,
    };
    if !inner.send_frame(open_frame).await {
        inner.remove_stream(stream_id).await;
        return;
    }

    // Wait for Opened acknowledgement from remote peer.
    let mut pre_buffered = Vec::new();
    let opened = tokio::time::timeout(inner.config.connect_timeout, async {
        loop {
            tokio::select! {
                biased;
                _ = global_shutdown.changed() => {
                    if *global_shutdown.borrow() {
                        return false;
                    }
                }
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(StreamCommand::Opened) => return true,
                        Some(StreamCommand::Data(data)) => pre_buffered.push(data),
                        Some(StreamCommand::Close) | Some(StreamCommand::Error) | None => {
                            return false;
                        }
                    }
                }
            }
        }
    })
    .await;

    if !matches!(opened, Ok(true)) {
        inner.remove_stream(stream_id).await;
        let _ = inner
            .send_frame(TunnelFrame::Close {
                stream_id,
                reason: Some("open failed or timed out".to_string()),
            })
            .await;
        return;
    }

    pump_tcp_stream(inner, stream_id, stream, cmd_rx, pre_buffered, global_shutdown).await;
}

async fn pump_tcp_stream(
    inner: Arc<EngineInner>,
    stream_id: u64,
    stream: TcpStream,
    mut cmd_rx: mpsc::Receiver<StreamCommand>,
    pre_buffered: Vec<Vec<u8>>,
    global_shutdown: watch::Receiver<bool>,
) {
    let (mut read_half, mut write_half) = stream.into_split();
    let chunk_size = inner.config.tcp_read_chunk_bytes;

    let (done_tx, mut done_rx) = oneshot::channel::<()>();

    // Task A: remote TunnelFrame -> local TCP write_half
    let mut shutdown_w = global_shutdown.clone();
    let writer_task = tokio::spawn(async move {
        for buf in pre_buffered {
            if write_half.write_all(&buf).await.is_err() {
                let _ = write_half.shutdown().await;
                return;
            }
        }
        loop {
            tokio::select! {
                biased;
                _ = shutdown_w.changed() => {
                    if *shutdown_w.borrow() {
                        break;
                    }
                }
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(StreamCommand::Data(payload)) => {
                            if write_half.write_all(&payload).await.is_err() {
                                break;
                            }
                        }
                        Some(StreamCommand::Opened) => {}
                        Some(StreamCommand::Close) | Some(StreamCommand::Error) | None => {
                            break;
                        }
                    }
                }
            }
        }
        let _ = write_half.shutdown().await;
        let _ = done_tx.send(());
    });

    // Task B: local TCP read_half -> remote TunnelFrame::Data
    let mut shutdown_r = global_shutdown;
    let mut buf = vec![0u8; chunk_size];
    let mut sent_close = false;
    loop {
        tokio::select! {
            biased;
            _ = &mut done_rx => {
                break;
            }
            _ = shutdown_r.changed() => {
                if *shutdown_r.borrow() {
                    break;
                }
            }
            read_res = read_half.read(&mut buf) => {
                match read_res {
                    Ok(0) => {
                        let _ = inner.send_frame(TunnelFrame::Close {
                            stream_id,
                            reason: None,
                        }).await;
                        sent_close = true;
                        break;
                    }
                    Ok(n) => {
                        let payload = buf[..n].to_vec();
                        if !inner.send_frame(TunnelFrame::Data { stream_id, payload }).await {
                            break;
                        }
                    }
                    Err(err) => {
                        let _ = inner.send_frame(TunnelFrame::Close {
                            stream_id,
                            reason: Some(err.to_string()),
                        }).await;
                        sent_close = true;
                        break;
                    }
                }
            }
        }
    }

    inner.remove_stream(stream_id).await;
    if !sent_close {
        let _ = inner
            .send_frame(TunnelFrame::Close {
                stream_id,
                reason: None,
            })
            .await;
    }
    writer_task.abort();
    let _ = writer_task.await;
}

struct UdpClientSession {
    stream_id: u64,
    last_active: Arc<Mutex<Instant>>,
    task: JoinHandle<()>,
}

async fn run_udp_listener(
    inner: Arc<EngineInner>,
    rule: PortForwardRule,
    socket: Arc<UdpSocket>,
    mut rule_stop: watch::Receiver<bool>,
    mut global_shutdown: watch::Receiver<bool>,
) {
    let mut sessions: HashMap<SocketAddr, UdpClientSession> = HashMap::new();
    let mut evict_tick = tokio::time::interval(inner.config.udp_evict_interval);
    evict_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut recv_buf = vec![0u8; MAX_FRAME_PAYLOAD_BYTES];

    loop {
        tokio::select! {
            biased;
            _ = rule_stop.changed() => {
                if *rule_stop.borrow() {
                    break;
                }
            }
            _ = global_shutdown.changed() => {
                if *global_shutdown.borrow() {
                    break;
                }
            }
            _ = evict_tick.tick() => {
                let now = Instant::now();
                let mut expired_addrs = Vec::new();
                for (addr, session) in &sessions {
                    let last = *session.last_active.lock().await;
                    if now.saturating_duration_since(last) >= inner.config.udp_idle_timeout
                        || session.task.is_finished()
                    {
                        expired_addrs.push(*addr);
                    }
                }
                for addr in expired_addrs {
                    if let Some(session) = sessions.remove(&addr) {
                        session.task.abort();
                        inner.remove_stream(session.stream_id).await;
                        let _ = inner.send_frame(TunnelFrame::Close {
                            stream_id: session.stream_id,
                            reason: Some("udp idle timeout".to_string()),
                        }).await;
                    }
                }
            }
            recv_res = socket.recv_from(&mut recv_buf) => {
                let (n, client_addr) = match recv_res {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if n > MAX_FRAME_PAYLOAD_BYTES {
                    continue;
                }

                // Clean up finished session if peer closed it previously.
                if let Some(existing) = sessions.get(&client_addr)
                    && existing.task.is_finished()
                {
                    if let Some(dead) = sessions.remove(&client_addr) {
                        inner.remove_stream(dead.stream_id).await;
                    }
                }

                let stream_id = if let Some(existing) = sessions.get(&client_addr) {
                    *existing.last_active.lock().await = Instant::now();
                    existing.stream_id
                } else {
                    let new_id = inner.alloc_stream_id();
                    let Some(mut cmd_rx) = inner
                        .register_stream(new_id, rule.id.clone(), TunnelProtocol::Udp)
                        .await
                    else {
                        continue;
                    };
                    let open_frame = TunnelFrame::Open {
                        stream_id: new_id,
                        rule_id: rule.id.clone(),
                        protocol: TunnelProtocol::Udp,
                        target_host: rule.target_host.clone(),
                        target_port: rule.target_port,
                    };
                    if !inner.send_frame(open_frame).await {
                        inner.remove_stream(new_id).await;
                        continue;
                    }

                    let last_active = Arc::new(Mutex::new(Instant::now()));
                    let last_active_clone = Arc::clone(&last_active);
                    let sock_clone = Arc::clone(&socket);
                    let inner_clone = Arc::clone(&inner);
                    let mut shutdown_rx = inner.shutdown_tx.subscribe();

                    let task = tokio::spawn(async move {
                        loop {
                            tokio::select! {
                                biased;
                                _ = shutdown_rx.changed() => {
                                    if *shutdown_rx.borrow() {
                                        break;
                                    }
                                }
                                cmd = cmd_rx.recv() => {
                                    match cmd {
                                        Some(StreamCommand::Data(payload)) => {
                                            *last_active_clone.lock().await = Instant::now();
                                            let _ = sock_clone.send_to(&payload, client_addr).await;
                                        }
                                        Some(StreamCommand::Opened) => {}
                                        Some(StreamCommand::Close)
                                        | Some(StreamCommand::Error)
                                        | None => {
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        inner_clone.remove_stream(new_id).await;
                    });

                    sessions.insert(
                        client_addr,
                        UdpClientSession {
                            stream_id: new_id,
                            last_active,
                            task,
                        },
                    );
                    new_id
                };

                let payload = recv_buf[..n].to_vec();
                let _ = inner
                    .send_frame(TunnelFrame::Data { stream_id, payload })
                    .await;
            }
        }
    }

    for (_, session) in sessions.drain() {
        session.task.abort();
        inner.remove_stream(session.stream_id).await;
        let _ = inner
            .send_frame(TunnelFrame::Close {
                stream_id: session.stream_id,
                reason: Some("listener stopped".to_string()),
            })
            .await;
    }
}

async fn run_inbound_dispatcher(
    inner: Arc<EngineInner>,
    mut rx: mpsc::Receiver<TunnelFrame>,
    mut global_shutdown: watch::Receiver<bool>,
) {
    loop {
        let frame = tokio::select! {
            biased;
            _ = global_shutdown.changed() => {
                if *global_shutdown.borrow() {
                    break;
                }
                continue;
            }
            maybe_frame = rx.recv() => {
                match maybe_frame {
                    Some(f) => f,
                    None => break,
                }
            }
        };

        inner
            .metrics
            .frames_received
            .fetch_add(1, Ordering::Relaxed);
        if let TunnelFrame::Data { payload, .. } = &frame {
            inner
                .metrics
                .bytes_received
                .fetch_add(payload.len() as u64, Ordering::Relaxed);
        }

        match frame {
            TunnelFrame::Open {
                stream_id,
                rule_id,
                protocol,
                target_host,
                target_port,
            } => {
                handle_inbound_open(
                    Arc::clone(&inner),
                    stream_id,
                    rule_id,
                    protocol,
                    target_host,
                    target_port,
                )
                .await;
            }
            TunnelFrame::Opened { stream_id } => {
                let tx = inner
                    .streams
                    .read()
                    .await
                    .get(&stream_id)
                    .map(|entry| entry.cmd_tx.clone());
                if let Some(tx) = tx {
                    let _ = tx.send(StreamCommand::Opened).await;
                }
            }
            TunnelFrame::Data { stream_id, payload } => {
                if payload.len() > MAX_FRAME_PAYLOAD_BYTES {
                    inner.remove_stream(stream_id).await;
                    let _ = inner
                        .send_frame(TunnelFrame::Error {
                            stream_id,
                            code: TunnelErrorCode::PayloadTooLarge,
                            message: format!(
                                "payload {} exceeds max {}",
                                payload.len(),
                                MAX_FRAME_PAYLOAD_BYTES
                            ),
                        })
                        .await;
                    continue;
                }
                let tx = inner
                    .streams
                    .read()
                    .await
                    .get(&stream_id)
                    .map(|entry| entry.cmd_tx.clone());
                if let Some(tx) = tx {
                    if tx.send(StreamCommand::Data(payload)).await.is_err() {
                        inner.remove_stream(stream_id).await;
                    }
                }
            }
            TunnelFrame::Close { stream_id, .. } => {
                let entry = inner.streams.write().await.remove(&stream_id);
                if let Some(entry) = entry {
                    let _ = entry.cmd_tx.try_send(StreamCommand::Close);
                }
            }
            TunnelFrame::Error { stream_id, .. } => {
                let entry = inner.streams.write().await.remove(&stream_id);
                if let Some(entry) = entry {
                    let _ = entry.cmd_tx.try_send(StreamCommand::Error);
                }
            }
        }
    }

    inner.close_all_streams().await;
}

async fn handle_inbound_open(
    inner: Arc<EngineInner>,
    stream_id: u64,
    rule_id: String,
    protocol: TunnelProtocol,
    target_host: String,
    target_port: u16,
) {
    if target_port == 0 || !is_valid_host(&target_host, false) {
        let _ = inner
            .send_frame(TunnelFrame::Error {
                stream_id,
                code: TunnelErrorCode::InvalidFrame,
                message: format!("invalid target {target_host}:{target_port}"),
            })
            .await;
        return;
    }

    let Some(cmd_rx) = inner.register_stream(stream_id, rule_id, protocol).await else {
        let _ = inner
            .send_frame(TunnelFrame::Error {
                stream_id,
                code: TunnelErrorCode::StreamLimitReached,
                message: "stream limit reached or duplicate stream_id".to_string(),
            })
            .await;
        return;
    };

    let global_shutdown = inner.shutdown_tx.subscribe();
    match protocol {
        TunnelProtocol::Tcp => {
            tokio::spawn(async move {
                run_inbound_tcp_connector(
                    inner,
                    stream_id,
                    target_host,
                    target_port,
                    cmd_rx,
                    global_shutdown,
                )
                .await;
            });
        }
        TunnelProtocol::Udp => {
            tokio::spawn(async move {
                run_inbound_udp_connector(
                    inner,
                    stream_id,
                    target_host,
                    target_port,
                    cmd_rx,
                    global_shutdown,
                )
                .await;
            });
        }
    }
}

async fn run_inbound_tcp_connector(
    inner: Arc<EngineInner>,
    stream_id: u64,
    target_host: String,
    target_port: u16,
    cmd_rx: mpsc::Receiver<StreamCommand>,
    global_shutdown: watch::Receiver<bool>,
) {
    let connect_result = tokio::time::timeout(
        inner.config.connect_timeout,
        TcpStream::connect((target_host.as_str(), target_port)),
    )
    .await;

    let stream = match connect_result {
        Ok(Ok(stream)) => {
            let _ = stream.set_nodelay(true);
            stream
        }
        Ok(Err(err)) => {
            inner.remove_stream(stream_id).await;
            let _ = inner
                .send_frame(TunnelFrame::Error {
                    stream_id,
                    code: TunnelErrorCode::ConnectFailed,
                    message: err.to_string(),
                })
                .await;
            return;
        }
        Err(_) => {
            inner.remove_stream(stream_id).await;
            let _ = inner
                .send_frame(TunnelFrame::Error {
                    stream_id,
                    code: TunnelErrorCode::ConnectTimeout,
                    message: "target tcp connect timed out".to_string(),
                })
                .await;
            return;
        }
    };

    if !inner.send_frame(TunnelFrame::Opened { stream_id }).await {
        inner.remove_stream(stream_id).await;
        return;
    }

    pump_tcp_stream(inner, stream_id, stream, cmd_rx, Vec::new(), global_shutdown).await;
}

async fn run_inbound_udp_connector(
    inner: Arc<EngineInner>,
    stream_id: u64,
    target_host: String,
    target_port: u16,
    mut cmd_rx: mpsc::Receiver<StreamCommand>,
    mut global_shutdown: watch::Receiver<bool>,
) {
    let resolved = match tokio::net::lookup_host((target_host.as_str(), target_port)).await {
        Ok(mut addrs) => match addrs.next() {
            Some(addr) => addr,
            None => {
                inner.remove_stream(stream_id).await;
                let _ = inner
                    .send_frame(TunnelFrame::Error {
                        stream_id,
                        code: TunnelErrorCode::ConnectFailed,
                        message: format!("unable to resolve {target_host}:{target_port}"),
                    })
                    .await;
                return;
            }
        },
        Err(err) => {
            inner.remove_stream(stream_id).await;
            let _ = inner
                .send_frame(TunnelFrame::Error {
                    stream_id,
                    code: TunnelErrorCode::ConnectFailed,
                    message: err.to_string(),
                })
                .await;
            return;
        }
    };

    let bind_addr = if resolved.is_ipv6() {
        "[::]:0"
    } else {
        "0.0.0.0:0"
    };
    let socket = match UdpSocket::bind(bind_addr).await {
        Ok(s) => s,
        Err(err) => {
            inner.remove_stream(stream_id).await;
            let _ = inner
                .send_frame(TunnelFrame::Error {
                    stream_id,
                    code: TunnelErrorCode::IoError,
                    message: err.to_string(),
                })
                .await;
            return;
        }
    };
    if let Err(err) = socket.connect(resolved).await {
        inner.remove_stream(stream_id).await;
        let _ = inner
            .send_frame(TunnelFrame::Error {
                stream_id,
                code: TunnelErrorCode::ConnectFailed,
                message: err.to_string(),
            })
            .await;
        return;
    }

    if !inner.send_frame(TunnelFrame::Opened { stream_id }).await {
        inner.remove_stream(stream_id).await;
        return;
    }

    let mut recv_buf = vec![0u8; MAX_FRAME_PAYLOAD_BYTES];
    let mut evict_tick = tokio::time::interval(inner.config.udp_evict_interval);
    evict_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_active = Instant::now();

    loop {
        tokio::select! {
            biased;
            _ = global_shutdown.changed() => {
                if *global_shutdown.borrow() {
                    break;
                }
            }
            _ = evict_tick.tick() => {
                if last_active.elapsed() >= inner.config.udp_idle_timeout {
                    let _ = inner.send_frame(TunnelFrame::Close {
                        stream_id,
                        reason: Some("udp idle timeout".to_string()),
                    }).await;
                    break;
                }
            }
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(StreamCommand::Data(payload)) => {
                        last_active = Instant::now();
                        if socket.send(&payload).await.is_err() {
                            break;
                        }
                    }
                    Some(StreamCommand::Opened) => {}
                    Some(StreamCommand::Close) | Some(StreamCommand::Error) | None => {
                        break;
                    }
                }
            }
            recv_res = socket.recv(&mut recv_buf) => {
                match recv_res {
                    Ok(n) => {
                        last_active = Instant::now();
                        let payload = recv_buf[..n].to_vec();
                        if !inner.send_frame(TunnelFrame::Data { stream_id, payload }).await {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        }
    }

    inner.remove_stream(stream_id).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_rule(id: &str, protocol: TunnelProtocol, bind_port: u16, target_port: u16) -> PortForwardRule {
        PortForwardRule {
            id: id.to_string(),
            name: format!("rule-{id}"),
            protocol,
            direction: TunnelDirection::LocalToRemote,
            bind_host: "127.0.0.1".to_string(),
            bind_port,
            target_host: "127.0.0.1".to_string(),
            target_port,
            enabled: true,
        }
    }

    async fn reserve_free_tcp_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        port
    }

    async fn reserve_free_udp_port() -> u16 {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = socket.local_addr().unwrap().port();
        drop(socket);
        port
    }

    async fn wait_until<F, Fut>(timeout: Duration, mut pred: F) -> bool
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if pred().await {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
        pred().await
    }

    #[test]
    fn rule_validation_rejects_invalid_ports_hosts_loops_and_limits() {
        let valid = sample_rule("r1", TunnelProtocol::Tcp, 18080, 8080);
        assert!(valid.validate().is_ok());

        // Empty rule id
        let mut bad = valid.clone();
        bad.id = "   ".to_string();
        assert_eq!(bad.validate(), Err(RuleValidationError::EmptyRuleId));

        // Zero ports
        let mut bad = valid.clone();
        bad.bind_port = 0;
        assert_eq!(bad.validate(), Err(RuleValidationError::InvalidBindPort));

        let mut bad = valid.clone();
        bad.target_port = 0;
        assert_eq!(bad.validate(), Err(RuleValidationError::InvalidTargetPort));

        // Invalid hosts
        let mut bad = valid.clone();
        bad.bind_host = "bad host".to_string();
        assert!(matches!(
            bad.validate(),
            Err(RuleValidationError::InvalidBindHost(_))
        ));

        let mut bad = valid.clone();
        bad.target_host = "0.0.0.0".to_string();
        assert!(matches!(
            bad.validate(),
            Err(RuleValidationError::InvalidTargetHost(_))
        ));

        // Loop guards
        let mut loop_same = valid.clone();
        loop_same.bind_host = "127.0.0.1".to_string();
        loop_same.target_host = "localhost".to_string();
        loop_same.bind_port = 9090;
        loop_same.target_port = 9090;
        assert!(matches!(
            loop_same.validate(),
            Err(RuleValidationError::LoopbackSelfForward { port: 9090, .. })
        ));

        let mut loop_wildcard = valid.clone();
        loop_wildcard.bind_host = "0.0.0.0".to_string();
        loop_wildcard.target_host = "127.0.0.1".to_string();
        loop_wildcard.bind_port = 9091;
        loop_wildcard.target_port = 9091;
        assert!(matches!(
            loop_wildcard.validate(),
            Err(RuleValidationError::LoopbackSelfForward { port: 9091, .. })
        ));

        // Rule set validation: max active rules, duplicate id, duplicate bind
        let r1 = sample_rule("r1", TunnelProtocol::Tcp, 10001, 20001);
        let r2 = sample_rule("r2", TunnelProtocol::Tcp, 10002, 20002);
        let mut r3_disabled = sample_rule("r3", TunnelProtocol::Tcp, 10001, 20003);
        r3_disabled.enabled = false;
        assert!(validate_rules(&[r1.clone(), r2.clone(), r3_disabled], 2).is_ok());
        assert_eq!(
            validate_rules(&[r1.clone(), r2.clone()], 1),
            Err(RuleValidationError::MaxActiveRulesExceeded { active: 2, max: 1 })
        );

        let dup_id = sample_rule("r1", TunnelProtocol::Udp, 10005, 20005);
        assert_eq!(
            validate_rules(&[r1.clone(), dup_id], 4),
            Err(RuleValidationError::DuplicateRuleId("r1".to_string()))
        );

        let dup_bind = sample_rule("r4", TunnelProtocol::Tcp, 10001, 20009);
        assert!(matches!(
            validate_rules(&[r1, dup_bind], 4),
            Err(RuleValidationError::DuplicateBindAddress {
                bind_port: 10001,
                ..
            })
        ));
    }

    #[test]
    fn tunnel_frame_roundtrip_and_payload_bounds() {
        let frames = vec![
            TunnelFrame::Open {
                stream_id: 42,
                rule_id: "ssh".to_string(),
                protocol: TunnelProtocol::Tcp,
                target_host: "127.0.0.1".to_string(),
                target_port: 22,
            },
            TunnelFrame::Opened { stream_id: 42 },
            TunnelFrame::Data {
                stream_id: 42,
                payload: vec![0xAB; MAX_FRAME_PAYLOAD_BYTES],
            },
            TunnelFrame::Close {
                stream_id: 42,
                reason: Some("eof".to_string()),
            },
            TunnelFrame::Error {
                stream_id: 42,
                code: TunnelErrorCode::ConnectFailed,
                message: "connection refused".to_string(),
            },
        ];

        for frame in frames {
            assert_eq!(frame.stream_id(), 42);
            let encoded = frame.encode().expect("encode should succeed");
            let decoded = TunnelFrame::decode(&encoded).expect("decode should succeed");
            assert_eq!(decoded, frame);
        }

        let oversized = TunnelFrame::Data {
            stream_id: 7,
            payload: vec![0u8; MAX_FRAME_PAYLOAD_BYTES + 1],
        };
        assert!(matches!(
            oversized.encode(),
            Err(TunnelCodecError::PayloadTooLarge { .. })
        ));

        let huge_wire = vec![0u8; MAX_TUNNEL_FRAME_WIRE_BYTES + 1];
        assert!(matches!(
            TunnelFrame::decode(&huge_wire),
            Err(TunnelCodecError::WireFrameTooLarge { .. })
        ));
    }

    #[tokio::test]
    async fn tcp_end_to_end_echo_and_graceful_close() {
        // Start a real loopback TCP echo server.
        let echo_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let echo_port = echo_listener.local_addr().unwrap().port();
        let echo_server = tokio::spawn(async move {
            while let Ok((mut sock, _)) = echo_listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    loop {
                        match sock.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                if sock.write_all(&buf[..n]).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                });
            }
        });

        let (client_transport, host_transport) = loopback_transport_pair(128);
        let cfg = PortForwardConfig {
            tcp_read_chunk_bytes: 1024,
            ..Default::default()
        };
        let client_engine = PortForwardEngine::spawn(client_transport, cfg.clone());
        let host_engine = PortForwardEngine::spawn(host_transport, cfg);

        let bind_port = reserve_free_tcp_port().await;
        let rule = sample_rule("tcp-echo", TunnelProtocol::Tcp, bind_port, echo_port);
        let bound_addr = client_engine
            .add_rule(rule)
            .await
            .expect("add_rule should succeed")
            .expect("enabled rule should bind");

        // Run two concurrent TCP clients sending multi-chunk payloads.
        let payload_a: Vec<u8> = (0..3500u32).map(|i| (i % 251) as u8).collect();
        let payload_b: Vec<u8> = (0..2200u32).map(|i| ((i * 7) % 251) as u8).collect();

        let p_a = payload_a.clone();
        let p_b = payload_b.clone();
        let (res_a, res_b) = tokio::join!(
            async move {
                let mut stream = TcpStream::connect(bound_addr).await.unwrap();
                stream.write_all(&p_a).await.unwrap();
                let mut echoed = vec![0u8; p_a.len()];
                stream.read_exact(&mut echoed).await.unwrap();
                echoed
            },
            async move {
                let mut stream = TcpStream::connect(bound_addr).await.unwrap();
                stream.write_all(&p_b).await.unwrap();
                let mut echoed = vec![0u8; p_b.len()];
                stream.read_exact(&mut echoed).await.unwrap();
                echoed
            }
        );

        assert_eq!(res_a, payload_a);
        assert_eq!(res_b, payload_b);

        // Both clients dropped -> both engines must clean up streams back to 0.
        let cleaned = wait_until(Duration::from_secs(2), || async {
            client_engine.stats().await.active_streams == 0
                && host_engine.stats().await.active_streams == 0
        })
        .await;
        assert!(cleaned, "expected all TCP streams to be cleaned up");

        assert!(client_engine.stats().await.bytes_sent >= (payload_a.len() + payload_b.len()) as u64);
        assert!(host_engine.stats().await.bytes_sent >= (payload_a.len() + payload_b.len()) as u64);

        // Removing the rule stops the listener.
        assert!(client_engine.remove_rule("tcp-echo").await.unwrap());
        assert!(TcpStream::connect(bound_addr).await.is_err());

        client_engine.shutdown().await;
        host_engine.shutdown().await;
        echo_server.abort();
    }

    #[tokio::test]
    async fn udp_end_to_end_datagram_forwarding_and_idle_eviction() {
        // Start a real loopback UDP echo server.
        let echo_sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let echo_port = echo_sock.local_addr().unwrap().port();
        let echo_server = tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            while let Ok((n, peer)) = echo_sock.recv_from(&mut buf).await {
                let _ = echo_sock.send_to(&buf[..n], peer).await;
            }
        });

        let (client_transport, host_transport) = loopback_transport_pair(128);
        let cfg = PortForwardConfig {
            udp_idle_timeout: Duration::from_millis(160),
            udp_evict_interval: Duration::from_millis(30),
            ..Default::default()
        };
        let client_engine = PortForwardEngine::spawn(client_transport, cfg.clone());
        let host_engine = PortForwardEngine::spawn(host_transport, cfg);

        let bind_port = reserve_free_udp_port().await;
        let rule = sample_rule("udp-echo", TunnelProtocol::Udp, bind_port, echo_port);
        let bound_addr = client_engine
            .add_rule(rule)
            .await
            .expect("add_rule should succeed")
            .expect("enabled UDP rule should bind");

        // Two distinct UDP client sockets multiplexed over the same forwarding rule.
        let client1 = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let client2 = UdpSocket::bind("127.0.0.1:0").await.unwrap();

        client1.send_to(b"hello-from-client-1", bound_addr).await.unwrap();
        client2.send_to(b"hello-from-client-2", bound_addr).await.unwrap();

        let mut buf1 = [0u8; 128];
        let mut buf2 = [0u8; 128];
        let (n1, _) = tokio::time::timeout(Duration::from_secs(2), client1.recv_from(&mut buf1))
            .await
            .expect("client1 reply timeout")
            .unwrap();
        let (n2, _) = tokio::time::timeout(Duration::from_secs(2), client2.recv_from(&mut buf2))
            .await
            .expect("client2 reply timeout")
            .unwrap();

        assert_eq!(&buf1[..n1], b"hello-from-client-1");
        assert_eq!(&buf2[..n2], b"hello-from-client-2");

        assert_eq!(client_engine.stats().await.active_udp_sessions, 2);
        assert_eq!(host_engine.stats().await.active_udp_sessions, 2);

        // Wait for idle timeout eviction on both sides.
        let evicted = wait_until(Duration::from_secs(2), || async {
            let cs = client_engine.stats().await;
            let hs = host_engine.stats().await;
            cs.active_streams == 0
                && cs.active_udp_sessions == 0
                && hs.active_streams == 0
                && hs.active_udp_sessions == 0
        })
        .await;
        assert!(evicted, "expected idle UDP sessions to be evicted on both peers");

        client_engine.shutdown().await;
        host_engine.shutdown().await;
        echo_server.abort();
    }

    #[tokio::test]
    async fn remote_to_local_forwarding_and_connect_error_cleanup() {
        let echo_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let echo_port = echo_listener.local_addr().unwrap().port();
        let echo_server = tokio::spawn(async move {
            while let Ok((mut sock, _)) = echo_listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    if let Ok(n) = sock.read(&mut buf).await
                        && n > 0
                    {
                        let _ = sock.write_all(&buf[..n]).await;
                    }
                });
            }
        });

        let (client_transport, host_transport) = loopback_transport_pair(64);
        let client_engine = PortForwardEngine::spawn(client_transport, PortForwardConfig::default());
        let host_engine = PortForwardEngine::spawn(host_transport, PortForwardConfig::default());

        // RemoteToLocal: host listens and forwards into client's local echo server.
        let reverse_bind_port = reserve_free_tcp_port().await;
        let reverse_rule = PortForwardRule {
            id: "rev-tcp".to_string(),
            name: "reverse-echo".to_string(),
            protocol: TunnelProtocol::Tcp,
            direction: TunnelDirection::RemoteToLocal,
            bind_host: "127.0.0.1".to_string(),
            bind_port: reverse_bind_port,
            target_host: "127.0.0.1".to_string(),
            target_port: echo_port,
            enabled: true,
        };
        let rev_addr = host_engine
            .add_rule(reverse_rule)
            .await
            .unwrap()
            .unwrap();

        let mut sock = TcpStream::connect(rev_addr).await.unwrap();
        sock.write_all(b"reverse-ping").await.unwrap();
        let mut resp = [0u8; 12];
        sock.read_exact(&mut resp).await.unwrap();
        assert_eq!(&resp, b"reverse-ping");
        drop(sock);

        // Also verify connect-refused target cleans up cleanly.
        let closed_target_port = reserve_free_tcp_port().await;
        let err_bind_port = reserve_free_tcp_port().await;
        let err_rule = sample_rule("err-tcp", TunnelProtocol::Tcp, err_bind_port, closed_target_port);
        let err_addr = client_engine.add_rule(err_rule).await.unwrap().unwrap();
        let mut failing_sock = TcpStream::connect(err_addr).await.unwrap();
        let mut probe = [0u8; 16];
        let n = tokio::time::timeout(Duration::from_secs(2), failing_sock.read(&mut probe))
            .await
            .expect("expected prompt EOF on target connect failure")
            .unwrap();
        assert_eq!(n, 0);

        let cleaned = wait_until(Duration::from_secs(2), || async {
            client_engine.stats().await.active_streams == 0
                && host_engine.stats().await.active_streams == 0
        })
        .await;
        assert!(cleaned, "expected stream maps to be empty after close & connect error");

        client_engine.shutdown().await;
        host_engine.shutdown().await;
        echo_server.abort();
    }
}
