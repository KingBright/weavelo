use protocol::realtime_input::{HeldState, InputBinding, InputPayload, RealtimeInput};
use protocol::session::SessionCommand;
use std::collections::HashMap;
use std::time::Instant;

/// Local composition ownership survives transport lease renewal, never a source
/// replacement, input suspension or explicit release. This is not a wire grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TextInputOwner {
    pub connection_id: u32,
    pub subscription_id: u32,
    pub source_revision: u32,
    pub generation: u64,
}

#[derive(Default)]
pub(crate) struct InputState {
    slots: HashMap<u32, Slot>,
    next_request: u64,
    next_text: u64,
    next_owner: u64,
}
struct Slot {
    enabled: bool,
    revision: u32,
    request: u64,
    lease: Option<Lease>,
    owner: u64,
}
struct Lease {
    binding: InputBinding,
    host_ms: u64,
    received: Instant,
    sequence: u64,
    held: HeldState,
    release: Option<u64>,
    frontend_intent: Option<Instant>,
    frontend_expired: bool,
}
impl Lease {
    fn expire_frontend(&mut self,now:Instant)->bool {
        if self.frontend_intent.is_some_and(|at|now.saturating_duration_since(at).as_millis()>=protocol::realtime_input::HOLD_TIMEOUT_MS as u128) {
            self.held=HeldState::default();self.frontend_intent=None;self.frontend_expired=true;return true;
        }false
    }

