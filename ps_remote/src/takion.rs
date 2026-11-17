use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use prost::Message;
use rand::RngExt;
use thiserror::Error;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch};
use tokio::time::{interval, interval_at, timeout, MissedTickBehavior};

use crate::ecdh::Ecdh;
use crate::fec::fec_decode;
use crate::feedback::{
    format_feedback_state_v12, FeedbackHistoryBuffer, Ps5ControllerState, RumbleEvent,
    TriggerEffectsEvent,
};
use crate::gkcrypt::{GkCrypt, KeyState};
use crate::launchspec::{encode_launchspec, LaunchSpecConfig};
use crate::proto::{self, PayloadType};
use crate::rpcrypt::Rpcrypt;
use crate::senkusha::SenkushaResult;

/// Default UDP port for Takion media + controller stream.
pub const TAKION_PORT: u16 = 9296;

const PKT_CONTROL: u8 = 0x00;
const PKT_FEEDBACK_HISTORY: u8 = 0x01;
const PKT_VIDEO: u8 = 0x02;
const PKT_AUDIO: u8 = 0x03;
const PKT_CONGESTION: u8 = 0x05;
const PKT_FEEDBACK_STATE: u8 = 0x06;

const CHUNK_DATA: u8 = 0x00;
const CHUNK_INIT: u8 = 0x01;
const CHUNK_INIT_ACK: u8 = 0x02;
const CHUNK_DATA_ACK: u8 = 0x03;
const CHUNK_HEARTBEAT: u8 = 0x06;
const CHUNK_HEARTBEAT_ACK: u8 = 0x07;
const CHUNK_COOKIE: u8 = 0x0a;
const CHUNK_COOKIE_ACK: u8 = 0x0b;

const DATA_FLAG_FRAG: u8 = 0x00;
const DATA_FLAG_FIN: u8 = 0x01;

const DATA_TYPE_PROTOBUF: u8 = 0;
const DATA_TYPE_RUMBLE: u8 = 7;
const DATA_TYPE_PAD_INFO: u8 = 9;
const DATA_TYPE_TRIGGER_EFFECTS: u8 = 11;

const AUDIO_CODEC_OPUS: u8 = 5;
const A_RWND_DEFAULT: u32 = 0x0001_9000;

