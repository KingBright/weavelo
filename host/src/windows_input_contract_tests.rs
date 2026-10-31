//! Compile and exercise the actual Windows injector against a recording API.
//! This checks ownership/failure logic, not Windows ABI, desktop or permissions.
use protocol::{InputEvent, input_modifiers};
use remote_core::InputInjector;
use std::sync::Mutex;

mod actual {
    use super::recording_api as windows_sys;
    include!("windows_input.rs");
    pub(super) fn held_lock(
        input: &WindowsInputInjector,
    ) -> std::sync::MutexGuard<'_, std::collections::BTreeSet<u16>> {
        input.pressed_keys.lock().unwrap()
    }
}
// The native handle/geometry APIs are recorded here; no desktop API is invoked.
mod windows_sources {
    use protocol::session::CaptureSource;
    use crate::windows_capture_plan::Rect;
    #[derive(Clone)] pub struct Selection {pub source:CaptureSource,pub bounds:Rect,pub handle:u64,pub window:bool}
    impl Selection {
        pub fn current_bounds(&self)->Result<Rect,String>{Ok(self.bounds)}
        pub fn same_size(&self)->Result<Rect,String>{Ok(self.bounds)}
    }
    pub fn resolve(source:CaptureSource)->Result<Selection,String>{Ok(Selection{source,bounds:Rect{x:0,y:0,width:1920,height:1080},handle:1,window:matches!(source,CaptureSource::Window(_))})}
    pub struct PhysicalCoordinates;
    impl PhysicalCoordinates {pub fn enter()->Result<Self,String>{Ok(Self)}}
}
#[derive(Debug, PartialEq, Eq)]
enum Posted {
    Mouse(u32),
    Key(u16, bool),
    Unicode(u16,bool),
}
#[derive(Default)]
struct Calls {
    attempts: usize,
    fail_next: bool,
    accept_next: Option<u32>,
    fail_attempt: Option<usize>,
    pause_first_until: Option<std::time::Instant>,
    posted: Vec<Posted>,
    foreground_blocked: bool,
    covered: bool,
    raise_uncovers: bool,
}
static CALLS: Mutex<Calls> = Mutex::new(Calls {
    attempts: 0,
    fail_next: false,
    accept_next: None,
    fail_attempt: None,
    pause_first_until: None,
    posted: Vec::new(),
    foreground_blocked: false,
    covered: false,
    raise_uncovers: false,
});
static SERIAL: Mutex<()> = Mutex::new(());

