//! Owner-local observe -> bounded action adapter, not a network authorization API.
//! Only a trusted owner may mint a receipt after checking its existing session.
//! The owner holds its session lock across current-state validation and dispatch.
//! No focus, permission, source grant or presentation requirement is relaxed here.
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

const MAX_OBSERVATION_AGE: Duration = Duration::from_secs(30);
pub(crate) const MAX_BATCH_ACTIONS: usize = 32;
pub(crate) const CASCADE_ABORT_MESSAGE: &str =
    "Not executed: an earlier computer action in this turn failed.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Access {
    Observe,
    Control,
}

/// Normalized observed-image coordinates (`0..=65535`) or snapshot-scoped `@<snapshot_id>:e<id>` refs,
/// never raw desktop pixels or an implicit default target.
/// Supports both single-step actions and serial fail-closed batch execution.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Action {
    Click {
        x: u16,
        y: u16,
        #[serde(default)]
        button: u8,
    },
    ClickRef {
        target_ref: String,
        #[serde(default)]
        button: u8,
    },
    DoubleClick {
        x: u16,
        y: u16,
        #[serde(default)]
        button: u8,
    },
    DoubleClickRef {
        target_ref: String,
        #[serde(default)]
        button: u8,
    },
    TripleClick {
        x: u16,
        y: u16,
        #[serde(default)]
        button: u8,
    },
    Move {
        x: u16,
        y: u16,
    },
    MoveRef {
        target_ref: String,
    },
    Drag {
        from_x: u16,
        from_y: u16,
        to_x: u16,
        to_y: u16,
    },
    KeyPress {
        key_code: u16,
    },
    KeyCombo {
        key_code: u16,
        modifiers: u8,
    },
    CommitText {
        text: String,
    },
    Scroll {
        x: u16,
        y: u16,
        delta_x: i32,
        delta_y: i32,
    },
    Wait {
        duration_ms: u32,
    },
    WaitForStability {
        timeout_ms: u32,
    },
}

impl Action {
    pub(crate) fn valid(&self) -> bool {
        match self {
            Self::Click { button, .. }
            | Self::DoubleClick { button, .. }
            | Self::TripleClick { button, .. } => *button <= 2,
            Self::ClickRef { target_ref, button }
            | Self::DoubleClickRef { target_ref, button } => {
                *button <= 2 && Self::valid_ref_syntax(target_ref)
            }
            Self::MoveRef { target_ref } => Self::valid_ref_syntax(target_ref),
            Self::Move { .. } | Self::Drag { .. } => true,
            Self::Scroll {
                delta_x, delta_y, ..
            } => delta_x.unsigned_abs() <= 1200 && delta_y.unsigned_abs() <= 1200,
            Self::KeyPress { key_code } | Self::KeyCombo { key_code, .. } => {
                *key_code < 128 && ![54, 55, 56, 57, 58, 59, 60, 61, 62, 63].contains(key_code)
            }
            Self::CommitText { text } => !text.is_empty() && text.len() <= 1024,
            Self::Wait { duration_ms } => *duration_ms <= 5_000,
            Self::WaitForStability { timeout_ms } => (1..=5_000).contains(timeout_ms),
        }
    }

