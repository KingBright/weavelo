mod shared_exchange;
use crate::file_transfer::{
    FileManifestAssembler, FileReceivePolicy, FileTransferError, FileTransferPolicy,
    FileTransferReader, FileTransferSpec, IncomingFileTransfer, control_from_envelope,
    control_to_envelope,
};
use crate::scheduled_sender::{ScheduledDataSendError, ScheduledDataSender};
use protocol::{ContentKind, DataEnvelope, FileTransferControl, FileTransferGroup};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::error::Error;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncReadExt;
use tokio::sync::{broadcast, mpsc};

/// Default token-bucket burst allowance for independent bulk file transfers (512 KiB).
pub const DEFAULT_FILE_TRANSFER_BURST_BYTES: usize = 512 * 1024;
/// Default fraction of `max_bytes_per_sec` retained when `yield_for_video` is active (20%).
pub const DEFAULT_VIDEO_YIELD_RATIO: f64 = 0.20;
/// Default fallback pacing cap when `max_bytes_per_sec == 0` (unlimited idle) and
/// `yield_for_video` is active during realtime video streaming (2 MiB/s).
pub const DEFAULT_VIDEO_YIELD_FALLBACK_BYTES_PER_SEC: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct FileTransferBandwidthStats {
    pub total_paced_bytes: u64,
    pub total_wait_us: u64,
    pub video_yield_events: u64,
}

#[derive(Debug)]
struct TokenBucketState {
    tokens_bytes: f64,
    last_refill: Instant,
    stats: FileTransferBandwidthStats,
}

#[derive(Debug)]
struct FileTransferBandwidthBudgetInner {
    max_bytes_per_sec: u64,
    burst_bytes: usize,
    yield_for_video: bool,
    video_yield_bytes_per_sec: Option<u64>,
    video_active: Arc<AtomicBool>,
    state: Mutex<TokenBucketState>,
}

/// Independent token-bucket rate and burst pacer for bulk file transfers.
///
/// Cloning shares the same underlying token bucket and `video_active` signal
/// so concurrent file transfers share one bandwidth lane and yield cleanly to
/// realtime video frames whenever `yield_for_video` is enabled and video is active.
#[derive(Debug, Clone)]
pub struct FileTransferBandwidthBudget {
    inner: Arc<FileTransferBandwidthBudgetInner>,
}

impl FileTransferBandwidthBudget {
    /// Create a token-bucket budget.
    ///
    /// - `max_bytes_per_sec`: steady-state cap when video is idle (`0` = unlimited).
    /// - `burst_bytes`: token-bucket burst capacity (clamped to at least 1 byte).
    /// - `yield_for_video`: when `true`, throttles down while `is_video_active()` is `true`.
    pub fn new(max_bytes_per_sec: u64, burst_bytes: usize, yield_for_video: bool) -> Self {
        Self::new_internal(
            max_bytes_per_sec,
            burst_bytes,
            yield_for_video,
            None,
            Instant::now(),
        )
    }

    /// Create a token-bucket budget with an explicit yield rate while realtime video is active.
    pub fn with_video_yield_rate(
        max_bytes_per_sec: u64,
        burst_bytes: usize,
        yield_for_video: bool,
        video_yield_bytes_per_sec: u64,
    ) -> Self {
        Self::new_internal(
            max_bytes_per_sec,
            burst_bytes,
            yield_for_video,
            Some(video_yield_bytes_per_sec.max(1)),
            Instant::now(),
        )
    }

    /// Preset for full-speed transfers when idle while yielding to a fixed rate during active video.
    pub fn unlimited_with_video_yield(video_yield_bytes_per_sec: u64) -> Self {
        Self::with_video_yield_rate(
            0,
            DEFAULT_FILE_TRANSFER_BURST_BYTES,
            true,
            video_yield_bytes_per_sec,
        )
    }

    fn new_internal(
        max_bytes_per_sec: u64,
        burst_bytes: usize,
        yield_for_video: bool,
        video_yield_bytes_per_sec: Option<u64>,
        now: Instant,
    ) -> Self {
        let burst_bytes = burst_bytes.max(1);
        Self {
            inner: Arc::new(FileTransferBandwidthBudgetInner {
                max_bytes_per_sec,
                burst_bytes,
                yield_for_video,
                video_yield_bytes_per_sec,
                video_active: Arc::new(AtomicBool::new(false)),
                state: Mutex::new(TokenBucketState {
                    tokens_bytes: burst_bytes as f64,
                    last_refill: now,
                    stats: FileTransferBandwidthStats::default(),
                }),
            }),
        }
    }

    pub fn max_bytes_per_sec(&self) -> u64 {
        self.inner.max_bytes_per_sec
    }

    pub fn burst_bytes(&self) -> usize {
        self.inner.burst_bytes
    }

    pub fn yield_for_video(&self) -> bool {
        self.inner.yield_for_video
    }

    pub fn is_video_active(&self) -> bool {
        self.inner.video_active.load(Relaxed)
    }

    pub fn set_video_active(&self, active: bool) {
        self.inner.video_active.store(active, Relaxed);
    }

    /// Returns the shared `AtomicBool` indicating whether realtime video is currently streaming.
    pub fn video_active_flag(&self) -> Arc<AtomicBool> {
        self.inner.video_active.clone()
    }

    /// Returns the current effective rate cap in bytes/sec (`0` means unlimited).
    pub fn effective_bytes_per_sec(&self) -> u64 {
        if self.inner.yield_for_video && self.is_video_active() {
            if let Some(explicit) = self.inner.video_yield_bytes_per_sec {
                return explicit.max(1);
            }
            if self.inner.max_bytes_per_sec > 0 {
                return ((self.inner.max_bytes_per_sec as f64) * DEFAULT_VIDEO_YIELD_RATIO)
                    .round()
                    .max(1.0) as u64;
            }
            return DEFAULT_VIDEO_YIELD_FALLBACK_BYTES_PER_SEC;
        }
        self.inner.max_bytes_per_sec
    }

    /// Reserve `bytes` from the token bucket at the current wall-clock instant and return
    /// the delay the caller should sleep before transmitting.
    pub fn reserve_delay(&self, bytes: usize) -> Duration {
        self.reserve_delay_at(bytes, Instant::now())
    }

    /// Deterministic token-bucket reservation at caller-supplied `now`.
    pub fn reserve_delay_at(&self, bytes: usize, now: Instant) -> Duration {
        if bytes == 0 {
            return Duration::ZERO;
        }

        let yielding = self.inner.yield_for_video && self.is_video_active();
        let effective_rate = self.effective_bytes_per_sec();
        let mut state = self
            .inner
            .state
            .lock()
            .expect("file transfer bandwidth budget lock should not be poisoned");
        state.stats.total_paced_bytes = state.stats.total_paced_bytes.saturating_add(bytes as u64);

        if effective_rate == 0 {
            state.last_refill = now;
            state.tokens_bytes = self.inner.burst_bytes as f64;
            return Duration::ZERO;
        }

        let rate_f64 = effective_rate as f64;
        // When yielding to realtime video, trim burst headroom to at most 100ms of the
        // yield rate (bounded by configured burst_bytes) so bulk file chunks yield immediately.
        let effective_burst = if yielding {
            (self.inner.burst_bytes as f64)
                .min((rate_f64 * 0.1).max(1.0))
                .max(1.0)
        } else {
            self.inner.burst_bytes.max(1) as f64
        };

        let elapsed = now.saturating_duration_since(state.last_refill);
        state.tokens_bytes = (state.tokens_bytes + elapsed.as_secs_f64() * rate_f64)
            .min(effective_burst);
        state.last_refill = now;

        state.tokens_bytes -= bytes as f64;
        if state.tokens_bytes >= 0.0 {
            return Duration::ZERO;
        }

        let deficit = -state.tokens_bytes;
        let delay_secs = deficit / rate_f64;
        let delay = if delay_secs.is_finite() && delay_secs > 0.0 {
            Duration::from_secs_f64(delay_secs)
        } else {
            Duration::ZERO
        };

        if !delay.is_zero() {
            state.stats.total_wait_us = state
                .stats
                .total_wait_us
                .saturating_add(u64::try_from(delay.as_micros()).unwrap_or(u64::MAX));
            if yielding {
                state.stats.video_yield_events = state.stats.video_yield_events.saturating_add(1);
            }
        }
        delay
    }

    /// Asynchronously wait for token-bucket budget to cover `bytes`.
    pub async fn acquire(&self, bytes: usize) {
        let delay = self.reserve_delay(bytes);
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
    }

    pub fn stats(&self) -> FileTransferBandwidthStats {
        self.inner
            .state
            .lock()
            .expect("file transfer bandwidth budget lock should not be poisoned")
            .stats
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferCheckpointError {
    ZeroChunkSize,
    OffsetExceedsTotal {
        confirmed_offset: u64,
        total_bytes: u64,
    },
    UnalignedOffset {
        confirmed_offset: u64,
        chunk_size: u32,
    },
    TooManyChunks {
        chunk_count: u64,
    },
    TransferIdMismatch {
        expected: u64,
        actual: u64,
    },
    FingerprintMismatch,
    TotalBytesMismatch {
        expected: u64,
        actual: u64,
    },
    ChunkSizeMismatch {
        expected: u32,
        actual: u32,
    },
    OffsetRegression {
        current: u64,
        proposed: u64,
    },
    NonContiguousChunk {
        expected_chunk: u32,
        actual_chunk: u32,
    },
    InvalidChunkLength {
        chunk_index: u32,
        expected: usize,
        actual: usize,
    },
}

impl fmt::Display for TransferCheckpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroChunkSize => write!(f, "checkpoint chunk_size must be greater than zero"),
            Self::OffsetExceedsTotal {
                confirmed_offset,
                total_bytes,
            } => write!(
                f,
                "checkpoint confirmed_offset ({confirmed_offset}) exceeds total_bytes ({total_bytes})"
            ),
            Self::UnalignedOffset {
                confirmed_offset,
                chunk_size,
            } => write!(
                f,
                "checkpoint confirmed_offset ({confirmed_offset}) is not aligned to chunk_size ({chunk_size})"
            ),
            Self::TooManyChunks { chunk_count } => {
                write!(f, "checkpoint chunk count exceeds u32::MAX ({chunk_count})")
            }
            Self::TransferIdMismatch { expected, actual } => {
                write!(
                    f,
                    "checkpoint transfer_id mismatch: expected {expected}, got {actual}"
                )
            }
            Self::FingerprintMismatch => write!(f, "checkpoint SHA-256 fingerprint mismatch"),
            Self::TotalBytesMismatch { expected, actual } => write!(
                f,
                "checkpoint total_bytes mismatch: expected {expected}, got {actual}"
            ),
            Self::ChunkSizeMismatch { expected, actual } => write!(
                f,
                "checkpoint chunk_size mismatch: expected {expected}, got {actual}"
            ),
            Self::OffsetRegression { current, proposed } => write!(
                f,
                "checkpoint confirmed_offset cannot regress from {current} to {proposed}"
            ),
            Self::NonContiguousChunk {
                expected_chunk,
                actual_chunk,
            } => write!(
                f,
                "checkpoint cannot advance out-of-order chunk {actual_chunk}; expected {expected_chunk}"
            ),
            Self::InvalidChunkLength {
                chunk_index,
                expected,
                actual,
            } => write!(
                f,
                "checkpoint chunk {chunk_index} length mismatch: expected {expected} bytes, got {actual}"
            ),
        }
    }
}

impl Error for TransferCheckpointError {}

/// Persistent checkpoint describing the contiguous confirmed prefix of a file transfer
/// so interrupted large transfers can validate integrity and resume without re-sending
/// already-confirmed chunks.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TransferCheckpoint {
    pub transfer_id: u64,
    /// SHA-256 fingerprint of the source file content.
    pub sha256: [u8; 32],
    pub total_bytes: u64,
    pub confirmed_offset: u64,
    pub chunk_size: u32,
}

impl TransferCheckpoint {
    pub fn new(
        transfer_id: u64,
        sha256: [u8; 32],
        total_bytes: u64,
        chunk_size: u32,
    ) -> Result<Self, TransferCheckpointError> {
        let checkpoint = Self {
            transfer_id,
            sha256,
            total_bytes,
            confirmed_offset: 0,
            chunk_size,
        };
        checkpoint.validate()?;
        Ok(checkpoint)
    }

    /// Compute the SHA-256 fingerprint of an in-memory buffer.
    pub fn fingerprint_from_bytes(bytes: &[u8]) -> [u8; 32] {
        let digest = Sha256::digest(bytes);
        let mut out = [0_u8; 32];
        out.copy_from_slice(&digest);
        out
    }

