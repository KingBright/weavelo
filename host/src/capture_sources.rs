use protocol::session::{CaptureSource, CaptureSourceInfo};
use std::error::Error;

#[cfg(any(target_os = "macos", test))]
fn window_is_transformed(frame: (f64, f64), original: (f64, f64)) -> bool {
    frame.0 >= 2.0 && frame.1 >= 2.0
        && (frame.0 * 2.0 < original.0 || frame.1 * 2.0 < original.1)
}

#[cfg(any(target_os = "macos", test))]
fn mac_window_is_shareable(layer: i32, on_screen: bool) -> bool {
    // Keep ordinary windows on other Spaces and visible floating panels.
    // Hidden auxiliary panels (for example ChatGPT's layer-3 overlay) can
    // appear in SCK's directory without ever producing a captured frame.
    layer == 0 || on_screen
}

#[cfg(any(target_os = "macos", test))]
fn mac_display_catalog_title(index: usize, width: u32, height: u32, is_builtin: bool, is_main: bool) -> String {
    let base = if is_builtin {
        "Built-in Display".to_owned()
    } else if is_main {
        "Main Display".to_owned()
    } else {
        format!("Display {}", index + 1)
    };
    if width > 0 && height > 0 {
        format!("{base} ({width}×{height})")
    } else {
        base
    }
}