    fn valid_ref_syntax(target_ref: &str) -> bool {
        let trimmed = target_ref.trim().trim_start_matches('@');
        !trimmed.is_empty()
            && trimmed.len() <= 64
            && trimmed.contains(':')
            && trimmed
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b':' || b == b'_' || b == b'-')
    }

    /// Resolve any snapshot-scoped `@<snapshot_id>:e<id>` reference into concrete
    /// normalized `(x, y)` coordinates using the lease's active `SnapshotRefStore`.
    pub(crate) fn resolve_with(
        &self,
        mut resolver: impl FnMut(&str) -> Result<(u16, u16), &'static str>,
    ) -> Result<Self, &'static str> {
        match self {
            Self::ClickRef { target_ref, button } => {
                let (x, y) = resolver(target_ref)?;
                Ok(Self::Click {
                    x,
                    y,
                    button: *button,
                })
            }
            Self::DoubleClickRef { target_ref, button } => {
                let (x, y) = resolver(target_ref)?;
                Ok(Self::DoubleClick {
                    x,
                    y,
                    button: *button,
                })
            }
            Self::MoveRef { target_ref } => {
                let (x, y) = resolver(target_ref)?;
                Ok(Self::Move { x, y })
            }
            other => Ok(other.clone()),
        }
    }

    pub(crate) fn visual_feedback(
        &self,
    ) -> Option<((u16, u16), client::observation_pixels::ReticleStyle, String)> {
        use client::observation_pixels::ReticleStyle;
        match self {
            Self::Click { x, y, .. } => Some(((*x, *y), ReticleStyle::Click, "Click".into())),
            Self::DoubleClick { x, y, .. } => {
                Some(((*x, *y), ReticleStyle::Click, "Double Click".into()))
            }
            Self::TripleClick { x, y, .. } => {
                Some(((*x, *y), ReticleStyle::Click, "Triple Click".into()))
            }
            Self::Move { x, y } => Some(((*x, *y), ReticleStyle::Hover, "Move".into())),
            Self::Drag {
                from_x,
                from_y,
                to_x,
                to_y,
            } => Some((
                (*to_x, *to_y),
                ReticleStyle::Drag {
                    from_x: *from_x,
                    from_y: *from_y,
                },
                "Drag".into(),
            )),
            Self::Scroll { x, y, .. } => Some(((*x, *y), ReticleStyle::Hover, "Scroll".into())),
            Self::CommitText { .. } | Self::KeyPress { .. } | Self::KeyCombo { .. }=>None,
            Self::ClickRef { .. }
            | Self::DoubleClickRef { .. }
            | Self::MoveRef { .. }
            | Self::Wait { .. }
            | Self::WaitForStability { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    Move { x: u16, y: u16 },
    Button { button: u8, pressed: bool },
    Key { key_code: u16, pressed: bool, modifiers: u8 },
    Modifiers { modifiers: u8 },
    Scroll { delta_x: i32, delta_y: i32 },
    Text(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rejection {
    NoSession,
    StaleSession,
    Unpaired,
    FrameUnavailable,
    InputNotAllowed,
    InputBusy,
    Expired,
    Consumed,
    InvalidAction,
    StaleSnapshotRef,
    QueueRejected { release_queued: bool },
}

impl Rejection {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::NoSession => "no_session",
            Self::StaleSession => "stale_session",
            Self::Unpaired => "unpaired",
            Self::FrameUnavailable => "frame_unavailable",
            Self::InputNotAllowed => "input_not_allowed",
            Self::InputBusy => "input_busy",
            Self::Expired => "expired",
            Self::Consumed => "consumed",
            Self::InvalidAction => "invalid_action",
            Self::StaleSnapshotRef => "stale_snapshot_ref",
            Self::QueueRejected { .. } => "outcome_unknown",
        }
    }

    pub(crate) fn retry_safe(self) -> bool {
        !matches!(self, Self::QueueRejected { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Queued {
    pub events: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct BatchStepResult {
    pub index: usize,
    pub is_error: bool,
    pub events_queued: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_safe: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BatchOutcome {
    pub results: Vec<BatchStepResult>,
    pub failed_at: Option<usize>,
}

/// Must retain the exact observed host input grant, including its epoch.
/// A failed send may already have updated local held state; always attempt an
/// exact-grant release. Never substitute the replacement source's grant.
pub(crate) trait InputPort {
    fn send(&mut self, event: Event) -> bool;
    fn release(&mut self) -> bool;
}

fn expand_action(action: &Action) -> Result<Vec<Event>, Rejection> {
    if !action.valid() {
        return Err(Rejection::InvalidAction);
    }
    match action {
        Action::Click { x, y, button } => Ok(vec![
            Event::Move { x: *x, y: *y },
            Event::Button {
                button: *button,
                pressed: true,
            },
            Event::Button {
                button: *button,
                pressed: false,
            },
        ]),
        Action::DoubleClick { x, y, button } => Ok(vec![
            Event::Move { x: *x, y: *y },
            Event::Button {
                button: *button,
                pressed: true,
            },
            Event::Button {
                button: *button,
                pressed: false,
            },
            Event::Button {
                button: *button,
                pressed: true,
            },
            Event::Button {
                button: *button,
                pressed: false,
            },
        ]),
        Action::TripleClick { x, y, button } => Ok(vec![
            Event::Move { x: *x, y: *y },
            Event::Button {
                button: *button,
                pressed: true,
            },
            Event::Button {
                button: *button,
                pressed: false,
            },
            Event::Button {
                button: *button,
                pressed: true,
            },
            Event::Button {
                button: *button,
                pressed: false,
            },
            Event::Button {
                button: *button,
                pressed: true,
            },
            Event::Button {
                button: *button,
                pressed: false,
            },
        ]),
        Action::Move { x, y } => Ok(vec![Event::Move { x: *x, y: *y }]),
        Action::Drag {
            from_x,
            from_y,
            to_x,
            to_y,
        } => {
            let mid_x = ((u32::from(*from_x) + u32::from(*to_x)) / 2) as u16;
            let mid_y = ((u32::from(*from_y) + u32::from(*to_y)) / 2) as u16;
            Ok(vec![
                Event::Move {
                    x: *from_x,
                    y: *from_y,
                },
                Event::Button {
                    button: 0,
                    pressed: true,
                },
                Event::Move { x: mid_x, y: mid_y },
                Event::Move { x: *to_x, y: *to_y },
                Event::Button {
                    button: 0,
                    pressed: false,
                },
            ])
        }
        Action::KeyPress { key_code } => Ok(vec![
            Event::Key {
                key_code: *key_code,
                pressed: true,
                modifiers: 0,
            },
            Event::Key {
                key_code: *key_code,
                pressed: false,
                modifiers: 0,
            },
        ]),
        Action::KeyCombo {
            key_code,
            modifiers,
        } => {
            if *modifiers == 0 {
                Ok(vec![
                    Event::Key {
                        key_code: *key_code,
                        pressed: true,
                        modifiers: 0,
                    },
                    Event::Key {
                        key_code: *key_code,
                        pressed: false,
                        modifiers: 0,
                    },
                ])
            } else {
                Ok(vec![
                    Event::Modifiers {
                        modifiers: *modifiers,
                    },
                    Event::Key {
                        key_code: *key_code,
                        pressed: true,
                        modifiers: *modifiers,
                    },
                    Event::Key {
                        key_code: *key_code,
                        pressed: false,
                        modifiers: 0,
                    },
                    Event::Modifiers { modifiers: 0 },
                ])
            }
        }
        Action::CommitText { text } => Ok(vec![Event::Text(text.clone())]),
        Action::Scroll {
            x,
            y,
            delta_x,
            delta_y,
        } => Ok(vec![
            Event::Move { x: *x, y: *y },
            Event::Scroll {
                delta_x: *delta_x,
                delta_y: *delta_y,
            },
        ]),
        Action::Wait { .. } | Action::WaitForStability { .. } => Ok(Vec::new()),
        // Unresolved ref actions must be resolved via Action::resolve_with before expansion.
        Action::ClickRef { .. } | Action::DoubleClickRef { .. } | Action::MoveRef { .. } => {
            Err(Rejection::StaleSnapshotRef)
        }
    }
}

/// Process-local, non-serializable, single-use input receipt. Not an auth token.
/// Frame ownership preserves a snapshot only; it does not keep a channel alive.
pub(crate) struct Observation<C, Fence, Frame> {
    connection: Weak<C>,
    fence: Fence,
    frame: Arc<Frame>,
    access: Access,
    observed_at: Instant,
    consumed: bool,
}

impl<C, Fence: PartialEq, Frame> Observation<C, Fence, Frame> {
    pub(super) fn new(
        connection: &Arc<C>,
        fence: Fence,
        frame: Arc<Frame>,
        access: Access,
        observed_at: Instant,
    ) -> Self {
        Self {
            connection: Arc::downgrade(connection),
            fence,
            frame,
            access,
            observed_at,
            consumed: false,
        }
    }

    pub(crate) fn frame(&self) -> &Arc<Frame> {
        &self.frame
    }
    pub(crate) fn fence(&self) -> &Fence {
        &self.fence
    }
    pub(super) fn invalidate(&mut self) {
        self.consumed = true;
    }

    pub(super) fn validate_current(
        &mut self,
        connection: &Arc<C>,
        current: &Fence,
        now: Instant,
    ) -> Result<(), Rejection> {
        if self.consumed {
            return Err(Rejection::Consumed);
        }
        if now.saturating_duration_since(self.observed_at) > MAX_OBSERVATION_AGE {
            self.consumed = true;
            return Err(Rejection::Expired);
        }
        let Some(observed) = self.connection.upgrade() else {
            self.consumed = true;
            return Err(Rejection::StaleSession);
        };
        if !Arc::ptr_eq(&observed, connection) || &self.fence != current {
            self.consumed = true;
            return Err(Rejection::StaleSession);
        }
        Ok(())
    }

    pub(super) fn act(
        &mut self,
        connection: &Arc<C>,
        current: &Fence,
        now: Instant,
        input_allowed: bool,
        action: Action,
        port: &mut impl InputPort,
    ) -> Result<Queued, Rejection> {
        self.validate_current(connection, current, now)?;
        if self.access != Access::Control || !input_allowed {
            self.consumed = true;
            return Err(Rejection::InputNotAllowed);
        }
        let events = expand_action(&action)?;
        // Consume before the first effect; queue failure must not be retried
        // with this receipt, because some events may already have been accepted.
        self.consumed = true;
        let mut count = 0;
        for event in events {
            if !port.send(event) {
                return Err(Rejection::QueueRejected {
                    release_queued: port.release(),
                });
            }
            count += 1;
        }
        Ok(Queued { events: count })
    }

    /// Executes an ordered batch of actions against this single observation receipt.
    /// Enforces Anthropic `computer_toolset_20260801` cascading abort semantics:
    /// if step `k` fails, step `k` reports its exact failure and steps `k+1..N`
    /// are aborted with `CASCADE_ABORT_MESSAGE`.
    pub(crate) fn act_batch(
        &mut self,
        connection: &Arc<C>,
        current: &Fence,
        now: Instant,
        input_allowed: bool,
        actions: &[Action],
        port: &mut impl InputPort,
    ) -> Result<BatchOutcome, Rejection> {
        self.act_batch_with_wait(
            connection,
            current,
            now,
            input_allowed,
            actions,
            port,
            &mut |action| match action {
                Action::Wait{duration_ms}=>{std::thread::sleep(Duration::from_millis(u64::from(*duration_ms)));true},
                _=>false, // Visual stability requires the real owner's native observer.
            },
        )
    }

    pub(crate) fn act_batch_with_wait(
        &mut self,
        connection: &Arc<C>,
        current: &Fence,
        now: Instant,
        input_allowed: bool,
        actions: &[Action],
        port: &mut impl InputPort,
        wait_fn: &mut impl FnMut(&Action) -> bool,
    ) -> Result<BatchOutcome, Rejection> {
        if actions.is_empty() || actions.len() > MAX_BATCH_ACTIONS {
            return Err(Rejection::InvalidAction);
        }
        self.validate_current(connection, current, now)?;
        if self.access != Access::Control || !input_allowed {
            self.consumed = true;
            return Err(Rejection::InputNotAllowed);
        }
        self.consumed = true;
        let mut results = Vec::with_capacity(actions.len());
        let mut failed_at = None;
        for (index, action) in actions.iter().enumerate() {
            if failed_at.is_some() {
                results.push(BatchStepResult {
                    index,
                    is_error: true,
                    events_queued: 0,
                    error: Some(CASCADE_ABORT_MESSAGE.into()),
                    retry_safe: Some(true),
                });
                continue;
            }
            if let Action::Wait { duration_ms }
            | Action::WaitForStability {
                timeout_ms: duration_ms,
            } = action
            {
                if !action.valid() {
                    let _ = port.release();
                    failed_at = Some(index);
                    results.push(BatchStepResult {
                        index,
                        is_error: true,
                        events_queued: 0,
                        error: Some(Rejection::InvalidAction.code().into()),
                        retry_safe: Some(true),
                    });
                    continue;
                }
                if !wait_fn(action) {
                    let _ = port.release();
                    failed_at = Some(index);
                    results.push(BatchStepResult {
                        index,
                        is_error: true,
                        events_queued: 0,
                        error: Some(Rejection::Expired.code().into()),
                        retry_safe: Some(true),
                    });
                    continue;
                }
                results.push(BatchStepResult {
                    index,
                    is_error: false,
                    events_queued: 0,
                    error: None,
                    retry_safe: None,
                });
                continue;
            }
            let events = match expand_action(action) {
                Ok(events) => events,
                Err(rejection) => {
                    let _ = port.release();
                    failed_at = Some(index);
                    results.push(BatchStepResult {
                        index,
                        is_error: true,
                        events_queued: 0,
                        error: Some(rejection.code().into()),
                        retry_safe: Some(rejection.retry_safe()),
                    });
                    continue;
                }
            };
            let mut count = 0;
            let mut step_failed = None;
            for event in events {
                if !port.send(event) {
                    let released = port.release();
                    step_failed = Some(Rejection::QueueRejected {
                        release_queued: released,
                    });
                    break;
                }
                count += 1;
            }
            if let Some(rejection) = step_failed {
                failed_at = Some(index);
                results.push(BatchStepResult {
                    index,
                    is_error: true,
                    events_queued: count,
                    error: Some(rejection.code().into()),
                    retry_safe: Some(rejection.retry_safe()),
                });
            } else {
                results.push(BatchStepResult {
                    index,
                    is_error: false,
                    events_queued: count,
                    error: None,
                    retry_safe: None,
                });
            }
        }
        Ok(BatchOutcome { results, failed_at })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Fence = [u64; 7];
    #[derive(Default)]
    struct Recorder {
        sent: Vec<Event>,
        fail_at: Option<usize>,
        releases: usize,
        release_ok: bool,
    }
    impl InputPort for Recorder {
        fn send(&mut self, event: Event) -> bool {
            if self.fail_at == Some(self.sent.len()) {
                return false;
            }
            self.sent.push(event);
            true
        }
        fn release(&mut self) -> bool {
            self.releases += 1;
            self.release_ok
        }
    }

    fn observation(c: &Arc<u32>, access: Access, now: Instant) -> Observation<u32, Fence, &'static str> {
        Observation::new(c, [1; 7], Arc::new("frame"), access, now)
    }

    #[test]
    fn control_receipt_is_single_use_and_expands_bounded_click() {
        let c = Arc::new(42);
        let now = Instant::now();
        let mut o = observation(&c, Access::Control, now);
        let mut port = Recorder::default();
        let action = Action::Click {
            x: 123,
            y: 456,
            button: 0,
        };
        assert_eq!(
            o.act(&c, &[1; 7], now, true, action.clone(), &mut port),
            Ok(Queued { events: 3 })
        );
        assert_eq!(
            port.sent,
            [
                Event::Move { x: 123, y: 456 },
                Event::Button {
                    button: 0,
                    pressed: true
                },
                Event::Button {
                    button: 0,
                    pressed: false
                }
            ]
        );
        assert_eq!(
            o.act(&c, &[1; 7], now, true, action, &mut port),
            Err(Rejection::Consumed)
        );
        assert_eq!(port.sent.len(), 3);
    }

    #[test]
    fn rejects_observe_only_local_disqualification_and_expired_frames() {
        let c = Arc::new(42);
        let now = Instant::now();
        let mut port = Recorder::default();
        let click = Action::Click {
            x: 1,
            y: 1,
            button: 0,
        };
        let mut read_only = observation(&c, Access::Observe, now);
        assert_eq!(
            read_only.act(&c, &[1; 7], now, true, click.clone(), &mut port),
            Err(Rejection::InputNotAllowed)
        );
        assert_eq!(
            read_only.act(&c, &[1; 7], now, true, click.clone(), &mut port),
            Err(Rejection::Consumed)
        );

        let mut locked = observation(&c, Access::Control, now);
        assert_eq!(
            locked.act(&c, &[1; 7], now, false, click.clone(), &mut port),
            Err(Rejection::InputNotAllowed)
        );
        assert_eq!(
            locked.act(&c, &[1; 7], now, true, click.clone(), &mut port),
            Err(Rejection::Consumed)
        );

        let mut expired = observation(&c, Access::Control, now);
        assert_eq!(
            expired.act(
                &c,
                &[1; 7],
                now + MAX_OBSERVATION_AGE + Duration::from_millis(1),
                true,
                click.clone(),
                &mut port
            ),
            Err(Rejection::Expired)
        );
        assert_eq!(
            expired.act(&c, &[1; 7], now, true, click, &mut port),
            Err(Rejection::Consumed)
        );
        assert!(port.sent.is_empty());
    }

    #[test]
    fn rejects_connection_or_any_fence_component_drift() {
        let c = Arc::new(1);
        let other = Arc::new(2);
        let now = Instant::now();
        let mut port = Recorder::default();
        let key = Action::KeyPress { key_code: 30 };
        let mut wrong_conn = observation(&c, Access::Control, now);
        assert_eq!(
            wrong_conn.act(&other, &[1; 7], now, true, key.clone(), &mut port),
            Err(Rejection::StaleSession)
        );
        assert_eq!(
            wrong_conn.act(&c, &[1; 7], now, true, key.clone(), &mut port),
            Err(Rejection::Consumed)
        );

        for idx in 0..7 {
            let mut fence = [1; 7];
            fence[idx] = 2;
            let mut o = observation(&c, Access::Control, now);
            assert_eq!(
                o.act(&c, &fence, now, true, key.clone(), &mut port),
                Err(Rejection::StaleSession)
            );
            assert_eq!(
                o.act(&c, &[1; 7], now, true, key.clone(), &mut port),
                Err(Rejection::Consumed)
            );
        }
        assert!(port.sent.is_empty());
    }

    #[test]
    fn dropped_connection_invalidates_even_when_frame_arc_is_retained() {
        let c = Arc::new(1);
        let now = Instant::now();
        let mut o = observation(&c, Access::Control, now);
        let retained = o.frame().clone();
        drop(c);
        let replacement = Arc::new(1);
        let mut port = Recorder::default();
        assert_eq!(*retained, "frame");
        assert_eq!(
            o.act(
                &replacement,
                &[1; 7],
                now,
                true,
                Action::KeyPress { key_code: 1 },
                &mut port
            ),
            Err(Rejection::StaleSession)
        );
    }

    #[test]
    fn partial_queue_failure_consumes_receipt_and_attempts_exact_release() {
        for release_ok in [true, false] {
            let c = Arc::new(1);
            let now = Instant::now();
            let mut o = observation(&c, Access::Control, now);
            let mut port = Recorder {
                fail_at: Some(2),
                release_ok,
                ..Recorder::default()
            };
            let click = Action::Click {
                x: 10,
                y: 20,
                button: 1,
            };
            assert_eq!(
                o.act(&c, &[1; 7], now, true, click.clone(), &mut port),
                Err(Rejection::QueueRejected {
                    release_queued: release_ok
                })
            );
            assert_eq!(port.releases, 1);
            assert_eq!(
                o.act(&c, &[1; 7], now, true, click, &mut port),
                Err(Rejection::Consumed)
            );
            assert_eq!(port.releases, 1);
        }
    }

    #[test]
    fn rejects_invalid_button_and_unbounded_scroll_before_consuming_or_sending() {
        let c = Arc::new(1);
        let now = Instant::now();
        let mut o = observation(&c, Access::Control, now);
        let mut port = Recorder::default();
        assert_eq!(
            o.act(
                &c,
                &[1; 7],
                now,
                true,
                Action::Click {
                    x: 0,
                    y: 0,
                    button: 3
                },
                &mut port
            ),
            Err(Rejection::InvalidAction)
        );
        assert_eq!(
            o.act(
                &c,
                &[1; 7],
                now,
                true,
                Action::Scroll {
                    x: 0,
                    y: 0,
                    delta_x: 1201,
                    delta_y: 0
                },
                &mut port
            ),
            Err(Rejection::InvalidAction)
        );
        assert_eq!(
            o.act(
                &c,
                &[1; 7],
                now,
                true,
                Action::Scroll {
                    x: u16::MAX,
                    y: 0,
                    delta_x: -3,
                    delta_y: 9
                },
                &mut port
            ),
            Ok(Queued { events: 2 })
        );
        assert_eq!(
            port.sent,
            [
                Event::Move { x: u16::MAX, y: 0 },
                Event::Scroll {
                    delta_x: -3,
                    delta_y: 9
                }
            ]
        );
    }

    #[test]
    fn batch_execution_dispatches_commit_text_waits_and_cascades_abort_on_failure() {
        let c = Arc::new(42);
        let now = Instant::now();
        let mut o = observation(&c, Access::Control, now);
        let mut port = Recorder {
            fail_at: Some(4),
            release_ok: true,
            ..Recorder::default()
        };
        let mut waited_ms = Vec::new();
        let batch = vec![
            Action::Click {
                x: 200,
                y: 300,
                button: 0,
            },
            Action::Wait { duration_ms: 120 },
            Action::CommitText {
                text: "hello 世界".into(),
            },
            Action::KeyPress { key_code: 36 },
            Action::Move { x: 400, y: 500 },
        ];
        let outcome = o
            .act_batch_with_wait(&c, &[1; 7], now, true, &batch, &mut port, &mut |action| {
                if let Action::Wait{duration_ms}=action {waited_ms.push(*duration_ms);}
                true
            })
            .unwrap();
        assert_eq!(waited_ms, vec![120]);
        assert_eq!(outcome.failed_at, Some(3));
        assert!(!outcome.results[0].is_error);
        assert_eq!(outcome.results[0].events_queued, 3);
        assert!(!outcome.results[1].is_error);
        assert_eq!(outcome.results[1].events_queued, 0);
        assert!(!outcome.results[2].is_error);
        assert_eq!(outcome.results[2].events_queued, 1);
        assert!(outcome.results[3].is_error);
        assert_eq!(outcome.results[3].error.as_deref(), Some("outcome_unknown"));
        assert!(outcome.results[4].is_error);
        assert_eq!(
            outcome.results[4].error.as_deref(),
            Some(CASCADE_ABORT_MESSAGE)
        );
        assert_eq!(port.releases, 1);
    }

    #[test]
    fn action_ref_resolution_converts_scoped_refs_or_fails_closed() {
        let click_ref = Action::ClickRef {
            target_ref: "@s3_1:e4".into(),
            button: 0,
        };
        assert!(click_ref.valid());
        let resolved = click_ref
            .resolve_with(|r| {
                if r == "@s3_1:e4" {
                    Ok((420, 610))
                } else {
                    Err("stale_snapshot_ref")
                }
            })
            .unwrap();
        assert_eq!(
            resolved,
            Action::Click {
                x: 420,
                y: 610,
                button: 0
            }
        );
        assert_eq!(
            Action::MoveRef {
                target_ref: "@s2_1:e4".into()
            }
            .resolve_with(|_| Err("stale_snapshot_ref")),
            Err("stale_snapshot_ref")
        );
    }
}
