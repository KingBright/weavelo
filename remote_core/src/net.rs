use crate::session_crypto::{MULTIPLEX_ENCRYPTED, SessionCrypto};
use protocol::{CompactRealtimeError, ControlMessage, DataEnvelope, RtpPacket};
use std::error::Error;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

pub const DEFAULT_CONTROL_PORT: u16 = 39271;
/// Target UDP send/recv socket buffer size for high-bitrate LAN & 4K/144Hz/500Mbps bursts.
/// Applied with graceful halving fallback when OS limits (e.g. macOS `kern.ipc.maxsockbuf`)
/// reject the full target size.
pub const UDP_BUFFER_TARGET_BYTES: usize = 8 * 1024 * 1024;
/// Minimum fallback buffer floor when stepping down from `UDP_BUFFER_TARGET_BYTES`.
pub const UDP_BUFFER_MIN_FALLBACK_BYTES: usize = 256 * 1024;
const RELAY_TUNNEL_UDP_RECV_BUFFER_BYTES: usize = 4 * 1024 * 1024;
const RELAY_TUNNEL_UDP_SEND_BUFFER_BYTES: usize = UDP_BUFFER_TARGET_BYTES;
const RELAY_TCP_SEND_BUFFER_BYTES: usize = 256 * 1024;
const RELAY_TCP_RECV_BUFFER_BYTES: usize = 512 * 1024;
#[cfg(any(target_os = "macos", target_os = "ios"))]
const TCP_NOTSENT_LOWAT_OPT: libc::c_int = 0x201; // <netinet/tcp.h> TCP_NOTSENT_LOWAT
#[cfg(target_os = "linux")]
const TCP_NOTSENT_LOWAT_OPT: libc::c_int = 25; // <linux/tcp.h> TCP_NOTSENT_LOWAT
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "linux"))]
const RELAY_TCP_NOTSENT_LOWAT_BYTES: libc::c_int = 64 * 1024;

/// Effective requested UDP socket buffer sizes that succeeded after OS fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UdpBufferSizingResult {
    pub requested_recv_bytes: Option<usize>,
    pub requested_send_bytes: Option<usize>,
}

/// Attempts to configure a socket buffer to `target_bytes`, stepping down by
/// halves to `min_bytes` when the OS rejects larger sizes (for example, macOS
/// returning `ENOBUFS` when exceeding `kern.ipc.maxsockbuf`).
fn set_socket_buffer_with_fallback<F>(
    target_bytes: usize,
    min_bytes: usize,
    mut apply: F,
) -> Option<usize>
where
    F: FnMut(usize) -> std::io::Result<()>,
{
    let floor = min_bytes.max(1);
    let mut candidate = target_bytes.max(floor);
    loop {
        if apply(candidate).is_ok() {
            return Some(candidate);
        }
        if candidate <= floor {
            return None;
        }
        candidate = (candidate / 2).max(floor);
    }
}

fn apply_udp_buffer_sizes(
    socket: &socket2::SockRef<'_>,
    target_recv_bytes: usize,
    target_send_bytes: usize,
) -> UdpBufferSizingResult {
    let requested_recv_bytes = set_socket_buffer_with_fallback(
        target_recv_bytes,
        UDP_BUFFER_MIN_FALLBACK_BYTES,
        |bytes| socket.set_recv_buffer_size(bytes),
    );
    let requested_send_bytes = set_socket_buffer_with_fallback(
        target_send_bytes,
        UDP_BUFFER_MIN_FALLBACK_BYTES,
        |bytes| socket.set_send_buffer_size(bytes),
    );
    UdpBufferSizingResult {
        requested_recv_bytes,
        requested_send_bytes,
    }
}

/// Relay adapters carry the same fragmented frames as the application socket.
/// Small OS defaults can discard encoded references on these local UDP hops
/// before the reliable relay transport sees them. Keep the existing bounded
/// application buffer policy on every adapter socket as well, with graceful OS
/// fallback so high-bitrate requests never leave sockets at tiny OS defaults.
pub(crate) fn configure_udp_buffers(socket: &UdpSocket) -> UdpBufferSizingResult {
    let socket = socket2::SockRef::from(socket);
    apply_udp_buffer_sizes(&socket, UDP_BUFFER_TARGET_BYTES, UDP_BUFFER_TARGET_BYTES)
}

/// Relay TCP/WebSocket local UDP adapters must accommodate high-bitrate bursts
/// of fragments without silently hoarding multi-second megabytes of stale video.
pub(crate) fn configure_relay_tunnel_udp_buffers(socket: &UdpSocket) -> UdpBufferSizingResult {
    let socket = socket2::SockRef::from(socket);
    apply_udp_buffer_sizes(
        &socket,
        RELAY_TUNNEL_UDP_RECV_BUFFER_BYTES,
        RELAY_TUNNEL_UDP_SEND_BUFFER_BYTES,
    )
}

/// Tune real-time relay TCP sockets to prevent multi-second kernel bufferbloat
/// while keeping enough window (256 KB send / 512 KB recv) to saturate >50 Mbps.
/// Setting `TCP_NOTSENT_LOWAT` on macOS/Linux ensures backpressure reaches the
/// WebSocket writer within tens of milliseconds instead of megabytes later.
pub(crate) fn configure_relay_tcp_stream(stream: &tokio::net::TcpStream) {
    let _ = stream.set_nodelay(true);
    let sock = socket2::SockRef::from(stream);
    let _ = sock.set_send_buffer_size(RELAY_TCP_SEND_BUFFER_BYTES);
    let _ = sock.set_recv_buffer_size(RELAY_TCP_RECV_BUFFER_BYTES);
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "linux"))]
    {
        use std::os::fd::AsRawFd;
        let lowat = RELAY_TCP_NOTSENT_LOWAT_BYTES;
        unsafe {
            let _ = libc::setsockopt(
                stream.as_raw_fd(),
                libc::IPPROTO_TCP,
                TCP_NOTSENT_LOWAT_OPT,
                (&lowat as *const libc::c_int).cast(),
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        }
    }
}

pub struct UdpMultiplexer {
    socket: Arc<UdpSocket>,
}

impl UdpMultiplexer {
    pub async fn bind(addr: &str) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let std_addr: SocketAddr = addr.parse()?;
        let socket2_sock = socket2::Socket::new(
            if std_addr.is_ipv4() {
                socket2::Domain::IPV4
            } else {
                socket2::Domain::IPV6
            },
            socket2::Type::DGRAM,
            None,
        )?;

