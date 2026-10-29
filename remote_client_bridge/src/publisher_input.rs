//! Bounded admission between authenticated input and Android's asynchronous OS API.
//! An enqueue is not an acknowledgement; only the matching OS completion is.
use protocol::{
    InputEvent,
    realtime_input::{InputBinding, InputLease, MAX_INPUT_AGE_MS, RealtimeInput},
};
use std::{collections::VecDeque, net::SocketAddr};

const MAX_PENDING: usize = 32;
pub(super) fn now_ms() -> u64 {
    remote_core::timing::quanta_now_us() / 1000
}

struct Owner {
    addr: SocketAddr,
    lease: InputLease,
    text: protocol::interaction::TextReceiver,
}
struct Pending {
    token: u64,
    sequence: u64,
    text_id: Option<u64>,
    created: u64,
    events: Vec<InputEvent>,
}

#[derive(serde::Serialize)]
#[serde(tag = "kind")]
pub(super) enum Dispatch {
    Reset {
        generation: u64,
    },
    Events {
        generation: u64,
        token: u64,
        events: Vec<InputEvent>,
    },
}
pub(super) struct Handled {
    pub addr: SocketAddr,
    pub binding: InputBinding,
    pub sequence: u64,
    pub text: Option<(u64, bool)>,
}

