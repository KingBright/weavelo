//! Explicit foreground-window control, with live WindowServer identity and hit guards.
//! Accessibility confirms activation and the focused editor behind non-key floating
//! UI. It never redirects a selected window to an application root object.
use core_foundation::{
    array::CFArray,
    base::{CFType, CFTypeRef, TCFType},
    boolean::CFBoolean,
    dictionary::{CFDictionary, CFDictionaryGetValue},
    number::CFNumber,
    string::CFString,
};
use core_graphics::geometry::{CGPoint, CGRect, CGSize};
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
    fn AXUIElementCreateSystemWide() -> CFTypeRef;
    fn AXUIElementCopyElementAtPosition(
        element: CFTypeRef,
        x: f32,
        y: f32,
        result: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementGetPid(element: CFTypeRef, pid: *mut i32) -> i32;
    fn AXUIElementCopyAttributeValue(
        element: CFTypeRef,
        name: CFTypeRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementSetAttributeValue(element: CFTypeRef, name: CFTypeRef, value: CFTypeRef) -> i32;
    fn AXUIElementPerformAction(element: CFTypeRef, action: CFTypeRef) -> i32;
    fn AXUIElementSetMessagingTimeout(element: CFTypeRef, timeout: f32) -> i32;
    fn AXValueGetType(value: CFTypeRef) -> u32;
    fn AXValueGetValue(value: CFTypeRef, kind: u32, output: *mut std::ffi::c_void) -> bool;
    fn CGWindowListCopyWindowInfo(options: u32, window: u32) -> CFTypeRef;
}
pub fn permission_available() -> bool {
    unsafe { AXIsProcessTrusted() }
}
fn attr(element: &CFType, name: &str) -> Option<CFType> {
    let name = CFString::new(name);
    let mut out = std::ptr::null();
    unsafe {
        if AXUIElementCopyAttributeValue(element.as_CFTypeRef(), name.as_CFTypeRef(), &mut out) != 0
            || out.is_null()
        {
            return None;
        }
        Some(CFType::wrap_under_create_rule(out))
    }
}
fn ax_window_bounds(window: &CFType) -> Option<CGRect> {
    let position = attr(window, "AXPosition")?;
    let size = attr(window, "AXSize")?;
    let mut origin = CGPoint::new(0., 0.);
    let mut dimensions = CGSize::new(0., 0.);
    unsafe {
        if AXValueGetType(position.as_CFTypeRef()) != 1 || AXValueGetType(size.as_CFTypeRef()) != 2
            || !AXValueGetValue(position.as_CFTypeRef(), 1, (&mut origin as *mut CGPoint).cast())
            || !AXValueGetValue(size.as_CFTypeRef(), 2, (&mut dimensions as *mut CGSize).cast()) {
            return None;
        }
    }
    Some(CGRect::new(&origin, &dimensions))
}
fn value(dictionary: &CFType, key: &str) -> Option<CFType> {
    let dictionary = dictionary.downcast::<CFDictionary>()?;
    let key = CFString::new(key);
    unsafe {
        let raw = CFDictionaryGetValue(dictionary.as_concrete_TypeRef(), key.as_CFTypeRef());
        (!raw.is_null()).then(|| CFType::wrap_under_get_rule(raw))
    }
}
fn number(d: &CFType, key: &str) -> Option<f64> {
    value(d, key)?.downcast::<CFNumber>()?.to_f64()
}
fn valid_bounds(r: CGRect) -> bool {
    [r.origin.x, r.origin.y, r.size.width, r.size.height]
        .iter()
        .all(|v| v.is_finite())
        && r.size.width >= 2.
        && r.size.height >= 2.
}
#[derive(Clone, Copy, Debug)]
struct WindowDescription {
    id: u32,
    pid: i32,
    bounds: CGRect,
    visible: bool,
    layer: i32,
    alpha: f64,
    unnamed: bool,
    generic_frame_caption: bool,
}
fn decode(d: CFType) -> Option<WindowDescription> {
    let b = value(&d, "kCGWindowBounds")?;
    let r = CGRect::new(
        &CGPoint::new(number(&b, "X")?, number(&b, "Y")?),
        &CGSize::new(number(&b, "Width")?, number(&b, "Height")?),
    );
    if !valid_bounds(r) {
        return None;
    }
    Some(WindowDescription {
        id: number(&d, "kCGWindowNumber")? as u32,
        pid: number(&d, "kCGWindowOwnerPID")? as i32,
        bounds: r,
        visible: value(&d, "kCGWindowIsOnscreen")
            .and_then(|v| v.downcast::<CFBoolean>())
            .is_some_and(|v| v.into()),
        layer: number(&d, "kCGWindowLayer")? as i32,
        alpha: number(&d, "kCGWindowAlpha").unwrap_or(1.),
        generic_frame_caption: value(&d, "kCGWindowName")
            .and_then(|v| v.downcast::<CFString>())
            .is_some_and(|s| s == "Window"),
        unnamed: value(&d, "kCGWindowName")
            .and_then(|v| v.downcast::<CFString>())
            .is_none_or(|v| v.to_string().is_empty()),
    })
}
fn windows(options: u32, id: u32) -> Result<Vec<WindowDescription>, String> {
    let raw = unsafe { CGWindowListCopyWindowInfo(options, id) };
    if raw.is_null() {
        return Err("WindowServer did not provide current window identities".into());
    }
    let list = unsafe { CFType::wrap_under_create_rule(raw) };
    let list = list
        .downcast::<CFArray>()
        .ok_or("Invalid WindowServer list")?;
    if list.len() > 4096 {
        return Err("Window list exceeds the bounded input check".into());
    }
    Ok(list
        .iter()
        .filter_map(|v| decode(unsafe { CFType::wrap_under_get_rule(*v) }))
        .collect())
}
fn current(id: u32, pid: i32) -> Result<WindowDescription, String> {
    selected(&windows(1 << 3, id)?, id, pid)
}
fn selected(list: &[WindowDescription], id: u32, pid: i32) -> Result<WindowDescription, String> {
    let w = list
        .iter()
        .find(|w| w.id == id)
        .copied()
        .ok_or("The selected application window has closed; input was not redirected")?;
    if w.pid != pid {
        return Err("The selected window changed process; reconnect the window".into());
    }
    if w.layer != 0 {
        return Err("The selected source is not a normal application window".into());
    }
    Ok(w)
}
fn contains(r: CGRect, p: CGPoint) -> bool {
    p.x >= r.origin.x
        && p.y >= r.origin.y
        && p.x < r.origin.x + r.size.width
        && p.y < r.origin.y + r.size.height
}
fn top_at(windows: &[WindowDescription], point: CGPoint) -> Option<u32> {
    windows
        .iter()
        .find(|w| w.visible && w.alpha > 0. && w.layer >= 0 && contains(w.bounds, point))
        .map(|w| w.id)
}
/// Native titlebar traffic-light surfaces can be separate layer-zero windows.
/// They are not document focus, but a same-process dialog still must block input.
/// This is deliberately narrower than discarding all small or unnamed windows.
fn observed_native_caption_geometry(child: &WindowDescription, parent: &WindowDescription) -> bool {
    // Some macOS releases name the traffic-light surface "Window". Never ignore
    // that caption alone: require the measured native size and top-left inset.
    // Pointer hit testing is unchanged; this normalization is only for focus.
    child.generic_frame_caption
        && (child.bounds.size.width - 66.).abs() <= 1.
        && (child.bounds.size.height - 20.).abs() <= 1.
        && [6., 16.].into_iter().any(|inset| {
            (child.bounds.origin.x - parent.bounds.origin.x - inset).abs() <= 1.
                && (child.bounds.origin.y - parent.bounds.origin.y - inset).abs() <= 1.
        })
}

fn frame_accessory_shape(child: &WindowDescription, parent: &WindowDescription) -> bool {
    child.id != parent.id
        && child.pid == parent.pid
        && child.layer == 0
        && parent.layer == 0
        && (child.unnamed || observed_native_caption_geometry(child, parent))
        && parent.bounds.size.width >= 160.
        && parent.bounds.size.height >= 80.
        && child.bounds.size.width <= 112.
        && child.bounds.size.height <= 36.
        && child.bounds.origin.x >= parent.bounds.origin.x
        && child.bounds.origin.y >= parent.bounds.origin.y
        && child.bounds.origin.x + child.bounds.size.width <= parent.bounds.origin.x + 120.
        && child.bounds.origin.y + child.bounds.size.height <= parent.bounds.origin.y + 40.
}
fn frame_accessory(child: &WindowDescription, parent: &WindowDescription) -> bool {
    parent.visible && parent.alpha > 0. && frame_accessory_shape(child, parent)
}
fn catalog_frame_caption_ids(list: &[WindowDescription]) -> std::collections::HashSet<u32> {
    list.iter().filter(|child| child.generic_frame_caption &&
        list.iter().any(|parent| frame_accessory_shape(child, parent)
            || hidden_fullscreen_caption(child, parent, list)))
        .map(|child| child.id).collect()
}
fn hidden_fullscreen_caption(child: &WindowDescription, titlebar: &WindowDescription, list: &[WindowDescription]) -> bool {
    // AppKit parks its hidden 32-point titlebar immediately above fullscreen
    // content. The 66x20 traffic lights remain a separate SCK "Window" source.
    // Require both native surfaces and their content owner; a small real window
    // above another window must not disappear from the catalog.
    child.id != titlebar.id && child.pid == titlebar.pid
        && child.layer == 0 && titlebar.layer == 0 && titlebar.unnamed
        && titlebar.alpha == 0. && (titlebar.bounds.size.height - 32.).abs() <= 1.
        && observed_native_caption_geometry(child, titlebar)
        && list.iter().any(|content| content.id != titlebar.id && content.id != child.id
            && content.pid == titlebar.pid && content.layer == 0
            && content.bounds.size.width >= 160. && content.bounds.size.height >= 80.
            && (content.bounds.origin.x - titlebar.bounds.origin.x).abs() <= 1.
            && (content.bounds.size.width - titlebar.bounds.size.width).abs() <= 1.
            && (content.bounds.origin.y - titlebar.bounds.origin.y - titlebar.bounds.size.height).abs() <= 1.)
}
/// These native traffic-light surfaces have no independent application content.
/// Read one WindowServer snapshot, including parents in other desktop spaces.
pub(crate) fn native_frame_caption_ids() -> Result<std::collections::HashSet<u32>, String> {
    Ok(catalog_frame_caption_ids(&windows(16, 0)?))
}
fn front_of_app(windows: &[WindowDescription], pid: i32) -> Option<u32> {
    windows
        .iter()
        .find(|w| {
            w.pid == pid
                && w.visible
                && w.layer >= 0
                && w.alpha > 0.
                && !windows.iter().any(|parent| frame_accessory(w, parent))
        })
        .map(|w| w.id)
}
fn focused_window_id(windows: &[WindowDescription], pid: i32, bounds: CGRect) -> Option<u32> {
    if !valid_bounds(bounds) { return None; }
    let mut matches=windows.iter().filter(|w| w.pid==pid && w.layer==0 && w.visible && w.alpha>0.
        && (w.bounds.origin.x-bounds.origin.x).abs()<=0.5
        && (w.bounds.origin.y-bounds.origin.y).abs()<=0.5
        && (w.bounds.size.width-bounds.size.width).abs()<=0.5
        && (w.bounds.size.height-bounds.size.height).abs()<=0.5);
    let id=matches.next()?.id;
    matches.next().is_none().then_some(id)
}
fn keyboard_focus(windows: &[WindowDescription], id:u32, pid:i32, focused_window:impl FnOnce(bool)->Option<u32>)->bool {
    let Some(front)=front_of_app(windows,pid) else {return false;};
    if front==id {return true;}
    let Some(surface)=windows.iter().find(|w|w.id==front) else {return false;};
    let Some(selected)=windows.iter().find(|w|w.id==id && w.pid==pid) else {return false;};
    // Custom titlebars can move the native traffic-light surface away from
    // AppKit's default inset. Its position alone must not change key ownership:
    // require AXFocusedWindow to identify the exact selected live window.
    let caption=surface.layer==0 && surface.generic_frame_caption
        && (surface.bounds.size.width-66.).abs()<=1.
        && (surface.bounds.size.height-20.).abs()<=1.
        && surface.bounds.origin.x>=selected.bounds.origin.x
        && surface.bounds.origin.y>=selected.bounds.origin.y
        && surface.bounds.origin.x+surface.bounds.size.width<=selected.bounds.origin.x+120.
        && surface.bounds.origin.y+surface.bounds.size.height<=selected.bounds.origin.y+40.;
    if caption {return focused_window(false)==Some(id);}
    // Non-key floating UI still requires an editor in that exact window.
    surface.layer==3 && focused_window(true)==Some(id)
}
// The callbacks are the existing WindowServer/AX operations. Keep the deadline
// check adjacent to the focus write, after every potentially blocking read.
fn focus_for_click_with(
    deadline: Option<std::time::Instant>,
    bounds: impl FnOnce() -> Result<(), String>,
    mut frontmost: impl FnMut() -> bool,
    activate: impl FnOnce() -> i32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    bounds()?;
    if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
        return Err(remote_core::InputExpired("before application activation").into());
    }
    let active = frontmost();
    if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
        return Err(remote_core::InputExpired("before application activation").into());
    }
    if active {
        return Ok(());
    }
    let status = activate();
    if status != 0 {
        return Err(format!("The selected application could not be activated ({status}); input was not redirected").into());
    }
    // AX accepts activation before AppKit finishes it. Wait only after that one
    // write; already-active input never sleeps or repeats its AX round trip.
    let until = deadline.unwrap_or_else(|| std::time::Instant::now() + std::time::Duration::from_millis(100));
    loop {
        if std::time::Instant::now() >= until {
            return Err(remote_core::InputExpired("while waiting for application activation").into());
        }
        let active = frontmost();
        // A blocking AX read must not renew or outlive the original event budget.
        if std::time::Instant::now() >= until {
            return Err(remote_core::InputExpired("while waiting for application activation").into());
        }
        if active { return Ok(()); }
        std::thread::sleep(std::time::Duration::from_millis(5)
            .min(until.saturating_duration_since(std::time::Instant::now())));
    }
}

