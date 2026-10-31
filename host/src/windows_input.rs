use crate::windows_keymap::mac_key_to_vk;
use super::windows_sources as native_sources;
use protocol::{InputEvent, input_modifiers};
use remote_core::InputInjector;
use std::collections::BTreeSet;
use std::error::Error;
use std::sync::Mutex;
use std::time::Instant;
type InputResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;
fn check_deadline(deadline: Option<Instant>) -> InputResult {
    if deadline.is_some_and(|d| Instant::now() >= d) {
        return Err("input expired before SendInput".into());
    }
    Ok(())
}
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT, SendInput, VIRTUAL_KEY,
};

pub struct WindowsInputInjector {
    pressed: AtomicU8,
    cleanup_unconfirmed: AtomicBool,
    modifiers: AtomicU8,
    pressed_keys: Mutex<BTreeSet<VIRTUAL_KEY>>,
    unicode_up: Mutex<Option<u16>>,
    source: Option<(native_sources::Selection,(u32,u32))>,
}

impl WindowsInputInjector {
    pub fn new() -> Result<Self, Box<dyn Error + Send + Sync>> {
        let selection=native_sources::resolve(protocol::session::CaptureSource::MainDisplay)?;
        let bounds=selection.bounds;
        Self::for_source(selection.source,(bounds.width,bounds.height),(bounds.width,bounds.height))
    }
    pub fn for_source(source:protocol::session::CaptureSource,canvas:(u32,u32),expected:(u32,u32))->InputResult<Self> {
        let selection=native_sources::resolve(source)?;
        let bounds=selection.current_bounds()?;
        if (bounds.width,bounds.height)!=expected{return Err("Source geometry changed before control was enabled".into());}
        Ok(Self {
            pressed: AtomicU8::new(0),
            cleanup_unconfirmed: AtomicBool::new(false),
            modifiers: AtomicU8::new(0),
            pressed_keys: Mutex::new(BTreeSet::new()),
            unicode_up: Mutex::new(None),
            source:Some((selection,canvas)),
        })
    }

    fn prepare_source(&self,event:InputEvent,deadline:Option<Instant>)->InputResult<InputEvent> {
        use windows_sys::Win32::{Foundation::{HWND,POINT},UI::WindowsAndMessaging::*};
        let Some((source,canvas))=&self.source else {return Ok(event);};
        let bounds=source.same_size()?;
        let desktop=crate::windows_capture_plan::Rect{x:unsafe{GetSystemMetrics(SM_XVIRTUALSCREEN)},y:unsafe{GetSystemMetrics(SM_YVIRTUALSCREEN)},
            width:unsafe{GetSystemMetrics(SM_CXVIRTUALSCREEN)}.try_into()?,height:unsafe{GetSystemMetrics(SM_CYVIRTUALSCREEN)}.try_into()?};
        if source.window {
            let hwnd=source.handle as usize as HWND;
            let pointer=matches!(&event,InputEvent::MouseMoveAbsolute{..}|InputEvent::MouseDown(_)|InputEvent::MouseUp(_));
            if unsafe{GetAncestor(GetForegroundWindow(),GA_ROOT)}!=hwnd {
                if !pointer{return Err("The selected window is not focused".into());}
                check_deadline(deadline)?;
                // Windows may reject background activation. A hit-tested native
                // click can still activate this window; motion must not revoke
                // control. Keyboard and wheel input still require actual focus.
                unsafe{SetForegroundWindow(hwnd)};
                // Raise only the selected window without moving, resizing or
                // changing its topmost policy. The hit test below remains final.
                unsafe{SetWindowPos(hwnd,HWND_TOP,0,0,0,0,SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE)};
            }
            if let InputEvent::MouseMoveAbsolute{x,y}=&event {
                let (vx,vy)=crate::windows_capture_plan::absolute(bounds,desktop,*canvas,*x,*y)?;
                let point=POINT{x:desktop.x+(f64::from(vx)/65535.0*f64::from(desktop.width-1)).round() as i32,
                    y:desktop.y+(f64::from(vy)/65535.0*f64::from(desktop.height-1)).round() as i32};
                if unsafe{GetAncestor(WindowFromPoint(point),GA_ROOT)}!=hwnd {return Err("The selected window is covered at this pointer position".into());}
            } else if matches!(&event,InputEvent::MouseDown(_)|InputEvent::MouseUp(_)|InputEvent::MouseScroll{..}) {
                let mut point=POINT{x:0,y:0};
                if unsafe{GetCursorPos(&mut point)}==0 || unsafe{GetAncestor(WindowFromPoint(point),GA_ROOT)}!=hwnd {
                    return Err("Pointer is no longer inside the selected window".into());
                }
            } else if matches!(&event,InputEvent::MouseMove{..}) {
                return Err("Window control requires a position in the visible content".into());
            }
        }
        check_deadline(deadline)?;
        match event {
            InputEvent::MouseMoveAbsolute{x,y}=>{
                let (x,y)=crate::windows_capture_plan::absolute(bounds,desktop,*canvas,x,y)?;
                Ok(InputEvent::MouseMoveAbsolute{x:x.try_into()?,y:y.try_into()?})
            }
            event=>Ok(event),
        }
    }

