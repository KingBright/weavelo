use crate::data_plane::{LaneScheduler, LaneSchedulerConfig, SchedulerPushResult, is_stale};
use crate::net::UdpSender;
use protocol::{ContentKind, DataEnvelope, DataLane};
use std::collections::VecDeque;
use std::error::Error;
use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub type ScheduledDataSenderJoin = JoinHandle<Result<(), Box<dyn Error + Send + Sync>>>;

/// Maximum video pacing ceiling (600 Mbps = 500 Mbps target + 20% burst headroom).
pub const MAX_PACING_BITRATE_BPS: u64 = 600_000_000;
/// Minimum non-zero pacing rate (1 Mbps) when pacing is enabled.
pub const MIN_PACING_BITRATE_BPS: u64 = 1_000_000;
/// Default token-bucket burst allowance (256 KiB): lets single normal 144Hz
/// P-frames depart with zero delay while smoothing multi-frame / multi-stream
/// IDR bursts.
pub const DEFAULT_VIDEO_PACING_BURST_BYTES: usize = 256 * 1024;
/// Hard per-frame pacing delay cap (4 ms < 6.94 ms 144Hz frame interval) so
/// burst smoothing never induces multi-frame latency buildup.
pub const DEFAULT_VIDEO_PACING_MAX_DELAY_MS: u64 = 4;

/// Token-bucket burst smoother configuration for high-bitrate video frames.
/// Control, interactive, and audio traffic always bypasses video pacing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoPacingConfig {
    /// Pacing rate in bits per second (`0` disables pacing).
    pub bitrate_bps: u64,
    /// Maximum burst bytes allowed without sleeping (token-bucket capacity).
    pub max_burst_bytes: usize,
    /// Hard upper bound on per-frame pacing delay.
    pub max_delay: Duration,
}

impl VideoPacingConfig {
    pub const fn disabled() -> Self {
        Self {
            bitrate_bps: 0,
            max_burst_bytes: DEFAULT_VIDEO_PACING_BURST_BYTES,
            max_delay: Duration::from_millis(DEFAULT_VIDEO_PACING_MAX_DELAY_MS),
        }
    }

    pub const fn high_bitrate_default() -> Self {
        Self {
            bitrate_bps: MAX_PACING_BITRATE_BPS,
            max_burst_bytes: DEFAULT_VIDEO_PACING_BURST_BYTES,
            max_delay: Duration::from_millis(DEFAULT_VIDEO_PACING_MAX_DELAY_MS),
        }
    }
}