#[cfg(any(target_os = "macos", test))]
fn mac_window_catalog_title(title: Option<String>, application: &str, layer: i32, on_screen: bool, active: bool) -> Option<String> {
    if let Some(title) = title.filter(|title| !title.trim().is_empty()) {
        return Some(title);
    }
    // Some real application windows (including Gemini) intentionally have no
    // title. Their WindowServer ID remains the capture identity. Do not turn
    // unnamed hidden menu surfaces or floating helpers into application sources.
    (layer == 0 && (on_screen || active) && !application.trim().is_empty())
        .then(|| application.to_owned())
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn windows_display_catalog_title(
    index: usize,
    total: usize,
    adapter: &str,
    width: u32,
    height: u32,
) -> String {
    let adapter = adapter.trim();
    let base = if total <= 1 {
        "Main Display".to_owned()
    } else {
        format!("Display {}", index + 1)
    };
    match (!adapter.is_empty(), width > 0 && height > 0) {
        (true, true) => format!("{base} · {adapter} ({width}×{height})"),
        (false, true) => format!("{base} ({width}×{height})"),
        (true, false) => format!("{base} · {adapter}"),
        (false, false) => base,
    }
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn windows_shell_helper_process(application: &str) -> bool {
    matches!(
        application.trim().to_ascii_lowercase().as_str(),
        "textinputhost.exe"
            | "shellexperiencehost.exe"
            | "startmenuexperiencehost.exe"
            | "searchhost.exe"
            | "lockapp.exe"
            | "systemsettingsbroker.exe"
            | "widgetservice.exe"
            | "widgets.exe"
    )
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn filter_windows_catalog_entries<T>(
    windows: Vec<T>,
    meta: impl Fn(&T) -> (&str, &str),
) -> Vec<T> {
    let filtered: Vec<T> = windows
        .into_iter()
        .filter(|entry| {
            let (_, app) = meta(entry);
            !windows_shell_helper_process(app)
        })
        .collect();
    let real_titles: std::collections::HashSet<String> = filtered
        .iter()
        .filter_map(|entry| {
            let (title, app) = meta(entry);
            let trimmed = title.trim();
            (!trimmed.is_empty() && !app.trim().eq_ignore_ascii_case("applicationframehost.exe"))
                .then(|| trimmed.to_owned())
        })
        .collect();
    filtered
        .into_iter()
        .filter(|entry| {
            let (title, app) = meta(entry);
            !app.trim().eq_ignore_ascii_case("applicationframehost.exe")
                || !real_titles.contains(title.trim())
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn window_pixel_size(
    window: &screencapturekit::shareable_content::SCWindow,
    filter: &screencapturekit::prelude::SCContentFilter,
) -> (u32, u32) {
    // Stage Manager reports transformed thumbnail bounds in both SCWindow.frame
    // and SCContentFilter.contentRect. AXSize retains the original window size.
    // Only use a unique title match; never choose another same-titled window.
    let original = window.owning_application().and_then(|app| {
        window
            .title()
            .and_then(|title| accessible_window_size(app.process_id(), &title))
    });
    let rect = filter.content_rect();
    let size = original.unwrap_or_else(|| {
        let rect = if rect.size().width >= 2.0 {
            rect
        } else {
            window.frame()
        };
        (rect.size().width, rect.size().height)
    });
    let scale = f64::from(filter.point_pixel_scale()).clamp(1.0, 4.0);
    ((size.0 * scale) as u32, (size.1 * scale) as u32)
}

#[cfg(target_os = "macos")]
fn accessible_window_size(pid: i32, title: &str) -> Option<(f64, f64)> {
    use core_foundation::{
        array::CFArray,
        base::{CFType, CFTypeRef, TCFType},
        string::CFString,
    };
    use core_graphics::geometry::CGSize;
    use std::ffi::c_void;
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
        fn AXUIElementCopyAttributeValue(
            element: CFTypeRef,
            attribute: CFTypeRef,
            value: *mut CFTypeRef,
        ) -> i32;
        fn AXUIElementSetMessagingTimeout(element: CFTypeRef, timeout: f32) -> i32;
        fn AXValueGetType(value: CFTypeRef) -> u32;
        fn AXValueGetValue(value: CFTypeRef, kind: u32, output: *mut c_void) -> bool;
    }
    fn attribute(element: CFTypeRef, name: &str) -> Option<CFType> {
        let mut value = std::ptr::null();
        let name = CFString::new(name);
        // Copy follows the CF ownership rule; errors (including no permission)
        // simply leave capture on ScreenCaptureKit's geometry.
        unsafe {
            if AXUIElementCopyAttributeValue(element, name.as_CFTypeRef(), &mut value) != 0
                || value.is_null()
            {
                return None;
            }
            Some(CFType::wrap_under_create_rule(value))
        }
    }
    unsafe {
        let raw = AXUIElementCreateApplication(pid);
        if raw.is_null() {
            return None;
        }
        let app = CFType::wrap_under_create_rule(raw);
        AXUIElementSetMessagingTimeout(app.as_CFTypeRef(), 0.05);
        let windows = attribute(app.as_CFTypeRef(), "AXWindows")?.downcast::<CFArray>()?;
        let mut matched = None;
        for window in windows.iter() {
            let Some(name) =
                attribute(*window, "AXTitle").and_then(|value| value.downcast::<CFString>())
            else {
                continue;
            };
            if name != title {
                continue;
            }
            if matched.is_some() {
                return None;
            }
            let value = attribute(*window, "AXSize")?;
            if AXValueGetType(value.as_CFTypeRef()) != 2 {
                return None;
            }
            let mut size = CGSize::new(0.0, 0.0);
            if !AXValueGetValue(value.as_CFTypeRef(), 2, (&mut size as *mut CGSize).cast()) {
                return None;
            }
            if size.width < 2.0 || size.height < 2.0 {
                return None;
            }
            matched = Some((size.width, size.height));
        }
        matched
    }
}

pub fn list() -> Result<Vec<CaptureSourceInfo>, Box<dyn Error + Send + Sync>> {
    let screens=list_screens();
    #[cfg(any(target_os="macos",target_os="linux",target_os="windows"))]
    {
        let cameras=crate::cameras::list();
        return match (screens,cameras) {
            (Ok(mut screens),Ok(cameras))=>{screens.extend(cameras);Ok(screens)},
            (Err(_),Ok(cameras)) if !cameras.is_empty()=>Ok(cameras),
            (Ok(screens),Err(_))=>Ok(screens),
            (Err(error),_)=>Err(error),
        };
    }
    #[cfg(not(any(target_os="macos",target_os="linux",target_os="windows")))]
    screens
}

fn list_screens() -> Result<Vec<CaptureSourceInfo>, Box<dyn Error + Send + Sync>> {
    #[cfg(target_os = "windows")]
    {
        crate::capture_readiness::ffmpeg_program()?;
        return crate::windows_sources::list().map_err(Into::into);
    }
    #[cfg(target_os = "macos")]
    {
        use screencapturekit::prelude::*;
        let content = SCShareableContent::get()?;
        let native_captions = crate::window_input::native_frame_caption_ids()?;
        let main_display_id = core_graphics::display::CGDisplay::main().id;
        let mut sources = Vec::new();
        for (index, display) in content.displays().into_iter().enumerate() {
            let display_id = display.display_id();
            let is_builtin = core_graphics::display::CGDisplay::new(display_id).is_builtin();
            let is_main = display_id == main_display_id;
            let width = display.width();
            let height = display.height();
            sources.push(CaptureSourceInfo {
                source: CaptureSource::Display(display_id),
                title: mac_display_catalog_title(index, width, height, is_builtin, is_main),
                application: String::new(),
                process_id: None,
                width,
                height,
                // Capability only: subscription/injection separately check the
                // actual running host's input-posting permission.
                supports_input: crate::input_injector::display_input_capable(display_id),
                camera: None,
            });
        }
        for window in content.windows() {
            if native_captions.contains(&window.window_id())
                || !mac_window_is_shareable(window.window_layer(), window.is_on_screen()) {
                continue;
            }
            let Some(app) = window.owning_application() else {
                continue;
            };
            // Stage Manager's icon surfaces are compositor internals, not app windows.
            // Skip them before AX geometry queries and native artwork extraction.
            if app.process_id() == std::process::id() as i32
                || app.bundle_identifier() == "com.apple.WindowManager" {
                continue;
            }
            let Some(title) = mac_window_catalog_title(
                window.title(), &app.application_name(), window.window_layer(), window.is_on_screen(), window.is_active(),
            ) else {
                continue;
            };
            let frame = window.frame();
            if frame.size().width < 2.0 || frame.size().height < 2.0 {
                continue;
            }
            let independent = SCContentFilter::create().with_window(&window).build();
            let size = window_pixel_size(&window, &independent);
            sources.push(CaptureSourceInfo {
                source: CaptureSource::Window(window.window_id()),
                title,
                application: app.application_name(),
                process_id: Some(app.process_id()),
                width: size.0,
                height: size.1,
                // Geometry/owner capability, not permission. The selected PID
                // is bound by the host; readiness requires actual OS grants.
                supports_input: app.process_id() > 0,
                camera: None,
            });
        }
        return Ok(sources);
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let desktop = CaptureSourceInfo {
        source: CaptureSource::MainDisplay,
        title: "Desktop".into(),
        application: String::new(),
        process_id: None,
        width: 0,
        height: 0,
        supports_input: {
            #[cfg(target_os = "linux")]
            { crate::linux_capture_geometry::require_current_x11_desktop().is_ok() }
            #[cfg(not(target_os = "linux"))]
            { true }
        },
        camera: None,
    };
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return Ok(Vec::new());
        }
        let mut sources = vec![desktop];
        match crate::linux_kwin::list() {
            Ok(windows) => sources.extend(windows),
            Err(error) => eprintln!("KWin window catalog unavailable: {error}"),
        }
        Ok(sources)
    }
    #[cfg(all(
        not(target_os = "macos"),
        not(target_os = "windows"),
        not(target_os = "linux")
    ))]
    Ok(vec![desktop])
}

#[cfg(target_os = "macos")]
pub(crate) struct CapturePlan {
    pub filter: screencapturekit::prelude::SCContentFilter,
    pub stream_size: (u32, u32),
    pub window_crop: Option<protocol::viewport::PixelRect>,
    pub includes_children: bool,
    pub family_display_mode: bool,
}

#[cfg(target_os = "macos")]
pub(crate) fn filter(
    source: CaptureSource,
) -> Result<(screencapturekit::prelude::SCContentFilter, (u32, u32)), Box<dyn Error + Send + Sync>>
{
    let plan = capture_plan(source)?;
    let logical_pixels = plan
        .window_crop
        .map_or(plan.stream_size, |c| (c.width, c.height));
    Ok((plan.filter, logical_pixels))
}

#[cfg(target_os = "macos")]
pub(crate) fn capture_plan(
    source: CaptureSource,
) -> Result<CapturePlan, Box<dyn Error + Send + Sync>> {
    use screencapturekit::prelude::*;
    let content = SCShareableContent::get()?;
    match source {
        CaptureSource::Camera(_) => Err("Camera source requires native camera capture".into()),
        CaptureSource::Window(id) => {
            let all_windows = content.windows();
            let window = all_windows
                .iter()
                .find(|window| window.window_id() == id)
                .ok_or("selected window is no longer available")?;
            if !mac_window_is_shareable(window.window_layer(), window.is_on_screen()) {
                return Err("Selected window is a hidden auxiliary panel. Choose an open application window.".into());
            }
            let includes_children = !crate::window_input::window_has_parked_fullscreen_titlebar(id);
            if let Some(app) = window.owning_application()
                && let Some(overlay) = crate::window_input::active_detached_overlay(id, app.process_id())
            {
                let cx = overlay.union_bounds.origin.x + overlay.union_bounds.size.width * 0.5;
                let cy = overlay.union_bounds.origin.y + overlay.union_bounds.size.height * 0.5;
                for display in content.displays() {
                    let probe = SCContentFilter::create().with_display(&display).build();
                    let drect = probe.content_rect();
                    let (dx, dy, dw, dh) = (
                        drect.origin().x,
                        drect.origin().y,
                        drect.size().width,
                        drect.size().height,
                    );
                    if dw < 2.0 || dh < 2.0 || cx < dx || cx >= dx + dw || cy < dy || cy >= dy + dh {
                        continue;
                    }
                    let x0 = overlay.union_bounds.origin.x.max(dx);
                    let y0 = overlay.union_bounds.origin.y.max(dy);
                    let x1 = (overlay.union_bounds.origin.x + overlay.union_bounds.size.width).min(dx + dw);
                    let y1 = (overlay.union_bounds.origin.y + overlay.union_bounds.size.height).min(dy + dh);
                    if x1 - x0 >= 4.0 && y1 - y0 >= 4.0 {
                        let excluded_refs: Vec<_> = all_windows
                            .iter()
                            .filter(|w| overlay.excluded_window_ids.contains(&w.window_id()))
                            .collect();
                        let fam_filter = SCContentFilter::create()
                            .with_display(&display)
                            .with_including_applications(&[&app], &excluded_refs)
                            .build();
                        let scale = f64::from(fam_filter.point_pixel_scale()).clamp(1.0, 4.0);
                        let disp_px_w = (((dw * scale).round() as u32) & !1).max(2);
                        let disp_px_h = (((dh * scale).round() as u32) & !1).max(2);
                        let crop_x = (((x0 - dx) * scale).floor().max(0.0) as u32) & !1;
                        let crop_y = (((y0 - dy) * scale).floor().max(0.0) as u32) & !1;
                        let max_w = disp_px_w.saturating_sub(crop_x) & !1;
                        let max_h = disp_px_h.saturating_sub(crop_y) & !1;
                        let crop_w = ((((x1 - x0) * scale).round() as u32) & !1).clamp(2, max_w.max(2));
                        let crop_h = ((((y1 - y0) * scale).round() as u32) & !1).clamp(2, max_h.max(2));
                        if crop_x + crop_w <= disp_px_w && crop_y + crop_h <= disp_px_h {
                            return Ok(CapturePlan {
                                filter: fam_filter,
                                stream_size: (disp_px_w, disp_px_h),
                                window_crop: Some(protocol::viewport::PixelRect {
                                    x: crop_x,
                                    y: crop_y,
                                    width: crop_w,
                                    height: crop_h,
                                }),
                                includes_children,
                                family_display_mode: true,
                            });
                        }
                    }
                    break;
                }
            }
            let filter = SCContentFilter::create().with_window(window).build();
            let size = window_pixel_size(window, &filter);
            let scale = f64::from(filter.point_pixel_scale()).clamp(1.0, 4.0);
            let frame = window.frame().size();
            // Compare point sizes, never Retina pixels with logical bounds.
            // A plain off-screen or covered window needs no activation.
            if window_is_transformed((frame.width, frame.height), (f64::from(size.0) / scale, f64::from(size.1) / scale)) {
                let app = window.owning_application().ok_or("The selected window's application is unavailable")?;
                let title = window.title().ok_or("The selected window has no stable title")?;
                crate::window_input::WindowTarget::new(id, app.process_id())?.restore_for_capture(&title)?;
            }
            Ok(CapturePlan {
                filter,
                stream_size: size,
                window_crop: None,
                includes_children,
                family_display_mode: false,
            })
        }
        CaptureSource::MainDisplay | CaptureSource::Display(_) => {
            let id = match source {
                CaptureSource::Display(id) => id,
                _ => core_graphics::display::CGDisplay::main().id,
            };
            let display = content
                .displays()
                .into_iter()
                .find(|display| display.display_id() == id)
                .ok_or("selected display is no longer available")?;
            Ok(CapturePlan {
                filter: SCContentFilter::create().with_display(&display).build(),
                stream_size: (display.width(), display.height()),
                window_crop: None,
                includes_children: false,
                family_display_mode: false,
            })
        }
    }
}

#[cfg(test)]
mod transformed_window_tests {
    use super::{
        filter_windows_catalog_entries, mac_display_catalog_title, mac_window_catalog_title,
        mac_window_is_shareable, window_is_transformed, windows_display_catalog_title,
    };
    #[test]
    fn windows_catalog_helpers_format_displays_and_remove_ghost_shell_windows() {
        assert_eq!(
            windows_display_catalog_title(0, 1, "NVIDIA GeForce RTX 4090", 3840, 2160),
            "Main Display · NVIDIA GeForce RTX 4090 (3840×2160)"
        );
        assert_eq!(
            windows_display_catalog_title(1, 2, "NVIDIA GeForce RTX 4090", 1920, 1080),
            "Display 2 · NVIDIA GeForce RTX 4090 (1920×1080)"
        );
        let entries = vec![
            ("Windows 输入体验", "TextInputHost.exe"),
            ("设置", "SystemSettings.exe"),
            ("设置", "ApplicationFrameHost.exe"),
            ("Calculator", "ApplicationFrameHost.exe"),
            ("Visual Studio Code", "Code.exe"),
        ];
        let kept = filter_windows_catalog_entries(entries, |(title, app)| (*title, *app));
        assert_eq!(
            kept,
            vec![
                ("设置", "SystemSettings.exe"),
                ("Calculator", "ApplicationFrameHost.exe"),
                ("Visual Studio Code", "Code.exe"),
            ]
        );
    }
    #[test]
    fn mac_display_titles_are_human_friendly_instead_of_raw_cg_ids() {
        assert_eq!(mac_display_catalog_title(0, 1728, 1117, true, true), "Built-in Display (1728×1117)");
        assert_eq!(mac_display_catalog_title(0, 2560, 1440, false, true), "Main Display (2560×1440)");
        assert_eq!(mac_display_catalog_title(1, 1920, 1080, false, false), "Display 2 (1920×1080)");
    }
    #[test]
    fn untitled_visible_application_windows_keep_their_application_name() {
        assert_eq!(mac_window_catalog_title(None, "Gemini", 0, true, true).as_deref(), Some("Gemini"));
        assert_eq!(mac_window_catalog_title(Some("  ".into()), "Gemini", 0, true, true).as_deref(), Some("Gemini"));
        assert_eq!(mac_window_catalog_title(Some("Document".into()), "Editor", 0, false, false).as_deref(), Some("Document"));
        assert_eq!(mac_window_catalog_title(None, "Gemini", 0, false, true).as_deref(), Some("Gemini"), "Stage Manager may park an active window off screen");
        assert_eq!(mac_window_catalog_title(None, "Gemini", 0, false, false), None);
        assert_eq!(mac_window_catalog_title(None, "Gemini", 3, true, true), None);
        assert_eq!(mac_window_catalog_title(None, "", 0, true, true), None);
    }
    #[test]
    fn hidden_auxiliary_panels_are_not_capture_sources() {
        assert!(!mac_window_is_shareable(3, false));
        assert!(!mac_window_is_shareable(103, false));
        assert!(mac_window_is_shareable(0, true));
        assert!(mac_window_is_shareable(0, false), "Other Spaces remain available");
        assert!(mac_window_is_shareable(3, true), "Visible floating panels remain available");
    }
    #[test]
    fn stage_manager_thumbnail_differs_from_normal_and_retina_window_points() {
        assert!(window_is_transformed((69.0, 110.0), (640.0, 632.0)));
        assert!(!window_is_transformed((640.0, 632.0), (1280.0 / 2.0, 1264.0 / 2.0)));
        assert!(!window_is_transformed((69.0, 110.0), (69.0, 110.0)));
        assert!(!window_is_transformed((0.0, 0.0), (640.0, 632.0)));
    }
}
/// The same serialized native query serves local selection and remote catalogs.
pub async fn local_catalog() -> Result<Vec<protocol::session::CaptureSourceInfo>, String> {
    let directory=crate::source_catalog::enumerate_native(|| {
        list().map(|mut sources| {
            sources.truncate(protocol::interaction::MAX_APPLICATION_ICONS);
            crate::source_artwork::Directory {sources,..Default::default()}
        }).map_err(|error|error.to_string())
    }).await?;
    Ok(directory.sources)
}

/// Native icons are loaded in the frontend worker, never sent as JSON pixels
/// through the background control endpoint or fetched on a video/input tick.
pub fn with_artwork(sources: Vec<protocol::session::CaptureSourceInfo>) -> protocol::session::SessionCommand {
    let directory=crate::source_artwork::Directory::new(sources,&[]);
    let sources=directory.sources.clone();
    directory.reply(0,sources)
}
