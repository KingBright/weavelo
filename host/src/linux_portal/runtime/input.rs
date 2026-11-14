//! Input belongs to the exact consented RemoteDesktop session and stream.
//! Text handed to the local input method uses the same consent and frame checks.

use super::{Connection, OwnedObjectPath, REMOTE_DESKTOP, SelectedPortalSource, SourceKind};
use crate::linux_frame::{OwnedFrame, Transform};
use protocol::{InputEvent, input_modifiers};
use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::sync::{Mutex as AsyncMutex, watch};
use zbus::zvariant::{Type, Value};

#[cfg(target_os = "linux")]
mod eis;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortalInputError {
    ViewOnly,
    DevicePermission,
    WindowTarget,
    GeometryUnknown,
    FrameBinding,
    SessionClosed,
    InvalidEvent,
    Expired,
    TextUnavailable,
    TextFocusChanged,
    Transport,
}
impl std::fmt::Display for PortalInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ViewOnly => "This source is view-only; select control locally through the normal RemoteDesktop dialog",
            Self::DevicePermission => "The local RemoteDesktop dialog did not grant both keyboard and pointer access",
            Self::WindowTarget => "This Wayland window has no verified input focus binding; desktop input is disabled",
            Self::GeometryUnknown => "The selected Linux stream has no verified logical geometry",
            Self::FrameBinding => "Linux stream geometry changed or has not been validated; select the source again for control",
            Self::SessionClosed => "The local Linux sharing session was cancelled or revoked",
            Self::InvalidEvent => "Input event is unsupported for this Linux source; no desktop fallback was used",
            Self::Expired => "Input event expired before native dispatch",
            Self::TextUnavailable => "Fcitx text input is unavailable; use the remote keyboard or activate the Weavelo Fcitx addon",
            Self::TextFocusChanged => "Linux text focus changed before input was committed",
            Self::Transport => "Linux portal input failed; release and select the source again",
        })
    }
}
impl std::error::Error for PortalInputError {}
type Result<T> = std::result::Result<T, PortalInputError>;

#[derive(Clone, Copy, PartialEq, Eq)]
struct FrameBinding {
    width: u32,
    height: u32,
}
fn frame_binding(frame: &OwnedFrame) -> Option<FrameBinding> {
    let format = frame.source_format;
    if format.transform != Transform::Identity
        || format.crop.is_some_and(|crop| {
            crop.x != 0 || crop.y != 0 || crop.width != format.width || crop.height != format.height
        })
        || frame.width == 0
        || frame.height == 0
    {
        return None;
    }
    Some(FrameBinding {
        width: frame.width,
        height: frame.height,
    })
}

#[derive(Default)]
struct Held {
    keys: BTreeSet<i32>,
    keysyms: BTreeSet<i32>,
    buttons: BTreeSet<i32>,
    deadline: Option<std::time::Instant>,
}