    /// Stream a file from disk and compute its SHA-256 fingerprint.
    pub async fn fingerprint_file(path: &Path) -> Result<[u8; 32], std::io::Error> {
        let mut file = tokio::fs::File::open(path).await?;
        let mut hasher = Sha256::new();
        let mut buf = [0_u8; 64 * 1024];
        loop {
            let n = file.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let digest = hasher.finalize();
        let mut out = [0_u8; 32];
        out.copy_from_slice(&digest);
        Ok(out)
    }

    /// Lowercase 64-character hex representation of `sha256`.
    pub fn fingerprint_hex(&self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.sha256 {
            use std::fmt::Write as _;
            let _ = write!(&mut out, "{byte:02x}");
        }
        out
    }

    /// Validate structural invariants of this checkpoint.
    pub fn validate(&self) -> Result<(), TransferCheckpointError> {
        if self.chunk_size == 0 {
            return Err(TransferCheckpointError::ZeroChunkSize);
        }
        if self.confirmed_offset > self.total_bytes {
            return Err(TransferCheckpointError::OffsetExceedsTotal {
                confirmed_offset: self.confirmed_offset,
                total_bytes: self.total_bytes,
            });
        }
        let chunk_size = u64::from(self.chunk_size);
        if self.confirmed_offset < self.total_bytes && self.confirmed_offset % chunk_size != 0 {
            return Err(TransferCheckpointError::UnalignedOffset {
                confirmed_offset: self.confirmed_offset,
                chunk_size: self.chunk_size,
            });
        }
        let total_chunks = self.total_bytes.div_ceil(chunk_size);
        if u32::try_from(total_chunks).is_err() {
            return Err(TransferCheckpointError::TooManyChunks {
                chunk_count: total_chunks,
            });
        }
        Ok(())
    }

    /// Validate this checkpoint against the expected source transfer metadata before resuming.
    pub fn validate_against(
        &self,
        expected_transfer_id: Option<u64>,
        expected_sha256: &[u8; 32],
        expected_total_bytes: u64,
        expected_chunk_size: u32,
    ) -> Result<(), TransferCheckpointError> {
        self.validate()?;
        if let Some(expected) = expected_transfer_id
            && self.transfer_id != expected
        {
            return Err(TransferCheckpointError::TransferIdMismatch {
                expected,
                actual: self.transfer_id,
            });
        }
        if self.sha256 != *expected_sha256 {
            return Err(TransferCheckpointError::FingerprintMismatch);
        }
        if self.total_bytes != expected_total_bytes {
            return Err(TransferCheckpointError::TotalBytesMismatch {
                expected: expected_total_bytes,
                actual: self.total_bytes,
            });
        }
        if self.chunk_size != expected_chunk_size {
            return Err(TransferCheckpointError::ChunkSizeMismatch {
                expected: expected_chunk_size,
                actual: self.chunk_size,
            });
        }
        Ok(())
    }

    /// Byte offset from which an interrupted transfer should resume sending.
    pub fn resume_offset(&self) -> Result<u64, TransferCheckpointError> {
        self.validate()?;
        Ok(self.confirmed_offset)
    }

    /// Total number of chunks required for `total_bytes` at `chunk_size`.
    pub fn total_chunks(&self) -> Result<u32, TransferCheckpointError> {
        self.validate()?;
        let chunks = self.total_bytes.div_ceil(u64::from(self.chunk_size));
        u32::try_from(chunks).map_err(|_| TransferCheckpointError::TooManyChunks {
            chunk_count: chunks,
        })
    }

    /// Zero-based index of the next unconfirmed chunk to send.
    /// Equals `total_chunks()` when the transfer is complete.
    pub fn resume_chunk_index(&self) -> Result<u32, TransferCheckpointError> {
        self.validate()?;
        if self.confirmed_offset == self.total_bytes {
            return self.total_chunks();
        }
        let chunk_index = self.confirmed_offset / u64::from(self.chunk_size);
        u32::try_from(chunk_index).map_err(|_| TransferCheckpointError::TooManyChunks {
            chunk_count: chunk_index,
        })
    }

    pub fn remaining_bytes(&self) -> Result<u64, TransferCheckpointError> {
        self.validate()?;
        Ok(self.total_bytes - self.confirmed_offset)
    }

    pub fn remaining_chunks(&self) -> Result<u32, TransferCheckpointError> {
        let total = self.total_chunks()?;
        let next = self.resume_chunk_index()?;
        Ok(total.saturating_sub(next))
    }

    pub fn is_complete(&self) -> bool {
        self.confirmed_offset >= self.total_bytes
    }

    pub fn is_chunk_confirmed(&self, chunk_index: u32) -> Result<bool, TransferCheckpointError> {
        Ok(chunk_index < self.resume_chunk_index()?)
    }

    /// Advance `confirmed_offset` to an explicit validated offset (monotonically non-decreasing).
    pub fn advance_to(&mut self, new_confirmed_offset: u64) -> Result<u64, TransferCheckpointError> {
        self.validate()?;
        if new_confirmed_offset < self.confirmed_offset {
            return Err(TransferCheckpointError::OffsetRegression {
                current: self.confirmed_offset,
                proposed: new_confirmed_offset,
            });
        }
        let candidate = Self {
            confirmed_offset: new_confirmed_offset,
            ..self.clone()
        };
        candidate.validate()?;
        self.confirmed_offset = new_confirmed_offset;
        Ok(self.confirmed_offset)
    }

    /// Record confirmation of a chunk by index and payload length.
    ///
    /// Already-confirmed chunks are treated idempotently; the next contiguous chunk
    /// advances `confirmed_offset`; out-of-order or wrong-length chunks are rejected.
    pub fn advance_chunk(
        &mut self,
        chunk_index: u32,
        chunk_len: usize,
    ) -> Result<u64, TransferCheckpointError> {
        self.validate()?;
        let expected_chunk = self.resume_chunk_index()?;
        let total_chunks = self.total_chunks()?;
        if chunk_index >= total_chunks {
            return Err(TransferCheckpointError::NonContiguousChunk {
                expected_chunk,
                actual_chunk: chunk_index,
            });
        }

        let chunk_size = u64::from(self.chunk_size);
        let offset = u64::from(chunk_index) * chunk_size;
        let expected_len = (self.total_bytes - offset).min(chunk_size) as usize;
        if chunk_len != expected_len {
            return Err(TransferCheckpointError::InvalidChunkLength {
                chunk_index,
                expected: expected_len,
                actual: chunk_len,
            });
        }

        if chunk_index < expected_chunk {
            // Idempotent duplicate confirmation of an already-checkpointed chunk.
            return Ok(self.confirmed_offset);
        }
        if chunk_index > expected_chunk {
            return Err(TransferCheckpointError::NonContiguousChunk {
                expected_chunk,
                actual_chunk: chunk_index,
            });
        }

        self.confirmed_offset = offset + chunk_len as u64;
        Ok(self.confirmed_offset)
    }
}

#[derive(Debug, Clone)]
pub struct FileTransferRuntimeConfig {
    pub share_catalog: Arc<crate::shared_files::SharedFileCatalog>,
    pub share_scope: crate::shared_files::ShareScope,
    pub stream_id: u32,
    pub next_transfer_id: u64,
    pub next_object_id: u64,
    pub next_sequence_number: u64,
    pub chunk_payload_len: usize,
    pub send_policy: FileTransferPolicy,
    pub receive_policy: FileReceivePolicy,
    pub receive_dir: PathBuf,
    /// True while queued sends or incomplete incoming objects need the connection.
    pub active: Arc<AtomicBool>,
    pub clipboard_owner: Option<crate::clipboard_file_runtime::ClipboardOwnerSlot>,
    /// Optional independent token-bucket bandwidth pacer for bulk file transfers.
    pub bandwidth_budget: Option<FileTransferBandwidthBudget>,
}

impl Default for FileTransferRuntimeConfig {
    fn default() -> Self {
        Self {
            share_catalog: crate::shared_files::shared_file_catalog(),
            share_scope: crate::shared_files::current_share_scope(),
            stream_id: 2,
            next_transfer_id: 1,
            next_object_id: 10_000,
            next_sequence_number: 1,
            chunk_payload_len: crate::file_transfer::DEFAULT_FILE_CHUNK_PAYLOAD_LEN,
            send_policy: FileTransferPolicy::default(),
            receive_policy: FileReceivePolicy::default(),
            receive_dir: std::env::temp_dir().join("weavelo-received-files"),
            active: Arc::new(AtomicBool::new(false)),
            clipboard_owner: None,
            bandwidth_budget: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileTransferCommand {
    /// Finish queued and active transfers, then emit Drained and stop. This is
    /// process-local lifecycle control, never a network command.
    Drain,
    SharedRequest {
        request_id: u64,
        request: protocol::shared_files::SharedFileRequest,
    },
    SendFile {
        path: PathBuf,
        mime_type: Option<String>,
    },
    SendFileGroup {
        files: Vec<FileTransferGroupFile>,
    },
    SendClipboardFiles {
        files: Vec<FileTransferGroupFile>,
    },
    SendOwnedClipboardFiles {
        files: Vec<FileTransferGroupFile>,
        admission: crate::clipboard_file_runtime::ClipboardFileAdmission,
    },
    CancelTransfer {
        transfer_id: u64,
    },
    CancelGroup {
        group_id: u64,
    },
    SetReceiveConfig {
        receive_dir: PathBuf,
        receive_policy: FileReceivePolicy,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTransferGroupFile {
    pub path: PathBuf,
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileTransferEvent {
    Drained,
    SharedResponse {
        request_id: u64,
        response: protocol::shared_files::SharedFileResponse,
    },
    IncomingClipboardReady {
        ownership: Option<crate::clipboard_file_runtime::ClipboardOwnership>,
        group_id: u64,
        paths: Vec<PathBuf>,
    },
    OutgoingGroupStarted {
        group_id: u64,
        file_count: u32,
        total_size_bytes: u64,
    },
    OutgoingStarted {
        transfer_id: u64,
        file_object_id: u64,
        group: Option<FileTransferGroup>,
        name: String,
        size_bytes: u64,
        total_chunks: u32,
    },
    OutgoingProgress {
        transfer_id: u64,
        file_object_id: u64,
        sent_chunks: u32,
        total_chunks: u32,
        sent_bytes: u64,
        total_size: u64,
    },
    OutgoingCompleted {
        transfer_id: u64,
        file_object_id: u64,
        group: Option<FileTransferGroup>,
    },
    OutgoingCancelled {
        transfer_id: u64,
        file_object_id: u64,
        group: Option<FileTransferGroup>,
    },
    OutgoingGroupCompleted {
        group_id: u64,
        file_count: u32,
        total_size_bytes: u64,
    },
    OutgoingGroupCancelled {
        group_id: u64,
    },
    IncomingStarted {
        transfer_id: u64,
        file_object_id: u64,
        group: Option<FileTransferGroup>,
        name: String,
        size_bytes: u64,
        path: PathBuf,
    },
    IncomingProgress {
        transfer_id: u64,
        file_object_id: u64,
        received_chunks: u32,
        total_chunks: u32,
        received_bytes: u64,
        total_size: u64,
    },
    IncomingCompleted {
        transfer_id: u64,
        file_object_id: u64,
        group: Option<FileTransferGroup>,
        path: PathBuf,
        size_bytes: u64,
    },
    IncomingCancelled {
        transfer_id: u64,
        file_object_id: u64,
        group: Option<FileTransferGroup>,
        path: PathBuf,
    },
    IncomingGroupCompleted {
        group_id: u64,
        paths: Vec<PathBuf>,
        total_size_bytes: u64,
    },
    IncomingGroupCancelled {
        group_id: u64,
    },
    Error {
        transfer_id: Option<u64>,
        message: String,
    },
}

#[derive(Debug)]
pub enum FileTransferRuntimeError {
    Send(ScheduledDataSendError),
    Transfer(FileTransferError),
    Delivery(String),
}

impl fmt::Display for FileTransferRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileTransferRuntimeError::Delivery(reason) => {
                write!(f, "file delivery failed: {reason}")
            }
            FileTransferRuntimeError::Send(err) => {
                write!(f, "file transfer sender failed: {err}")
            }
            FileTransferRuntimeError::Transfer(err) => {
                write!(f, "file transfer failed: {err}")
            }
        }
    }
}

impl Error for FileTransferRuntimeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            FileTransferRuntimeError::Delivery(_) => None,
            FileTransferRuntimeError::Send(err) => Some(err),
            FileTransferRuntimeError::Transfer(err) => Some(err),
        }
    }
}

impl From<ScheduledDataSendError> for FileTransferRuntimeError {
    fn from(value: ScheduledDataSendError) -> Self {
        Self::Send(value)
    }
}

impl From<FileTransferError> for FileTransferRuntimeError {
    fn from(value: FileTransferError) -> Self {
        Self::Transfer(value)
    }
}

#[derive(Clone)]
struct FileTransferIdAllocator {
    next_transfer_id: Arc<AtomicU64>,
    next_object_id: Arc<AtomicU64>,
    next_sequence_number: Arc<AtomicU64>,
    delivery: crate::file_delivery::FileDeliveryTracker,
}

impl FileTransferIdAllocator {
    fn new(config: &FileTransferRuntimeConfig) -> Self {
        Self {
            next_transfer_id: Arc::new(AtomicU64::new(config.next_transfer_id)),
            next_object_id: Arc::new(AtomicU64::new(config.next_object_id)),
            next_sequence_number: Arc::new(AtomicU64::new(config.next_sequence_number)),
            delivery: crate::file_delivery::FileDeliveryTracker::default(),
        }
    }

    fn next_transfer_id(&self) -> u64 {
        self.next_transfer_id.fetch_add(1, Relaxed)
    }

    fn next_group_id(&self) -> u64 {
        self.next_transfer_id.fetch_add(1, Relaxed)
    }

    fn next_object_id_pair(&self) -> (u64, u64) {
        let manifest_object_id = self.next_object_id.fetch_add(2, Relaxed);
        (manifest_object_id, manifest_object_id + 1)
    }

    fn next_sequence_number(&self) -> u64 {
        self.next_sequence_number.fetch_add(1, Relaxed)
    }
}

#[derive(Clone, Default)]
struct FileTransferCancelState {
    inner: Arc<Mutex<FileTransferCancelStateInner>>,
}

#[derive(Default)]
struct FileTransferCancelStateInner {
    transfer_ids: HashSet<u64>,
    group_ids: HashSet<u64>,
}

impl FileTransferCancelState {
    fn cancel_transfer(&self, transfer_id: u64) {
        self.inner
            .lock()
            .expect("file transfer cancel state lock should not be poisoned")
            .transfer_ids
            .insert(transfer_id);
    }

    fn cancel_group(&self, group_id: u64) {
        self.inner
            .lock()
            .expect("file transfer cancel state lock should not be poisoned")
            .group_ids
            .insert(group_id);
    }

    fn is_transfer_cancelled(&self, transfer_id: u64) -> bool {
        self.inner
            .lock()
            .expect("file transfer cancel state lock should not be poisoned")
            .transfer_ids
            .contains(&transfer_id)
    }

    fn is_group_cancelled(&self, group_id: u64) -> bool {
        self.inner
            .lock()
            .expect("file transfer cancel state lock should not be poisoned")
            .group_ids
            .contains(&group_id)
    }
}

fn clipboard_command_admitted(command: &FileTransferCommand) -> bool {
    !matches!(command, FileTransferCommand::SendOwnedClipboardFiles { admission, .. } if !admission.ownership.current())
}

pub async fn run_file_transfer_runtime(
    data_sender: ScheduledDataSender,
    mut command_rx: mpsc::Receiver<FileTransferCommand>,
    mut inbound_rx: mpsc::Receiver<DataEnvelope>,
    event_tx: mpsc::UnboundedSender<FileTransferEvent>,
    mut cancel_rx: broadcast::Receiver<()>,
    config: FileTransferRuntimeConfig,
) -> Result<(), FileTransferRuntimeError> {
    let allocator = FileTransferIdAllocator::new(&config);
    let cancel_state = FileTransferCancelState::default();
    let mut inbound = InboundFileTransfers::new(config.receive_dir.clone(), config.receive_policy);
    inbound.clipboard_owner = config.clipboard_owner.clone();
    let mut send_tasks = tokio::task::JoinSet::new();
    struct ClearActivity(Arc<AtomicBool>);
    impl Drop for ClearActivity {
        fn drop(&mut self) {
            self.0.store(false, Relaxed);
        }
    }
    let _clear_activity = ClearActivity(config.active.clone());
    let mut exchange = shared_exchange::SharedExchange::new(
        config.clone(),
        allocator.clone(),
        data_sender.clone(),
        event_tx.clone(),
        cancel_rx.resubscribe(),
        cancel_state.clone(),
    );
    let mut share_retry = tokio::time::interval(std::time::Duration::from_millis(500));
    share_retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut draining = false;
    loop {
        let active = !send_tasks.is_empty()
            || exchange.pending()
            || !command_rx.is_empty()
            || !inbound_rx.is_empty()
            || !inbound.incoming_files.is_empty()
            || !inbound.manifest_assemblers.is_empty()
            || !inbound.incoming_groups.is_empty();
        config.active.store(active, Relaxed);
        // Incoming completion precedes delivery of its final ACK. Retain its
        // duplicate-ACK responder for the sender's existing retry deadline.
        let receipt_deadline = inbound
            .receipt_deadline
            .filter(|until| *until > tokio::time::Instant::now());
        if draining && !active && receipt_deadline.is_none() {
            emit_event(&event_tx, FileTransferEvent::Drained);
            return Ok(());
        }
        tokio::select! {
            _ = tokio::time::sleep_until(receipt_deadline.unwrap_or_else(tokio::time::Instant::now)), if draining && receipt_deadline.is_some() => {}
            _ = cancel_rx.recv() => {
                return Ok(());
            }
            _ = send_tasks.join_next(), if !send_tasks.is_empty() => {}
            _ = share_retry.tick(), if exchange.pending() => { exchange.tick().await?; }
            maybe_command = command_rx.recv() => {
                let Some(command) = maybe_command else {
                    return Ok(());
                };
                if !clipboard_command_admitted(&command) {
                    if let FileTransferCommand::SendOwnedClipboardFiles { admission, .. } = &command { admission.reject(); }
                    continue;
                }
                if send_tasks.len() >= 4 && matches!(&command, FileTransferCommand::SendFile { .. } | FileTransferCommand::SendFileGroup { .. } | FileTransferCommand::SendClipboardFiles { .. } | FileTransferCommand::SendOwnedClipboardFiles { .. }) {
                    if let FileTransferCommand::SendOwnedClipboardFiles { admission, .. } = &command { admission.reject(); }
                    emit_event(&event_tx, FileTransferEvent::Error { transfer_id: None,
                        message: "Four file transfers are already active; wait for one to finish".into() });
                    continue;
                }
                if let FileTransferCommand::SendOwnedClipboardFiles { admission, .. } = &command {
                    if !admission.accept() { continue; }
                }
                let clipboard = matches!(&command, FileTransferCommand::SendClipboardFiles { .. } | FileTransferCommand::SendOwnedClipboardFiles { .. });
                match command {
                    FileTransferCommand::Drain => { draining = true; }
                    FileTransferCommand::SharedRequest { request_id, request } => { exchange.request(request_id, request).await?; }
                    FileTransferCommand::SendFile { path, mime_type } => {
                        let spec = next_file_transfer_spec(&config, &allocator);
                        SendFileTask {
                            path,
                            mime_type,
                            spec,
                            data_sender: data_sender.clone(),
                            event_tx: event_tx.clone(),
                            cancel_rx: cancel_rx.resubscribe(),
                            send_policy: config.send_policy,
                            allocator: allocator.clone(),
                            cancel_state: cancel_state.clone(),
                            stream_id: config.stream_id,
                            bandwidth_budget: config.bandwidth_budget.clone(),
                        }
                        .spawn(&mut send_tasks);
                    }
                    FileTransferCommand::SendFileGroup { files } | FileTransferCommand::SendClipboardFiles { files } | FileTransferCommand::SendOwnedClipboardFiles { files, .. } => {
                        let group_id = allocator.next_group_id();
                        SendFileGroupTask {
                            clipboard,
                            group_id,
                            files,
                            config: config.clone(),
                            data_sender: data_sender.clone(),
                            event_tx: event_tx.clone(),
                            cancel_rx: cancel_rx.resubscribe(),
                            allocator: allocator.clone(),
                            cancel_state: cancel_state.clone(),
                        }
                        .spawn(&mut send_tasks);
                    }
                    FileTransferCommand::CancelTransfer { transfer_id } => {
                        cancel_state.cancel_transfer(transfer_id);
                    }
                    FileTransferCommand::CancelGroup { group_id } => {
                        cancel_state.cancel_group(group_id);
                    }
                    FileTransferCommand::SetReceiveConfig {
                        receive_dir,
                        receive_policy,
                    } => {
                        inbound.set_receive_config(receive_dir, receive_policy);
                    }
                }
            }
            maybe_envelope = inbound_rx.recv() => {
                let Some(envelope) = maybe_envelope else {
                    return Ok(());
                };
                if exchange.handle(&envelope, &mut send_tasks).await? { continue; }
                if envelope.header.kind == ContentKind::FileControl
                    && let Ok(control @ (FileTransferControl::ChunkAccepted { .. } | FileTransferControl::Rejected { .. })) = control_from_envelope(&envelope)
                {
                    allocator.delivery.route(control);
                    continue;
                }
                if let Some(reply) = inbound.handle_envelope(envelope, &event_tx).await {
                    send_file_control(reply, config.stream_id, &data_sender, &allocator).await?;
                }
            }
        }
    }
}

fn next_file_transfer_spec(
    config: &FileTransferRuntimeConfig,
    allocator: &FileTransferIdAllocator,
) -> FileTransferSpec {
    let transfer_id = allocator.next_transfer_id();
    let (manifest_object_id, file_object_id) = allocator.next_object_id_pair();
    FileTransferSpec {
        transfer_id,
        manifest_object_id,
        file_object_id,
        stream_id: config.stream_id,
        first_sequence_number: 0,
        timestamp_ms: now_ms(),
        chunk_payload_len: config.chunk_payload_len,
        group: None,
    }
}

struct SendFileTask {
    path: PathBuf,
    mime_type: Option<String>,
    spec: FileTransferSpec,
    data_sender: ScheduledDataSender,
    event_tx: mpsc::UnboundedSender<FileTransferEvent>,
    cancel_rx: broadcast::Receiver<()>,
    send_policy: FileTransferPolicy,
    allocator: FileTransferIdAllocator,
    cancel_state: FileTransferCancelState,
    stream_id: u32,
    bandwidth_budget: Option<FileTransferBandwidthBudget>,
}

struct SendFileGroupTask {
    clipboard: bool,
    group_id: u64,
    files: Vec<FileTransferGroupFile>,
    config: FileTransferRuntimeConfig,
    data_sender: ScheduledDataSender,
    event_tx: mpsc::UnboundedSender<FileTransferEvent>,
    cancel_rx: broadcast::Receiver<()>,
    allocator: FileTransferIdAllocator,
    cancel_state: FileTransferCancelState,
}

struct PreparedGroupFile {
    path: PathBuf,
    mime_type: Option<String>,
    relative_path: String,
}

impl SendFileGroupTask {
    fn spawn(self, tasks: &mut tokio::task::JoinSet<()>) {
        tasks.spawn(async move {
            let group_id = self.group_id;
            let event_tx = self.event_tx.clone();
            if let Err(err) = self.run().await {
                emit_event(
                    &event_tx,
                    FileTransferEvent::Error {
                        transfer_id: Some(group_id),
                        message: err.to_string(),
                    },
                );
            }
        });
    }

