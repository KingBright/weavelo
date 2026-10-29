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
            self.items.push_back(Item::Text {
                request,
                queued,
                sent: None,
            });
        }
    }
    pub fn input(&mut self, packet: RealtimeInput, queued: Instant) -> bool {
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
}
