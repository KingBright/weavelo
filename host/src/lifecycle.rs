//! Scalar process-local diagnostics. No frame, title, credential or input data.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, AtomicUsize, Ordering::SeqCst},
    },
};

static ACTIVE_VIDEO_STREAMS: AtomicUsize = AtomicUsize::new(0);

#[derive(Default)]
pub(crate) struct Resources {
    pub peer_device:OnceLock<String>,
    pub workers: AtomicUsize,
    pub video_streams: AtomicUsize,
    pub native_streams: AtomicUsize,
    pub native_stops_pending: AtomicUsize,
    pub native_stops_completed: AtomicU64,
    pub native_stop_errors: AtomicU64,
    pub captured_frames: AtomicU64,
    pub encoded_frames: AtomicU64,
    pub capture_wait: StageTimer,
    pub video_submit: StageTimer,
    pub media_send: StageTimer,
    pub encode_queue: StageTimer,
    pub upload_vpp: StageTimer,
    pub hardware_encode: StageTimer,
    pub native_encode_total: StageTimer,
    pub last_slow_encode: Mutex<Option<SlowEncodeFrame>>,
    pub encoder_backpressure_ticks: AtomicU64,
    pub native_camera_frames: AtomicU64,
    pub raw_capture_samples: AtomicU64,
    pub capture_pacing_skips: AtomicU64,
    pub capture_target_fps: AtomicU64,
    pub native_capture_geometry: Mutex<Option<NativeCaptureGeometry>>,
    pub scheduled_data: Mutex<Option<ScheduledDataObservation>>,
    pub focus_probe: FocusProbeDiagnostics,
}

#[derive(Default)]
pub(crate) struct StageTimer {calls:AtomicU64,total_us:AtomicU64,max_us:AtomicU64,over_10ms:AtomicU64,over_20ms:AtomicU64}
impl StageTimer {
    pub(crate) fn record(&self,duration:std::time::Duration) {
        let us=duration.as_micros().min(u64::MAX as u128) as u64;
        self.calls.fetch_add(1,SeqCst);self.total_us.fetch_add(us,SeqCst);self.max_us.fetch_max(us,SeqCst);
        if us>10_000 {self.over_10ms.fetch_add(1,SeqCst);}
        if us>20_000 {self.over_20ms.fetch_add(1,SeqCst);}
    }
    fn snapshot(&self)->StageTimingSnapshot {StageTimingSnapshot {
        calls:self.calls.load(SeqCst),total_us:self.total_us.load(SeqCst),max_us:self.max_us.load(SeqCst),
        over_10ms:self.over_10ms.load(SeqCst),over_20ms:self.over_20ms.load(SeqCst),
    }}
}
/// Wall-clock time in a stage, including waits; this does not measure CPU use.
#[derive(Clone,Default,Serialize,Deserialize)]
pub struct StageTimingSnapshot {
    pub calls:u64,pub total_us:u64,pub max_us:u64,
    #[serde(default)] pub over_10ms:u64,
    #[serde(default)] pub over_20ms:u64,
}
#[derive(Clone,Default,Serialize,Deserialize)]
pub struct VideoTimingSnapshot {
    pub capture_wait:StageTimingSnapshot,pub submit:StageTimingSnapshot,pub send:StageTimingSnapshot,
    #[serde(default)] pub encode_queue:StageTimingSnapshot,
    #[serde(default)] pub upload_vpp:StageTimingSnapshot,
    #[serde(default)] pub hardware_encode:StageTimingSnapshot,
    #[serde(default)] pub native_encode_total:StageTimingSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_slow_encode: Option<SlowEncodeFrame>,
    pub encoder_backpressure_ticks:u64,
    #[serde(default)] pub native_camera_frames:u64,
    #[serde(default)] pub raw_capture_samples:u64,
    #[serde(default)] pub capture_pacing_skips:u64,
    #[serde(default)] pub capture_target_fps:u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_capture_geometry: Option<NativeCaptureGeometry>,
}