impl Default for VideoPacingConfig {
    fn default() -> Self {
        Self::high_bitrate_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct VideoPacingStats {
    pub paced_video_frames: u64,
    pub total_pacing_delay_us: u64,
    pub bypass_non_video_sent: u64,
}

/// High-resolution token-bucket pacer for smoothing high-bitrate video bursts
/// without adding latency to single frames or starving control/audio packets.
#[derive(Debug)]
struct VideoPacer {
    config: VideoPacingConfig,
    tokens_bytes: f64,
    last_refill: Instant,
}

impl VideoPacer {
    fn new(config: VideoPacingConfig, now: Instant) -> Self {
        let mut pacer = Self {
            config,
            tokens_bytes: config.max_burst_bytes as f64,
            last_refill: now,
        };
        pacer.set_bitrate_bps(config.bitrate_bps);
        pacer
    }

    fn set_bitrate_bps(&mut self, bitrate_bps: u64) {
        self.config.bitrate_bps = if bitrate_bps == 0 {
            0
        } else {
            bitrate_bps.clamp(MIN_PACING_BITRATE_BPS, MAX_PACING_BITRATE_BPS)
        };
    }

    fn reserve_delay(
        &mut self,
        bytes: usize,
        now: Instant,
        deadline_remaining: Option<Duration>,
    ) -> Duration {
        if self.config.bitrate_bps == 0 || bytes == 0 || self.config.max_delay.is_zero() {
            self.last_refill = now;
            return Duration::ZERO;
        }

        let bytes_per_sec = self.config.bitrate_bps as f64 / 8.0;
        let burst_cap = self.config.max_burst_bytes.max(1) as f64;
        let elapsed = now.saturating_duration_since(self.last_refill);
        self.tokens_bytes = (self.tokens_bytes + elapsed.as_secs_f64() * bytes_per_sec).min(burst_cap);
        self.last_refill = now;

        self.tokens_bytes -= bytes as f64;
        if self.tokens_bytes >= 0.0 {
            return Duration::ZERO;
        }

        let max_debt_bytes = bytes_per_sec * self.config.max_delay.as_secs_f64();
        if -self.tokens_bytes > max_debt_bytes {
            self.tokens_bytes = -max_debt_bytes;
        }

        let deficit = -self.tokens_bytes;
        let mut delay = Duration::from_secs_f64((deficit / bytes_per_sec).max(0.0))
            .min(self.config.max_delay);

        if let Some(remaining) = deadline_remaining {
            let guard = Duration::from_millis(1);
            if remaining <= guard {
                return Duration::ZERO;
            }
            delay = delay.min(remaining.saturating_sub(guard));
        }

        delay
    }
}

fn is_paced_video(envelope: &DataEnvelope) -> bool {
    envelope.header.lane == DataLane::Realtime && envelope.header.kind == ContentKind::VideoH265
}

#[derive(Debug, Clone, Copy)]
pub struct ScheduledDataSenderConfig {
    pub scheduler: LaneSchedulerConfig,
    /// Dedicated ingress capacity for deadline-sensitive/interactive traffic.
    pub realtime_queue_capacity: usize,
    /// Independent ingress capacity for reliable/background traffic.
    pub reliable_queue_capacity: usize,
}

impl Default for ScheduledDataSenderConfig {
    fn default() -> Self {
        Self {
            scheduler: LaneSchedulerConfig::default(),
            realtime_queue_capacity: 256,
            reliable_queue_capacity: 64,
        }
    }
}

#[derive(Clone)]
pub struct ScheduledDataSender {
    realtime_tx: mpsc::Sender<DataEnvelope>,
    reliable_tx: mpsc::Sender<DataEnvelope>,
    latest_video_overflow: Arc<std::sync::Mutex<std::collections::HashMap<u32, DataEnvelope>>>,
    overflow_notify: Arc<tokio::sync::Notify>,
    pacing_bitrate_bps: Arc<AtomicU64>,
    counters: Arc<ScheduledDataSenderCounters>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct ScheduledDataSenderStats {
    pub entrance_queued: usize,
    pub entrance_enqueued: u64,
    pub entrance_full: u64,
    pub entrance_closed: u64,
    pub scheduler_dropped_stale_realtime: u64,
    pub scheduler_dropped_realtime_capacity: u64,
    pub scheduler_rejected_reliable_capacity: u64,
    pub sent_realtime: u64,
    pub sent_reliable: u64,
    pub send_errors: u64,
}

#[derive(Default)]
struct ScheduledDataSenderCounters {
    entrance_enqueued: AtomicU64,
    entrance_full: AtomicU64,
    entrance_closed: AtomicU64,
    scheduler_dropped_stale_realtime: AtomicU64,
    scheduler_dropped_realtime_capacity: AtomicU64,
    scheduler_rejected_reliable_capacity: AtomicU64,
    pending_video_evictions: AtomicU64,
    sent_realtime: AtomicU64,
    sent_reliable: AtomicU64,
    send_errors: AtomicU64,
    paced_video_frames: AtomicU64,
    total_pacing_delay_us: AtomicU64,
    bypass_non_video_sent: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduledDataSendError {
    Full,
    Closed,
}

impl fmt::Display for ScheduledDataSendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScheduledDataSendError::Full => write!(f, "scheduled data sender queue is full"),
            ScheduledDataSendError::Closed => write!(f, "scheduled data sender is closed"),
        }
    }
}

impl Error for ScheduledDataSendError {}

impl ScheduledDataSender {
    pub fn spawn(
        udp_sender: UdpSender,
        target: SocketAddr,
        config: ScheduledDataSenderConfig,
    ) -> (Self, ScheduledDataSenderJoin) {
        Self::spawn_inner(
            udp_sender,
            target,
            config,
            false,
            VideoPacingConfig::high_bitrate_default(),
        )
    }

    pub fn spawn_with_pacing(
        udp_sender: UdpSender,
        target: SocketAddr,
        config: ScheduledDataSenderConfig,
        pacing: VideoPacingConfig,
    ) -> (Self, ScheduledDataSenderJoin) {
        Self::spawn_inner(udp_sender, target, config, false, pacing)
    }

    pub fn spawn_low_latency_realtime(
        udp_sender: UdpSender,
        target: SocketAddr,
    ) -> (Self, ScheduledDataSenderJoin) {
        Self::spawn_inner(
            udp_sender,
            target,
            ScheduledDataSenderConfig {
                scheduler: LaneSchedulerConfig {
                    // Sized for up to 32 concurrent subscriptions (MAX_SUBSCRIPTIONS)
                    // plus audio/control headroom while remaining strictly bounded.
                    max_realtime_queued: 64,
                    max_reliable_queued: 64,
                },
                realtime_queue_capacity: 64,
                reliable_queue_capacity: 64,
            },
            true,
            VideoPacingConfig::high_bitrate_default(),
        )
    }

    fn spawn_inner(
        udp_sender: UdpSender,
        target: SocketAddr,
        mut config: ScheduledDataSenderConfig,
        evict_older_video: bool,
        pacing: VideoPacingConfig,
    ) -> (Self, ScheduledDataSenderJoin) {
        config.realtime_queue_capacity = config.realtime_queue_capacity.max(1);
        config.reliable_queue_capacity = config.reliable_queue_capacity.max(1);
        let (realtime_tx, realtime_rx) = mpsc::channel(config.realtime_queue_capacity);
        let (reliable_tx, reliable_rx) = mpsc::channel(config.reliable_queue_capacity);
        let latest_video_overflow =
            Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        let overflow_notify = Arc::new(tokio::sync::Notify::new());
        let pacing_bitrate_bps = Arc::new(AtomicU64::new(pacing.bitrate_bps));
        let counters = Arc::new(ScheduledDataSenderCounters::default());
        let worker = tokio::spawn(run_sender_worker(
            udp_sender,
            target,
            config.scheduler,
            evict_older_video,
            pacing,
            pacing_bitrate_bps.clone(),
            realtime_rx,
            reliable_rx,
            latest_video_overflow.clone(),
            overflow_notify.clone(),
            counters.clone(),
        ));
        (
            Self {
                realtime_tx,
                reliable_tx,
                latest_video_overflow,
                overflow_notify,
                pacing_bitrate_bps,
                counters,
            },
            worker,
        )
    }