    async fn run(mut self) -> Result<(), FileTransferRuntimeError> {
        if self.files.is_empty() {
            return Ok(());
        }

        let files = expand_group_files(&self.files).await?;
        if files.is_empty() {
            return Ok(());
        }

        let mut readers = Vec::with_capacity(files.len());
        for file in &files {
            let spec = next_file_transfer_spec(&self.config, &self.allocator);
            let reader = FileTransferReader::from_path(
                &file.path,
                spec,
                file.mime_type.clone(),
                self.config.send_policy,
            )
            .await?;
            readers.push(PreparedGroupReader {
                reader,
                relative_path: file.relative_path.clone(),
            });
        }

        let group_total_size_bytes = readers
            .iter()
            .map(|reader| reader.reader.manifest().size_bytes)
            .sum::<u64>();
        let group_checksum_crc32 = file_group_checksum_crc32(&readers);
        let file_count = u32::try_from(readers.len()).unwrap_or(u32::MAX);

        if self.clipboard {
            let (object_id, spare) = self.allocator.next_object_id_pair();
            let mut delivery = self.allocator.delivery.window([object_id, spare]);
            let envelope = control_to_envelope(
                &FileTransferControl::ClipboardGroup {
                    group_id: self.group_id,
                },
                object_id,
                self.config.stream_id,
                self.allocator.next_sequence_number(),
                now_ms(),
            )?;
            let cancelled = || self.cancel_state.is_group_cancelled(self.group_id);
            delivery
                .send(envelope, &self.data_sender, cancelled)
                .await
                .map_err(FileTransferRuntimeError::Delivery)?;
            if !delivery
                .flush(&self.data_sender, cancelled)
                .await
                .map_err(FileTransferRuntimeError::Delivery)?
            {
                return Ok(());
            }
        }

        emit_event(
            &self.event_tx,
            FileTransferEvent::OutgoingGroupStarted {
                group_id: self.group_id,
                file_count,
                total_size_bytes: group_total_size_bytes,
            },
        );

        for (index, reader) in readers.into_iter().enumerate() {
            if self.cancel_state.is_group_cancelled(self.group_id) {
                send_file_control(
                    FileTransferControl::CancelGroup {
                        group_id: self.group_id,
                    },
                    self.config.stream_id,
                    &self.data_sender,
                    &self.allocator,
                )
                .await?;
                emit_event(
                    &self.event_tx,
                    FileTransferEvent::OutgoingGroupCancelled {
                        group_id: self.group_id,
                    },
                );
                return Ok(());
            }

            let mut file_reader = reader.reader;
            let file_index = u32::try_from(index).unwrap_or(u32::MAX);
            let relative_path = reader.relative_path;
            file_reader.set_group(FileTransferGroup {
                group_id: self.group_id,
                file_index,
                file_count,
                relative_path,
                group_total_size_bytes,
                group_checksum_crc32,
            });
            match send_reader_with_budget(
                &mut file_reader,
                &self.data_sender,
                &self.event_tx,
                &mut self.cancel_rx,
                &self.allocator,
                &self.cancel_state,
                self.config.stream_id,
                self.config.bandwidth_budget.as_ref(),
            )
            .await?
            {
                SendReaderOutcome::Completed => {}
                SendReaderOutcome::TransferCancelled => return Ok(()),
                SendReaderOutcome::GroupCancelled { group_id } => {
                    self.cancel_state.cancel_group(group_id);
                    emit_event(
                        &self.event_tx,
                        FileTransferEvent::OutgoingGroupCancelled { group_id },
                    );
                    return Ok(());
                }
            }
        }

        emit_event(
            &self.event_tx,
            FileTransferEvent::OutgoingGroupCompleted {
                group_id: self.group_id,
                file_count,
                total_size_bytes: group_total_size_bytes,
            },
        );
        Ok(())
    }
}

enum SendReaderOutcome {
    Completed,
    TransferCancelled,
    GroupCancelled { group_id: u64 },
}

struct PreparedGroupReader {
    reader: FileTransferReader,
    relative_path: String,
}

async fn expand_group_files(
    files: &[FileTransferGroupFile],
) -> Result<Vec<PreparedGroupFile>, FileTransferError> {
    let mut expanded = Vec::new();
    for file in files {
        let metadata = tokio::fs::symlink_metadata(&file.path).await?;
        if metadata.is_file() {
            expanded.push(PreparedGroupFile {
                path: file.path.clone(),
                mime_type: file.mime_type.clone(),
                relative_path: top_level_relative_path(&file.path)?,
            });
        } else if metadata.is_dir() {
            expand_group_directory(&file.path, &mut expanded).await?;
        } else {
            return Err(FileTransferError::NotAFile(file.path.clone()));
        }
    }

    expanded.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(expanded)
}

async fn expand_group_directory(
    root: &Path,
    expanded: &mut Vec<PreparedGroupFile>,
) -> Result<(), FileTransferError> {
    let root_name = top_level_relative_path(root)?;
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let mut entries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = read_dir.next_entry().await? {
            entries.push(entry.path());
        }
        entries.sort();

        for path in entries.into_iter().rev() {
            let metadata = tokio::fs::symlink_metadata(&path).await?;
            if metadata.is_dir() {
                stack.push(path);
            } else if metadata.is_file() {
                let child = path
                    .strip_prefix(root)
                    .map_err(|_| FileTransferError::InvalidFileName)?;
                expanded.push(PreparedGroupFile {
                    relative_path: nested_relative_path(&root_name, child)?,
                    path,
                    mime_type: None,
                });
            } else {
                return Err(FileTransferError::NotAFile(path));
            }
        }
    }

    Ok(())
}

fn top_level_relative_path(path: &Path) -> Result<String, FileTransferError> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or(FileTransferError::InvalidFileName)?;
    Ok(name.to_string())
}

fn nested_relative_path(root_name: &str, child: &Path) -> Result<String, FileTransferError> {
    let mut parts = vec![root_name.to_string()];
    for component in child.components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_str().ok_or(FileTransferError::InvalidFileName)?;
                if value.is_empty() {
                    return Err(FileTransferError::InvalidFileName);
                }
                parts.push(value.to_string());
            }
            _ => return Err(FileTransferError::InvalidFileName),
        }
    }
    Ok(parts.join("/"))
}

impl SendFileTask {
    fn spawn(self, tasks: &mut tokio::task::JoinSet<()>) {
        tasks.spawn(async move {
            let transfer_id = self.spec.transfer_id;
            let event_tx = self.event_tx.clone();
            if let Err(err) = self.run().await {
                emit_event(
                    &event_tx,
                    FileTransferEvent::Error {
                        transfer_id: Some(transfer_id),
                        message: err.to_string(),
                    },
                );
            }
        });
    }

    async fn run(mut self) -> Result<(), FileTransferRuntimeError> {
        let mut reader =
            FileTransferReader::from_path(&self.path, self.spec, self.mime_type, self.send_policy)
                .await?;
        send_reader_with_budget(
            &mut reader,
            &self.data_sender,
            &self.event_tx,
            &mut self.cancel_rx,
            &self.allocator,
            &self.cancel_state,
            self.stream_id,
            self.bandwidth_budget.as_ref(),
        )
        .await?;
        Ok(())
    }
}

async fn send_reader(
    reader: &mut FileTransferReader,
    data_sender: &ScheduledDataSender,
    event_tx: &mpsc::UnboundedSender<FileTransferEvent>,
    cancel_rx: &mut broadcast::Receiver<()>,
    allocator: &FileTransferIdAllocator,
    cancel_state: &FileTransferCancelState,
    stream_id: u32,
) -> Result<SendReaderOutcome, FileTransferRuntimeError> {
    send_reader_with_budget(
        reader,
        data_sender,
        event_tx,
        cancel_rx,
        allocator,
        cancel_state,
        stream_id,
        None,
    )
    .await
}

async fn send_reader_with_budget(
    reader: &mut FileTransferReader,
    data_sender: &ScheduledDataSender,
    event_tx: &mpsc::UnboundedSender<FileTransferEvent>,
    cancel_rx: &mut broadcast::Receiver<()>,
    allocator: &FileTransferIdAllocator,
    cancel_state: &FileTransferCancelState,
    stream_id: u32,
    bandwidth_budget: Option<&FileTransferBandwidthBudget>,
) -> Result<SendReaderOutcome, FileTransferRuntimeError> {
    let manifest = reader.manifest().clone();
    let total_chunks = reader.total_chunks();
    let mut delivery = allocator
        .delivery
        .window([reader.manifest_object_id(), manifest.file_object_id]);
    let cancelled = || {
        cancel_state.is_transfer_cancelled(manifest.transfer_id)
            || manifest
                .group
                .as_ref()
                .is_some_and(|group| cancel_state.is_group_cancelled(group.group_id))
    };
    emit_event(
        event_tx,
        FileTransferEvent::OutgoingStarted {
            transfer_id: manifest.transfer_id,
            file_object_id: manifest.file_object_id,
            group: manifest.group.clone(),
            name: manifest.name.clone(),
            size_bytes: manifest.size_bytes,
            total_chunks,
        },
    );

    let mut sent_file_chunks = 0_u32;
    let mut sent_file_bytes = 0_u64;

    loop {
        if let Some(outcome) = handle_outgoing_cancel(
            &manifest,
            data_sender,
            event_tx,
            allocator,
            cancel_state,
            stream_id,
        )
        .await?
        {
            return Ok(outcome);
        }

        let maybe_envelope = tokio::select! {
            _ = cancel_rx.recv() => {
                return Ok(SendReaderOutcome::TransferCancelled);
            }
            result = reader.next_envelope() => {
                result?
            }
        };

        if let Some(outcome) = handle_outgoing_cancel(
            &manifest,
            data_sender,
            event_tx,
            allocator,
            cancel_state,
            stream_id,
        )
        .await?
        {
            return Ok(outcome);
        }

        let Some(mut envelope) = maybe_envelope else {
            let confirmed = tokio::select! {
                _ = cancel_rx.recv() => return Ok(SendReaderOutcome::TransferCancelled),
                result = delivery.flush(data_sender, cancelled) => result.map_err(FileTransferRuntimeError::Delivery)?,
            };
            if !confirmed {
                continue;
            }
            emit_event(
                event_tx,
                FileTransferEvent::OutgoingCompleted {
                    transfer_id: manifest.transfer_id,
                    file_object_id: manifest.file_object_id,
                    group: manifest.group.clone(),
                },
            );
            return Ok(SendReaderOutcome::Completed);
        };

        assign_sequence_number(&mut envelope, allocator.next_sequence_number());
        if envelope.header.kind == ContentKind::FileChunk {
            if let Some(budget) = bandwidth_budget {
                let delay = budget.reserve_delay(envelope.payload.len());
                if !delay.is_zero() {
                    tokio::select! {
                        _ = cancel_rx.recv() => return Ok(SendReaderOutcome::TransferCancelled),
                        _ = tokio::time::sleep(delay) => {}
                    }
                }
            }
            sent_file_chunks = sent_file_chunks.saturating_add(1);
            sent_file_bytes = sent_file_bytes.saturating_add(envelope.payload.len() as u64);
        }

        let is_manifest = envelope.header.kind == ContentKind::FileManifest;
        let confirmed = tokio::select! {
            _ = cancel_rx.recv() => return Ok(SendReaderOutcome::TransferCancelled),
            result = async {
                if !delivery.send(envelope, data_sender, cancelled).await? { return Ok(false); }
                // Do not send data until the receiver has accepted the manifest.
                if is_manifest { delivery.flush(data_sender, cancelled).await } else { Ok(true) }
            } => result.map_err(FileTransferRuntimeError::Delivery)?,
        };
        if !confirmed {
            continue;
        }

        if sent_file_chunks > 0 {
            emit_event(
                event_tx,
                FileTransferEvent::OutgoingProgress {
                    transfer_id: manifest.transfer_id,
                    file_object_id: manifest.file_object_id,
                    sent_chunks: sent_file_chunks,
                    total_chunks,
                    sent_bytes: sent_file_bytes,
                    total_size: manifest.size_bytes,
                },
            );
        }
    }
}