pub struct WindowTarget {
    pub id: u32,
    pub pid: i32,
    application: CFType,
}
impl WindowTarget {
    /// Restore only the selected, uniquely identified app window. Stage
    /// Manager can leave its thumbnail visible while SCK produces no frames.
    pub fn restore_for_capture(&self, title: &str) -> Result<(), String> {
        current(self.id, self.pid)?;
        let Some(windows) = attr(&self.application, "AXWindows")
            .and_then(|v| v.downcast::<CFArray>()) else {
            // Parked native windows may expose no AX window tree. Activate
            // their verified owner; SCK still captures the exact window ID.
            // Pointer and keyboard input retain their live focus/hit guards.
            let name = CFString::new("AXFrontmost");
            let yes = CFBoolean::true_value();
            focus_for_click_with(
                None,
                || current(self.id, self.pid).map(|_| ()),
                || self.app_frontmost(),
                || unsafe {
                    AXUIElementSetAttributeValue(self.application.as_CFTypeRef(),
                        name.as_CFTypeRef(), yes.as_CFTypeRef())
                },
            ).map_err(|error| format!("The selected application could not be restored for capture: {error}"))?;
            current(self.id, self.pid)?;
            return Ok(());
        };
        let mut selected = None;
        for raw in windows.iter() {
            let window = unsafe { CFType::wrap_under_get_rule(*raw) };
            if !attr(&window, "AXTitle").and_then(|v|v.downcast::<CFString>()).is_some_and(|v|v == title) { continue; }
            if selected.is_some() { return Err("Open the selected window on the source Mac; its title is ambiguous".into()); }
            selected = Some(window);
        }
        let window = selected.ok_or("The selected application window is no longer available")?;
        let mut pid = 0;
        if unsafe { AXUIElementGetPid(window.as_CFTypeRef(), &mut pid) } != 0 || pid != self.pid {
            return Err("The selected window changed process".into());
        }
        let set = |element: &CFType, attribute: &str, value: CFBoolean| {
            let name = CFString::new(attribute);
            unsafe { AXUIElementSetAttributeValue(element.as_CFTypeRef(), name.as_CFTypeRef(), value.as_CFTypeRef()) }
        };
        if attr(&window, "AXMinimized").and_then(|v|v.downcast::<CFBoolean>()).is_some_and(|v|v.into())
            && set(&window, "AXMinimized", CFBoolean::false_value()) != 0 {
            return Err("The selected window could not be restored for capture".into());
        }
        if set(&self.application, "AXFrontmost", CFBoolean::true_value()) != 0 {
            return Err("The selected application could not be activated for capture".into());
        }
        let action = CFString::new("AXRaise");
        if unsafe { AXUIElementPerformAction(window.as_CFTypeRef(), action.as_CFTypeRef()) } != 0 {
            return Err("The selected window could not be raised for capture".into());
        }
        current(self.id, self.pid)?;
        Ok(())
    }
    pub fn new(id: u32, pid: i32) -> Result<Self, String> {
        if id == 0 || pid <= 0 {
            return Err("Invalid selected window identity".into());
        }
        if !permission_available() {
            return Err(
                "This signed Weavelo app needs Accessibility permission on the host".into(),
            );
        }
        current(id, pid)?;
        let raw = unsafe { AXUIElementCreateApplication(pid) };
        if raw.is_null() {
            return Err("Application activation object unavailable".into());
        }
        let application = unsafe { CFType::wrap_under_create_rule(raw) };
        unsafe {
            AXUIElementSetMessagingTimeout(application.as_CFTypeRef(), 0.05);
        }
        Ok(Self {
            id,
            pid,
            application,
        })
    }
    pub fn bounds(&self) -> Result<CGRect, String> {
        let w = current(self.id, self.pid)?;
        if !w.visible || w.alpha <= 0. {
            return Err(
                "Restore the selected window on the host desktop before controlling it".into(),
            );
        }
        Ok(w.bounds)
    }
    fn app_frontmost(&self) -> bool {
        attr(&self.application, "AXFrontmost")
            .and_then(|v| v.downcast::<CFBoolean>())
            .is_some_and(|v| v.into())
    }
    pub fn focused(&self) -> bool {
        self.app_frontmost() && windows(1 | 16, 0).is_ok_and(|list|
            keyboard_focus(&list,self.id,self.pid,|require_editor|self.focused_ax_window(&list,require_editor)))
    }
    fn focused_ax_window(&self, list:&[WindowDescription], require_editor:bool)->Option<u32> {
        let window=attr(&self.application,"AXFocusedWindow")?;
        if require_editor {
            let editor=attr(&self.application,"AXFocusedUIElement")?;
            if attr(&editor,"AXWindow")? != window {return None;}
        }
        let mut pid=0;
        if unsafe {AXUIElementGetPid(window.as_CFTypeRef(),&mut pid)}!=0 || pid!=self.pid {return None;}
        // Public AX geometry must identify exactly one live WindowServer window.
        // Missing/ambiguous accessibility data never weakens the old rejection.
        focused_window_id(list,pid,ax_window_bounds(&window)?)
    }
    fn raise_selected_window(&self, deadline: Option<std::time::Instant>) {
        let Ok(list) = windows(1 | 16, 0) else { return; };
        let Some(windows) = attr(&self.application, "AXWindows").and_then(|v| v.downcast::<CFArray>()) else { return; };
        let mut selected = None;
        for raw in windows.iter() {
            if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) { return; }
            let window = unsafe { CFType::wrap_under_get_rule(*raw) };
            let mut pid = 0;
            if unsafe { AXUIElementGetPid(window.as_CFTypeRef(), &mut pid) } != 0 || pid != self.pid { continue; }
            if ax_window_bounds(&window).and_then(|bounds| focused_window_id(&list, pid, bounds)) != Some(self.id) { continue; }
            // Identical/ambiguous AX windows are never substituted for the
            // selected WindowServer identity, even inside the same process.
            if selected.is_some() { return; }
            selected = Some(window);
        }
        if let Some(window) = selected {
            if current(self.id, self.pid).is_err() { return; }
            if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) { return; }
            let action = CFString::new("AXRaise");
            unsafe { AXUIElementPerformAction(window.as_CFTypeRef(), action.as_CFTypeRef()); }
        }
    }
    pub fn focus_for_input(&self, point: Option<(u16, u16)>, deadline: Option<std::time::Instant>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let name = CFString::new("AXFrontmost");
        let yes = CFBoolean::true_value();
        focus_for_click_with(
            deadline,
            || self.bounds().map(|_| ()),
            || match point {
                // App activation can finish before Stage Manager exposes the
                // window. Wait for the actual hit target, within this event's
                // original deadline, rather than permanently pausing control.
                Some((x, y)) => self.app_frontmost() && self.check_pointer(x, y).is_ok(),
                None => self.focused(),
            },
            || {
                self.raise_selected_window(deadline);
                if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) { return 0; }
                unsafe { AXUIElementSetAttributeValue(
                    self.application.as_CFTypeRef(), name.as_CFTypeRef(), yes.as_CFTypeRef(),
                ) }
            },
        )
    }
    pub fn check_pointer(&self, x: u16, y: u16) -> Result<(), String> {
        // One live snapshot binds identity, geometry and same-process ordering.
        // Do not cache it across input events or reuse pre-activation geometry.
        let list = windows(1 | 16, 0)?;
        let window = selected(&list, self.id, self.pid)?;
        if !window.visible || window.alpha <= 0. {
            return Err("Restore the selected window on the host desktop before controlling it".into());
        }
        let r = window.bounds;
        let p = crate::input_injector::normalized_pointer_location(x, y, r);
        // Same-process window ordering excludes the wrong document or sheet.
        // Cross-process hit testing below uses Accessibility, not the bounding box
        // of Dock's transparent full-screen Stage Manager surface.
        let owned: Vec<_> = list
            .into_iter()
            .filter(|w| w.pid == self.pid)
            .collect();
        if top_at(&owned, p) == Some(self.id) {
            // The global HID path is allowed only after the public hit-test agrees
            // the point belongs to the selected process. An AXApplication result
            // is sufficient here; a window title or synthetic AX child is not used.
            let raw = unsafe { AXUIElementCreateSystemWide() };
            if raw.is_null() {
                return Err("Cannot verify the foreground input target".into());
            }
            let system = unsafe { CFType::wrap_under_create_rule(raw) };
            unsafe {
                AXUIElementSetMessagingTimeout(system.as_CFTypeRef(), 0.05);
            }
            let mut raw_hit = std::ptr::null();
            let status = unsafe {
                AXUIElementCopyElementAtPosition(
                    system.as_CFTypeRef(),
                    p.x as f32,
                    p.y as f32,
                    &mut raw_hit,
                )
            };
            if status != 0 || raw_hit.is_null() {
                return Err(format!(
                    "Foreground hit test unavailable ({status}); no input sent"
                ));
            }
            let hit = unsafe { CFType::wrap_under_create_rule(raw_hit) };
            let mut pid = 0;
            if unsafe { AXUIElementGetPid(hit.as_CFTypeRef(), &mut pid) } != 0 || pid != self.pid {
                return Err("Another application covers the selected point; foreground window input is blocked".into());
            }
            Ok(())
        } else {
            Err("Another window covers the selected point. Bring the selected window forward; no input was sent to the covering window".into())
        }
    }
    pub fn alive(&self) -> bool {
        current(self.id, self.pid).is_ok()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires explicit owned AppKit fixtures and existing OS input permission"]
    fn native_two_window_input_keeps_text_and_enter_on_the_selected_source() {
        use remote_core::InputInjector;
        use protocol::{InputEvent, interaction::CommittedText};
        use std::time::{Duration, Instant};
        let receipt = std::env::var("WEAVELO_NATIVE_INPUT_FIXTURE").expect("owned fixture receipt required");
        let targets: serde_json::Value = serde_json::from_slice(&std::fs::read(receipt).unwrap()).unwrap();
        let targets = targets.as_array().unwrap();
        assert_eq!(targets.len(), 2);
        let mut expected = [String::new(), String::new()];
        for (step, index) in [0usize, 1, 0, 1].into_iter().enumerate() {
            let target = &targets[index];
            let id = u32::try_from(target["window"].as_u64().unwrap()).unwrap();
            let pid = i32::try_from(target["pid"].as_i64().unwrap()).unwrap();
            let title = target["title"].as_str().unwrap();
            assert!(title.starts_with("Weavelo native input acceptance "));
            let raw = unsafe { CGWindowListCopyWindowInfo(1 << 3, id) };
            assert!(!raw.is_null());
            let list = unsafe { CFType::wrap_under_create_rule(raw) }.downcast::<CFArray>().unwrap();
            assert!(list.iter().any(|raw| {
                let entry = unsafe { CFType::wrap_under_get_rule(*raw) };
                number(&entry, "kCGWindowNumber") == Some(f64::from(id))
                    && number(&entry, "kCGWindowOwnerPID") == Some(f64::from(pid))
                    && value(&entry, "kCGWindowName").and_then(|v| v.downcast::<CFString>()).is_some_and(|name| name == title)
            }), "native fixture identity changed");
            let injector = crate::input_injector::MacInputInjector::for_window(id, pid).unwrap();
            let text = format!("window-{index}-step-{step}你好🙂");
            let run = || -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
                let inject = |event: InputEvent| {
                    let started = Instant::now();
                    let result = injector.inject_input_before(event.clone(), started + Duration::from_millis(protocol::realtime_input::MAX_INPUT_AGE_MS));
                    eprintln!("NATIVE_EVENT event={event:?} elapsed_ms={} outcome={result:?}", started.elapsed().as_millis());
                    result
                };
                if step < 2 {
                    inject(InputEvent::MouseMoveAbsolute { x: 30000, y: 45000 })?;
                    inject(InputEvent::MouseDown(0))?;
                    inject(InputEvent::MouseUp(0))?;
                }
                inject(InputEvent::Text(CommittedText::new(text.clone()).unwrap()))?;
                inject(InputEvent::Key { key_code: 0x24, pressed: true, modifiers: 0 })?;
                inject(InputEvent::Key { key_code: 0x24, pressed: false, modifiers: 0 })?;
                Ok(())
            };
            let started = Instant::now();
            let result = run();
            injector.release_all_input();
            result.unwrap();
            expected[index].push_str(&text);
            expected[index].push('\n');
            let until = Instant::now() + Duration::from_secs(2);
            loop {
                let log = std::fs::read_to_string(target["log"].as_str().unwrap()).unwrap();
                let observed = log.lines().rev().filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                    .find(|event| event["stage"] == "textview_key_after" && event["receiver"] == title)
                    .and_then(|event| event["text"].as_str().map(str::to_owned));
                if observed.as_deref() == Some(expected[index].as_str()) { break; }
                assert!(Instant::now() < until, "native editor mismatch: source={id} expected={:?} observed={observed:?}", expected[index]);
                std::thread::sleep(Duration::from_millis(20));
            }
            eprintln!("NATIVE_WINDOW_INPUT source={id} step={step} editor_confirmed=true elapsed_ms={}", started.elapsed().as_millis());
            if step == 1 {
                let old = &targets[0];
                let previous = crate::input_injector::MacInputInjector::for_window(
                    old["window"].as_u64().unwrap().try_into().unwrap(), old["pid"].as_i64().unwrap().try_into().unwrap()).unwrap();
                previous.inject_input_before(InputEvent::ModifiersChanged(0), Instant::now() + Duration::from_millis(protocol::realtime_input::MAX_INPUT_AGE_MS)).unwrap();
                assert!(WindowTarget::new(id, pid).unwrap().focused(), "old modifier cleanup must not steal the selected editor's focus");
            }
        }
    }
    #[test]
    fn focus_deadline_read_delay_prevents_activation_and_following_input() {
        use std::{cell::Cell, time::{Duration, Instant}};
        for delay_in_bounds in [false, true] {
            let deadline = Instant::now() + Duration::from_millis(20);
            let focus = Cell::new(0);
            let input = Cell::new(0);
            let wait = || std::thread::sleep(deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(2));
            let result = focus_for_click_with(Some(deadline),
                || { if delay_in_bounds { wait(); } Ok(()) },
                || { if !delay_in_bounds { wait(); } false },
                || { focus.set(focus.get() + 1); 0 });
            if result.is_ok() { input.set(input.get() + 1); }
            assert!(result.unwrap_err().is::<remote_core::InputExpired>());
            assert_eq!(focus.get(), 0);
            assert_eq!(input.get(), 0);
        }
    }
    #[test]
    fn focus_deadline_fresh_click_can_still_activate() {
        use std::{cell::Cell, time::{Duration, Instant}};
        let active = Cell::new(false);
        let writes = Cell::new(0);
        focus_for_click_with(Some(Instant::now() + Duration::from_secs(1)), || Ok(()),
            || active.get(), || { writes.set(writes.get() + 1); active.set(true); 0 }).unwrap();
        assert_eq!(writes.get(), 1);
    }
    #[test]
    fn foreground_click_does_not_repeat_the_accessibility_round_trip() {
        use std::{cell::Cell, time::{Duration, Instant}};
        let reads = Cell::new(0);
        focus_for_click_with(Some(Instant::now() + Duration::from_secs(1)), || Ok(()),
            || { reads.set(reads.get() + 1); true },
            || panic!("an already foreground app must not be activated")).unwrap();
        assert_eq!(reads.get(), 1, "each extra AX read consumes the original input deadline");
    }
    #[test]
    fn click_waits_for_asynchronous_foreground_activation() {
        use std::{cell::Cell, time::{Duration, Instant}};
        let activated = Cell::new(None);
        let writes = Cell::new(0);
        focus_for_click_with(Some(Instant::now() + Duration::from_secs(1)), || Ok(()),
            || activated.get().is_some_and(|at: Instant| at.elapsed() >= Duration::from_millis(15)),
            || { writes.set(writes.get() + 1); activated.set(Some(Instant::now())); 0 }).unwrap();
        assert_eq!(writes.get(), 1, "wait for the one activation, never repeat the focus write");
    }
    #[test]
    fn active_application_waits_for_its_selected_pointer_target() {
        use std::{cell::Cell, time::{Duration, Instant}};
        let application_active = true;
        let raised_at = Cell::new(None);
        let raises = Cell::new(0);
        focus_for_click_with(Some(Instant::now() + Duration::from_secs(1)), || Ok(()),
            || application_active && raised_at.get().is_some_and(|at: Instant| at.elapsed() >= Duration::from_millis(15)),
            || { raises.set(raises.get() + 1); raised_at.set(Some(Instant::now())); 0 }).unwrap();
        assert_eq!(raises.get(), 1, "an active process alone must not admit input before the selected window is ready");
    }
    #[test]
    fn activation_completion_read_cannot_dispatch_after_the_original_deadline() {
        use std::{cell::Cell, time::{Duration, Instant}};
        let deadline = Instant::now() + Duration::from_millis(20);
        let writes = Cell::new(0);
        let result = focus_for_click_with(Some(deadline), || Ok(()), || {
            if writes.get() == 0 { return false; }
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(2));
            true
        }, || { writes.set(writes.get() + 1); 0 });
        assert!(result.unwrap_err().is::<remote_core::InputExpired>());
        assert_eq!(writes.get(), 1);
    }
    #[test]
    fn delayed_foreground_read_cannot_admit_expired_click() {
        use std::time::{Duration, Instant};
        let deadline = Instant::now() + Duration::from_millis(20);
        let result = focus_for_click_with(Some(deadline), || Ok(()), || {
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(2));
            true
        }, || panic!("expired input must never activate an app"));
        assert!(result.unwrap_err().is::<remote_core::InputExpired>());
    }
    fn w(id: u32, pid: i32, x: f64) -> WindowDescription {
        WindowDescription {
            id,
            pid,
            bounds: CGRect::new(&CGPoint::new(x, 20.), &CGSize::new(300., 200.)),
            visible: true,
            layer: 0,
            alpha: 1.,
            unnamed: false,
            generic_frame_caption: false,
        }
    }
    #[test]
    fn pointer_snapshot_keeps_live_geometry_identity_and_occlusion() {
        let moved = w(1, 10, 600.);
        let overlay = w(2, 10, 650.);
        let live = [overlay, moved];
        assert_eq!(selected(&live, 1, 10).unwrap().bounds.origin.x, 600.);
        assert_eq!(top_at(&live, CGPoint::new(700., 50.)), Some(2));
        assert!(selected(&live, 1, 20).unwrap_err().contains("changed process"));
        assert!(selected(&live, 3, 10).unwrap_err().contains("closed"));
        let mut invalid = moved;
        invalid.layer = 1;
        assert!(selected(&[invalid], 1, 10).unwrap_err().contains("normal application"));
    }
    #[test]
    fn native_titlebar_surface_is_not_mistaken_for_keyboard_focus() {
        let parent = w(1, 10, 150.);
        let mut controls = w(9, 10, 156.);
        controls.unnamed = true;
        controls.bounds = CGRect::new(&CGPoint::new(156., 26.), &CGSize::new(66., 20.));
        assert!(frame_accessory(&controls, &parent));
        assert_eq!(front_of_app(&[controls, parent], 10), Some(1));
    }
    #[test]
    fn observed_named_native_caption_does_not_steal_document_focus() {
        let parent = w(1, 10, 150.);
        let mut caption = w(9, 10, 156.);
        caption.generic_frame_caption = true;
        caption.bounds = CGRect::new(&CGPoint::new(156., 26.), &CGSize::new(66., 20.));
        assert!(frame_accessory(&caption, &parent));
        assert_eq!(front_of_app(&[caption, parent], 10), Some(1));
        // Do not normalize the hit-test path: clicking the actual accessory is not
        // permission to send arbitrary pointer input through it.
        assert_eq!(top_at(&[caption, parent], CGPoint::new(160., 30.)), Some(9));
    }
    #[test]
    fn source_catalog_omits_only_verified_native_captions_even_for_offscreen_parents() {
        let mut parent = w(1, 10, 150.);
        let mut caption = w(9, 10, 156.);
        caption.generic_frame_caption = true;
        caption.bounds = CGRect::new(&CGPoint::new(156., 26.), &CGSize::new(66., 20.));
        let mut small_content = caption;
        small_content.id = 12;
        small_content.bounds.origin.y += 50.;
        let mut dialog = w(14, 10, 170.);
        dialog.layer = 8;
        for visible in [true, false] {
            parent.visible = visible;
            caption.visible = visible;
            let hidden = catalog_frame_caption_ids(&[caption, small_content, dialog, parent]);
            assert_eq!(hidden, [9].into_iter().collect());
            assert_eq!(frame_accessory(&caption, &parent), visible);
        }
        let mut other_process = parent;
        other_process.pid = 20;
        assert!(catalog_frame_caption_ids(&[caption, other_process]).is_empty());
        assert!(catalog_frame_caption_ids(&[small_content, dialog, parent]).is_empty());
    }
    #[test]
    fn gemini_inset_native_caption_is_not_an_independent_source() {
        // Actual Gemini WindowServer geometry: 66x20 controls at (119,46)
        // inside its 1229x937 document at (103,30).
        let mut parent = w(20363, 13356, 103.);
        parent.bounds = CGRect::new(&CGPoint::new(103., 30.), &CGSize::new(1229., 937.));
        let mut caption = w(20434, 13356, 119.);
        caption.generic_frame_caption = true;
        caption.bounds = CGRect::new(&CGPoint::new(119., 46.), &CGSize::new(66., 20.));
        assert_eq!(catalog_frame_caption_ids(&[caption, parent]), [20434].into_iter().collect());
        assert_eq!(top_at(&[caption, parent], CGPoint::new(120., 47.)), Some(20434));
        caption.bounds.origin.y += 10.;
        assert!(catalog_frame_caption_ids(&[caption, parent]).is_empty());
    }
    #[test]
    fn catalog_omits_parked_fullscreen_caption_only_with_its_hidden_titlebar_and_content() {
        let mut content = w(1, 10, 0.);
        content.bounds = CGRect::new(&CGPoint::new(0., 0.), &CGSize::new(1920., 1080.));
        let mut titlebar = w(2, 10, 0.);
        titlebar.unnamed = true;
        titlebar.alpha = 0.;
        titlebar.bounds = CGRect::new(&CGPoint::new(0., -32.), &CGSize::new(1920., 32.));
        let mut caption = w(3, 10, 6.);
        caption.generic_frame_caption = true;
        caption.bounds = CGRect::new(&CGPoint::new(6., -26.), &CGSize::new(66., 20.));
        assert_eq!(catalog_frame_caption_ids(&[caption, titlebar, content]), [3].into_iter().collect());
        assert!(!frame_accessory(&caption, &content)); // input guards stay strict
        assert!(catalog_frame_caption_ids(&[caption, titlebar]).is_empty());
        for variation in 0..4 {
            let (mut bar, mut owner) = (titlebar, content);
            match variation {
                0 => bar.alpha = 1.,
                1 => bar.unnamed = false,
                2 => owner.pid = 20,
                _ => owner.bounds.origin.y = 20.,
            }
            assert!(catalog_frame_caption_ids(&[caption, bar, owner]).is_empty());
        }
    }
    #[test]
    fn generic_caption_is_not_a_blanket_focus_bypass() {
        let parent = w(1, 10, 150.);
        let mut child = w(9, 10, 156.);
        child.generic_frame_caption = true;
        child.bounds = CGRect::new(&CGPoint::new(156., 26.), &CGSize::new(66., 20.));
        for variation in 0..5 {
            let mut candidate = child;
            match variation {
                0 => candidate.bounds.origin.y += 50.,
                1 => candidate.bounds.size.width = 100.,
                2 => candidate.layer = 8,
                3 => candidate.pid = 20,
                _ => candidate.bounds.origin.x += 20.,
            }
            assert!(!frame_accessory(&candidate, &parent));
        }
        let modal = w(3, 10, 170.);
        assert_eq!(front_of_app(&[modal, child, parent], 10), Some(3));
    }

    #[test]
    fn small_content_window_or_modal_is_not_ignored() {
        let parent = w(1, 10, 150.);
        let mut popup = w(2, 10, 160.);
        popup.unnamed = true;
        popup.bounds = CGRect::new(&CGPoint::new(160., 100.), &CGSize::new(66., 20.));
        assert!(!frame_accessory(&popup, &parent));
        assert_eq!(front_of_app(&[popup, parent], 10), Some(2));
        popup.bounds.origin.y = 26.;
        popup.layer = 8;
        assert_eq!(front_of_app(&[popup, parent], 10), Some(2));
    }
    #[test]
    fn named_window_in_titlebar_area_is_not_ignored() {
        let parent = w(1, 10, 150.);
        let mut popup = w(2, 10, 156.);
        popup.bounds = CGRect::new(&CGPoint::new(156., 26.), &CGSize::new(66., 20.));
        assert!(!frame_accessory(&popup, &parent));
    }
    #[test]
    fn topmost_overlap_never_selects_background_window() {
        let list = [w(2, 10, 0.), w(1, 10, 0.)];
        assert_eq!(top_at(&list, CGPoint::new(50., 60.)), Some(2));
        assert_eq!(front_of_app(&list, 10), Some(2));
    }
    #[test]
    fn other_application_occlusion_is_respected() {
        let list = [w(2, 20, 0.), w(1, 10, 0.)];
        assert_eq!(top_at(&list, CGPoint::new(50., 60.)), Some(2));
    }
    #[test]
    fn higher_layer_same_process_modal_blocks_keyboard_target() {
        let mut modal = w(2, 10, 0.);
        modal.layer = 8;
        assert_eq!(front_of_app(&[modal, w(1, 10, 0.)], 10), Some(2));
    }
    #[test]
    fn non_key_floating_ui_requires_an_exact_focused_editor_window() {
        let parent=w(1,10,0.);
        let mut floating=w(2,10,20.);floating.layer=3;
        let list=[floating,parent];
        assert!(keyboard_focus(&list,1,10,|require_editor|{assert!(require_editor);focused_window_id(&list,10,parent.bounds)}));
        assert!(!keyboard_focus(&list,1,10,|_|None));
        assert!(!keyboard_focus(&list,1,10,|_|Some(2)));
        // The pointer path still sees the actual overlapping floating window.
        assert_eq!(top_at(&list,CGPoint::new(50.,60.)),Some(2));
        let mut duplicate=parent;duplicate.id=3;
        assert_eq!(focused_window_id(&[floating,parent,duplicate],10,parent.bounds),None);
        assert_eq!(focused_window_id(&list,20,parent.bounds),None);
    }
    #[test]
    fn keyboard_focus_keeps_dialog_menu_and_fast_path_guards() {
        let parent=w(1,10,0.);
        assert!(keyboard_focus(&[parent],1,10,|_|panic!("normal input must not add AX queries")));
        for layer in [0,8,101] {
            let mut dialog=w(2,10,0.);dialog.layer=layer;
            assert!(!keyboard_focus(&[dialog,parent],1,10,|_|panic!("not a non-key floating surface")));
        }
    }
    #[test]
    fn custom_titlebar_caption_uses_exact_key_window_without_requiring_an_editor() {
        // Measured DeepSeek Harness window and native caption after activation.
        let mut parent=w(18029,11682,594.);
        parent.bounds=CGRect::new(&CGPoint::new(594.,64.),&CGSize::new(1280.,820.));
        let mut caption=w(18079,11682,607.);
        caption.generic_frame_caption=true;
        caption.bounds=CGRect::new(&CGPoint::new(607.,79.),&CGSize::new(66.,20.));
        let list=[caption,parent];
        assert_eq!(front_of_app(&list,11682),Some(18079));
        assert!(keyboard_focus(&list,18029,11682,|require_editor| {
            assert!(!require_editor,"custom native windows need not export AXFocusedUIElement");
            focused_window_id(&list,11682,parent.bounds)
        }));
        assert!(!keyboard_focus(&list,18029,11682,|_|None));
        assert!(!keyboard_focus(&list,18029,11682,|_|Some(18079)));
        for variation in 0..4 {
            let mut other=caption;
            match variation {
                0=>other.bounds.size.width=100.,
                1=>other.bounds.origin.y+=50.,
                2=>other.generic_frame_caption=false,
                _=>other.layer=8,
            }
            assert!(!keyboard_focus(&[other,parent],18029,11682,|_|panic!("an ordinary overlay must still block input")));
        }
        assert_eq!(top_at(&list,CGPoint::new(620.,85.)),Some(18079),"pointer hit guards are unchanged");
    }
    #[test]
    fn same_process_filter_still_requires_separate_cross_process_hit_test() {
        let mut overlay = w(99, 20, 0.);
        overlay.layer = 20;
        let list = [overlay, w(1, 10, 0.)];
        let owned: Vec<_> = list.into_iter().filter(|w| w.pid == 10).collect();
        assert_eq!(top_at(&owned, CGPoint::new(50., 60.)), Some(1));
    }
    #[test]
    fn nonoverlap_selects_correct_same_process_window() {
        let list = [w(2, 10, 400.), w(1, 10, 0.)];
        assert_eq!(top_at(&list, CGPoint::new(50., 60.)), Some(1));
    }
    #[test]
    fn hidden_windows_cannot_receive_input() {
        let mut hidden = w(2, 10, 0.);
        hidden.visible = false;
        assert_eq!(
            top_at(&[hidden, w(1, 10, 0.)], CGPoint::new(50., 60.)),
            Some(1)
        );
    }
    #[test]
    fn negative_origins_are_valid_but_right_bottom_borders_excluded() {
        let r = w(1, 10, -400.).bounds;
        assert!(valid_bounds(r));
        assert!(contains(r, CGPoint::new(-300., 50.)));
        assert!(!contains(r, CGPoint::new(-100., 50.)));
    }
    #[test]
    fn nonfinite_geometry_rejected() {
        assert!(!valid_bounds(CGRect::new(
            &CGPoint::new(f64::NAN, 0.),
            &CGSize::new(100., 100.)
        )));
    }
}