    /// Dynamically adjusts the video frame pacing rate in bits per second.
    /// Pass `0` to disable video pacing; non-zero values are clamped to
    /// `MIN_PACING_BITRATE_BPS..=MAX_PACING_BITRATE_BPS`.
    pub fn set_video_pacing_bitrate_bps(&self, bitrate_bps: u64) {
        self.pacing_bitrate_bps.store(bitrate_bps, Relaxed);
    }

    pub fn pacing_stats(&self) -> VideoPacingStats {
        VideoPacingStats {
            paced_video_frames: self.counters.paced_video_frames.load(Relaxed),
            total_pacing_delay_us: self.counters.total_pacing_delay_us.load(Relaxed),
            bypass_non_video_sent: self.counters.bypass_non_video_sent.load(Relaxed),
        }
    }

    fn ingress(&self, envelope: &DataEnvelope) -> &mpsc::Sender<DataEnvelope> {
        if envelope.header.lane.is_realtime() {
            &self.realtime_tx
        } else {
            &self.reliable_tx
        }
    }

    pub async fn send(&self, envelope: DataEnvelope) -> Result<(), ScheduledDataSendError> {
        let tx = self.ingress(&envelope);
        match tx.send(envelope).await {
            Ok(()) => {
                self.counters.entrance_enqueued.fetch_add(1, Relaxed);
                Ok(())
            }
            Err(_) => {
                self.counters.entrance_closed.fetch_add(1, Relaxed);
                Err(ScheduledDataSendError::Closed)
            }
        }
    }

    pub fn try_send(&self, envelope: DataEnvelope) -> Result<(), ScheduledDataSendError> {
        let tx = self.ingress(&envelope);
        match tx.try_send(envelope) {
            Ok(()) => {
                self.counters.entrance_enqueued.fetch_add(1, Relaxed);
                Ok(())
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.counters.entrance_full.fetch_add(1, Relaxed);
                Err(ScheduledDataSendError::Full)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.counters.entrance_closed.fetch_add(1, Relaxed);
                Err(ScheduledDataSendError::Closed)
            }
        }
    }

    /// Enqueue a realtime video frame without blocking; if the ingress channel is
    /// full, overwrite the per-stream overflow slot so only the newest frame survives.
    pub fn send_latest_video(&self, envelope: DataEnvelope) -> Result<(), ScheduledDataSendError> {
        match self.realtime_tx.try_send(envelope) {
            Ok(()) => {
                self.counters.entrance_enqueued.fetch_add(1, Relaxed);
                Ok(())
            }
            Err(mpsc::error::TrySendError::Full(envelope)) => {
                let stream_id = envelope.header.stream_id;
                let replaced = self
                    .latest_video_overflow
                    .lock()
                    .unwrap()
                    .insert(stream_id, envelope)
                    .is_some();
                self.counters.entrance_enqueued.fetch_add(1, Relaxed);
                if replaced {
                    self.counters
                        .scheduler_dropped_realtime_capacity
                        .fetch_add(1, Relaxed);
                }
                self.counters.pending_video_evictions.fetch_add(1, Relaxed);
                self.overflow_notify.notify_one();
                Ok(())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.counters.entrance_closed.fetch_add(1, Relaxed);
                Err(ScheduledDataSendError::Closed)
            }
        }
    }

    pub fn take_video_evictions(&self) -> u64 {
        self.counters.pending_video_evictions.swap(0, Relaxed)
    }

