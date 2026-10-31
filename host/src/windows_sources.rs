//! Current native handles stay in this process; the wire only carries opaque source IDs.
use crate::windows_capture_plan::{Encoder, Rect};
use protocol::session::{CaptureSource, CaptureSourceInfo};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709;
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, IDXGIOutput6};
use windows::core::Interface;
use windows_sys::Win32::{
    Foundation::{CloseHandle, FILETIME, HWND, LPARAM, POINT, RECT},
    Graphics::Gdi::ClientToScreen,
    System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    },
    UI::{
        Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
        WindowsAndMessaging::*,
    },
};

#[derive(Clone, Debug)]
pub(crate) struct Selection {
    pub source: CaptureSource,
    pub handle: u64,
    pub window: bool,
    pub bounds: Rect,
    pub pid: u32,
    created: u64,
    pub title: String,
    pub application: String,
    pub monitor: crate::capture_readiness::WindowsCaptureRect,
    pub encoder: Encoder,
}
#[derive(Default)]
struct Registry {
    next: u32,
    entries: HashMap<u32, (Selection, u32)>,
}
static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(Default::default)
}
fn wide(s: &[u16]) -> String {
    String::from_utf16_lossy(&s[..s.iter().position(|v| *v == 0).unwrap_or(s.len())])
}

pub(crate) struct PhysicalCoordinates(windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT);
impl PhysicalCoordinates {
    pub fn enter() -> Result<Self, String> {
        unsafe {
            use windows_sys::Win32::UI::HiDpi::*;
            let previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            if previous.is_null() {
                return Err("Physical input coordinates require per-monitor DPI awareness".into());
            }
            Ok(Self(previous))
        }
    }
}
impl Drop for PhysicalCoordinates {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(self.0);
        }
    }
}
fn process_created(pid: u32) -> Option<u64> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let (mut created, mut exited, mut kernel, mut user) = (
            std::mem::zeroed(),
            std::mem::zeroed(),
            std::mem::zeroed(),
            std::mem::zeroed(),
        );
        let okay = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) != 0;
        CloseHandle(handle);
        okay.then_some((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
    }
}

fn process(pid: u32) -> Option<(u64, String)> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let (mut created, mut exited, mut kernel, mut user) = (
            std::mem::zeroed(),
            std::mem::zeroed(),
            std::mem::zeroed(),
            std::mem::zeroed(),
        );
        let mut name = vec![0u16; 32768];
        let mut len = name.len() as u32;
        let okay = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) != 0
            && QueryFullProcessImageNameW(handle, 0, name.as_mut_ptr(), &mut len) != 0;
        CloseHandle(handle);
        if !okay {
            return None;
        }
        Some((
            (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime),
            String::from_utf16_lossy(&name[..len as usize]),
        ))
    }
}

pub(crate) fn client_rect(hwnd: HWND) -> Option<Rect> {
    unsafe {
        let _coordinates = PhysicalCoordinates::enter().ok()?;
        let mut r = std::mem::zeroed();
        if GetClientRect(hwnd, &mut r) == 0 {
            return None;
        }
        let mut origin = POINT {
            x: r.left,
            y: r.top,
        };
        if ClientToScreen(hwnd, &mut origin) == 0 {
            return None;
        }
        let bounds = Rect {
            x: origin.x,
            y: origin.y,
            width: (r.right - r.left).try_into().ok()?,
            height: (r.bottom - r.top).try_into().ok()?,
        };
        bounds.valid().then_some(bounds)
    }
}

unsafe extern "system" fn lifetime(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    child: i32,
    _thread: u32,
    time: u32,
) {
    if object != 0
        || child != 0
        || hwnd.is_null()
        || !matches!(event, EVENT_OBJECT_CREATE | EVENT_OBJECT_DESTROY)
    {
        return;
    }
    if let Some(state) = REGISTRY.get() {
        state.lock().unwrap().entries.retain(|_, (s, issued)| {
            !(s.window
                && s.handle == hwnd as usize as u64
                && (event == EVENT_OBJECT_DESTROY || time.wrapping_sub(*issued) as i32 >= 0))
        });
    }
}
fn observe_lifetimes() -> Result<(), String> {
    static OBSERVER: OnceLock<Result<(), String>> = OnceLock::new();
    OBSERVER
        .get_or_init(|| {
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            std::thread::Builder::new()
                .name("weavelo-window-lifetimes".into())
                .spawn(move || unsafe {
                    let hook = SetWinEventHook(
                        EVENT_OBJECT_CREATE,
                        EVENT_OBJECT_DESTROY,
                        std::ptr::null_mut(),
                        Some(lifetime),
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT,
                    );
                    if hook.is_null() {
                        let _ = tx.send(Err(
                            "Windows could not observe capture-source lifetimes".into()
                        ));
                        return;
                    }
                    let _ = tx.send(Ok(()));
                    let mut message = std::mem::zeroed();
                    while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                        TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                    UnhookWinEvent(hook);
                })
                .map_err(|e| e.to_string())?;
            rx.recv_timeout(std::time::Duration::from_secs(2))
                .map_err(|e| e.to_string())?
        })
        .clone()
}