        let sock_ref = socket2::SockRef::from(&socket2_sock);
        let _ = apply_udp_buffer_sizes(&sock_ref, UDP_BUFFER_TARGET_BYTES, UDP_BUFFER_TARGET_BYTES);
        // Keep default Best-Effort DSCP (0x00) instead of EF (0xB8 / Wi-Fi AC_VO):
        // high-bitrate video bursts overflow tiny Wi-Fi voice queues and lose fragments.
        socket2_sock.set_nonblocking(true)?;
        socket2_sock.bind(&std_addr.into())?;

        let std_socket: std::net::UdpSocket = socket2_sock.into();
        let socket = UdpSocket::from_std(std_socket)?;

        Ok(Self {
            socket: Arc::new(socket),
        })
    }

    pub fn split(&self) -> (UdpSender, UdpReceiver) {
        let crypto = Arc::new(OnceLock::new());
        let peer_crypto = Arc::new(RwLock::new(HashMap::new()));
        (
            UdpSender {
                socket: self.socket.clone(),
                crypto: crypto.clone(),
                peer_crypto: peer_crypto.clone(),
            },
            UdpReceiver {
                socket: self.socket.clone(),
                fragments: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
                crypto,
                peer_crypto,
            },
        )
    }

    pub fn local_addr(&self) -> Result<SocketAddr, std::io::Error> {
        self.socket.local_addr()
    }
}

#[derive(Clone)]
pub struct UdpSender {
    socket: Arc<UdpSocket>,
    crypto: Arc<OnceLock<SessionCrypto>>,
    peer_crypto: Arc<RwLock<HashMap<SocketAddr, Arc<SessionCrypto>>>>,
}
impl UdpSender {
    pub fn install_crypto(&self, crypto: SessionCrypto) -> bool {
        self.crypto.set(crypto).is_ok()
    }

    pub fn install_peer_crypto(&self, peer: SocketAddr, crypto: SessionCrypto) {
        self.peer_crypto
            .write()
            .unwrap()
            .insert(peer, Arc::new(crypto));
    }

    pub fn peer_session_id(&self, peer: SocketAddr) -> Option<[u8; 16]> {
        self.peer_crypto
            .read()
            .unwrap()
            .get(&peer)
            .map(|crypto| crypto.session_id())
            .or_else(|| self.crypto.get().map(SessionCrypto::session_id))
    }

    pub fn remove_peer_crypto(&self, peer: SocketAddr) {
        self.peer_crypto.write().unwrap().remove(&peer);
    }

    async fn send_multiplexed(
        &self,
        header: u8,
        bytes: &[u8],
        target: SocketAddr,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let peer_key = self.peer_crypto.read().unwrap().get(&target).cloned();
        if let Some(crypto) = peer_key.as_deref().or_else(|| self.crypto.get()) {
            let mut inner = Vec::with_capacity(1 + bytes.len());
            inner.push(header);
            inner.extend_from_slice(bytes);
            let sealed = crypto.seal(&inner)?;
            return self.send_raw(MULTIPLEX_ENCRYPTED, &sealed, target).await;
        }
        self.send_raw(header, bytes, target).await
    }

    async fn send_raw(
        &self,
        header: u8,
        bytes: &[u8],
        target: SocketAddr,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        if bytes.len() > MAX_FRAGMENT_PAYLOAD * MAX_FRAGMENT_CHUNKS {
            return Err("UDP message exceeds fragment size limit".into());
        }
        // Keep individual datagrams strictly below standard PPPoE/WAN/5G MTU
        // even after RoutedRelay (RPMX) and Relay (RPR1) encapsulation headers.
        // Only fall back to MAX_FRAGMENT_PAYLOAD for rare >4.5 MB messages so
        // the 4096-chunk ceiling remains unchanged.
        let max_payload = if bytes.len() <= SENDER_FRAGMENT_PAYLOAD * MAX_FRAGMENT_CHUNKS {
            SENDER_FRAGMENT_PAYLOAD
        } else {
            MAX_FRAGMENT_PAYLOAD
        };
        if bytes.len() <= max_payload {
            let mut buf = [0u8; MAX_FRAGMENT_PAYLOAD + 1];
            buf[0] = header;
            buf[1..1 + bytes.len()].copy_from_slice(bytes);
            self.socket.send_to(&buf[..1 + bytes.len()], target).await?;
        } else {
            // Fragment the packet
            // Format: 0x03, original_header(1), fragment_id(4), chunk_idx(2), total_chunks(2), data...
            let fragment_id = rand::random::<u32>();
            let chunks = bytes.chunks(max_payload);
            let total_chunks_usize = chunks.len();
            let total_chunks = total_chunks_usize as u16;
            let is_loopback = target.ip().is_loopback();
            let pace_burst = if is_loopback {
                32
            } else {
                (total_chunks_usize / 4).max(16)
            };

            let mut buf = [0u8; 1410];
            buf[0] = 0x03; // Fragment multiplex header
            buf[1] = header;
            buf[2..6].copy_from_slice(&fragment_id.to_be_bytes());
            buf[8..10].copy_from_slice(&total_chunks.to_be_bytes());

            for (i, chunk) in chunks.enumerate() {
                buf[6..8].copy_from_slice(&(i as u16).to_be_bytes());
                buf[10..10 + chunk.len()].copy_from_slice(chunk);
                self.socket
                    .send_to(&buf[..10 + chunk.len()], target)
                    .await?;
                let sent = i + 1;
                if sent < total_chunks_usize && sent % pace_burst == 0 {
                    if is_loopback {
                        tokio::task::yield_now().await;
                    } else if total_chunks_usize > 16 {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn send_rtp(
        &self,
        packet: &RtpPacket,
        target: SocketAddr,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let bytes = packet.encode()?;
        self.send_multiplexed(0x01, &bytes, target).await
    }

    pub async fn send_control(
        &self,
        msg: &ControlMessage,
        target: SocketAddr,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let bytes = msg.encode()?;
        // Session handshake must remain plaintext so peers can derive the AEAD key.
        if matches!(
            msg,
            ControlMessage::SessionHandshake(_)
                | ControlMessage::SessionHello { .. }
                | ControlMessage::SessionAccept { .. }
                | ControlMessage::SessionReject { .. }
        ) {
            return self.send_raw(0x02, &bytes, target).await;
        }
        self.send_multiplexed(0x02, &bytes, target).await
    }

    pub async fn send_data(
        &self,
        envelope: &DataEnvelope,
        target: SocketAddr,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.send_data_with_timing(envelope, envelope.transport_timing, target)
            .await
    }

    pub async fn send_data_with_timing(
        &self,
        envelope: &DataEnvelope,
        timing: Option<protocol::FrameTimingCheckpoints>,
        target: SocketAddr,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let timing = timing.or(envelope.transport_timing);
        match envelope.encode_compact_realtime_with_timing(timing) {
            Ok(bytes) => self.send_multiplexed(0x05, &bytes, target).await,
            Err(
                CompactRealtimeError::NonRealtimeLane(_)
                | CompactRealtimeError::UnsupportedRealtimeMetadata,
            ) => {
                let bytes = envelope.encode()?;
                self.send_multiplexed(0x04, &bytes, target).await
            }
            Err(err) => Err(Box::new(err)),
        }
    }
}

use std::collections::{HashMap, hash_map::Entry};
use tokio::sync::Mutex;

const MAX_FRAGMENT_AGE: Duration = Duration::from_millis(500);
const MAX_FRAGMENT_CACHE_ENTRIES: usize = 64;
const MAX_FRAGMENT_PAYLOAD: usize = 1400;
const SENDER_FRAGMENT_PAYLOAD: usize = 1120;
const MAX_FRAGMENT_CHUNKS: usize = 4096;

type FragmentChunks = Vec<Option<Vec<u8>>>;
// Fragment IDs are only unique within one sender and multiplexed content type.
type FragmentKey = (SocketAddr, u8, u32);
type FragmentMap = HashMap<FragmentKey, FragmentEntry>;

#[derive(Debug)]
struct FragmentEntry {
    received_chunks: u16,
    received_bytes: usize,
    chunks: FragmentChunks,
    created_at: Instant,
}

impl FragmentEntry {
    fn new(total_chunks: u16, now: Instant) -> Self {
        Self {
            received_chunks: 0,
            received_bytes: 0,
            chunks: vec![None; total_chunks as usize],
            created_at: now,
        }
    }
}

/// A rejected datagram is not an authenticated application command and not a
/// socket failure. Exposing it as SessionCommand::Error would let invalid input
/// influence an otherwise valid session's user-visible state.
#[derive(Debug)]
pub struct UdpPacketRejection {
    pub peer: SocketAddr,
    cause: Box<dyn Error + Send + Sync>,
}
impl std::fmt::Display for UdpPacketRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.cause.fmt(f)
    }
}
impl Error for UdpPacketRejection {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.cause.as_ref())
    }
}
fn rejected_packet(
    peer: SocketAddr,
    cause: impl Into<Box<dyn Error + Send + Sync>>,
) -> Box<dyn Error + Send + Sync> {
    Box::new(UdpPacketRejection {
        peer,
        cause: cause.into(),
    })
}

pub struct UdpReceiver {
    socket: Arc<UdpSocket>,
    fragments: Arc<Mutex<FragmentMap>>,
    crypto: Arc<OnceLock<SessionCrypto>>,
    peer_crypto: Arc<RwLock<HashMap<SocketAddr, Arc<SessionCrypto>>>>,
}

#[derive(Debug)]
pub enum MultiplexedPacket {
    Rtp(RtpPacket, SocketAddr),
    Control(ControlMessage, SocketAddr),
    ClockProbe(protocol::session_auth::ClockProbe, u64, SocketAddr),
    ClockReply(protocol::session_auth::ClockReply, u64, SocketAddr),
    Data(DataEnvelope, SocketAddr),
    DataWithTiming(
        DataEnvelope,
        Option<protocol::FrameTimingCheckpoints>,
        SocketAddr,
    ),
}

impl UdpReceiver {
    /// Reply on the same route and with the same session encryption as ingress.
    pub fn sender(&self) -> UdpSender {
        UdpSender {
            socket: self.socket.clone(),
            crypto: self.crypto.clone(),
            peer_crypto: self.peer_crypto.clone(),
        }
    }

