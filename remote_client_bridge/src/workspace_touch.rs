//! UI gestures stay local; only translated events with the current grant enter shared input.
use crate::touch_mapper::{RemoteScreenBounds, TouchMode, TouchStateTracker};
use protocol::{InputEvent, TouchAction, realtime_input::InputBinding};
use std::time::Instant;

pub(crate) struct WorkspaceTouch {
    pub revision: u32,
    pub source: Option<protocol::session::CaptureSource>,
    supported: bool,
    native_touch_points: u8,
    visible: bool,
    surface: u64,
    attached: bool,
    frame_ready: bool,
    binding: Option<InputBinding>,
    last_intent: Option<Instant>,
    tracker: TouchStateTracker,
    keyboard: crate::touch_mapper::VirtualKeyboard,
    cursor: (f64, f64),
    geometry:Option<protocol::viewport::FrameGeometry>,
}

impl WorkspaceTouch {
    pub fn new() -> Self {
        Self {
            revision: 0,
            source: None,
            supported: false,
            native_touch_points: 0,
            visible: true,
            surface: 0,
            attached: false,
            frame_ready: false,
            binding: None,
            last_intent: None,
            tracker: TouchStateTracker::default(),
            keyboard: Default::default(),
            cursor: (0.5, 0.5),
            geometry:None,
        }
    }
    pub fn cancel(&mut self) -> Option<InputBinding> {
        self.tracker.cancel_all();
        self.keyboard=Default::default();
        self.last_intent = None;
        self.binding.take()
    }
    fn cancel_pointer(&mut self) -> Vec<InputEvent> {
        // A local layout/IME inset change ends the gesture, not composition in
        // the same source. Full suspension/end still uses cancel() above.
        self.last_intent = None;
        self.tracker.cancel_all()
    }
    pub fn expire(&mut self, now: Instant) -> Option<InputBinding> {
        if self.last_intent.is_some_and(|last| {
            now.saturating_duration_since(last).as_millis()
                >= protocol::realtime_input::HOLD_TIMEOUT_MS as u128
        }) {
            self.cancel()
        } else {
            None
        }
    }
    pub fn alive(&mut self, token: u64, now: Instant) -> Option<InputBinding> {
        let release = self.expire(now);
        // A transport heartbeat or a late UI callback cannot recreate a cancelled gesture.
        if token == self.surface
            && self.attached
            && self.frame_ready
            && self.visible
            && self.supported
            && self.binding.is_some()
            && self.tracker.has_active_pointers()
        {
            self.last_intent = Some(now);
        }
        release
    }
    /// An old surface's heartbeat cannot attest the replacement gesture.
    pub fn healthy_binding(&self,token:u64)->Option<InputBinding>{
        (token==self.surface&&self.attached&&self.frame_ready&&self.visible&&self.supported&&self.tracker.has_active_pointers()).then_some(self.binding).flatten()
    }
    pub fn invalidate(&mut self) -> Option<InputBinding> {
        self.attached = false;
        self.frame_ready = false;
        self.cancel()
    }
    pub fn support(&mut self, supported: bool) -> Option<InputBinding> {
        self.supported = supported;
        if !supported { self.cursor = (0.5, 0.5); }
        if supported { None } else { self.cancel() }
    }
    pub fn capabilities(&mut self, supported: bool, contacts: u8) -> Option<InputBinding> {
        let next = if supported { contacts.min(protocol::touch::MAX_CONTACTS as u8) } else { 0 };
        let release = (next != self.native_touch_points).then(|| self.cancel()).flatten();
        self.native_touch_points = next;
        self.support(supported).or(release)
    }
    pub fn native_touch_points(&self) -> u8 { self.native_touch_points }
    pub fn current_surface(&self, token: u64) -> bool { token != 0 && token == self.surface && self.attached }
    /// Only a new gesture may change its mode/geometry. Late continuations
    /// cannot cancel or reinterpret the replacement gesture.
    pub fn select_mode(&mut self, token:u64, mode:TouchMode, bounds:RemoteScreenBounds,
        starting:bool) -> Option<Vec<InputEvent>> {
        if token != self.surface || !self.attached || bounds.width == 0 || bounds.height == 0 {
            return None;
        }
        let changed = self.tracker.mode != mode
            || self.tracker.bounds.width != bounds.width || self.tracker.bounds.height != bounds.height;
        if changed && !starting { return None; }
        let releases = if changed {
            self.last_intent = None;
            let releases = if self.tracker.mode != mode {self.tracker.set_mode(mode)}
                else {self.tracker.cancel_all()};
            self.tracker.bounds = bounds;
            releases
        } else { Vec::new() };
        Some(releases)
    }
    fn normalized_trackpad(&mut self, events:Vec<InputEvent>) -> Vec<InputEvent> {
        let trackpad = self.tracker.mode == TouchMode::VirtualTrackpad;
        let mut output = Vec::with_capacity(events.len()+1);
        let position = |point:(f64,f64)|InputEvent::MouseMoveAbsolute {
            x:(point.0*f64::from(u16::MAX)).round() as u16,
            y:(point.1*f64::from(u16::MAX)).round() as u16,
        };
        for event in events {
            match event {
                InputEvent::MouseMove {dx,dy} if trackpad => {
                    self.cursor.0 = (self.cursor.0+f64::from(dx)/f64::from(self.tracker.bounds.width)).clamp(0.,1.);
                    self.cursor.1 = (self.cursor.1+f64::from(dy)/f64::from(self.tracker.bounds.height)).clamp(0.,1.);
                    if let Some(g)=self.geometry {self.cursor=g.clamp_source_point(self.cursor);}
                    output.push(position(self.cursor));
                }
                InputEvent::MouseMoveAbsolute {x,y} => {
                    self.cursor = (f64::from(x)/f64::from(u16::MAX),f64::from(y)/f64::from(u16::MAX));
                    output.push(InputEvent::MouseMoveAbsolute {x,y});
                }
                event @ (InputEvent::MouseDown(_) | InputEvent::MouseScroll {..}) if trackpad => {
                    if let Some(g)=self.geometry {self.cursor=g.clamp_source_point(self.cursor);}
                    // Window input requires source-normalized coordinates. A tap
                    // uses the cursor, never the finger's position on the glass.
                    output.push(position(self.cursor)); output.push(event);
                }
                event => output.push(event),
            }
        }
        output
    }
    pub fn activity(&mut self, visible: bool) -> Option<InputBinding> {
        self.visible = visible;
        if visible { None } else { self.invalidate() }
    }
    pub fn presented_geometry(&mut self,geometry:Option<protocol::viewport::FrameGeometry>) {self.geometry=geometry;}
    pub fn matches_presented_geometry(&self,geometry:Option<protocol::viewport::FrameGeometry>)->bool {
        self.geometry.is_none() || self.geometry==geometry
    }
    pub fn surface(&mut self, token: u64, phase: &str) -> (Option<InputBinding>, Vec<InputEvent>) {
        if phase == "begin" {
            if token == 0 || token <= self.surface {
                return (None, Vec::new());
            }
            let release = self.cancel();
            self.surface = token;
            self.attached = self.visible;
            self.frame_ready = false;
            self.geometry = None;
            return (release, Vec::new());
        }
        if token != self.surface || !self.attached {
            return (None, Vec::new());
        }
        match phase {
            "ready" => {
                self.frame_ready = self.visible;
                (None, Vec::new())
            }
            "reset" => {
                self.frame_ready = false;
                self.geometry = None;
                (self.cancel(), Vec::new())
            }
            "end" => (self.invalidate(), Vec::new()),
            "cancel" => (None, self.cancel_pointer()),
            _ => (None, Vec::new()),
        }
    }
    pub fn keyboard_binding(&self,token:u64,current:Option<InputBinding>)->Option<InputBinding> {
        self.unavailable_reason(token,current).is_none().then_some(current).flatten()
    }
    /// The HUD uses the same source position sent to the host, including its
    /// wire quantization. It must not maintain a second gesture accumulator.
    pub fn cursor(&self,token:u64,current:Option<InputBinding>)->Option<(f64,f64)> {
        self.unavailable_reason(token,current).is_none().then(|| {
            let quantize=|value:f64|(value*f64::from(u16::MAX)).round()/f64::from(u16::MAX);
            (quantize(self.cursor.0),quantize(self.cursor.1))
        })
    }
    pub fn unavailable_reason(&self,token:u64,current:Option<InputBinding>)->Option<&'static str> {
        if token==0 || token!=self.surface {Some("This video view has changed.")}
        else if !self.visible {Some("Control is paused while this stream is hidden.")}
        else if !self.attached {Some("Control is waiting for the video view.")}
        else if !self.frame_ready {Some("Control is waiting for the first video frame.")}
        else if !self.supported {Some("This source supports viewing only.")}
        else if current.is_none() {Some("Control is waiting for this device connection.")}
        else {None}
    }
    pub fn keyboard_holds_modifiers(&self,binding:InputBinding)->bool {
        self.keyboard.holds_modifiers_for(binding)
    }
    pub fn key_events(&mut self,binding:InputBinding,name:&str,pressed:bool)->Vec<InputEvent> {
        self.keyboard.event_for_grant(binding,name,pressed)
    }
    pub fn touch(
        &mut self,
        token: u64,
        event: InputEvent,
        current: Option<InputBinding>,
        now: Instant,
    ) -> (Option<InputBinding>, Vec<InputEvent>) {
        if token != self.surface || !self.attached {
            return (None, Vec::new());
        }
        if !self.visible || !self.frame_ready || !self.supported || current.is_none() {
            return (self.cancel(), Vec::new());
        }
        let expired = self.expire(now);
        let release = if self.binding != current {
            let release = self.cancel().or(expired);
            self.binding = current;
            release
        } else {
            expired
        };
        let InputEvent::Touch {
            action,
            pointer_id,
            normalized_x,
            normalized_y,
            pressure,
        } = event
        else {
            return (self.cancel().or(release), Vec::new());
        };
        if action == TouchAction::Cancel && self.tracker.mode != TouchMode::NativeTouch {
            return (release, self.cancel_pointer());
        }
        let events = self.tracker.process_touch(
            action,
            pointer_id,
            normalized_x,
            normalized_y,
            pressure,
            now,
        );
        self.last_intent = self.tracker.has_active_pointers().then_some(now);
        (release, self.normalized_trackpad(events))
    }
    pub fn touch_frame(&mut self, token: u64, contacts: Vec<protocol::touch::Contact>,
        current: Option<InputBinding>, now: Instant) -> (Option<InputBinding>, Vec<InputEvent>) {
        if !self.current_surface(token) { return (None,Vec::new()); }
        if self.native_touch_points == 0 || !protocol::touch::valid_frame(&contacts)
            || contacts.len() > usize::from(self.native_touch_points)
            || self.tracker.mode != TouchMode::NativeTouch {
            return (self.cancel(),Vec::new());
        }
        let mut released = None;
        let mut output = Vec::with_capacity(contacts.len());
        for c in contacts {
            let (release,events) = self.touch(token,InputEvent::Touch {action:c.action,pointer_id:c.pointer_id,
                normalized_x:c.normalized_x,normalized_y:c.normalized_y,pressure:c.pressure},current,now);
            released = release.or(released);
            for event in events { if let InputEvent::TouchFrame(points) = event { output.extend(points); } }
        }
        if !output.is_empty() && !protocol::touch::valid_frame(&output) {
            return (self.cancel().or(released),Vec::new());
        }
        let events = if output.is_empty() {Vec::new()} else {vec![InputEvent::TouchFrame(output)]};
        (released,events)
    }
    pub fn binding(&self) -> Option<InputBinding> {
        self.binding
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trackpad_moves_a_source_bound_cursor_and_taps_without_relative_wire_input() {
        let mut touch=ready(); let grant=Some(binding(1));let now=Instant::now();
        assert!(touch.select_mode(1,TouchMode::VirtualTrackpad,
            RemoteScreenBounds {width:640,height:420},true).unwrap().is_empty());
        assert!(touch.touch(1,event(TouchAction::Down,0,0.1),grant,now).1.is_empty());
        let moved=touch.touch(1,event(TouchAction::Move,0,0.2),grant,now).1;
        assert!(matches!(moved.as_slice(),[InputEvent::MouseMoveAbsolute {x,y}]
            if (i32::from(*x)-42598).abs()<3 && (i32::from(*y)-32768).abs()<2));
        let cursor=touch.cursor(1,grant).unwrap();
        let InputEvent::MouseMoveAbsolute {x,y}=&moved[0] else {panic!("absolute cursor expected")};
        assert_eq!(cursor,(f64::from(*x)/65535.,f64::from(*y)/65535.));
        assert!(touch.cursor(2,grant).is_none());
        assert!(touch.cursor(1,None).is_none());
        touch.touch(1,event(TouchAction::Up,0,0.2),grant,now);
        touch.touch(1,event(TouchAction::Down,0,0.9),grant,now);
        let clicked=touch.touch(1,event(TouchAction::Up,0,0.9),grant,now).1;
        assert!(matches!(clicked.as_slice(),[InputEvent::MouseMoveAbsolute {x,..},
            InputEvent::MouseDown(0),InputEvent::MouseUp(0)] if (i32::from(*x)-42598).abs()<3));
    }
    #[test]
    fn mode_switch_releases_drag_and_ignores_old_mode_geometry_or_surface() {
        let mut touch=ready();let now=Instant::now();let grant=Some(binding(1));
        touch.touch(1,event(TouchAction::Down,0,0.2),grant,now);
        touch.touch(1,event(TouchAction::Move,0,0.5),grant,now);
        let bounds=RemoteScreenBounds {width:640,height:420};
        assert_eq!(touch.select_mode(1,TouchMode::VirtualTrackpad,bounds.clone(),true),Some(vec![InputEvent::MouseUp(0)]));
        assert!(touch.select_mode(1,TouchMode::DirectTouch,bounds.clone(),false).is_none());
        assert!(touch.select_mode(2,TouchMode::DirectTouch,bounds,true).is_none());
        assert!(touch.select_mode(1,TouchMode::VirtualTrackpad,RemoteScreenBounds {width:0,height:420},true).is_none());
    }
    fn binding(epoch: u8) -> InputBinding {
        InputBinding {
            connection_id: 4,
            subscription_id: 256,
            source_revision: 0,
            epoch: [epoch; 16],
        }
    }
    fn event(action: TouchAction, pointer_id: u32, x: f32) -> InputEvent {
        InputEvent::Touch {
            action,
            pointer_id,
            normalized_x: x,
            normalized_y: 0.5,
            pressure: 1.0,
        }
    }
    fn ready() -> WorkspaceTouch {
        let mut touch = WorkspaceTouch::new();
        touch.support(true);
        touch.surface(1, "begin");
        touch.surface(1, "ready");
        touch
    }
    #[test]
    fn native_samples_keep_contacts_simultaneous_and_cancel_only_owned_surfaces() {
        let mut touch=ready();touch.capabilities(true,10);
        touch.select_mode(1,TouchMode::NativeTouch,RemoteScreenBounds{width:400,height:800},true).unwrap();
        let grant=Some(binding(1));let now=Instant::now();
        let point=|id,action,x|protocol::touch::Contact{pointer_id:id,action,normalized_x:x,normalized_y:0.5,pressure:1.};
        let first=vec![point(1,TouchAction::Down,0.1),point(8,TouchAction::Down,0.8)];
        assert_eq!(touch.touch_frame(1,first.clone(),grant,now).1,vec![InputEvent::TouchFrame(first)]);
        let next=vec![point(1,TouchAction::Move,0.3),point(8,TouchAction::Up,0.9)];
        let result=touch.touch_frame(1,next,grant,now).1;
        assert!(matches!(result.as_slice(),[InputEvent::TouchFrame(c)] if c.len()==2 && c[0].pointer_id==1 && c[1].pressure==0.));
        assert_eq!(touch.healthy_binding(1),grant);
        assert!(touch.touch_frame(99,vec![point(1,TouchAction::Cancel,0.)],grant,now).1.is_empty());
        assert!(touch.touch_frame(99,vec![point(1,TouchAction::Down,f32::NAN)],grant,now).0.is_none());
        assert_eq!(touch.healthy_binding(1),grant);
        let cancelled=touch.surface(1,"cancel").1;
        assert!(matches!(cancelled.as_slice(),[InputEvent::TouchFrame(c)] if c.len()==1 && c[0].pointer_id==1 && c[0].action==TouchAction::Cancel));
        assert!(touch.healthy_binding(1).is_none());
    }
    #[test]
    fn unavailable_reasons_follow_the_same_surface_and_grant_fences_as_input() {
        let grant=Some(binding(1));
        let mut touch=WorkspaceTouch::new();
        assert!(touch.unavailable_reason(1,grant).unwrap().contains("changed"));
        touch.support(true);touch.surface(1,"begin");
        assert!(touch.unavailable_reason(1,grant).unwrap().contains("first video frame"));
        touch.surface(1,"ready");
        assert!(touch.unavailable_reason(1,None).unwrap().contains("device connection"));
        assert_eq!(touch.unavailable_reason(1,grant),None);
        assert_eq!(touch.keyboard_binding(1,grant),grant);
        touch.activity(false);
        assert!(touch.unavailable_reason(1,grant).unwrap().contains("hidden"));
        touch.activity(true);
        assert!(touch.unavailable_reason(1,grant).unwrap().contains("video view"));
        assert_eq!(touch.keyboard_binding(1,grant),None);
        touch.surface(2,"begin");touch.surface(2,"ready");
        assert_eq!(touch.keyboard_binding(1,grant),None);
        assert_eq!(touch.keyboard_binding(2,grant),grant);
        touch.support(false);
        assert!(touch.unavailable_reason(2,grant).unwrap().contains("viewing only"));
        assert_eq!(touch.keyboard_binding(2,grant),None);
    }
    #[test]
    fn current_frame_support_and_grant_are_all_required() {
        let mut touch = WorkspaceTouch::new();
        let now = Instant::now();
        touch.surface(1, "begin");
        for (supported, frame, grant) in [
            (false, true, Some(binding(1))),
            (true, false, Some(binding(1))),
            (true, true, None),
        ] {
            touch.support(supported);
            touch.surface(1, if frame { "ready" } else { "reset" });
            assert!(
                touch
                    .touch(1, event(TouchAction::Down, 7, 0.1), grant, now)
                    .1
                    .is_empty()
            );
        }
        touch.surface(1, "ready");
        assert!(
            !touch
                .touch(1, event(TouchAction::Down, 7, 0.1), Some(binding(1)), now)
                .1
                .is_empty()
        );
    }
    #[test]
    fn drag_is_translated_and_cancel_discards_the_tail() {
        let mut touch = ready();
        let now = Instant::now();
        let grant = Some(binding(1));
        let down = touch
            .touch(1, event(TouchAction::Down, 7, 0.1), grant, now)
            .1;
        let drag = touch
            .touch(1, event(TouchAction::Move, 7, 0.8), grant, now)
            .1;
        assert!(drag.contains(&InputEvent::MouseDown(0)));
        assert!(
            !down
                .iter()
                .chain(&drag)
                .any(|event| matches!(event, InputEvent::Touch { .. }))
        );
        assert_eq!(touch.touch(1, event(TouchAction::Cancel, 7, 0.8), grant, now),
            (None, vec![InputEvent::MouseUp(0)]));
        assert!(
            touch
                .touch(1, event(TouchAction::Up, 7, 0.8), grant, now)
                .1
                .is_empty()
        );
    }
    #[test]
    fn changed_epoch_cannot_complete_an_old_click_or_drag() {
        let mut touch = ready();
        let now = Instant::now();
        touch.touch(1, event(TouchAction::Down, 7, 0.1), Some(binding(1)), now);
        let (release, events) =
            touch.touch(1, event(TouchAction::Up, 7, 0.1), Some(binding(2)), now);
        assert_eq!(release, Some(binding(1)));
        assert!(events.is_empty());
        assert!(
            !touch
                .touch(1, event(TouchAction::Down, 8, 0.3), Some(binding(2)), now)
                .1
                .is_empty()
        );
    }
    #[test]
    fn hidden_destroyed_and_replaced_surfaces_reject_late_callbacks() {
        let mut touch = ready();
        let now = Instant::now();
        let grant = Some(binding(1));
        touch.touch(1, event(TouchAction::Down, 7, 0.1), grant, now);
        assert_eq!(touch.activity(false), grant);
        touch.activity(true);
        touch.surface(1, "ready");
        assert!(
            touch
                .touch(1, event(TouchAction::Up, 7, 0.1), grant, now)
                .1
                .is_empty()
        );
        touch.surface(2, "begin");
        touch.surface(2, "ready");
        touch.touch(2, event(TouchAction::Down, 8, 0.3), grant, now);
        touch.surface(1, "end");
        touch.surface(1, "begin");
        assert!(
            touch
                .touch(1, event(TouchAction::Up, 7, 0.1), grant, now)
                .1
                .is_empty()
        );
        assert!(
            !touch
                .touch(2, event(TouchAction::Up, 8, 0.3), grant, now)
                .1
                .is_empty()
        );
        touch.surface(2, "end");
        touch.surface(2, "ready");
        assert!(
            touch
                .touch(2, event(TouchAction::Down, 9, 0.3), grant, now)
                .1
                .is_empty()
        );
    }
    #[test]
    fn frozen_ui_expires_and_late_alive_or_gesture_tail_cannot_revive_hold() {
        use std::time::Duration;
        let mut touch = ready();
        let now = Instant::now();
        let grant = Some(binding(1));
        touch.touch(1, event(TouchAction::Down, 7, 0.1), grant, now);
        touch.touch(1, event(TouchAction::Move, 7, 0.8), grant, now);
        assert_eq!(touch.alive(1, now + Duration::from_millis(100)), None);
        assert_eq!(touch.expire(now + Duration::from_millis(399)), None);
        assert_eq!(touch.expire(now + Duration::from_millis(400)), grant);
        assert_eq!(touch.binding(), None);
        assert_eq!(touch.alive(1, now + Duration::from_millis(401)), None);
        assert_eq!(touch.binding(), None);
        assert!(
            touch
                .touch(
                    1,
                    event(TouchAction::Move, 7, 0.9),
                    grant,
                    now + Duration::from_millis(402)
                )
                .1
                .is_empty()
        );
        assert!(
            touch
                .touch(
                    1,
                    event(TouchAction::Up, 7, 0.9),
                    grant,
                    now + Duration::from_millis(403)
                )
                .1
                .is_empty()
        );
        assert!(
            !touch
                .touch(
                    1,
                    event(TouchAction::Down, 8, 0.2),
                    grant,
                    now + Duration::from_millis(404)
                )
                .1
                .is_empty()
        );
        // Even when the watchdog is delayed, a late heartbeat expires before refreshing.
        assert_eq!(touch.alive(1, now + Duration::from_millis(704)), grant);
        assert_eq!(touch.binding(), None);
    }
    #[test]
    fn pointers_and_panes_have_independent_ownership() {
        let mut first = ready();
        let mut second = ready();
        let now = Instant::now();
        let grant = Some(binding(1));
        first.touch(1, event(TouchAction::Down, 7, 0.1), grant, now);
        assert!(
            first
                .touch(1, event(TouchAction::Move, 8, 0.8), grant, now)
                .1
                .is_empty()
        );
        assert!(
            second
                .touch(1, event(TouchAction::Up, 7, 0.1), grant, now)
                .1
                .is_empty()
        );
        assert!(
            !first
                .touch(1, event(TouchAction::Up, 7, 0.1), grant, now)
                .1
                .is_empty()
        );
    }
    #[test]
    fn only_current_live_surface_can_refresh_a_held_binding(){
        let mut touch=ready();let now=Instant::now();let grant=Some(binding(1));
        touch.touch(1,event(TouchAction::Down,7,0.1),grant,now);touch.touch(1,event(TouchAction::Move,7,0.8),grant,now);
        touch.alive(1,now+std::time::Duration::from_millis(100));assert_eq!(touch.healthy_binding(1),grant);assert_eq!(touch.healthy_binding(2),None);
        touch.alive(1,now+std::time::Duration::from_millis(500));assert_eq!(touch.healthy_binding(1),None);
    }

}