    fn release_unicode(&self) -> bool {
        let mut pending=self.unicode_up.lock().unwrap();
        let Some(unit)=*pending else {return true;};
        let mut input=INPUT {r#type:INPUT_KEYBOARD,Anonymous:INPUT_0 {ki:KEYBDINPUT {
            wVk:0,wScan:unit,dwFlags:KEYEVENTF_UNICODE|KEYEVENTF_KEYUP,time:0,dwExtraInfo:0}}};
        if unsafe {SendInput(1,&mut input,std::mem::size_of::<INPUT>() as i32)}==1 {*pending=None;true} else {false}
    }

    fn post_key(&self, vk: VIRTUAL_KEY, pressed: bool, deadline: Option<Instant>) -> InputResult {
        let mut keys = self.pressed_keys.lock().unwrap();
        check_deadline(deadline)?;
        if keys.contains(&vk) == pressed {
            return Ok(());
        }
        // This lock excludes cleanup until the final call has a known result.
        if !send_key(vk, pressed, deadline)? {
            return Err("Windows did not accept this input; no retry was issued".into());
        }
        if pressed {
            keys.insert(vk);
        } else {
            keys.remove(&vk);
        }
        Ok(())
    }

    fn set_modifiers(&self, modifiers: u8, deadline: Option<Instant>) -> InputResult {
        for (flag, vk) in [
            (input_modifiers::SHIFT, 0xA0),
            (input_modifiers::CONTROL, 0xA2),
            (input_modifiers::ALT, 0xA4),
            (input_modifiers::META, 0x5B),
        ] {
            if (self.modifiers.load(Ordering::Relaxed) ^ modifiers) & flag != 0 {
                let down = modifiers & flag != 0;
                self.post_key(vk, down, deadline)?;
                if down {
                    self.modifiers.fetch_or(flag, Ordering::Relaxed);
                } else {
                    self.modifiers.fetch_and(!flag, Ordering::Relaxed);
                }
            }
        }
        Ok(())
    }
}

fn send_mouse(
    flags: u32,
    dx: i32,
    dy: i32,
    data: u32,
    deadline: Option<Instant>,
) -> InputResult<bool> {
    let mut input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    check_deadline(deadline)?;
    Ok(unsafe { SendInput(1, &mut input, std::mem::size_of::<INPUT>() as i32) == 1 })
}

fn send_key(vk: VIRTUAL_KEY, pressed: bool, deadline: Option<Instant>) -> InputResult<bool> {
    let extended = matches!(vk, 0x21..=0x28 | 0x2D | 0x2E | 0x5B | 0x5C | 0xA3 | 0xA5);
    let mut input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: (if pressed { 0 } else { KEYEVENTF_KEYUP })
                    | if extended { KEYEVENTF_EXTENDEDKEY } else { 0 },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    check_deadline(deadline)?;
    Ok(unsafe { SendInput(1, &mut input, std::mem::size_of::<INPUT>() as i32) == 1 })
}