/// Bounded counters for the authenticated editor-focus probe path. These values
/// contain no input payload, address, window identity, or per-tap history.
#[derive(Default)]
pub(crate) struct FocusProbeDiagnostics {
    valid_requests: AtomicU64,
    rejected_requests: AtomicU64,
    accepted_primary_releases: AtomicU64,
    exact_matches: AtomicU64,
    scheduled: AtomicU64,
    expired_before_ax: AtomicU64,
    expired_after_ax: AtomicU64,
    invalidated_before_ax: AtomicU64,
    probe_unavailable: AtomicU64,
    probe_busy: AtomicU64,
    editable_results: AtomicU64,
    noneditable_results: AtomicU64,
    result_queue_drops: AtomicU64,
    reply_attempts: AtomicU64,
    reply_rejections: AtomicU64,
    ax_timing: StageTimer,
}
impl FocusProbeDiagnostics {
    pub(crate) fn request(&self, valid: bool) {
        if valid { self.valid_requests.fetch_add(1, SeqCst); }
        else { self.rejected_requests.fetch_add(1, SeqCst); }
    }
    pub(crate) fn accepted_primary_release(&self) { self.accepted_primary_releases.fetch_add(1, SeqCst); }
    pub(crate) fn exact_match(&self) { self.exact_matches.fetch_add(1, SeqCst); }
    pub(crate) fn scheduled(&self) { self.scheduled.fetch_add(1, SeqCst); }
    pub(crate) fn expired_before_ax(&self) { self.expired_before_ax.fetch_add(1, SeqCst); }
    pub(crate) fn expired_after_ax(&self) { self.expired_after_ax.fetch_add(1, SeqCst); }
    pub(crate) fn invalidated_before_ax(&self) { self.invalidated_before_ax.fetch_add(1, SeqCst); }
    pub(crate) fn probe_unavailable(&self) { self.probe_unavailable.fetch_add(1, SeqCst); }
    pub(crate) fn probe_busy(&self) { self.probe_busy.fetch_add(1, SeqCst); }
    pub(crate) fn editable_result(&self, editable: bool) {
        if editable { self.editable_results.fetch_add(1, SeqCst); }
        else { self.noneditable_results.fetch_add(1, SeqCst); }
    }
    pub(crate) fn result_queue_dropped(&self) { self.result_queue_drops.fetch_add(1, SeqCst); }
    pub(crate) fn reply_attempted(&self) { self.reply_attempts.fetch_add(1, SeqCst); }
    pub(crate) fn reply_rejected(&self) { self.reply_rejections.fetch_add(1, SeqCst); }
    pub(crate) fn ax_timing(&self, duration: std::time::Duration) { self.ax_timing.record(duration); }

