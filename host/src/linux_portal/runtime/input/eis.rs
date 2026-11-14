//! Direct libei (Emulated Input Server) transport for consented RemoteDesktop v2 sessions.
//! Eliminates per-event D-Bus round-trips and preserves high-resolution `v120` + pixel scroll
//! across Wayland compositors (including KDE Plasma 6 KWin `eis.so` and GNOME Mutter).

use super::{OwnedObjectPath, PortalInputError, Result, SelectedPortalSource, options};
use protocol::{MomentumPhase, ScrollGesture, ScrollPhase};
use std::collections::BTreeSet;
use std::ffi::{c_char, c_int, c_uint, c_void};
use std::os::fd::IntoRawFd;
use std::ptr::null_mut;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[repr(C)]
struct Ei(u8);
#[repr(C)]
struct EiEvent(u8);
#[repr(C)]
struct EiSeat(u8);
#[repr(C)]
struct EiDevice(u8);
#[repr(C)]
struct EiRegion(u8);

const EI_EVENT_DISCONNECT: c_uint = 2;
const EI_EVENT_SEAT_ADDED: c_uint = 3;
const EI_EVENT_DEVICE_ADDED: c_uint = 5;
const EI_EVENT_DEVICE_REMOVED: c_uint = 6;
const EI_EVENT_DEVICE_PAUSED: c_uint = 7;
const EI_EVENT_DEVICE_RESUMED: c_uint = 8;

const EI_CAP_POINTER: c_uint = 1;
const EI_CAP_POINTER_ABSOLUTE: c_uint = 2;
const EI_CAP_KEYBOARD: c_uint = 4;
const EI_CAP_SCROLL: c_uint = 16;
const EI_CAP_BUTTON: c_uint = 32;
const EI_CAP_TEXT: c_uint = 64;

struct LibEi {
    new_sender: unsafe extern "C" fn(*mut c_void) -> *mut Ei,
    unref: unsafe extern "C" fn(*mut Ei) -> *mut Ei,
    configure_name: unsafe extern "C" fn(*mut Ei, *const c_char),
    log_set_handler: unsafe extern "C" fn(*mut Ei, Option<unsafe extern "C" fn()>),
    setup_backend_fd: unsafe extern "C" fn(*mut Ei, c_int) -> c_int,
    get_fd: unsafe extern "C" fn(*mut Ei) -> c_int,
    dispatch: unsafe extern "C" fn(*mut Ei),
    get_event: unsafe extern "C" fn(*mut Ei) -> *mut EiEvent,
    event_unref: unsafe extern "C" fn(*mut EiEvent) -> *mut EiEvent,
    event_get_type: unsafe extern "C" fn(*const EiEvent) -> c_uint,
    event_get_seat: unsafe extern "C" fn(*const EiEvent) -> *mut EiSeat,
    event_get_device: unsafe extern "C" fn(*const EiEvent) -> *mut EiDevice,
    seat_has_capability: unsafe extern "C" fn(*const EiSeat, c_uint) -> bool,
    seat_bind_capabilities: unsafe extern "C" fn(*mut EiSeat, ...),
    device_ref: unsafe extern "C" fn(*mut EiDevice) -> *mut EiDevice,
    device_unref: unsafe extern "C" fn(*mut EiDevice) -> *mut EiDevice,
    device_has_capability: unsafe extern "C" fn(*const EiDevice, c_uint) -> bool,
    device_get_region_at: unsafe extern "C" fn(*mut EiDevice, usize) -> *mut EiRegion,
    region_get_x: unsafe extern "C" fn(*const EiRegion) -> u32,
    region_get_y: unsafe extern "C" fn(*const EiRegion) -> u32,
    region_get_width: unsafe extern "C" fn(*const EiRegion) -> u32,
    region_get_height: unsafe extern "C" fn(*const EiRegion) -> u32,
    device_start_emulating: unsafe extern "C" fn(*mut EiDevice, u32),
    now: unsafe extern "C" fn(*const Ei) -> u64,
    device_frame: unsafe extern "C" fn(*mut EiDevice, u64),
    device_pointer_motion_absolute: unsafe extern "C" fn(*mut EiDevice, f64, f64),
    device_button_button: unsafe extern "C" fn(*mut EiDevice, u32, bool),
    device_scroll_discrete: unsafe extern "C" fn(*mut EiDevice, i32, i32),
    device_scroll_stop: unsafe extern "C" fn(*mut EiDevice, bool, bool),
    device_scroll_cancel: unsafe extern "C" fn(*mut EiDevice, bool, bool),
    device_keyboard_key: unsafe extern "C" fn(*mut EiDevice, u32, bool),
    device_text_utf8_with_length: Option<unsafe extern "C" fn(*mut EiDevice, *const c_char, usize)>,
}