async fn handle_outgoing_cancel(
    manifest: &protocol::FileTransferManifest,
    data_sender: &ScheduledDataSender,
    event_tx: &mpsc::UnboundedSender<FileTransferEvent>,
    allocator: &FileTransferIdAllocator,
    cancel_state: &FileTransferCancelState,
    stream_id: u32,
) -> Result<Option<SendReaderOutcome>, FileTransferRuntimeError> {
    if let Some(group) = &manifest.group
        && (cancel_state.is_group_cancelled(group.group_id)
            || cancel_state.is_transfer_cancelled(manifest.transfer_id))
    {
        send_file_control(
            FileTransferControl::CancelGroup {
                group_id: group.group_id,
            },
            stream_id,
            data_sender,
            allocator,
        )
        .await?;
        emit_event(
            event_tx,
            FileTransferEvent::OutgoingCancelled {
                transfer_id: manifest.transfer_id,
                file_object_id: manifest.file_object_id,
                group: manifest.group.clone(),
            },
        );
        return Ok(Some(SendReaderOutcome::GroupCancelled {
            group_id: group.group_id,
        }));
    }

    if cancel_state.is_transfer_cancelled(manifest.transfer_id) {
        send_file_control(
            FileTransferControl::CancelTransfer {
                transfer_id: manifest.transfer_id,
                file_object_id: manifest.file_object_id,
            },
            stream_id,
            data_sender,
            allocator,
        )
        .await?;
        emit_event(
            event_tx,
            FileTransferEvent::OutgoingCancelled {
                transfer_id: manifest.transfer_id,
                file_object_id: manifest.file_object_id,
                group: manifest.group.clone(),
            },
        );
        return Ok(Some(SendReaderOutcome::TransferCancelled));
    }

    Ok(None)
}

async fn send_file_control(
    control: FileTransferControl,
    stream_id: u32,
    data_sender: &ScheduledDataSender,
    allocator: &FileTransferIdAllocator,
) -> Result<(), FileTransferRuntimeError> {
    let (object_id, _) = allocator.next_object_id_pair();
    let sequence_number = allocator.next_sequence_number();
    let envelope = control_to_envelope(&control, object_id, stream_id, sequence_number, now_ms())?;
    data_sender.send(envelope).await?;
    Ok(())
}

fn file_group_checksum_crc32(readers: &[PreparedGroupReader]) -> u32 {
    let mut bytes = Vec::new();
    for reader in readers {
        let manifest = reader.reader.manifest();
        bytes.extend_from_slice(reader.relative_path.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&manifest.size_bytes.to_be_bytes());
        bytes.extend_from_slice(&manifest.checksum_crc32.to_be_bytes());
    }
    crate::data_plane::checksum_crc32(&bytes)
}

fn assign_sequence_number(envelope: &mut DataEnvelope, sequence_number: u64) {
    envelope.header.sequence_number = sequence_number;
    if let Some(reliability) = &mut envelope.header.reliability_info {
        reliability.ack_id = sequence_number;
    }
}

struct InboundFileTransfers {
    clipboard_groups: HashMap<u64, Option<crate::clipboard_file_runtime::ClipboardOwnership>>,
    clipboard_owner: Option<crate::clipboard_file_runtime::ClipboardOwnerSlot>,
    receive_dir: PathBuf,
    receive_policy: FileReceivePolicy,
    manifest_assemblers: HashMap<u64, FileManifestAssembler>,
    incoming_files: HashMap<u64, IncomingFileTransfer>,
    incoming_groups: HashMap<u64, IncomingFileGroup>,
    cancelled_file_objects: HashSet<u64>,
    cancelled_groups: HashSet<u64>,
    pending_receive_config: Option<(PathBuf, FileReceivePolicy)>,
    completed_files: HashMap<u64, protocol::FileTransferManifest>,
    completed_order: VecDeque<u64>,
    receipt_deadline: Option<tokio::time::Instant>,
}

impl InboundFileTransfers {
    fn new(receive_dir: PathBuf, receive_policy: FileReceivePolicy) -> Self {
        Self {
            clipboard_groups: HashMap::new(),
            clipboard_owner: None,
            receive_dir,
            receive_policy,
            manifest_assemblers: HashMap::new(),
            incoming_files: HashMap::new(),
            incoming_groups: HashMap::new(),
            cancelled_file_objects: HashSet::new(),
            cancelled_groups: HashSet::new(),
            pending_receive_config: None,
            completed_files: HashMap::new(),
            completed_order: VecDeque::new(),
            receipt_deadline: None,
        }
    }

    fn set_receive_config(&mut self, receive_dir: PathBuf, receive_policy: FileReceivePolicy) {
        if self.has_active_incoming_work() {
            self.pending_receive_config = Some((receive_dir, receive_policy));
            return;
        }

        self.receive_dir = receive_dir;
        self.receive_policy = receive_policy;
    }

    fn has_active_incoming_work(&self) -> bool {
        !self.manifest_assemblers.is_empty()
            || !self.incoming_files.is_empty()
            || !self.incoming_groups.is_empty()
    }

    fn apply_pending_receive_config_if_idle(&mut self) {
        if self.has_active_incoming_work() {
            return;
        }

        if let Some((receive_dir, receive_policy)) = self.pending_receive_config.take() {
            self.receive_dir = receive_dir;
            self.receive_policy = receive_policy;
        }
    }

    async fn handle_envelope(
        &mut self,
        envelope: DataEnvelope,
        event_tx: &mpsc::UnboundedSender<FileTransferEvent>,
    ) -> Option<FileTransferControl> {
        let identity = envelope
            .header
            .chunk
            .as_ref()
            .map(|chunk| (chunk.object_id, chunk.chunk_index));
        let acknowledge = matches!(
            envelope.header.kind,
            ContentKind::FileManifest | ContentKind::FileChunk | ContentKind::FileControl
        );
        let result = match envelope.header.kind {
            ContentKind::FileManifest => self.handle_manifest(envelope, event_tx).await,
            ContentKind::FileChunk => self.handle_file_chunk(envelope, event_tx).await,
            ContentKind::FileControl => self.handle_control(envelope, event_tx).await,
            other => Err(FileTransferError::UnexpectedContentKind(other)),
        };

        let reply = match result {
            Ok(()) if acknowledge => {
                identity.map(
                    |(object_id, chunk_index)| FileTransferControl::ChunkAccepted {
                        object_id,
                        chunk_index,
                    },
                )
            }
            Ok(()) => None,
            Err(err) => {
                let reason = err.to_string();
                emit_event(
                    event_tx,
                    FileTransferEvent::Error {
                        transfer_id: None,
                        message: reason.clone(),
                    },
                );
                identity.map(|(object_id, _)| FileTransferControl::Rejected { object_id, reason })
            }
        };

        self.apply_pending_receive_config_if_idle();
        reply
    }

    async fn handle_manifest(
        &mut self,
        envelope: DataEnvelope,
        event_tx: &mpsc::UnboundedSender<FileTransferEvent>,
    ) -> Result<(), FileTransferError> {
        let object_id = envelope
            .header
            .chunk
            .as_ref()
            .map(|chunk| chunk.object_id)
            .ok_or(FileTransferError::MissingChunkMetadata)?;

        if self.manifest_assemblers.len() >= 32
            && !self.manifest_assemblers.contains_key(&object_id)
        {
            return Err(FileTransferError::InvalidChunk {
                reason: "too many incomplete file manifests",
            });
        }

        let assembler = match self.manifest_assemblers.entry(object_id) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(FileManifestAssembler::from_first_chunk(&envelope)?)
            }
        };
        let progress = assembler.push_chunk(envelope)?;
        if !progress.is_complete {
            return Ok(());
        }

        let assembler = self
            .manifest_assemblers
            .remove(&object_id)
            .expect("complete manifest assembler should still be present");
        let manifest = assembler.finish(self.receive_policy)?;
        if self.is_manifest_cancelled(&manifest) {
            return Err(FileTransferError::InvalidChunk {
                reason: "transfer was cancelled",
            });
        }
        if let Some(existing) = self
            .incoming_files
            .get(&manifest.file_object_id)
            .map(|incoming| incoming.manifest())
            .or_else(|| self.completed_files.get(&manifest.file_object_id))
        {
            return if existing == &manifest {
                Ok(())
            } else {
                Err(FileTransferError::InvalidChunk {
                    reason: "conflicting duplicate manifest",
                })
            };
        }
        if self.incoming_files.len() >= 16 {
            return Err(FileTransferError::InvalidChunk {
                reason: "too many incoming files",
            });
        }
        if let Some(group) = &manifest.group {
            if self.incoming_groups.len() >= 32
                && !self.incoming_groups.contains_key(&group.group_id)
            {
                return Err(FileTransferError::InvalidChunk {
                    reason: "too many incoming groups",
                });
            }
            if let Some(existing) = self.incoming_groups.get(&group.group_id)
                && (existing.file_count != group.file_count
                    || existing.total_size != group.group_total_size_bytes
                    || existing.checksum != group.group_checksum_crc32)
            {
                return Err(FileTransferError::InvalidChunk {
                    reason: "inconsistent file group metadata",
                });
            }
        }
        let incoming =
            IncomingFileTransfer::start(manifest.clone(), &self.receive_dir, self.receive_policy)
                .await?;
        let path = incoming.path().to_path_buf();
        let file_object_id = manifest.file_object_id;

        emit_event(
            event_tx,
            FileTransferEvent::IncomingStarted {
                transfer_id: manifest.transfer_id,
                file_object_id,
                group: manifest.group.clone(),
                name: manifest.name.clone(),
                size_bytes: manifest.size_bytes,
                path,
            },
        );
        if incoming.progress().is_complete {
            let received = incoming.finish().await?;
            self.record_group_completion(&received.manifest, received.path.clone(), event_tx)?;
            self.remember_completed(received.manifest.clone());
            let group = received.manifest.group.clone();
            emit_event(
                event_tx,
                FileTransferEvent::IncomingCompleted {
                    transfer_id: received.manifest.transfer_id,
                    file_object_id: received.manifest.file_object_id,
                    group: group.clone(),
                    path: received.path,
                    size_bytes: received.manifest.size_bytes,
                },
            );
        } else {
            self.incoming_files.insert(file_object_id, incoming);
        }
        Ok(())
    }

    async fn handle_control(
        &mut self,
        envelope: DataEnvelope,
        event_tx: &mpsc::UnboundedSender<FileTransferEvent>,
    ) -> Result<(), FileTransferError> {
        match control_from_envelope(&envelope)? {
            FileTransferControl::ClipboardGroup { group_id } => {
                if self.clipboard_groups.len() >= 32
                    && !self.clipboard_groups.contains_key(&group_id)
                {
                    return Err(FileTransferError::InvalidChunk {
                        reason: "too many pending clipboard groups",
                    });
                }
                self.clipboard_groups.entry(group_id).or_insert_with(|| {
                    self.clipboard_owner
                        .as_ref()
                        .and_then(|slot| slot.lock().unwrap().clone())
                });
            }
            FileTransferControl::CancelTransfer {
                transfer_id,
                file_object_id,
            } => {
                self.cancelled_file_objects.insert(file_object_id);
                self.cancel_incoming_file(transfer_id, file_object_id, event_tx)
                    .await?;
            }
            FileTransferControl::CancelGroup { group_id } => {
                self.cancelled_groups.insert(group_id);
                self.cancel_incoming_group(group_id, event_tx).await?;
            }
            FileTransferControl::ChunkAccepted { .. }
            | FileTransferControl::Rejected { .. }
            | FileTransferControl::SharedRequest { .. }
            | FileTransferControl::SharedResponse { .. } => {}
        }
        Ok(())
    }

    async fn handle_file_chunk(
        &mut self,
        envelope: DataEnvelope,
        event_tx: &mpsc::UnboundedSender<FileTransferEvent>,
    ) -> Result<(), FileTransferError> {
        let object_id = envelope
            .header
            .chunk
            .as_ref()
            .map(|chunk| chunk.object_id)
            .ok_or(FileTransferError::MissingChunkMetadata)?;
        if let Some(manifest) = self.completed_files.get(&object_id) {
            let chunk = envelope.header.chunk.as_ref().unwrap();
            let offset = u64::from(chunk.chunk_index) * u64::from(manifest.chunk_payload_len);
            let total_chunks = manifest
                .size_bytes
                .div_ceil(u64::from(manifest.chunk_payload_len));
            if chunk.total_size == manifest.size_bytes
                && u64::from(chunk.total_chunks) == total_chunks
                && u64::from(chunk.chunk_index) < total_chunks
                && chunk.offset == offset
                && envelope.payload.len() as u64
                    == (manifest.size_bytes - offset).min(u64::from(manifest.chunk_payload_len))
            {
                return Ok(());
            }
            return Err(FileTransferError::InvalidChunk {
                reason: "conflicting completed file chunk",
            });
        }
        let incoming =
            self.incoming_files
                .get_mut(&object_id)
                .ok_or(FileTransferError::InvalidChunk {
                    reason: "file chunk arrived before its manifest",
                })?;

        let progress = incoming.push_chunk(envelope).await?;
        emit_event(
            event_tx,
            FileTransferEvent::IncomingProgress {
                transfer_id: progress.transfer_id,
                file_object_id: progress.file_object_id,
                received_chunks: progress.received_chunks,
                total_chunks: progress.total_chunks,
                received_bytes: progress.received_bytes,
                total_size: progress.total_size,
            },
        );

        if progress.is_complete {
            let incoming = self
                .incoming_files
                .remove(&object_id)
                .expect("complete incoming transfer should still be present");
            let received = incoming.finish().await?;
            self.record_group_completion(&received.manifest, received.path.clone(), event_tx)?;
            self.remember_completed(received.manifest.clone());
            let group = received.manifest.group.clone();
            emit_event(
                event_tx,
                FileTransferEvent::IncomingCompleted {
                    transfer_id: received.manifest.transfer_id,
                    file_object_id: received.manifest.file_object_id,
                    group: group.clone(),
                    path: received.path,
                    size_bytes: received.manifest.size_bytes,
                },
            );
        }

        Ok(())
    }

    fn remember_completed(&mut self, manifest: protocol::FileTransferManifest) {
        self.receipt_deadline = Some(tokio::time::Instant::now() + crate::file_delivery::DEADLINE);
        self.completed_order.push_back(manifest.file_object_id);
        self.completed_files
            .insert(manifest.file_object_id, manifest);
        while self.completed_order.len() > 4096 {
            if let Some(old) = self.completed_order.pop_front() {
                self.completed_files.remove(&old);
            }
        }
    }

    fn record_group_completion(
        &mut self,
        manifest: &protocol::FileTransferManifest,
        path: PathBuf,
        event_tx: &mpsc::UnboundedSender<FileTransferEvent>,
    ) -> Result<(), FileTransferError> {
        let Some(group) = manifest.group.as_ref() else {
            return Ok(());
        };

        let publish_path =
            received_group_publish_path(&self.receive_dir, &group.relative_path, &path)?;
        let entry = self
            .incoming_groups
            .entry(group.group_id)
            .or_insert_with(|| IncomingFileGroup::new(group));
        entry.record(
            group,
            path,
            publish_path,
            manifest.size_bytes,
            manifest.checksum_crc32,
        )?;

        if entry.is_complete() {
            let completed = self
                .incoming_groups
                .remove(&group.group_id)
                .expect("complete incoming group should still be present");
            if !completed.valid_checksum() {
                self.clipboard_groups.remove(&group.group_id);
                return Err(FileTransferError::InvalidChunk {
                    reason: "file group checksum or size mismatch",
                });
            }
            let paths = completed.paths();
            if let Some(ownership) = self.clipboard_groups.remove(&group.group_id) {
                if self.clipboard_owner.is_none()
                    || ownership.as_ref().is_some_and(|token| token.current())
                {
                    emit_event(
                        event_tx,
                        FileTransferEvent::IncomingClipboardReady {
                            ownership,
                            group_id: group.group_id,
                            paths: paths.clone(),
                        },
                    );
                }
            }
            emit_event(
                event_tx,
                FileTransferEvent::IncomingGroupCompleted {
                    group_id: group.group_id,
                    paths,
                    total_size_bytes: group.group_total_size_bytes,
                },
            );
        }

        Ok(())
    }

    fn is_manifest_cancelled(&self, manifest: &protocol::FileTransferManifest) -> bool {
        self.cancelled_file_objects
            .contains(&manifest.file_object_id)
            || manifest
                .group
                .as_ref()
                .is_some_and(|group| self.cancelled_groups.contains(&group.group_id))
    }

    async fn cancel_incoming_file(
        &mut self,
        transfer_id: u64,
        file_object_id: u64,
        event_tx: &mpsc::UnboundedSender<FileTransferEvent>,
    ) -> Result<(), FileTransferError> {
        let Some(incoming) = self.incoming_files.remove(&file_object_id) else {
            return Ok(());
        };
        let group = incoming.manifest().group.clone();
        let path = incoming.cancel().await?;
        cleanup_empty_parent_dirs(&path, &self.receive_dir).await?;
        emit_event(
            event_tx,
            FileTransferEvent::IncomingCancelled {
                transfer_id,
                file_object_id,
                group,
                path,
            },
        );
        Ok(())
    }

    async fn cancel_incoming_group(
        &mut self,
        group_id: u64,
        event_tx: &mpsc::UnboundedSender<FileTransferEvent>,
    ) -> Result<(), FileTransferError> {
        let mut file_object_ids = self
            .incoming_files
            .iter()
            .filter_map(|(file_object_id, incoming)| {
                incoming
                    .manifest()
                    .group
                    .as_ref()
                    .filter(|group| group.group_id == group_id)
                    .map(|_| *file_object_id)
            })
            .collect::<Vec<_>>();
        file_object_ids.sort_unstable();

        for file_object_id in file_object_ids {
            if let Some(incoming) = self.incoming_files.remove(&file_object_id) {
                let transfer_id = incoming.manifest().transfer_id;
                let group = incoming.manifest().group.clone();
                let path = incoming.cancel().await?;
                cleanup_empty_parent_dirs(&path, &self.receive_dir).await?;
                emit_event(
                    event_tx,
                    FileTransferEvent::IncomingCancelled {
                        transfer_id,
                        file_object_id,
                        group,
                        path,
                    },
                );
            }
        }

        if let Some(group) = self.incoming_groups.remove(&group_id) {
            for path in group.completed_paths() {
                remove_materialized_file(&path, &self.receive_dir).await?;
            }
        }

        emit_event(
            event_tx,
            FileTransferEvent::IncomingGroupCancelled { group_id },
        );
        Ok(())
    }
}