#[allow(non_snake_case, non_camel_case_types, dead_code)]
mod recording_api {
    pub type VIRTUAL_KEY = u16;
    #[derive(Clone, Copy)]
    pub struct MOUSEINPUT {
        pub dx: i32,
        pub dy: i32,
        pub mouseData: u32,
        pub dwFlags: u32,
        pub time: u32,
        pub dwExtraInfo: usize,
    }
    #[derive(Clone, Copy)]
    pub struct KEYBDINPUT {
        pub wVk: u16,
        pub wScan: u16,
        pub dwFlags: u32,
        pub time: u32,
        pub dwExtraInfo: usize,
    }
    #[derive(Clone, Copy)]
    pub union INPUT_0 {
        pub mi: MOUSEINPUT,
        pub ki: KEYBDINPUT,
    }
    pub struct INPUT {
        pub r#type: u32,
        pub Anonymous: INPUT_0,
    }
    pub const INPUT_MOUSE: u32 = 0;
    pub const INPUT_KEYBOARD: u32 = 1;
    pub const KEYEVENTF_EXTENDEDKEY: u32 = 1;
    pub const KEYEVENTF_KEYUP: u32 = 2;
    pub const KEYEVENTF_UNICODE: u32 = 4;
    pub const MOUSEEVENTF_MOVE: u32 = 1;
    pub const MOUSEEVENTF_LEFTDOWN: u32 = 2;
    pub const MOUSEEVENTF_LEFTUP: u32 = 4;
    pub const MOUSEEVENTF_RIGHTDOWN: u32 = 8;
    pub const MOUSEEVENTF_RIGHTUP: u32 = 16;
    pub const MOUSEEVENTF_MIDDLEDOWN: u32 = 32;
    pub const MOUSEEVENTF_MIDDLEUP: u32 = 64;
    pub const MOUSEEVENTF_WHEEL: u32 = 0x800;
    pub const MOUSEEVENTF_ABSOLUTE: u32 = 0x8000;
    pub const MOUSEEVENTF_VIRTUALDESK:u32=0x4000;
    pub unsafe fn SendInput(count: u32, inputs: *mut INPUT, _size: i32) -> u32 {
        let mut calls = super::CALLS.lock().unwrap();
        calls.attempts += 1;
        if calls.fail_next || calls.fail_attempt==Some(calls.attempts) {
            calls.fail_next = false;
            return 0;
        }
        let accepted=calls.accept_next.take().unwrap_or(count).min(count);
        for i in 0..accepted {
            let input = unsafe { &*inputs.add(i as usize) };
            let posted = if input.r#type == INPUT_MOUSE {
                super::Posted::Mouse(unsafe { input.Anonymous.mi.dwFlags })
            } else {
                let key = unsafe { input.Anonymous.ki };
                if key.dwFlags & KEYEVENTF_UNICODE != 0 {super::Posted::Unicode(key.wScan,key.dwFlags&KEYEVENTF_KEYUP==0)}
                else {super::Posted::Key(key.wVk, key.dwFlags & KEYEVENTF_KEYUP == 0)}
            };
            calls.posted.push(posted);
        }
        if calls.attempts == 1 {
            if let Some(until) = calls.pause_first_until {
                std::thread::sleep(until.saturating_duration_since(std::time::Instant::now()));
            }
        }
        accepted
    }
    pub mod Win32 {
        pub mod Foundation {
            pub type HWND=*mut core::ffi::c_void;
            pub struct POINT {pub x:i32,pub y:i32}
        }
        pub mod UI {
            pub mod WindowsAndMessaging {
                use super::super::Foundation::{HWND,POINT};
                pub const SM_XVIRTUALSCREEN:i32=76;pub const SM_YVIRTUALSCREEN:i32=77;
                pub const SM_CXVIRTUALSCREEN:i32=78;pub const SM_CYVIRTUALSCREEN:i32=79;
                pub const GA_ROOT:u32=2;
                pub const HWND_TOP:HWND=core::ptr::null_mut();
                pub const SWP_NOMOVE:u32=2;pub const SWP_NOSIZE:u32=1;pub const SWP_NOACTIVATE:u32=16;
                pub unsafe fn GetSystemMetrics(index:i32)->i32{match index{78=>1920,79=>1080,_=>0}}
                pub unsafe fn GetForegroundWindow()->HWND {if super::super::super::super::CALLS.lock().unwrap().foreground_blocked {2usize as HWND}else{1usize as HWND}}
                pub unsafe fn GetAncestor(window:HWND,_flags:u32)->HWND {window}
                pub unsafe fn SetForegroundWindow(_window:HWND)->i32{if super::super::super::super::CALLS.lock().unwrap().foreground_blocked {0}else{1}}
                pub unsafe fn SetWindowPos(_window:HWND,_after:HWND,_x:i32,_y:i32,_w:i32,_h:i32,flags:u32)->i32{
                    assert_eq!(flags,SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE);
                    let mut calls=super::super::super::super::CALLS.lock().unwrap();
                    if calls.raise_uncovers{calls.covered=false;}1
                }
                pub unsafe fn WindowFromPoint(_point:POINT)->HWND {if super::super::super::super::CALLS.lock().unwrap().covered {2usize as HWND}else{1usize as HWND}}
                pub unsafe fn GetCursorPos(point:*mut POINT)->i32 {unsafe{(*point).x=0;(*point).y=0;}1}
            }
            pub mod Input {
                pub mod KeyboardAndMouse {
                    pub use super::super::super::super::*;
                }
            }
        }
    }
}

#[test]
fn blocked_background_activation_allows_hit_tested_click_but_never_unfocused_keys() {
    let _serial=SERIAL.lock().unwrap();
    *CALLS.lock().unwrap()=Calls{foreground_blocked:true,..Default::default()};
    let input=actual::WindowsInputInjector::for_source(protocol::session::CaptureSource::Window(77),(1920,1080),(1920,1080)).unwrap();
    input.inject_input(InputEvent::MouseMoveAbsolute{x:32768,y:32768}).unwrap();
    input.inject_input(InputEvent::MouseDown(0)).unwrap();
    input.inject_input(InputEvent::MouseUp(0)).unwrap();
    assert!(input.inject_input(InputEvent::Key{key_code:0,pressed:true,modifiers:0}).is_err());
    assert!(input.inject_input(InputEvent::MouseScroll{delta_x:0,delta_y:1}).is_err());
    assert_eq!(CALLS.lock().unwrap().posted.len(),3);
}