fn monitors() -> Result<Vec<Selection>, String> {
    unsafe {
        let physical = crate::capture_readiness::physical_display_rects()?;
        let factory: IDXGIFactory1 = CreateDXGIFactory1().map_err(|e| e.to_string())?;
        let mut result = Vec::new();
        for index in 0..16 {
            let Ok(adapter) = factory.EnumAdapters1(index) else {
                break;
            };
            let description = adapter.GetDesc1().map_err(|e| e.to_string())?;
            let Ok(encoder) = Encoder::from_vendor(description.VendorId) else {
                continue;
            };
            for index in 0..32 {
                let Ok(output) = adapter.EnumOutputs(index) else {
                    break;
                };
                let desc = output.GetDesc().map_err(|e| e.to_string())?;
                let device = wide(&desc.DeviceName);
                let Some(monitor) = physical.iter().find(|m| m.device_name == device) else {
                    continue;
                };
                let color = output
                    .cast::<IDXGIOutput6>()
                    .and_then(|output| output.GetDesc1())
                    .map_err(|e| format!("Cannot verify the native display color space: {e}"))?;
                if color.ColorSpace != DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709 {
                    continue;
                }
                result.push(Selection {
                    source: CaptureSource::MainDisplay,
                    handle: desc.Monitor.0 as usize as u64,
                    window: false,
                    bounds: Rect {
                        x: monitor.x,
                        y: monitor.y,
                        width: monitor.width,
                        height: monitor.height,
                    },
                    pid: 0,
                    created: 0,
                    title: format!("Display · {}", monitor.adapter),
                    application: String::new(),
                    monitor: monitor.clone(),
                    encoder,
                });
            }
        }
        result.sort_by_key(|s| {
            physical
                .iter()
                .position(|m| m.device_name == s.monitor.device_name)
                .unwrap_or(usize::MAX)
        });
        if result.is_empty() {
            return Err(
                "No physical SDR display with a supported hardware HEVC encoder is available"
                    .into(),
            );
        }
        Ok(result)
    }
}

unsafe extern "system" fn enum_window(hwnd: HWND, data: LPARAM) -> i32 {
    unsafe {
        let list = &mut *(data as *mut Vec<(u64, u32, u64, String, String, Rect)>);
        if list.len() >= 256 {
            return 0;
        }
        if IsWindowVisible(hwnd) == 0
            || IsIconic(hwnd) != 0
            || hwnd == GetShellWindow()
            || hwnd == GetDesktopWindow()
            || GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW != 0
        {
            return 1;
        }
        let mut affinity = 0;
        if GetWindowDisplayAffinity(hwnd, &mut affinity) != 0 && affinity != 0 {
            return 1;
        }
        let mut title = [0u16; 1024];
        if GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32) == 0 {
            return 1;
        }
        let Some(bounds) = client_rect(hwnd) else {
            return 1;
        };
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let Some((created, path)) = process(pid) else {
            return 1;
        };
        let app = std::path::Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        list.push((
            hwnd as usize as u64,
            pid,
            created,
            wide(&title),
            app,
            bounds,
        ));
        1
    }
}

