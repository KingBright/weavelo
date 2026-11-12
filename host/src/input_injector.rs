use core_graphics::display::CGDisplay;
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton, EventField,
    ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;
use protocol::InputEvent;
use remote_core::InputInjector;
use std::collections::BTreeSet;
use std::error::Error;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

// CoreGraphics CGEventTypes.h: these values differ from AppKit NSEventPhase.
const SCROLL_PHASE_FIELD: u32 = 99;
const MOMENTUM_PHASE_FIELD: u32 = 123;
fn native_scroll_phases(gesture: protocol::ScrollGesture) -> (i64, i64) {
    use protocol::{MomentumPhase, ScrollPhase};
    let phase = match gesture.phase {
        ScrollPhase::None => 0,
        ScrollPhase::MayBegin => 128,
        ScrollPhase::Began => 1,
        ScrollPhase::Changed => 2,
        ScrollPhase::Ended => 4,
        ScrollPhase::Cancelled => 8,
    };
    let momentum = match gesture.momentum {
        MomentumPhase::None => 0,
        MomentumPhase::Began => 1,
        MomentumPhase::Changed => 2,
        MomentumPhase::Ended => 3,
    };
    (phase, momentum)
}

enum WindowScrollContinuation {
    Other,
    Ignore,
    Validated(CGPoint),
}

fn window_scroll_continuation(
    event: &InputEvent,
    point: Option<(u16, u16)>,
    check: impl FnOnce(u16, u16) -> Result<CGPoint, String>,
) -> WindowScrollContinuation {
    if !matches!(event, InputEvent::MouseScroll { delta_x, delta_y, gesture }
        if !gesture.can_take_control(*delta_x, *delta_y)) {
        return WindowScrollContinuation::Other;
    }
    // An obsolete gesture must not activate or send to another target. Reuse
    // only this event's successful live hit test and its exact coordinates.
    match point.and_then(|(x, y)| check(x, y).ok()) {
        Some(position) => WindowScrollContinuation::Validated(position),
        None => WindowScrollContinuation::Ignore,
    }
}

pub struct MacInputInjector {
    source: CGEventSource,
    #[cfg(test)]
    post_observer: Option<std::sync::Arc<dyn Fn(&CGEvent) + Send + Sync>>,
    display_id: Option<u32>,
    window: Option<crate::window_input::WindowTarget>,
    target_pointer: Mutex<Option<(u16, u16)>>,
    pressed_buttons: AtomicU8,
    active_modifiers: AtomicU8,
    pressed_keys: Mutex<BTreeSet<u16>>,
    scroll_gesture: Mutex<protocol::ScrollGesture>,
}

unsafe impl Send for MacInputInjector {}
unsafe impl Sync for MacInputInjector {}

impl MacInputInjector {
    pub fn new() -> Result<Self, Box<dyn Error + Send + Sync>> {
        Self::for_display(CGDisplay::main().id)
    }