struct IncomingFileGroup {
    total_size: u64,
    checksum: u32,
    file_count: u32,
    completed_files: Vec<Option<PathBuf>>,
    completed_metadata: Vec<Option<(String, u64, u32)>>,
    publish_paths: Vec<PathBuf>,
    completed_count: u32,
}

impl IncomingFileGroup {
    fn new(group: &FileTransferGroup) -> Self {
        Self {
            total_size: group.group_total_size_bytes,
            checksum: group.group_checksum_crc32,
            file_count: group.file_count,
            completed_files: vec![None; group.file_count as usize],
            completed_metadata: vec![None; group.file_count as usize],
            publish_paths: Vec::new(),
            completed_count: 0,
        }
    }

    fn record(
        &mut self,
        group: &FileTransferGroup,
        path: PathBuf,
        publish_path: PathBuf,
        size: u64,
        checksum: u32,
    ) -> Result<(), FileTransferError> {
        if group.file_count != self.file_count
            || group.file_index >= self.file_count
            || group.group_total_size_bytes != self.total_size
            || group.group_checksum_crc32 != self.checksum
        {
            return Err(FileTransferError::InvalidChunk {
                reason: "file group completion metadata is inconsistent",
            });
        }
        let slot = &mut self.completed_files[group.file_index as usize];
        if slot.is_none() {
            self.completed_count = self.completed_count.saturating_add(1);
            if !self.publish_paths.contains(&publish_path) {
                self.publish_paths.push(publish_path);
            }
        }
        *slot = Some(path);
        self.completed_metadata[group.file_index as usize] =
            Some((group.relative_path.clone(), size, checksum));
        Ok(())
    }

    fn valid_checksum(&self) -> bool {
        let mut bytes = Vec::new();
        let mut total = 0u64;
        for metadata in &self.completed_metadata {
            let Some((path, size, checksum)) = metadata else {
                return false;
            };
            let Some(next_total) = total.checked_add(*size) else {
                return false;
            };
            total = next_total;
            bytes.extend_from_slice(path.as_bytes());
            bytes.push(0);
            bytes.extend_from_slice(&size.to_be_bytes());
            bytes.extend_from_slice(&checksum.to_be_bytes());
        }
        total == self.total_size && crate::data_plane::checksum_crc32(&bytes) == self.checksum
    }

    fn is_complete(&self) -> bool {
        self.completed_count == self.file_count
    }

    fn paths(self) -> Vec<PathBuf> {
        self.publish_paths
    }

    fn completed_paths(self) -> Vec<PathBuf> {
        self.completed_files.into_iter().flatten().collect()
    }
}

async fn remove_materialized_file(
    path: &Path,
    receive_dir: &Path,
) -> Result<(), FileTransferError> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(FileTransferError::Io(err)),
    }
    cleanup_empty_parent_dirs(path, receive_dir).await
}

async fn cleanup_empty_parent_dirs(
    path: &Path,
    receive_dir: &Path,
) -> Result<(), FileTransferError> {
    let mut parent = path.parent();
    while let Some(dir) = parent {
        if dir == receive_dir {
            break;
        }
        match tokio::fs::remove_dir(dir).await {
            Ok(()) => parent = dir.parent(),
            Err(err)
                if err.kind() == std::io::ErrorKind::NotFound
                    || err.kind() == std::io::ErrorKind::DirectoryNotEmpty =>
            {
                break;
            }
            Err(err) => return Err(FileTransferError::Io(err)),
        }
    }
    Ok(())
}

fn received_group_publish_path(
    receive_dir: &Path,
    relative_path: &str,
    actual: &Path,
) -> Result<PathBuf, FileTransferError> {
    let original = group_publish_path(receive_dir, relative_path)?;
    // Top-level files may have been numbered; publish the materialized path,
    // never an existing namesake. Directory-group roots remain the same.
    Ok(if Path::new(relative_path).components().count() == 1 {
        actual.to_path_buf()
    } else {
        original
    })
}

fn group_publish_path(
    receive_dir: &Path,
    relative_path: &str,
) -> Result<PathBuf, FileTransferError> {
    let mut components = Path::new(relative_path).components();
    let Some(Component::Normal(first)) = components.next() else {
        return Err(FileTransferError::InvalidFileName);
    };
    if components.any(|component| !matches!(component, Component::Normal(_))) {
        return Err(FileTransferError::InvalidFileName);
    }
    Ok(receive_dir.join(first))
}