pub(super) struct PortalInput {
    connection: Connection,
    destination: String,
    session: OwnedObjectPath,
    source: SelectedPortalSource,
    devices: u32,
    revoked: watch::Receiver<bool>,
    cancelled: watch::Receiver<bool>,
    binding: Mutex<Option<FrameBinding>>,
    valid: AtomicBool,
    failed: AtomicBool,
    held: AsyncMutex<Held>,
    portal_proxy: AsyncMutex<Option<zbus::Proxy<'static>>>,
    #[cfg(target_os = "linux")]
    eis: AsyncMutex<eis::EisState>,
}
impl PortalInput {
    pub(super) fn new(
        connection: Connection,
        destination: String,
        session: OwnedObjectPath,
        source: SelectedPortalSource,
        devices: u32,
        revoked: watch::Receiver<bool>,
        cancelled: watch::Receiver<bool>,
    ) -> Self {
        Self {
            connection,
            destination,
            session,
            source,
            devices,
            revoked,
            cancelled,
            binding: Mutex::new(None),
            valid: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            held: AsyncMutex::new(Held::default()),
            portal_proxy: AsyncMutex::new(None),
            #[cfg(target_os = "linux")]
            eis: AsyncMutex::new(eis::EisState::Uninitialized),
        }
    }
    pub(super) fn bind_frame(&self, frame: &OwnedFrame) {
        let mut binding = self.binding.lock().unwrap_or_else(|e| e.into_inner());
        if binding.is_none() && frame.stamp.generation == self.source.generation {
            *binding = frame_binding(frame);
            self.valid.store(binding.is_some(), Ordering::Release);
        }
    }
    pub(super) fn observe_frame(self: &std::sync::Arc<Self>, frame: &OwnedFrame) {
        let binding = *self.binding.lock().unwrap_or_else(|e| e.into_inner());
        if (frame.stamp.generation != self.source.generation || binding != frame_binding(frame))
            && self.valid.swap(false, Ordering::AcqRel)
        {
            // Start's logical geometry cannot be inferred from changed video pixels.
            // Keep video working, but require a new consented selection for input.
            let input = self.clone();
            tokio::spawn(async move {
                let _ = input.release().await;
            });
        }
    }
    pub(super) fn available(&self) -> bool {
        self.status().is_ok()
    }
    pub(super) fn status(&self) -> Result<()> {
        if *self.revoked.borrow()
            || *self.cancelled.borrow()
            || self.revoked.has_changed().is_err()
            || self.cancelled.has_changed().is_err()
        {
            return Err(PortalInputError::SessionClosed);
        }
        if self.devices != 3 {
            return Err(PortalInputError::DevicePermission);
        }
        if self.source.kind != SourceKind::Monitor {
            return Err(PortalInputError::WindowTarget);
        }
        if self.source.node_id == 0
            || self.source.node_id == u32::MAX
            || self
                .source
                .logical_size
                .is_none_or(|(w, h)| w <= 0 || h <= 0)
        {
            return Err(PortalInputError::GeometryUnknown);
        }
        if self.failed.load(Ordering::Acquire) {
            return Err(PortalInputError::Transport);
        }
        if !self.valid.load(Ordering::Acquire) {
            return Err(PortalInputError::FrameBinding);
        }
        Ok(())
    }
    async fn cached_portal_proxy(&self) -> Result<zbus::Proxy<'static>> {
        let mut guard = self.portal_proxy.lock().await;
        if let Some(existing) = guard.as_ref() {
            return Ok(existing.clone());
        }
        let created = zbus::proxy::Builder::<'static, zbus::Proxy<'static>>::new(&self.connection)
            .destination(self.destination.clone())
            .map_err(|_| PortalInputError::Transport)?
            .path(super::DESKTOP_PATH)
            .map_err(|_| PortalInputError::Transport)?
            .interface(REMOTE_DESKTOP)
            .map_err(|_| PortalInputError::Transport)?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await
            .map_err(|_| PortalInputError::Transport)?;
        *guard = Some(created.clone());
        Ok(created)
    }
    async fn notify<B: Serialize + Type>(
        &self,
        name: &str,
        body: &B,
        deadline: Option<std::time::Instant>,
    ) -> Result<()> {
        tokio::time::timeout(Duration::from_millis(500), async {
            let portal = self.cached_portal_proxy().await?;
            if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
                return Err(PortalInputError::Expired);
            }
            portal
                .call::<_, _, ()>(name, body)
                .await
                .map_err(|_| PortalInputError::Transport)
        })
        .await
        .map_err(|_| PortalInputError::Transport)?
    }
    #[cfg(target_os = "linux")]
    async fn ensure_eis(&self) -> Result<bool> {
        if self.destination != super::DESKTOP {
            return Ok(false);
        }
        let mut state = self.eis.lock().await;
        match &*state {
            eis::EisState::Active(_) => return Ok(true),
            eis::EisState::FallbackDbus => return Ok(false),
            eis::EisState::Failed => return Err(PortalInputError::Transport),
            eis::EisState::Uninitialized => {}
        }
        let portal = self.cached_portal_proxy().await?;
        match eis::EisSession::connect(&portal, &self.session, self.source).await {
            eis::ConnectOutcome::Connected(session) => {
                *state = eis::EisState::Active(session);
                Ok(true)
            }
            eis::ConnectOutcome::Unsupported => {
                *state = eis::EisState::FallbackDbus;
                Ok(false)
            }
            eis::ConnectOutcome::Failed => {
                *state = eis::EisState::Failed;
                Err(PortalInputError::Transport)
            }
        }
    }
    async fn key(&self, held: &mut Held, key: i32, pressed: bool) -> Result<()> {
        // Record before RPC: a missing reply does not prove the compositor did
        // not inject the down event. A later release must include that key.
        if pressed {
            self.status()?;
            if !held.keys.insert(key) {
                return Ok(());
            }
        } else if !held.keys.remove(&key) {
            return Ok(());
        }
        #[cfg(target_os = "linux")]
        if self.ensure_eis().await? {
            if held
                .deadline
                .is_some_and(|deadline| std::time::Instant::now() >= deadline)
            {
                if pressed {
                    held.keys.remove(&key);
                } else {
                    held.keys.insert(key);
                }
                return Err(PortalInputError::Expired);
            }
            let mut state = self.eis.lock().await;
            if let eis::EisState::Active(eis) = &mut *state {
                return eis.key(key, pressed);
            }
            return Err(PortalInputError::Transport);
        }
        let result = self
            .notify(
                "NotifyKeyboardKeycode",
                &(&self.session, options(), key, u32::from(pressed)),
                held.deadline,
            )
            .await;
        if result == Err(PortalInputError::Expired) {
            // Definitely not dispatched: retain an earlier owned down for cleanup.
            if pressed {
                held.keys.remove(&key);
            } else {
                held.keys.insert(key);
            }
        }
        result
    }
    async fn keysym(&self, held: &mut Held, sym: i32, pressed: bool) -> Result<()> {
        if pressed {
            self.status()?;
            if !held.keysyms.insert(sym) {
                return Ok(());
            }
        } else if !held.keysyms.remove(&sym) {
            return Ok(());
        }
        let result = self
            .notify(
                "NotifyKeyboardKeysym",
                &(&self.session, options(), sym, u32::from(pressed)),
                held.deadline,
            )
            .await;
        if result == Err(PortalInputError::Expired) {
            if pressed {
                held.keysyms.remove(&sym);
            } else {
                held.keysyms.insert(sym);
            }
        }
        result
    }
    async fn button(&self, held: &mut Held, button: i32, pressed: bool) -> Result<()> {
        if pressed {
            self.status()?;
            if !held.buttons.insert(button) {
                return Ok(());
            }
        } else if !held.buttons.remove(&button) {
            return Ok(());
        }
        #[cfg(target_os = "linux")]
        if self.ensure_eis().await? {
            if held
                .deadline
                .is_some_and(|deadline| std::time::Instant::now() >= deadline)
            {
                if pressed {
                    held.buttons.remove(&button);
                } else {
                    held.buttons.insert(button);
                }
                return Err(PortalInputError::Expired);
            }
            let mut state = self.eis.lock().await;
            if let eis::EisState::Active(eis) = &mut *state {
                return eis.button(button, pressed);
            }
            return Err(PortalInputError::Transport);
        }
        let result = self
            .notify(
                "NotifyPointerButton",
                &(&self.session, options(), button, u32::from(pressed)),
                held.deadline,
            )
            .await;
        if result == Err(PortalInputError::Expired) {
            // Definitely not dispatched: retain an earlier owned down for cleanup.
            if pressed {
                held.buttons.remove(&button);
            } else {
                held.buttons.insert(button);
            }
        }
        result
    }
    async fn modifiers(&self, held: &mut Held, flags: u8) -> Result<()> {
        if flags
            & !(input_modifiers::SHIFT
                | input_modifiers::CONTROL
                | input_modifiers::ALT
                | input_modifiers::META
                | input_modifiers::CAPS_LOCK)
            != 0
        {
            return Err(PortalInputError::InvalidEvent);
        }
        // Caps Lock is a toggle key, not a held modifier; only its explicit key
        // event may toggle it. Do not infer the host's lock state from a viewer.
        for (flag, code) in [
            (input_modifiers::SHIFT, 42),
            (input_modifiers::CONTROL, 29),
            (input_modifiers::ALT, 56),
            (input_modifiers::META, 125),
        ] {
            let pressed = flags & flag != 0;
            if held.keys.contains(&code) != pressed {
                self.key(held, code, pressed).await?;
            }
        }
        Ok(())
    }
    pub(super) async fn inject(&self, event: InputEvent) -> Result<()> {
        self.inject_before(event, None).await
    }
    pub(super) async fn inject_before(
        &self,
        event: InputEvent,
        deadline: Option<std::time::Instant>,
    ) -> Result<()> {
        let mut held = self.held.lock().await;
        held.deadline = deadline;
        if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
            return Err(PortalInputError::Expired);
        }
        self.status()?;
        let result = async {
            match event {
                InputEvent::Text(text) => {
                    if [42,54,29,97,56,100,125,126].iter().any(|key|held.keys.contains(key)) {
                        return Err(PortalInputError::InvalidEvent);
                    }
                    #[cfg(target_os = "linux")]
                    if !text.as_str().is_ascii() {
                        use std::time::Instant;

                        let deadline = deadline.unwrap_or_else(|| Instant::now()
                            + Duration::from_millis(protocol::realtime_input::MAX_INPUT_AGE_MS));
                        if let Some(commit) = super::fcitx_text::prepare(&self.connection, deadline).await? {
                            // The addon has captured the focused input context, but has not
                            // received any text. Recheck the exact sharing grant before sending.
                            self.status()?;
                            let mut revoked = self.revoked.clone();
                            let mut cancelled = self.cancelled.clone();
                            return tokio::select! {
                                biased;
                                _ = revoked.wait_for(|closed| *closed) => Err(PortalInputError::SessionClosed),
                                _ = cancelled.wait_for(|closed| *closed) => Err(PortalInputError::SessionClosed),
                                result = commit.send(text.as_str(), deadline) => result,
                            };
                        }
                    }
                    #[cfg(target_os = "linux")]
                    if self.ensure_eis().await? {
                        let sent_utf8 = {
                            let mut state = self.eis.lock().await;
                            if let eis::EisState::Active(eis) = &mut *state {
                                eis.text_ascii(text.as_str())?
                            } else {
                                return Err(PortalInputError::Transport);
                            }
                        };
                        if sent_utf8 {
                            return Ok(());
                        }
                        for ch in text.as_str().chars() {
                            if let Some((code, need_shift)) = eis::ascii_to_evdev(ch) {
                                if need_shift {
                                    self.key(&mut held, 42, true).await?;
                                }
                                self.key(&mut held, code, true).await?;
                                self.key(&mut held, code, false).await?;
                                if need_shift {
                                    self.key(&mut held, 42, false).await?;
                                }
                            }
                        }
                        return Ok(());
                    }
                    for ch in text.as_str().chars() {
                        let sym = match ch { '\n'|'\r'=>0xff0d, '\t'=>0xff09,
                            ch if (ch as u32)<=0xff => ch as i32,
                            ch => 0x01000000 | ch as i32 };
                        self.keysym(&mut held,sym,true).await?;
                        self.keysym(&mut held,sym,false).await?;
                    }
                    Ok(())
                }
                InputEvent::MouseMoveAbsolute { x, y } => {
                    #[cfg(target_os = "linux")]
                    if self.ensure_eis().await? {
                        if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
                            return Err(PortalInputError::Expired);
                        }
                        let mut state = self.eis.lock().await;
                        if let eis::EisState::Active(eis) = &mut *state {
                            return eis.pointer_motion_absolute(x, y);
                        }
                        return Err(PortalInputError::Transport);
                    }
                    let (width, height) = self
                        .source
                        .logical_size
                        .ok_or(PortalInputError::GeometryUnknown)?;
                    // Session-local stream coordinates: no monitor offset, video
                    // pixel size, guessed scale, or raw node from a remote peer.
                    let x = f64::from(x) * f64::from(width - 1) / f64::from(u16::MAX);
                    let y = f64::from(y) * f64::from(height - 1) / f64::from(u16::MAX);
                    self.notify(
                        "NotifyPointerMotionAbsolute",
                        &(&self.session, options(), self.source.node_id, x, y),deadline,
                    )
                    .await
                }
                InputEvent::MouseDown(button) => {
                    self.button(&mut held, button_code(button)?, true).await
                }
                InputEvent::MouseUp(button) => {
                    self.button(&mut held, button_code(button)?, false).await
                }
                InputEvent::MouseScroll { delta_x, delta_y, gesture } => {
                    let _ = gesture;
                    #[cfg(target_os = "linux")]
                    if self.ensure_eis().await? {
                        if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
                            return Err(PortalInputError::Expired);
                        }
                        let mut state = self.eis.lock().await;
                        if let eis::EisState::Active(eis) = &mut *state {
                            return eis.scroll(delta_x, delta_y, gesture);
                        }
                        return Err(PortalInputError::Transport);
                    }
                    let Some((axis_x, axis_y)) = portal_scroll_axis(
                        delta_x,
                        delta_y,
                        self.destination == super::DESKTOP,
                        is_kde_portal(),
                    ) else {
                        return Ok(());
                    };
                    self.notify(
                        "NotifyPointerAxis",
                        &(&self.session, options(), axis_x, axis_y),
                        deadline,
                    )
                    .await
                }
                InputEvent::KeyDown(code) => {
                    self.key(
                        &mut held,
                        key_code(u16::try_from(code).map_err(|_| PortalInputError::InvalidEvent)?)?,
                        true,
                    )
                    .await
                }
                InputEvent::KeyUp(code) => {
                    self.key(
                        &mut held,
                        key_code(u16::try_from(code).map_err(|_| PortalInputError::InvalidEvent)?)?,
                        false,
                    )
                    .await
                }
                InputEvent::Key {
                    key_code: code,
                    pressed,
                    modifiers,
                } => {
                    let code = key_code(code)?;
                    let owned_modifiers=[(1,42),(2,29),(4,56),(8,125)].into_iter().fold(0,|mask,(flag,key)|if held.keys.contains(&key){mask|flag}else{mask});
                    let modifiers=if pressed {modifiers} else {modifiers & owned_modifiers};
                    self.modifiers(&mut held, modifiers).await?;
                    self.key(&mut held, code, pressed).await
                }
                InputEvent::ModifiersChanged(flags) => self.modifiers(&mut held, flags).await,
                InputEvent::MouseMove { .. }
                | InputEvent::Touch { .. }
                | InputEvent::TouchFrame(_)
                | InputEvent::Gamepad(_) => {
                    Err(PortalInputError::InvalidEvent)
                }
            }
        }
        .await;
        if result == Err(PortalInputError::Transport) {
            self.failed.store(true, Ordering::Release);
            self.valid.store(false, Ordering::Release);
            drop(held);
            let _ = self.release().await;
        }
        result
    }
    pub(super) async fn release(&self) -> Result<()> {
        let result = tokio::time::timeout(Duration::from_millis(750), self.release_inner())
            .await
            .unwrap_or(Err(PortalInputError::Transport));
        if result.is_err() {
            self.failed.store(true, Ordering::Release);
        }
        result
    }
    async fn release_inner(&self) -> Result<()> {
        let mut held = self.held.lock().await;
        held.deadline = None;
        let mut result = Ok(());
        for key in held.keys.clone() {
            if self.key(&mut held, key, false).await.is_err() {
                result = Err(PortalInputError::Transport);
            }
        }
        for sym in held.keysyms.clone() {
            if self.keysym(&mut held, sym, false).await.is_err() {
                result = Err(PortalInputError::Transport);
            }
        }
        for button in held.buttons.clone() {
            if self.button(&mut held, button, false).await.is_err() {
                result = Err(PortalInputError::Transport);
            }
        }
        result
    }
}
fn options() -> std::collections::HashMap<&'static str, Value<'static>> {
    std::collections::HashMap::new()
}
fn is_kde_portal() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|val| val.split(':').any(|part| part.eq_ignore_ascii_case("KDE")))
        .unwrap_or(false)
}
/// Wire scroll deltas follow GPUI/macOS/Windows pixel sign (`delta_y < 0` scrolls down).
/// On live Wayland portals (`live_portal == true`), GNOME/wlroots expect `+dy` for downward
/// scroll (`(-dx, -dy)`), whereas `xdg-desktop-portal-kde` negates `dy` internally (`dy * -256.0`)
/// so KDE fallback needs `(-dx, +dy)`. Private test fixtures (`live_portal == false`) observe raw `(dx, dy)`.
fn portal_scroll_axis(
    delta_x: i32,
    delta_y: i32,
    live_portal: bool,
    kde_portal: bool,
) -> Option<(f64, f64)> {
    if delta_x == 0 && delta_y == 0 {
        return None;
    }
    if !live_portal {
        return Some((f64::from(delta_x), f64::from(delta_y)));
    }
    let axis_x = -f64::from(delta_x);
    let axis_y = if kde_portal {
        f64::from(delta_y)
    } else {
        -f64::from(delta_y)
    };
    Some((axis_x, axis_y))
}
fn button_code(button: u8) -> Result<i32> {
    Ok(match button {
        0 => 0x110,
        1 => 0x111,
        2 => 0x112,
        3 => 0x113,
        4 => 0x114,
        _ => return Err(PortalInputError::InvalidEvent),
    })
}
/// The existing wire key convention is macOS virtual keys, including GPUI on
/// Linux/Windows. Unknown values must not become another printable key.
fn key_code(code: u16) -> Result<i32> {
    Ok(match code {
        0x00 => 30,
        0x01 => 31,
        0x02 => 32,
        0x03 => 33,
        0x04 => 35,
        0x05 => 34,
        0x06 => 44,
        0x07 => 45,
        0x08 => 46,
        0x09 => 47,
        0x0b => 48,
        0x0c => 16,
        0x0d => 17,
        0x0e => 18,
        0x0f => 19,
        0x10 => 21,
        0x11 => 20,
        0x12 => 2,
        0x13 => 3,
        0x14 => 4,
        0x15 => 5,
        0x16 => 7,
        0x17 => 6,
        0x18 => 13,
        0x19 => 10,
        0x1a => 8,
        0x1b => 12,
        0x1c => 9,
        0x1d => 11,
        0x1e => 27,
        0x1f => 24,
        0x20 => 22,
        0x21 => 26,
        0x22 => 23,
        0x23 => 25,
        0x24 => 28,
        0x25 => 38,
        0x26 => 36,
        0x27 => 40,
        0x28 => 37,
        0x29 => 39,
        0x2a => 43,
        0x2b => 51,
        0x2c => 53,
        0x2d => 49,
        0x2e => 50,
        0x2f => 52,
        0x30 => 15,
        0x31 => 57,
        0x32 => 41,
        0x33 => 14,
        0x35 => 1,
        0x37 => 125,
        0x38 => 42,
        0x39 => 58,
        0x3a => 56,
        0x3b => 29,
        0x3c => 54,
        0x3d => 100,
        0x3e => 97,
        0x7a => 59,
        0x78 => 60,
        0x63 => 61,
        0x76 => 62,
        0x60 => 63,
        0x61 => 64,
        0x62 => 65,
        0x64 => 66,
        0x65 => 67,
        0x6d => 68,
        0x67 => 87,
        0x6f => 88,
        0x72 => 110,
        0x73 => 102,
        0x74 => 104,
        0x75 => 111,
        0x77 => 107,
        0x79 => 109,
        0x7b => 105,
        0x7c => 106,
        0x7d => 108,
        0x7e => 103,
        _ => return Err(PortalInputError::InvalidEvent),
    })
}

#[cfg(test)]
mod tests;