    /// Bind normalized input to the actual selected display, including its
    /// global logical origin. The shared host must pass the captured display ID.
    pub fn for_display(id: u32) -> Result<Self, Box<dyn Error + Send + Sync>> {
        require_post_event_access(unsafe { CGPreflightPostEventAccess() })?;
        display_bounds(id)?;
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|_| "Failed to create CGEventSource")?;
        Ok(Self {
            source,
            #[cfg(test)]
            post_observer: None,
            display_id: Some(id),
            window: None,
            target_pointer: Mutex::new(None),
            pressed_buttons: AtomicU8::new(0),
            active_modifiers: AtomicU8::new(0),
            pressed_keys: Mutex::new(BTreeSet::new()),
            scroll_gesture: Mutex::new(protocol::ScrollGesture::default()),
        })
    }

    pub fn for_window(id: u32, pid: i32) -> Result<Self, Box<dyn Error + Send + Sync>> {
        require_post_event_access(unsafe { CGPreflightPostEventAccess() })?;
        let window = crate::window_input::WindowTarget::new(id, pid)?;
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|_| "Cannot create window input source")?;
        Ok(Self {
            source,
            #[cfg(test)]
            post_observer: None,
            display_id: None,
            window: Some(window),
            target_pointer: Mutex::new(None),
            pressed_buttons: AtomicU8::new(0),
            active_modifiers: AtomicU8::new(0),
            pressed_keys: Mutex::new(BTreeSet::new()),
            scroll_gesture: Mutex::new(protocol::ScrollGesture::default()),
        })
    }
    fn post(&self, event: &CGEvent, deadline: Option<Instant>) -> bool {
        if let Some(window) = &self.window {
            if !window.alive() {
                // A destroyed target has no safe global release destination.
                // Consume ownership, drop the post, never redirect to a new app.
                return false;
            }
            event.set_integer_value_field(
                EventField::MOUSE_EVENT_WINDOW_UNDER_MOUSE_POINTER,
                i64::from(window.id),
            );
            event.set_integer_value_field(
                EventField::MOUSE_EVENT_WINDOW_UNDER_MOUSE_POINTER_THAT_CAN_HANDLE_THIS_EVENT,
                i64::from(window.id),
            );
            // This is explicitly foreground-window control, not background posting.
        }
        // Last application check, after target validation and event construction.
        if deadline.is_some_and(|d| Instant::now() >= d) {
            return false;
        }
        #[cfg(test)]
        if let Some(record) = &self.post_observer {
            record(event);
            return true;
        }
        event.post(CGEventTapLocation::HID);
        true
    }
    fn pointer_position(&self) -> CGPoint {
        if let Some(window) = &self.window {
            if let (Ok(bounds), Some((x, y))) = (
                window.bounds(),
                *self.target_pointer.lock().expect("source pointer lock"),
            ) {
                return normalized_pointer_location(x, y, bounds);
            }
            return CGPoint::new(0., 0.);
        }
        if let Some(id) = self.display_id {
            if let Ok(bounds) = display_bounds(id) {
                if let Some((x, y)) = *self.target_pointer.lock().expect("source pointer lock") {
                    return normalized_pointer_location(x, y, bounds);
                }
                return clamp_display_pointer(current_pointer_location(&self.source), bounds);
            }
        }
        current_pointer_location(&self.source)
    }
    fn scroll_continuation(&self, event: &InputEvent) -> WindowScrollContinuation {
        let Some(window) = &self.window else { return WindowScrollContinuation::Other; };
        let point = *self.target_pointer.lock().expect("source pointer lock");
        window_scroll_continuation(event, point, |x, y| window.check_pointer(x, y))
    }
    fn prepare_window_event(&self, event: &InputEvent, deadline: Option<Instant>) -> Result<(), Box<dyn Error + Send + Sync>> {
        let Some(window) = &self.window else {
            display_bounds(self.display_id.ok_or("Input display is not bound")?)?;
            if matches!(
                event,
                InputEvent::MouseDown(_) | InputEvent::MouseScroll { .. }
            ) && self
                .target_pointer
                .lock()
                .map_err(|_| "Source pointer lock poisoned")?
                .is_none()
            {
                return Err("Pointer location for the selected display is unconfirmed. Click inside its remote picture before control".into());
            }
            if let InputEvent::Touch {
                normalized_x,
                normalized_y,
                ..
            } = event
                && (!normalized_x.is_finite() || !normalized_y.is_finite())
            {
                return Err("Invalid touch coordinates".into());
            }
            return Ok(());
        };
        let click = matches!(event, InputEvent::MouseDown(_)
            | InputEvent::Touch { action: protocol::TouchAction::Down, .. });
        // A click checks live bounds before activation and again in its hit test.
        // Other events still validate their current selected window here.
        if !click && !matches!(event, InputEvent::MouseScroll { .. }) { window.bounds()?; }
        if matches!(event, InputEvent::MouseMove { .. }) {
            return Err("Window control requires normalized absolute coordinates".into());
        }
        if matches!(event, InputEvent::MouseMoveAbsolute { .. }) {
            return Ok(());
        }
        if matches!(
            event,
            InputEvent::MouseDown(_) | InputEvent::MouseScroll { .. }
        ) && self
            .target_pointer
            .lock()
            .expect("source pointer lock")
            .is_none()
        {
            return Err("Click inside the selected remote window before using its pointer".into());
        }
        if click {
            let point = match event {
                InputEvent::Touch {
                    normalized_x,
                    normalized_y,
                    ..
                } => {
                    if !normalized_x.is_finite() || !normalized_y.is_finite() {
                        return Err("Invalid touch coordinates".into());
                    }
                    (
                        (normalized_x.clamp(0., 1.) * 65535.) as u16,
                        (normalized_y.clamp(0., 1.) * 65535.) as u16,
                    )
                }
                _ => self
                    .target_pointer
                    .lock()
                    .expect("source pointer lock")
                    .ok_or("Pointer position missing")?,
            };
            window.focus_for_input(Some(point), deadline)?;
            return Ok(());
        }
        // Release only previously tracked presses. Source changes also release held state.
        if matches!(
            event,
            InputEvent::MouseUp(_)
                | InputEvent::KeyUp(_)
                | InputEvent::Key { pressed: false, .. }
                | InputEvent::Touch {
                    action: protocol::TouchAction::Up | protocol::TouchAction::Cancel,
                    ..
                }
        ) {
            return Ok(());
        }
        if let InputEvent::ModifiersChanged(mask) = event
            && mask & !self.current_modifiers() & 0x0f == 0
        {
            // Removing old modifiers is cleanup, not intent to switch the
            // foreground window back after another viewer has taken control.
            return Ok(());
        }
        if matches!(event, InputEvent::MouseScroll { .. }) {
            let (x, y) = self
                .target_pointer
                .lock()
                .expect("source pointer lock")
                .ok_or("Pointer position missing")?;
            // Scrolling is fresh pointer intent, just like clicking. Restore
            // the selected application before the live hit test; otherwise a
            // background app's first wheel event permanently pauses control.
            // The hit test still prevents delivery to another window/app.
            window.focus_for_input(Some((x, y)), deadline)?;
            return Ok(());
        }
        // A fresh key/text event is already bound to this selected source.
        // Switching viewers must not require a synthetic click in an editor.
        // Tracked releases return above and never activate an old target.
        window.focus_for_input(None, deadline)?;
        Ok(())
    }

    fn commit_text(&self, text: &protocol::interaction::CommittedText, deadline: Option<Instant>) -> Result<(), Box<dyn Error + Send + Sync>> {
        if self.current_modifiers() & 0x0f != 0 && !self.set_modifiers(0, deadline) {
            return Err("text input target changed or dispatch expired".into());
        }
        // Small UTF-16 batches retain surrogate pairs, with no keyboard-layout or
        // clipboard mutation. Native dispatch confirmation is not field readback.
        for chunk in unicode_event_chunks(text.as_str()) {
            let units: Vec<u16> = chunk.encode_utf16().collect();
            for pressed in [true, false] {
                let event = CGEvent::new_keyboard_event(self.source.clone(), 0, pressed)
                    .map_err(|_| "cannot create native Unicode event")?;
                event.set_flags(CGEventFlags::empty());
                event.set_string_from_utf16_unchecked(&units);
                if !self.post(&event, deadline) {
                    return Err("text input target changed or dispatch expired".into());
                }
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn inject(&self, event: InputEvent, deadline: Option<Instant>) {
        self.inject_with_scroll_position(event, deadline, None);
    }

    fn inject_with_scroll_position(&self, event: InputEvent, deadline: Option<Instant>, scroll_position: Option<CGPoint>) {
        match event {
            InputEvent::Text(_) => unreachable!("text uses checked native dispatch"),
            InputEvent::KeyDown(scancode) => {
                self.post_tracked_key(scancode as u16, true, self.current_modifiers(), deadline);
            }
            InputEvent::KeyUp(scancode) => {
                self.post_tracked_key(scancode as u16, false, self.current_modifiers(), deadline);
            }
            InputEvent::Key {
                key_code,
                pressed,
                modifiers,
            } => {
                let modifiers = if pressed {
                    modifiers
                } else {
                    modifiers & self.current_modifiers()
                };
                if !self.set_modifiers(modifiers, deadline) {
                    return;
                }
                self.post_tracked_key(key_code, pressed, modifiers, deadline);
            }
            InputEvent::ModifiersChanged(modifiers) => {
                if !self.set_modifiers(modifiers, deadline) {
                    return;
                }
            }
            InputEvent::MouseMove { dx, dy } => {
                let current_pos = self.pointer_position();
                let mut new_pos =
                    CGPoint::new(current_pos.x + dx as f64, current_pos.y + dy as f64);
                if let Some(id) = self.display_id {
                    let Ok(bounds) = display_bounds(id) else {
                        return;
                    };
                    new_pos = clamp_display_pointer(new_pos, bounds);
                    *self.target_pointer.lock().expect("source pointer lock") =
                        Some(pointer_coordinates(new_pos, bounds));
                }
                if let Ok(cg_event) = CGEvent::new_mouse_event(
                    self.source.clone(),
                    pointer_event_type(self.pressed_buttons.load(Ordering::Relaxed)),
                    new_pos,
                    pointer_mouse_button(self.pressed_buttons.load(Ordering::Relaxed)),
                ) {
                    self.post(&cg_event, deadline);
                }
            }
            InputEvent::MouseMoveAbsolute { x, y } => {
                let bounds = if let Some(window) = &self.window {
                    let Ok(bounds) = window.bounds() else {
                        return;
                    };
                    *self.target_pointer.lock().expect("source pointer lock") = Some((x, y));
                    if !window.focused() || window.check_pointer(x, y).is_err() {
                        return;
                    }
                    bounds
                } else {
                    let Some(id) = self.display_id else {
                        return;
                    };
                    let Ok(bounds) = display_bounds(id) else {
                        return;
                    };
                    *self.target_pointer.lock().expect("source pointer lock") = Some((x, y));
                    bounds
                };
                self.post_pointer_move(normalized_pointer_location(x, y, bounds), deadline);
            }
            InputEvent::MouseDown(button) => {
                let position = self.pointer_position();
                if self.window.is_some() || self.display_id.is_some() {
                    self.post_pointer_move(position, deadline);
                }
                self.post_mouse_button_at(button, true, position, deadline);
            }
            InputEvent::MouseUp(button) => {
                self.post_mouse_button(button, false, deadline);
            }
            InputEvent::MouseScroll { delta_x, delta_y, gesture } => {
                let position = scroll_position.unwrap_or_else(|| self.pointer_position());
                if let Ok(cg_event) = CGEvent::new_scroll_event(
                    self.source.clone(),
                    ScrollEventUnit::PIXEL,
                    2,
                    delta_y,
                    delta_x,
                    0,
                ) {
                    // Bind the wheel directly to its source coordinates. An
                    // extra synthetic mouse move adds native work and hover
                    // side effects without changing the scroll target.
                    // CGEvent dereferences to the borrowed native event. The
                    // existing binding does not wrap CGEventSetLocation.
                    unsafe { CGEventSetLocation(std::ptr::from_ref(&*cg_event).cast_mut().cast(),position); }
                    let (phase, momentum) = native_scroll_phases(gesture);
                    cg_event.set_integer_value_field(SCROLL_PHASE_FIELD, phase);
                    cg_event.set_integer_value_field(MOMENTUM_PHASE_FIELD, momentum);
                    if self.post(&cg_event, deadline) {
                        *self.scroll_gesture.lock().expect("scroll gesture lock") = gesture;
                    }
                }
            }
            // macOS advertises no native touchscreen capability. Never convert
            // a multi-contact sample into a different mouse action.
            InputEvent::TouchFrame(_) => {}
            InputEvent::Touch {
                action,
                pointer_id: _,
                normalized_x,
                normalized_y,
                pressure: _,
            } => {
                let bounds = if let Some(window) = &self.window {
                    let Ok(bounds) = window.bounds() else {
                        return;
                    };
                    bounds
                } else {
                    let Some(id) = self.display_id else {
                        return;
                    };
                    let Ok(bounds) = display_bounds(id) else {
                        return;
                    };
                    bounds
                };
                let x = (normalized_x.clamp(0.0, 1.0) * f32::from(u16::MAX)) as u16;
                let y = (normalized_y.clamp(0.0, 1.0) * f32::from(u16::MAX)) as u16;
                if self.window.is_some() || self.display_id.is_some() {
                    *self.target_pointer.lock().expect("source pointer lock") = Some((x, y));
                }
                let pos = normalized_pointer_location(x, y, bounds);
                self.post_pointer_move(pos, deadline);

                match action {
                    protocol::TouchAction::Down => {
                        self.post_mouse_button_at(0, true, pos, deadline);
                    }
                    protocol::TouchAction::Move => {}
                    protocol::TouchAction::Up | protocol::TouchAction::Cancel => {
                        self.post_mouse_button_at(0, false, pos, deadline);
                    }
                }
            }
        }
    }

    fn cancel_scroll(&self) {
        self.cancel_scroll_with(None, || {
            if let Some(window) = &self.window {
                let point = *self.target_pointer.lock().expect("source pointer lock");
                let (x, y) = point.ok_or(())?;
                window.check_pointer(x, y).map(Some).map_err(|_| ())
            } else if self.display_id.is_some_and(|id| display_bounds(id).is_err()) {
                Err(())
            } else {
                Ok(None)
            }
        });
    }

    fn cancel_scroll_with(&self, deadline: Option<Instant>, validate: impl FnOnce() -> Result<Option<CGPoint>, ()>) {
        let gesture = std::mem::take(&mut *self.scroll_gesture.lock().expect("scroll gesture lock"));
        let Some(gesture) = gesture.cancellation() else { return; };
        let Ok(position) = validate() else { return; };
        // End only this owner's native sequence; never raise a displaced window.
        self.inject_with_scroll_position(InputEvent::MouseScroll { delta_x: 0, delta_y: 0, gesture }, deadline, position);
    }

    pub fn release_all_input(&self) {
        self.cancel_scroll();
        let pressed_keys = {
            let mut keys = self.pressed_keys.lock().expect("pressed keys lock");
            std::mem::take(&mut *keys)
        };
        for key_code in pressed_keys {
            self.post_key(key_code, false, 0, None);
        }
        let pressed_buttons = self.pressed_buttons.load(Ordering::Relaxed);
        for button in 0..=2 {
            if pressed_buttons & (1 << button) != 0 {
                self.post_mouse_button(button, false, None);
            }
        }
        self.pressed_buttons.store(0, Ordering::Relaxed);
        self.set_modifiers(0, None);
        *self.target_pointer.lock().expect("source pointer lock") = None;
    }

    fn current_modifiers(&self) -> u8 {
        self.active_modifiers.load(Ordering::Relaxed)
    }

    fn post_key(
        &self,
        key_code: u16,
        pressed: bool,
        modifiers: u8,
        deadline: Option<Instant>,
    ) -> bool {
        if let Ok(cg_event) = CGEvent::new_keyboard_event(self.source.clone(), key_code, pressed) {
            cg_event.set_flags(modifier_event_flags(modifiers));
            return self.post(&cg_event, deadline);
        }
        false
    }

    fn post_tracked_key(
        &self,
        key_code: u16,
        pressed: bool,
        modifiers: u8,
        deadline: Option<Instant>,
    ) {
        let mut keys = self.pressed_keys.lock().expect("pressed keys lock");
        if keys.contains(&key_code) == pressed {
            return;
        }
        if self.post_key(key_code, pressed, modifiers, deadline) {
            if pressed {
                keys.insert(key_code);
            } else {
                keys.remove(&key_code);
            }
        }
    }

    fn set_modifiers(&self, modifiers: u8, deadline: Option<Instant>) -> bool {
        for (flag, key_code) in modifier_key_codes() {
            if (self.current_modifiers() ^ modifiers) & flag != 0 {
                let down = modifiers & flag != 0;
                let accepted_flags = if down {
                    self.current_modifiers() | flag
                } else {
                    self.current_modifiers() & !flag
                };
                if self.post_key(key_code, down, accepted_flags, deadline) {
                    if down {
                        self.active_modifiers.fetch_or(flag, Ordering::Relaxed);
                    } else {
                        self.active_modifiers.fetch_and(!flag, Ordering::Relaxed);
                    }
                } else {
                    return false;
                }
            }
        }
        true
    }

    fn post_pointer_move(&self, position: CGPoint, deadline: Option<Instant>) {
        let pressed_buttons = self.pressed_buttons.load(Ordering::Relaxed);
        if let Ok(cg_event) = CGEvent::new_mouse_event(
            self.source.clone(),
            pointer_event_type(pressed_buttons),
            position,
            pointer_mouse_button(pressed_buttons),
        ) {
            self.post(&cg_event, deadline);
        }
    }

    fn post_mouse_button(&self, button: u8, pressed: bool, deadline: Option<Instant>) {
        self.post_mouse_button_at(button, pressed, self.pointer_position(), deadline);
    }

    fn post_mouse_button_at(&self, button: u8, pressed: bool, position: CGPoint, deadline: Option<Instant>) {
        if !pressed
            && (button > 2 || self.pressed_buttons.load(Ordering::Relaxed) & (1 << button) == 0)
        {
            return;
        }
        let (Some(cg_button), Some(event_type)) =
            (mouse_button(button), mouse_event_type(button, pressed))
        else {
            return;
        };
        let Ok(cg_event) = CGEvent::new_mouse_event(
            self.source.clone(),
            event_type,
            position,
            cg_button,
        ) else {
            return;
        };
        cg_event.set_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER, button as i64);
        cg_event.set_flags(modifier_event_flags(self.current_modifiers()));
        let mask = 1_u8 << button;
        if (self.pressed_buttons.load(Ordering::Relaxed) & mask != 0) == pressed {
            return;
        }
        if self.post(&cg_event, deadline) {
            if pressed {
                self.pressed_buttons.fetch_or(mask, Ordering::Relaxed);
            } else {
                self.pressed_buttons.fetch_and(!mask, Ordering::Relaxed);
            }
        }
    }
}

fn require_post_event_access(allowed: bool) -> Result<(), Box<dyn Error + Send + Sync>> {
    if allowed {
        Ok(())
    } else {
        Err("The running signed Weavelo host lacks input-posting permission. Verify its App identity and enable Weavelo in System Settings > Privacy & Security > Accessibility".into())
    }
}

fn display_bounds(
    id: u32,
) -> Result<core_graphics::geometry::CGRect, Box<dyn Error + Send + Sync>> {
    let display = CGDisplay::new(id);
    validate_display_bounds(display.bounds(), display.is_online(), display.is_active())
}

pub(crate) fn display_input_status(id: u32) -> Result<(), Box<dyn Error + Send + Sync>> {
    display_bounds(id).map(|_| ())
}

pub(crate) fn post_event_access_status() -> Result<(), Box<dyn Error + Send + Sync>> {
    require_post_event_access(unsafe { CGPreflightPostEventAccess() })
}

/// Geometry capability only. Permission is checked by the constructor and
/// every injected event so an OS denial uses the existing INPUT_UNAVAILABLE
/// error path instead of being mistaken for an unimplemented display target.
pub(crate) fn display_input_capable(id: u32) -> bool {
    display_bounds(id).is_ok()
}

fn validate_display_bounds(
    bounds: core_graphics::geometry::CGRect,
    online: bool,
    active: bool,
) -> Result<core_graphics::geometry::CGRect, Box<dyn Error + Send + Sync>> {
    if !online || !active {
        return Err(
            "Selected display is no longer active. Reselect an available source on the host".into(),
        );
    }
    if ![
        bounds.origin.x,
        bounds.origin.y,
        bounds.size.width,
        bounds.size.height,
    ]
    .iter()
    .all(|n| n.is_finite())
        || bounds.size.width < 1.
        || bounds.size.height < 1.
    {
        return Err(
            "Selected display geometry is unavailable. Reselect its source before enabling control"
                .into(),
        );
    }
    Ok(bounds)
}

fn clamp_display_pointer(point: CGPoint, bounds: core_graphics::geometry::CGRect) -> CGPoint {
    CGPoint::new(
        point
            .x
            .clamp(bounds.origin.x, bounds.origin.x + bounds.size.width - 0.5),
        point
            .y
            .clamp(bounds.origin.y, bounds.origin.y + bounds.size.height - 0.5),
    )
}

fn pointer_coordinates(point: CGPoint, bounds: core_graphics::geometry::CGRect) -> (u16, u16) {
    let point = clamp_display_pointer(point, bounds);
    (
        (((point.x - bounds.origin.x) / bounds.size.width) * f64::from(u16::MAX)).round() as u16,
        (((point.y - bounds.origin.y) / bounds.size.height) * f64::from(u16::MAX)).round() as u16,
    )
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightPostEventAccess() -> bool;
    fn CGEventSetLocation(event:core_graphics::sys::CGEventRef,location:CGPoint);
}

fn current_pointer_location(source: &CGEventSource) -> CGPoint {
    CGEvent::new(source.clone())
        .map(|event| event.location())
        .unwrap_or_else(|_| CGPoint::new(0.0, 0.0))
}

pub(crate) fn normalized_pointer_location(
    x: u16,
    y: u16,
    display_bounds: core_graphics::geometry::CGRect,
) -> CGPoint {
    let normalized_x = f64::from(x) / f64::from(u16::MAX);
    let normalized_y = f64::from(y) / f64::from(u16::MAX);
    CGPoint::new(
        display_bounds.origin.x
            + (display_bounds.size.width * normalized_x)
                .min((display_bounds.size.width - 0.5).max(0.)),
        display_bounds.origin.y
            + (display_bounds.size.height * normalized_y)
                .min((display_bounds.size.height - 0.5).max(0.)),
    )
}

fn modifier_key_codes() -> [(u8, u16); 6] {
    use protocol::input_modifiers;
    [
        (input_modifiers::SHIFT, 0x38),
        (input_modifiers::CONTROL, 0x3b),
        (input_modifiers::ALT, 0x3a),
        (input_modifiers::META, 0x37),
        (input_modifiers::FUNCTION, 0x3f),
        (input_modifiers::CAPS_LOCK, 0x39),
    ]
}

fn modifier_event_flags(modifiers: u8) -> CGEventFlags {
    use protocol::input_modifiers;
    let mut flags = CGEventFlags::empty();
    if modifiers & input_modifiers::SHIFT != 0 {
        flags.insert(CGEventFlags::CGEventFlagShift);
    }
    if modifiers & input_modifiers::CONTROL != 0 {
        flags.insert(CGEventFlags::CGEventFlagControl);
    }
    if modifiers & input_modifiers::ALT != 0 {
        flags.insert(CGEventFlags::CGEventFlagAlternate);
    }
    if modifiers & input_modifiers::META != 0 {
        flags.insert(CGEventFlags::CGEventFlagCommand);
    }
    if modifiers & input_modifiers::FUNCTION != 0 {
        flags.insert(CGEventFlags::CGEventFlagSecondaryFn);
    }
    if modifiers & input_modifiers::CAPS_LOCK != 0 {
        flags.insert(CGEventFlags::CGEventFlagAlphaShift);
    }
    flags
}

fn mouse_button(button: u8) -> Option<CGMouseButton> {
    match button {
        0 => Some(CGMouseButton::Left),
        1 => Some(CGMouseButton::Right),
        2 => Some(CGMouseButton::Center),
        _ => None,
    }
}

fn mouse_event_type(button: u8, pressed: bool) -> Option<CGEventType> {
    match (button, pressed) {
        (0, true) => Some(CGEventType::LeftMouseDown),
        (0, false) => Some(CGEventType::LeftMouseUp),
        (1, true) => Some(CGEventType::RightMouseDown),
        (1, false) => Some(CGEventType::RightMouseUp),
        (2, true) => Some(CGEventType::OtherMouseDown),
        (2, false) => Some(CGEventType::OtherMouseUp),
        _ => None,
    }
}

fn pointer_event_type(pressed_buttons: u8) -> CGEventType {
    if pressed_buttons & (1 << 0) != 0 {
        CGEventType::LeftMouseDragged
    } else if pressed_buttons & (1 << 1) != 0 {
        CGEventType::RightMouseDragged
    } else if pressed_buttons & (1 << 2) != 0 {
        CGEventType::OtherMouseDragged
    } else {
        CGEventType::MouseMoved
    }
}

fn pointer_mouse_button(pressed_buttons: u8) -> CGMouseButton {
    if pressed_buttons & (1 << 1) != 0 {
        CGMouseButton::Right
    } else if pressed_buttons & (1 << 2) != 0 {
        CGMouseButton::Center
    } else {
        CGMouseButton::Left
    }
}

fn unicode_event_chunks(text:&str)->impl Iterator<Item=&str> {
    let mut remaining=text;
    std::iter::from_fn(move || {
        if remaining.is_empty() {return None;}
        let mut units=0;
        let end=remaining.char_indices().find_map(|(i,ch)| {
            units+=ch.len_utf16(); (units>20).then_some(i)
        }).unwrap_or(remaining.len());
        let (chunk,rest)=remaining.split_at(end);remaining=rest;Some(chunk)
    })
}

impl InputInjector for MacInputInjector {
    fn focused_editor_probe(&self) -> Option<Box<dyn FnOnce() -> bool + Send>> {
        if self.target_pointer.lock().ok()?.is_none() { return None; }
        let point = self.pointer_position();
        let target = self.window.as_ref().map(|window| (window.id,window.pid));
        Some(Box::new(move || {
            for attempt in 0..3 {
                if crate::window_input::focused_editor_at(point,target) { return true; }
                if attempt < 2 { std::thread::sleep(std::time::Duration::from_millis(20)); }
            }
            false
        }))
    }
    fn inject_input(&self, event: InputEvent) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Consent can be revoked after the injector was created. Propagate a
        // real error through INPUT_UNAVAILABLE before changing held state or
        // posting events; CGEvent::post itself has no success return value.
        require_post_event_access(unsafe { CGPreflightPostEventAccess() })?;
        let scroll_position = match self.scroll_continuation(&event) {
            WindowScrollContinuation::Ignore => return Ok(()),
            WindowScrollContinuation::Validated(position) => Some(position),
            WindowScrollContinuation::Other => {
                self.prepare_window_event(&event, None)?;
                None
            }
        };
        if let InputEvent::Text(text) = &event { return self.commit_text(text, None); }
        self.inject_with_scroll_position(event, None, scroll_position);
        Ok(())
    }

    fn inject_input_before(
        &self,
        event: InputEvent,
        deadline: std::time::Instant,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        if Instant::now() >= deadline {
            return Err(remote_core::InputExpired("before target validation").into());
        }
        let started = Instant::now();
        require_post_event_access(unsafe { CGPreflightPostEventAccess() })?;
        let access_checked = Instant::now();
        let scroll_position = match self.scroll_continuation(&event) {
            WindowScrollContinuation::Ignore => return Ok(()),
            WindowScrollContinuation::Validated(position) => Some(position),
            WindowScrollContinuation::Other => {
                self.prepare_window_event(&event, Some(deadline))?;
                None
            }
        };
        // AX focus/identity validation may have waited; check after it as well.
        if std::time::Instant::now() >= deadline {
            eprintln!("MACOS_INPUT_EXPIRED phase=target_validation access_us={} target_us={} budget_us={}",
                access_checked.duration_since(started).as_micros(), access_checked.elapsed().as_micros(),
                deadline.saturating_duration_since(started).as_micros());
            return Err(remote_core::InputExpired("after target validation").into());
        }
        if let InputEvent::Text(text) = &event { return self.commit_text(text, Some(deadline)); }
        self.inject_with_scroll_position(event, Some(deadline), scroll_position);
        if Instant::now() >= deadline {
            return Err(remote_core::InputExpired("during native dispatch").into());
        }
        Ok(())
    }
    fn release_all_input(&self) {
        self.release_all_input();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_revoked_post_access_is_an_error_not_injection_success() {
        assert!(require_post_event_access(true).is_ok());
        let error = require_post_event_access(false).unwrap_err().to_string();
        assert!(error.contains("running signed Weavelo host"));
        assert!(error.contains("Accessibility"));
    }

    #[test]
    fn protocol_buttons_map_to_core_graphics_buttons_and_event_types() {
        assert_eq!(mouse_button(0).map(|button| button as u32), Some(0));
        assert_eq!(mouse_button(1).map(|button| button as u32), Some(1));
        assert_eq!(mouse_button(2).map(|button| button as u32), Some(2));
        assert!(mouse_button(3).is_none());

        assert_eq!(mouse_event_type(0, true).map(|kind| kind as u32), Some(1));
        assert_eq!(mouse_event_type(0, false).map(|kind| kind as u32), Some(2));
        assert_eq!(mouse_event_type(1, true).map(|kind| kind as u32), Some(3));
        assert_eq!(mouse_event_type(1, false).map(|kind| kind as u32), Some(4));
        assert_eq!(mouse_event_type(2, true).map(|kind| kind as u32), Some(25));
        assert_eq!(mouse_event_type(2, false).map(|kind| kind as u32), Some(26));
    }

    #[test]
    fn unicode_event_batches_preserve_cjk_surrogates_and_combining_text() {
        let text="中文👨‍👩‍👧‍👦é🙂".repeat(20);
        let chunks:Vec<_>=unicode_event_chunks(&text).collect();
        assert_eq!(chunks.concat(),text);
        assert!(chunks.iter().all(|chunk|!chunk.is_empty()&&chunk.encode_utf16().count()<=20));
    }

    #[test]
    fn drag_event_type_follows_the_pressed_button() {
        assert_eq!(pointer_event_type(0) as u32, 5);
        assert_eq!(pointer_event_type(1 << 0) as u32, 6);
        assert_eq!(pointer_event_type(1 << 1) as u32, 7);
        assert_eq!(pointer_event_type(1 << 2) as u32, 27);
        assert_eq!(pointer_mouse_button(0) as u32, 0);
        assert_eq!(pointer_mouse_button(1 << 0) as u32, 0);
        assert_eq!(pointer_mouse_button(1 << 1) as u32, 1);
        assert_eq!(pointer_mouse_button(1 << 2) as u32, 2);
    }

    #[test]
    fn normalized_pointer_coordinates_cover_the_display_bounds() {
        let bounds = core_graphics::geometry::CGRect::new(
            &CGPoint::new(100.0, 200.0),
            &core_graphics::geometry::CGSize::new(1600.0, 900.0),
        );

        let top_left = normalized_pointer_location(0, 0, bounds);
        assert_eq!((top_left.x, top_left.y), (100.0, 200.0));

        let bottom_right = normalized_pointer_location(u16::MAX, u16::MAX, bounds);
        assert_eq!((bottom_right.x, bottom_right.y), (1699.5, 1099.5));
    }

    #[test]
    fn selected_display_mapping_retains_negative_origins_and_logical_dimensions() {
        use core_graphics::geometry::{CGRect, CGSize};
        // A Retina source is normalized before transmission; injection uses
        // Quartz logical coordinates, not the encoded pixel dimensions.
        for bounds in [
            CGRect::new(&CGPoint::new(-1728., 0.), &CGSize::new(1728., 1117.)),
            CGRect::new(&CGPoint::new(0., -1080.), &CGSize::new(1920., 1080.)),
            CGRect::new(&CGPoint::new(1920., 120.), &CGSize::new(1080., 1920.)),
        ] {
            validate_display_bounds(bounds, true, true).unwrap();
            let first = normalized_pointer_location(0, 0, bounds);
            let last = normalized_pointer_location(u16::MAX, u16::MAX, bounds);
            assert_eq!((first.x, first.y), (bounds.origin.x, bounds.origin.y));
            assert_eq!(
                (last.x, last.y),
                (
                    bounds.origin.x + bounds.size.width - 0.5,
                    bounds.origin.y + bounds.size.height - 0.5
                )
            );
            let clamped = clamp_display_pointer(CGPoint::new(-10000., 10000.), bounds);
            assert_eq!(
                (clamped.x, clamped.y),
                (bounds.origin.x, bounds.origin.y + bounds.size.height - 0.5)
            );
        }
    }

    #[test]
    fn inactive_or_invalid_display_never_falls_back_to_main_display() {
        use core_graphics::geometry::{CGRect, CGSize};
        let valid = CGRect::new(&CGPoint::new(-1920., 0.), &CGSize::new(1920., 1080.));
        assert!(validate_display_bounds(valid, false, true).is_err());
        assert!(validate_display_bounds(valid, true, false).is_err());
        for size in [
            CGSize::new(0., 1080.),
            CGSize::new(f64::NAN, 900.),
            CGSize::new(900., f64::INFINITY),
        ] {
            assert!(
                validate_display_bounds(CGRect::new(&CGPoint::new(0., 0.), &size), true, true)
                    .is_err()
            );
        }
    }
}

#[cfg(test)]
mod deadline_tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    fn fixture(until: Option<Instant>) -> (Arc<MacInputInjector>, Arc<Mutex<Vec<(u32, i64)>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let sink = calls.clone();
        let input = MacInputInjector {
            source: CGEventSource::new(CGEventSourceStateID::Private).unwrap(),
            display_id: None,
            window: None,
            target_pointer: Mutex::new(None),
            pressed_buttons: AtomicU8::new(0),
            active_modifiers: AtomicU8::new(0),
            pressed_keys: Mutex::new(BTreeSet::new()),
            scroll_gesture: Mutex::new(protocol::ScrollGesture::default()),
            post_observer: Some(Arc::new(move |event| {
                let mut calls = sink.lock().unwrap();
                calls.push((
                    event.get_type() as u32,
                    event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE),
                ));
                if calls.len() == 1 {
                    if let Some(until) = until {
                        std::thread::sleep(until.saturating_duration_since(Instant::now()));
                    }
                }
            })),
        };
        (Arc::new(input), calls)
    }
    #[test]
    fn window_scroll_continuations_use_one_live_hit_and_ignore_invalid_targets() {
        use protocol::{MomentumPhase as M, ScrollGesture, ScrollPhase as P};
        let changed = InputEvent::MouseScroll { delta_x: 0, delta_y: 5,
            gesture: ScrollGesture { phase: P::Changed, momentum: M::None } };
        let expected = CGPoint::new(-13., 42.);
        let calls = std::cell::Cell::new(0);
        let validate = |x, y| {
            calls.set(calls.get() + 1);
            assert_eq!((x, y), (123, 456));
            Ok(expected)
        };
        assert!(matches!(window_scroll_continuation(&changed, Some((123, 456)), validate),
            WindowScrollContinuation::Validated(p) if (p.x, p.y) == (expected.x, expected.y)));
        assert_eq!(calls.get(), 1);
        assert!(matches!(window_scroll_continuation(&changed, Some((123, 456)), |_, _| Err("covered".into())),
            WindowScrollContinuation::Ignore));
        assert!(matches!(window_scroll_continuation(&changed, None, |_, _| panic!("missing pointer must not be checked")),
            WindowScrollContinuation::Ignore));
        let began = InputEvent::MouseScroll { delta_x: 0, delta_y: 0,
            gesture: ScrollGesture { phase: P::Began, momentum: M::None } };
        assert!(matches!(window_scroll_continuation(&began, Some((123, 456)), |_, _| panic!("new gesture takes the activation path")),
            WindowScrollContinuation::Other));
    }
    #[test]
    fn validated_scroll_uses_the_live_hit_test_position() {
        let (mut input, _) = fixture(None);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        Arc::get_mut(&mut input).unwrap().post_observer = Some(Arc::new(move |event| {
            let p = event.location();
            sink.lock().unwrap().push((p.x, p.y));
        }));
        let expected = CGPoint::new(-13., 42.);
        input.inject_with_scroll_position(InputEvent::scroll(0, 5), None, Some(expected));
        assert_eq!(*seen.lock().unwrap(), vec![(-13., 42.)]);
        input.inject_with_scroll_position(InputEvent::scroll(0, 5), Some(Instant::now()), Some(expected));
        assert_eq!(*seen.lock().unwrap(), vec![(-13., 42.)]);
    }
    #[test]
    fn cancellation_posts_at_the_validated_position_and_honors_deadline() {
        use protocol::{MomentumPhase as M, ScrollGesture, ScrollPhase as P};
        let (mut input, _) = fixture(None);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        Arc::get_mut(&mut input).unwrap().post_observer = Some(Arc::new(move |event| {
            let p = event.location();
            sink.lock().unwrap().push((p.x, p.y, event.get_integer_value_field(SCROLL_PHASE_FIELD)));
        }));
        *input.scroll_gesture.lock().unwrap() = ScrollGesture { phase: P::Changed, momentum: M::None };
        let calls = std::cell::Cell::new(0);
        input.cancel_scroll_with(None, || {
            calls.set(calls.get() + 1);
            Ok(Some(CGPoint::new(-13., 42.)))
        });
        assert_eq!(calls.get(), 1);
        assert_eq!(*seen.lock().unwrap(), vec![(-13., 42., 8)]);
        *input.scroll_gesture.lock().unwrap() = ScrollGesture { phase: P::Changed, momentum: M::None };
        input.cancel_scroll_with(Some(Instant::now()), || Ok(Some(CGPoint::new(17., 24.))));
        assert_eq!(*seen.lock().unwrap(), vec![(-13., 42., 8)]);
        assert!(input.scroll_gesture.lock().unwrap().cancellation().is_none());
        *input.scroll_gesture.lock().unwrap() = ScrollGesture { phase: P::Changed, momentum: M::None };
        input.cancel_scroll_with(None, || Err(()));
        assert_eq!(*seen.lock().unwrap(), vec![(-13., 42., 8)]);
        assert!(input.scroll_gesture.lock().unwrap().cancellation().is_none());
    }
    #[test]
    fn native_scroll_uses_source_coordinates_before_any_cursor_motion_is_posted() {
        let (mut input,_)=fixture(None);
        let id=CGDisplay::main().id;
        let bounds=display_bounds(id).unwrap();
        let point=(u16::MAX/3,u16::MAX/4);
        let expected=normalized_pointer_location(point.0,point.1,bounds);
        let seen=Arc::new(Mutex::new(Vec::new()));
        let sink=seen.clone();
        let native=Arc::get_mut(&mut input).unwrap();
        native.display_id=Some(id);
        *native.target_pointer.lock().unwrap()=Some(point);
        native.post_observer=Some(Arc::new(move|event| {
            let position=event.location();
            sink.lock().unwrap().push((event.get_type() as u32,position.x,position.y));
        }));
        // The observer records the real CGEvent and prevents every OS post.
        input.inject(InputEvent::scroll(7,42),None);
        assert_eq!(*seen.lock().unwrap(),vec![(CGEventType::ScrollWheel as u32,expected.x,expected.y)]);
    }
    #[test]
    fn release_closes_native_touch_and_momentum_once() {
        use protocol::{ScrollGesture, ScrollPhase as P, MomentumPhase as M};
        for (gesture, expected) in [
            (ScrollGesture { phase:P::Changed, momentum:M::None }, vec![(2,0),(8,0)]),
            (ScrollGesture { phase:P::None, momentum:M::Changed }, vec![(0,2),(0,3)]),
        ] {
            let (mut input, _) = fixture(None);
            let seen = Arc::new(Mutex::new(Vec::new()));
            let sink = seen.clone();
            Arc::get_mut(&mut input).unwrap().post_observer = Some(Arc::new(move |event| {
                if event.get_type() as u32 == CGEventType::ScrollWheel as u32 {
                    sink.lock().unwrap().push((event.get_integer_value_field(SCROLL_PHASE_FIELD),
                        event.get_integer_value_field(MOMENTUM_PHASE_FIELD)));
                }
            }));
            input.inject(InputEvent::MouseScroll { delta_x:0, delta_y:8, gesture }, None);
            input.release_all_input();
            input.release_all_input();
            assert_eq!(*seen.lock().unwrap(), expected);
        }
    }
    #[test]
    fn native_scroll_preserves_gesture_and_momentum_including_zero_end() {
        use protocol::{ScrollGesture, ScrollPhase, MomentumPhase};
        let (mut input, _) = fixture(None);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        Arc::get_mut(&mut input).unwrap().post_observer = Some(Arc::new(move |event| {
            sink.lock().unwrap().push((
                event.get_integer_value_field(SCROLL_PHASE_FIELD),
                event.get_integer_value_field(MOMENTUM_PHASE_FIELD),
            ));
        }));
        for (phase, momentum) in [
            (ScrollPhase::MayBegin, MomentumPhase::None),
            (ScrollPhase::Began, MomentumPhase::None),
            (ScrollPhase::Changed, MomentumPhase::None),
            (ScrollPhase::Ended, MomentumPhase::None),
            (ScrollPhase::None, MomentumPhase::Began),
            (ScrollPhase::None, MomentumPhase::Changed),
            (ScrollPhase::None, MomentumPhase::Ended),
            (ScrollPhase::Cancelled, MomentumPhase::None),
        ] {
            input.inject(InputEvent::MouseScroll {
                delta_x: 0, delta_y: 0, gesture: ScrollGesture { phase, momentum },
            }, None);
        }
        assert_eq!(*seen.lock().unwrap(), vec![(128,0),(1,0),(2,0),(4,0),(0,1),(0,2),(0,3),(8,0)]);
    }
    #[test]
    fn fresh_deadline_posts_and_cleans_only_the_accepted_key() {
        let (input, calls) = fixture(None);
        // First CoreGraphics event creation may initialize native services.
        // The positive case tests accepted ownership, not initialization speed.
        CGEvent::new_keyboard_event(input.source.clone(), 0, true)
            .expect("initialize keyboard event fixture");
        input.inject(
            InputEvent::KeyDown(0),
            Some(Instant::now() + std::time::Duration::from_secs(1)),
        );
        input.release_all_input();
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                (CGEventType::KeyDown as u32, 0),
                (CGEventType::KeyUp as u32, 0)
            ]
        );
    }
    #[test]
    fn key_lock_wait_expiry_does_not_post_or_create_cleanup_up() {
        let (input, calls) = fixture(None);
        let held = input.pressed_keys.lock().unwrap();
        let until = Instant::now() + std::time::Duration::from_millis(40);
        let (tx, rx) = std::sync::mpsc::channel();
        let other = input.clone();
        let worker = std::thread::spawn(move || {
            tx.send(()).unwrap();
            other.inject(InputEvent::KeyDown(0), Some(until));
        });
        rx.recv().unwrap();
        std::thread::sleep(
            until.saturating_duration_since(Instant::now()) + std::time::Duration::from_millis(2),
        );
        drop(held);
        worker.join().unwrap();
        input.release_all_input();
        assert!(calls.lock().unwrap().is_empty());
    }
    #[test]
    fn batch_expiry_does_not_post_unaccepted_modifiers_or_key_cleanup() {
        let (mut input, calls) = fixture(None);
        // Finish the native fixture setup before starting the event budget.
        let _prepared = CGEvent::new_keyboard_event(input.source.clone(), 56, true).unwrap();
        let until = Instant::now() + std::time::Duration::from_millis(40);
        let sink = calls.clone();
        Arc::get_mut(&mut input).unwrap().post_observer = Some(Arc::new(move |event| {
            let mut calls = sink.lock().unwrap();
            calls.push((
                event.get_type() as u32,
                event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE),
            ));
            if calls.len() == 1 {
                std::thread::sleep(
                    until.saturating_duration_since(Instant::now())
                        + std::time::Duration::from_millis(2),
                );
            }
        }));
        input.inject(
            InputEvent::Key {
                key_code: 0,
                pressed: true,
                modifiers: protocol::input_modifiers::SHIFT | protocol::input_modifiers::CONTROL,
            },
            Some(until),
        );
        assert_eq!(
            *calls.lock().unwrap(),
            vec![(CGEventType::FlagsChanged as u32, 56)]
        );
        input.release_all_input();
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                (CGEventType::FlagsChanged as u32, 56),
                (CGEventType::FlagsChanged as u32, 56)
            ]
        );
    }
}