fn emit_event(event_tx: &mpsc::UnboundedSender<FileTransferEvent>, event: FileTransferEvent) {
    let _ = event_tx.send(event);
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_plane::LaneSchedulerConfig;
    use crate::media_plane::rtp_to_realtime_data;
    use crate::net::{MultiplexedPacket, UdpMultiplexer};
    use crate::scheduled_sender::{ScheduledDataSender, ScheduledDataSenderConfig};
    use protocol::{PayloadType, RtpHeader, RtpPacket};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use tokio::time::timeout;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ObservedDataPacket {
        Media(ContentKind),
        File(ContentKind),
        Other(ContentKind),
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn media_packet(payload_type: PayloadType, sequence_number: u16, ssrc: u32) -> RtpPacket {
        let timestamp = if payload_type == PayloadType::VideoH265 {
            now_ms().saturating_add(1_000) as u32
        } else {
            sequence_number as u32 * 960
        };

        RtpPacket {
            header: RtpHeader {
                version: 2,
                payload_type: payload_type as u8,
                sequence_number,
                timestamp,
                ssrc,
            },
            payload: vec![sequence_number as u8; 64],
        }
    }

    #[tokio::test]
    async fn clipboard_group_keeps_admission_generation_and_queued_send_checks_it() {
        use crate::clipboard_file_runtime::ClipboardOwnership;
        use std::sync::atomic::Ordering;
        let a = Arc::new(AtomicBool::new(true));
        let slot = Arc::new(Mutex::new(Some(ClipboardOwnership(a.clone()))));
        let mut inbound =
            InboundFileTransfers::new(PathBuf::from("/tmp"), FileReceivePolicy::default());
        inbound.clipboard_owner = Some(slot.clone());
        let (events, _rx) = mpsc::unbounded_channel();
        let marker = control_to_envelope(
            &FileTransferControl::ClipboardGroup { group_id: 42 },
            0,
            2,
            0,
            0,
        )
        .unwrap();
        inbound
            .handle_control(marker.clone(), &events)
            .await
            .unwrap();
        a.store(false, Ordering::Release);
        let replacement = Arc::new(AtomicBool::new(true));
        *slot.lock().unwrap() = Some(ClipboardOwnership(replacement));
        inbound.handle_control(marker, &events).await.unwrap();
        let admitted = inbound.clipboard_groups.get(&42).unwrap().as_ref().unwrap();
        assert!(Arc::ptr_eq(&admitted.0, &a));
        let command = FileTransferCommand::SendOwnedClipboardFiles {
            files: vec![],
            admission: crate::clipboard_file_runtime::ClipboardFileAdmission::new(admitted.clone()),
        };
        assert!(!clipboard_command_admitted(&command));
        assert!(clipboard_command_admitted(
            &FileTransferCommand::SendFileGroup { files: vec![] }
        ));
    }

    #[test]
    fn numbered_group_file_publishes_actual_path_and_keeps_directory_roots() {
        let root = Path::new("/received");
        assert_eq!(
            received_group_publish_path(root, "a.txt", Path::new("/received/a (1).txt")).unwrap(),
            PathBuf::from("/received/a (1).txt")
        );
        assert_eq!(
            received_group_publish_path(
                root,
                "Project/a.txt",
                Path::new("/received/Project/a (1).txt")
            )
            .unwrap(),
            PathBuf::from("/received/Project")
        );
        assert!(received_group_publish_path(root, "../escape", Path::new("/escape")).is_err());
    }

    #[test]
    fn inbound_receive_config_applies_immediately_when_idle() {
        let mut inbound =
            InboundFileTransfers::new(PathBuf::from("/tmp/old"), FileReceivePolicy::default());
        let next_policy = FileReceivePolicy {
            allow_overwrite: true,
            ..FileReceivePolicy::default()
        };

        inbound.set_receive_config(PathBuf::from("/tmp/new"), next_policy);

        assert_eq!(inbound.receive_dir, PathBuf::from("/tmp/new"));
        assert_eq!(inbound.receive_policy, next_policy);
        assert_eq!(inbound.pending_receive_config, None);
    }

    #[test]
    fn inbound_receive_config_waits_for_active_group_boundary() {
        let mut inbound =
            InboundFileTransfers::new(PathBuf::from("/tmp/old"), FileReceivePolicy::default());
        let group = FileTransferGroup {
            group_id: 9,
            file_index: 0,
            file_count: 2,
            relative_path: "Folder/a.txt".to_string(),
            group_total_size_bytes: 10,
            group_checksum_crc32: 0,
        };
        inbound
            .incoming_groups
            .insert(group.group_id, IncomingFileGroup::new(&group));
        let next_policy = FileReceivePolicy {
            allow_overwrite: true,
            ..FileReceivePolicy::default()
        };

        inbound.set_receive_config(PathBuf::from("/tmp/new"), next_policy);

        assert_eq!(inbound.receive_dir, PathBuf::from("/tmp/old"));
        assert_eq!(inbound.receive_policy, FileReceivePolicy::default());
        assert_eq!(
            inbound.pending_receive_config,
            Some((PathBuf::from("/tmp/new"), next_policy))
        );

        inbound.incoming_groups.clear();
        inbound.apply_pending_receive_config_if_idle();

        assert_eq!(inbound.receive_dir, PathBuf::from("/tmp/new"));
        assert_eq!(inbound.receive_policy, next_policy);
        assert_eq!(inbound.pending_receive_config, None);
    }

    #[tokio::test]
    async fn runtime_sends_file_over_scheduled_udp_path_and_receiver_materializes_it() {
        udp_file_roundtrip(false).await;
    }

    #[tokio::test]
    async fn retransmits_lost_manifest_data_and_final_ack_without_duplicate_completion() {
        udp_file_roundtrip(true).await;
    }

    async fn udp_file_roundtrip(inject_faults: bool) {
        let base = unique_temp_dir("weavelo-file-runtime");
        let source_dir = base.join("source");
        let receive_dir = base.join("received");
        tokio::fs::create_dir_all(&source_dir)
            .await
            .expect("source dir should be created");
        let source = source_dir.join("runtime.bin");
        let bytes: Vec<u8> = (0..33_000).map(|index| (index % 229) as u8).collect();
        tokio::fs::write(&source, &bytes)
            .await
            .expect("source file should be written");

        tokio::fs::create_dir_all(&receive_dir).await.unwrap();
        let retained = receive_dir.join("runtime.bin");
        tokio::fs::write(&retained, b"original file must survive retry")
            .await
            .unwrap();

        let source_mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("source bind should succeed");
        let (source_sender, source_udp_receiver) = source_mux.split();
        let target_mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("target bind should succeed");
        let target_addr = target_mux.local_addr().expect("target addr should exist");
        let (target_udp_sender, target_udp_receiver) = target_mux.split();

        let (source_scheduled_sender, _source_worker) = ScheduledDataSender::spawn(
            source_sender,
            target_addr,
            ScheduledDataSenderConfig {
                ..ScheduledDataSenderConfig::default()
            },
        );
        let (target_scheduled_sender, _target_worker) = ScheduledDataSender::spawn(
            target_udp_sender,
            source_mux.local_addr().expect("source addr should exist"),
            ScheduledDataSenderConfig::default(),
        );

        let (source_command_tx, source_command_rx) = mpsc::channel(8);
        let (source_inbound_tx, source_inbound_rx) = mpsc::channel(8);
        let (source_event_tx, mut source_event_rx) = mpsc::unbounded_channel();
        let (target_command_tx, target_command_rx) = mpsc::channel(8);
        let (target_inbound_tx, target_inbound_rx) = mpsc::channel(128);
        let (target_event_tx, mut target_event_rx) = mpsc::unbounded_channel();
        let (cancel_tx, _) = broadcast::channel(4);
        let source_active = Arc::new(AtomicBool::new(false));
        let target_active = Arc::new(AtomicBool::new(false));

        let source_runtime = tokio::spawn(run_file_transfer_runtime(
            source_scheduled_sender,
            source_command_rx,
            source_inbound_rx,
            source_event_tx,
            cancel_tx.subscribe(),
            FileTransferRuntimeConfig {
                chunk_payload_len: 4096,
                active: source_active.clone(),
                ..FileTransferRuntimeConfig::default()
            },
        ));
        let target_runtime = tokio::spawn(run_file_transfer_runtime(
            target_scheduled_sender,
            target_command_rx,
            target_inbound_rx,
            target_event_tx,
            cancel_tx.subscribe(),
            FileTransferRuntimeConfig {
                chunk_payload_len: 4096,
                receive_dir: receive_dir.clone(),
                active: target_active.clone(),
                receive_policy: FileReceivePolicy::default(),
                ..FileTransferRuntimeConfig::default()
            },
        ));

        let reply_router = tokio::spawn(async move {
            let mut lost_final_ack = false;
            while let Ok(packet) = source_udp_receiver.recv().await {
                if let MultiplexedPacket::Data(envelope, _) = packet {
                    if inject_faults
                        && !lost_final_ack
                        && matches!(
                            control_from_envelope(&envelope),
                            Ok(FileTransferControl::ChunkAccepted { chunk_index: 8, .. })
                        )
                    {
                        lost_final_ack = true;
                        continue;
                    }
                    if source_inbound_tx.send(envelope).await.is_err() {
                        break;
                    }
                }
            }
        });
        let udp_router = tokio::spawn(async move {
            let mut draining_receiver = false;
            let mut lost_manifest = false;
            let mut lost_chunk = false;
            loop {
                let packet = match target_udp_receiver.recv().await {
                    Ok(packet) => packet,
                    Err(_) => break,
                };
                if let MultiplexedPacket::Data(envelope, _) = packet {
                    if inject_faults
                        && !lost_manifest
                        && envelope.header.kind == ContentKind::FileManifest
                    {
                        lost_manifest = true;
                        continue;
                    }
                    if inject_faults
                        && !lost_chunk
                        && envelope.header.kind == ContentKind::FileChunk
                        && envelope
                            .header
                            .chunk
                            .as_ref()
                            .is_some_and(|chunk| chunk.chunk_index == 2)
                    {
                        lost_chunk = true;
                        continue;
                    }
                    if target_inbound_tx.send(envelope.clone()).await.is_err() {
                        break;
                    }
                    if !draining_receiver && envelope.header.kind == ContentKind::FileManifest {
                        target_command_tx
                            .send(FileTransferCommand::Drain)
                            .await
                            .unwrap();
                        draining_receiver = true;
                    }
                    if inject_faults
                        && !(envelope.header.kind == ContentKind::FileChunk
                            && envelope
                                .header
                                .chunk
                                .as_ref()
                                .is_some_and(|chunk| chunk.chunk_index == 2))
                    {
                        let _ = target_inbound_tx.send(envelope).await;
                    }
                }
            }
        });

        source_command_tx
            .send(FileTransferCommand::SendFile {
                path: source.clone(),
                mime_type: Some("application/octet-stream".to_string()),
            })
            .await
            .expect("send command should queue");
        // Closing a media view immediately after queuing a file must retain
        // both the queued command and its retransmissions until completion.
        source_command_tx
            .send(FileTransferCommand::Drain)
            .await
            .unwrap();

        let mut outgoing_completed = false;
        while !outgoing_completed {
            let event = timeout(Duration::from_secs(2), source_event_rx.recv())
                .await
                .expect("source event should arrive")
                .expect("source event channel should remain open");
            if inject_faults && matches!(event, FileTransferEvent::OutgoingStarted { .. }) {
                assert!(
                    source_active.load(Relaxed),
                    "retrying transfer must retain the connection"
                );
            }
            if matches!(event, FileTransferEvent::OutgoingCompleted { .. }) {
                outgoing_completed = true;
            }
        }

        assert!(matches!(
            timeout(Duration::from_secs(2), source_event_rx.recv())
                .await
                .unwrap(),
            Some(FileTransferEvent::Drained)
        ));

        let received_path = loop {
            let event = timeout(Duration::from_secs(2), target_event_rx.recv())
                .await
                .expect("target event should arrive")
                .expect("target event channel should remain open");
            if let FileTransferEvent::IncomingCompleted { path, .. } = event {
                break path;
            }
        };

        while let Ok(event) = target_event_rx.try_recv() {
            assert!(!matches!(
                event,
                FileTransferEvent::IncomingCompleted { .. }
            ));
        }
        timeout(Duration::from_secs(2), async {
            while source_active.load(Relaxed) || target_active.load(Relaxed) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("finished transfers must release idle connection retention");
        let received_bytes = tokio::fs::read(&received_path)
            .await
            .expect("received file should read");
        assert_eq!(received_bytes, bytes);
        assert_eq!(received_path, receive_dir.join("runtime (1).bin"));
        assert_eq!(
            tokio::fs::read(&retained).await.unwrap(),
            b"original file must survive retry"
        );
        assert_eq!(
            std::fs::read_dir(&receive_dir).unwrap().count(),
            2,
            "Lost final acknowledgement and duplicate data must not create another numbered copy"
        );

        assert!(
            !target_runtime.is_finished(),
            "Receiver must retain final-ACK retries after its view closes"
        );
        let _ = cancel_tx.send(());
        udp_router.abort();
        reply_router.abort();
        source_runtime
            .await
            .expect("source runtime should join")
            .expect("source runtime should stop cleanly");
        target_runtime
            .await
            .expect("target runtime should join")
            .expect("target runtime should stop cleanly");
        let _ = tokio::fs::remove_dir_all(&base).await;
    }

    #[tokio::test]
    async fn shared_sender_prioritizes_media_while_file_transfer_completes() {
        let base = unique_temp_dir("weavelo-file-runtime-media-mix");
        let source_dir = base.join("source");
        let receive_dir = base.join("received");
        tokio::fs::create_dir_all(&source_dir)
            .await
            .expect("source dir should be created");
        let source = source_dir.join("mixed.bin");
        let bytes: Vec<u8> = (0..4096).map(|index| (index % 251) as u8).collect();
        tokio::fs::write(&source, &bytes)
            .await
            .expect("source file should be written");

        let source_mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("source bind should succeed");
        let (source_sender, source_udp_receiver) = source_mux.split();
        let target_mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("target bind should succeed");
        let target_addr = target_mux.local_addr().expect("target addr should exist");
        let (target_udp_sender, target_udp_receiver) = target_mux.split();

        let (source_scheduled_sender, _source_worker) = ScheduledDataSender::spawn(
            source_sender,
            target_addr,
            ScheduledDataSenderConfig {
                scheduler: LaneSchedulerConfig {
                    max_realtime_queued: 8,
                    max_reliable_queued: 64,
                },
                realtime_queue_capacity: 128,
                reliable_queue_capacity: 128,
            },
        );
        let (target_scheduled_sender, _target_worker) = ScheduledDataSender::spawn(
            target_udp_sender,
            source_mux.local_addr().expect("source addr should exist"),
            ScheduledDataSenderConfig::default(),
        );

        let (source_command_tx, source_command_rx) = mpsc::channel(8);
        let (source_inbound_tx, source_inbound_rx) = mpsc::channel(8);
        let (source_event_tx, mut source_event_rx) = mpsc::unbounded_channel();
        let (_target_command_tx, target_command_rx) = mpsc::channel(8);
        let (target_inbound_tx, target_inbound_rx) = mpsc::channel(128);
        let (target_event_tx, mut target_event_rx) = mpsc::unbounded_channel();
        let (observed_tx, mut observed_rx) = mpsc::unbounded_channel::<ObservedDataPacket>();
        let (cancel_tx, _) = broadcast::channel(4);

        let source_runtime = tokio::spawn(run_file_transfer_runtime(
            source_scheduled_sender.clone(),
            source_command_rx,
            source_inbound_rx,
            source_event_tx,
            cancel_tx.subscribe(),
            FileTransferRuntimeConfig {
                chunk_payload_len: 1024,
                ..FileTransferRuntimeConfig::default()
            },
        ));
        let target_runtime = tokio::spawn(run_file_transfer_runtime(
            target_scheduled_sender,
            target_command_rx,
            target_inbound_rx,
            target_event_tx,
            cancel_tx.subscribe(),
            FileTransferRuntimeConfig {
                chunk_payload_len: 1024,
                receive_dir: receive_dir.clone(),
                receive_policy: FileReceivePolicy {
                    allow_overwrite: true,
                    ..FileReceivePolicy::default()
                },
                ..FileTransferRuntimeConfig::default()
            },
        ));

        let reply_router = tokio::spawn(async move {
            while let Ok(packet) = source_udp_receiver.recv().await {
                if let MultiplexedPacket::Data(envelope, _) = packet
                    && source_inbound_tx.send(envelope).await.is_err()
                {
                    break;
                }
            }
        });
        let udp_router = tokio::spawn(async move {
            loop {
                let packet = match target_udp_receiver.recv().await {
                    Ok(packet) => packet,
                    Err(_) => break,
                };
                let MultiplexedPacket::Data(envelope, _) = packet else {
                    continue;
                };

                let kind = envelope.header.kind;
                let observed = if matches!(envelope.header.lane, protocol::DataLane::Realtime) {
                    ObservedDataPacket::Media(kind)
                } else if matches!(
                    kind,
                    ContentKind::FileManifest | ContentKind::FileChunk | ContentKind::FileControl
                ) {
                    ObservedDataPacket::File(kind)
                } else {
                    ObservedDataPacket::Other(kind)
                };
                let _ = observed_tx.send(observed);

                if matches!(
                    kind,
                    ContentKind::FileManifest | ContentKind::FileChunk | ContentKind::FileControl
                ) && target_inbound_tx.send(envelope).await.is_err()
                {
                    break;
                }
            }
        });

        // Realtime traffic is event-driven and must not wait for a batching tick.
        // Queue it before starting bulk transfer; the lower-level scheduler tests
        // separately cover preemption when both classes are already pending.
        let video = media_packet(PayloadType::VideoH265, 77, 900);
        let audio = media_packet(PayloadType::AudioOpus, 78, 901);
        source_scheduled_sender
            .try_send(rtp_to_realtime_data(&video).expect("video should adapt"))
            .expect("video should queue in the shared sender");
        source_scheduled_sender
            .try_send(rtp_to_realtime_data(&audio).expect("audio should adapt"))
            .expect("audio should queue in the shared sender");

        source_command_tx
            .send(FileTransferCommand::SendFile {
                path: source.clone(),
                mime_type: Some("application/octet-stream".to_string()),
            })
            .await
            .expect("send command should queue");

        let first = timeout(Duration::from_secs(2), observed_rx.recv())
            .await
            .expect("first packet should arrive")
            .expect("observed channel should remain open");
        let second = timeout(Duration::from_secs(2), observed_rx.recv())
            .await
            .expect("second packet should arrive")
            .expect("observed channel should remain open");

        assert_eq!(first, ObservedDataPacket::Media(ContentKind::VideoH265));
        assert_eq!(second, ObservedDataPacket::Media(ContentKind::AudioOpus));

        let mut outgoing_completed = false;
        while !outgoing_completed {
            let event = timeout(Duration::from_secs(5), source_event_rx.recv())
                .await
                .expect("source event should arrive")
                .expect("source event channel should remain open");
            if matches!(event, FileTransferEvent::OutgoingCompleted { .. }) {
                outgoing_completed = true;
            }
        }

        let received_path = loop {
            let event = timeout(Duration::from_secs(5), target_event_rx.recv())
                .await
                .expect("target event should arrive")
                .expect("target event channel should remain open");
            if let FileTransferEvent::IncomingCompleted { path, .. } = event {
                break path;
            }
        };
        let received_bytes = tokio::fs::read(&received_path)
            .await
            .expect("received file should read");
        assert_eq!(received_bytes, bytes);

        let stats = source_scheduled_sender.stats();
        assert_eq!(stats.sent_realtime, 2);
        assert!(stats.sent_reliable >= 2);
        assert_eq!(stats.scheduler_dropped_stale_realtime, 0);
        assert_eq!(stats.scheduler_dropped_realtime_capacity, 0);
        assert_eq!(stats.send_errors, 0);

        let _ = cancel_tx.send(());
        udp_router.abort();
        reply_router.abort();
        source_runtime
            .await
            .expect("source runtime should join")
            .expect("source runtime should stop cleanly");
        target_runtime
            .await
            .expect("target runtime should join")
            .expect("target runtime should stop cleanly");
        let _ = tokio::fs::remove_dir_all(&base).await;
    }

    #[tokio::test]
    async fn runtime_publishes_group_completion_after_all_files_materialize() {
        let base = unique_temp_dir("weavelo-file-runtime-group");
        let source_dir = base.join("source");
        let receive_dir = base.join("received");
        tokio::fs::create_dir_all(&source_dir)
            .await
            .expect("source dir should be created");
        let first = source_dir.join("first.txt");
        let second = source_dir.join("second.txt");
        tokio::fs::write(&first, b"first")
            .await
            .expect("first file should write");
        tokio::fs::write(&second, b"second file")
            .await
            .expect("second file should write");

        let source_mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("source bind should succeed");
        let (source_sender, source_udp_receiver) = source_mux.split();
        let target_mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("target bind should succeed");
        let target_addr = target_mux.local_addr().expect("target addr should exist");
        let (target_udp_sender, target_udp_receiver) = target_mux.split();

        let (source_scheduled_sender, _source_worker) = ScheduledDataSender::spawn(
            source_sender,
            target_addr,
            ScheduledDataSenderConfig {
                ..ScheduledDataSenderConfig::default()
            },
        );
        let (target_scheduled_sender, _target_worker) = ScheduledDataSender::spawn(
            target_udp_sender,
            source_mux.local_addr().expect("source addr should exist"),
            ScheduledDataSenderConfig::default(),
        );

        let (source_command_tx, source_command_rx) = mpsc::channel(8);
        let (source_inbound_tx, source_inbound_rx) = mpsc::channel(8);
        let (source_event_tx, mut source_event_rx) = mpsc::unbounded_channel();
        let (_target_command_tx, target_command_rx) = mpsc::channel(8);
        let (target_inbound_tx, target_inbound_rx) = mpsc::channel(128);
        let (target_event_tx, mut target_event_rx) = mpsc::unbounded_channel();
        let (cancel_tx, _) = broadcast::channel(4);

        let source_runtime = tokio::spawn(run_file_transfer_runtime(
            source_scheduled_sender,
            source_command_rx,
            source_inbound_rx,
            source_event_tx,
            cancel_tx.subscribe(),
            FileTransferRuntimeConfig {
                chunk_payload_len: 4,
                ..FileTransferRuntimeConfig::default()
            },
        ));
        let target_runtime = tokio::spawn(run_file_transfer_runtime(
            target_scheduled_sender,
            target_command_rx,
            target_inbound_rx,
            target_event_tx,
            cancel_tx.subscribe(),
            FileTransferRuntimeConfig {
                chunk_payload_len: 4,
                receive_dir: receive_dir.clone(),
                receive_policy: FileReceivePolicy {
                    allow_overwrite: true,
                    ..FileReceivePolicy::default()
                },
                ..FileTransferRuntimeConfig::default()
            },
        ));

        let reply_router = tokio::spawn(async move {
            while let Ok(packet) = source_udp_receiver.recv().await {
                if let MultiplexedPacket::Data(envelope, _) = packet
                    && source_inbound_tx.send(envelope).await.is_err()
                {
                    break;
                }
            }
        });
        let udp_router = tokio::spawn(async move {
            loop {
                let packet = match target_udp_receiver.recv().await {
                    Ok(packet) => packet,
                    Err(_) => break,
                };
                if let MultiplexedPacket::Data(envelope, _) = packet
                    && target_inbound_tx.send(envelope).await.is_err()
                {
                    break;
                }
            }
        });

        source_command_tx
            .send(FileTransferCommand::SendFileGroup {
                files: vec![
                    FileTransferGroupFile {
                        path: first.clone(),
                        mime_type: Some("text/plain".to_string()),
                    },
                    FileTransferGroupFile {
                        path: second.clone(),
                        mime_type: Some("text/plain".to_string()),
                    },
                ],
            })
            .await
            .expect("send group command should queue");

        let mut outgoing_group_completed = false;
        while !outgoing_group_completed {
            let event = timeout(Duration::from_secs(2), source_event_rx.recv())
                .await
                .expect("source event should arrive")
                .expect("source event channel should remain open");
            if matches!(event, FileTransferEvent::OutgoingGroupCompleted { .. }) {
                outgoing_group_completed = true;
            }
        }

        let group_paths = loop {
            let event = timeout(Duration::from_secs(2), target_event_rx.recv())
                .await
                .expect("target event should arrive")
                .expect("target event channel should remain open");
            if let FileTransferEvent::IncomingGroupCompleted { paths, .. } = event {
                break paths;
            }
        };

        assert_eq!(group_paths.len(), 2);
        assert_eq!(
            tokio::fs::read(&group_paths[0])
                .await
                .expect("first received file should read"),
            b"first"
        );
        assert_eq!(
            tokio::fs::read(&group_paths[1])
                .await
                .expect("second received file should read"),
            b"second file"
        );

        let _ = cancel_tx.send(());
        udp_router.abort();
        reply_router.abort();
        source_runtime
            .await
            .expect("source runtime should join")
            .expect("source runtime should stop cleanly");
        target_runtime
            .await
            .expect("target runtime should join")
            .expect("target runtime should stop cleanly");
        let _ = tokio::fs::remove_dir_all(&base).await;
    }

    #[tokio::test]
    async fn runtime_expands_directory_groups_and_publishes_top_level_path() {
        let base = unique_temp_dir("weavelo-file-runtime-directory-group");
        let source_dir = base.join("source");
        let receive_dir = base.join("received");
        let project_dir = source_dir.join("Project");
        let nested_dir = project_dir.join("nested");
        tokio::fs::create_dir_all(&nested_dir)
            .await
            .expect("nested source dir should be created");
        tokio::fs::write(project_dir.join("root.txt"), b"root file")
            .await
            .expect("root file should write");
        tokio::fs::write(nested_dir.join("child.txt"), b"child file")
            .await
            .expect("child file should write");

        let source_mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("source bind should succeed");
        let (source_sender, source_udp_receiver) = source_mux.split();
        let target_mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("target bind should succeed");
        let target_addr = target_mux.local_addr().expect("target addr should exist");
        let (target_udp_sender, target_udp_receiver) = target_mux.split();

        let (source_scheduled_sender, _source_worker) = ScheduledDataSender::spawn(
            source_sender,
            target_addr,
            ScheduledDataSenderConfig {
                ..ScheduledDataSenderConfig::default()
            },
        );
        let (target_scheduled_sender, _target_worker) = ScheduledDataSender::spawn(
            target_udp_sender,
            source_mux.local_addr().expect("source addr should exist"),
            ScheduledDataSenderConfig::default(),
        );

        let (source_command_tx, source_command_rx) = mpsc::channel(8);
        let (source_inbound_tx, source_inbound_rx) = mpsc::channel(8);
        let (source_event_tx, mut source_event_rx) = mpsc::unbounded_channel();
        let (_target_command_tx, target_command_rx) = mpsc::channel(8);
        let (target_inbound_tx, target_inbound_rx) = mpsc::channel(128);
        let (target_event_tx, mut target_event_rx) = mpsc::unbounded_channel();
        let (cancel_tx, _) = broadcast::channel(4);

        let source_runtime = tokio::spawn(run_file_transfer_runtime(
            source_scheduled_sender,
            source_command_rx,
            source_inbound_rx,
            source_event_tx,
            cancel_tx.subscribe(),
            FileTransferRuntimeConfig {
                chunk_payload_len: 4,
                ..FileTransferRuntimeConfig::default()
            },
        ));
        let target_runtime = tokio::spawn(run_file_transfer_runtime(
            target_scheduled_sender,
            target_command_rx,
            target_inbound_rx,
            target_event_tx,
            cancel_tx.subscribe(),
            FileTransferRuntimeConfig {
                chunk_payload_len: 4,
                receive_dir: receive_dir.clone(),
                receive_policy: FileReceivePolicy {
                    allow_overwrite: true,
                    ..FileReceivePolicy::default()
                },
                ..FileTransferRuntimeConfig::default()
            },
        ));

        let reply_router = tokio::spawn(async move {
            while let Ok(packet) = source_udp_receiver.recv().await {
                if let MultiplexedPacket::Data(envelope, _) = packet
                    && source_inbound_tx.send(envelope).await.is_err()
                {
                    break;
                }
            }
        });
        let udp_router = tokio::spawn(async move {
            loop {
                let packet = match target_udp_receiver.recv().await {
                    Ok(packet) => packet,
                    Err(_) => break,
                };
                if let MultiplexedPacket::Data(envelope, _) = packet
                    && target_inbound_tx.send(envelope).await.is_err()
                {
                    break;
                }
            }
        });

        source_command_tx
            .send(FileTransferCommand::SendFileGroup {
                files: vec![FileTransferGroupFile {
                    path: project_dir.clone(),
                    mime_type: None,
                }],
            })
            .await
            .expect("send directory group command should queue");

        let mut outgoing_file_count = None;
        while outgoing_file_count.is_none() {
            let event = timeout(Duration::from_secs(2), source_event_rx.recv())
                .await
                .expect("source event should arrive")
                .expect("source event channel should remain open");
            if let FileTransferEvent::OutgoingGroupStarted { file_count, .. } = event {
                outgoing_file_count = Some(file_count);
            }
        }
        assert_eq!(outgoing_file_count, Some(2));

        let group_paths = loop {
            let event = timeout(Duration::from_secs(2), target_event_rx.recv())
                .await
                .expect("target event should arrive")
                .expect("target event channel should remain open");
            if let FileTransferEvent::IncomingGroupCompleted { paths, .. } = event {
                break paths;
            }
        };

        assert_eq!(group_paths, vec![receive_dir.join("Project")]);
        assert_eq!(
            tokio::fs::read(receive_dir.join("Project/root.txt"))
                .await
                .expect("received root file should read"),
            b"root file"
        );
        assert_eq!(
            tokio::fs::read(receive_dir.join("Project/nested/child.txt"))
                .await
                .expect("received nested file should read"),
            b"child file"
        );

        let _ = cancel_tx.send(());
        udp_router.abort();
        reply_router.abort();
        source_runtime
            .await
            .expect("source runtime should join")
            .expect("source runtime should stop cleanly");
        target_runtime
            .await
            .expect("target runtime should join")
            .expect("target runtime should stop cleanly");
        let _ = tokio::fs::remove_dir_all(&base).await;
    }

    #[tokio::test]
    async fn runtime_cleans_partial_file_on_remote_cancel_transfer() {
        let base = unique_temp_dir("weavelo-file-runtime-cancel-file");
        let receive_dir = base.join("received");
        let mut inbound = InboundFileTransfers::new(
            receive_dir.clone(),
            FileReceivePolicy {
                allow_overwrite: true,
                ..FileReceivePolicy::default()
            },
        );
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();

        let manifest = protocol::FileTransferManifest {
            transfer_id: 55,
            file_object_id: 56,
            name: "cancel.bin".to_string(),
            group: None,
            mime_type: None,
            size_bytes: 1024,
            chunk_payload_len: 512,
            checksum_crc32: 0,
        };
        let manifest_envelope =
            crate::file_transfer::manifest_to_envelope(&manifest, 54, 2, 1, 1_000)
                .expect("manifest should encode");
        inbound.handle_envelope(manifest_envelope, &event_tx).await;

        let partial_path = match event_rx.recv().await.expect("incoming started should emit") {
            FileTransferEvent::IncomingStarted { path, .. } => path,
            event => panic!("unexpected event: {event:?}"),
        };
        assert!(
            tokio::fs::metadata(&partial_path).await.is_err(),
            "uncommitted file must not be visible at its final name"
        );

        let cancel = FileTransferControl::CancelTransfer {
            transfer_id: manifest.transfer_id,
            file_object_id: manifest.file_object_id,
        };
        let cancel_envelope =
            control_to_envelope(&cancel, 90, 2, 2, 1_001).expect("cancel control should encode");
        inbound.handle_envelope(cancel_envelope, &event_tx).await;

        match event_rx.recv().await.expect("incoming cancel should emit") {
            FileTransferEvent::IncomingCancelled {
                transfer_id,
                file_object_id,
                path,
                ..
            } => {
                assert_eq!(transfer_id, manifest.transfer_id);
                assert_eq!(file_object_id, manifest.file_object_id);
                assert_eq!(path, partial_path);
            }
            event => panic!("unexpected event: {event:?}"),
        }
        assert!(
            tokio::fs::metadata(&partial_path).await.is_err(),
            "partial file should be removed after cancellation"
        );

        let _ = tokio::fs::remove_dir_all(&base).await;
    }

    #[tokio::test]
    async fn outgoing_cancel_transfer_sends_remote_control_and_stops_reader() {
        let base = unique_temp_dir("weavelo-file-runtime-outgoing-cancel");
        tokio::fs::create_dir_all(&base)
            .await
            .expect("temp dir should be created");
        let source = base.join("cancel-source.bin");
        tokio::fs::write(&source, vec![7_u8; 8192])
            .await
            .expect("source file should write");

        let mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("sender bind should succeed");
        let sender = mux.split().0;
        let receiver_mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("receiver bind should succeed");
        let receiver_addr = receiver_mux
            .local_addr()
            .expect("receiver addr should exist");
        let receiver = receiver_mux.split().1;
        let (scheduled_sender, _worker) = ScheduledDataSender::spawn(
            sender,
            receiver_addr,
            ScheduledDataSenderConfig {
                ..ScheduledDataSenderConfig::default()
            },
        );
        let config = FileTransferRuntimeConfig {
            chunk_payload_len: 1024,
            ..FileTransferRuntimeConfig::default()
        };
        let allocator = FileTransferIdAllocator::new(&config);
        let spec = FileTransferSpec {
            chunk_payload_len: 1024,
            ..FileTransferSpec::new(7, 11, 12, config.stream_id, 0, 1_000)
        };
        let mut reader = FileTransferReader::from_path(
            &source,
            spec,
            Some("application/octet-stream".to_string()),
            FileTransferPolicy::default(),
        )
        .await
        .expect("reader should create");
        let cancel_state = FileTransferCancelState::default();
        cancel_state.cancel_transfer(7);
        let (_shutdown_tx, shutdown_rx) = broadcast::channel(1);
        let mut shutdown_rx = shutdown_rx;
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();

        let outcome = send_reader(
            &mut reader,
            &scheduled_sender,
            &event_tx,
            &mut shutdown_rx,
            &allocator,
            &cancel_state,
            config.stream_id,
        )
        .await
        .expect("send reader should stop cleanly");
        assert!(matches!(outcome, SendReaderOutcome::TransferCancelled));

        assert!(matches!(
            event_rx.recv().await.expect("start event should emit"),
            FileTransferEvent::OutgoingStarted { transfer_id: 7, .. }
        ));
        assert!(matches!(
            event_rx.recv().await.expect("cancel event should emit"),
            FileTransferEvent::OutgoingCancelled {
                transfer_id: 7,
                file_object_id: 12,
                ..
            }
        ));

        let packet = timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("cancel control should arrive")
            .expect("cancel control packet should decode");
        let MultiplexedPacket::Data(envelope, _) = packet else {
            panic!("expected data packet");
        };
        assert_eq!(
            control_from_envelope(&envelope).expect("cancel control should decode"),
            FileTransferControl::CancelTransfer {
                transfer_id: 7,
                file_object_id: 12
            }
        );

        let _ = tokio::fs::remove_dir_all(&base).await;
    }

    #[tokio::test]
    async fn runtime_cancel_group_removes_completed_and_partial_files_without_publish() {
        let base = unique_temp_dir("weavelo-file-runtime-cancel-group");
        let receive_dir = base.join("received");
        let mut inbound = InboundFileTransfers::new(
            receive_dir.clone(),
            FileReceivePolicy {
                allow_overwrite: true,
                ..FileReceivePolicy::default()
            },
        );
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let group_id = 77;

        let first_manifest = protocol::FileTransferManifest {
            transfer_id: 101,
            file_object_id: 102,
            name: "first.txt".to_string(),
            group: Some(FileTransferGroup {
                group_id,
                file_index: 0,
                file_count: 2,
                relative_path: "Folder/first.txt".to_string(),
                group_total_size_bytes: 4,
                group_checksum_crc32: 0,
            }),
            mime_type: Some("text/plain".to_string()),
            size_bytes: 0,
            chunk_payload_len: 512,
            checksum_crc32: crate::data_plane::checksum_crc32(&[]),
        };
        inbound
            .handle_envelope(
                crate::file_transfer::manifest_to_envelope(&first_manifest, 100, 2, 1, 1_000)
                    .expect("first manifest should encode"),
                &event_tx,
            )
            .await;
        let _ = event_rx.recv().await.expect("first start should emit");
        let first_path = match event_rx.recv().await.expect("first complete should emit") {
            FileTransferEvent::IncomingCompleted { path, .. } => path,
            event => panic!("unexpected event: {event:?}"),
        };
        assert!(
            tokio::fs::metadata(&first_path).await.is_ok(),
            "completed group file should exist before group cancellation"
        );

        let second_manifest = protocol::FileTransferManifest {
            transfer_id: 103,
            file_object_id: 104,
            name: "second.txt".to_string(),
            group: Some(FileTransferGroup {
                group_id,
                file_index: 1,
                file_count: 2,
                relative_path: "Folder/second.txt".to_string(),
                group_total_size_bytes: 4,
                group_checksum_crc32: 0,
            }),
            mime_type: Some("text/plain".to_string()),
            size_bytes: 4,
            chunk_payload_len: 512,
            checksum_crc32: crate::data_plane::checksum_crc32(b"data"),
        };
        inbound
            .handle_envelope(
                crate::file_transfer::manifest_to_envelope(&second_manifest, 101, 2, 2, 1_001)
                    .expect("second manifest should encode"),
                &event_tx,
            )
            .await;
        let second_path = match event_rx.recv().await.expect("second start should emit") {
            FileTransferEvent::IncomingStarted { path, .. } => path,
            event => panic!("unexpected event: {event:?}"),
        };
        assert!(
            tokio::fs::metadata(&second_path).await.is_err(),
            "uncommitted group member must not be published"
        );

        inbound
            .handle_envelope(
                control_to_envelope(
                    &FileTransferControl::CancelGroup { group_id },
                    200,
                    2,
                    3,
                    1_002,
                )
                .expect("group cancel should encode"),
                &event_tx,
            )
            .await;

        assert!(matches!(
            event_rx
                .recv()
                .await
                .expect("file cancellation should emit"),
            FileTransferEvent::IncomingCancelled {
                file_object_id: 104,
                ..
            }
        ));
        assert!(matches!(
            event_rx
                .recv()
                .await
                .expect("group cancellation should emit"),
            FileTransferEvent::IncomingGroupCancelled { group_id: 77 }
        ));
        assert!(
            tokio::fs::metadata(&first_path).await.is_err(),
            "completed group file should be removed after group cancellation"
        );
        assert!(
            tokio::fs::metadata(&second_path).await.is_err(),
            "partial group file should be removed after group cancellation"
        );
        assert!(
            timeout(Duration::from_millis(20), event_rx.recv())
                .await
                .is_err(),
            "cancelled group must not emit IncomingGroupCompleted"
        );

        let _ = tokio::fs::remove_dir_all(&base).await;
    }

    #[tokio::test]
    async fn runtime_completes_zero_byte_file_after_manifest() {
        let base = unique_temp_dir("weavelo-file-runtime-empty");
        let receive_dir = base.join("received");
        tokio::fs::create_dir_all(&base)
            .await
            .expect("temp dir should be created");

        let (_command_tx, command_rx) = mpsc::channel(1);
        let (inbound_tx, inbound_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let (cancel_tx, _) = broadcast::channel(1);

        let mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("bind should succeed");
        let (scheduled_sender, _worker) = ScheduledDataSender::spawn(
            mux.split().0,
            mux.local_addr().expect("local addr should exist"),
            ScheduledDataSenderConfig::default(),
        );
        let runtime = tokio::spawn(run_file_transfer_runtime(
            scheduled_sender,
            command_rx,
            inbound_rx,
            event_tx,
            cancel_tx.subscribe(),
            FileTransferRuntimeConfig {
                receive_dir: receive_dir.clone(),
                receive_policy: FileReceivePolicy {
                    allow_overwrite: true,
                    ..FileReceivePolicy::default()
                },
                ..FileTransferRuntimeConfig::default()
            },
        ));

        let manifest = protocol::FileTransferManifest {
            transfer_id: 99,
            file_object_id: 100,
            name: "empty.txt".to_string(),
            group: None,
            mime_type: Some("text/plain".to_string()),
            size_bytes: 0,
            chunk_payload_len: 4096,
            checksum_crc32: crate::data_plane::checksum_crc32(&[]),
        };
        let envelope = crate::file_transfer::manifest_to_envelope(&manifest, 98, 2, 1, 1_000)
            .expect("manifest should encode");
        inbound_tx
            .send(envelope)
            .await
            .expect("manifest should queue");

        let mut completed_path = None;
        for _ in 0..2 {
            let event = timeout(Duration::from_secs(1), event_rx.recv())
                .await
                .expect("event should arrive")
                .expect("event channel should remain open");
            if let FileTransferEvent::IncomingCompleted { path, .. } = event {
                completed_path = Some(path);
            }
        }

        let path = completed_path.expect("empty file should complete");
        assert_eq!(
            tokio::fs::read(&path)
                .await
                .expect("empty file should read"),
            Vec::<u8>::new()
        );

        let _ = cancel_tx.send(());
        runtime
            .await
            .expect("runtime should join")
            .expect("runtime should stop cleanly");
        let _ = tokio::fs::remove_dir_all(&base).await;
    }

    #[test]
    fn runtime_config_bandwidth_budget_defaults_to_none() {
        let config = FileTransferRuntimeConfig::default();
        assert!(config.bandwidth_budget.is_none());
    }

    #[test]
    fn bandwidth_budget_independent_token_bucket_pacing_and_refill() {
        let t0 = Instant::now();
        // 100 KB/s steady rate, 20 KB burst allowance.
        let budget = FileTransferBandwidthBudget::new_internal(100_000, 20_000, false, None, t0);
        assert_eq!(budget.max_bytes_per_sec(), 100_000);
        assert_eq!(budget.burst_bytes(), 20_000);
        assert!(!budget.yield_for_video());
        assert_eq!(budget.effective_bytes_per_sec(), 100_000);

        // Zero-byte reservation is always a no-op.
        assert_eq!(budget.reserve_delay_at(0, t0), Duration::ZERO);

        // First 20 KB fits entirely in the burst bucket with zero delay.
        assert_eq!(budget.reserve_delay_at(20_000, t0), Duration::ZERO);

        // Next 10 KB at the same instant incurs 10_000 / 100_000 = 100ms delay.
        let delay = budget.reserve_delay_at(10_000, t0);
        assert!(
            delay >= Duration::from_millis(99) && delay <= Duration::from_millis(101),
            "expected ~100ms pacing delay, got {delay:?}"
        );

        // Another 10 KB at the same instant queues behind the first (200ms total deficit).
        let second_delay = budget.reserve_delay_at(10_000, t0);
        assert!(
            second_delay >= Duration::from_millis(199)
                && second_delay <= Duration::from_millis(201),
            "expected ~200ms cumulative pacing delay, got {second_delay:?}"
        );

        // After 500ms, the bucket refills (+50 KB, capped at 20 KB burst), so 20 KB is free again.
        let t1 = t0 + Duration::from_millis(500);
        assert_eq!(budget.reserve_delay_at(20_000, t1), Duration::ZERO);

        let stats = budget.stats();
        assert_eq!(stats.total_paced_bytes, 60_000);
        assert!(stats.total_wait_us >= 298_000);
        assert_eq!(stats.video_yield_events, 0);
    }

    #[test]
    fn bandwidth_budget_unlimited_when_idle_and_yields_when_video_active() {
        let t0 = Instant::now();
        let budget = FileTransferBandwidthBudget::new_internal(
            0,
            DEFAULT_FILE_TRANSFER_BURST_BYTES,
            true,
            None,
            t0,
        );

        // Idle: unlimited full speed, even for large bursts.
        assert_eq!(budget.effective_bytes_per_sec(), 0);
        assert_eq!(budget.reserve_delay_at(4 * 1024 * 1024, t0), Duration::ZERO);

        // Turn video active: drops to fallback yield rate and trims burst headroom.
        budget.set_video_active(true);
        assert!(budget.is_video_active());
        assert_eq!(
            budget.effective_bytes_per_sec(),
            DEFAULT_VIDEO_YIELD_FALLBACK_BYTES_PER_SEC
        );
        // Request 1 MiB at 2 MiB/s with <= 200 KiB yield burst -> must sleep significantly.
        let yield_delay = budget.reserve_delay_at(1024 * 1024, t0);
        assert!(
            yield_delay >= Duration::from_millis(350),
            "expected video-yield throttling delay, got {yield_delay:?}"
        );
        assert_eq!(budget.stats().video_yield_events, 1);

        // Turn video idle again: immediately returns to full-speed unlimited mode.
        budget.set_video_active(false);
        assert_eq!(budget.effective_bytes_per_sec(), 0);
        assert_eq!(budget.reserve_delay_at(4 * 1024 * 1024, t0), Duration::ZERO);
    }

    #[test]
    fn bandwidth_budget_yield_for_video_ratio_custom_rate_and_shared_clone() {
        let t0 = Instant::now();
        // Ratio-based yield: 1_000_000 B/s idle -> 200_000 B/s when video is active.
        let ratio_budget =
            FileTransferBandwidthBudget::new_internal(1_000_000, 100_000, true, None, t0);
        let shared_flag = ratio_budget.video_active_flag();
        let cloned_budget = ratio_budget.clone();

        assert_eq!(ratio_budget.effective_bytes_per_sec(), 1_000_000);
        shared_flag.store(true, Relaxed);
        assert!(cloned_budget.is_video_active());
        assert_eq!(cloned_budget.effective_bytes_per_sec(), 200_000);

        // Explicit video yield rate: 1_000_000 B/s idle -> 50_000 B/s when video is active.
        let explicit = FileTransferBandwidthBudget::new_internal(
            1_000_000,
            64_000,
            true,
            Some(50_000),
            t0,
        );
        explicit.set_video_active(true);
        assert_eq!(explicit.effective_bytes_per_sec(), 50_000);
        // Yield burst is min(64_000, 50_000 * 0.1 = 5_000) = 5_000 bytes.
        // Requesting 25_000 bytes leaves 20_000 deficit -> 400ms delay.
        let delay = explicit.reserve_delay_at(25_000, t0);
        assert!(
            delay >= Duration::from_millis(395) && delay <= Duration::from_millis(405),
            "expected ~400ms explicit yield delay, got {delay:?}"
        );

        // When yield_for_video is false, video_active does not throttle the file lane.
        let no_yield =
            FileTransferBandwidthBudget::new_internal(1_000_000, 100_000, false, Some(50_000), t0);
        no_yield.set_video_active(true);
        assert_eq!(no_yield.effective_bytes_per_sec(), 1_000_000);
        assert_eq!(no_yield.reserve_delay_at(50_000, t0), Duration::ZERO);
    }

    #[tokio::test]
    async fn runtime_transfers_file_with_bandwidth_budget_pacing() {
        let base = unique_temp_dir("weavelo-file-runtime-paced");
        let receive_dir = base.join("received");
        tokio::fs::create_dir_all(&base)
            .await
            .expect("temp dir should be created");
        let source_path = base.join("paced.bin");
        let payload: Vec<u8> = (0..12_288).map(|i| (i % 251) as u8).collect();
        tokio::fs::write(&source_path, &payload)
            .await
            .expect("source file should write");

        let source_mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("source bind should succeed");
        let target_mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("target bind should succeed");
        let source_addr = source_mux.local_addr().expect("source addr should exist");
        let target_addr = target_mux.local_addr().expect("target addr should exist");
        let (source_udp_sender, source_udp_receiver) = source_mux.split();
        let (target_udp_sender, target_udp_receiver) = target_mux.split();

        let (source_scheduled_sender, _source_worker) = ScheduledDataSender::spawn(
            source_udp_sender,
            target_addr,
            ScheduledDataSenderConfig::default(),
        );
        let (target_scheduled_sender, _target_worker) = ScheduledDataSender::spawn(
            target_udp_sender,
            source_addr,
            ScheduledDataSenderConfig::default(),
        );

        let (source_command_tx, source_command_rx) = mpsc::channel(4);
        let (source_inbound_tx, source_inbound_rx) = mpsc::channel(64);
        let (source_event_tx, mut source_event_rx) = mpsc::unbounded_channel();
        let (_target_command_tx, target_command_rx) = mpsc::channel(4);
        let (target_inbound_tx, target_inbound_rx) = mpsc::channel(64);
        let (target_event_tx, mut target_event_rx) = mpsc::unbounded_channel();
        let (cancel_tx, _) = broadcast::channel(4);

        // Configure a tight budget with video yield active so pacing actually engages
        // while still finishing quickly (~100ms).
        let budget = FileTransferBandwidthBudget::with_video_yield_rate(
            512 * 1024,
            4096,
            true,
            128 * 1024,
        );
        budget.set_video_active(true);

        let source_runtime = tokio::spawn(run_file_transfer_runtime(
            source_scheduled_sender,
            source_command_rx,
            source_inbound_rx,
            source_event_tx,
            cancel_tx.subscribe(),
            FileTransferRuntimeConfig {
                chunk_payload_len: 4096,
                bandwidth_budget: Some(budget.clone()),
                ..FileTransferRuntimeConfig::default()
            },
        ));
        let target_runtime = tokio::spawn(run_file_transfer_runtime(
            target_scheduled_sender,
            target_command_rx,
            target_inbound_rx,
            target_event_tx,
            cancel_tx.subscribe(),
            FileTransferRuntimeConfig {
                chunk_payload_len: 4096,
                receive_dir: receive_dir.clone(),
                receive_policy: FileReceivePolicy {
                    allow_overwrite: true,
                    ..FileReceivePolicy::default()
                },
                ..FileTransferRuntimeConfig::default()
            },
        ));

        let reply_router = tokio::spawn(async move {
            while let Ok(packet) = source_udp_receiver.recv().await {
                if let MultiplexedPacket::Data(envelope, _) = packet
                    && source_inbound_tx.send(envelope).await.is_err()
                {
                    break;
                }
            }
        });
        let udp_router = tokio::spawn(async move {
            while let Ok(packet) = target_udp_receiver.recv().await {
                if let MultiplexedPacket::Data(envelope, _) = packet
                    && target_inbound_tx.send(envelope).await.is_err()
                {
                    break;
                }
            }
        });

        source_command_tx
            .send(FileTransferCommand::SendFile {
                path: source_path.clone(),
                mime_type: Some("application/octet-stream".to_string()),
            })
            .await
            .expect("send command should queue");

        let received_path = loop {
            let event = timeout(Duration::from_secs(5), target_event_rx.recv())
                .await
                .expect("target event should arrive")
                .expect("target event channel should remain open");
            if let FileTransferEvent::IncomingCompleted { path, .. } = event {
                break path;
            }
        };
        loop {
            let event = timeout(Duration::from_secs(5), source_event_rx.recv())
                .await
                .expect("source event should arrive")
                .expect("source event channel should remain open");
            if matches!(event, FileTransferEvent::OutgoingCompleted { .. }) {
                break;
            }
        }

        assert_eq!(
            tokio::fs::read(&received_path)
                .await
                .expect("received file should read"),
            payload
        );
        let stats = budget.stats();
        assert_eq!(stats.total_paced_bytes, payload.len() as u64);
        assert!(
            stats.video_yield_events >= 1,
            "expected at least one video-yield pacing event, got {stats:?}"
        );
        assert!(
            stats.total_wait_us > 0,
            "expected non-zero pacing wait time, got {stats:?}"
        );

        let _ = cancel_tx.send(());
        udp_router.abort();
        reply_router.abort();
        source_runtime
            .await
            .expect("source runtime should join")
            .expect("source runtime should stop cleanly");
        target_runtime
            .await
            .expect("target runtime should join")
            .expect("target runtime should stop cleanly");
        let _ = tokio::fs::remove_dir_all(&base).await;
    }

    #[test]
    fn transfer_checkpoint_validation_and_resume_offset_math() {
        let fingerprint = TransferCheckpoint::fingerprint_from_bytes(b"large-file-content");
        let mut checkpoint =
            TransferCheckpoint::new(42, fingerprint, 1000, 256).expect("valid checkpoint");

        assert_eq!(checkpoint.fingerprint_hex().len(), 64);
        assert_eq!(checkpoint.total_chunks().unwrap(), 4);
        assert_eq!(checkpoint.resume_offset().unwrap(), 0);
        assert_eq!(checkpoint.resume_chunk_index().unwrap(), 0);
        assert_eq!(checkpoint.remaining_bytes().unwrap(), 1000);
        assert_eq!(checkpoint.remaining_chunks().unwrap(), 4);
        assert!(!checkpoint.is_complete());
        assert!(!checkpoint.is_chunk_confirmed(0).unwrap());

        // Advance to chunk 2 boundary (512 bytes confirmed).
        assert_eq!(checkpoint.advance_to(512).unwrap(), 512);
        assert_eq!(checkpoint.resume_offset().unwrap(), 512);
        assert_eq!(checkpoint.resume_chunk_index().unwrap(), 2);
        assert_eq!(checkpoint.remaining_bytes().unwrap(), 488);
        assert_eq!(checkpoint.remaining_chunks().unwrap(), 2);
        assert!(checkpoint.is_chunk_confirmed(0).unwrap());
        assert!(checkpoint.is_chunk_confirmed(1).unwrap());
        assert!(!checkpoint.is_chunk_confirmed(2).unwrap());

        // Unaligned offset before total_bytes is rejected.
        let unaligned = TransferCheckpoint {
            confirmed_offset: 300,
            ..checkpoint.clone()
        };
        assert_eq!(
            unaligned.validate(),
            Err(TransferCheckpointError::UnalignedOffset {
                confirmed_offset: 300,
                chunk_size: 256,
            })
        );

        // Offset beyond total_bytes is rejected.
        let overflow = TransferCheckpoint {
            confirmed_offset: 1024,
            ..checkpoint.clone()
        };
        assert_eq!(
            overflow.validate(),
            Err(TransferCheckpointError::OffsetExceedsTotal {
                confirmed_offset: 1024,
                total_bytes: 1000,
            })
        );

        // Zero chunk_size is rejected.
        assert_eq!(
            TransferCheckpoint::new(42, fingerprint, 1000, 0),
            Err(TransferCheckpointError::ZeroChunkSize)
        );
    }

    #[test]
    fn transfer_checkpoint_validate_against_rejects_mismatches() {
        let sha_a = TransferCheckpoint::fingerprint_from_bytes(b"version-a");
        let sha_b = TransferCheckpoint::fingerprint_from_bytes(b"version-b");
        let checkpoint = TransferCheckpoint {
            transfer_id: 7,
            sha256: sha_a,
            total_bytes: 2048,
            confirmed_offset: 1024,
            chunk_size: 512,
        };

        assert!(checkpoint.validate_against(Some(7), &sha_a, 2048, 512).is_ok());
        assert!(checkpoint.validate_against(None, &sha_a, 2048, 512).is_ok());
        assert_eq!(
            checkpoint.validate_against(Some(8), &sha_a, 2048, 512),
            Err(TransferCheckpointError::TransferIdMismatch {
                expected: 8,
                actual: 7,
            })
        );
        assert_eq!(
            checkpoint.validate_against(Some(7), &sha_b, 2048, 512),
            Err(TransferCheckpointError::FingerprintMismatch)
        );
        assert_eq!(
            checkpoint.validate_against(Some(7), &sha_a, 4096, 512),
            Err(TransferCheckpointError::TotalBytesMismatch {
                expected: 4096,
                actual: 2048,
            })
        );
        assert_eq!(
            checkpoint.validate_against(Some(7), &sha_a, 2048, 1024),
            Err(TransferCheckpointError::ChunkSizeMismatch {
                expected: 1024,
                actual: 512,
            })
        );
    }

    #[test]
    fn transfer_checkpoint_advancement_idempotency_and_errors() {
        let sha = TransferCheckpoint::fingerprint_from_bytes(b"resumable-payload");
        let mut checkpoint = TransferCheckpoint::new(11, sha, 700, 256).unwrap();

        // Out-of-order chunk 1 before chunk 0 is rejected.
        assert_eq!(
            checkpoint.advance_chunk(1, 256),
            Err(TransferCheckpointError::NonContiguousChunk {
                expected_chunk: 0,
                actual_chunk: 1,
            })
        );

        // Wrong length for chunk 0 is rejected.
        assert_eq!(
            checkpoint.advance_chunk(0, 128),
            Err(TransferCheckpointError::InvalidChunkLength {
                chunk_index: 0,
                expected: 256,
                actual: 128,
            })
        );

        // Contiguous advancement through chunks 0, 1, and final partial chunk 2 (188 bytes).
        assert_eq!(checkpoint.advance_chunk(0, 256).unwrap(), 256);
        // Duplicate confirmation of chunk 0 is idempotent.
        assert_eq!(checkpoint.advance_chunk(0, 256).unwrap(), 256);
        assert_eq!(checkpoint.advance_chunk(1, 256).unwrap(), 512);
        // Regression via advance_to is rejected.
        assert_eq!(
            checkpoint.advance_to(256),
            Err(TransferCheckpointError::OffsetRegression {
                current: 512,
                proposed: 256,
            })
        );
        assert_eq!(checkpoint.advance_chunk(2, 188).unwrap(), 700);
        assert!(checkpoint.is_complete());
        assert_eq!(checkpoint.resume_offset().unwrap(), 700);
        assert_eq!(checkpoint.resume_chunk_index().unwrap(), 3);
        assert_eq!(checkpoint.remaining_bytes().unwrap(), 0);
        assert_eq!(checkpoint.remaining_chunks().unwrap(), 0);
    }

    #[tokio::test]
    async fn transfer_checkpoint_zero_byte_file_fingerprint_and_serde_roundtrip() {
        let base = unique_temp_dir("weavelo-checkpoint-fingerprint");
        tokio::fs::create_dir_all(&base)
            .await
            .expect("temp dir should be created");
        let file_path = base.join("sample.bin");
        let data = b"hello resumable checkpoint world";
        tokio::fs::write(&file_path, data)
            .await
            .expect("sample file should write");

        let from_file = TransferCheckpoint::fingerprint_file(&file_path)
            .await
            .expect("file fingerprint should succeed");
        let from_bytes = TransferCheckpoint::fingerprint_from_bytes(data);
        assert_eq!(from_file, from_bytes);

        let zero_cp = TransferCheckpoint::new(99, TransferCheckpoint::fingerprint_from_bytes(&[]), 0, 4096)
            .expect("zero-byte checkpoint should be valid");
        assert!(zero_cp.is_complete());
        assert_eq!(zero_cp.total_chunks().unwrap(), 0);
        assert_eq!(zero_cp.resume_chunk_index().unwrap(), 0);
        assert_eq!(zero_cp.remaining_bytes().unwrap(), 0);

        let checkpoint = TransferCheckpoint {
            transfer_id: 100,
            sha256: from_file,
            total_bytes: data.len() as u64,
            confirmed_offset: 16,
            chunk_size: 16,
        };
        let encoded = serde_json::to_string(&checkpoint).expect("checkpoint should serialize");
        let decoded: TransferCheckpoint =
            serde_json::from_str(&encoded).expect("checkpoint should deserialize");
        assert_eq!(decoded, checkpoint);

        let _ = tokio::fs::remove_dir_all(&base).await;
    }
}