impl WindowsInputInjector {
    fn inject_before(&self, event: InputEvent, deadline: Option<Instant>) -> InputResult {
        check_deadline(deadline)?;
        if self.cleanup_unconfirmed.load(Ordering::Acquire) {
            return Err("Windows input cleanup is unconfirmed; new input is disabled".into());
        }
        let _coordinates=native_sources::PhysicalCoordinates::enter()?;
        let event=match self.prepare_source(event,deadline) {Ok(event)=>event,Err(error)=>{self.release_all_input();return Err(error);}};
        match event {
            InputEvent::Text(text) => {
                if self.modifiers.load(Ordering::Relaxed) & 0x0f != 0 {
                    return Err("release remote modifiers before committing text".into());
                }
                // SendInput inserts this UTF-16 array in order; the zero VK
                // selects VK_PACKET rather than the machine's keyboard layout.
                let mut inputs: Vec<INPUT> = text.as_str().encode_utf16().flat_map(|unit| {
                    [0, KEYEVENTF_KEYUP].map(|up| INPUT { r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 {ki: KEYBDINPUT {wVk:0,wScan:unit,
                            dwFlags:KEYEVENTF_UNICODE|up,time:0,dwExtraInfo:0}}})
                }).collect();
                check_deadline(deadline)?;
                let accepted=unsafe {SendInput(inputs.len() as u32,inputs.as_mut_ptr(),std::mem::size_of::<INPUT>() as i32)};
                if accepted != inputs.len() as u32 {
                    // A partial positive prefix may end in our known down.
                    // Release that exact Unicode unit, without replaying text.
                    if accepted % 2 == 1 && (accepted as usize)<inputs.len() {
                        *self.unicode_up.lock().unwrap()=Some(unsafe {inputs[accepted as usize].Anonymous.ki.wScan});
                        self.cleanup_unconfirmed.store(!self.release_unicode(),Ordering::Release);
                    }
                    return Err("Windows did not accept the complete text; it will not be replayed".into());
                }
            }
            InputEvent::MouseMove { dx, dy } => {
                if !send_mouse(MOUSEEVENTF_MOVE, dx, dy, 0, deadline)? {
                    return Err("Windows did not accept pointer motion".into());
                }
            }
            InputEvent::MouseMoveAbsolute { x, y } => {
                // The wire already uses SendInput's 0..65535 absolute range.
                if !send_mouse(
                    MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                    i32::from(x),
                    i32::from(y),
                    0,
                    deadline,
                )? {
                    return Err("Windows did not accept pointer motion".into());
                }
            }
            InputEvent::MouseDown(button) | InputEvent::MouseUp(button) => {
                let down = matches!(event, InputEvent::MouseDown(_));
                let flags = match (button, down) {
                    (0, true) => MOUSEEVENTF_LEFTDOWN,
                    (0, false) => MOUSEEVENTF_LEFTUP,
                    (1, true) => MOUSEEVENTF_RIGHTDOWN,
                    (1, false) => MOUSEEVENTF_RIGHTUP,
                    (2, true) => MOUSEEVENTF_MIDDLEDOWN,
                    (2, false) => MOUSEEVENTF_MIDDLEUP,
                    _ => 0,
                };
                if flags != 0 {
                    let mask = 1 << button;
                    if (self.pressed.load(Ordering::Relaxed) & mask != 0) != down {
                        if !send_mouse(flags, 0, 0, 0, deadline)? {
                            return Err("Windows did not accept this button transition".into());
                        }
                        if down {
                            self.pressed.fetch_or(mask, Ordering::Relaxed);
                        } else {
                            self.pressed.fetch_and(!mask, Ordering::Relaxed);
                        }
                    }
                }
            }
            InputEvent::MouseScroll {
                delta_x: _,
                delta_y,
            } => {
                if !send_mouse(
                    MOUSEEVENTF_WHEEL,
                    0,
                    0,
                    delta_y.saturating_mul(120) as u32,
                    deadline,
                )? {
                    return Err("Windows did not accept scrolling".into());
                }
            }
            InputEvent::Key {
                key_code,
                pressed,
                modifiers,
            } => {
                let modifiers = if pressed {
                    modifiers
                } else {
                    modifiers & self.modifiers.load(Ordering::Relaxed)
                };
                self.set_modifiers(modifiers, deadline)?;
                if let Some(vk) = mac_key_to_vk(key_code) {
                    self.post_key(vk, pressed, deadline)?;
                }
            }
            InputEvent::KeyDown(code) => self.post_key(code as VIRTUAL_KEY, true, deadline)?,
            InputEvent::KeyUp(code) => self.post_key(code as VIRTUAL_KEY, false, deadline)?,
            InputEvent::ModifiersChanged(modifiers) => self.set_modifiers(modifiers, deadline)?,
            InputEvent::Touch { .. } => {}
        }
        Ok(())
    }
}

impl InputInjector for WindowsInputInjector {
    fn inject_input(&self, event: InputEvent) -> InputResult {
        self.inject_before(event, None)
    }
    fn inject_input_before(&self, event: InputEvent, deadline: Instant) -> InputResult {
        self.inject_before(event, Some(deadline))
    }
    fn release_all_input(&self) {
        self.modifiers.store(0, Ordering::Relaxed);
        let keys = std::mem::take(&mut *self.pressed_keys.lock().unwrap());
        let mut unconfirmed = !self.release_unicode();
        for vk in keys {
            if !send_key(vk, false, None).unwrap_or(false) {
                // SendInput reports zero accepted events, unlike an unknown RPC.
                self.pressed_keys.lock().unwrap().insert(vk);
                unconfirmed = true;
            }
        }
        let buttons = self.pressed.swap(0, Ordering::Relaxed);
        for (mask, flag) in [
            (1, MOUSEEVENTF_LEFTUP),
            (2, MOUSEEVENTF_RIGHTUP),
            (4, MOUSEEVENTF_MIDDLEUP),
        ] {
            if buttons & mask != 0 && !send_mouse(flag, 0, 0, 0, None).unwrap_or(false) {
                self.pressed.fetch_or(mask, Ordering::Relaxed);
                unconfirmed = true;
            }
        }
        self.cleanup_unconfirmed
            .store(unconfirmed, Ordering::Release);
    }
}