#[test]
fn rejected_activation_never_clicks_a_covering_window() {
    let _serial=SERIAL.lock().unwrap();
    *CALLS.lock().unwrap()=Calls{foreground_blocked:true,covered:true,..Default::default()};
    let input=actual::WindowsInputInjector::for_source(protocol::session::CaptureSource::Window(77),(1920,1080),(1920,1080)).unwrap();
    assert!(input.inject_input(InputEvent::MouseMoveAbsolute{x:32768,y:32768}).is_err());
    assert!(CALLS.lock().unwrap().posted.is_empty());
}

#[test]
fn selected_window_can_be_raised_without_requiring_foreground_activation() {
    let _serial=SERIAL.lock().unwrap();
    *CALLS.lock().unwrap()=Calls{foreground_blocked:true,covered:true,raise_uncovers:true,..Default::default()};
    let input=actual::WindowsInputInjector::for_source(protocol::session::CaptureSource::Window(77),(1920,1080),(1920,1080)).unwrap();
    input.inject_input(InputEvent::MouseMoveAbsolute{x:32768,y:32768}).unwrap();
    input.inject_input(InputEvent::MouseDown(0)).unwrap();
    input.inject_input(InputEvent::MouseUp(0)).unwrap();
    assert_eq!(CALLS.lock().unwrap().posted.len(),3);
}

#[test]
fn cleanup_never_releases_unowned_buttons_or_replays_duplicate_downs() {
    let _serial = SERIAL.lock().unwrap();
    *CALLS.lock().unwrap() = Calls::default();
    let input = actual::WindowsInputInjector::new().unwrap();
    input.inject_input(InputEvent::MouseUp(1)).unwrap();
    input.inject_input(InputEvent::MouseDown(0)).unwrap();
    input.inject_input(InputEvent::MouseDown(0)).unwrap();
    input.release_all_input();
    input.release_all_input();
    let calls = CALLS.lock().unwrap();
    assert_eq!(calls.attempts, 2);
    assert_eq!(calls.posted, vec![Posted::Mouse(2), Posted::Mouse(4)]);
}
#[test]
fn failed_modifier_never_forwards_the_bare_printable_key() {
    let _serial = SERIAL.lock().unwrap();
    *CALLS.lock().unwrap() = Calls {
        fail_next: true,
        ..Default::default()
    };
    let input = actual::WindowsInputInjector::new().unwrap();
    assert!(
        input
            .inject_input(InputEvent::Key {
                key_code: 0,
                pressed: true,
                modifiers: input_modifiers::CONTROL
            })
            .is_err()
    );
    input.release_all_input();
    let calls = CALLS.lock().unwrap();
    assert_eq!(calls.attempts, 1);
    assert!(calls.posted.is_empty());
}
#[test]
fn rejected_cleanup_blocks_new_input_until_known_unaccepted_up_is_cleared() {
    let _serial = SERIAL.lock().unwrap();
    *CALLS.lock().unwrap() = Calls::default();
    let input = actual::WindowsInputInjector::new().unwrap();
    input
        .inject_input(InputEvent::Key {
            key_code: 0,
            pressed: true,
            modifiers: 0,
        })
        .unwrap();
    CALLS.lock().unwrap().fail_next = true;
    input.release_all_input();
    assert!(
        input
            .inject_input(InputEvent::Key {
                key_code: 11,
                pressed: true,
                modifiers: 0
            })
            .is_err()
    );
    assert_eq!(CALLS.lock().unwrap().attempts, 2);
    // Zero accepted events proves the previous up did not execute. This is
    // owner cleanup, distinct from replaying an unknown Portal RPC outcome.
    input.release_all_input();
    input
        .inject_input(InputEvent::Key {
            key_code: 11,
            pressed: true,
            modifiers: 0,
        })
        .unwrap();
    assert_eq!(
        CALLS.lock().unwrap().posted,
        vec![
            Posted::Key(0x41, true),
            Posted::Key(0x41, false),
            Posted::Key(0x42, true)
        ]
    );
}