#[derive(Debug, Error)]
pub enum TakionError {
    #[error("I/O error on PS5 Takion UDP socket: {0}")]
    Io(#[from] std::io::Error),
    #[error("Takion handshake timed out at stage: {0}")]
    Timeout(&'static str),
    #[error("Malformed Takion packet: {0}")]
    Malformed(String),
    #[error("Takion protobuf error: {0}")]
    Proto(#[from] prost::DecodeError),
    #[error("Takion ECDH / key exchange failed: {0}")]
    Crypto(String),
    #[error("Takion stream closed: {0}")]
    Closed(String),
}

/// Decoded audio stream parameters advertised by PS5 `STREAM_INFO`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TakionAudioConfig {
    pub channels: u8,
    pub bits: u8,
    pub sample_rate: u32,
    pub frame_size: u32,
}

impl Default for TakionAudioConfig {
    fn default() -> Self {
        Self {
            channels: 2,
            bits: 16,
            sample_rate: 48_000,
            frame_size: 480,
        }
    }
}

/// Events emitted by the PS5 Takion stream loop to the session / bridge.
#[derive(Debug, Clone)]
pub enum TakionEvent {
    AudioConfig(TakionAudioConfig),
    VideoHeaders(Vec<Vec<u8>>),
    VideoFrame {
        frame_index: u16,
        adaptive_stream_index: u8,
        data: Vec<u8>,
    },
    AudioFrame(Vec<u8>),
    AudioHaptics(Vec<u8>),
    Rumble(RumbleEvent),
    TriggerEffects(TriggerEffectsEvent),
    Disconnected(String),
}

/// Configuration used to establish a Takion v12 session on UDP 9296.
pub struct TakionConfig {
    pub host: IpAddr,
    pub port: u16,
    pub session_id: String,
    pub rpcrypt: Rpcrypt,
    pub launch_spec: LaunchSpecConfig,
    pub senkusha: SenkushaResult,
}

#[derive(Debug)]
enum TakionCommand {
    ControllerState(Ps5ControllerState),
    RequestIdr,
    Shutdown,
}

/// Async handle to an active PS5 Takion session.
#[derive(Clone)]
pub struct TakionHandle {
    cmd_tx: mpsc::Sender<TakionCommand>,
    closed_rx: watch::Receiver<Option<String>>,
}

impl TakionHandle {
    /// Enqueue the latest PS5 controller state for v12 feedback transmission.
    pub fn send_controller_state(&self, state: Ps5ControllerState) {
        let _ = self.cmd_tx.try_send(TakionCommand::ControllerState(state));
    }

    /// Ask the PS5 encoder for a fresh recovery / keyframe range.
    pub fn request_idr(&self) {
        let _ = self.cmd_tx.try_send(TakionCommand::RequestIdr);
    }

    /// Gracefully stop the Takion loop.
    pub async fn shutdown(&self) {
        let _ = self.cmd_tx.send(TakionCommand::Shutdown).await;
    }

    pub fn closed_reason(&self) -> Option<String> {
        self.closed_rx.borrow().clone()
    }
}

/// Active connected Takion transport plus initial audio configuration.
pub struct TakionSession {
    pub handle: TakionHandle,
    pub events: mpsc::Receiver<TakionEvent>,
    pub audio_config: TakionAudioConfig,
    pub video_headers: Vec<Vec<u8>>,
}

struct TakionTxCrypt {
    crypt: GkCrypt,
    next_pos: u64,
}

impl TakionTxCrypt {
    fn new(crypt: GkCrypt) -> Self {
        Self { crypt, next_pos: 0 }
    }

    fn current_pos(&self) -> u64 {
        self.next_pos
    }

    fn advance(&mut self, bytes: usize) -> u64 {
        let pos = self.next_pos;
        self.next_pos += bytes as u64;
        pos
    }
}

struct TakionRxCrypt {
    crypt: GkCrypt,
    key_state: KeyState,
}

impl TakionRxCrypt {
    fn new(crypt: GkCrypt) -> Self {
        Self {
            crypt,
            key_state: KeyState::new(),
        }
    }

    fn verify_and_decrypt(
        &mut self,
        key_pos_low: u32,
        rx_mac: &[u8; 4],
        mac_buf_with_zeroed_mac: &[u8],
        payload: &mut [u8],
    ) -> bool {
        let full_pos = self.key_state.request_pos(key_pos_low, false);
        let expected_mac = self.crypt.gmac(full_pos, mac_buf_with_zeroed_mac);
        if rx_mac != &expected_mac {
            return false;
        }
        self.key_state.commit(full_pos);
        self.crypt.crypt(full_pos + 16, payload);
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvPacketKind {
    Video,
    Audio,
}

/// Parsed Takion v12 AV UDP header prior to payload decryption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvPacketHeader {
    pub kind: AvPacketKind,
    pub uses_nalu_info_structs: bool,
    pub packet_index: u16,
    pub frame_index: u16,
    pub unit_index: u16,
    pub units_total: u16,
    pub units_fec: u16,
    pub codec: u8,
    pub mac: [u8; 4],
    pub key_pos_low: u32,
    pub adaptive_stream_index: u8,
    pub is_haptics: bool,
    pub payload_offset: usize,
}

/// Parse a Takion v12 video (`buf[0] & 0x0f == 2`) or audio (`buf[0] & 0x0f == 3`) header.
pub fn parse_av_header_v12(buf: &[u8]) -> Result<AvPacketHeader, TakionError> {
    if buf.len() < 20 {
        return Err(TakionError::Malformed("v12 AV packet shorter than 20B".into()));
    }
    let kind = match buf[0] & 0x0f {
        PKT_VIDEO => AvPacketKind::Video,
        PKT_AUDIO => AvPacketKind::Audio,
        other => {
            return Err(TakionError::Malformed(format!(
                "unexpected AV packet base type {other}"
            )))
        }
    };
    let uses_nalu_info_structs = (buf[0] & 0x10) != 0;
    let packet_index = u16::from_be_bytes([buf[1], buf[2]]);
    let frame_index = u16::from_be_bytes([buf[3], buf[4]]);
    let dword2 = u32::from_be_bytes([buf[5], buf[6], buf[7], buf[8]]);
    let codec = buf[9];
    let mut mac = [0u8; 4];
    mac.copy_from_slice(&buf[10..14]);
    let key_pos_low = u32::from_be_bytes([buf[14], buf[15], buf[16], buf[17]]);

    let nalu_extra = if uses_nalu_info_structs { 3 } else { 0 };
    match kind {
        AvPacketKind::Video => {
            let payload_offset = 21 + nalu_extra;
            if buf.len() < payload_offset {
                return Err(TakionError::Malformed(
                    "v12 video packet shorter than header".into(),
                ));
            }
            let unit_index = ((dword2 >> 21) & 0x7ff) as u16;
            let units_total = (((dword2 >> 10) & 0x7ff) + 1) as u16;
            let units_fec = (dword2 & 0x3ff) as u16;
            let adaptive_stream_index = buf[20] >> 5;
            Ok(AvPacketHeader {
                kind,
                uses_nalu_info_structs,
                packet_index,
                frame_index,
                unit_index,
                units_total,
                units_fec,
                codec,
                mac,
                key_pos_low,
                adaptive_stream_index,
                is_haptics: false,
                payload_offset,
            })
        }
        AvPacketKind::Audio => {
            let haptics_flag_off = 19 + nalu_extra;
            let payload_offset = 20 + nalu_extra;
            if buf.len() < payload_offset {
                return Err(TakionError::Malformed(
                    "v12 audio packet shorter than header".into(),
                ));
            }
            let unit_index = ((dword2 >> 24) & 0xff) as u16;
            let units_total = (((dword2 >> 16) & 0xff) + 1) as u16;
            let units_fec = (dword2 & 0xffff) as u16;
            let is_haptics = buf[haptics_flag_off] == 0x02;
            Ok(AvPacketHeader {
                kind,
                uses_nalu_info_structs,
                packet_index,
                frame_index,
                unit_index,
                units_total,
                units_fec,
                codec,
                mac,
                key_pos_low,
                adaptive_stream_index: 0,
                is_haptics,
                payload_offset,
            })
        }
    }
}

/// Format a 15-byte `CONGESTION` (`0x05`) packet with GMAC calculated over `[0..15]`
/// with MAC and key_pos (`7..15`) zeroed during GMAC (`takion.c`).
pub fn build_congestion_packet(
    word_0: u16,
    received: u16,
    lost: u16,
    crypt: &mut GkCrypt,
    key_pos: u64,
) -> [u8; 15] {
    let mut pkt = [0u8; 15];
    pkt[0] = PKT_CONGESTION;
    pkt[1..3].copy_from_slice(&word_0.to_be_bytes());
    pkt[3..5].copy_from_slice(&received.to_be_bytes());
    pkt[5..7].copy_from_slice(&lost.to_be_bytes());

    // Bytes 7..15 stay zeroed while computing GMAC, then key_pos is written afterwards.
    let gmac = crypt.gmac(key_pos, &pkt);
    pkt[7..11].copy_from_slice(&gmac);
    pkt[11..15].copy_from_slice(&(key_pos as u32).to_be_bytes());
    pkt
}

/// Format a `FEEDBACK_STATE` (`0x06`) or `FEEDBACK_HISTORY` (`0x01`) UDP packet.
fn build_feedback_packet_raw(
    packet_type: u8,
    seq: u16,
    count_minus_1: u8,
    plain_payload: &[u8],
    tx_crypt: &mut TakionTxCrypt,
) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(12 + plain_payload.len());
    pkt.resize(12, 0);
    pkt[0] = packet_type;
    pkt[1..3].copy_from_slice(&seq.to_be_bytes());
    pkt[3] = count_minus_1;

    let key_pos = tx_crypt.advance(plain_payload.len() + 16);
    pkt[4..8].copy_from_slice(&(key_pos as u32).to_be_bytes());

    pkt.extend_from_slice(plain_payload);
    tx_crypt.crypt.crypt(key_pos + 16, &mut pkt[12..]);

    let gmac = tx_crypt.crypt.gmac(key_pos, &pkt);
    pkt[8..12].copy_from_slice(&gmac);
    pkt
}

#[derive(Debug, Clone)]
struct PendingVideoFrame {
    frame_index: u16,
    adaptive_stream_index: u8,
    units_total: usize,
    units_source: usize,
    units_fec: usize,
    units: Vec<Option<Vec<u8>>>,
    received_total: usize,
    received_source: usize,
}

#[derive(Debug, Default)]
pub struct VideoFrameProcessor {
    frames: BTreeMap<u16, PendingVideoFrame>,
    completed: VecDeque<u16>,
    last_emitted_frame: Option<u16>,
}

impl VideoFrameProcessor {
    fn is_completed(&self, frame_index: u16) -> bool {
        self.completed.contains(&frame_index)
    }

    fn mark_completed(&mut self, frame_index: u16) {
        if self.completed.len() >= 64 {
            self.completed.pop_front();
        }
        self.completed.push_back(frame_index);
    }

    /// Insert one decrypted video unit.
    pub fn push_unit(
        &mut self,
        hdr: &AvPacketHeader,
        unit_payload: Vec<u8>,
    ) -> (Option<(u16, u8, Vec<u8>)>, Vec<(u16, u16)>) {
        let mut corrupt_ranges = Vec::new();
        let total = hdr.units_total as usize;
        let fec = hdr.units_fec as usize;
        let idx = hdr.unit_index as usize;
        if total == 0 || fec >= total || idx >= total {
            return (None, corrupt_ranges);
        }
        if self.is_completed(hdr.frame_index) {
            return (None, corrupt_ranges);
        }
        let source = total - fec;

        let stale_keys: Vec<u16> = self
            .frames
            .keys()
            .copied()
            .filter(|&k| hdr.frame_index.wrapping_sub(k) as i16 > 4)
            .collect();
        for k in stale_keys {
            self.frames.remove(&k);
            corrupt_ranges.push((k, k));
        }

        let entry = self
            .frames
            .entry(hdr.frame_index)
            .or_insert_with(|| PendingVideoFrame {
                frame_index: hdr.frame_index,
                adaptive_stream_index: hdr.adaptive_stream_index,
                units_total: total,
                units_source: source,
                units_fec: fec,
                units: vec![None; total],
                received_total: 0,
                received_source: 0,
            });

        if entry.units_total != total || idx >= entry.units.len() {
            return (None, corrupt_ranges);
        }

        if entry.units[idx].is_none() {
            entry.units[idx] = Some(unit_payload);
            entry.received_total += 1;
            if idx < entry.units_source {
                entry.received_source += 1;
            }
        }

        let ready = entry.received_source == entry.units_source
            || (entry.units_fec > 0 && entry.received_total >= entry.units_source);
        if !ready {
            return (None, corrupt_ranges);
        }

        let pending = self.frames.remove(&hdr.frame_index).unwrap();
        let assembled = if pending.received_source == pending.units_source {
            let mut out = Vec::new();
            for unit in pending.units.into_iter().take(pending.units_source) {
                if let Some(u) = unit {
                    out.extend_from_slice(&u);
                }
            }
            Some(out)
        } else {
            recover_video_frame_fec(&pending)
        };

        if let Some(bytes) = assembled {
            if let Some(prev) = self.last_emitted_frame {
                let diff = pending.frame_index.wrapping_sub(prev);
                if diff > 1 && (diff as i16) > 1 {
                    let start = prev.wrapping_add(1);
                    let end = pending.frame_index.wrapping_sub(1);
                    if !self.is_completed(start) {
                        corrupt_ranges.push((start, end));
                    }
                }
            }
            self.last_emitted_frame = Some(pending.frame_index);
            self.mark_completed(pending.frame_index);
            return (
                Some((
                    pending.frame_index,
                    pending.adaptive_stream_index,
                    bytes,
                )),
                corrupt_ranges,
            );
        } else {
            corrupt_ranges.push((pending.frame_index, pending.frame_index));
        }

        (None, corrupt_ranges)
    }
}

fn recover_video_frame_fec(pending: &PendingVideoFrame) -> Option<Vec<u8>> {
    let unit_stride = pending
        .units
        .iter()
        .flatten()
        .map(|u| u.len())
        .max()
        .unwrap_or(0);
    if unit_stride == 0 {
        return None;
    }

    let mut flat = vec![0u8; pending.units_total * unit_stride];
    let mut erasures = Vec::new();
    for (i, slot) in pending.units.iter().enumerate() {
        match slot {
            Some(u) => {
                let dst = &mut flat[i * unit_stride..i * unit_stride + u.len()];
                dst.copy_from_slice(u);
            }
            None => erasures.push(i),
        }
    }

    if fec_decode(
        &mut flat,
        unit_stride,
        unit_stride,
        pending.units_source,
        pending.units_fec,
        &erasures,
    )
    .is_err()
    {
        return None;
    }

    Some(flat[..pending.units_source * unit_stride].to_vec())
}

#[derive(Debug, Default)]
pub struct AudioReceiver {
    seen_seqs: VecDeque<u16>,
    recent_units: HashMap<u16, Vec<u8>>,
}

impl AudioReceiver {
    fn mark_seen(&mut self, seq: u16) -> bool {
        if self.seen_seqs.contains(&seq) {
            return false;
        }
        if self.seen_seqs.len() >= 128 {
            if let Some(old) = self.seen_seqs.pop_front() {
                self.recent_units.remove(&old);
            }
        }
        self.seen_seqs.push_back(seq);
        true
    }

    /// Extract ordered Opus frames from a decrypted v12 non-haptics audio packet.
    /// Implements `audioreceiver.c:86-136` multi-unit extraction and single-parity recovery.
    pub fn process_packet(&mut self, hdr: &AvPacketHeader, payload: &[u8]) -> Vec<Vec<u8>> {
        if hdr.codec != AUDIO_CODEC_OPUS || hdr.is_haptics {
            return Vec::new();
        }
        let source_units = (hdr.units_fec & 0x0f) as usize;
        let fec_units = ((hdr.units_fec >> 4) & 0x0f) as usize;
        let unit_size = ((hdr.units_fec >> 8) & 0xff) as usize;
        let total = hdr.units_total as usize;

        if unit_size == 0
            || source_units == 0
            || source_units + fec_units != total
            || payload.len() < unit_size * total
        {
            return Vec::new();
        }

        let mut emitted = Vec::with_capacity(source_units);

        if fec_units == 1 {
            let parity = &payload[source_units * unit_size..(source_units + 1) * unit_size];
            let win_start = hdr.frame_index.wrapping_sub(source_units as u16);
            let mut missing_seq: Option<u16> = None;
            let mut present_cnt = 0usize;
            for k in 0..source_units {
                let s = win_start.wrapping_add(k as u16);
                if self.recent_units.contains_key(&s) {
                    present_cnt += 1;
                } else {
                    missing_seq = Some(s);
                }
            }
            if present_cnt + 1 == source_units {
                if let Some(miss) = missing_seq {
                    let mut recovered = parity.to_vec();
                    for k in 0..source_units {
                        let s = win_start.wrapping_add(k as u16);
                        if s != miss {
                            if let Some(u) = self.recent_units.get(&s) {
                                for (r, &b) in recovered.iter_mut().zip(u.iter()) {
                                    *r ^= b;
                                }
                            }
                        }
                    }
                    if self.mark_seen(miss) {
                        self.recent_units.insert(miss, recovered.clone());
                        emitted.push(recovered);
                    }
                }
            }
        }

        for i in 0..source_units {
            let seq = hdr.frame_index.wrapping_add(i as u16);
            let slice = &payload[i * unit_size..(i + 1) * unit_size];
            self.recent_units
                .entry(seq)
                .or_insert_with(|| slice.to_vec());
            if self.mark_seen(seq) {
                emitted.push(slice.to_vec());
            }
        }

        emitted
    }
}

fn encode_control_packet(
    tag: u32,
    chunk_type: u8,
    chunk_flags: u8,
    chunk_payload: &[u8],
    tx_crypt: Option<&mut TakionTxCrypt>,
) -> Vec<u8> {
    let chunk_len = 4 + chunk_payload.len();
    let mut pkt = Vec::with_capacity(13 + chunk_len);
    pkt.push(PKT_CONTROL);
    pkt.extend_from_slice(&tag.to_be_bytes());
    pkt.extend_from_slice(&[0u8; 4]);
    pkt.extend_from_slice(&[0u8; 4]);
    pkt.push(chunk_type);
    pkt.push(chunk_flags);
    pkt.extend_from_slice(&(chunk_len as u16).to_be_bytes());
    pkt.extend_from_slice(chunk_payload);

    // CONTROL payloads remain plaintext on PS5; both `mac` (5..9) and `key_pos` (9..13)
    // stay zeroed during GMAC calculation (`takion.c:555-565`).
    if let Some(tc) = tx_crypt {
        let mut advance_len = pkt.len();
        advance_len += advance_len % 16;
        let key_pos = tc.advance(advance_len);
        let gmac = tc.crypt.gmac(key_pos, &pkt);
        pkt[5..9].copy_from_slice(&gmac);
        pkt[9..13].copy_from_slice(&(key_pos as u32).to_be_bytes());
    }

    pkt
}

#[derive(Debug)]
struct ParsedChunk {
    chunk_type: u8,
    chunk_flags: u8,
    payload: Vec<u8>,
}

fn decode_control_chunks(
    buf: &mut [u8],
    rx_crypt: Option<&mut TakionRxCrypt>,
    expected_tag: Option<u32>,
) -> Result<(u32, Vec<ParsedChunk>), TakionError> {
    if buf.len() < 17 || buf[0] != PKT_CONTROL {
        return Err(TakionError::Malformed("CONTROL packet shorter than 17B".into()));
    }
    let tag = u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]);
    if let Some(exp) = expected_tag {
        if tag != exp {
            return Err(TakionError::Malformed(format!(
                "CONTROL tag mismatch: expected {exp:#x}, got {tag:#x}"
            )));
        }
    }

