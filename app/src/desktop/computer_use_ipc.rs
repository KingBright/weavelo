//! Same-user observation and bounded input bridge for the existing Octendra desktop authority.
//! Actor fields are not credentials: Octendra authenticates its task/executor;
//! private IPC authenticates the local user and the owner checks the exact source.
use super::{
    computer_use::Access,
    computer_use_mcp::SnapshotRefStore,
    original_owner::{ComputerUseMode, OriginalOwner, SourceViewBinding},
};
use protocol::session::CaptureSource;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// Diagnostic wall-time projection only. Authority and freshness keep using Instant.
fn decoded_frame_utc_us(decoded: Instant, now: Instant, wall: SystemTime) -> Option<u64> {
    let age = u64::try_from(now.checked_duration_since(decoded)?.as_micros()).ok()?;
    u64::try_from(wall.duration_since(UNIX_EPOCH).ok()?.as_micros()).ok()?.checked_sub(age)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Actor {
    pub owner: String,
    pub task_id: String,
    pub executor_device_id: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Target {
    pub peer_device_id: String,
    pub source: CaptureSource,
    pub connection_id: u32,
}
impl Target {
    pub fn desktop_id(&self) -> String {
        let source = match self.source {
            CaptureSource::MainDisplay => "main".into(),
            CaptureSource::Display(id) => format!("display:{id}"),
            CaptureSource::Window(id) => format!("window:{id}"),
            CaptureSource::Camera(id) => format!("camera:{id}"),
        };
        format!(
            "weavelo:{}:{source}:{}",
            self.connection_id, self.peer_device_id
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Call {
    Open {
        actor: Actor,
        target: Target,
        lease_id: String,
        lease_ms: u32,
        max_bytes: usize,
        #[serde(default)]
        control: bool,
    },
    Observe {
        actor: Actor,
        lease_id: String,
    },
    Zoom {
        actor: Actor,
        lease_id: String,
        source_revision: u64,
        frame_sequence: u64,
        region: [u16; 4],
    },
    Snapshot {
        actor: Actor,
        lease_id: String,
        #[serde(default)]
        skeleton: bool,
        #[serde(default)]
        interactive_only: bool,
        #[serde(default)]
        root_ref: Option<String>,
    },
    Act {
        actor: Actor,
        lease_id: String,
        source_revision: u64,
        frame_sequence: u64,
        action: super::computer_use::Action,
    },
    BatchAct {
        actor: Actor,
        lease_id: String,
        source_revision: u64,
        frame_sequence: u64,
        actions: Vec<super::computer_use::Action>,
        #[serde(default)]
        keep_lease: bool,
    },
    Validate {
        actor: Actor,
        lease_id: String,
    },
    Close {
        actor: Actor,
        lease_id: String,
    },
}
struct Lease {
    caller: super::computer_use_authority::Caller,
    watchdog: tokio::task::JoinHandle<()>,
    owner: Arc<OriginalOwner>,
    scope: super::original_owner::ComputerUseObservation,
    actor: Actor,
    target: Target,
    id: String,
    expires: Instant,
    maximum: usize,
    sequence: u64,
    control: bool,
    pending: Option<PendingFrame<super::original_owner::ComputerUseObservation>>,
    ref_store: SnapshotRefStore,
    binding: SourceViewBinding,
}
struct PendingFrame<T> {
    revision: u64,
    sequence: u64,
    observation: T,
}
fn take_frame<T>(
    pending: &mut Option<PendingFrame<T>>,
    revision: u64,
    sequence: u64,
) -> Result<T, &'static str> {
    if !pending
        .as_ref()
        .is_some_and(|p| p.revision == revision && p.sequence == sequence)
    {
        return Err("frame_mismatch_or_consumed");
    }
    Ok(pending.take().unwrap().observation)
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.watchdog.abort();
        self.owner.revoke_computer_use();
        if self.owner.snapshot().computer_use_status.mode != Some(ComputerUseMode::PausedByHuman) {
            self.owner.set_computer_use_active(None, None);
        }
    }
}
pub(crate) struct Dispatcher(
    Mutex<Option<Lease>>,
    super::computer_use_authority::Authority,
);
pub(crate) struct Output {
    pub body: serde_json::Value,
    pub pixels: Option<Arc<Vec<u8>>>,
}
fn uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
        && value.bytes().any(|b| b != b'0' && b != b'-')
}
impl Call {
    pub fn identity(&self) -> (&Actor, &str) {
        match self {
            Self::Open {
                actor, lease_id, ..
            }
            | Self::Act {
                actor, lease_id, ..
            }
            | Self::BatchAct {
                actor, lease_id, ..
            }
            | Self::Zoom {
                actor, lease_id, ..
            }
            | Self::Snapshot {
                actor, lease_id, ..
            }
            | Self::Observe { actor, lease_id }
            | Self::Validate { actor, lease_id }
            | Self::Close { actor, lease_id } => (actor, lease_id),
        }
    }
    pub fn valid(&self) -> bool {
        let (actor, id) = self.identity();
        if actor.owner.is_empty()
            || actor.owner.len() > 128
            || actor.owner.chars().any(char::is_control)
            || !uuid(&actor.task_id)
            || !uuid(&actor.executor_device_id)
            || !uuid(id)
        {
            return false;
        }
        match self {
            Self::Open {
                target,
                lease_ms,
                max_bytes,
                ..
            } => {
                target.connection_id > 0
                    && !target.peer_device_id.is_empty()
                    && target.peer_device_id.len() <= 128
                    && target
                        .peer_device_id
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_.:".contains(&c))
                    && (1..=30_000).contains(lease_ms)
                    && (1..=16 * 1024 * 1024).contains(max_bytes)
            }
            Self::Zoom {
                source_revision,
                frame_sequence,
                region,
                ..
            } => {
                *source_revision > 0
                    && *frame_sequence > 0
                    && region[0] < region[2]
                    && region[1] < region[3]
                    && region[2] <= 1000
                    && region[3] <= 1000
            }
            Self::Act {
                source_revision,
                frame_sequence,
                action,
                ..
            } => *source_revision > 0 && *frame_sequence > 0 && action.valid(),
            Self::BatchAct {
                source_revision,
                frame_sequence,
                actions,
                ..
            } => {
                *source_revision > 0
                    && *frame_sequence > 0
                    && !actions.is_empty()
                    && actions.len() <= super::computer_use::MAX_BATCH_ACTIONS
                    && actions.iter().all(super::computer_use::Action::valid)
            }
            _ => true,
        }
    }
}
impl Dispatcher {
    pub fn new(directory: &std::path::Path, generation: &str, hash: &str) -> Self {
        Self(
            Mutex::new(None),
            super::computer_use_authority::Authority::new(directory, generation, hash),
        )
    }
    pub fn configured(&self) -> bool {
        self.1.configured()
    }
    pub fn dispatch(
        self: &Arc<Self>,
        owner: &Arc<OriginalOwner>,
        call: Call,
        peer: Option<i32>,
        cancelled: impl Fn() -> bool,
    ) -> Output {
        self.dispatch_owners(std::slice::from_ref(owner), call, peer, cancelled)
    }
    pub fn dispatch_owners(
        self: &Arc<Self>,
        owners: &[Arc<OriginalOwner>],
        call: Call,
        peer: Option<i32>,
        cancelled: impl Fn() -> bool,
    ) -> Output {
        let result = (|| {
            if !call.valid() {
                return Err("invalid_request");
            }
            let saved = self
                .0
                .lock()
                .map_err(|_| "state_unavailable")?
                .as_ref()
                .map(|l| l.caller.clone());
            let caller = self
                .1
                .verify(&call, peer, saved.as_ref())
                .inspect_err(|_| {
                    // Observed revocation cannot be revived by restoring an authority file.
                    if saved.as_ref().is_some_and(|caller| caller.owns_peer(peer)) {
                        if let Ok(mut state) = self.0.lock() {
                            state.take();
                        }
                    }
                })?;
            let output = self.run(owners, call.clone(), caller.clone(), &cancelled)?;
            if output.pixels.is_some() {
                self.1.verify(&call, peer, Some(&caller)).inspect_err(|_| {
                    // A failed post-readback authorization check also retires
                    // its exact lease; restoring files cannot revive that frame.
                    if let Ok(mut state) = self.0.lock() {
                        if state.as_ref().is_some_and(|lease| {
                            lease.caller == caller && lease.id == call.identity().1
                        }) {
                            state.take();
                        }
                    }
                })?;
            }
            Ok(output)
        })();
        result.unwrap_or_else(|code| Output {
            body: serde_json::json!({"ok":false,"code":code}),
            pixels: None,
        })
    }
    fn run(
        self: &Arc<Self>,
        owners: &[Arc<OriginalOwner>],
        call: Call,
        caller: super::computer_use_authority::Caller,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Output, &'static str> {
        if !call.valid() {
            return Err("invalid_request");
        }
        let mut state = self.0.lock().map_err(|_| "state_unavailable")?;
        if state.as_ref().is_some_and(|l| Instant::now() >= l.expires) {
            state.take();
        }
        match call {
            Call::Open {
                actor,
                target,
                lease_id,
                lease_ms,
                max_bytes,
                control,
            } => {
                if state.is_some() {
                    return Err("observer_busy");
                }
                let mut matched = owners.iter().filter_map(|candidate| {
                    candidate
                        .computer_use_target_binding(
                            &target.peer_device_id,
                            target.source,
                            target.connection_id,
                        )
                        .map(|binding| (candidate.clone(), binding))
                });
                let (owner, binding) = matched.next().ok_or("target_not_selected")?;
                if matched.next().is_some() {
                    return Err("target_not_selected");
                }
                let mut scope = owner
                    .observe_bound_computer_use(
                        &binding,
                        &target.peer_device_id,
                        target.source,
                        Access::Observe,
                    )
                    .map_err(|_| "session_unavailable")?;
                owner
                    .validate_bound_computer_use(&mut scope)
                    .map_err(|_| "frame_expired")?;
                owner.set_computer_use_active(
                    Some(if control {
                        ComputerUseMode::Controlling
                    } else {
                        ComputerUseMode::Observing
                    }),
                    Some(if control {
                        "AI Controlling — Move mouse or press Esc to take over".into()
                    } else {
                        "AI Observing screen".into()
                    }),
                );
                let expires = Instant::now() + Duration::from_millis(lease_ms.into());
                let weak = Arc::downgrade(self);
                let key = lease_id.clone();
                let watchdog = tokio::spawn(async move {
                    tokio::time::sleep_until(tokio::time::Instant::from_std(expires)).await;
                    if let Some(dispatcher) = weak.upgrade() {
                        if let Ok(mut state) = dispatcher.0.lock() {
                            if state
                                .as_ref()
                                .is_some_and(|l| l.id == key && l.expires == expires)
                            {
                                state.take();
                            }
                        }
                    }
                });
                *state = Some(Lease {
                    caller,
                    watchdog,
                    owner,
                    scope,
                    actor,
                    target,
                    id: lease_id.clone(),
                    expires,
                    maximum: max_bytes,
                    sequence: 0,
                    control,
                    pending: None,
                    ref_store: SnapshotRefStore::new(),
                    binding,
                });
                Ok(Output {
                    body: serde_json::json!({"ok":true,"lease_id":lease_id,"lease_ms":lease_ms}),
                    pixels: None,
                })
            }
            Call::Close { actor, lease_id } => {
                if state.is_some()
                    && !state
                        .as_ref()
                        .is_some_and(|l| l.actor == actor && l.id == lease_id)
                {
                    return Err("lease_mismatch");
                }
                state.take();
                Ok(Output {
                    body: serde_json::json!({"ok":true}),
                    pixels: None,
                })
            }
            Call::Act {
                actor,
                lease_id,
                source_revision,
                frame_sequence,
                action,
            } => {
                let lease = state
                    .as_mut()
                    .filter(|l| l.actor == actor && l.id == lease_id && l.control)
                    .ok_or("control_lease_mismatch")?;
                let resolved_action = action.resolve_with(|target_ref| {
                    lease
                        .ref_store
                        .resolve(target_ref)
                        .map(|node| (grid_to_input(node.center_norm.0),grid_to_input(node.center_norm.1)))
                })?;
                let owner = lease.owner.clone();
                owner
                    .validate_bound_computer_use(&mut lease.scope)
                    .map_err(|_| "source_changed")?;
                let mut observation =
                    take_frame(&mut lease.pending, source_revision, frame_sequence)?;
                let remaining = caller.expires_at_ms.saturating_sub(crate::unix_now_ms());
                let deadline = lease
                    .expires
                    .min(Instant::now() + Duration::from_millis(remaining));
                // Retire the entire lease before any effect. The owner revokes its
                // epoch on dispatch, so refreshing the old lease would revive a fence.
                // A new gesture requires close/open/observe through existing authority.
                let _retired=state.take();
                owner
                    .act_bound_computer_use(&mut observation, resolved_action, deadline, cancelled)
                    .map_err(|error| match error {
                        super::computer_use::Rejection::QueueRejected { .. } => "outcome_unknown",
                        _ => "input_rejected",
                    })?;
                Ok(Output {
                    body: serde_json::json!({"ok":true,"effect_confirmed":false}),
                    pixels: None,
                })
            }
            Call::BatchAct {
                actor,
                lease_id,
                source_revision,
                frame_sequence,
                actions,
                keep_lease,
            } => {
                let lease = state
                    .as_mut()
                    .filter(|l| l.actor == actor && l.id == lease_id && l.control)
                    .ok_or("control_lease_mismatch")?;
                let mut resolved_actions = Vec::with_capacity(actions.len());
                for act in &actions {
                    let resolved = act.resolve_with(|target_ref| {
                        lease
                            .ref_store
                            .resolve(target_ref)
                            .map(|node| (grid_to_input(node.center_norm.0),grid_to_input(node.center_norm.1)))
                    })?;
                    resolved_actions.push(resolved);
                }
                let owner = lease.owner.clone();
                owner
                    .validate_bound_computer_use(&mut lease.scope)
                    .map_err(|_| "source_changed")?;
                let mut observation =
                    take_frame(&mut lease.pending, source_revision, frame_sequence)?;
                let remaining = caller.expires_at_ms.saturating_sub(crate::unix_now_ms());
                let deadline = lease
                    .expires
                    .min(Instant::now() + Duration::from_millis(remaining));
                let binding = lease.binding.clone();
                let target = lease.target.clone();
                drop(state);
                let outcome = owner
                    .act_batch_bound_computer_use(&mut observation, &resolved_actions, deadline, cancelled)
                    .map_err(|error| match error {
                        super::computer_use::Rejection::QueueRejected { .. } => "outcome_unknown",
                        _ => "input_rejected",
                    })?;
                if keep_lease {
                    let mut state = self.0.lock().map_err(|_| "state_unavailable")?;
                    // Refresh the lease scope fence to the post-batch owner epoch so a
                    // subsequent Observe in the same lease can mint a new frame receipt,
                    // or drop the lease if the source/session became unavailable.
                    match owner.observe_bound_computer_use(
                        &binding,
                        &target.peer_device_id,
                        target.source,
                        Access::Observe,
                    ) {
                        Ok(new_scope) => {
                            if let Some(active_lease) = state
                                .as_mut()
                                .filter(|l| l.actor == actor && l.id == lease_id)
                            {
                                active_lease.scope = new_scope;
                            }
                        }
                        Err(_) => {
                            state.take();
                        }
                    }
                }
                if !keep_lease {
                    let mut state=self.0.lock().map_err(|_|"state_unavailable")?;
                    if state.as_ref().is_some_and(|lease|lease.actor==actor&&lease.id==lease_id){state.take();}
                }
                let completed = outcome.failed_at.is_none();
                Ok(Output {
                    body: serde_json::json!({
                        "ok": completed,
                        "completed": completed,
                        "failed_at": outcome.failed_at,
                        "effect_confirmed": false,
                        "steps": outcome.results,
                    }),
                    pixels: None,
                })
            }
            Call::Zoom {
                actor,
                lease_id,
                source_revision,
                frame_sequence,
                region,
            } => {
                let lease = state
                    .as_mut()
                    .filter(|l| l.actor == actor && l.id == lease_id)
                    .ok_or("lease_mismatch")?;
                let owner = lease.owner.clone();
                owner
                    .validate_bound_computer_use(&mut lease.scope)
                    .map_err(|_| "source_changed")?;
                let pending = lease
                    .pending
                    .as_mut()
                    .filter(|p| p.revision == source_revision && p.sequence == frame_sequence)
                    .ok_or("frame_mismatch_or_consumed")?;
                let maximum = lease.maximum;
                let cropped = owner
                    .zoom_bound_computer_use(&mut pending.observation, region, maximum)
                    .map_err(|_| "pixels_unavailable")?;
                let decoded_at_utc_us = decoded_frame_utc_us(
                    pending.observation.frame().decoded_at, Instant::now(), SystemTime::now());
                let body = serde_json::json!({
                    "ok": true,
                    "lease_id": lease_id,
                    "source_revision": source_revision,
                    "frame_sequence": frame_sequence,
                    "region": region,
                    "decoded_at_utc_us": decoded_at_utc_us,
                    "size": [cropped.layout().width(), cropped.layout().height()],
                    "bytes": cropped.layout().bytes(),
                    "encoding": match cropped.color() {
                        client::observation_pixels::ColorMetadata::Srgb => "rgba8_srgb",
                        client::observation_pixels::ColorMetadata::Unspecified => "rgba8",
                    }
                });
                Ok(Output {
                    body,
                    pixels: Some(cropped.into_pixels()),
                })
            }
            Call::Snapshot {
                actor,
                lease_id,
                skeleton,
                interactive_only,
                root_ref,
            } => {
                let lease = state
                    .as_mut()
                    .filter(|l| l.actor == actor && l.id == lease_id)
                    .ok_or("lease_mismatch")?;
                let owner = lease.owner.clone();
                owner
                    .validate_bound_computer_use(&mut lease.scope)
                    .map_err(|_| "source_changed")?;
                let ctx = owner
                    .computer_use_snapshot_context()
                    .ok_or("session_unavailable")?;
                lease.sequence=lease.sequence.checked_add(1).ok_or("snapshot_sequence_exhausted")?;
                let body = lease.ref_store.build_session_snapshot(
                    &ctx,
                    lease.sequence,
                    skeleton,
                    interactive_only,
                    root_ref.as_deref(),
                )?;
                Ok(Output { body, pixels: None })
            }
            Call::Validate { actor, lease_id } => {
                let lease = state
                    .as_mut()
                    .filter(|l| l.actor == actor && l.id == lease_id)
                    .ok_or("lease_mismatch")?;
                let owner = lease.owner.clone();
                owner
                    .validate_bound_computer_use(&mut lease.scope)
                    .map_err(|_| "source_changed")?;
                Ok(Output {
                    body: serde_json::json!({"ok":true}),
                    pixels: None,
                })
            }
            Call::Observe { actor, lease_id } => {
                let lease = state
                    .as_mut()
                    .filter(|l| l.actor == actor && l.id == lease_id)
                    .ok_or("lease_mismatch")?;
                let owner = lease.owner.clone();
                owner
                    .validate_bound_computer_use(&mut lease.scope)
                    .map_err(|_| "source_changed")?;
                lease.pending.take();
                let (binding, target, maximum, control) = (
                    lease.binding.clone(),
                    lease.target.clone(),
                    lease.maximum,
                    lease.control,
                );
                drop(state);
                let mut observation = owner
                    .observe_bound_computer_use(
                        &binding,
                        &target.peer_device_id,
                        target.source,
                        if control {
                            Access::Control
                        } else {
                            Access::Observe
                        },
                    )
                    .map_err(|_| "session_unavailable")?;
                let revision = OriginalOwner::computer_use_source_revision(&observation);
                let decoded_at_utc_us = decoded_frame_utc_us(
                    observation.frame().decoded_at, Instant::now(), SystemTime::now());
                let pixels = owner
                    .export_bound_computer_use(&mut observation, maximum)
                    .map_err(|_| "pixels_unavailable")?;
                let mut state = self.0.lock().map_err(|_| "state_unavailable")?;
                let lease = state
                    .as_mut()
                    .filter(|l| l.actor == actor && l.id == lease_id && Instant::now() < l.expires)
                    .ok_or("lease_expired")?;
                if !Arc::ptr_eq(&lease.owner, &owner) {
                    return Err("source_changed");
                }
                owner
                    .validate_bound_computer_use(&mut lease.scope)
                    .map_err(|_| "source_changed")?;
                lease.sequence = lease.sequence.checked_add(1).ok_or("sequence_exhausted")?;
                lease.pending = Some(PendingFrame {
                    revision,
                    sequence: lease.sequence,
                    observation,
                });
                let body = serde_json::json!({"ok":true,"lease_id":lease_id,"source_revision":revision,
                "frame_sequence":lease.sequence,"decoded_at_utc_us":decoded_at_utc_us,
                "size":[pixels.layout().width(),pixels.layout().height()],
                "bytes":pixels.layout().bytes(),"encoding":match pixels.color() {
                    client::observation_pixels::ColorMetadata::Srgb=>"rgba8_srgb",
                    client::observation_pixels::ColorMetadata::Unspecified=>"rgba8",
                }});
                Ok(Output {
                    body,
                    pixels: Some(pixels.into_pixels()),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostic_decode_time_keeps_clock_domains_and_rejects_invalid_projections() {
        let now = Instant::now();
        let wall = UNIX_EPOCH + Duration::from_secs(20);
        assert_eq!(decoded_frame_utc_us(now - Duration::from_millis(25), now, wall), Some(19_975_000));
        assert_eq!(decoded_frame_utc_us(now + Duration::from_micros(1), now, wall), None);
        assert_eq!(decoded_frame_utc_us(now - Duration::from_secs(21), now, wall), None);
        assert_eq!(decoded_frame_utc_us(now, now, UNIX_EPOCH - Duration::from_secs(1)), None);
    }
    #[test]
    fn original_frame_receipt_is_exact_and_single_use() {
        let mut pending = Some(PendingFrame {
            revision: 7,
            sequence: 3,
            observation: "original",
        });
        assert!(take_frame(&mut pending, 8, 3).is_err());
        assert!(take_frame(&mut pending, 7, 4).is_err());
        assert_eq!(take_frame(&mut pending, 7, 3), Ok("original"));
        assert!(take_frame(&mut pending, 7, 3).is_err());
    }

    #[test]
    fn strict_actor_keeps_executor_distinct_from_peer_and_rejects_incomplete_calls() {
        let actor = Actor {
            owner: "fixture".into(),
            task_id: "11111111-1111-1111-1111-111111111111".into(),
            executor_device_id: "22222222-2222-2222-2222-222222222222".into(),
        };
        let call = Call::Open {
            actor: actor.clone(),
            target: Target {
                peer_device_id: "synthetic-weavelo-peer".into(),
                source: CaptureSource::Window(42),
                connection_id: 7,
            },
            lease_id: "33333333-3333-3333-3333-333333333333".into(),
            lease_ms: 30_000,
            max_bytes: 1024,
            control: false,
        };
        assert!(call.valid());
        let mut value = serde_json::to_value(call).unwrap();
        value["actor"]["executor_device_id"] = serde_json::json!("synthetic-weavelo-peer");
        assert!(!serde_json::from_value::<Call>(value).unwrap().valid());
        assert!(
            serde_json::from_str::<Call>(
                r#"{"operation":"observe","actor":{},"lease_id":"x","act":true}"#
            )
            .is_err()
        );
    }

    #[test]
    fn zoom_and_batch_act_calls_validate_bounds_and_actions() {
        let actor = Actor {
            owner: "fixture".into(),
            task_id: "11111111-1111-1111-1111-111111111111".into(),
            executor_device_id: "22222222-2222-2222-2222-222222222222".into(),
        };
        let zoom_ok = Call::Zoom {
            actor: actor.clone(),
            lease_id: "33333333-3333-3333-3333-333333333333".into(),
            source_revision: 1,
            frame_sequence: 1,
            region: [100, 100, 400, 400],
        };
        assert!(zoom_ok.valid());
        let zoom_bad = Call::Zoom {
            actor: actor.clone(),
            lease_id: "33333333-3333-3333-3333-333333333333".into(),
            source_revision: 1,
            frame_sequence: 1,
            region: [500, 100, 400, 400],
        };
        assert!(!zoom_bad.valid());

        let batch_ok = Call::BatchAct {
            actor,
            lease_id: "33333333-3333-3333-3333-333333333333".into(),
            source_revision: 1,
            frame_sequence: 1,
            actions: vec![
                super::super::computer_use::Action::Move { x: 250, y: 300 },
                super::super::computer_use::Action::Click {
                    x: 250,
                    y: 300,
                    button: 0,
                },
                super::super::computer_use::Action::CommitText {
                    text: "hello".into(),
                },
            ],
            keep_lease: true,
        };
        assert!(batch_ok.valid());
    }
}

fn grid_to_input(value:u16)->u16 { ((u32::from(value)*65535+500)/1000).min(65535) as u16 }