    pub(crate) fn snapshot(&self) -> FocusProbeSnapshot {
        FocusProbeSnapshot {
            valid_requests: self.valid_requests.load(SeqCst),
            rejected_requests: self.rejected_requests.load(SeqCst),
            accepted_primary_releases: self.accepted_primary_releases.load(SeqCst),
            exact_matches: self.exact_matches.load(SeqCst),
            scheduled: self.scheduled.load(SeqCst),
            expired_before_ax: self.expired_before_ax.load(SeqCst),
            expired_after_ax: self.expired_after_ax.load(SeqCst),
            invalidated_before_ax: self.invalidated_before_ax.load(SeqCst),
            probe_unavailable: self.probe_unavailable.load(SeqCst),
            probe_busy: self.probe_busy.load(SeqCst),
            editable_results: self.editable_results.load(SeqCst),
            noneditable_results: self.noneditable_results.load(SeqCst),
            result_queue_drops: self.result_queue_drops.load(SeqCst),
            reply_attempts: self.reply_attempts.load(SeqCst),
            reply_rejections: self.reply_rejections.load(SeqCst),
            ax_timing: self.ax_timing.snapshot(),
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct FocusProbeSnapshot {
    #[serde(default)] pub valid_requests: u64,
    #[serde(default)] pub rejected_requests: u64,
    #[serde(default)] pub accepted_primary_releases: u64,
    #[serde(default)] pub exact_matches: u64,
    #[serde(default)] pub scheduled: u64,
    #[serde(default)] pub expired_before_ax: u64,
    #[serde(default)] pub expired_after_ax: u64,
    #[serde(default)] pub invalidated_before_ax: u64,
    #[serde(default)] pub probe_unavailable: u64,
    #[serde(default)] pub probe_busy: u64,
    #[serde(default)] pub editable_results: u64,
    #[serde(default)] pub noneditable_results: u64,
    #[serde(default)] pub result_queue_drops: u64,
    #[serde(default)] pub reply_attempts: u64,
    #[serde(default)] pub reply_rejections: u64,
    #[serde(default)] pub ax_timing: StageTimingSnapshot,
}

/// Correlated parts of the most recent >20ms frame, without any media bytes.
#[derive(Clone, Serialize, Deserialize)]
pub struct SlowEncodeFrame {
    pub observed_at_ms: u64,
    pub total_us: u64,
    pub queue_us: u64,
    pub upload_vpp_us: u64,
    pub hardware_encode_us: u64,
    pub overhead_us: u64,
    pub keyframe: bool,
    pub encoded_bytes: usize,
}

/// Last native format transition, with no pixels, titles or input payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct NativeCaptureGeometry {
    pub source: protocol::session::CaptureSource,
    pub buffer: [u32; 2],
    pub content_rect: Option<[f64; 4]>,
    #[serde(default)]
    pub bounding_rect: Option<[f64; 4]>,
    #[serde(default)]
    pub screen_rect: Option<[f64; 4]>,
    pub content_scale: Option<f64>,
    pub scale_factor: Option<f64>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ScheduledDataObservation {
    pub observed_at_ms: u64,
    pub stats: remote_core::scheduled_sender::ScheduledDataSenderStats,
}

pub(crate) const CLEANUP_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// A receipt is observation only. Dropping it cannot release a native owner.
#[derive(Clone)]
pub(crate) struct NativeStopReceipt(Arc<Mutex<Option<Result<(), String>>>>);
pub(crate) struct NativeStopCompletion {
    receipt: NativeStopReceipt,
    resources: Option<Arc<Resources>>,
}
impl NativeStopReceipt {
    pub(crate) async fn wait(&self) -> Result<(), String> {
        tokio::time::timeout(CLEANUP_WAIT, async {
            loop {
                if let Some(result) = self.0.lock().unwrap().clone() {
                    return result;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| "native stop completion is unknown; owner remains retained".to_string())?
    }
}
impl NativeStopCompletion {
    pub(crate) fn new(resources: Option<Arc<Resources>>) -> (NativeStopReceipt, Self) {
        if let Some(x) = &resources {
            x.native_stops_pending.fetch_add(1, SeqCst);
        }
        let receipt = NativeStopReceipt(Arc::new(Mutex::new(None)));
        (receipt.clone(), Self { receipt, resources })
    }
    /// Success requires actual native owner destruction; error retains the owner.
    pub(crate) fn complete(self, result: Result<(), String>) {
        if let Some(x) = &self.resources {
            if result.is_ok() {
                x.native_stops_completed.fetch_add(1, SeqCst);
                x.native_streams.fetch_sub(1, SeqCst);
            } else {
                x.native_stop_errors.fetch_add(1, SeqCst);
            }
            x.native_stops_pending.fetch_sub(1, SeqCst);
        }
        *self.receipt.0.lock().unwrap() = Some(result);
    }
}
pub(crate) async fn wait_native_stops(resources: &Resources) {
    while resources.native_stops_pending.load(SeqCst) != 0 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}
pub(crate) struct WorkerGuard(pub Arc<Resources>);
impl WorkerGuard {
    pub fn new(resources: Arc<Resources>) -> Self {
        resources.workers.fetch_add(1, SeqCst);
        Self(resources)
    }
}
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.0.workers.fetch_sub(1, SeqCst);
    }
}

pub(crate) struct VideoStreamGuard(WorkerGuard);
impl VideoStreamGuard {
    pub fn new(resources: Arc<Resources>) -> Self {
        resources.video_streams.fetch_add(1, SeqCst);
        ACTIVE_VIDEO_STREAMS.fetch_add(1, SeqCst);
        Self(WorkerGuard::new(resources))
    }
}
impl Drop for VideoStreamGuard {
    fn drop(&mut self) {
        self.0.0.video_streams.fetch_sub(1, SeqCst);
        ACTIVE_VIDEO_STREAMS.fetch_sub(1, SeqCst);
    }
}

/// Scalar activity only: file connections and remembered portal grants are idle.
pub fn active_video_streams() -> usize {
    ACTIVE_VIDEO_STREAMS.load(SeqCst)
}

/// Active subscriptions grouped by their routed device identity. A direct
/// connection with no device binding remains explicitly unattributed.
pub fn peer_video_streams()->Vec<(Option<String>,usize)> {
    let mut peers=std::collections::BTreeMap::new();
    for record in registry().lock().unwrap().values().flatten() {
        let streams=record.resources.video_streams.load(SeqCst);
        if record.state=="open" && streams>0 {
            *peers.entry(record.resources.peer_device.get().cloned()).or_insert(0)+=streams;
        }
    }
    peers.into_iter().collect()
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ConnectionLifecycleSnapshot {
    pub connection_id: u32,
    pub state: String,
    pub observed_at_ms: u64,
    pub active_capture_workers: usize,
    pub active_video_streams: usize,
    pub native_streams: usize,
    pub native_stops_pending: usize,
    pub native_stops_completed: u64,
    pub native_stop_errors: u64,
    pub captured_frames: u64,
    pub encoded_frames: u64,
    #[serde(default)]
    pub video_timing:VideoTimingSnapshot,
    #[serde(default)]
    pub focus_probe: FocusProbeSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_data: Option<ScheduledDataObservation>,
}
struct Record {
    resources: Arc<Resources>,
    state: String,
    at: u64,
}
fn registry() -> &'static Mutex<HashMap<u32, Vec<Record>>> {
    static REG: OnceLock<Mutex<HashMap<u32, Vec<Record>>>> = OnceLock::new();
    REG.get_or_init(Default::default)
}
pub(crate) fn publish(id: u32, resources: &Arc<Resources>, state: &str) {
    let now = remote_core::session_crypto::now_unix_ms();
    let mut reg = registry().lock().unwrap();
    // Keep active records; bounded terminal retention. ID collisions remain
    // separate records, never falsely identify one user's session as another.
    reg.retain(|_, records| {
        records.retain(|r| {
            r.state == "open"
                || r.state == "closing"
                || r.resources.workers.load(SeqCst) != 0
                || r.resources.native_streams.load(SeqCst) != 0
                || now.saturating_sub(r.at) < 60_000
        });
        !records.is_empty()
    });
    if reg.len() >= 128 && !reg.contains_key(&id) {
        return;
    }
    let records = reg.entry(id).or_default();
    if let Some(record) = records
        .iter_mut()
        .find(|r| Arc::ptr_eq(&r.resources, resources))
    {
        record.state = state.into();
        record.at = now;
    } else if records.len() < 8 {
        records.push(Record {
            resources: resources.clone(),
            state: state.into(),
            at: now,
        });
    }
}
/// Same-user IPC can query its known connection ID without enumerating sessions.
pub fn inspect_connection(id: u32) -> Vec<ConnectionLifecycleSnapshot> {
    let now = remote_core::session_crypto::now_unix_ms();
    let reg = registry().lock().unwrap();
    reg.get(&id)
        .into_iter()
        .flatten()
        .filter(|r| {
            r.state == "open"
                || r.state == "closing"
                || r.resources.workers.load(SeqCst) != 0
                || r.resources.native_streams.load(SeqCst) != 0
                || now.saturating_sub(r.at) < 60_000
        })
        .map(|r| {
            let x = &r.resources;
            ConnectionLifecycleSnapshot {
                connection_id: id,
                state: r.state.clone(),
                observed_at_ms: r.at,
                active_capture_workers: x.workers.load(SeqCst),
                active_video_streams: x.video_streams.load(SeqCst),
                native_streams: x.native_streams.load(SeqCst),
                native_stops_pending: x.native_stops_pending.load(SeqCst),
                native_stops_completed: x.native_stops_completed.load(SeqCst),
                native_stop_errors: x.native_stop_errors.load(SeqCst),
                captured_frames: x.captured_frames.load(SeqCst),
                encoded_frames: x.encoded_frames.load(SeqCst),
                video_timing:VideoTimingSnapshot {capture_wait:x.capture_wait.snapshot(),submit:x.video_submit.snapshot(),
                    encode_queue:x.encode_queue.snapshot(),upload_vpp:x.upload_vpp.snapshot(),hardware_encode:x.hardware_encode.snapshot(),
                    native_encode_total:x.native_encode_total.snapshot(),
                    last_slow_encode:x.last_slow_encode.lock().unwrap().clone(),
                    send:x.media_send.snapshot(),encoder_backpressure_ticks:x.encoder_backpressure_ticks.load(SeqCst),
                    native_camera_frames:x.native_camera_frames.load(SeqCst),raw_capture_samples:x.raw_capture_samples.load(SeqCst),
                    capture_pacing_skips:x.capture_pacing_skips.load(SeqCst),capture_target_fps:x.capture_target_fps.load(SeqCst),
                    native_capture_geometry:x.native_capture_geometry.lock().unwrap().clone()},
                focus_probe: x.focus_probe.snapshot(),
                scheduled_data: x.scheduled_data.lock().unwrap().clone(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn video_activity_clears_cancelled_streams_independently_of_other_channels() {
        let resources = Arc::new(Resources::default());
        resources.peer_device.set("activity-owner-fixture".into()).unwrap();
        publish(0xfed10001,&resources,"open");
        let file_worker = WorkerGuard::new(resources.clone());
        let other_stream = VideoStreamGuard::new(resources.clone());
        let (ready, started) = tokio::sync::oneshot::channel();
        let observed = resources.clone();
        let task = tokio::spawn(async move {
            let _stream = VideoStreamGuard::new(observed);
            ready.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        started.await.unwrap();
        let count=||peer_video_streams().into_iter().find(|(peer,_)|peer.as_deref()==Some("activity-owner-fixture")).map_or(0,|(_,count)|count);
        assert_eq!(count(),2);
        assert_eq!(resources.video_streams.load(SeqCst), 2);
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(resources.video_streams.load(SeqCst), 1);
        drop(other_stream);
        assert_eq!(count(),0,"file-only connections cannot leave a sharing badge");
        publish(0xfed10001,&resources,"closed");
        assert_eq!(resources.video_streams.load(SeqCst), 0);
        assert_eq!(resources.workers.load(SeqCst), 1);
        drop(file_worker);
        assert_eq!(resources.workers.load(SeqCst), 0);
    }
    #[test]
    fn connection_diagnostics_keep_unobserved_sender_unknown_and_export_observed_counters() {
        let resources=Arc::new(Resources::default());
        publish(90_991,&resources,"open");
        assert!(inspect_connection(90_991).remove(0).scheduled_data.is_none());
        *resources.scheduled_data.lock().unwrap()=Some(ScheduledDataObservation {
            observed_at_ms:1234,
            stats:remote_core::scheduled_sender::ScheduledDataSenderStats {
                scheduler_dropped_stale_realtime:7,scheduler_dropped_realtime_capacity:2,
                ..Default::default()
            },
        });
        let observed=inspect_connection(90_991).remove(0).scheduled_data.unwrap();
        assert_eq!(observed.observed_at_ms,1234);
        assert_eq!(observed.stats.scheduler_dropped_stale_realtime,7);
        assert_eq!(observed.stats.scheduler_dropped_realtime_capacity,2);
    }
    #[test]
    fn focus_probe_counters_are_exported_and_older_snapshots_default_them() {
        let resources=Arc::new(Resources::default());
        let focus=&resources.focus_probe;
        focus.request(true);focus.request(false);focus.accepted_primary_release();
        focus.exact_match();focus.scheduled();focus.expired_before_ax();focus.expired_after_ax();
        focus.invalidated_before_ax();focus.probe_unavailable();focus.probe_busy();
        focus.editable_result(true);focus.editable_result(false);focus.result_queue_dropped();
        focus.reply_attempted();focus.reply_rejected();focus.ax_timing(std::time::Duration::from_millis(35));
        publish(90_992,&resources,"open");
        let snapshot=inspect_connection(90_992).remove(0);
        assert_eq!(snapshot.focus_probe.valid_requests,1);
        assert_eq!(snapshot.focus_probe.rejected_requests,1);
        assert_eq!(snapshot.focus_probe.accepted_primary_releases,1);
        assert_eq!(snapshot.focus_probe.exact_matches,1);
        assert_eq!(snapshot.focus_probe.scheduled,1);
        assert_eq!(snapshot.focus_probe.expired_before_ax,1);
        assert_eq!(snapshot.focus_probe.expired_after_ax,1);
        assert_eq!(snapshot.focus_probe.editable_results,1);
        assert_eq!(snapshot.focus_probe.noneditable_results,1);
        assert_eq!(snapshot.focus_probe.reply_attempts,1);
        assert_eq!(snapshot.focus_probe.reply_rejections,1);
        assert_eq!(snapshot.focus_probe.ax_timing.calls,1);
        assert_eq!(snapshot.focus_probe.ax_timing.max_us,35_000);

        let mut old=serde_json::to_value(snapshot).unwrap();
        old.as_object_mut().unwrap().remove("focus_probe");
        let old:ConnectionLifecycleSnapshot=serde_json::from_value(old).unwrap();
        assert_eq!(old.focus_probe.valid_requests,0);
        assert_eq!(old.focus_probe.ax_timing.calls,0);
    }
    #[cfg(target_os = "macos")]
    struct CallbackOwner(Arc<std::sync::atomic::AtomicBool>);
    #[cfg(target_os = "macos")]
    impl Drop for CallbackOwner {
        fn drop(&mut self) {
            self.0.store(true, SeqCst);
        }
    }
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn actual_owned_callback_bridge_withheld_completion_keeps_owner_without_runtime_waiter() {
        use screencapturekit::utils::completion::OwnedUnitCompletion;
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let resources = Arc::new(Resources::default());
        resources.native_streams.store(1, SeqCst);
        let (receipt, completion) = NativeStopCompletion::new(Some(resources.clone()));
        let context = OwnedUnitCompletion::new(CallbackOwner(dropped.clone()), move |result| {
            completion.complete(result)
        });
        assert!(receipt.wait().await.unwrap_err().contains("unknown"));
        assert!(!dropped.load(SeqCst));
        assert_eq!(resources.native_streams.load(SeqCst), 1);
        assert_eq!(resources.native_stops_pending.load(SeqCst), 1);
        // The same runtime thread still schedules unrelated IPC/timer work.
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            tokio::time::sleep(std::time::Duration::from_millis(5)),
        )
        .await
        .unwrap();
        unsafe {
            OwnedUnitCompletion::<CallbackOwner>::complete_context(context, true, std::ptr::null());
        }
        receipt.wait().await.unwrap();
        assert!(dropped.load(SeqCst));
        assert_eq!(resources.native_streams.load(SeqCst), 0);
        assert_eq!(resources.native_stops_completed.load(SeqCst), 1);
    }
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn actual_owned_callback_bridge_error_keeps_owner_and_failure_counts() {
        use screencapturekit::utils::completion::OwnedUnitCompletion;
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let resources = Arc::new(Resources::default());
        resources.native_streams.store(1, SeqCst);
        let (receipt, completion) = NativeStopCompletion::new(Some(resources.clone()));
        let context = OwnedUnitCompletion::new(CallbackOwner(dropped.clone()), move |result| {
            completion.complete(result)
        });
        unsafe {
            OwnedUnitCompletion::<CallbackOwner>::complete_context(
                context,
                false,
                c"test native failure".as_ptr(),
            );
        }
        assert!(receipt.wait().await.is_err());
        assert!(receipt.wait().await.is_err());
        assert!(!dropped.load(SeqCst));
        assert_eq!(resources.native_streams.load(SeqCst), 1);
        assert_eq!(resources.native_stops_pending.load(SeqCst), 0);
        assert_eq!(resources.native_stop_errors.load(SeqCst), 1);
        // Release only this synthetic owner after asserting the production
        // bridge's retained-error behavior. No SCStream or OS capture exists.
        unsafe {
            drop(Box::from_raw(
                context.cast::<OwnedUnitCompletion<CallbackOwner>>(),
            ));
        }
        assert!(dropped.load(SeqCst));
    }
    #[test]
    fn unknown_connection_does_not_enumerate_and_expired_terminal_is_hidden() {
        assert!(inspect_connection(987654).is_empty());
        let resource = Arc::new(Resources::default());
        publish(987655, &resource, "closed");
        registry().lock().unwrap().get_mut(&987655).unwrap()[0].at = 0;
        assert!(inspect_connection(987655).is_empty());
    }
}