    pub fn install_crypto(&self, crypto: SessionCrypto) -> bool {
        self.crypto.set(crypto).is_ok()
    }

    fn decode_payload(
        &self,
        buf: &[u8],
        len: usize,
        addr: SocketAddr,
    ) -> Result<Option<MultiplexedPacket>, Box<dyn Error + Send + Sync>> {
        self.decode_payload_at(buf, len, addr, crate::timing::quanta_now_us())
    }

    fn decode_payload_at(
        &self,
        buf: &[u8],
        len: usize,
        addr: SocketAddr,
        received_us: u64,
    ) -> Result<Option<MultiplexedPacket>, Box<dyn Error + Send + Sync>> {
        self.decode_payload_inner(buf, len, addr, None, received_us)
            .map_err(|error| rejected_packet(addr, error))
    }

    fn decode_payload_inner(
        &self,
        buf: &[u8],
        len: usize,
        addr: SocketAddr,
        authenticated: Option<[u8; 16]>,
        received_us: u64,
    ) -> Result<Option<MultiplexedPacket>, Box<dyn Error + Send + Sync>> {
        if len == 0 {
            return Ok(None);
        }
        let peer_key = self.peer_crypto.read().unwrap().get(&addr).cloned();
        let key = peer_key.as_deref().or_else(|| self.crypto.get());
        if authenticated.is_none() && key.is_some() && buf[0] != MULTIPLEX_ENCRYPTED {
            let handshake = buf[0] == 0x02
                && matches!(
                    ControlMessage::decode(&buf[1..len]),
                    Ok(ControlMessage::SessionHandshake(_)
                        | ControlMessage::SessionHello { .. }
                        | ControlMessage::SessionAccept { .. }
                        | ControlMessage::SessionReject { .. })
                );
            if !handshake {
                return Err("plaintext packet rejected for encrypted peer".into());
            }
        }
        match buf[0] {
            MULTIPLEX_ENCRYPTED => {
                if authenticated.is_some() {
                    return Err("nested encryption is not supported".into());
                }
                let Some(crypto) = key else {
                    return Err("encrypted packet received without session crypto".into());
                };
                let opened = crypto.open(&buf[1..len])?;
                self.decode_payload_inner(
                    &opened,
                    opened.len(),
                    addr,
                    Some(crypto.session_id()),
                    received_us,
                )
            }
            0x01 => {
                let rtp = RtpPacket::decode(&buf[1..len])?;
                Ok(Some(MultiplexedPacket::Rtp(rtp, addr)))
            }
            0x02 => {
                let msg = ControlMessage::decode(&buf[1..len])?;
                match msg {
                    ControlMessage::ClockProbe(probe)
                        if authenticated == Some(probe.session_id) =>
                    {
                        Ok(Some(MultiplexedPacket::ClockProbe(
                            probe,
                            received_us,
                            addr,
                        )))
                    }
                    ControlMessage::ClockReply(reply)
                        if authenticated == Some(reply.probe.session_id) =>
                    {
                        Ok(Some(MultiplexedPacket::ClockReply(
                            reply,
                            received_us,
                            addr,
                        )))
                    }
                    ControlMessage::ClockProbe(_) | ControlMessage::ClockReply(_) => {
                        Err("clock measurements require the current authenticated session".into())
                    }
                    other => Ok(Some(MultiplexedPacket::Control(other, addr))),
                }
            }
            0x04 => {
                let envelope = DataEnvelope::decode(&buf[1..len])?;
                Ok(Some(MultiplexedPacket::Data(envelope, addr)))
            }
            0x05 => {
                let (envelope, timing) =
                    DataEnvelope::decode_compact_realtime_with_timing(&buf[1..len])?;
                if timing.is_some() {
                    Ok(Some(MultiplexedPacket::DataWithTiming(
                        envelope, timing, addr,
                    )))
                } else {
                    Ok(Some(MultiplexedPacket::Data(envelope, addr)))
                }
            }
            _ => Err("Unknown UDP multiplexing header".into()),
        }
    }