    pub fn stats(&self) -> ScheduledDataSenderStats {
        ScheduledDataSenderStats {
            entrance_queued: (self.realtime_tx.max_capacity() - self.realtime_tx.capacity())
                + (self.reliable_tx.max_capacity() - self.reliable_tx.capacity()),
            entrance_enqueued: self.counters.entrance_enqueued.load(Relaxed),
            entrance_full: self.counters.entrance_full.load(Relaxed),
            entrance_closed: self.counters.entrance_closed.load(Relaxed),
            scheduler_dropped_stale_realtime: self
                .counters
                .scheduler_dropped_stale_realtime
                .load(Relaxed),
            scheduler_dropped_realtime_capacity: self
                .counters
                .scheduler_dropped_realtime_capacity
                .load(Relaxed),
            scheduler_rejected_reliable_capacity: self
                .counters
                .scheduler_rejected_reliable_capacity
                .load(Relaxed),
            sent_realtime: self.counters.sent_realtime.load(Relaxed),
            sent_reliable: self.counters.sent_reliable.load(Relaxed),
            send_errors: self.counters.send_errors.load(Relaxed),
        }
    }
}

async fn run_sender_worker(
    udp_sender: UdpSender,
    target: SocketAddr,
    scheduler_config: LaneSchedulerConfig,
    evict_older_video: bool,
    pacing_config: VideoPacingConfig,
    pacing_bitrate_bps: Arc<AtomicU64>,
    mut realtime_rx: mpsc::Receiver<DataEnvelope>,
    mut reliable_rx: mpsc::Receiver<DataEnvelope>,
    latest_video_overflow: Arc<std::sync::Mutex<std::collections::HashMap<u32, DataEnvelope>>>,
    overflow_notify: Arc<tokio::sync::Notify>,
    counters: Arc<ScheduledDataSenderCounters>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut scheduler =
        LaneScheduler::with_config(scheduler_config).with_latest_video_only(evict_older_video);
    let mut pacer = VideoPacer::new(pacing_config, Instant::now());
    let mut deferred_video: Option<(DataEnvelope, Instant)> = None;
    let mut pacing_bypass: VecDeque<DataEnvelope> = VecDeque::new();
    let mut realtime_open = true;
    let mut reliable_open = true;

    loop {
        // Always ingest deadline-sensitive traffic before choosing the next
        // packet. This keeps bulk backpressure completely off the hot path.
        drain_realtime_ingress(
            &mut realtime_rx,
            &latest_video_overflow,
            &mut scheduler,
            &mut deferred_video,
            &mut pacing_bypass,
            evict_older_video,
            &counters,
        );

        // Non-video realtime/interactive packets (control, audio) bypass an
        // active video pacing wait so high-bitrate bursts never starve control.
        if let Some(envelope) = pacing_bypass.pop_front() {
            counters.bypass_non_video_sent.fetch_add(1, Relaxed);
            send_scheduled_envelope(&udp_sender, target, envelope, &counters).await?;
            continue;
        }

        if let Some((video_env, ready_at)) = deferred_video.take() {
            if is_stale(&video_env, now_ms()) {
                counters
                    .scheduler_dropped_stale_realtime
                    .fetch_add(1, Relaxed);
                counters.pending_video_evictions.fetch_add(1, Relaxed);
                continue;
            }
            let now = Instant::now();
            if now >= ready_at {
                send_scheduled_envelope(&udp_sender, target, video_env, &counters).await?;
                continue;
            }
            deferred_video = Some((video_env, ready_at));
            let sleep_until = tokio::time::Instant::from_std(ready_at);
            tokio::select! {
                biased;
                _ = overflow_notify.notified() => {
                    drain_realtime_ingress(
                        &mut realtime_rx,
                        &latest_video_overflow,
                        &mut scheduler,
                        &mut deferred_video,
                        &mut pacing_bypass,
                        evict_older_video,
                        &counters,
                    );
                }
                maybe = realtime_rx.recv(), if realtime_open => {
                    match maybe {
                        Some(envelope) => ingest_realtime_envelope(
                            envelope,
                            &mut scheduler,
                            &mut deferred_video,
                            &mut pacing_bypass,
                            evict_older_video,
                            &counters,
                        ),
                        None => realtime_open = false,
                    }
                }
                _ = tokio::time::sleep_until(sleep_until) => {}
            }
            continue;
        }

        if let Some(envelope) = pop_next_observed(&mut scheduler, now_ms(), &counters) {
            if is_paced_video(&envelope) {
                pacer.set_bitrate_bps(pacing_bitrate_bps.load(Relaxed));
                let deadline_remaining = envelope
                    .header
                    .deadline_ms
                    .map(|deadline| Duration::from_millis(deadline.saturating_sub(now_ms())));
                let now = Instant::now();
                let delay = pacer.reserve_delay(envelope.payload.len(), now, deadline_remaining);
                if !delay.is_zero() {
                    counters.paced_video_frames.fetch_add(1, Relaxed);
                    counters
                        .total_pacing_delay_us
                        .fetch_add(delay.as_micros() as u64, Relaxed);
                    deferred_video = Some((envelope, now + delay));
                    continue;
                }
            }
            send_scheduled_envelope(&udp_sender, target, envelope, &counters).await?;
            continue;
        }

        if !realtime_open && !reliable_open {
            return Ok(());
        }

        tokio::select! {
            biased;
            _ = overflow_notify.notified() => {
                drain_realtime_ingress(
                    &mut realtime_rx,
                    &latest_video_overflow,
                    &mut scheduler,
                    &mut deferred_video,
                    &mut pacing_bypass,
                    evict_older_video,
                    &counters,
                );
            }
            maybe = realtime_rx.recv(), if realtime_open => {
                match maybe {
                    Some(envelope) => push_observed(&mut scheduler, envelope, &counters),
                    None => realtime_open = false,
                }
            }
            maybe = reliable_rx.recv(), if reliable_open => {
                match maybe {
                    Some(envelope) => {
                        push_observed(&mut scheduler, envelope, &counters);
                        // Bulk traffic is allowed to yield one executor turn so a
                        // simultaneously-produced realtime/control packet can
                        // preempt it. Control/audio itself is never timer-delayed.
                        tokio::task::yield_now().await;
                    }
                    None => reliable_open = false,
                }
            }
        }
    }
}

fn ingest_realtime_envelope(
    envelope: DataEnvelope,
    scheduler: &mut LaneScheduler,
    deferred_video: &mut Option<(DataEnvelope, Instant)>,
    pacing_bypass: &mut VecDeque<DataEnvelope>,
    evict_older_video: bool,
    counters: &ScheduledDataSenderCounters,
) {
    if deferred_video.is_some() {
        if !is_paced_video(&envelope) {
            if is_stale(&envelope, now_ms()) {
                counters
                    .scheduler_dropped_stale_realtime
                    .fetch_add(1, Relaxed);
                counters.pending_video_evictions.fetch_add(1, Relaxed);
            } else {
                pacing_bypass.push_back(envelope);
            }
            return;
        }
        if evict_older_video
            && crate::media_plane::is_hevc_keyframe(&envelope.payload)
            && deferred_video
                .as_ref()
                .is_some_and(|(pending, _)| pending.header.stream_id == envelope.header.stream_id)
        {
            *deferred_video = None;
            counters
                .scheduler_dropped_realtime_capacity
                .fetch_add(1, Relaxed);
        }
    }
    push_observed(scheduler, envelope, counters);
}

fn pop_next_observed(
    scheduler: &mut LaneScheduler,
    now_ms: u64,
    counters: &ScheduledDataSenderCounters,
) -> Option<DataEnvelope> {
    let before = scheduler.stats().dropped_stale_realtime;
    let next = scheduler.pop_next(now_ms);
    let expired = scheduler.stats().dropped_stale_realtime - before;
    if expired > 0 {
        counters
            .scheduler_dropped_stale_realtime
            .fetch_add(expired, Relaxed);
        counters.pending_video_evictions.fetch_add(expired, Relaxed);
    }
    next
}

fn push_observed(
    scheduler: &mut LaneScheduler,
    envelope: DataEnvelope,
    counters: &ScheduledDataSenderCounters,
) {
    let before_evicted = scheduler.evicted_video_frames();
    let result = scheduler.push(envelope, now_ms());
    let evicted = scheduler.evicted_video_frames() - before_evicted;
    if evicted > 0 {
        // Older queued frames were cleanly superseded by a newer IDR keyframe;
        // do not trigger a redundant recovery keyframe request on the encoder.
        counters
            .scheduler_dropped_realtime_capacity
            .fetch_add(evicted, Relaxed);
    }
    if matches!(
        result,
        SchedulerPushResult::DroppedStaleRealtime | SchedulerPushResult::DroppedRealtimeForCapacity
    ) {
        counters.pending_video_evictions.fetch_add(1, Relaxed);
    }
    record_push_result(counters, result);
}

fn drain_realtime_ingress(
    rx: &mut mpsc::Receiver<DataEnvelope>,
    latest_video_overflow: &std::sync::Mutex<std::collections::HashMap<u32, DataEnvelope>>,
    scheduler: &mut LaneScheduler,
    deferred_video: &mut Option<(DataEnvelope, Instant)>,
    pacing_bypass: &mut VecDeque<DataEnvelope>,
    evict_older_video: bool,
    counters: &ScheduledDataSenderCounters,
) {
    while let Ok(envelope) = rx.try_recv() {
        ingest_realtime_envelope(
            envelope,
            scheduler,
            deferred_video,
            pacing_bypass,
            evict_older_video,
            counters,
        );
    }
    let overflow: Vec<DataEnvelope> = latest_video_overflow
        .lock()
        .unwrap()
        .drain()
        .map(|(_, env)| env)
        .collect();
    for envelope in overflow {
        ingest_realtime_envelope(
            envelope,
            scheduler,
            deferred_video,
            pacing_bypass,
            evict_older_video,
            counters,
        );
    }
}

fn record_push_result(counters: &ScheduledDataSenderCounters, result: SchedulerPushResult) {
    match result {
        SchedulerPushResult::Queued => {}
        SchedulerPushResult::DroppedStaleRealtime => {
            counters
                .scheduler_dropped_stale_realtime
                .fetch_add(1, Relaxed);
        }
        SchedulerPushResult::DroppedRealtimeForCapacity => {
            counters
                .scheduler_dropped_realtime_capacity
                .fetch_add(1, Relaxed);
        }
        SchedulerPushResult::RejectedReliableForCapacity => {
            counters
                .scheduler_rejected_reliable_capacity
                .fetch_add(1, Relaxed);
        }
    }
}

async fn send_scheduled_envelope(
    udp_sender: &UdpSender,
    target: SocketAddr,
    envelope: DataEnvelope,
    counters: &ScheduledDataSenderCounters,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let is_realtime = envelope.header.lane.is_realtime();
    if let Err(err) = udp_sender.send_data(&envelope, target).await {
        counters.send_errors.fetch_add(1, Relaxed);
        return Err(err);
    }
    if is_realtime {
        counters.sent_realtime.fetch_add(1, Relaxed);
    } else {
        counters.sent_reliable.fetch_add(1, Relaxed);
    }
    Ok(())
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
    use crate::data_plane::{
        ObjectChunkSpec, RealtimePacketSpec, make_realtime_envelope, object_chunk_envelope,
    };
    use crate::media_plane::{realtime_data_to_rtp, rtp_to_realtime_data};
    use crate::net::{MultiplexedPacket, UdpMultiplexer};
    use protocol::{PayloadType, RtpHeader, RtpPacket};
    use std::time::Duration;
    use tokio::time::{sleep, timeout};

    fn realtime(sequence_number: u64) -> DataEnvelope {
        let now = now_ms();
        make_realtime_envelope(
            RealtimePacketSpec::video(1, sequence_number, now, now + 1_000),
            vec![sequence_number as u8],
        )
    }

    fn bulk(sequence_number: u64) -> DataEnvelope {
        object_chunk_envelope(
            ObjectChunkSpec {
                object_id: sequence_number,
                stream_id: 7,
                sequence_number,
                timestamp_ms: now_ms(),
                chunk_index: 0,
                total_chunks: 1,
                offset: 0,
                total_size: 1,
                checksum_crc32: None,
            },
            vec![sequence_number as u8],
        )
    }

    fn media_packet(payload_type: PayloadType, sequence_number: u16, ssrc: u32) -> RtpPacket {
        let timestamp = if payload_type == PayloadType::VideoH265 {
            now_ms().saturating_add(1_000) as u32
        } else {
            sequence_number as u32 * 16
        };

        RtpPacket {
            header: RtpHeader {
                version: 2,
                payload_type: payload_type as u8,
                sequence_number,
                timestamp,
                ssrc,
            },
            payload: vec![sequence_number as u8; 16],
        }
    }

    #[test]
    fn queued_expiration_is_observed_once_and_preserves_ingress_drop_counts() {
        let mut scheduler = LaneScheduler::new();
        let counters = ScheduledDataSenderCounters::default();
        let video = || make_realtime_envelope(RealtimePacketSpec::video(1, 1, 1, 10), vec![1]);
        record_push_result(&counters, scheduler.push(video(), 11));
        record_push_result(&counters, scheduler.push(video(), 1));
        assert_eq!(counters.scheduler_dropped_stale_realtime.load(Relaxed), 1);
        scheduler.push(bulk(2), 1);
        let next = pop_next_observed(&mut scheduler, 11, &counters).unwrap();
        assert!(!next.header.lane.is_realtime());
        assert_eq!(counters.scheduler_dropped_stale_realtime.load(Relaxed), 2);
        assert!(pop_next_observed(&mut scheduler, 12, &counters).is_none());
        assert_eq!(counters.scheduler_dropped_stale_realtime.load(Relaxed), 2);
    }

    #[tokio::test]
    async fn zero_configuration_still_sends_and_shuts_down() {
        let sender_mux = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let receiver_mux = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let (udp_sender, _) = sender_mux.split();
        let (_, receiver) = receiver_mux.split();
        let (sender, worker) = ScheduledDataSender::spawn(
            udp_sender,
            receiver_mux.local_addr().unwrap(),
            ScheduledDataSenderConfig {
                realtime_queue_capacity: 0,
                reliable_queue_capacity: 0,
                ..Default::default()
            },
        );
        sender.send(bulk(1)).await.unwrap();
        drop(sender);
        assert!(!recv_data(&receiver).await.header.lane.is_realtime());
        timeout(Duration::from_secs(1), worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    async fn recv_data(receiver: &crate::net::UdpReceiver) -> DataEnvelope {
        match timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("receive should not time out")
            .expect("receive should succeed")
        {
            MultiplexedPacket::Data(envelope, _) => envelope,
            other => panic!("unexpected packet: {other:?}"),
        }
    }

    async fn wait_for_stats(
        scheduled: &ScheduledDataSender,
        predicate: impl Fn(ScheduledDataSenderStats) -> bool,
    ) -> ScheduledDataSenderStats {
        timeout(Duration::from_secs(2), async {
            loop {
                let stats = scheduled.stats();
                if predicate(stats) {
                    return stats;
                }
                sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("stats predicate should become true")
    }

    #[test]
    fn scheduled_sender_reports_entrance_backpressure() {
        let (realtime_tx, _realtime_rx) = mpsc::channel(1);
        let (reliable_tx, _reliable_rx) = mpsc::channel(1);
        let scheduled = ScheduledDataSender {
            realtime_tx,
            reliable_tx,
            latest_video_overflow: Arc::new(
                std::sync::Mutex::new(std::collections::HashMap::new()),
            ),
            overflow_notify: Arc::new(tokio::sync::Notify::new()),
            pacing_bitrate_bps: Arc::new(AtomicU64::new(0)),
            counters: Arc::new(ScheduledDataSenderCounters::default()),
        };

        scheduled
            .try_send(realtime(1))
            .expect("first packet should fit");
        assert_eq!(
            scheduled
                .try_send(realtime(2))
                .expect_err("second packet should hit full entrance"),
            ScheduledDataSendError::Full
        );

        let stats = scheduled.stats();
        assert_eq!(stats.entrance_queued, 1);
        assert_eq!(stats.entrance_enqueued, 1);
        assert_eq!(stats.entrance_full, 1);
        assert_eq!(stats.entrance_closed, 0);
    }

    #[tokio::test]
    async fn scheduled_sender_sends_realtime_before_queued_bulk() {
        let sender_mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("sender should bind");
        let receiver_mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("receiver should bind");
        let target = receiver_mux
            .local_addr()
            .expect("receiver should have local addr");
        let (udp_sender, _) = sender_mux.split();
        let (_, receiver) = receiver_mux.split();
        let (scheduled, worker) = ScheduledDataSender::spawn(
            udp_sender,
            target,
            ScheduledDataSenderConfig {
                scheduler: LaneSchedulerConfig {
                    max_realtime_queued: 8,
                    max_reliable_queued: 8,
                },
                realtime_queue_capacity: 8,
                reliable_queue_capacity: 8,
            },
        );

        scheduled.send(bulk(1)).await.expect("bulk should queue");
        scheduled
            .send(realtime(2))
            .await
            .expect("realtime should queue");

        let first = recv_data(&receiver).await;
        let second = recv_data(&receiver).await;

        assert!(first.header.lane.is_realtime());
        assert!(!second.header.lane.is_realtime());
        let stats = scheduled.stats();
        assert_eq!(stats.sent_realtime, 1);
        assert_eq!(stats.sent_reliable, 1);
        assert_eq!(stats.send_errors, 0);

        drop(scheduled);
        worker
            .await
            .expect("worker should join")
            .expect("worker should stop");
    }

    #[tokio::test]
    async fn scheduled_sender_loopback_prioritizes_media_over_bulk() {
        let sender_mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("sender should bind");
        let receiver_mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("receiver should bind");
        let target = receiver_mux
            .local_addr()
            .expect("receiver should have local addr");
        let (udp_sender, _) = sender_mux.split();
        let (_, receiver) = receiver_mux.split();
        let (scheduled, worker) = ScheduledDataSender::spawn(
            udp_sender,
            target,
            ScheduledDataSenderConfig {
                scheduler: LaneSchedulerConfig {
                    max_realtime_queued: 8,
                    max_reliable_queued: 8,
                },
                realtime_queue_capacity: 8,
                reliable_queue_capacity: 8,
            },
        );
        let video = media_packet(PayloadType::VideoH265, 2, 10);
        let audio = media_packet(PayloadType::AudioOpus, 3, 11);

        scheduled.send(bulk(1)).await.expect("bulk should queue");
        scheduled
            .send(rtp_to_realtime_data(&audio).expect("audio should adapt"))
            .await
            .expect("audio should queue");
        scheduled
            .send(rtp_to_realtime_data(&video).expect("video should adapt"))
            .await
            .expect("video should queue");

        let first = recv_data(&receiver).await;
        let second = recv_data(&receiver).await;
        let third = recv_data(&receiver).await;

        let first_media = realtime_data_to_rtp(first).expect("first should be media");
        let second_media = realtime_data_to_rtp(second).expect("second should be media");
        assert_eq!(first_media, video);
        assert_eq!(second_media, audio);
        assert!(!third.header.lane.is_realtime());
        let stats = scheduled.stats();
        assert_eq!(stats.sent_realtime, 2);
        assert_eq!(stats.sent_reliable, 1);
        assert_eq!(stats.send_errors, 0);

        drop(scheduled);
        worker
            .await
            .expect("worker should join")
            .expect("worker should stop");
    }

    #[tokio::test]
    async fn scheduled_sender_keeps_realtime_first_under_reliable_pressure() {
        let sender_mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("sender should bind");
        let receiver_mux = UdpMultiplexer::bind("127.0.0.1:0")
            .await
            .expect("receiver should bind");
        let target = receiver_mux
            .local_addr()
            .expect("receiver should have local addr");
        let (udp_sender, _) = sender_mux.split();
        let (_, receiver) = receiver_mux.split();
        let (scheduled, worker) = ScheduledDataSender::spawn(
            udp_sender,
            target,
            ScheduledDataSenderConfig {
                scheduler: LaneSchedulerConfig {
                    max_realtime_queued: 8,
                    max_reliable_queued: 4,
                },
                realtime_queue_capacity: 32,
                reliable_queue_capacity: 32,
            },
        );
        let video = media_packet(PayloadType::VideoH265, 50, 10);
        let audio = media_packet(PayloadType::AudioOpus, 51, 11);

        for sequence_number in 1..=12 {
            scheduled
                .send(bulk(sequence_number))
                .await
                .expect("bulk should enter scheduled sender");
        }
        scheduled
            .send(rtp_to_realtime_data(&audio).expect("audio should adapt"))
            .await
            .expect("audio should enter scheduled sender");
        scheduled
            .send(rtp_to_realtime_data(&video).expect("video should adapt"))
            .await
            .expect("video should enter scheduled sender");

        let first = recv_data(&receiver).await;
        let second = recv_data(&receiver).await;
        let first_media = realtime_data_to_rtp(first).expect("first should be media");
        let second_media = realtime_data_to_rtp(second).expect("second should be media");

        assert_eq!(first_media, video);
        assert_eq!(second_media, audio);

        let stats = wait_for_stats(&scheduled, |stats| {
            stats.sent_realtime == 2 && stats.sent_reliable > 0
        })
        .await;
        assert_eq!(stats.scheduler_dropped_stale_realtime, 0);
        assert_eq!(stats.scheduler_dropped_realtime_capacity, 0);
        assert_eq!(stats.entrance_full, 0);
        assert_eq!(stats.send_errors, 0);

        drop(scheduled);
        worker
            .await
            .expect("worker should join")
            .expect("worker should stop");
    }

    #[test]
    fn video_pacer_allows_single_frame_burst_without_delay_and_smooths_500mbps_burst() {
        let start = Instant::now();
        let mut pacer = VideoPacer::new(
            VideoPacingConfig {
                bitrate_bps: 500_000_000, // 500 Mbps = 62.5 MB/s
                max_burst_bytes: 256 * 1024,
                max_delay: Duration::from_millis(4),
            },
            start,
        );

        // Single 200 KiB frame fits in burst budget: zero added latency.
        assert_eq!(
            pacer.reserve_delay(200 * 1024, start, Some(Duration::from_millis(20))),
            Duration::ZERO
        );

        // Immediate second 200 KiB frame exceeds remaining burst budget and is smoothed.
        let delay = pacer.reserve_delay(200 * 1024, start, Some(Duration::from_millis(20)));
        assert!(delay > Duration::ZERO && delay <= Duration::from_millis(4));

        // Sustained 144Hz @ 500 Mbps (~434 KiB every 6.94 ms) stays bounded and never
        // accumulates runaway debt.
        let mut t = start + Duration::from_millis(20);
        for _ in 0..144 {
            let d = pacer.reserve_delay(434 * 1024, t, Some(Duration::from_millis(16)));
            assert!(
                d <= Duration::from_millis(4),
                "144Hz 500Mbps frame delay exceeded cap: {d:?}"
            );
            t += Duration::from_micros(6_944);
        }

        // Imminent deadline overrides pacing delay so frames are not paced past expiry.
        let near_deadline = pacer.reserve_delay(400 * 1024, t, Some(Duration::from_millis(1)));
        assert_eq!(near_deadline, Duration::ZERO);

        // Disabling pacing (0 bps) yields zero delay even on huge bursts.
        pacer.set_bitrate_bps(0);
        assert_eq!(
            pacer.reserve_delay(1024 * 1024, t, Some(Duration::from_millis(20))),
            Duration::ZERO
        );
    }

    #[tokio::test]
    async fn paced_video_burst_does_not_starve_control_or_audio_packets() {
        let sender_mux = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let receiver_mux = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let target = receiver_mux.local_addr().unwrap();
        let (udp_sender, _) = sender_mux.split();
        let (_, receiver) = receiver_mux.split();

        let (scheduled, worker) = ScheduledDataSender::spawn_with_pacing(
            udp_sender,
            target,
            ScheduledDataSenderConfig {
                scheduler: LaneSchedulerConfig {
                    max_realtime_queued: 32,
                    max_reliable_queued: 16,
                },
                realtime_queue_capacity: 32,
                reliable_queue_capacity: 16,
            },
            VideoPacingConfig {
                // 8 Mbps = 1 MB/s, 8 KiB burst -> a 24 KiB second frame needs ~16ms pacing wait,
                // giving plenty of deterministic headroom for control & audio to bypass.
                bitrate_bps: 8_000_000,
                max_burst_bytes: 8 * 1024,
                max_delay: Duration::from_millis(40),
            },
        );

        let now = now_ms();
        let large_video = |seq: u64| {
            make_realtime_envelope(
                RealtimePacketSpec::video(1, seq, now, now + 2_000),
                vec![seq as u8; 24 * 1024],
            )
        };
        let control = DataEnvelope::new(
            DataLane::Interactive,
            ContentKind::Control,
            0,
            999,
            now,
            b"ping".to_vec(),
        );
        let audio = make_realtime_envelope(
            RealtimePacketSpec::audio(2, 1000, now, now + 2_000),
            vec![0xaa; 64],
        );

        // Queue two large video frames so the second enters the pacing wait.
        scheduled.send(large_video(1)).await.unwrap();
        scheduled.send(large_video(2)).await.unwrap();
        // Wait briefly until frame 1 is sent and frame 2 is deferred in the pacer.
        let first = recv_data(&receiver).await;
        assert_eq!(first.header.kind, ContentKind::VideoH265);
        assert_eq!(first.header.sequence_number, 1);

        // Now send control and audio while frame 2 is waiting on the video pacer.
        scheduled.send(control.clone()).await.unwrap();
        scheduled.send(audio.clone()).await.unwrap();

        let second = recv_data(&receiver).await;
        let third = recv_data(&receiver).await;
        let fourth = recv_data(&receiver).await;

        assert_eq!(second.header.kind, ContentKind::Control);
        assert_eq!(third.header.kind, ContentKind::AudioOpus);
        assert_eq!(fourth.header.kind, ContentKind::VideoH265);
        assert_eq!(fourth.header.sequence_number, 2);

        let pacing_stats = scheduled.pacing_stats();
        assert!(pacing_stats.paced_video_frames >= 1);
        assert!(pacing_stats.total_pacing_delay_us > 0);
        assert!(pacing_stats.bypass_non_video_sent >= 2);

        drop(scheduled);
        worker.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn spawn_low_latency_realtime_handles_32_stream_burst_without_capacity_drops() {
        let sender_mux = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let receiver_mux = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let target = receiver_mux.local_addr().unwrap();
        let (udp_sender, _) = sender_mux.split();
        let (_, receiver) = receiver_mux.split();

        let (scheduled, worker) = ScheduledDataSender::spawn_low_latency_realtime(udp_sender, target);
        let now = now_ms();
        for stream_id in 1..=protocol::session::MAX_SUBSCRIPTIONS as u32 {
            let env = make_realtime_envelope(
                RealtimePacketSpec::video(stream_id, stream_id as u64, now, now + 2_000),
                vec![stream_id as u8; 128],
            );
            scheduled.send_latest_video(env).unwrap();
        }

        for _ in 0..protocol::session::MAX_SUBSCRIPTIONS {
            let received = recv_data(&receiver).await;
            assert_eq!(received.header.kind, ContentKind::VideoH265);
        }

        let stats = scheduled.stats();
        assert_eq!(stats.sent_realtime, protocol::session::MAX_SUBSCRIPTIONS as u64);
        assert_eq!(stats.scheduler_dropped_realtime_capacity, 0);
        assert_eq!(stats.send_errors, 0);

        drop(scheduled);
        worker.await.unwrap().unwrap();
    }
}