#[derive(Default)]
pub(super) struct PublisherInput {
    service: bool,
    capture: bool,
    generation: u64,
    reset_pending: bool,
    next_token: u64,
    owner: Option<Owner>,
    queued: VecDeque<Pending>,
    executing: Option<Pending>,
    handled: VecDeque<Handled>,
}
impl PublisherInput {
    pub fn ready(&self) -> bool {
        self.service && self.capture
    }
    pub fn set_service(&mut self, ready: bool) {
        if self.service != ready {
            self.service = ready;
            self.revoke();
        }
    }
    pub fn set_capture(&mut self, ready: bool) {
        if self.capture != ready {
            self.capture = ready;
            self.revoke();
        }
    }
    pub fn revoke(&mut self) {
        if let Some(owner) = self.owner.take() {
            owner.lease.revocation().revoke();
        }
        self.queued.clear();
        self.executing = None;
        self.handled.clear();
        self.generation = self.generation.wrapping_add(1);
        self.reset_pending = true;
    }
    pub fn revoke_peer(&mut self, addr: SocketAddr) {
        if self.owner.as_ref().is_some_and(|owner| owner.addr == addr) {
            self.revoke();
        }
    }
    pub fn tick(&mut self) {
        let now = now_ms();
        if self
            .owner
            .as_ref()
            .is_some_and(|owner| owner.lease.expired(now))
            || self
                .executing
                .as_ref()
                .is_some_and(|job| now.saturating_sub(job.created) > MAX_INPUT_AGE_MS)
        {
            self.revoke();
        }
    }
    pub fn grant(
        &mut self,
        addr: SocketAddr,
        connection_id: u32,
        subscription_id: u32,
    ) -> Option<InputBinding> {
        if !self.ready() {
            return None;
        }
        self.tick();
        if self.owner.as_ref().is_none_or(|owner| {
            owner.addr != addr
                || owner.lease.binding.connection_id != connection_id
                || owner.lease.binding.subscription_id != subscription_id
        }) {
            self.revoke();
            let binding = InputBinding {
                connection_id,
                subscription_id,
                source_revision: 0,
                epoch: remote_core::session_crypto::random_bytes_16(),
            };
            self.owner = Some(Owner {
                addr,
                lease: InputLease::new(binding, now_ms()),
                text: Default::default(),
            });
        }
        self.owner.as_ref().map(|owner| owner.lease.binding)
    }
    pub fn admit(&mut self, addr: SocketAddr, packet: &RealtimeInput) {
        if !self.ready() {
            return;
        }
        self.tick();
        let Some(owner) = self.owner.as_mut().filter(|owner| owner.addr == addr) else {
            return;
        };
        // Reject invalid touch coordinates before they reach Android paths.
        if let protocol::realtime_input::InputPayload::Event(InputEvent::Touch {
            normalized_x,
            normalized_y,
            pressure,
            ..
        }) = &packet.payload
            && (!normalized_x.is_finite()
                || !normalized_y.is_finite()
                || !pressure.is_finite()
                || !(0.0..=1.0).contains(normalized_x)
                || !(0.0..=1.0).contains(normalized_y))
        {
            self.revoke();
            return;
        }
        let Some(events) = owner.lease.accept(packet, now_ms()) else {
            return;
        };
        // Touch continuations have no HeldState press. End those OS holds too.
        if matches!(
            packet.payload,
            protocol::realtime_input::InputPayload::Release
        ) {
            self.revoke();
            return;
        }
        if self.queued.len() + usize::from(self.executing.is_some()) >= MAX_PENDING {
            self.revoke();
            return;
        }
        let Some(token) = self.next_token.checked_add(1) else {
            self.revoke();
            return;
        };
        self.next_token = token;
        self.queued.push_back(Pending {
            token,
            sequence: packet.sequence,
            text_id: None,
            created: packet.created_host_ms,
            events,
        });
    }
    pub fn admit_text(&mut self, addr: SocketAddr, request: &protocol::interaction::TextCommit) -> Option<protocol::session::SessionCommand> {
        use protocol::{interaction::TextAdmission, session::SessionCommand};
        self.tick();
        let result=(|| {
            if !self.ready() {return Err("Android screen control is unavailable");}
            let owner=self.owner.as_mut().filter(|owner|owner.addr==addr).ok_or("text input owner changed")?;
            owner.text.begin(request,&mut owner.lease,now_ms())
        })();
        let error = match result {
            Ok(TextAdmission::Accepted)=>return Some(SessionCommand::TextCommitted {binding:request.binding,request_id:request.request_id}),
            Ok(TextAdmission::Pending)=>return None,
            Err(error)=>Some(error),
            Ok(TextAdmission::New)=> {
                if self.queued.len()+usize::from(self.executing.is_some())>=MAX_PENDING {Some("Android input queue is full")}
                else if let Some(token)=self.next_token.checked_add(1) {
                    self.next_token=token;
                    self.queued.push_back(Pending {token,sequence:0,text_id:Some(request.request_id),
                        created:request.created_host_ms,events:vec![InputEvent::Text(request.text.clone())]});
                    return None;
                } else {Some("Android input sequence exhausted")}
            }
        };
        if let Some(owner)=self.owner.as_mut().filter(|owner|owner.addr==addr) {
            owner.text.complete(request.request_id,false);
        }
        Some(SessionCommand::TextRejected {binding:request.binding,request_id:request.request_id,reason:error.unwrap().into()})
    }
    pub fn poll(&mut self) -> Option<Dispatch> {
        self.tick();
        if self.reset_pending {
            self.reset_pending = false;
            return Some(Dispatch::Reset {
                generation: self.generation,
            });
        }
        if !self.ready() || self.executing.is_some() {
            return None;
        }
        let job = self.queued.pop_front()?;
        if now_ms().saturating_sub(job.created) > MAX_INPUT_AGE_MS {
            self.revoke();
            return self.poll();
        }
        let dispatch = Dispatch::Events {
            generation: self.generation,
            token: job.token,
            events: job.events.clone(),
        };
        self.executing = Some(job);
        Some(dispatch)
    }
    pub fn execution_valid(&mut self, generation: u64, token: u64) -> bool {
        self.tick();
        self.ready()
            && generation == self.generation
            && self
                .executing
                .as_ref()
                .is_some_and(|job| job.token == token)
    }
    pub fn complete(&mut self, generation: u64, token: u64, accepted: bool) {
        self.tick();
        if generation != self.generation
            || self.executing.as_ref().is_none_or(|job| job.token != token)
        {
            return;
        }
        let job = self.executing.take().unwrap();
        let accepted=accepted && self.ready() && self.handled.len()<MAX_PENDING;
        let receipt=self.owner.as_mut().map(|owner| {
            if let Some(id)=job.text_id {owner.text.complete(id,accepted);}
            Handled {addr:owner.addr,binding:owner.lease.binding,sequence:job.sequence,
                text:job.text_id.map(|id|(id,accepted))}
        });
        if !accepted {self.revoke();}
        if let Some(receipt)=receipt {
            if accepted || receipt.text.is_some() {self.handled.push_back(receipt);}
        }
    }
    pub fn take_handled(&mut self) -> Vec<Handled> {
        self.tick();
        self.handled.drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::realtime_input::InputPayload;
    fn fixture() -> (PublisherInput, SocketAddr, InputBinding) {
        let mut input = PublisherInput::default();
        input.set_capture(true);
        input.set_service(true);
        let addr = "127.0.0.1:5000".parse().unwrap();
        let binding = input.grant(addr, 1, 256).unwrap();
        assert!(matches!(input.poll(), Some(Dispatch::Reset { .. })));
        (input, addr, binding)
    }
    fn movement(binding: InputBinding, sequence: u64) -> RealtimeInput {
        RealtimeInput {
            binding,
            sequence,
            created_host_ms: now_ms(),
            payload: InputPayload::Event(InputEvent::MouseMoveAbsolute { x: 100, y: 200 }),
        }
    }
    fn take(input: &mut PublisherInput) -> (u64, u64) {
        let Some(Dispatch::Events {
            generation, token, ..
        }) = input.poll()
        else {
            panic!("expected admitted input")
        };
        (generation, token)
    }
    fn text(binding:InputBinding)->protocol::interaction::TextCommit {
        protocol::interaction::TextCommit {binding,request_id:70,created_host_ms:now_ms(),
            text:protocol::interaction::CommittedText::new("你好👨‍👩‍👧‍👦".into()).unwrap()}
    }
    #[test]
    fn text_receipt_waits_for_os_and_lost_receipt_never_reinjects() {
        use protocol::session::SessionCommand;
        let (mut input,addr,binding)=fixture();let request=text(binding);
        assert!(input.admit_text(addr,&request).is_none());
        assert!(input.admit_text(addr,&request).is_none());
        assert_eq!(input.queued.len(),1);
        let (generation,token)=take(&mut input);
        assert!(input.take_handled().is_empty());
        input.complete(generation,token,true);
        assert_eq!(input.take_handled()[0].text,Some((70,true)));
        assert!(matches!(input.admit_text(addr,&request),Some(SessionCommand::TextCommitted {request_id:70,..})));
        assert!(input.poll().is_none());
        let mut altered=request.clone();altered.text=protocol::interaction::CommittedText::new("different".into()).unwrap();
        assert!(matches!(input.admit_text(addr,&altered),Some(SessionCommand::TextRejected {..})));
        assert!(input.poll().is_none());
    }
    #[test]
    fn failed_or_changed_android_text_target_is_never_acknowledged_or_replayed() {
        use protocol::session::SessionCommand;
        let (mut input,addr,binding)=fixture();let request=text(binding);
        input.admit_text(addr,&request);let (generation,token)=take(&mut input);
        input.complete(generation,token,false);
        assert_eq!(input.take_handled()[0].text,Some((70,false)));
        input.poll();let next=input.grant(addr,1,256).unwrap();assert_ne!(next,binding);
        assert!(matches!(input.admit_text(addr,&request),Some(SessionCommand::TextRejected {..})));
        input.poll();assert!(input.poll().is_none());
    }
    #[test]
    fn requires_both_os_service_and_explicit_capture_control() {
        let mut input = PublisherInput::default();
        let addr = "127.0.0.1:5000".parse().unwrap();
        assert!(input.grant(addr, 1, 256).is_none());
        input.set_service(true);
        assert!(input.grant(addr, 1, 256).is_none());
        input.set_capture(true);
        assert!(input.grant(addr, 1, 256).is_some());
        input.set_service(false);
        assert!(input.grant(addr, 1, 256).is_none());
    }
    #[test]
    fn acknowledgements_require_os_completion_and_never_replay_an_outcome() {
        let (mut input, addr, binding) = fixture();
        let packet = movement(binding, 1);
        input.admit(addr, &packet);
        assert!(input.take_handled().is_empty());
        let (generation, token) = take(&mut input);
        assert!(input.take_handled().is_empty());
        input.complete(generation, token, true);
        let handled = input.take_handled();
        assert_eq!(handled.len(), 1);
        assert_eq!(handled[0].sequence, 1);
        input.complete(generation, token, true);
        input.admit(addr, &packet);
        assert!(input.take_handled().is_empty());
        assert!(input.poll().is_none());
    }
    #[test]
    fn wrong_peer_stale_binding_and_old_os_callbacks_cannot_act_on_new_owner() {
        let (mut input, addr, old) = fixture();
        input.admit("127.0.0.1:5001".parse().unwrap(), &movement(old, 1));
        assert!(input.poll().is_none());
        input.admit(addr, &movement(old, 1));
        let (generation, token) = take(&mut input);
        input.revoke_peer(addr);
        let new = input.grant(addr, 1, 256).unwrap();
        assert_ne!(old, new);
        input.poll();
        input.admit(addr, &movement(old, 2));
        assert!(input.poll().is_none());
        input.admit(addr, &movement(new, 1));
        let (next, job) = take(&mut input);
        assert!(!input.execution_valid(generation, token));
        assert!(input.execution_valid(next, job));
        input.complete(generation, token, false);
        input.complete(next, job, true);
        assert_eq!(input.take_handled()[0].binding, new);
    }
    #[test]
    fn explicit_release_retires_touch_continuations_and_inflight_callbacks() {
        let (mut input, addr, binding) = fixture();
        input.admit(addr, &movement(binding, 1));
        let (generation, token) = take(&mut input);
        input.admit(
            addr,
            &RealtimeInput {
                binding,
                sequence: 2,
                created_host_ms: now_ms(),
                payload: InputPayload::Release,
            },
        );
        assert!(!input.execution_valid(generation, token));
        assert!(input.owner.is_none());
        assert!(matches!(input.poll(), Some(Dispatch::Reset { .. })));
    }
    #[test]
    fn overflow_and_permission_revocation_clear_queued_and_executing_input() {
        let (mut input, addr, binding) = fixture();
        for seq in 1..=MAX_PENDING as u64 + 1 {
            input.admit(addr, &movement(binding, seq));
        }
        assert!(input.owner.is_none());
        assert!(input.queued.is_empty());
        assert!(matches!(input.poll(), Some(Dispatch::Reset { .. })));
        let binding = input.grant(addr, 1, 256).unwrap();
        input.poll();
        input.admit(addr, &movement(binding, 1));
        let (generation, token) = take(&mut input);
        input.set_capture(false);
        input.complete(generation, token, true);
        assert!(input.take_handled().is_empty());
        assert!(matches!(input.poll(), Some(Dispatch::Reset { .. })));
    }
    #[test]
    fn expired_queued_input_and_unknown_native_outcomes_are_never_acknowledged() {
        let (mut input, addr, binding) = fixture();
        input.admit(addr, &movement(binding, 1));
        let (generation, token) = take(&mut input);
        input.executing.as_mut().unwrap().created = now_ms().saturating_sub(MAX_INPUT_AGE_MS + 1);
        input.complete(generation, token, true);
        assert!(input.take_handled().is_empty());
        assert!(input.owner.is_none());
    }
}
