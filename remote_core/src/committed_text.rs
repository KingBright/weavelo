//! Ordered committed text; pointer/key packets wait behind its native dispatch receipt.
use protocol::{
    interaction::TextCommit,
    realtime_input::{InputBinding, MAX_INPUT_AGE_MS, RealtimeInput},
    session::SessionCommand,
};
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::time::Instant;
pub(crate) const MAX_TEXT_EVENTS: usize = 64;
const MAX_ORDERED_INPUTS: usize = 128;
const RETRY: Duration = Duration::from_millis(50);
enum Item {
    Text {
        request: TextCommit,
        queued: Instant,
        sent: Option<Instant>,
    },
    Input {
        packet: RealtimeInput,
        queued: Instant,
    },
}
impl Item {
    fn binding(&self) -> InputBinding {
        match self {
            Self::Text { request, .. } => request.binding,
            Self::Input { packet, .. } => packet.binding,
        }
    }
}
#[derive(Default)]
pub(crate) struct TextOutbox {
    pub pending: Arc<AtomicUsize>,
    items: VecDeque<Item>,
}
pub(crate) enum Action {
    Send(SessionCommand),
    Failed {
        binding: InputBinding,
        request_id: u64,
        reason: &'static str,
    },
}
impl TextOutbox {
    pub fn waiting(&self) -> bool {
        !self.items.is_empty()
    }
    pub fn text(&mut self, requests: Vec<TextCommit>, queued: Instant) {
        for request in requests {
            // Merge only adjacent, unsent commits to this exact wire owner.
            // The first ID is retained (IDs may have gaps); a sent request and
            // its retries are immutable. Input items remain ordering barriers.
            if let Some(Item::Text { request: tail, queued: oldest, sent: None }) = self.items.back_mut()
                && tail.binding == request.binding
                && tail.request_id < request.request_id
                && tail.text.as_str().len() + request.text.as_str().len()
                    <= protocol::interaction::MAX_COMMITTED_TEXT_BYTES
            {
                let text = format!("{}{}", tail.text.as_str(), request.text.as_str());
                tail.text = protocol::interaction::CommittedText::new(text).expect("bounded valid text concatenation");
                tail.created_host_ms = tail.created_host_ms.min(request.created_host_ms);
                *oldest = (*oldest).min(queued);
                self.pending.fetch_sub(1, Ordering::AcqRel);
                continue;
            }
            self.items.push_back(Item::Text {
                request,
                queued,
                sent: None,
            });
        }
    }
    pub fn input(&mut self, packet: RealtimeInput, queued: Instant) -> bool {
        // Only an unsent, adjacent absolute position can be superseded.
        // Clicks, relative deltas, text and grant changes remain barriers.
        if matches!(packet.payload, protocol::realtime_input::InputPayload::Event(protocol::InputEvent::MouseMoveAbsolute { .. }))
            && let Some(Item::Input { packet: tail, queued: at }) = self.items.back_mut()
            && tail.binding == packet.binding
            && matches!(tail.payload, protocol::realtime_input::InputPayload::Event(protocol::InputEvent::MouseMoveAbsolute { .. }))
        {
            *tail = packet;
            *at = queued;
            return true;
        }
        if self.items.len() >= MAX_ORDERED_INPUTS {
            return false;
        }
        self.items.push_back(Item::Input { packet, queued });
        true
    }
    fn remove_front(&mut self) {
        if matches!(self.items.pop_front(), Some(Item::Text { .. })) {
            self.pending.fetch_sub(1, Ordering::AcqRel);
        }
    }
    pub fn acknowledge(&mut self, binding: InputBinding, request_id: u64) -> bool {
        if !matches!(self.items.front(),Some(Item::Text{request,..}) if request.binding==binding&&request.request_id==request_id)
        {
            return false;
        }
        self.remove_front();
        true
    }
    pub fn cancel(&mut self, binding: InputBinding) {
        self.items.retain(|item| {
            if item.binding() != binding {
                return true;
            }
            if matches!(item, Item::Text { .. }) {
                self.pending.fetch_sub(1, Ordering::AcqRel);
            }
            false
        });
    }
    pub fn advance(
        &mut self,
        now: Instant,
        mut current: impl FnMut(InputBinding) -> bool,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        loop {
            let Some(item) = self.items.front() else {
                break;
            };
            let binding = item.binding();
            let queued = match item {
                Item::Text { queued, .. } | Item::Input { queued, .. } => *queued,
            };
            if !current(binding)
                || now.saturating_duration_since(queued).as_millis() > MAX_INPUT_AGE_MS as u128
            {
                let request_id = match item {
                    Item::Text { request, .. } => request.request_id,
                    Item::Input { .. } => 0,
                };
                self.cancel(binding);
                actions.push(Action::Failed {
                    binding,
                    request_id,
                    reason: "text input was cancelled before dispatch confirmation",
                });
                break;
            }
            match self.items.front_mut().unwrap() {
                Item::Text { request, sent, .. } => {
                    if sent.is_none_or(|at| now.saturating_duration_since(at) >= RETRY) {
                        *sent = Some(now);
                        actions.push(Action::Send(SessionCommand::CommitText(request.clone())));
                    }
                    break;
                }
                Item::Input { packet, .. } => {
                    actions.push(Action::Send(SessionCommand::RealtimeInput(packet.clone())));
                    self.remove_front();
                }
            }
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{interaction::CommittedText, realtime_input::InputPayload};
    fn request() -> TextCommit {
        TextCommit {
            binding: InputBinding {
                connection_id: 1,
                subscription_id: 2,
                source_revision: 3,
                epoch: [4; 16],
            },
            request_id: 5,
            created_host_ms: 100,
            text: CommittedText::new("你好🙂".into()).unwrap(),
        }
    }
    #[test]
    fn retries_preserve_timestamps_and_later_focus_input_waits_for_the_ack() {
        let now = Instant::now();
        let request = request();
        let mut out = TextOutbox::default();
        out.pending.store(1, Ordering::Release);
        out.text(vec![request.clone()], now);
        let packet = RealtimeInput {
            binding: request.binding,
            sequence: 8,
            created_host_ms: 110,
            payload: InputPayload::Event(protocol::InputEvent::MouseDown(0)),
        };
        assert!(out.input(packet, now));
        assert!(
            matches!(&out.advance(now, |_|true)[0],Action::Send(SessionCommand::CommitText(r)) if r==&request)
        );
        assert!(
            out.advance(now + Duration::from_millis(30), |_| true)
                .is_empty()
        );
        assert!(
            matches!(&out.advance(now+Duration::from_millis(60), |_|true)[0],Action::Send(SessionCommand::CommitText(r)) if r==&request)
        );
        assert!(!out.acknowledge(request.binding, 4));
        assert!(out.acknowledge(request.binding, 5));
        assert!(
            matches!(&out.advance(now+Duration::from_millis(70), |_|true)[0],Action::Send(SessionCommand::RealtimeInput(p)) if p.created_host_ms==110&&p.sequence==8)
        );
        assert_eq!(out.pending.load(Ordering::Acquire), 0);
        assert!(!out.waiting());
    }
    #[test]
    fn expiry_or_source_change_cancels_the_whole_old_input_barrier() {
        for changed in [false, true] {
            let now = Instant::now();
            let request = request();
            let mut out = TextOutbox::default();
            out.pending.store(1, Ordering::Release);
            out.text(vec![request.clone()], now);
            let time = if changed {
                now
            } else {
                now + Duration::from_millis(251)
            };
            assert!(
                matches!(&out.advance(time, |_|!changed)[0],Action::Failed{binding,..} if *binding==request.binding)
            );
            assert_eq!(out.pending.load(Ordering::Acquire), 0);
            assert!(!out.waiting());
            assert!(!out.acknowledge(request.binding, 5));
        }
    }

    #[test]
    fn fast_typing_batches_behind_the_inflight_commit_without_changing_its_retry() {
        let now = Instant::now();
        let mut first = request();
        first.text = CommittedText::new("W".into()).unwrap();
        let mut out = TextOutbox::default();
        out.pending.store(9, Ordering::Release);
        out.text(vec![first.clone()], now);
        out.advance(now, |_| true);
        for (i, ch) in "eavelo42".chars().enumerate() {
            let mut next = first.clone();
            next.request_id += i as u64 + 1;
            next.created_host_ms += i as u64 + 1;
            next.text = CommittedText::new(ch.to_string()).unwrap();
            out.text(vec![next], now + Duration::from_millis(i as u64 + 1));
        }
        assert_eq!(out.pending.load(Ordering::Acquire), 2);
        assert!(matches!(&out.advance(now+Duration::from_millis(60), |_|true)[0],
            Action::Send(SessionCommand::CommitText(r)) if r==&first));
        assert!(out.acknowledge(first.binding, first.request_id));
        let actions = out.advance(now + Duration::from_millis(80), |_| true);
        let Action::Send(SessionCommand::CommitText(batch)) = &actions[0] else { panic!() };
        assert_eq!(batch.text.as_str(), "eavelo42");
        assert_eq!(batch.request_id, first.request_id + 1);
        assert_eq!(batch.created_host_ms, first.created_host_ms + 1);
        assert!(out.acknowledge(batch.binding, batch.request_id));
        assert_eq!(out.pending.load(Ordering::Acquire), 0);
    }

    #[test]
    fn batching_preserves_click_order_and_exact_source_ownership() {
        let now = Instant::now();
        let first = request();
        let mut next = first.clone(); next.request_id += 1;
        let mut other = next.clone(); other.binding.source_revision += 1;
        let mut out = TextOutbox::default();
        out.pending.store(3, Ordering::Release);
        out.text(vec![first.clone()], now);
        let packet = RealtimeInput {binding:first.binding,sequence:8,created_host_ms:100,
            payload:InputPayload::Event(protocol::InputEvent::MouseDown(0))};
        assert!(out.input(packet.clone(), now));
        out.text(vec![next.clone(), other.clone()], now);
        out.advance(now, |_| true);
        out.acknowledge(first.binding, first.request_id);
        let actions = out.advance(now, |_| true);
        assert!(matches!(&actions[..], [Action::Send(SessionCommand::RealtimeInput(p)),
            Action::Send(SessionCommand::CommitText(r))] if p.sequence==packet.sequence
                && p.binding==packet.binding && p.created_host_ms==packet.created_host_ms
                && matches!(p.payload,InputPayload::Event(protocol::InputEvent::MouseDown(0))) && r==&next));
        out.acknowledge(next.binding, next.request_id);
        assert!(matches!(&out.advance(now, |_|true)[0],
            Action::Send(SessionCommand::CommitText(r)) if r==&other));
    }

    #[test]
    fn pointer_burst_behind_text_keeps_latest_position_and_click_order() {
        let now = Instant::now();
        let text = request();
        let mut out = TextOutbox::default();
        out.pending.store(1, Ordering::Release);
        out.text(vec![text.clone()], now);
        let packet = |sequence, payload| RealtimeInput {
            binding: text.binding, sequence, created_host_ms: 100 + sequence, payload,
        };
        for sequence in 0..1024 {
            assert!(out.input(packet(sequence, InputPayload::Event(
                protocol::InputEvent::MouseMoveAbsolute { x: sequence as u16, y: 3 }
            )), now + Duration::from_micros(sequence)), "motion {sequence} rejected");
        }
        let down = packet(1024, InputPayload::Event(protocol::InputEvent::MouseDown(0)));
        assert!(out.input(down, now));
        assert!(out.input(packet(1025, InputPayload::Event(
            protocol::InputEvent::MouseMoveAbsolute { x: 99, y: 4 }
        )), now));
        assert_eq!(out.items.len(), 4);
        assert!(matches!(&out.items[1], Item::Input { queued, .. }
            if *queued == now + Duration::from_micros(1023)));
        assert!(out.acknowledge(text.binding, text.request_id));
        let actions = out.advance(now + Duration::from_millis(2), |_| true);
        assert!(matches!(&actions[..], [Action::Send(SessionCommand::RealtimeInput(a)),
            Action::Send(SessionCommand::RealtimeInput(b)), Action::Send(SessionCommand::RealtimeInput(c))]
            if a.sequence == 1023 && a.created_host_ms == 1123
                && matches!(a.payload, InputPayload::Event(protocol::InputEvent::MouseMoveAbsolute { x: 1023, .. }))
                && matches!(b.payload, InputPayload::Event(protocol::InputEvent::MouseDown(0)))
                && c.sequence == 1025));
    }

    #[test]
    fn pointer_coalescing_keeps_epoch_text_and_relative_motion_barriers() {
        let now = Instant::now();
        let text = request();
        let motion = |binding, sequence| RealtimeInput {
            binding, sequence, created_host_ms: 100,
            payload: InputPayload::Event(protocol::InputEvent::MouseMoveAbsolute { x: 1, y: 2 }),
        };
        let mut out = TextOutbox::default();
        let mut other = text.binding; other.epoch = [9; 16];
        assert!(out.input(motion(text.binding, 1), now));
        assert!(out.input(motion(other, 2), now));
        let mut relative = motion(other, 3);
        relative.payload = InputPayload::Event(protocol::InputEvent::MouseMove { dx: 1, dy: 2 });
        assert!(out.input(relative, now));
        assert!(out.input(motion(other, 4), now));
        out.pending.store(1, Ordering::Release);
        out.text(vec![text], now);
        assert!(out.input(motion(other, 5), now));
        assert_eq!(out.items.len(), 6);
    }

    #[test]
    fn batching_keeps_utf8_byte_limit_and_the_oldest_original_deadline() {
        let now = Instant::now();
        let mut first = request();
        first.text = CommittedText::new("a".repeat(1020)).unwrap();
        let mut emoji = first.clone(); emoji.request_id += 1; emoji.created_host_ms -= 20;
        emoji.text = CommittedText::new("😀".into()).unwrap();
        let mut after = emoji.clone(); after.request_id += 1;
        after.text = CommittedText::new("x".into()).unwrap();
        let mut out = TextOutbox::default();out.pending.store(3, Ordering::Release);
        out.text(vec![first], now);
        out.text(vec![emoji], now-Duration::from_millis(20));
        out.text(vec![after], now);
        assert_eq!(out.pending.load(Ordering::Acquire), 2);
        let Item::Text {request,queued,..} = out.items.front().unwrap() else {panic!()};
        assert_eq!(request.text.as_str().len(), 1024);
        assert!(request.text.as_str().ends_with('😀'));
        assert_eq!(request.created_host_ms, 80);
        assert_eq!(*queued, now-Duration::from_millis(20));
        assert!(matches!(&out.advance(now+Duration::from_millis(231), |_|true)[0],Action::Failed{..}));
        assert_eq!(out.pending.load(Ordering::Acquire), 0);
    }
}