pub fn list() -> Result<Vec<CaptureSourceInfo>, String> {
    observe_lifetimes()?;
    let mut native = monitors()?;
    let displays = native.clone();
    let mut windows = Vec::new();
    unsafe {
        EnumWindows(Some(enum_window), &mut windows as *mut _ as isize);
    }
    for (handle, pid, created, title, application, bounds) in windows {
        // A mixed/unknown monitor color space cannot be described by guessed RGB metadata.
        let Some(display) = displays.iter().find(|s| s.bounds.contains(bounds)) else {
            continue;
        };
        native.push(Selection {
            source: CaptureSource::Window(0),
            handle,
            window: true,
            bounds,
            pid,
            created,
            title,
            application,
            monitor: display.monitor.clone(),
            encoder: display.encoder,
        });
    }
    let mut state = registry().lock().unwrap();
    let mut result = Vec::new();
    state
        .entries
        .retain(|_, (s, _)| !s.window || unsafe { IsWindow(s.handle as usize as HWND) != 0 });
    for mut source in native {
        let existing = state
            .entries
            .iter()
            .find(|(_, (old, _))| {
                old.window == source.window
                    && old.handle == source.handle
                    && old.pid == source.pid
                    && old.created == source.created
            })
            .map(|(id, _)| *id);
        let id = if let Some(id) = existing {
            id
        } else {
            if state.entries.len() >= 4096 {
                return Err("Native source catalog is full".into());
            }
            state.next = state
                .next
                .checked_add(1)
                .ok_or("Native source IDs are exhausted")?;
            state.next
        };
        source.source = if source.window {
            CaptureSource::Window(id)
        } else {
            CaptureSource::Display(id)
        };
        result.push(CaptureSourceInfo {
            source: source.source,
            title: source.title.clone(),
            application: source.application.clone(),
            process_id: source.window.then_some(source.pid as i32),
            width: source.bounds.width,
            height: source.bounds.height,
            supports_input: true,
        });
        let issued = state
            .entries
            .get(&id)
            .map(|(_, issued)| *issued)
            .unwrap_or_else(|| unsafe {
                windows_sys::Win32::System::SystemInformation::GetTickCount()
            });
        state.entries.insert(id, (source, issued));
    }
    Ok(result)
}

pub(crate) fn resolve(source: CaptureSource) -> Result<Selection, String> {
    if matches!(source, CaptureSource::MainDisplay) {
        let first = list()?
            .into_iter()
            .find(|s| matches!(s.source, CaptureSource::Display(_)))
            .ok_or("No physical display is available")?;
        return resolve(first.source);
    }
    let id = match source {
        CaptureSource::Window(id) | CaptureSource::Display(id) => id,
        _ => unreachable!(),
    };
    let selected = registry()
        .lock()
        .unwrap()
        .entries
        .get(&id)
        .map(|(s, _)| s.clone())
        .ok_or("The selected native source ended; refresh screens and windows")?;
    if selected.window {
        let hwnd = selected.handle as usize as HWND;
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
        }
        if pid != selected.pid || process_created(pid) != Some(selected.created) {
            return Err("The selected native window is no longer available".into());
        }
    } else {
        unsafe {
            use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFOEXW};
            let _coordinates = PhysicalCoordinates::enter()?;
            let mut info: MONITORINFOEXW = std::mem::zeroed();
            info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            if GetMonitorInfoW(selected.handle as usize as _, &mut info.monitorInfo) == 0
                || wide(&info.szDevice) != selected.monitor.device_name
            {
                return Err("The selected display is no longer attached".into());
            }
            let r = info.monitorInfo.rcMonitor;
            let current = Rect {
                x: r.left,
                y: r.top,
                width: (r.right - r.left)
                    .try_into()
                    .map_err(|_| "Invalid display width")?,
                height: (r.bottom - r.top)
                    .try_into()
                    .map_err(|_| "Invalid display height")?,
            };
            if current != selected.bounds {
                return Err("Display geometry changed; choose the updated source".into());
            }
        }
    }
    Ok(selected)
}

impl Selection {
    pub fn current_bounds(&self) -> Result<Rect, String> {
        let current_source = resolve(self.source)?;
        if self.window {
            let hwnd = self.handle as usize as HWND;
            if unsafe { IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 } {
                return Err("The captured window is hidden or minimized".into());
            }
            client_rect(hwnd).ok_or("Window client geometry is unavailable".into())
        } else {
            Ok(current_source.bounds)
        }
    }
    pub fn same_size(&self) -> Result<Rect, String> {
        let current = self.current_bounds()?;
        if (current.width, current.height) != (self.bounds.width, self.bounds.height) {
            return Err("Window size changed; select the updated source to resume control".into());
        }
        Ok(current)
    }
}

pub(crate) fn application_path(pid: u32) -> Option<String> {
    process(pid).map(|p| p.1)
}