    if let Some(rc) = rx_crypt {
        let mut rx_mac = [0u8; 4];
        rx_mac.copy_from_slice(&buf[5..9]);
        let key_pos_low = u32::from_be_bytes([buf[9], buf[10], buf[11], buf[12]]);
        buf[5..13].fill(0);
        let full_pos = rc.key_state.request_pos(key_pos_low, false);
        let expected_mac = rc.crypt.gmac(full_pos, buf);
        if rx_mac != expected_mac {
            return Err(TakionError::Crypto("CONTROL GMAC verification failed".into()));
        }
        rc.key_state.commit(full_pos);
    }

    let chunk_type = buf[13];
    let chunk_flags = buf[14];
    let chunk_len = u16::from_be_bytes([buf[15], buf[16]]) as usize;
    if chunk_len < 4 || 13 + chunk_len != buf.len() {
        return Err(TakionError::Malformed(format!(
            "CONTROL chunk length mismatch: chunk_len={chunk_len}, pkt_len={}",
            buf.len()
        )));
    }
    let chunks = vec![ParsedChunk {
        chunk_type,
        chunk_flags,
        payload: buf[17..].to_vec(),
    }];
    Ok((tag, chunks))
}

fn parse_cookie_from_init_ack(payload: &[u8]) -> Result<(u32, u32, Vec<u8>), TakionError> {
    if payload.len() < 48 {
        return Err(TakionError::Malformed("INIT_ACK chunk shorter than 48B".into()));
    }
    let remote_tag = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let remote_tsn = u32::from_be_bytes([payload[12], payload[13], payload[14], payload[15]]);
    Ok((remote_tag, remote_tsn, payload[16..].to_vec()))
}