    fn packet(&mut self, payload: InputPayload) -> Option<RealtimeInput> {
        self.sequence = self.sequence.checked_add(1)?;
        Some(RealtimeInput {
            binding: self.binding,
            sequence: self.sequence,
            created_host_ms: self
                .host_ms
                .saturating_add(self.received.elapsed().as_millis() as u64),
            payload,
        })
    }
}
impl InputState {
    pub(crate) fn source(&mut self, id: u32, revision: u32) {
        self.next_owner = self.next_owner.checked_add(1).expect("input owner exhausted");
        self.slots.insert(
            id,
            Slot {
                enabled: true,
                revision,
                request: 0,
                lease: None,
                owner: self.next_owner,
            },
        );
    }
    pub(crate) fn remove(&mut self, id: u32) {
        self.slots.remove(&id);
    }
    pub(crate) fn invalidate_text_owner(&mut self, id: u32) {
        self.next_owner = self.next_owner.checked_add(1).expect("input owner exhausted");
        if let Some(slot) = self.slots.get_mut(&id) { slot.owner = self.next_owner; }
    }
    pub(crate) fn ready(&self, id: u32, revision: u32) -> bool {
        self.slots.get(&id).is_some_and(|slot| {
            slot.enabled
                && slot.revision == revision
                && slot
                    .lease
                    .as_ref()
                    .is_some_and(|lease| lease.received.elapsed().as_millis() <= 750)
        })
    }
    pub(crate) fn binding(&self, id: u32, revision: u32) -> Option<InputBinding> {
        self.ready(id, revision).then(|| self.slots[&id].lease.as_ref().unwrap().binding)
    }
    pub(crate) fn text_owner(&self, id: u32, revision: u32) -> Option<TextInputOwner> {
        let binding = self.binding(id, revision)?;
        Some(TextInputOwner { connection_id: binding.connection_id, subscription_id: id,
            source_revision: revision, generation: self.slots[&id].owner })
    }
    pub(crate) fn text_binding(&self, owner: TextInputOwner) -> Option<InputBinding> {
        (self.text_owner(owner.subscription_id, owner.source_revision) == Some(owner))
            .then(|| self.binding(owner.subscription_id, owner.source_revision)).flatten()
    }
    pub(crate) fn text_for_owner(&mut self, owner: TextInputOwner, text: &str, age: std::time::Duration)
        -> Result<Vec<protocol::interaction::TextCommit>, String> {
        let binding = self.text_binding(owner).ok_or("text input source changed")?;
        self.text_aged(binding, text, age)
    }
    pub(crate) fn seal_bound(
        &mut self,
        binding: InputBinding,
        event: Option<protocol::InputEvent>,
    ) -> Result<SessionCommand, String> {
        if self.binding(binding.subscription_id, binding.source_revision) != Some(binding) {
            return Err("input binding changed".into());
        }
        self.seal(match event {
            Some(event) => SessionCommand::SourceInput {
                id: binding.subscription_id,
                source_revision: binding.source_revision,
                event,
            },
            None => SessionCommand::ReleaseInput { id: binding.subscription_id },
        })
    }
    pub(crate) fn grant(
        &mut self,
        connection: u32,
        binding: InputBinding,
        request: u64,
        host_ms: u64,
    ) {
        let Some(slot) = self.slots.get_mut(&binding.subscription_id) else {
            return;
        };
        if !slot.enabled
            || binding.connection_id != connection
            || slot.revision != binding.source_revision
            || slot.request != request
            || request == 0
        {
            return;
        }
        slot.request = 0;
        if let Some(lease) = &mut slot.lease
            && lease.binding == binding
        {
            lease.host_ms = host_ms;
            lease.received = Instant::now();
            return;
        }
        // Reconnect/source generation changes discard held intent, never replay it.
        slot.lease = Some(Lease {
            binding,
            host_ms,
            received: Instant::now(),
            sequence: 0,
            held: HeldState::default(),
            release: None,
            frontend_intent:None,
            frontend_expired:false,
        });
    }
    #[cfg(test)]
    pub(crate) fn text(&mut self, binding: InputBinding, text: &str) -> Result<Vec<protocol::interaction::TextCommit>, String> {
        self.text_aged(binding,text,std::time::Duration::ZERO)
    }
    pub(crate) fn text_aged(&mut self,binding:InputBinding,text:&str,age:std::time::Duration)->Result<Vec<protocol::interaction::TextCommit>,String> {
        if age.as_millis()>protocol::realtime_input::MAX_INPUT_AGE_MS as u128 {return Err("text input expired in the local UI queue".into());}
        if text.len()>crate::committed_text::MAX_TEXT_EVENTS*protocol::interaction::MAX_COMMITTED_TEXT_BYTES {return Err("text input is too large".into());}
        let chunks=protocol::interaction::text_chunks(text)?.map(|s|protocol::interaction::CommittedText::new(s.to_owned())).collect::<Result<Vec<_>,_>>()?;
        if self.binding(binding.subscription_id,binding.source_revision)!=Some(binding) {return Err("text input ownership changed".into());}
        let lease=self.slots[&binding.subscription_id].lease.as_ref().unwrap();
        let created_host_ms=lease.host_ms.saturating_add(lease.received.elapsed().as_millis() as u64).saturating_sub(age.as_millis() as u64);
        let mut requests=Vec::with_capacity(chunks.len());
        for text in chunks {
            self.next_text=self.next_text.checked_add(1).ok_or("text input sequence exhausted")?;
            requests.push(protocol::interaction::TextCommit {binding,request_id:self.next_text,created_host_ms,text});
        }
        Ok(requests)
    }
    pub(crate) fn requests(&mut self, connection: u32) -> Vec<SessionCommand> {
        self.slots
            .iter_mut()
            .filter_map(|(id, slot)| {
                if !slot.enabled {
                    return None;
                }
                self.next_request = self.next_request.checked_add(1)?;
                slot.request = self.next_request;
                Some(SessionCommand::RequestInputBinding {
                    connection_id: connection,
                    id: *id,
                    source_revision: slot.revision,
                    request: slot.request,
                })
            })
            .collect()
    }
    /// Only a healthy input frontend can extend its current held assertion.
    /// Grants/transport snapshots never call this and cannot revive expired keys.
    pub(crate) fn refresh_bound(&mut self,binding:InputBinding)->bool {
        if self.binding(binding.subscription_id,binding.source_revision)!=Some(binding){return false;}
        let lease=self.slots.get_mut(&binding.subscription_id).unwrap().lease.as_mut().unwrap();
        let now=Instant::now();if lease.expire_frontend(now)||lease.frontend_expired{return false;}
        if !lease.held.snapshot().is_empty(){lease.frontend_intent=Some(now);}
        true
    }
    pub(crate) fn snapshots(&mut self) -> Vec<RealtimeInput> {
        self.slots
            .values_mut()
            .filter_map(|slot| {
                let lease = slot.lease.as_mut()?;
                // An unresponsive grant path cannot keep old ownership alive forever.
                if lease.received.elapsed().as_millis() > 750 {
                    slot.lease = None;
                    return None;
                }
                lease.expire_frontend(Instant::now());
                let held=lease.held.snapshot();
                let age=if held.is_empty(){0}else{lease.frontend_intent.map_or(0,|at|at.elapsed().as_millis() as u64)};
                let mut packet=lease.packet(InputPayload::Held(held))?;
                // The actor must not stamp cached UI intent with a newer time.
                packet.created_host_ms=packet.created_host_ms.saturating_sub(age);Some(packet)
            })
            .collect()
    }
    pub(crate) fn seal(&mut self, command: SessionCommand) -> Result<SessionCommand, String> {
        let (id, revision, event, release) = match command {
            SessionCommand::Close { connection_id } => {
                // control() seals before its future waits for queue capacity.
                // Revoke held intent now so actor ticks cannot refresh it while
                // Close is queued. Keep request IDs monotonic against late grants.
                self.slots.clear();
                return Ok(SessionCommand::Close { connection_id });
            }
            SessionCommand::Input { id, event } => (id, None, Some(event), false),
            SessionCommand::SourceInput {
                id,
                source_revision,
                event,
            } => (id, Some(source_revision), Some(event), false),
            SessionCommand::ReleaseInput { id } => {
                self.next_owner = self.next_owner.checked_add(1).expect("input owner exhausted");
                if let Some(slot) = self.slots.get_mut(&id) { slot.owner = self.next_owner; }
                (id, None, None, true)
            }
            SessionCommand::SwitchSource {
                id,
                request_id,
                source,
            } => {
                self.source(id, request_id);
                return Ok(SessionCommand::SwitchSource {
                    id,
                    request_id,
                    source,
                });
            }
            SessionCommand::SetActivity {
                id,
                revision,
                video,
                audio,
            } => {
                if let Some(slot) = self.slots.get_mut(&id) {
                    slot.enabled = video;
                    if !video {
                        self.next_owner = self.next_owner.checked_add(1).expect("input owner exhausted");
                        slot.owner = self.next_owner;
                        slot.lease = None;
                        slot.request = 0;
                    }
                }
                return Ok(SessionCommand::SetActivity {
                    id,
                    revision,
                    video,
                    audio,
                });
            }
            SessionCommand::Unsubscribe { id } => {
                self.remove(id);
                return Ok(SessionCommand::Unsubscribe { id });
            }
            SessionCommand::RealtimeInput(_) => {
                return Err("raw input envelopes are not accepted by this connection".into());
            }
            other => return Ok(other),
        };
        let slot = self.slots.get_mut(&id).ok_or("input binding unavailable")?;
        if revision.is_some_and(|r| r != slot.revision) {
            return Err("input source changed".into());
        }
        let lease = slot.lease.as_mut().ok_or("input binding unavailable")?;
        if lease.received.elapsed().as_millis() > 750 {
            slot.lease = None;
            return Err("input binding expired".into());
        }
        lease.expire_frontend(Instant::now());
        let was_empty=lease.held.snapshot().is_empty();
        let payload = if release {
            lease.held = HeldState::default();
            lease.frontend_intent=None;lease.frontend_expired=false;
            InputPayload::Release
        } else {
            let seq = lease
                .sequence
                .checked_add(1)
                .ok_or("input sequence exhausted")?;
            // Preserve the caller's chord. A changed lease must reject stale
            // modifier intent, not silently turn Ctrl+C into a bare C.
            if let Some(protocol::InputEvent::Key {
                pressed: true,
                modifiers,
                ..
            }) = &event
            {
                let owned = lease
                    .held
                    .snapshot()
                    .iter()
                    .fold(0, |mask, (press, _)| match press {
                        protocol::realtime_input::Press::Modifier(bit) => mask | bit,
                        _ => mask,
                    });
                if *modifiers != owned {
                    return Err("input modifier ownership changed".into());
                }
            }
            InputPayload::Event(
                lease
                    .held
                    .event(event.unwrap(), seq)
                    .ok_or("duplicate or unowned input transition")?,
            )
        };
        if !release && was_empty && !lease.held.snapshot().is_empty(){lease.frontend_intent=Some(Instant::now());lease.frontend_expired=false;}
        if lease.held.snapshot().is_empty(){lease.frontend_intent=None;}
        let packet = lease.packet(payload).ok_or("input sequence exhausted")?;
        if release {
            lease.release = Some(packet.sequence);
        }
        Ok(SessionCommand::RealtimeInput(packet))
    }
    pub(crate) fn release_ack(&mut self, binding: InputBinding, sequence: u64) -> bool {
        let Some(lease) = self
            .slots
            .get_mut(&binding.subscription_id)
            .and_then(|s| s.lease.as_mut())
        else {
            return false;
        };
        if lease.binding == binding && lease.release == Some(sequence) {
            lease.release = None;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn composition_survives_lease_renewal_but_keeps_the_commit_age() {
        let mut state=InputState::default();state.source(7,0);grant(&mut state,1);
        let owner=state.text_owner(7,0).unwrap();let old=state.binding(7,0).unwrap();
        grant(&mut state,2);
        assert_eq!(state.text_owner(7,0),Some(owner));
        assert!(state.text_aged(old,"old wire grant",std::time::Duration::ZERO).is_err());
        let commits=state.text_for_owner(owner,"你好🙂",std::time::Duration::from_millis(160)).unwrap();
        assert_eq!(commits[0].binding.epoch,[2;16]);
        let lease=state.slots[&7].lease.as_ref().unwrap();
        let now=lease.host_ms+lease.received.elapsed().as_millis() as u64;
        assert!(now.saturating_sub(commits[0].created_host_ms)>=160);
        assert!(state.text_for_owner(owner,"expired",std::time::Duration::from_millis(251)).is_err());
    }
    #[test]
    fn composition_never_adopts_replaced_suspended_released_or_foreign_owners() {
        let mut state=InputState::default();state.source(7,0);grant(&mut state,1);
        let initial=state.text_owner(7,0).unwrap();
        let mut foreign=initial;foreign.connection_id+=1;
        assert!(state.text_for_owner(foreign,"wrong peer",std::time::Duration::ZERO).is_err());
        state.source(7,0);grant(&mut state,2);
        assert!(state.text_for_owner(initial,"reused source ID",std::time::Duration::ZERO).is_err());
        let owner=state.text_owner(7,0).unwrap();
        state.seal(SessionCommand::SetActivity{id:7,revision:1,video:false,audio:false}).unwrap();
        state.seal(SessionCommand::SetActivity{id:7,revision:2,video:true,audio:true}).unwrap();grant(&mut state,3);
        assert!(state.text_for_owner(owner,"resumed",std::time::Duration::ZERO).is_err());
        let owner=state.text_owner(7,0).unwrap();
        state.seal(SessionCommand::ReleaseInput{id:7}).unwrap();
        assert!(state.text_for_owner(owner,"released",std::time::Duration::ZERO).is_err());
        let owner=state.text_owner(7,0).unwrap();state.invalidate_text_owner(7);
        assert!(state.text_for_owner(owner,"permission revoked",std::time::Duration::ZERO).is_err());
        state.remove(7);assert!(state.text_owner(7,0).is_none());
    }
    #[test]
    fn queued_text_preserves_its_original_age_and_source_epoch() {
        let mut state=InputState::default();state.source(7,0);grant(&mut state,1);
        let binding=state.binding(7,0).unwrap();
        let requests=state.text_aged(binding,"中文🙂",std::time::Duration::from_millis(160)).unwrap();
        let lease=state.slots[&7].lease.as_ref().unwrap();
        let now=lease.host_ms.saturating_add(lease.received.elapsed().as_millis() as u64);
        assert!(now.saturating_sub(requests[0].created_host_ms)>=160);
        assert!(state.text_aged(binding,"stale",std::time::Duration::from_millis(251)).is_err());
        grant(&mut state,2);assert!(state.text_aged(binding,"wrong source",std::time::Duration::ZERO).is_err());
    }
    fn grant(state: &mut InputState, epoch: u8) {
        let SessionCommand::RequestInputBinding { request, .. } = state.requests(1).remove(0)
        else {
            panic!()
        };
        state.grant(
            1,
            InputBinding {
                connection_id: 1,
                subscription_id: 7,
                source_revision: 0,
                epoch: [epoch; 16],
            },
            request,
            1000,
        );
    }
    #[test]
    fn exact_grant_rejects_changed_epoch_without_mutating_new_holds() {
        let mut state = InputState::default();
        state.source(7, 0); grant(&mut state, 1);
        let old = state.binding(7, 0).unwrap();
        state.seal_bound(old, Some(protocol::InputEvent::MouseDown(0))).unwrap();
        grant(&mut state, 2);
        let current = state.binding(7, 0).unwrap();
        assert_ne!(old, current);
        assert!(state.seal_bound(old, Some(protocol::InputEvent::MouseUp(0))).is_err());
        assert!(state.seal_bound(old, Some(protocol::InputEvent::MouseDown(0))).is_err());
        assert!(state.seal_bound(old, None).is_err());
        assert!(matches!(state.snapshots().remove(0).payload, InputPayload::Held(held) if held.is_empty()));
        assert!(state.seal_bound(current, Some(protocol::InputEvent::Touch {
            action: protocol::TouchAction::Down, pointer_id: 0,
            normalized_x: 0.5, normalized_y: 0.5, pressure: 1.0,
        })).is_err(), "raw touch must still fail closed");
        state.seal_bound(current, Some(protocol::InputEvent::MouseDown(0))).unwrap();
        let SessionCommand::RealtimeInput(release) = state.seal_bound(current, None).unwrap() else { panic!() };
        assert_eq!(release.binding, current);
        assert!(matches!(release.payload, InputPayload::Release));
    }
    #[test]
    fn queued_input_is_stamped_before_switch_and_never_rebound() {
        let mut state = InputState::default();
        state.source(7, 0);
        grant(&mut state, 1);
        let SessionCommand::RealtimeInput(old) = state
            .seal(SessionCommand::Input {
                id: 7,
                event: protocol::InputEvent::MouseDown(0),
            })
            .unwrap()
        else {
            panic!()
        };
        state.source(7, 0);
        grant(&mut state, 2);
        let snapshot = state.snapshots().remove(0);
        assert_ne!(old.binding, snapshot.binding);
        let InputPayload::Held(held) = snapshot.payload else {
            panic!()
        };
        assert!(held.is_empty());
    }
    #[test]
    fn release_is_sent_once_and_ack_cannot_follow_reconnect() {
        let mut state = InputState::default();
        state.source(7, 0);
        grant(&mut state, 1);
        let SessionCommand::RealtimeInput(release) =
            state.seal(SessionCommand::ReleaseInput { id: 7 }).unwrap()
        else {
            panic!()
        };
        state.source(7, 0);
        grant(&mut state, 2);
        assert!(!state.release_ack(release.binding, release.sequence));
    }
    #[test]
    fn host_input_time_is_opaque_and_compatible_with_existing_viewer_estimation() {
        use protocol::realtime_input::InputLease;
        for epoch_ms in [0_u64, 1_700_000_000_000] {
            let mut state = InputState::default();
            state.source(7, 0);
            let SessionCommand::RequestInputBinding { request, .. } = state.requests(1).remove(0)
            else {
                panic!()
            };
            let binding = InputBinding {
                connection_id: 1,
                subscription_id: 7,
                source_revision: 0,
                epoch: [1; 16],
            };
            state.grant(1, binding, request, epoch_ms);
            state
                .slots
                .get_mut(&7)
                .unwrap()
                .lease
                .as_mut()
                .unwrap()
                .received = Instant::now() - std::time::Duration::from_millis(40);
            let SessionCommand::RealtimeInput(packet) = state
                .seal(SessionCommand::Input {
                    id: 7,
                    event: protocol::InputEvent::MouseDown(0),
                })
                .unwrap()
            else {
                panic!()
            };
            assert!(packet.created_host_ms >= epoch_ms + 40);
            // A host clock near zero is valid: no client wall-clock comparison,
            // epoch reinterpretation, new wire field or handshake is required.
            let received_host_ms = packet.created_host_ms + 20;
            let mut receiver = InputLease::new(binding, epoch_ms);
            assert_eq!(
                receiver.accept(&packet, received_host_ms),
                Some(vec![protocol::InputEvent::MouseDown(0)])
            );
        }
    }

    #[test]
    fn paused_input_cannot_be_regranted_or_replay_held_intent() {
        let mut state = InputState::default();
        state.source(7, 0);
        grant(&mut state, 1);
        state
            .seal(SessionCommand::Input {
                id: 7,
                event: protocol::InputEvent::ModifiersChanged(2),
            })
            .unwrap();
        let SessionCommand::RequestInputBinding { request, .. } = state.requests(1).remove(0)
        else {
            panic!()
        };
        state
            .seal(SessionCommand::SetActivity {
                id: 7,
                revision: 1,
                video: false,
                audio: false,
            })
            .unwrap();
        state.grant(
            1,
            InputBinding {
                connection_id: 1,
                subscription_id: 7,
                source_revision: 0,
                epoch: [1; 16],
            },
            request,
            1000,
        );
        assert!(!state.ready(7, 0));
        assert!(state.requests(1).is_empty());
        assert!(state.snapshots().is_empty());
        state
            .seal(SessionCommand::SetActivity {
                id: 7,
                revision: 2,
                video: true,
                audio: true,
            })
            .unwrap();
        grant(&mut state, 2);
        assert!(
            state
                .seal(SessionCommand::Input {
                    id: 7,
                    event: protocol::InputEvent::Key {
                        key_code: 8,
                        pressed: true,
                        modifiers: 2
                    }
                })
                .is_err()
        );
        assert!(
            matches!(&state.snapshots()[0].payload,InputPayload::Held(held) if held.is_empty())
        );
    }

    #[test]
    fn lost_grants_expire_and_late_grants_cannot_revive_a_paused_hold() {
        let mut state = InputState::default();
        state.source(7, 0);
        grant(&mut state, 1);
        state
            .seal(SessionCommand::Input {
                id: 7,
                event: protocol::InputEvent::MouseDown(0),
            })
            .unwrap();
        state
            .slots
            .get_mut(&7)
            .unwrap()
            .lease
            .as_mut()
            .unwrap()
            .received = Instant::now() - std::time::Duration::from_millis(751);
        assert!(!state.ready(7, 0));
        assert!(state.snapshots().is_empty());
        assert!(
            state
                .seal(SessionCommand::Input {
                    id: 7,
                    event: protocol::InputEvent::MouseUp(0)
                })
                .is_err()
        );
        grant(&mut state, 2);
        assert!(
            matches!(&state.snapshots()[0].payload,InputPayload::Held(held) if held.is_empty())
        );
    }
    #[test]
    fn network_grants_cannot_keep_a_frozen_frontends_cached_hold_alive() {
        let mut state=InputState::default();state.source(7,0);grant(&mut state,1);
        state.seal(SessionCommand::Input{id:7,event:protocol::InputEvent::MouseDown(0)}).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(330));
        grant(&mut state,1); // Network actor/grant is healthy; frontend did nothing.
        assert!(matches!(&state.snapshots()[0].payload,InputPayload::Held(held) if held.is_empty()),"a transport grant must not renew stale UI hold intent");
    }

    #[test]
    fn healthy_frontend_keeps_long_hold_but_late_refresh_and_old_epoch_cannot_revive_it(){
        let mut state=InputState::default();state.source(7,0);grant(&mut state,1);let binding=state.binding(7,0).unwrap();
        state.seal_bound(binding,Some(protocol::InputEvent::MouseDown(0))).unwrap();
        for _ in 0..5 {
            std::thread::sleep(std::time::Duration::from_millis(100));grant(&mut state,1);
            assert!(state.refresh_bound(binding));assert!(matches!(&state.snapshots()[0].payload,InputPayload::Held(held) if !held.is_empty()));
        }
        std::thread::sleep(std::time::Duration::from_millis(330));grant(&mut state,1);
        assert!(matches!(&state.snapshots()[0].payload,InputPayload::Held(held) if held.is_empty()));
        assert!(!state.refresh_bound(binding));assert!(!state.refresh_bound(binding));
        assert!(state.seal_bound(binding,Some(protocol::InputEvent::MouseUp(0))).is_err());
        state.seal_bound(binding,Some(protocol::InputEvent::MouseDown(0))).unwrap();assert!(state.refresh_bound(binding));
        grant(&mut state,2);assert!(!state.refresh_bound(binding));assert!(matches!(&state.snapshots()[0].payload,InputPayload::Held(held) if held.is_empty()));
    }
    #[test]
    fn transport_snapshot_does_not_refresh_the_frontends_timestamp(){
        let mut state=InputState::default();state.source(7,0);grant(&mut state,1);
        let SessionCommand::RealtimeInput(down)=state.seal(SessionCommand::Input{id:7,event:protocol::InputEvent::MouseDown(0)}).unwrap()else{panic!()};
        std::thread::sleep(std::time::Duration::from_millis(100));let snapshot=state.snapshots().remove(0);
        assert!(snapshot.created_host_ms<=down.created_host_ms+2,"network snapshot made frontend intent younger");
    }

}