unsafe impl Send for LibEi {}
unsafe impl Sync for LibEi {}

impl LibEi {
    fn get() -> Option<&'static Self> {
        static LIB: OnceLock<Option<LibEi>> = OnceLock::new();
        LIB.get_or_init(|| unsafe { Self::load() }).as_ref()
    }

    unsafe fn sym<T: Copy>(handle: *mut c_void, name: &[u8]) -> Option<T> {
        let ptr = unsafe { libc::dlsym(handle, name.as_ptr().cast::<c_char>()) };
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { std::mem::transmute_copy(&ptr) })
        }
    }

    unsafe fn load() -> Option<Self> {
        let handle =
            unsafe { libc::dlopen(c"libei.so.1".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if handle.is_null() {
            return None;
        }
        unsafe {
            Some(Self {
                new_sender: Self::sym(handle, b"ei_new_sender\0")?,
                unref: Self::sym(handle, b"ei_unref\0")?,
                configure_name: Self::sym(handle, b"ei_configure_name\0")?,
                log_set_handler: Self::sym(handle, b"ei_log_set_handler\0")?,
                setup_backend_fd: Self::sym(handle, b"ei_setup_backend_fd\0")?,
                get_fd: Self::sym(handle, b"ei_get_fd\0")?,
                dispatch: Self::sym(handle, b"ei_dispatch\0")?,
                get_event: Self::sym(handle, b"ei_get_event\0")?,
                event_unref: Self::sym(handle, b"ei_event_unref\0")?,
                event_get_type: Self::sym(handle, b"ei_event_get_type\0")?,
                event_get_seat: Self::sym(handle, b"ei_event_get_seat\0")?,
                event_get_device: Self::sym(handle, b"ei_event_get_device\0")?,
                seat_has_capability: Self::sym(handle, b"ei_seat_has_capability\0")?,
                seat_bind_capabilities: Self::sym(handle, b"ei_seat_bind_capabilities\0")?,
                device_ref: Self::sym(handle, b"ei_device_ref\0")?,
                device_unref: Self::sym(handle, b"ei_device_unref\0")?,
                device_has_capability: Self::sym(handle, b"ei_device_has_capability\0")?,
                device_get_region_at: Self::sym(handle, b"ei_device_get_region_at\0")?,
                region_get_x: Self::sym(handle, b"ei_region_get_x\0")?,
                region_get_y: Self::sym(handle, b"ei_region_get_y\0")?,
                region_get_width: Self::sym(handle, b"ei_region_get_width\0")?,
                region_get_height: Self::sym(handle, b"ei_region_get_height\0")?,
                device_start_emulating: Self::sym(handle, b"ei_device_start_emulating\0")?,
                now: Self::sym(handle, b"ei_now\0")?,
                device_frame: Self::sym(handle, b"ei_device_frame\0")?,
                device_pointer_motion_absolute: Self::sym(
                    handle,
                    b"ei_device_pointer_motion_absolute\0",
                )?,
                device_button_button: Self::sym(handle, b"ei_device_button_button\0")?,
                device_scroll_discrete: Self::sym(handle, b"ei_device_scroll_discrete\0")?,
                device_scroll_stop: Self::sym(handle, b"ei_device_scroll_stop\0")?,
                device_scroll_cancel: Self::sym(handle, b"ei_device_scroll_cancel\0")?,
                device_keyboard_key: Self::sym(handle, b"ei_device_keyboard_key\0")?,
                device_text_utf8_with_length: Self::sym(
                    handle,
                    b"ei_device_text_utf8_with_length\0",
                ),
            })
        }
    }
}

pub(super) enum EisState {
    Uninitialized,
    Active(EisSession),
    FallbackDbus,
    Failed,
}

pub(super) struct EisSession {
    lib: &'static LibEi,
    ei: *mut Ei,
    source: SelectedPortalSource,
    abs_pointer: *mut EiDevice,
    button: *mut EiDevice,
    scroll: *mut EiDevice,
    keyboard: *mut EiDevice,
    text: *mut EiDevice,
    emulating: BTreeSet<usize>,
    sequence: u32,
    disconnected: bool,
    scroll_remainder_v120: (i32, i32),
    scrolling_axes: (bool, bool),
}

// Guarded exclusively behind `PortalInput::held` (`tokio::sync::Mutex`).
unsafe impl Send for EisSession {}

impl Drop for EisSession {
    fn drop(&mut self) {
        unsafe {
            let mut released = BTreeSet::new();
            for dev in [
                self.abs_pointer,
                self.button,
                self.scroll,
                self.keyboard,
                self.text,
            ] {
                if !dev.is_null() && released.insert(dev as usize) {
                    (self.lib.device_unref)(dev);
                }
            }
            if !self.ei.is_null() {
                (self.lib.unref)(self.ei);
            }
        }
    }
}

pub(super) enum ConnectOutcome {
    Connected(EisSession),
    Unsupported,
    Failed,
}

impl EisSession {
    pub(super) async fn connect(
        portal: &zbus::Proxy<'_>,
        session: &OwnedObjectPath,
        source: SelectedPortalSource,
    ) -> ConnectOutcome {
        let Some(lib) = LibEi::get() else {
            return ConnectOutcome::Unsupported;
        };
        let version = portal.get_property::<u32>("version").await.unwrap_or(1);
        if version < 2 {
            return ConnectOutcome::Unsupported;
        }
        let Ok(fd) = portal
            .call::<_, _, zbus::zvariant::OwnedFd>("ConnectToEIS", &(session, options()))
            .await
        else {
            return ConnectOutcome::Unsupported;
        };

        // Once `ConnectToEIS` succeeds on D-Bus, the portal requires all input on
        // this session to flow over EIS rather than `Notify*` D-Bus methods.
        tokio::task::block_in_place(|| Self::from_fd(lib, fd.into_raw_fd(), source))
    }

    fn from_fd(lib: &'static LibEi, raw_fd: c_int, source: SelectedPortalSource) -> ConnectOutcome {
        unsafe {
            let ei = (lib.new_sender)(null_mut());
            if ei.is_null() {
                libc::close(raw_fd);
                return ConnectOutcome::Failed;
            }
            (lib.log_set_handler)(ei, None);
            (lib.configure_name)(ei, c"Weavelo".as_ptr());
            if (lib.setup_backend_fd)(ei, raw_fd) != 0 {
                (lib.unref)(ei);
                libc::close(raw_fd);
                return ConnectOutcome::Failed;
            }
            let mut eis = Self {
                lib,
                ei,
                source,
                abs_pointer: null_mut(),
                button: null_mut(),
                scroll: null_mut(),
                keyboard: null_mut(),
                text: null_mut(),
                emulating: BTreeSet::new(),
                sequence: 1,
                disconnected: false,
                scroll_remainder_v120: (0, 0),
                scrolling_axes: (false, false),
            };
            let deadline = Instant::now() + Duration::from_millis(350);
            while Instant::now() < deadline {
                eis.pump();
                if eis.disconnected {
                    return ConnectOutcome::Failed;
                }
                if eis.ready() {
                    return ConnectOutcome::Connected(eis);
                }
                let wait_ms = deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .clamp(1, 25) as c_int;
                let mut pfd = libc::pollfd {
                    fd: (lib.get_fd)(ei),
                    events: libc::POLLIN,
                    revents: 0,
                };
                let _ = libc::poll(&mut pfd, 1, wait_ms);
            }
            eis.pump();
            if !eis.disconnected && eis.ready() {
                ConnectOutcome::Connected(eis)
            } else {
                ConnectOutcome::Failed
            }
        }
    }

    fn ready(&self) -> bool {
        !self.abs_pointer.is_null()
            && !self.button.is_null()
            && !self.scroll.is_null()
            && !self.keyboard.is_null()
            && self.emulating.contains(&(self.abs_pointer as usize))
            && self.emulating.contains(&(self.button as usize))
            && self.emulating.contains(&(self.scroll as usize))
            && self.emulating.contains(&(self.keyboard as usize))
    }

    fn pump(&mut self) {
        unsafe {
            (self.lib.dispatch)(self.ei);
            loop {
                let ev = (self.lib.get_event)(self.ei);
                if ev.is_null() {
                    break;
                }
                let kind = (self.lib.event_get_type)(ev);
                match kind {
                    EI_EVENT_DISCONNECT => {
                        self.disconnected = true;
                    }
                    EI_EVENT_SEAT_ADDED => {
                        let seat = (self.lib.event_get_seat)(ev);
                        if !seat.is_null() {
                            let mut caps = [0u32; 7];
                            let mut len = 0usize;
                            for cap in [
                                EI_CAP_POINTER,
                                EI_CAP_POINTER_ABSOLUTE,
                                EI_CAP_BUTTON,
                                EI_CAP_SCROLL,
                                EI_CAP_KEYBOARD,
                                EI_CAP_TEXT,
                            ] {
                                if (self.lib.seat_has_capability)(seat, cap) {
                                    caps[len] = cap;
                                    len += 1;
                                }
                            }
                            (self.lib.seat_bind_capabilities)(
                                seat, caps[0], caps[1], caps[2], caps[3], caps[4], caps[5], 0u32,
                            );
                        }
                    }
                    EI_EVENT_DEVICE_ADDED => {
                        let dev = (self.lib.event_get_device)(ev);
                        if !dev.is_null() {
                            let mut retained = false;
                            let mut assign = |slot: &mut *mut EiDevice, cap: c_uint| {
                                if slot.is_null() && (self.lib.device_has_capability)(dev, cap) {
                                    if !retained {
                                        (self.lib.device_ref)(dev);
                                        retained = true;
                                    }
                                    *slot = dev;
                                }
                            };
                            assign(&mut self.abs_pointer, EI_CAP_POINTER_ABSOLUTE);
                            assign(&mut self.button, EI_CAP_BUTTON);
                            assign(&mut self.scroll, EI_CAP_SCROLL);
                            assign(&mut self.keyboard, EI_CAP_KEYBOARD);
                            assign(&mut self.text, EI_CAP_TEXT);
                        }
                    }
                    EI_EVENT_DEVICE_REMOVED => {
                        let dev = (self.lib.event_get_device)(ev);
                        self.emulating.remove(&(dev as usize));
                        if dev == self.abs_pointer
                            || dev == self.button
                            || dev == self.scroll
                            || dev == self.keyboard
                        {
                            self.disconnected = true;
                        }
                        if dev == self.text {
                            self.text = null_mut();
                        }
                    }
                    EI_EVENT_DEVICE_PAUSED => {
                        let dev = (self.lib.event_get_device)(ev);
                        self.emulating.remove(&(dev as usize));
                    }
                    EI_EVENT_DEVICE_RESUMED => {
                        let dev = (self.lib.event_get_device)(ev);
                        if !dev.is_null() && self.emulating.insert(dev as usize) {
                            let seq = self.sequence;
                            self.sequence = self.sequence.wrapping_add(1).max(1);
                            (self.lib.device_start_emulating)(dev, seq);
                        }
                    }
                    _ => {}
                }
                (self.lib.event_unref)(ev);
            }
        }
    }

    fn ensure_ready(&mut self) -> Result<()> {
        self.pump();
        if self.disconnected || !self.ready() {
            return Err(PortalInputError::Transport);
        }
        Ok(())
    }

    fn target_region(&self, fallback_w: i32, fallback_h: i32) -> (f64, f64, f64, f64) {
        let fw = f64::from(fallback_w.max(1));
        let fh = f64::from(fallback_h.max(1));
        unsafe {
            let mut single_size_match = None;
            let mut size_matches = 0usize;
            let mut first_region = None;
            for idx in 0..32 {
                let region = (self.lib.device_get_region_at)(self.abs_pointer, idx);
                if region.is_null() {
                    break;
                }
                let rx = (self.lib.region_get_x)(region);
                let ry = (self.lib.region_get_y)(region);
                let rw = (self.lib.region_get_width)(region);
                let rh = (self.lib.region_get_height)(region);
                if rw == 0 || rh == 0 {
                    continue;
                }
                first_region.get_or_insert((rx, ry, rw, rh));
                if let Some((lx, ly)) = self.source.logical_position
                    && lx >= 0
                    && ly >= 0
                    && rx == lx as u32
                    && ry == ly as u32
                    && rw == fallback_w as u32
                    && rh == fallback_h as u32
                {
                    return (f64::from(rx), f64::from(ry), f64::from(rw), f64::from(rh));
                }
                if rw == fallback_w as u32 && rh == fallback_h as u32 {
                    size_matches += 1;
                    single_size_match = Some((rx, ry, rw, rh));
                }
            }
            if size_matches == 1
                && let Some((rx, ry, rw, rh)) = single_size_match
            {
                return (f64::from(rx), f64::from(ry), f64::from(rw), f64::from(rh));
            }
            if let Some((lx, ly)) = self.source.logical_position {
                return (f64::from(lx), f64::from(ly), fw, fh);
            }
            if let Some((rx, ry, rw, rh)) = first_region {
                return (f64::from(rx), f64::from(ry), f64::from(rw), f64::from(rh));
            }
            (0.0, 0.0, fw, fh)
        }
    }

    pub(super) fn pointer_motion_absolute(&mut self, x: u16, y: u16) -> Result<()> {
        self.ensure_ready()?;
        let (width, height) = self
            .source
            .logical_size
            .ok_or(PortalInputError::GeometryUnknown)?;
        let (rx, ry, rw, rh) = self.target_region(width, height);
        let px = rx + (f64::from(x) * (rw - 1.0).max(0.0) / f64::from(u16::MAX));
        let py = ry + (f64::from(y) * (rh - 1.0).max(0.0) / f64::from(u16::MAX));
        unsafe {
            (self.lib.device_pointer_motion_absolute)(self.abs_pointer, px, py);
            let ts = (self.lib.now)(self.ei);
            (self.lib.device_frame)(self.abs_pointer, ts);
        }
        Ok(())
    }

    pub(super) fn button(&mut self, code: i32, pressed: bool) -> Result<()> {
        self.ensure_ready()?;
        let code = u32::try_from(code).map_err(|_| PortalInputError::InvalidEvent)?;
        unsafe {
            (self.lib.device_button_button)(self.button, code, pressed);
            let ts = (self.lib.now)(self.ei);
            (self.lib.device_frame)(self.button, ts);
        }
        Ok(())
    }

    pub(super) fn key(&mut self, code: i32, pressed: bool) -> Result<()> {
        self.ensure_ready()?;
        let code = u32::try_from(code).map_err(|_| PortalInputError::InvalidEvent)?;
        unsafe {
            (self.lib.device_keyboard_key)(self.keyboard, code, pressed);
            let ts = (self.lib.now)(self.ei);
            (self.lib.device_frame)(self.keyboard, ts);
        }
        Ok(())
    }

    pub(super) fn text_ascii(&mut self, text: &str) -> Result<bool> {
        self.ensure_ready()?;
        if !self.text.is_null()
            && self.emulating.contains(&(self.text as usize))
            && let Some(send_utf8) = self.lib.device_text_utf8_with_length
        {
            if !text.is_empty() {
                unsafe {
                    send_utf8(self.text, text.as_ptr().cast::<c_char>(), text.len());
                    let ts = (self.lib.now)(self.ei);
                    (self.lib.device_frame)(self.text, ts);
                }
            }
            return Ok(true);
        }
        Ok(false)
    }

    pub(super) fn scroll(
        &mut self,
        delta_x: i32,
        delta_y: i32,
        gesture: ScrollGesture,
    ) -> Result<()> {
        self.ensure_ready()?;
        if matches!(gesture.phase, ScrollPhase::Began) {
            self.scroll_remainder_v120 = (0, 0);
        }
        if delta_x == 0 && delta_y == 0 {
            if matches!(gesture.phase, ScrollPhase::Cancelled) {
                self.scroll_remainder_v120 = (0, 0);
                let (sx, sy) = std::mem::take(&mut self.scrolling_axes);
                if sx || sy {
                    unsafe {
                        (self.lib.device_scroll_cancel)(self.scroll, sx, sy);
                        let ts = (self.lib.now)(self.ei);
                        (self.lib.device_frame)(self.scroll, ts);
                    }
                }
            } else if matches!(gesture.momentum, MomentumPhase::Ended)
                || matches!(gesture.phase, ScrollPhase::Ended)
            {
                self.scroll_remainder_v120 = (0, 0);
                let (sx, sy) = std::mem::take(&mut self.scrolling_axes);
                if sx || sy {
                    unsafe {
                        (self.lib.device_scroll_stop)(self.scroll, sx, sy);
                        let ts = (self.lib.now)(self.ei);
                        (self.lib.device_frame)(self.scroll, ts);
                    }
                }
            }
            return Ok(());
        }

        let continuous = gesture != ScrollGesture::default();
        let vx120 = step_eis_v120(delta_x, &mut self.scroll_remainder_v120.0, continuous);
        let vy120 = step_eis_v120(delta_y, &mut self.scroll_remainder_v120.1, continuous);
        if vx120 == 0 && vy120 == 0 {
            return Ok(());
        }
        if vx120 != 0 {
            self.scrolling_axes.0 = true;
        }
        if vy120 != 0 {
            self.scrolling_axes.1 = true;
        }
        unsafe {
            (self.lib.device_scroll_discrete)(self.scroll, vx120, vy120);
            let ts = (self.lib.now)(self.ei);
            (self.lib.device_frame)(self.scroll, ts);
        }
        Ok(())
    }
}

/// Convert Weavelo wire scroll delta (`120` = 1 detent; `delta_y < 0` scrolls down)
/// into libei `v120` (`+120` scrolls down; KWin `eis.so` maps `v120` simultaneously
/// to `wl_pointer.axis = v120 * 0.125` logical px and `wl_pointer.axis_value120 = v120`).
/// Accumulates `|acc| == 1` remainders because `libei` rejects `|v120| == 1` as a client bug.
pub(super) fn step_eis_v120(wire_delta: i32, remainder: &mut i32, continuous: bool) -> i32 {
    if wire_delta == 0 {
        return 0;
    }
    let acc = i64::from(*remainder) - i64::from(wire_delta);
    let clamped = acc.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i32;
    if clamped.abs() < 2 {
        *remainder = clamped;
        return 0;
    }
    *remainder = 0;
    if continuous && clamped % 120 == 0 {
        clamped - clamped.signum()
    } else {
        clamped
    }
}

/// Fallback US-layout ASCII to Linux evdev `(keycode, need_shift)` when an EIS seat
/// exposes `EI_CAP_KEYBOARD` without `EI_CAP_TEXT`.
pub(super) fn ascii_to_evdev(ch: char) -> Option<(i32, bool)> {
    Some(match ch {
        'a' => (30, false),
        'b' => (48, false),
        'c' => (46, false),
        'd' => (32, false),
        'e' => (18, false),
        'f' => (33, false),
        'g' => (34, false),
        'h' => (35, false),
        'i' => (23, false),
        'j' => (36, false),
        'k' => (37, false),
        'l' => (38, false),
        'm' => (50, false),
        'n' => (49, false),
        'o' => (24, false),
        'p' => (25, false),
        'q' => (16, false),
        'r' => (19, false),
        's' => (31, false),
        't' => (20, false),
        'u' => (22, false),
        'v' => (47, false),
        'w' => (17, false),
        'x' => (45, false),
        'y' => (21, false),
        'z' => (44, false),
        'A'..='Z' => {
            let (code, _) = ascii_to_evdev(ch.to_ascii_lowercase())?;
            (code, true)
        }
        '1' => (2, false),
        '2' => (3, false),
        '3' => (4, false),
        '4' => (5, false),
        '5' => (6, false),
        '6' => (7, false),
        '7' => (8, false),
        '8' => (9, false),
        '9' => (10, false),
        '0' => (11, false),
        '!' => (2, true),
        '@' => (3, true),
        '#' => (4, true),
        '$' => (5, true),
        '%' => (6, true),
        '^' => (7, true),
        '&' => (8, true),
        '*' => (9, true),
        '(' => (10, true),
        ')' => (11, true),
        '\n' | '\r' => (28, false),
        '\t' => (15, false),
        ' ' => (57, false),
        '-' => (12, false),
        '_' => (12, true),
        '=' => (13, false),
        '+' => (13, true),
        '[' => (26, false),
        '{' => (26, true),
        ']' => (27, false),
        '}' => (27, true),
        '\\' => (43, false),
        '|' => (43, true),
        ';' => (39, false),
        ':' => (39, true),
        '\'' => (40, false),
        '"' => (40, true),
        '`' => (41, false),
        '~' => (41, true),
        ',' => (51, false),
        '<' => (51, true),
        '.' => (52, false),
        '>' => (52, true),
        '/' => (53, false),
        '?' => (53, true),
        _ => return None,
    })
}