fn parse_audio_header(raw: &[u8]) -> Option<TakionAudioConfig> {
    if raw.len() < 14 {
        return None;
    }
    let channels = raw[0];
    let bits = raw[1];
    let sample_rate = u32::from_be_bytes([raw[2], raw[3], raw[4], raw[5]]);
    let frame_size = u32::from_be_bytes([raw[6], raw[7], raw[8], raw[9]]);
    Some(TakionAudioConfig {
        channels,
        bits,
        sample_rate,
        frame_size,
    })
}

fn encode_data_chunk(seq: u32, channel: u16, data_type: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(9 + body.len());
    out.extend_from_slice(&seq.to_be_bytes());
    out.extend_from_slice(&channel.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.push(data_type);
    out.extend_from_slice(body);
    out
}

fn encode_data_continuation_chunk(seq: u32, channel: u16, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&seq.to_be_bytes());
    out.extend_from_slice(&channel.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(body);
    out
}

fn encode_data_ack_chunk(cum_seq: u32) -> [u8; 12] {
    let mut out = [0u8; 12];
    out[0..4].copy_from_slice(&cum_seq.to_be_bytes());
    out[4..8].copy_from_slice(&A_RWND_DEFAULT.to_be_bytes());
    out
}

async fn send_protobuf_data(
    sock: &UdpSocket,
    remote_tag: u32,
    local_seq: &mut u32,
    mtu: u32,
    channel: u16,
    payload_bytes: &[u8],
    mut tx_crypt: Option<&mut TakionTxCrypt>,
) -> Result<(), TakionError> {
    let max_chunk = (mtu as usize).saturating_sub(76).max(256);
    if payload_bytes.len() <= max_chunk {
        let chunk = encode_data_chunk(*local_seq, channel, DATA_TYPE_PROTOBUF, payload_bytes);
        *local_seq = local_seq.wrapping_add(1);
        let pkt = encode_control_packet(
            remote_tag,
            CHUNK_DATA,
            DATA_FLAG_FIN,
            &chunk,
            tx_crypt.as_deref_mut(),
        );
        sock.send(&pkt).await?;
        return Ok(());
    }
    let total = payload_bytes.len();
    let mut off = 0usize;
    let mut first = true;
    while off < total {
        let take = (total - off).min(max_chunk);
        let slice = &payload_bytes[off..off + take];
        let is_last = off + take == total;
        let flags = if is_last {
            DATA_FLAG_FIN
        } else {
            DATA_FLAG_FRAG
        };
        let chunk = if first {
            first = false;
            encode_data_chunk(*local_seq, channel, DATA_TYPE_PROTOBUF, slice)
        } else {
            encode_data_continuation_chunk(*local_seq, channel, slice)
        };
        *local_seq = local_seq.wrapping_add(1);
        let pkt = encode_control_packet(
            remote_tag,
            CHUNK_DATA,
            flags,
            &chunk,
            tx_crypt.as_deref_mut(),
        );
        sock.send(&pkt).await?;
        off += take;
    }
    Ok(())
}

const MAX_REASSEMBLED_DATA_BYTES: usize = 1024 * 1024;
const MAX_OUT_OF_ORDER_DATA_CHUNKS: usize = 64;

#[derive(Default)]
struct DataReassembler {
    buffers: HashMap<u16, (u8, Vec<u8>)>,
    out_of_order: HashMap<u32, (u8, Vec<u8>)>,
    expected_seq: Option<u32>,
    last_cum_seq: Option<u32>,
}

impl DataReassembler {
    fn with_initial_seq(initial_seq: u32) -> Self {
        Self {
            buffers: HashMap::new(),
            out_of_order: HashMap::new(),
            expected_seq: Some(initial_seq),
            last_cum_seq: None,
        }
    }

    fn push(&mut self, flags: u8, payload: &[u8]) -> Vec<(u8, Vec<u8>)> {
        if payload.len() < 8 {
            return Vec::new();
        }
        let seq = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
        let expected = *self.expected_seq.get_or_insert(seq);
        let diff = seq.wrapping_sub(expected);

        if diff >= 0x8000_0000 {
            // Duplicate / already-acknowledged TSN (`seq < expected` in wrapping order).
            return Vec::new();
        }
        if diff > 0 {
            if (diff as usize) <= MAX_OUT_OF_ORDER_DATA_CHUNKS
                && self.out_of_order.len() < MAX_OUT_OF_ORDER_DATA_CHUNKS
                && payload.len() <= 65_536
            {
                self.out_of_order
                    .entry(seq)
                    .or_insert_with(|| (flags, payload.to_vec()));
            }
            return Vec::new();
        }

        let mut completed = Vec::new();
        if let Some(msg) = self.ingest_in_order(flags, payload) {
            completed.push(msg);
        }
        let mut cur = seq;
        let mut next = cur.wrapping_add(1);
        while let Some((buffered_flags, buffered_payload)) = self.out_of_order.remove(&next) {
            if let Some(msg) = self.ingest_in_order(buffered_flags, &buffered_payload) {
                completed.push(msg);
            }
            cur = next;
            next = cur.wrapping_add(1);
        }
        self.last_cum_seq = Some(cur);
        self.expected_seq = Some(next);
        completed
    }

    fn ingest_in_order(&mut self, flags: u8, payload: &[u8]) -> Option<(u8, Vec<u8>)> {
        if payload.len() < 8 {
            return None;
        }
        let channel = u16::from_be_bytes([payload[4], payload[5]]);
        let is_end = (flags & DATA_FLAG_FIN) != 0;

        if let Some((dt, mut buf)) = self.buffers.remove(&channel) {
            let chunk_body = &payload[8..];
            if buf.len().saturating_add(chunk_body.len()) > MAX_REASSEMBLED_DATA_BYTES {
                return None;
            }
            buf.extend_from_slice(chunk_body);
            if is_end {
                return Some((dt, buf));
            }
            self.buffers.insert(channel, (dt, buf));
            return None;
        }

        if payload.len() < 9 {
            return None;
        }
        let data_type = payload[8];
        let body = &payload[9..];
        if body.len() > MAX_REASSEMBLED_DATA_BYTES {
            return None;
        }
        if is_end {
            Some((data_type, body.to_vec()))
        } else {
            self.buffers.insert(channel, (data_type, body.to_vec()));
            None
        }
    }
}

/// Connect UDP 9296, run the Takion v12 handshake + ECDH key exchange,
/// wait for `STREAM_INFO`, and spawn the background send/recv loop.
pub async fn connect_takion(config: TakionConfig) -> Result<TakionSession, TakionError> {
    let bind_addr: SocketAddr = match config.host {
        IpAddr::V4(_) => "0.0.0.0:0".parse().unwrap(),
        IpAddr::V6(_) => "[::]:0".parse().unwrap(),
    };
    let sock = UdpSocket::bind(bind_addr).await?;
    sock.connect(SocketAddr::new(config.host, config.port)).await?;

    let local_tag: u32 = rand::rng().random::<u32>().max(1);
    let mut local_seq: u32 = local_tag;
    let mtu = config.senkusha.mtu_out;

    // 1. INIT -> INIT_ACK (16B fixed payload, initial_seq_num == local_tag)
    let mut init_payload = Vec::with_capacity(16);
    init_payload.extend_from_slice(&local_tag.to_be_bytes());
    init_payload.extend_from_slice(&A_RWND_DEFAULT.to_be_bytes());
    init_payload.extend_from_slice(&100u16.to_be_bytes());
    init_payload.extend_from_slice(&100u16.to_be_bytes());
    init_payload.extend_from_slice(&local_tag.to_be_bytes());

    let init_pkt = encode_control_packet(0, CHUNK_INIT, 0, &init_payload, None);
    let mut recv_buf = vec![0u8; 65_536];

    let (remote_tag, remote_tsn, cookie) = {
        let mut attempts = 0;
        loop {
            attempts += 1;
            sock.send(&init_pkt).await?;
            match timeout(Duration::from_millis(750), sock.recv(&mut recv_buf)).await {
                Ok(Ok(n)) if buf_has_chunk(&mut recv_buf[..n], CHUNK_INIT_ACK, Some(local_tag)) => {
                    let (_tag, chunks) =
                        decode_control_chunks(&mut recv_buf[..n], None, Some(local_tag))?;
                    let ack = chunks
                        .into_iter()
                        .find(|c| c.chunk_type == CHUNK_INIT_ACK)
                        .ok_or_else(|| TakionError::Malformed("missing INIT_ACK chunk".into()))?;
                    break parse_cookie_from_init_ack(&ack.payload)?;
                }
                _ if attempts < 6 => continue,
                _ => return Err(TakionError::Timeout("waiting for INIT_ACK")),
            }
        }
    };

    // 2. COOKIE -> COOKIE_ACK
    let cookie_pkt = encode_control_packet(remote_tag, CHUNK_COOKIE, 0, &cookie, None);
    {
        let mut attempts = 0;
        loop {
            attempts += 1;
            sock.send(&cookie_pkt).await?;
            match timeout(Duration::from_millis(750), sock.recv(&mut recv_buf)).await {
                Ok(Ok(n)) if buf_has_chunk(&mut recv_buf[..n], CHUNK_COOKIE_ACK, Some(local_tag)) => {
                    break
                }
                _ if attempts < 6 => continue,
                _ => return Err(TakionError::Timeout("waiting for COOKIE_ACK")),
            }
        }
    }

    // 3. ECDH + LaunchSpec + client BIG protobuf (channel 1, plaintext before BANG)
    let ecdh = Ecdh::generate();
    let (local_pub, local_sig) = ecdh.local_pub_key_and_sig(&config.launch_spec.handshake_key);
    let launch_spec_enc = encode_launchspec(&config.launch_spec, &config.rpcrypt);

    let mut client_big = proto::TakionMessage::with_type(PayloadType::Big);
    client_big.big_payload = Some(proto::BigPayload {
        client_version: 12,
        session_key: config.session_id.clone(),
        launch_spec: launch_spec_enc,
        encrypted_key: vec![0u8; 4],
        ecdh_pub_key: Some(local_pub.to_vec()),
        ecdh_sig: Some(local_sig.to_vec()),
    });

    let big_bytes = client_big.encode_to_vec();
    send_protobuf_data(
        &sock,
        remote_tag,
        &mut local_seq,
        mtu,
        1,
        &big_bytes,
        None,
    )
    .await?;

    // 4. Wait for server BANG (ECDH key exchange) + STREAM_INFO
    let mut reassembler = DataReassembler::with_initial_seq(remote_tsn);
    let mut tx_crypt: Option<TakionTxCrypt> = None;
    let mut rx_crypt: Option<TakionRxCrypt> = None;
    let mut audio_config = TakionAudioConfig::default();
    let mut video_headers: Vec<Vec<u8>> = Vec::new();
    let mut got_stream_info = false;

    let deadline = Instant::now() + Duration::from_secs(12);
    while tx_crypt.is_none() || !got_stream_info {
        let rem = deadline
            .checked_duration_since(Instant::now())
            .ok_or(TakionError::Timeout("waiting for server BANG / STREAM_INFO"))?;
        let n = timeout(rem, sock.recv(&mut recv_buf))
            .await
            .map_err(|_| TakionError::Timeout("waiting for server BANG / STREAM_INFO"))??;
        if n == 0 || recv_buf[0] != PKT_CONTROL {
            continue;
        }

        let (_tag, chunks) =
            decode_control_chunks(&mut recv_buf[..n], rx_crypt.as_mut(), Some(local_tag))?;
        for chunk in chunks {
            match chunk.chunk_type {
                CHUNK_HEARTBEAT => {
                    let ack = encode_control_packet(
                        remote_tag,
                        CHUNK_HEARTBEAT_ACK,
                        0,
                        &chunk.payload,
                        tx_crypt.as_mut(),
                    );
                    let _ = sock.send(&ack).await;
                }
                CHUNK_DATA => {
                    let completed = reassembler.push(chunk.chunk_flags, &chunk.payload);
                    if let Some(cum) = reassembler.last_cum_seq {
                        let ack_body = encode_data_ack_chunk(cum);
                        let ack_pkt = encode_control_packet(
                            remote_tag,
                            CHUNK_DATA_ACK,
                            0,
                            &ack_body,
                            tx_crypt.as_mut(),
                        );
                        let _ = sock.send(&ack_pkt).await;
                    }
                    for (data_type, body) in completed {
                        if data_type == DATA_TYPE_PROTOBUF {
                            let msg = proto::TakionMessage::decode(body.as_slice())?;
                            if let Some(bang) = msg.bang_payload {
                                let remote_pub = bang.ecdh_pub_key.ok_or_else(|| {
                                    TakionError::Crypto("BANG missing server ecdh_pub_key".into())
                                })?;
                                let secret = ecdh
                                    .derive_secret(
                                        &remote_pub,
                                        &config.launch_spec.handshake_key,
                                        bang.ecdh_sig.as_deref(),
                                    )
                                    .map_err(|e| TakionError::Crypto(e.to_string()))?;
                                // Client TX (`gkcrypt_local`) MUST use index 2; client RX (`gkcrypt_remote`) MUST use index 3.
                                tx_crypt = Some(TakionTxCrypt::new(GkCrypt::new(
                                    2,
                                    &config.launch_spec.handshake_key,
                                    &secret,
                                )));
                                rx_crypt = Some(TakionRxCrypt::new(GkCrypt::new(
                                    3,
                                    &config.launch_spec.handshake_key,
                                    &secret,
                                )));
                            }
                            if let Some(info) = msg.stream_info_payload {
                                if let Some(parsed) = parse_audio_header(&info.audio_header) {
                                    audio_config = parsed;
                                }
                                video_headers = info
                                    .resolution
                                    .iter()
                                    .map(|r| r.video_header.clone())
                                    .collect();
                                got_stream_info = true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Acknowledge STREAM_INFO (channel 9), announce DualSense (channel 1), and advertise mic STREAMINFO (channel 1).
    if let Some(ref mut tc) = tx_crypt {
        let info_ack = proto::TakionMessage::with_type(PayloadType::Streaminfoack);
        let _ = send_protobuf_data(
            &sock,
            remote_tag,
            &mut local_seq,
            mtu,
            9,
            &info_ack.encode_to_vec(),
            Some(tc),
        )
        .await;

        let mut ctrl_conn = proto::TakionMessage::with_type(PayloadType::Controllerconnection);
        ctrl_conn.controller_connection_payload = Some(proto::ControllerConnectionPayload {
            controller_id: None,
            connected: Some(true),
            controller_type: Some(proto::ControllerType::Dualsense as i32),
        });
        let _ = send_protobuf_data(
            &sock,
            remote_tag,
            &mut local_seq,
            mtu,
            1,
            &ctrl_conn.encode_to_vec(),
            Some(tc),
        )
        .await;

        let mut mic_info = proto::TakionMessage::with_type(PayloadType::Streaminfo);
        mic_info.stream_info_payload = Some(proto::StreamInfoPayload {
            resolution: Vec::new(),
            audio_header: vec![1, 16, 0, 0, 0xbb, 0x80, 0, 0, 0x01, 0xe0, 0, 0, 0, 1],
            start_timeout: None,
            afk_timeout: None,
            afk_timeout_disconnect: None,
            congestion_control_interval: None,
            audio_channel: Vec::new(),
        });
        let _ = send_protobuf_data(
            &sock,
            remote_tag,
            &mut local_seq,
            mtu,
            1,
            &mic_info.encode_to_vec(),
            Some(tc),
        )
        .await;
    }

    let (event_tx, event_rx) = mpsc::channel(256);
    let (cmd_tx, cmd_rx) = mpsc::channel(64);
    let (closed_tx, closed_rx) = watch::channel(None);

    let _ = event_tx.send(TakionEvent::AudioConfig(audio_config)).await;
    if !video_headers.is_empty() {
        let _ = event_tx
            .send(TakionEvent::VideoHeaders(video_headers.clone()))
            .await;
    }

    tokio::spawn(run_takion_loop(
        sock,
        local_tag,
        remote_tag,
        local_seq,
        mtu,
        tx_crypt.expect("initialized above"),
        rx_crypt.expect("initialized above"),
        reassembler,
        cmd_rx,
        event_tx,
        closed_tx,
    ));

    Ok(TakionSession {
        handle: TakionHandle { cmd_tx, closed_rx },
        events: event_rx,
        audio_config,
        video_headers,
    })
}

fn buf_has_chunk(buf: &mut [u8], expected_type: u8, expected_tag: Option<u32>) -> bool {
    if buf.len() < 17 || buf[0] != PKT_CONTROL {
        return false;
    }
    decode_control_chunks(buf, None, expected_tag)
        .map(|(_, chunks)| chunks.iter().any(|c| c.chunk_type == expected_type))
        .unwrap_or(false)
}

#[allow(clippy::too_many_arguments)]
async fn run_takion_loop(
    sock: UdpSocket,
    local_tag: u32,
    remote_tag: u32,
    mut local_seq: u32,
    mtu: u32,
    mut tx_crypt: TakionTxCrypt,
    mut rx_crypt: TakionRxCrypt,
    mut reassembler: DataReassembler,
    mut cmd_rx: mpsc::Receiver<TakionCommand>,
    event_tx: mpsc::Sender<TakionEvent>,
    closed_tx: watch::Sender<Option<String>>,
) {
    let mut recv_buf = vec![0u8; 65_536];
    let mut video_proc = VideoFrameProcessor::default();
    let mut audio_rx = AudioReceiver::default();
    let mut history_buf = FeedbackHistoryBuffer::default();
    let mut last_ctrl_state = Ps5ControllerState::default();
    let mut feedback_seq: u16 = 0;

    let mut pkt_received_window: u16 = 0;
    let mut pkt_lost_window: u16 = 0;
    let mut last_av_pkt_index: Option<u16> = None;

    let mut feedback_tick = interval(Duration::from_millis(16));
    feedback_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut congestion_tick = interval(Duration::from_millis(200));
    congestion_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut proto_heartbeat_tick = interval_at(
        tokio::time::Instant::now() + Duration::from_secs(5),
        Duration::from_secs(5),
    );
    proto_heartbeat_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let reason = loop {
        tokio::select! {
            recv_res = sock.recv(&mut recv_buf) => {
                match recv_res {
                    Ok(n) if n > 0 => {
                        let pkt_type = recv_buf[0] & 0x0f;
                        match pkt_type {
                            PKT_CONTROL => {
                                if let Ok((_tag, chunks)) = decode_control_chunks(&mut recv_buf[..n], Some(&mut rx_crypt), Some(local_tag)) {
                                    for chunk in chunks {
                                        match chunk.chunk_type {
                                            CHUNK_HEARTBEAT => {
                                                let ack = encode_control_packet(remote_tag, CHUNK_HEARTBEAT_ACK, 0, &chunk.payload, Some(&mut tx_crypt));
                                                let _ = sock.send(&ack).await;
                                            }
                                            CHUNK_DATA => {
                                                let completed = reassembler.push(chunk.chunk_flags, &chunk.payload);
                                                if let Some(cum) = reassembler.last_cum_seq {
                                                    let ack = encode_control_packet(remote_tag, CHUNK_DATA_ACK, 0, &encode_data_ack_chunk(cum), Some(&mut tx_crypt));
                                                    let _ = sock.send(&ack).await;
                                                }
                                                for (dt, body) in completed {
                                                    handle_takion_data(dt, &body, &event_tx).await;
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                            PKT_VIDEO | PKT_AUDIO => {
                                if let Ok(hdr) = parse_av_header_v12(&recv_buf[..n]) {
                                    let pkt = &mut recv_buf[..n];
                                    let mut rx_mac = [0u8; 4];
                                    rx_mac.copy_from_slice(&pkt[10..14]);
                                    // Zero MAC (10..14) while preserving key_pos (14..18).
                                    pkt[10..14].fill(0);
                                    let (head_slice, payload_slice) = pkt.split_at_mut(hdr.payload_offset);
                                    // `gmac` covers header + encrypted payload before decryption.
                                    let mut full_for_mac = Vec::with_capacity(head_slice.len() + payload_slice.len());
                                    full_for_mac.extend_from_slice(head_slice);
                                    full_for_mac.extend_from_slice(payload_slice);
                                    if rx_crypt.verify_and_decrypt(hdr.key_pos_low, &rx_mac, &full_for_mac, payload_slice) {
                                        pkt_received_window = pkt_received_window.saturating_add(1);
                                        if let Some(prev) = last_av_pkt_index {
                                            let gap = hdr.packet_index.wrapping_sub(prev);
                                            if gap > 1 && gap < 512 {
                                                pkt_lost_window = pkt_lost_window.saturating_add(gap - 1);
                                            }
                                        }
                                        last_av_pkt_index = Some(hdr.packet_index);

                                        match hdr.kind {
                                            AvPacketKind::Video => {
                                                let (done, corrupt_ranges) = video_proc.push_unit(&hdr, payload_slice.to_vec());
                                                if let Some((frame_index, adaptive_stream_index, data)) = done {
                                                    let _ = event_tx.try_send(TakionEvent::VideoFrame {
                                                        frame_index,
                                                        adaptive_stream_index,
                                                        data,
                                                    });
                                                }
                                                for (start, end) in corrupt_ranges {
                                                    send_corrupt_frame(&sock, remote_tag, &mut local_seq, mtu, &mut tx_crypt, start, end).await;
                                                }
                                            }
                                            AvPacketKind::Audio => {
                                                if hdr.is_haptics {
                                                    let _ = event_tx.try_send(TakionEvent::AudioHaptics(payload_slice.to_vec()));
                                                } else {
                                                    for unit in audio_rx.process_packet(&hdr, payload_slice) {
                                                        let _ = event_tx.try_send(TakionEvent::AudioFrame(unit));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    Ok(_) => {}
                    Err(e) => break format!("UDP recv error: {e}"),
                }
            }
            cmd_opt = cmd_rx.recv() => {
                match cmd_opt {
                    Some(TakionCommand::ControllerState(now)) => {
                        let changed = history_buf.record_state(&last_ctrl_state, &now);
                        last_ctrl_state = now;
                        if changed && !history_buf.is_empty() {
                            let (cnt, payload) = history_buf.format();
                            let count_minus_1 = cnt.saturating_sub(1) as u8;
                            let pkt = build_feedback_packet_raw(
                                PKT_FEEDBACK_HISTORY,
                                feedback_seq,
                                count_minus_1,
                                &payload,
                                &mut tx_crypt,
                            );
                            feedback_seq = feedback_seq.wrapping_add(1);
                            let _ = sock.send(&pkt).await;
                        }
                    }
                    Some(TakionCommand::RequestIdr) => {
                        send_idr_request(&sock, remote_tag, &mut local_seq, mtu, &mut tx_crypt).await;
                    }
                    Some(TakionCommand::Shutdown) => {
                        send_disconnect(&sock, remote_tag, &mut local_seq, mtu, &mut tx_crypt).await;
                        break "shutdown requested".into();
                    }
                    None => break "shutdown requested".into(),
                }
            }
            _ = feedback_tick.tick() => {
                let state_payload = format_feedback_state_v12(&last_ctrl_state);
                let pkt = build_feedback_packet_raw(
                    PKT_FEEDBACK_STATE,
                    feedback_seq,
                    0,
                    &state_payload,
                    &mut tx_crypt,
                );
                feedback_seq = feedback_seq.wrapping_add(1);
                let _ = sock.send(&pkt).await;
            }
            _ = congestion_tick.tick() => {
                let key_pos = tx_crypt.current_pos();
                let pkt = build_congestion_packet(0, pkt_received_window, pkt_lost_window, &mut tx_crypt.crypt, key_pos);
                pkt_received_window = 0;
                pkt_lost_window = 0;
                let _ = sock.send(&pkt).await;
            }
            _ = proto_heartbeat_tick.tick() => {
                let hb = proto::TakionMessage::with_type(PayloadType::Heartbeat);
                let _ = send_protobuf_data(&sock, remote_tag, &mut local_seq, mtu, 1, &hb.encode_to_vec(), Some(&mut tx_crypt)).await;
            }
        }
    };

    let _ = event_tx.send(TakionEvent::Disconnected(reason.clone())).await;
    let _ = closed_tx.send(Some(reason));
}

async fn handle_takion_data(
    data_type: u8,
    body: &[u8],
    event_tx: &mpsc::Sender<TakionEvent>,
) {
    match data_type {
        DATA_TYPE_PROTOBUF => {
            if let Ok(msg) = proto::TakionMessage::decode(body) {
                if let Some(info) = msg.stream_info_payload {
                    if let Some(cfg) = parse_audio_header(&info.audio_header) {
                        let _ = event_tx.send(TakionEvent::AudioConfig(cfg)).await;
                    }
                    let headers: Vec<Vec<u8>> = info
                        .resolution
                        .iter()
                        .map(|r| r.video_header.clone())
                        .collect();
                    if !headers.is_empty() {
                        let _ = event_tx.send(TakionEvent::VideoHeaders(headers)).await;
                    }
                }
            }
        }
        DATA_TYPE_RUMBLE => {
            if body.len() >= 3 {
                let _ = event_tx.try_send(TakionEvent::Rumble(RumbleEvent {
                    unknown: body[0],
                    left: body[1],
                    right: body[2],
                }));
            }
        }
        DATA_TYPE_TRIGGER_EFFECTS => {
            if body.len() >= 25 {
                let mut left = [0u8; 10];
                let mut right = [0u8; 10];
                left.copy_from_slice(&body[5..15]);
                right.copy_from_slice(&body[15..25]);
                let _ = event_tx.try_send(TakionEvent::TriggerEffects(TriggerEffectsEvent {
                    left,
                    right,
                }));
            }
        }
        DATA_TYPE_PAD_INFO | _ => {}
    }
}

async fn send_corrupt_frame(
    sock: &UdpSocket,
    remote_tag: u32,
    local_seq: &mut u32,
    mtu: u32,
    tx_crypt: &mut TakionTxCrypt,
    start: u16,
    end: u16,
) {
    let mut msg = proto::TakionMessage::with_type(PayloadType::Corruptframe);
    msg.corrupt_payload = Some(proto::CorruptFramePayload {
        start: start as u32,
        end: end as u32,
    });
    let _ = send_protobuf_data(
        sock,
        remote_tag,
        local_seq,
        mtu,
        2,
        &msg.encode_to_vec(),
        Some(tx_crypt),
    )
    .await;
}

async fn send_idr_request(
    sock: &UdpSocket,
    remote_tag: u32,
    local_seq: &mut u32,
    mtu: u32,
    tx_crypt: &mut TakionTxCrypt,
) {
    let msg = proto::TakionMessage::with_type(PayloadType::Idrrequest);
    let _ = send_protobuf_data(
        sock,
        remote_tag,
        local_seq,
        mtu,
        2,
        &msg.encode_to_vec(),
        Some(tx_crypt),
    )
    .await;
}

async fn send_disconnect(
    sock: &UdpSocket,
    remote_tag: u32,
    local_seq: &mut u32,
    mtu: u32,
    tx_crypt: &mut TakionTxCrypt,
) {
    let mut msg = proto::TakionMessage::with_type(PayloadType::Disconnect);
    msg.disconnect_payload = Some(proto::DisconnectPayload {
        reason: "Client Disconnecting".into(),
        extended_info: None,
    });
    let _ = send_protobuf_data(
        sock,
        remote_tag,
        local_seq,
        mtu,
        1,
        &msg.encode_to_vec(),
        Some(tx_crypt),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_v12_video_and_audio_headers() {
        let mut vid = vec![0u8; 30];
        vid[0] = 0x02;
        vid[1..3].copy_from_slice(&12u16.to_be_bytes());
        vid[3..5].copy_from_slice(&44u16.to_be_bytes());
        let dw2: u32 = (1 << 21) | (2 << 10) | 1;
        vid[5..9].copy_from_slice(&dw2.to_be_bytes());
        vid[20] = 0b0100_0000;
        let vh = parse_av_header_v12(&vid).unwrap();
        assert_eq!(vh.kind, AvPacketKind::Video);
        assert_eq!(vh.packet_index, 12);
        assert_eq!(vh.frame_index, 44);
        assert_eq!(vh.unit_index, 1);
        assert_eq!(vh.units_total, 3);
        assert_eq!(vh.units_fec, 1);
        assert_eq!(vh.adaptive_stream_index, 2);
        assert_eq!(vh.payload_offset, 21);

        vid[0] = 0x12;
        assert_eq!(parse_av_header_v12(&vid).unwrap().payload_offset, 24);

        let mut aud = vec![0u8; 40];
        aud[0] = 0x03;
        let adw2: u32 = (0 << 24) | (1 << 16) | (10 << 8) | (0 << 4) | 2;
        aud[5..9].copy_from_slice(&adw2.to_be_bytes());
        aud[9] = AUDIO_CODEC_OPUS;
        aud[19] = 0x02;
        let ah = parse_av_header_v12(&aud).unwrap();
        assert_eq!(ah.kind, AvPacketKind::Audio);
        assert!(ah.is_haptics);
        assert_eq!(ah.payload_offset, 20);
    }

    #[test]
    fn congestion_packet_zeros_mac_and_key_pos_for_gmac() {
        let mut gk = GkCrypt::new(3, &[0x5au8; 16], &[0x7bu8; 32]);
        let pkt = build_congestion_packet(0x0102, 500, 3, &mut gk, 0x1234);
        assert_eq!(pkt[0], 0x05);
        assert_eq!(&pkt[1..3], &0x0102u16.to_be_bytes());
        assert_eq!(&pkt[3..5], &500u16.to_be_bytes());
        assert_eq!(&pkt[5..7], &3u16.to_be_bytes());
        let key_pos = u32::from_be_bytes([pkt[11], pkt[12], pkt[13], pkt[14]]);
        assert_eq!(key_pos, 0x1234);
        let mut zeroed = pkt;
        zeroed[7..15].fill(0);
        assert_eq!(&pkt[7..11], &gk.gmac(key_pos as u64, &zeroed));
    }

    #[test]
    fn reassembler_holds_cum_ack_on_gap_and_reassembles_out_of_order() {
        let mut r = DataReassembler::with_initial_seq(100);
        let c0 = encode_data_chunk(100, 1, DATA_TYPE_PROTOBUF, b"hello ");
        let c1 = encode_data_continuation_chunk(101, 1, b"beautiful ");
        let c2 = encode_data_continuation_chunk(102, 1, b"world");

        // Arrival order: 100 (frag), then 102 (fin, out of order!), then duplicate 100, then 101 (middle).
        assert!(r.push(DATA_FLAG_FRAG, &c0).is_empty());
        assert_eq!(r.last_cum_seq, Some(100));

        // 102 arrives before 101: must NOT advance cum_ack past 100 or emit corrupt message!
        assert!(r.push(DATA_FLAG_FIN, &c2).is_empty());
        assert_eq!(r.last_cum_seq, Some(100));

        // Duplicate 100 must NOT re-append "hello "
        assert!(r.push(DATA_FLAG_FRAG, &c0).is_empty());
        assert_eq!(r.last_cum_seq, Some(100));

        // Missing middle 101 arrives -> completes 101 + 102 in order and advances cum_ack to 102!
        let done = r.push(DATA_FLAG_FRAG, &c1);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].0, DATA_TYPE_PROTOBUF);
        assert_eq!(done[0].1, b"hello beautiful world");
        assert_eq!(r.last_cum_seq, Some(102));
    }

    #[test]
    fn video_processor_recovers_missing_unit_via_fec() {
        let mut proc = VideoFrameProcessor::default();
        let u0 = vec![0x00, 0x00, 0x00, 0x01, 0xaa, 0xbb];
        let u1 = vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66];
        let stride = u0.len();
        let mut flat = vec![0u8; 3 * stride];
        flat[..stride].copy_from_slice(&u0);
        flat[stride..2 * stride].copy_from_slice(&u1);
        crate::fec::fec_encode(&mut flat, stride, stride, 2, 1).unwrap();
        let parity = flat[2 * stride..3 * stride].to_vec();

        let make_hdr = |unit_index| AvPacketHeader {
            kind: AvPacketKind::Video,
            uses_nalu_info_structs: false,
            packet_index: unit_index,
            frame_index: 10,
            unit_index,
            units_total: 3,
            units_fec: 1,
            codec: 0,
            mac: [0; 4],
            key_pos_low: 0,
            adaptive_stream_index: 0,
            is_haptics: false,
            payload_offset: 21,
        };

        let (done0, _) = proc.push_unit(&make_hdr(0), u0.clone());
        assert!(done0.is_none());
        let (done1, _) = proc.push_unit(&make_hdr(2), parity);
        let (fidx, _, assembled) = done1.expect("FEC should recover missing unit 1");
        assert_eq!(fidx, 10);
        assert_eq!(assembled, [u0, u1].concat());
    }

    #[test]
    fn audio_receiver_extracts_opus_units_and_recovers_parity() {
        let unit0 = vec![1, 2, 3, 4];
        let unit1 = vec![5, 6, 7, 8];
        let unit2 = vec![9, 10, 11, 12];
        let unit3 = vec![13, 14, 15, 16];
        let fec: Vec<u8> = unit0.iter().zip(unit1.iter()).map(|(a, b)| a ^ b).collect();

        let hdr0 = AvPacketHeader {
            kind: AvPacketKind::Audio,
            uses_nalu_info_structs: false,
            packet_index: 1,
            frame_index: 100,
            unit_index: 0,
            units_total: 2,
            units_fec: ((4 << 8) | (0 << 4) | 2) as u16,
            codec: AUDIO_CODEC_OPUS,
            mac: [0; 4],
            key_pos_low: 0,
            adaptive_stream_index: 0,
            is_haptics: false,
            payload_offset: 20,
        };
        let mut rx2 = AudioReceiver::default();
        let hdr_first = AvPacketHeader {
            units_total: 1,
            units_fec: ((4 << 8) | (0 << 4) | 1) as u16,
            ..hdr0
        };
        assert_eq!(rx2.process_packet(&hdr_first, &unit0), vec![unit0]);

        let hdr_next = AvPacketHeader {
            frame_index: 102,
            units_total: 3,
            units_fec: ((4 << 8) | (1 << 4) | 2) as u16,
            ..hdr0
        };
        let p_next = [unit2.clone(), unit3.clone(), fec].concat();
        let out = rx2.process_packet(&hdr_next, &p_next);
        assert_eq!(out, vec![unit1, unit2, unit3]);
    }

    #[test]
    fn control_packet_is_unpadded_plaintext_with_zeroed_mac_and_key_pos_gmac() {
        let handshake = [0x42u8; 16];
        let secret = [0x99u8; 32];
        let mut tx = TakionTxCrypt::new(GkCrypt::new(2, &handshake, &secret));
        let mut rx = TakionRxCrypt::new(GkCrypt::new(2, &handshake, &secret));

        // 5-byte payload -> chunk_len = 9, total packet = 17 + 5 = 22 bytes (no 4-byte padding!).
        let payload = [0xaa, 0xbb, 0xcc, 0xdd, 0xee];
        let mut pkt = encode_control_packet(0x1122_3344, CHUNK_DATA, DATA_FLAG_FIN, &payload, Some(&mut tx));
        assert_eq!(pkt.len(), 22);
        // Payload stays plaintext.
        assert_eq!(&pkt[17..22], &payload);
        // Verify advance calculation: 22 + (22 % 16) = 28 for the *next* packet.
        assert_eq!(tx.current_pos(), 28);

        let (tag, chunks) = decode_control_chunks(&mut pkt, Some(&mut rx), Some(0x1122_3344)).unwrap();
        assert_eq!(tag, 0x1122_3344);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, CHUNK_DATA);
        assert_eq!(chunks[0].chunk_flags, DATA_FLAG_FIN);
        assert_eq!(chunks[0].payload, payload);
    }

    #[test]
    fn init_ack_cookie_and_reassembler_and_trigger_effects() {
        // 48-byte raw PS5 INIT_ACK payload: 16B header fields + 32B raw cookie (no TLV 0x0007 scan).
        let mut init_ack = vec![0u8; 48];
        init_ack[0..4].copy_from_slice(&0x5566_7788u32.to_be_bytes());
        init_ack[12..16].copy_from_slice(&0x99aa_bbccu32.to_be_bytes());
        for (i, b) in init_ack[16..48].iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(3);
        }
        let (rtag, rtsn, cookie) = parse_cookie_from_init_ack(&init_ack).unwrap();
        assert_eq!(rtag, 0x5566_7788);
        assert_eq!(rtsn, 0x99aa_bbcc);
        assert_eq!(cookie, &init_ack[16..48]);

        // Multi-fragment DATA reassembly (flags = 0 then flags = 1).
        let mut reasm = DataReassembler::default();
        let frag1 = encode_data_chunk(100, 1, DATA_TYPE_PROTOBUF, b"hello ");
        assert!(reasm.push(DATA_FLAG_FRAG, &frag1).is_empty());
        assert_eq!(reasm.last_cum_seq, Some(100));
        let frag2 = encode_data_continuation_chunk(101, 1, b"ps5");
        let done = reasm.push(DATA_FLAG_FIN, &frag2);
        assert_eq!(done.len(), 1);
        assert_eq!(reasm.last_cum_seq, Some(101));
        assert_eq!(done[0].0, DATA_TYPE_PROTOBUF);
        assert_eq!(done[0].1, b"hello ps5");

        // 25-byte TRIGGER_EFFECTS offsets: left [5..15], right [15..25].
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(async {
            let (tx, mut rx) = mpsc::channel(4);
            let mut body = vec![0u8; 25];
            body[5..15].copy_from_slice(&[1; 10]);
            body[15..25].copy_from_slice(&[2; 10]);
            handle_takion_data(DATA_TYPE_TRIGGER_EFFECTS, &body, &tx).await;
            match rx.recv().await.unwrap() {
                TakionEvent::TriggerEffects(te) => {
                    assert_eq!(te.left, [1; 10]);
                    assert_eq!(te.right, [2; 10]);
                }
                other => panic!("unexpected event: {other:?}"),
            }
        });
    }
}