#[test]
fn fresh_deadline_posts_and_cleans_only_the_accepted_key() {
    let _serial = SERIAL.lock().unwrap();
    *CALLS.lock().unwrap() = Calls::default();
    let input = actual::WindowsInputInjector::new().unwrap();
    input
        .inject_input_before(
            InputEvent::KeyDown(0x41),
            std::time::Instant::now() + std::time::Duration::from_secs(1),
        )
        .unwrap();
    input.release_all_input();
    assert_eq!(
        CALLS.lock().unwrap().posted,
        vec![Posted::Key(0x41, true), Posted::Key(0x41, false)]
    );
}
#[test]
fn key_lock_wait_expiry_does_not_post_or_create_cleanup_up() {
    let _serial = SERIAL.lock().unwrap();
    *CALLS.lock().unwrap() = Calls::default();
    let input = std::sync::Arc::new(actual::WindowsInputInjector::new().unwrap());
    let lock = actual::held_lock(&input);
    let until = std::time::Instant::now() + std::time::Duration::from_millis(40);
    let (tx, rx) = std::sync::mpsc::channel();
    let other = input.clone();
    let worker = std::thread::spawn(move || {
        tx.send(()).unwrap();
        other
            .inject_input_before(InputEvent::KeyDown(0x41), until)
            .is_err()
    });
    rx.recv().unwrap();
    std::thread::sleep(
        until.saturating_duration_since(std::time::Instant::now())
            + std::time::Duration::from_millis(2),
    );
    drop(lock);
    assert!(worker.join().unwrap());
    input.release_all_input();
    assert_eq!(CALLS.lock().unwrap().attempts, 0);
}
#[test]
fn batch_expiry_after_first_modifier_does_not_post_remaining_modifier_or_key() {
    let _serial = SERIAL.lock().unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_millis(40);
    *CALLS.lock().unwrap() = Calls {
        pause_first_until: Some(until + std::time::Duration::from_millis(2)),
        ..Default::default()
    };
    let input = actual::WindowsInputInjector::new().unwrap();
    assert!(
        input
            .inject_input_before(
                InputEvent::Key {
                    key_code: 0,
                    pressed: true,
                    modifiers: input_modifiers::SHIFT | input_modifiers::CONTROL
                },
                until
            )
            .is_err()
    );
    assert_eq!(CALLS.lock().unwrap().posted, vec![Posted::Key(0xA0, true)]);
    input.release_all_input();
    assert_eq!(
        CALLS.lock().unwrap().posted,
        vec![Posted::Key(0xA0, true), Posted::Key(0xA0, false)]
    );
}

#[test]
fn unicode_text_uses_ordered_utf16_and_partial_prefix_only_releases_its_owned_down() {
    let _serial=SERIAL.lock().unwrap();
    *CALLS.lock().unwrap()=Calls::default();
    let input=actual::WindowsInputInjector::new().unwrap();
    let text=protocol::interaction::CommittedText::new("中文🙂".into()).unwrap();
    input.inject_input(InputEvent::Text(text.clone())).unwrap();
    let expected:Vec<_>=text.as_str().encode_utf16().flat_map(|unit|[Posted::Unicode(unit,true),Posted::Unicode(unit,false)]).collect();
    assert_eq!(CALLS.lock().unwrap().posted,expected);
    assert_eq!(CALLS.lock().unwrap().attempts,1);
    *CALLS.lock().unwrap()=Calls {accept_next:Some(1),..Default::default()};
    assert!(input.inject_input(InputEvent::Text(text)).is_err());
    let first="中".encode_utf16().next().unwrap();
    assert_eq!(CALLS.lock().unwrap().posted,vec![Posted::Unicode(first,true),Posted::Unicode(first,false)]);
    assert_eq!(CALLS.lock().unwrap().attempts,2);
}

#[test]
fn unicode_partial_down_survives_a_failed_up_until_owned_cleanup() {
    let _serial=SERIAL.lock().unwrap();
    *CALLS.lock().unwrap()=Calls {accept_next:Some(1),fail_attempt:Some(2),..Default::default()};
    let input=actual::WindowsInputInjector::new().unwrap();
    let text=protocol::interaction::CommittedText::new("🙂".into()).unwrap();
    assert!(input.inject_input(InputEvent::Text(text.clone())).is_err());
    assert!(input.inject_input(InputEvent::Text(text)).is_err());
    assert_eq!(CALLS.lock().unwrap().attempts,2,"unconfirmed cleanup must block new text");
    input.release_all_input();input.release_all_input();
    let unit="🙂".encode_utf16().next().unwrap();
    assert_eq!(CALLS.lock().unwrap().posted,vec![Posted::Unicode(unit,true),Posted::Unicode(unit,false)]);
    assert_eq!(CALLS.lock().unwrap().attempts,3,"cleanup never replays text");
}