    fn cleanup_expired_fragments(fragments: &mut FragmentMap, now: Instant) {
        fragments.retain(|_, entry| now.duration_since(entry.created_at) <= MAX_FRAGMENT_AGE);
    }

    fn enforce_fragment_cache_limit(fragments: &mut FragmentMap, protected_id: FragmentKey) {
        while fragments.len() > MAX_FRAGMENT_CACHE_ENTRIES {
            let oldest_unprotected_id = fragments
                .iter()
                .filter(|(id, _)| **id != protected_id)
                .min_by_key(|(_, entry)| entry.created_at)
                .map(|(id, _)| *id);

            if let Some(id) = oldest_unprotected_id {
                fragments.remove(&id);
            } else {
                break;
            }
        }
    }

    pub async fn recv(&self) -> Result<MultiplexedPacket, Box<dyn Error + Send + Sync>> {
        // Keep the established receive-buffer layout: the small-buffer variant
        // regressed small-packet loopback performance on the macOS target.
        let mut buf = [0u8; 65536];
        loop {
            let (len, addr) = self.socket.recv_from(&mut buf).await?;
            let received_us = crate::timing::quanta_now_us();
            if len == 0 {
                continue;
            }
            if len > MAX_FRAGMENT_PAYLOAD + 10 {
                return Err(rejected_packet(
                    addr,
                    if buf[0] == 0x03 {
                        "Invalid UDP fragment payload size"
                    } else {
                        "UDP datagram exceeds size limit"
                    },
                ));
            }
            match buf[0] {
                0x03 => {
                    if len < 10 {
                        continue;
                    }
                    let header = buf[1];
                    let fragment_id = u32::from_be_bytes(buf[2..6].try_into().unwrap());
                    let chunk_idx = u16::from_be_bytes(buf[6..8].try_into().unwrap());
                    let total_chunks = u16::from_be_bytes(buf[8..10].try_into().unwrap());
                    let fragment_key = (addr, header, fragment_id);

                    if total_chunks == 0 || total_chunks as usize > MAX_FRAGMENT_CHUNKS {
                        return Err(rejected_packet(
                            addr,
                            "Invalid UDP fragment total chunk count",
                        ));
                    }

                    if chunk_idx >= total_chunks {
                        return Err(rejected_packet(addr, "Invalid UDP fragment index"));
                    }
                    if len == 10 || len - 10 > MAX_FRAGMENT_PAYLOAD {
                        return Err(rejected_packet(addr, "Invalid UDP fragment payload size"));
                    }
                    if !matches!(header, 0x01 | 0x02 | 0x04 | 0x05 | MULTIPLEX_ENCRYPTED) {
                        return Err(rejected_packet(
                            addr,
                            "Invalid UDP fragment multiplexing header",
                        ));
                    }

                    let mut fragments = self.fragments.lock().await;
                    let now = Instant::now();
                    Self::cleanup_expired_fragments(&mut fragments, now);

                    let entry = match fragments.entry(fragment_key) {
                        Entry::Occupied(entry) => {
                            if entry.get().chunks.len() != total_chunks as usize {
                                return Err(rejected_packet(
                                    addr,
                                    "Mismatched UDP fragment total chunk count",
                                ));
                            }
                            entry.into_mut()
                        }
                        Entry::Vacant(entry) => entry.insert(FragmentEntry::new(total_chunks, now)),
                    };

                    if entry.chunks[chunk_idx as usize].is_none() {
                        entry.chunks[chunk_idx as usize] = Some(buf[10..len].to_vec());
                        entry.received_chunks += 1;
                        entry.received_bytes += len - 10;
                    }

                    if entry.received_chunks == total_chunks {
                        let entry = fragments.remove(&fragment_key).expect("completed fragment");
                        drop(fragments);
                        let mut assembled = Vec::with_capacity(1 + entry.received_bytes);
                        assembled.push(header);
                        for chunk in entry.chunks.into_iter().flatten() {
                            assembled.extend_from_slice(&chunk);
                        }
                        if let Some(packet) =
                            self.decode_payload_at(&assembled, assembled.len(), addr, received_us)?
                        {
                            return Ok(packet);
                        }
                    } else {
                        Self::enforce_fragment_cache_limit(&mut fragments, fragment_key);
                    }
                }
                _ => {
                    if let Some(packet) =
                        self.decode_payload_at(&buf[..len], len, addr, received_us)?
                    {
                        return Ok(packet);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_plane::{realtime_data_to_rtp, rtp_to_realtime_data};
    use protocol::{ChunkInfo, ContentKind, DataEnvelope, PayloadType, RtpHeader, RtpPacket};
    use std::time::Duration;
    use tokio::time::timeout;

    fn rtp_packet(payload_len: usize) -> RtpPacket {
        RtpPacket {
            header: RtpHeader {
                version: 2,
                payload_type: 96,
                sequence_number: 42,
                timestamp: 123_456,
                ssrc: 99,
            },
            payload: (0..payload_len).map(|i| (i % 251) as u8).collect(),
        }
    }

    fn data_envelope(payload_len: usize) -> DataEnvelope {
        DataEnvelope::realtime_video(
            99,
            42,
            123_456,
            123_472,
            (0..payload_len).map(|i| (i % 251) as u8).collect(),
        )
    }

    fn audio_packet(payload_len: usize) -> RtpPacket {
        RtpPacket {
            header: RtpHeader {
                version: 2,
                payload_type: PayloadType::AudioOpus as u8,
                sequence_number: 43,
                timestamp: 960,
                ssrc: 100,
            },
            payload: (0..payload_len).map(|i| (i % 251) as u8).collect(),
        }
    }

    fn reliable_envelope(payload_len: usize) -> DataEnvelope {
        DataEnvelope::reliable_object_chunk(
            ContentKind::FileChunk,
            7,
            44,
            123_456,
            ChunkInfo {
                object_id: 99,
                chunk_index: 0,
                total_chunks: 1,
                offset: 0,
                total_size: payload_len as u64,
            },
            None,
            (0..payload_len).map(|i| (i % 251) as u8).collect(),
        )
    }

    async fn bind_pair() -> (UdpMultiplexer, UdpMultiplexer) {
        let left = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("left socket should bind");
        let right = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("right socket should bind");
        (left, right)
    }

    async fn recv_with_timeout(receiver: &UdpReceiver) -> MultiplexedPacket {
        timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("receive should not time out")
            .expect("packet should decode")
    }

    async fn send_raw_fragment(
        target: SocketAddr,
        fragment_id: u32,
        chunk_idx: u16,
        total_chunks: u16,
        data: &[u8],
    ) -> UdpSocket {
        let raw_sender = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("raw sender should bind");
        send_fragment_from(
            &raw_sender,
            target,
            0x01,
            fragment_id,
            chunk_idx,
            total_chunks,
            data,
        )
        .await;
        raw_sender
    }

    async fn send_fragment_from(
        raw_sender: &UdpSocket,
        target: SocketAddr,
        header: u8,
        fragment_id: u32,
        chunk_idx: u16,
        total_chunks: u16,
        data: &[u8],
    ) {
        let mut bytes = Vec::with_capacity(10 + data.len());
        bytes.push(0x03);
        bytes.push(header);
        bytes.extend_from_slice(&fragment_id.to_be_bytes());
        bytes.extend_from_slice(&chunk_idx.to_be_bytes());
        bytes.extend_from_slice(&total_chunks.to_be_bytes());
        bytes.extend_from_slice(data);

        raw_sender
            .send_to(&bytes, target)
            .await
            .expect("raw fragment send should succeed");
    }

    async fn recv_error(receiver: &UdpReceiver) -> String {
        timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("receive should not time out")
            .expect_err("packet should fail")
            .to_string()
    }

    fn fragment_entry(total_chunks: u16, created_at: Instant) -> FragmentEntry {
        FragmentEntry::new(total_chunks, created_at)
    }

    fn fragment_key(id: u32) -> FragmentKey {
        ("127.0.0.1:39271".parse().unwrap(), 0x01, id)
    }

    #[tokio::test]
    async fn authenticated_clock_records_keep_ingress_timestamp_and_reject_replay_plaintext_and_wrong_session()
     {
        use crate::session_crypto::SessionRole;
        let (left, right) = bind_pair().await;
        let addr = left.local_addr().unwrap();
        let (tx, rx) = right.split();
        let client =
            crate::SessionCrypto::from_psk(b"clock-key", b"clock-salt", SessionRole::Client)
                .unwrap();
        tx.install_peer_crypto(
            addr,
            crate::SessionCrypto::from_psk(b"clock-key", b"clock-salt", SessionRole::Host).unwrap(),
        );
        let probe = protocol::session_auth::ClockProbe {
            session_id: client.session_id(),
            probe_id: 1,
            client_send_us: 99,
        };
        let mut plain = vec![2];
        plain.extend(ControlMessage::ClockProbe(probe).encode().unwrap());
        assert!(
            rx.decode_payload_at(&plain, plain.len(), addr, 100)
                .is_err()
        );
        let mut sealed = vec![MULTIPLEX_ENCRYPTED];
        sealed.extend(client.seal(&plain).unwrap());
        assert!(
            matches!(rx.decode_payload_at(&sealed,sealed.len(),addr,12345).unwrap(),Some(MultiplexedPacket::ClockProbe(p,12345,a))if p==probe&&a==addr)
        );
        assert!(
            rx.decode_payload_at(&sealed, sealed.len(), addr, 12346)
                .is_err()
        );
        let wrong = protocol::session_auth::ClockProbe {
            session_id: [0; 16],
            ..probe
        };
        let mut inner = vec![2];
        inner.extend(ControlMessage::ClockProbe(wrong).encode().unwrap());
        let mut sealed = vec![MULTIPLEX_ENCRYPTED];
        sealed.extend(client.seal(&inner).unwrap());
        assert!(
            rx.decode_payload_at(&sealed, sealed.len(), addr, 12347)
                .is_err()
        );
    }

    #[tokio::test]
    async fn receive_remains_usable_after_cancellation() {
        let (left, right) = bind_pair().await;
        let (sender, _) = left.split();
        let (_, receiver) = right.split();
        assert!(
            timeout(Duration::from_millis(5), receiver.recv())
                .await
                .is_err()
        );
        let packet = rtp_packet(32);
        sender
            .send_rtp(&packet, right.local_addr().unwrap())
            .await
            .unwrap();
        match recv_with_timeout(&receiver).await {
            MultiplexedPacket::Rtp(actual, _) => assert_eq!(actual, packet),
            other => panic!("unexpected packet: {other:?}"),
        }
    }

    #[tokio::test]
    async fn same_fragment_id_from_different_peers_is_isolated() {
        let (_, right) = bind_pair().await;
        let target = right.local_addr().unwrap();
        let (_, receiver) = right.split();
        let first = rtp_packet(64);
        let mut second = first.clone();
        second.payload.fill(9);
        let a = first.encode().unwrap();
        let b = second.encode().unwrap();
        let sender_a = send_raw_fragment(target, 7, 1, 2, &a[40..]).await;
        let sender_b = send_raw_fragment(target, 7, 0, 2, &b[..40]).await;
        send_fragment_from(&sender_a, target, 0x01, 7, 0, 2, &a[..40]).await;
        send_fragment_from(&sender_b, target, 0x01, 7, 1, 2, &b[40..]).await;
        for (expected, sender) in [(first, sender_a), (second, sender_b)] {
            match recv_with_timeout(&receiver).await {
                MultiplexedPacket::Rtp(packet, addr) => {
                    assert_eq!(packet, expected);
                    assert_eq!(addr, sender.local_addr().unwrap());
                }
                packet => panic!("unexpected packet: {packet:?}"),
            }
        }
        assert!(receiver.fragments.lock().await.is_empty());
    }

    #[tokio::test]
    async fn invalid_fragment_sizes_are_rejected_before_allocating() {
        let (_, right) = bind_pair().await;
        let target = right.local_addr().unwrap();
        let (_, receiver) = right.split();
        for (count, payload, error) in [
            (u16::MAX, vec![1], "Invalid UDP fragment total chunk count"),
            (
                2,
                vec![1; MAX_FRAGMENT_PAYLOAD + 1],
                "Invalid UDP fragment payload size",
            ),
            (2, vec![], "Invalid UDP fragment payload size"),
        ] {
            send_raw_fragment(target, 1, 0, count, &payload).await;
            assert_eq!(recv_error(&receiver).await, error);
            assert!(receiver.fragments.lock().await.is_empty());
        }
    }

    #[tokio::test]
    async fn configure_relay_tcp_stream_enables_nodelay_and_bounded_buffers() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (client, server) = tokio::join!(
            tokio::net::TcpStream::connect(addr),
            listener.accept(),
        );
        let client = client.unwrap();
        let (server, _) = server.unwrap();
        configure_relay_tcp_stream(&client);
        configure_relay_tcp_stream(&server);
        assert!(client.nodelay().unwrap());
        assert!(server.nodelay().unwrap());
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            use std::os::fd::AsRawFd;
            let mut actual: libc::c_int = 0;
            let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
            let rc = unsafe {
                libc::getsockopt(
                    client.as_raw_fd(),
                    libc::IPPROTO_TCP,
                    TCP_NOTSENT_LOWAT_OPT,
                    (&mut actual as *mut libc::c_int).cast(),
                    &mut len,
                )
            };
            assert_eq!(rc, 0, "getsockopt(TCP_NOTSENT_LOWAT) should succeed");
            assert_eq!(actual, RELAY_TCP_NOTSENT_LOWAT_BYTES);
        }
    }

    #[tokio::test]
    async fn sender_rejects_unrepresentable_message_sizes() {
        let (left, right) = bind_pair().await;
        let (sender, _) = left.split();
        let payload = vec![0; MAX_FRAGMENT_PAYLOAD * MAX_FRAGMENT_CHUNKS + 1];
        let error = sender
            .send_raw(0x01, &payload, right.local_addr().unwrap())
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "UDP message exceeds fragment size limit");
    }

    #[test]
    fn cleanup_expired_fragments_removes_old_entries() {
        let now = Instant::now();
        let mut fragments = FragmentMap::new();
        fragments.insert(
            fragment_key(1),
            fragment_entry(2, now - MAX_FRAGMENT_AGE - Duration::from_millis(1)),
        );
        fragments.insert(fragment_key(2), fragment_entry(2, now));

        UdpReceiver::cleanup_expired_fragments(&mut fragments, now);

        assert!(!fragments.contains_key(&fragment_key(1)));
        assert!(fragments.contains_key(&fragment_key(2)));
    }

    #[test]
    fn enforce_fragment_cache_limit_removes_oldest_unprotected_entries() {
        let now = Instant::now();
        let mut fragments = FragmentMap::new();

        for id in 0..(MAX_FRAGMENT_CACHE_ENTRIES as u32 + 2) {
            fragments.insert(
                fragment_key(id),
                fragment_entry(2, now + Duration::from_millis(id as u64)),
            );
        }

        UdpReceiver::enforce_fragment_cache_limit(&mut fragments, fragment_key(0));

        assert_eq!(fragments.len(), MAX_FRAGMENT_CACHE_ENTRIES);
        assert!(fragments.contains_key(&fragment_key(0)));
        assert!(!fragments.contains_key(&fragment_key(1)));
        assert!(!fragments.contains_key(&fragment_key(2)));
    }

    #[tokio::test]
    async fn sends_and_receives_control_messages() {
        let (left, right) = bind_pair().await;
        let left_addr = left
            .socket
            .local_addr()
            .expect("left should have local addr");
        let right_addr = right
            .socket
            .local_addr()
            .expect("right should have local addr");
        let (sender, _) = left.split();
        let (_, receiver) = right.split();

        sender
            .send_control(&ControlMessage::Heartbeat, right_addr)
            .await
            .expect("control send should succeed");

        match recv_with_timeout(&receiver).await {
            MultiplexedPacket::Control(ControlMessage::Heartbeat, addr) => {
                assert_eq!(addr, left_addr);
            }
            other => panic!("unexpected packet: {other:?}"),
        }
    }

    #[tokio::test]
    async fn encrypted_control_roundtrips_after_crypto_install() {
        let (left, right) = bind_pair().await;
        let right_addr = right
            .socket
            .local_addr()
            .expect("right should have local addr");
        let (sender, _) = left.split();
        let (_, receiver) = right.split();
        let psk = b"test-psk";
        let salt = b"salt-16-bytes!!!!";
        sender.install_crypto(
            crate::SessionCrypto::from_psk(psk, salt, crate::session_crypto::SessionRole::Client)
                .unwrap(),
        );
        receiver.install_crypto(
            crate::SessionCrypto::from_psk(psk, salt, crate::session_crypto::SessionRole::Host)
                .unwrap(),
        );

        sender
            .send_control(&ControlMessage::Heartbeat, right_addr)
            .await
            .expect("encrypted control send should succeed");

        match recv_with_timeout(&receiver).await {
            MultiplexedPacket::Control(ControlMessage::Heartbeat, _) => {}
            other => panic!("unexpected packet: {other:?}"),
        }
    }

    #[tokio::test]
    async fn peer_keys_are_isolated_and_authenticated_peers_cannot_downgrade_to_plaintext() {
        let server = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let first = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let second = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();
        let (server_tx, server_rx) = server.split();
        let (first_tx, _) = first.split();
        let (second_tx, _) = second.split();
        for (peer, tx, salt) in [
            (&first, &first_tx, b"first-salt-123456"),
            (&second, &second_tx, b"second-salt-12345"),
        ] {
            server_tx.install_peer_crypto(
                peer.local_addr().unwrap(),
                SessionCrypto::from_psk(b"shared", salt, crate::session_crypto::SessionRole::Host)
                    .unwrap(),
            );
            tx.install_peer_crypto(
                addr,
                SessionCrypto::from_psk(
                    b"shared",
                    salt,
                    crate::session_crypto::SessionRole::Client,
                )
                .unwrap(),
            );
            tx.send_control(&ControlMessage::Heartbeat, addr)
                .await
                .unwrap();
            assert!(
                matches!(recv_with_timeout(&server_rx).await, MultiplexedPacket::Control(ControlMessage::Heartbeat, source) if source == peer.local_addr().unwrap())
            );
        }
        first_tx
            .send_control(&ControlMessage::Heartbeat, addr)
            .await
            .unwrap();
        assert!(
            matches!(recv_with_timeout(&server_rx).await, MultiplexedPacket::Control(ControlMessage::Heartbeat, source) if source == first.local_addr().unwrap())
        );
        let mut plain = vec![0x02];
        plain.extend(ControlMessage::Heartbeat.encode().unwrap());
        assert!(
            server_rx
                .decode_payload(&plain, plain.len(), first.local_addr().unwrap())
                .is_err()
        );
        let mut wrong = vec![MULTIPLEX_ENCRYPTED];
        wrong.extend(
            SessionCrypto::from_psk(
                b"shared",
                b"second-salt-12345",
                crate::session_crypto::SessionRole::Client,
            )
            .unwrap()
            .seal(&plain)
            .unwrap(),
        );
        assert!(
            server_rx
                .decode_payload(&wrong, wrong.len(), first.local_addr().unwrap())
                .is_err()
        );
    }

    #[tokio::test]
    async fn sends_and_receives_rtp_packets() {
        let (left, right) = bind_pair().await;
        let right_addr = right
            .socket
            .local_addr()
            .expect("right should have local addr");
        let (sender, _) = left.split();
        let (_, receiver) = right.split();
        let packet = rtp_packet(32);

        sender
            .send_rtp(&packet, right_addr)
            .await
            .expect("rtp send should succeed");

        match recv_with_timeout(&receiver).await {
            MultiplexedPacket::Rtp(decoded, _) => assert_eq!(decoded, packet),
            other => panic!("unexpected packet: {other:?}"),
        }
    }

    #[tokio::test]
    async fn reassembles_fragmented_rtp_packets() {
        let (left, right) = bind_pair().await;
        let right_addr = right
            .socket
            .local_addr()
            .expect("right should have local addr");
        let (sender, _) = left.split();
        let (_, receiver) = right.split();
        let packet = rtp_packet(5_000);

        sender
            .send_rtp(&packet, right_addr)
            .await
            .expect("fragmented rtp send should succeed");

        match recv_with_timeout(&receiver).await {
            MultiplexedPacket::Rtp(decoded, _) => assert_eq!(decoded, packet),
            other => panic!("unexpected packet: {other:?}"),
        }
    }

    #[tokio::test]
    async fn sends_and_receives_data_envelopes() {
        let (left, right) = bind_pair().await;
        let right_addr = right
            .socket
            .local_addr()
            .expect("right should have local addr");
        let (sender, _) = left.split();
        let (_, receiver) = right.split();
        let envelope = data_envelope(32);

        sender
            .send_data(&envelope, right_addr)
            .await
            .expect("data send should succeed");

        match recv_with_timeout(&receiver).await {
            MultiplexedPacket::Data(decoded, _) => assert_eq!(decoded, envelope),
            other => panic!("unexpected packet: {other:?}"),
        }
    }

    #[tokio::test]
    async fn reassembles_fragmented_data_envelopes() {
        let (left, right) = bind_pair().await;
        let right_addr = right
            .socket
            .local_addr()
            .expect("right should have local addr");
        let (sender, _) = left.split();
        let (_, receiver) = right.split();
        let envelope = data_envelope(5_000);

        sender
            .send_data(&envelope, right_addr)
            .await
            .expect("fragmented data send should succeed");

        match recv_with_timeout(&receiver).await {
            MultiplexedPacket::Data(decoded, _) => assert_eq!(decoded, envelope),
            other => panic!("unexpected packet: {other:?}"),
        }
    }

    #[tokio::test]
    async fn sends_and_receives_video_media_data_plane_packets() {
        let (left, right) = bind_pair().await;
        let right_addr = right
            .socket
            .local_addr()
            .expect("right should have local addr");
        let (sender, _) = left.split();
        let (_, receiver) = right.split();
        let packet = rtp_packet(512);
        let envelope = rtp_to_realtime_data(&packet).expect("video packet should adapt");

        sender
            .send_data(&envelope, right_addr)
            .await
            .expect("media data send should succeed");

        match recv_with_timeout(&receiver).await {
            MultiplexedPacket::Data(decoded, _) => {
                let decoded_packet =
                    realtime_data_to_rtp(decoded).expect("media data should adapt back");
                assert_eq!(decoded_packet, packet);
            }
            other => panic!("unexpected packet: {other:?}"),
        }
    }

    #[tokio::test]
    async fn sends_and_receives_audio_media_data_plane_packets_without_deadline() {
        let (left, right) = bind_pair().await;
        let right_addr = right
            .socket
            .local_addr()
            .expect("right should have local addr");
        let (sender, _) = left.split();
        let (_, receiver) = right.split();
        let packet = audio_packet(96);
        let envelope = rtp_to_realtime_data(&packet).expect("audio packet should adapt");
        assert_eq!(envelope.header.deadline_ms, None);

        sender
            .send_data(&envelope, right_addr)
            .await
            .expect("media data send should succeed");

        match recv_with_timeout(&receiver).await {
            MultiplexedPacket::Data(decoded, _) => {
                assert_eq!(decoded.header.deadline_ms, None);
                let decoded_packet =
                    realtime_data_to_rtp(decoded).expect("media data should adapt back");
                assert_eq!(decoded_packet, packet);
            }
            other => panic!("unexpected packet: {other:?}"),
        }
    }

    #[tokio::test]
    async fn sends_and_receives_reliable_object_envelopes_on_full_data_path() {
        let (left, right) = bind_pair().await;
        let right_addr = right
            .socket
            .local_addr()
            .expect("right should have local addr");
        let (sender, _) = left.split();
        let (_, receiver) = right.split();
        let envelope = reliable_envelope(512);

        sender
            .send_data(&envelope, right_addr)
            .await
            .expect("reliable object data send should succeed");

        match recv_with_timeout(&receiver).await {
            MultiplexedPacket::Data(decoded, _) => assert_eq!(decoded, envelope),
            other => panic!("unexpected packet: {other:?}"),
        }
    }

    #[tokio::test]
    async fn returns_error_for_unknown_header() {
        let receiver_mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("receiver should bind");
        let receiver_addr = receiver_mux
            .socket
            .local_addr()
            .expect("receiver should have local addr");
        let (_, receiver) = receiver_mux.split();
        let raw_sender = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("raw sender should bind");

        raw_sender
            .send_to(&[0xff, 0x00], receiver_addr)
            .await
            .expect("raw send should succeed");

        let err = timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("receive should not time out")
            .expect_err("unknown header should fail");

        assert_eq!(err.to_string(), "Unknown UDP multiplexing header");
    }

    #[tokio::test]
    async fn returns_error_for_zero_fragment_count() {
        let receiver_mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("receiver should bind");
        let receiver_addr = receiver_mux
            .socket
            .local_addr()
            .expect("receiver should have local addr");
        let (_, receiver) = receiver_mux.split();

        send_raw_fragment(receiver_addr, 1, 0, 0, b"bad").await;

        assert_eq!(
            recv_error(&receiver).await,
            "Invalid UDP fragment total chunk count"
        );
    }

    #[tokio::test]
    async fn returns_error_for_out_of_range_fragment_index() {
        let receiver_mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("receiver should bind");
        let receiver_addr = receiver_mux
            .socket
            .local_addr()
            .expect("receiver should have local addr");
        let (_, receiver) = receiver_mux.split();

        send_raw_fragment(receiver_addr, 1, 1, 1, b"bad").await;

        assert_eq!(recv_error(&receiver).await, "Invalid UDP fragment index");
    }

    #[tokio::test]
    async fn returns_error_for_mismatched_fragment_count() {
        let receiver_mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("receiver should bind");
        let receiver_addr = receiver_mux
            .socket
            .local_addr()
            .expect("receiver should have local addr");
        let (_, receiver) = receiver_mux.split();

        let raw_sender = send_raw_fragment(receiver_addr, 7, 0, 2, b"first").await;
        send_fragment_from(&raw_sender, receiver_addr, 0x01, 7, 1, 3, b"second").await;

        assert_eq!(
            recv_error(&receiver).await,
            "Mismatched UDP fragment total chunk count"
        );
    }

    #[tokio::test]
    async fn sends_and_receives_data_envelopes_with_timing() {
        let (left, right) = bind_pair().await;
        let right_addr = right
            .socket
            .local_addr()
            .expect("right should have local addr");
        let (sender, _) = left.split();
        let (_, receiver) = right.split();
        let envelope = data_envelope(32);
        let mut timing = protocol::FrameTimingCheckpoints::new(123_456_000);
        timing.encode_queue_ts_us = 100;
        timing.encode_done_ts_us = 200;
        timing.packetize_ts_us = 300;
        timing.send_ts_us = 400;

        sender
            .send_data_with_timing(&envelope, Some(timing), right_addr)
            .await
            .expect("data send with timing should succeed");

        match recv_with_timeout(&receiver).await {
            MultiplexedPacket::DataWithTiming(decoded, decoded_timing, _) => {
                assert_eq!(
                    decoded,
                    envelope.clone().with_transport_timing(Some(timing))
                );
                assert_eq!(decoded_timing, Some(timing));
            }
            other => panic!("unexpected packet: {other:?}"),
        }
    }

    #[tokio::test]
    async fn reassembles_fragmented_data_envelopes_with_timing() {
        let (left, right) = bind_pair().await;
        let right_addr = right
            .socket
            .local_addr()
            .expect("right should have local addr");
        let (sender, _) = left.split();
        let (_, receiver) = right.split();
        let envelope = data_envelope(5_000);
        let mut timing = protocol::FrameTimingCheckpoints::new(999_888_000);
        timing.encode_queue_ts_us = 150;
        timing.encode_done_ts_us = 250;
        timing.packetize_ts_us = 350;
        timing.send_ts_us = 450;

        sender
            .send_data_with_timing(&envelope, Some(timing), right_addr)
            .await
            .expect("fragmented data send with timing should succeed");

        match recv_with_timeout(&receiver).await {
            MultiplexedPacket::DataWithTiming(decoded, decoded_timing, _) => {
                assert_eq!(
                    decoded,
                    envelope.clone().with_transport_timing(Some(timing))
                );
                assert_eq!(decoded_timing, Some(timing));
            }
            other => panic!("unexpected packet: {other:?}"),
        }
    }

    #[test]
    fn udp_buffer_fallback_steps_down_gracefully_until_os_accepts() {
        // Immediate accept at full 8 MiB target.
        let mut attempts = Vec::new();
        let applied = set_socket_buffer_with_fallback(
            UDP_BUFFER_TARGET_BYTES,
            UDP_BUFFER_MIN_FALLBACK_BYTES,
            |bytes| {
                attempts.push(bytes);
                Ok(())
            },
        );
        assert_eq!(applied, Some(UDP_BUFFER_TARGET_BYTES));
        assert_eq!(attempts, vec![8 * 1024 * 1024]);

        // OS rejects > 4 MiB (e.g. macOS kern.ipc.maxsockbuf ceiling).
        let mut attempts = Vec::new();
        let applied = set_socket_buffer_with_fallback(
            UDP_BUFFER_TARGET_BYTES,
            UDP_BUFFER_MIN_FALLBACK_BYTES,
            |bytes| {
                attempts.push(bytes);
                if bytes <= 4 * 1024 * 1024 {
                    Ok(())
                } else {
                    Err(std::io::Error::from_raw_os_error(libc::ENOBUFS))
                }
            },
        );
        assert_eq!(applied, Some(4 * 1024 * 1024));
        assert_eq!(attempts, vec![8 * 1024 * 1024, 4 * 1024 * 1024]);

        // OS rejects > 1 MiB.
        let mut attempts = Vec::new();
        let applied = set_socket_buffer_with_fallback(
            UDP_BUFFER_TARGET_BYTES,
            UDP_BUFFER_MIN_FALLBACK_BYTES,
            |bytes| {
                attempts.push(bytes);
                if bytes <= 1024 * 1024 {
                    Ok(())
                } else {
                    Err(std::io::Error::from_raw_os_error(libc::ENOBUFS))
                }
            },
        );
        assert_eq!(applied, Some(1024 * 1024));
        assert_eq!(
            attempts,
            vec![
                8 * 1024 * 1024,
                4 * 1024 * 1024,
                2 * 1024 * 1024,
                1024 * 1024
            ]
        );

        // OS rejects every candidate down to floor.
        let mut attempts = Vec::new();
        let applied = set_socket_buffer_with_fallback(
            UDP_BUFFER_TARGET_BYTES,
            UDP_BUFFER_MIN_FALLBACK_BYTES,
            |bytes| {
                attempts.push(bytes);
                Err(std::io::Error::from_raw_os_error(libc::ENOBUFS))
            },
        );
        assert_eq!(applied, None);
        assert_eq!(
            attempts,
            vec![
                8 * 1024 * 1024,
                4 * 1024 * 1024,
                2 * 1024 * 1024,
                1024 * 1024,
                512 * 1024,
                256 * 1024,
            ]
        );
    }

    #[tokio::test]
    async fn udp_multiplexer_and_relay_adapters_configure_high_bitrate_buffers_with_fallback() {
        let mux = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let mux_ref = socket2::SockRef::from(mux.socket.as_ref());
        let recv_size = mux_ref.recv_buffer_size().unwrap();
        let send_size = mux_ref.send_buffer_size().unwrap();
        assert!(
            recv_size >= UDP_BUFFER_MIN_FALLBACK_BYTES,
            "expected recv buffer >= {} bytes, got {}",
            UDP_BUFFER_MIN_FALLBACK_BYTES,
            recv_size
        );
        assert!(
            send_size >= UDP_BUFFER_MIN_FALLBACK_BYTES,
            "expected send buffer >= {} bytes, got {}",
            UDP_BUFFER_MIN_FALLBACK_BYTES,
            send_size
        );

        let direct_sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let direct_res = configure_udp_buffers(&direct_sock);
        assert!(direct_res.requested_recv_bytes.is_some());
        assert!(direct_res.requested_send_bytes.is_some());

        let relay_sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let relay_res = configure_relay_tunnel_udp_buffers(&relay_sock);
        assert!(relay_res.requested_recv_bytes.is_some());
        assert!(relay_res.requested_send_bytes.is_some());
        assert!(
            relay_res.requested_recv_bytes.unwrap() <= RELAY_TUNNEL_UDP_RECV_BUFFER_BYTES
        );
    }
}
