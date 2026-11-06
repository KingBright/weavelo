//! KWin's real window catalog and per-window PipeWire stream identity.
//!
//! Only Wayland/KWin is handled here. The compositor's screencast interface
//! remains behind its normal desktop-file allowlist; this module never changes
//! KWin policy or guesses a selected window from a title or rectangle.

use ::protocol::session::{CaptureSource, CaptureSourceInfo};
use std::{
    collections::{HashMap, HashSet},
    os::fd::AsRawFd,
    sync::{OnceLock, RwLock},
    time::Duration,
};
use tokio::sync::watch;
use wayland_client;
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry,
};
use zbus::zvariant::OwnedValue;
use zbus::{Connection as BusConnection, Proxy};

const SOURCE_ID_PREFIX: u32 = 0x4000_0000;
const SOURCE_ID_MASK: u32 = 0x3fff_ffff;
const DBUS_TIMEOUT: Duration = Duration::from_secs(3);
const CATALOG_TIMEOUT: Duration = Duration::from_secs(8);
const STREAM_TIMEOUT: Duration = Duration::from_secs(10);
const STREAM_STOP_POLL: i32 = 40;
const KWIN_BUS: &str = "org.kde.KWin";
const WINDOW_RUNNER_PATH: &str = "/WindowsRunner";
const WINDOW_RUNNER_IFACE: &str = "org.kde.krunner1";
const KWIN_PATH: &str = "/KWin";
const KWIN_IFACE: &str = "org.kde.KWin";
const KWIN_SCREENCAST_IFACE: &str = "zkde_screencast_unstable_v1";

mod kwin_wayland_bindings {
    #![allow(
        dead_code,
        non_camel_case_types,
        non_upper_case_globals,
        non_snake_case,
        unused_imports,
        unused_unsafe
    )]

    use wayland_client;
    use wayland_client::protocol::wl_output;
    use wayland_client::protocol::*;

    pub mod __interfaces {
        use wayland_backend;
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("src/zkde-screencast-unstable-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("src/zkde-screencast-unstable-v1.xml");
}

#[derive(Clone, Debug)]
pub(crate) struct WindowInfo {
    pub source: CaptureSourceInfo,
    pub uuid: String,
}

#[derive(Clone)]
struct CatalogEntry {
    info: WindowInfo,
    rgba: Option<RgbaIcon>,
}

#[derive(Clone)]
struct RgbaIcon {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

#[derive(Default)]
struct Catalog {
    by_source: HashMap<CaptureSource, CatalogEntry>,
    // Keep source IDs for the process lifetime, including while windows are
    // absent, so a later hash collision cannot retarget a still-selected ID.
    id_by_uuid: HashMap<String, u32>,
}

fn catalog() -> &'static RwLock<Catalog> {
    static CATALOG: OnceLock<RwLock<Catalog>> = OnceLock::new();
    CATALOG.get_or_init(|| RwLock::new(Catalog::default()))
}

pub(crate) fn is_kwin_window(source: CaptureSource) -> bool {
    matches!(source, CaptureSource::Window(id) if id & 0xc000_0000 == SOURCE_ID_PREFIX)
}

pub(crate) fn list() -> Result<Vec<CaptureSourceInfo>, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "KWin source discovery could not start")?;
    runtime.block_on(list_async())
}

async fn list_async() -> Result<Vec<CaptureSourceInfo>, String> {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        clear_catalog();
        return Err("KWin window capture requires a Wayland session".into());
    }
    let entries = match tokio::time::timeout(CATALOG_TIMEOUT, discover()).await {
        Ok(Ok(entries)) => entries,
        Ok(Err(error)) => {
            clear_catalog();
            return Err(error);
        }
        Err(_) => {
            clear_catalog();
            return Err("KWin window catalog timed out".into());
        }
    };
    let mut next = catalog().write().unwrap_or_else(|error| error.into_inner());
    let mut by_source = HashMap::with_capacity(entries.len());
    let mut sources = Vec::with_capacity(entries.len());
    for mut entry in entries {
        let Some(id) = source_id_for_uuid(&entry.info.uuid, &mut next.id_by_uuid) else {
            continue;
        };
        entry.info.source.source = CaptureSource::Window(id);
        sources.push(entry.info.source.clone());
        by_source.insert(entry.info.source.source, entry);
    }
    next.by_source = by_source;
    Ok(sources)
}

fn clear_catalog() {
    catalog()
        .write()
        .unwrap_or_else(|error| error.into_inner())
        .by_source
        .clear();
}

async fn resolve_async(source: CaptureSource) -> Result<WindowInfo, String> {
    if !is_kwin_window(source) {
        return Err("The selected Linux application window is no longer available".into());
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        clear_catalog();
        return Err("KWin window capture requires a Wayland session".into());
    }
    let window = catalog()
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .by_source
        .get(&source)
        .map(|entry| entry.info.clone())
        .ok_or_else(|| "The selected Linux application window is no longer available".to_owned())?;
    validate_window_alive(&window).await?;
    Ok(window)
}

async fn validate_window_alive(window: &WindowInfo) -> Result<(), String> {
    let connection = tokio::time::timeout(DBUS_TIMEOUT, BusConnection::session())
        .await
        .map_err(|_| "KWin window validation timed out")?
        .map_err(|_| "KWin window validation is unavailable")?;
    let kwin = Proxy::new_owned(
        connection,
        KWIN_BUS.to_owned(),
        KWIN_PATH.to_owned(),
        KWIN_IFACE.to_owned(),
    )
    .await
    .map_err(|_| "KWin window validation is unavailable")?;
    let info: HashMap<String, OwnedValue> = tokio::time::timeout(
        DBUS_TIMEOUT,
        kwin.call("getWindowInfo", &(window.uuid.as_str(),)),
    )
    .await
    .map_err(|_| "KWin selected window is no longer available")?
    .map_err(|_| "KWin selected window is no longer available")?;
    let uuid = value_string(&info, "uuid").unwrap_or_else(|| window.uuid.clone());
    if normalize_uuid(&uuid) != normalize_uuid(&window.uuid)
        || value_i32(&info, "pid") != window.source.process_id
    {
        return Err("KWin selected window identity changed".into());
    }
    Ok(())
}

pub(crate) async fn prepare_capture(
    source: CaptureSource,
) -> Result<crate::linux_pipewire::native::PreparedKwinCapture, String> {
    let info = resolve_async(source).await?;
    let uuid = info.uuid.clone();
    let (node, serial, stream) = tokio::task::spawn_blocking(move || start_window_stream(uuid))
        .await
        .map_err(|error| format!("KWin window stream startup task failed: {error}"))??;
    crate::linux_pipewire::native::prepare_kwin_window_capture(info, node, serial, stream).await
}

pub(crate) fn icon(
    source: CaptureSource,
    edge: u32,
) -> Option<::protocol::interaction::ApplicationIcon> {
    let entry = catalog()
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .by_source
        .get(&source)
        .cloned()?;
    let icon = entry.rgba?;
    if edge <= ::protocol::interaction::MAX_ICON_EDGE {
        remote_core::application_icons::encode_rgba(icon.width, icon.height, &icon.pixels).ok()
    } else {
        remote_core::application_icons::encode_artwork(icon.width, icon.height, &icon.pixels).ok()
    }
}

async fn discover() -> Result<Vec<CatalogEntry>, String> {
    if !tokio::task::spawn_blocking(screencast_available)
        .await
        .map_err(|_| "KWin window protocol probe failed")?
    {
        return Err("KWin native application-window capture is unavailable. Install the Weavelo KDE desktop entry and restart Weavelo.".into());
    }
    let connection = tokio::time::timeout(DBUS_TIMEOUT, BusConnection::session())
        .await
        .map_err(|_| "KWin window catalog timed out")?
        .map_err(|_| "KWin window catalog is unavailable")?;
    let runner = Proxy::new_owned(
        connection.clone(),
        KWIN_BUS.to_owned(),
        WINDOW_RUNNER_PATH.to_owned(),
        WINDOW_RUNNER_IFACE.to_owned(),
    )
    .await
    .map_err(|_| "KWin application-window catalog is unavailable")?;
    type Match = (
        String,
        String,
        String,
        i32,
        f64,
        HashMap<String, OwnedValue>,
    );
    let mut matches: Vec<Match> = tokio::time::timeout(DBUS_TIMEOUT, runner.call("Match", &("",)))
        .await
        .map_err(|_| "KWin application-window catalog timed out")?
        .map_err(|_| "KWin application-window catalog is unavailable")?;

    let kwin = Proxy::new_owned(
        connection,
        KWIN_BUS.to_owned(),
        KWIN_PATH.to_owned(),
        KWIN_IFACE.to_owned(),
    )
    .await
    .map_err(|_| "KWin window geometry is unavailable")?;
    matches.truncate(::protocol::interaction::MAX_APPLICATION_ICONS);
    let query_deadline = tokio::time::Instant::now() + CATALOG_TIMEOUT;
    let mut raw = Vec::new();
    let mut seen_uuid = HashSet::new();
    for (id, title, icon_name, _match_type, _relevance, properties) in matches {
        let remaining = query_deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let Some(match_uuid) = uuid_from_runner_id(&id) else {
            continue;
        };
        let info: HashMap<String, OwnedValue> = match tokio::time::timeout(
            remaining.min(DBUS_TIMEOUT),
            kwin.call("getWindowInfo", &(match_uuid.as_str(),)),
        )
        .await
        {
            Ok(Ok(info)) => info,
            _ => continue,
        };
        let Some(uuid) = value_string(&info, "uuid").or_else(|| Some(match_uuid.clone())) else {
            continue;
        };
        let normalized = normalize_uuid(&uuid);
        if normalized.is_empty() || !seen_uuid.insert(normalized.clone()) {
            continue;
        }
        let Some(pid) = value_i32(&info, "pid").filter(|pid| *pid > 0) else {
            continue;
        };
        if pid == std::process::id() as i32 {
            continue;
        }
        let Some(width) = value_dimension(&info, "width") else {
            continue;
        };
        let Some(height) = value_dimension(&info, "height") else {
            continue;
        };
        let caption = clean_label(&title)
            .or_else(|| value_string(&info, "caption").and_then(|value| clean_label(&value)));
        let Some(title) = caption else {
            continue;
        };
        let app_id = value_string(&info, "desktopFile")
            .or_else(|| value_string(&info, "appId"))
            .or_else(|| value_string(&info, "resourceClass"))
            .or_else(|| value_string(&info, "resourceName"))
            .unwrap_or_default();
        let application = application_name(&app_id);
        let icon = icon_rgba(properties.get("icon-data"))
            .or_else(|| named_application_icon(&icon_name, &app_id));
        raw.push((
            normalized,
            WindowInfo {
                source: CaptureSourceInfo {
                    source: CaptureSource::Window(0),
                    title,
                    application,
                    process_id: Some(pid),
                    width,
                    height,
                    // KWin's stream is scoped to one real UUID. Input remains
                    // disabled until an OS-authorized exact-window injector exists.
                    supports_input: false,
                    camera: None,
                },
                uuid,
            },
            icon,
        ));
    }
    raw.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(raw
        .into_iter()
        .map(|(_, info, rgba)| CatalogEntry { info, rgba })
        .collect())
}

fn screencast_available() -> bool {
    let Ok(connection) = Connection::connect_to_env() else {
        return false;
    };
    let Ok((globals, _queue)) = registry_queue_init::<RegistryState>(&connection) else {
        return false;
    };
    globals.contents().with_list(|globals| {
        globals
            .iter()
            .any(|global| global.interface == KWIN_SCREENCAST_IFACE && global.version >= 6)
    })
}

fn value_string(properties: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    properties
        .get(key)?
        .try_clone()
        .ok()
        .and_then(|value| String::try_from(value).ok())
}
fn value_i32(properties: &HashMap<String, OwnedValue>, key: &str) -> Option<i32> {
    properties
        .get(key)?
        .try_clone()
        .ok()
        .and_then(|value| i32::try_from(value).ok())
}

fn value_dimension(properties: &HashMap<String, OwnedValue>, key: &str) -> Option<u32> {
    let value = properties.get(key)?.try_clone().ok()?;
    let dimension = f64::try_from(value.try_clone().ok()?)
        .ok()
        .or_else(|| i32::try_from(value).ok().map(f64::from))?;
    let rounded = dimension.round();
    (rounded.is_finite() && (2.0..=f64::from(u32::MAX)).contains(&rounded))
        .then_some(rounded as u32)
}

fn uuid_from_runner_id(id: &str) -> Option<String> {
    let uuid = id.strip_prefix("0_")?.trim_matches(['{', '}']);
    let compact = normalize_uuid(uuid);
    (compact.len() == 32 && compact.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| uuid.to_owned())
}

fn normalize_uuid(uuid: &str) -> String {
    uuid.trim_matches(['{', '}'])
        .replace('-', "")
        .to_ascii_lowercase()
}

fn source_id(uuid: &str, used: &mut HashSet<u32>) -> Option<u32> {
    let mut hash = 0x811c_9dc5u32;
    for byte in uuid.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    source_id_from_hash(hash, used)
}

fn source_id_for_uuid(uuid: &str, ids_by_uuid: &mut HashMap<String, u32>) -> Option<u32> {
    let uuid = normalize_uuid(uuid);
    if let Some(id) = ids_by_uuid.get(&uuid) {
        return Some(*id);
    }
    let mut used = ids_by_uuid.values().copied().collect();
    let id = source_id(&uuid, &mut used)?;
    ids_by_uuid.insert(uuid, id);
    Some(id)
}

fn source_id_from_hash(hash: u32, used: &mut HashSet<u32>) -> Option<u32> {
    let base = hash & SOURCE_ID_MASK;
    for offset in 0..=SOURCE_ID_MASK {
        let id = SOURCE_ID_PREFIX | (base.wrapping_add(offset) & SOURCE_ID_MASK);
        if used.insert(id) {
            return Some(id);
        }
    }
    None
}

fn clean_label(text: &str) -> Option<String> {
    let value: String = text
        .chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect();
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn application_name(app_id: &str) -> String {
    if app_id.is_empty() {
        return String::new();
    }
    let desktop_id = app_id.trim();
    let candidates = desktop_paths(desktop_id);
    for path in candidates {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let mut in_desktop_entry = false;
        for line in content.lines() {
            if line.starts_with('[') {
                in_desktop_entry = line.trim() == "[Desktop Entry]";
            } else if in_desktop_entry
                && let Some(value) = line.strip_prefix("Name=")
                && let Some(value) = clean_label(value)
            {
                return value;
            }
        }
    }
    clean_label(desktop_id).unwrap_or_default()
}

fn desktop_paths(app_id: &str) -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os("XDG_DATA_HOME") {
        roots.push(std::path::PathBuf::from(home).join("applications"));
    } else if let Some(home) = std::env::var_os("HOME") {
        roots.push(std::path::PathBuf::from(home).join(".local/share/applications"));
    }
    if let Some(data_dirs) = std::env::var_os("XDG_DATA_DIRS") {
        roots.extend(std::env::split_paths(&data_dirs).map(|path| path.join("applications")));
    } else {
        roots.extend(
            ["/usr/local/share/applications", "/usr/share/applications"]
                .map(std::path::PathBuf::from),
        );
    }
    let desktop = std::path::Path::new(app_id);
    let mut paths = if desktop.is_absolute() {
        vec![desktop.to_owned()]
    } else {
        Vec::new()
    };
    let mut ids = vec![app_id.to_owned()];
    if !app_id.ends_with(".desktop") {
        ids.push(format!("{app_id}.desktop"));
    }
    paths.extend(
        roots
            .into_iter()
            .flat_map(|root| ids.iter().map(move |id| root.join(id))),
    );
    paths
}

fn desktop_icon_name(app_id: &str) -> Option<String> {
    for path in desktop_paths(app_id) {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let mut in_desktop_entry = false;
        for line in content.lines() {
            if line.starts_with('[') {
                in_desktop_entry = line.trim() == "[Desktop Entry]";
            } else if in_desktop_entry
                && let Some(value) = line.strip_prefix("Icon=")
                && let Some(value) = clean_icon_name(value.trim())
            {
                return Some(value);
            }
        }
    }
    None
}

fn clean_icon_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 512 || name.chars().any(char::is_control) {
        return None;
    }
    let path = std::path::Path::new(name);
    if path.is_absolute() {
        return Some(name.to_owned());
    }
    if name == "."
        || name == ".."
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.@".contains(&byte))
    {
        return None;
    }
    Some(name.to_owned())
}

fn named_application_icon(krunner_name: &str, app_id: &str) -> Option<RgbaIcon> {
    let desktop_name = desktop_icon_name(app_id);
    let names = [clean_icon_name(krunner_name), desktop_name];
    for name in names.into_iter().flatten() {
        for path in icon_paths(&name) {
            if let Some(icon) = load_gdk_pixbuf(&path) {
                return Some(icon);
            }
        }
    }
    None
}

fn icon_paths(name: &str) -> Vec<std::path::PathBuf> {
    let Some(name) = clean_icon_name(name) else {
        return Vec::new();
    };
    let direct = std::path::PathBuf::from(&name);
    if direct.is_absolute() {
        return vec![direct];
    }
    let file_names = ["png", "svg", "xpm"].map(|extension| format!("{name}.{extension}"));
    let mut paths = Vec::new();
    for (theme_root, directories) in icon_theme_roots() {
        for subdir in directories {
            for file in &file_names {
                paths.push(theme_root.join(subdir).join(file));
            }
        }
    }
    for data_dir in icon_data_dirs() {
        for file in &file_names {
            paths.push(data_dir.join("pixmaps").join(file));
        }
    }
    for root in ["/usr/local/share/pixmaps", "/usr/share/pixmaps"] {
        for file in &file_names {
            paths.push(std::path::PathBuf::from(root).join(file));
        }
    }
    paths
}

fn icon_theme_roots() -> &'static Vec<(std::path::PathBuf, Vec<std::path::PathBuf>)> {
    static ROOTS: OnceLock<Vec<(std::path::PathBuf, Vec<std::path::PathBuf>)>> = OnceLock::new();
    ROOTS.get_or_init(|| {
        let themes = icon_theme_names();
        icon_data_dirs()
            .into_iter()
            .flat_map(|data_dir| {
                let themes = themes.clone();
                themes.into_iter().map(move |theme| {
                    let root = data_dir.join("icons").join(theme);
                    let directories = theme_directories(&root);
                    (root, directories)
                })
            })
            .collect()
    })
}

fn icon_data_dirs() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
        paths.push(path.into());
    } else if let Some(home) = std::env::var_os("HOME") {
        paths.push(std::path::PathBuf::from(home).join(".local/share"));
    }
    if let Some(dirs) = std::env::var_os("XDG_DATA_DIRS") {
        paths.extend(std::env::split_paths(&dirs));
    } else {
        paths.extend(["/usr/local/share", "/usr/share"].map(std::path::PathBuf::from));
    }
    paths
}

fn icon_theme_names() -> Vec<String> {
    let mut themes = Vec::new();
    if let Some(theme) = std::env::var("GTK_THEME")
        .ok()
        .and_then(|value| value.split(':').next().map(str::to_owned))
    {
        themes.push(theme);
    }
    if let Some(home) = std::env::var_os("HOME") {
        for path in [
            std::path::PathBuf::from(&home).join(".config/kdeglobals"),
            std::path::PathBuf::from(&home).join(".config/gtk-3.0/settings.ini"),
        ] {
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            let mut in_icons = false;
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with('[') {
                    in_icons = line == "[Icons]" || line == "[Settings]";
                } else if in_icons {
                    if let Some(theme) = line
                        .strip_prefix("Theme=")
                        .or_else(|| line.strip_prefix("gtk-icon-theme-name="))
                    {
                        themes.push(theme.trim().to_owned());
                    }
                }
            }
        }
    }
    for fallback in ["breeze", "Adwaita", "hicolor"] {
        themes.push(fallback.to_owned());
    }
    let mut seen = HashSet::new();
    themes.retain(|theme| {
        !theme.is_empty()
            && theme.len() <= 128
            && theme
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
            && seen.insert(theme.clone())
    });
    themes
}

fn theme_directories(theme_root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut directories = Vec::new();
    if let Ok(content) = std::fs::read_to_string(theme_root.join("index.theme")) {
        let mut in_icon_theme = false;
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_icon_theme = line == "[Icon Theme]";
            } else if in_icon_theme {
                if let Some(value) = line
                    .strip_prefix("Directories=")
                    .or_else(|| line.strip_prefix("ScaledDirectories="))
                {
                    directories.extend(value.split(',').take(128).filter_map(safe_theme_directory));
                }
            }
        }
    }
    // Minimal standard locations also cover themes which omit index.theme.
    for relative in [
        "apps/256x256",
        "applications/256x256",
        "apps/128x128",
        "applications/128x128",
        "apps/64x64",
        "applications/64x64",
        "apps/48x48",
        "applications/48x48",
        "apps/scalable",
        "applications/scalable",
        "apps/32x32",
        "applications/32x32",
        "apps/24x24",
        "applications/24x24",
        "apps/22x22",
        "applications/22x22",
        "apps/16x16",
        "applications/16x16",
    ] {
        directories.push(relative.into());
    }
    let mut seen = HashSet::new();
    directories.retain(|directory| seen.insert(directory.clone()));
    directories
}

fn safe_theme_directory(directory: &str) -> Option<std::path::PathBuf> {
    let path = std::path::Path::new(directory);
    (!directory.is_empty()
        && directory.len() <= 256
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_))))
    .then(|| path.to_owned())
}

fn load_gdk_pixbuf(path: &std::path::Path) -> Option<RgbaIcon> {
    use std::os::unix::ffi::OsStrExt;
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 8 * 1024 * 1024 {
        return None;
    }
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    gdk_pixbuf::api()?.load(path.as_c_str())
}

#[cfg(target_os = "linux")]
mod gdk_pixbuf {
    use std::{
        ffi::{CStr, CString, c_char, c_int, c_uchar, c_void},
        sync::OnceLock,
    };

    type LoadFile =
        unsafe extern "C" fn(*const c_char, c_int, c_int, c_int, *mut *mut c_void) -> *mut c_void;
    type GetInt = unsafe extern "C" fn(*const c_void) -> c_int;
    type GetBool = unsafe extern "C" fn(*const c_void) -> c_int;
    type GetPixels = unsafe extern "C" fn(*const c_void) -> *const c_uchar;
    type Unref = unsafe extern "C" fn(*mut c_void);
    type ErrorFree = unsafe extern "C" fn(*mut c_void);

    struct Api {
        load_file: LoadFile,
        width: GetInt,
        height: GetInt,
        channels: GetInt,
        rowstride: GetInt,
        bits: GetInt,
        has_alpha: GetBool,
        pixels: GetPixels,
        object_unref: Unref,
        error_free: ErrorFree,
        _libraries: [usize; 3],
    }

    pub struct PixbufApi(Api);

    pub fn api() -> Option<&'static PixbufApi> {
        static API: OnceLock<Option<PixbufApi>> = OnceLock::new();
        API.get_or_init(load).as_ref()
    }

    impl PixbufApi {
        pub fn load(&self, path: &CStr) -> Option<super::RgbaIcon> {
            let api = &self.0;
            let mut error = std::ptr::null_mut();
            let pixbuf = unsafe { (api.load_file)(path.as_ptr(), 256, 256, 1, &mut error) };
            if !error.is_null() {
                unsafe { (api.error_free)(error) };
            }
            if pixbuf.is_null() {
                return None;
            }
            let result = unsafe {
                let width = (api.width)(pixbuf);
                let height = (api.height)(pixbuf);
                let channels = (api.channels)(pixbuf);
                let rowstride = (api.rowstride)(pixbuf);
                let bits = (api.bits)(pixbuf);
                let has_alpha = (api.has_alpha)(pixbuf) != 0;
                let source = (api.pixels)(pixbuf);
                if width <= 0
                    || height <= 0
                    || width > 256
                    || height > 256
                    || bits != 8
                    || !matches!(channels, 3 | 4)
                    || (has_alpha && channels != 4)
                    || rowstride < width.saturating_mul(channels)
                    || source.is_null()
                {
                    None
                } else {
                    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
                    for y in 0..height as usize {
                        let row = std::slice::from_raw_parts(
                            source.add(y * rowstride as usize),
                            width as usize * channels as usize,
                        );
                        for pixel in row.chunks_exact(channels as usize) {
                            pixels.extend_from_slice(&pixel[..3]);
                            pixels.push(if has_alpha { pixel[3] } else { 255 });
                        }
                    }
                    Some(super::RgbaIcon {
                        width: width as u32,
                        height: height as u32,
                        pixels,
                    })
                }
            };
            unsafe { (api.object_unref)(pixbuf) };
            result
        }
    }

    fn load() -> Option<PixbufApi> {
        unsafe {
            let gdk = open(&["libgdk_pixbuf-2.0.so.0", "libgdk_pixbuf-2.0.so"])?;
            let gobject = open(&["libgobject-2.0.so.0", "libgobject-2.0.so"])?;
            let glib = open(&["libglib-2.0.so.0", "libglib-2.0.so"])?;
            Some(PixbufApi(Api {
                load_file: symbol(gdk, "gdk_pixbuf_new_from_file_at_scale")?,
                width: symbol(gdk, "gdk_pixbuf_get_width")?,
                height: symbol(gdk, "gdk_pixbuf_get_height")?,
                channels: symbol(gdk, "gdk_pixbuf_get_n_channels")?,
                rowstride: symbol(gdk, "gdk_pixbuf_get_rowstride")?,
                bits: symbol(gdk, "gdk_pixbuf_get_bits_per_sample")?,
                has_alpha: symbol(gdk, "gdk_pixbuf_get_has_alpha")?,
                pixels: symbol(gdk, "gdk_pixbuf_read_pixels")?,
                object_unref: symbol(gobject, "g_object_unref")?,
                error_free: symbol(glib, "g_error_free")?,
                _libraries: [gdk as usize, gobject as usize, glib as usize],
            }))
        }
    }

    unsafe fn open(names: &[&str]) -> Option<*mut c_void> {
        for name in names {
            let name = CString::new(*name).ok()?;
            let handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL) };
            if !handle.is_null() {
                return Some(handle);
            }
        }
        None
    }

    unsafe fn symbol<T: Copy>(handle: *mut c_void, name: &str) -> Option<T> {
        let name = CString::new(name).ok()?;
        let value = unsafe { libc::dlsym(handle, name.as_ptr()) };
        (!value.is_null()).then(|| unsafe { std::mem::transmute_copy(&value) })
    }
}

fn icon_rgba(value: Option<&OwnedValue>) -> Option<RgbaIcon> {
    let value = value?.try_clone().ok()?;
    let (width, height, row_stride, has_alpha, bits, channels, data): (
        i32,
        i32,
        i32,
        bool,
        i32,
        i32,
        Vec<u8>,
    ) = value.try_into().ok()?;
    if width <= 0
        || height <= 0
        || width > 512
        || height > 512
        || bits != 8
        || !matches!(channels, 3 | 4)
        || row_stride < width.checked_mul(channels)?
        || (has_alpha && channels != 4)
    {
        return None;
    }
    let required = usize::try_from(row_stride)
        .ok()?
        .checked_mul(height as usize)?;
    if required > 2 * 1024 * 1024 || data.len() < required {
        return None;
    }
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height as usize {
        let row = &data[y * row_stride as usize..][..width as usize * channels as usize];
        for pixel in row.chunks_exact(channels as usize) {
            pixels.extend_from_slice(&pixel[..3]);
            pixels.push(if has_alpha { pixel[3] } else { 255 });
        }
    }
    Some(RgbaIcon {
        width: width as u32,
        height: height as u32,
        pixels,
    })
}

pub(crate) struct WindowStream {
    stop: watch::Sender<bool>,
    revoked: watch::Receiver<bool>,
    done: watch::Receiver<bool>,
    thread: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl WindowStream {
    pub(crate) fn revocation(&self) -> watch::Receiver<bool> {
        self.revoked.clone()
    }
    pub(crate) fn request_stop(&self) {
        self.stop.send_replace(true);
    }
    pub(crate) fn stop_signal(&self) -> watch::Sender<bool> {
        self.stop.clone()
    }
    pub(crate) async fn close_owned(&self) -> bool {
        self.request_stop();
        let mut done = self.done.clone();
        let stopped = if *done.borrow() {
            true
        } else {
            tokio::time::timeout(Duration::from_secs(2), async {
                while !*done.borrow_and_update() {
                    if done.changed().await.is_err() {
                        return false;
                    }
                }
                true
            })
            .await
            .unwrap_or(false)
        };
        if !stopped {
            return false;
        }
        let thread = self
            .thread
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        match thread {
            Some(thread) => tokio::task::spawn_blocking(move || thread.join().is_ok())
                .await
                .unwrap_or(false),
            None => true,
        }
    }
}
impl Drop for WindowStream {
    fn drop(&mut self) {
        self.request_stop();
        if let Some(thread) = self
            .thread
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            let _ = std::thread::Builder::new()
                .name("weavelo-kwin-reap".into())
                .spawn(move || {
                    let _ = thread.join();
                });
        }
    }
}

pub(crate) fn start_window_stream(uuid: String) -> Result<(u32, u64, WindowStream), String> {
    let (stop, stop_rx) = watch::channel(false);
    let (revoked_tx, revoked) = watch::channel(false);
    let (done_tx, done) = watch::channel(false);
    let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
    let thread = std::thread::Builder::new()
        .name("weavelo-kwin-window".into())
        .spawn(move || run_window_stream(uuid, stop_rx, revoked_tx, done_tx, started_tx))
        .map_err(|_| "KWin window stream could not start")?;
    let started = match started_rx.recv_timeout(STREAM_TIMEOUT) {
        Ok(Ok(started)) => started,
        Ok(Err(error)) => {
            stop.send_replace(true);
            let _ = thread.join();
            return Err(error);
        }
        Err(_) => {
            stop.send_replace(true);
            let _ = thread.join();
            return Err("KWin did not create the selected application-window stream".into());
        }
    };
    Ok((
        started.0,
        started.1,
        WindowStream {
            stop,
            revoked,
            done,
            thread: std::sync::Mutex::new(Some(thread)),
        },
    ))
}

fn run_window_stream(
    uuid: String,
    mut stop: watch::Receiver<bool>,
    revoked: watch::Sender<bool>,
    done: watch::Sender<bool>,
    started: std::sync::mpsc::SyncSender<Result<(u32, u64), String>>,
) {
    let result = (|| -> Result<(), String> {
        let connection = Connection::connect_to_env()
            .map_err(|_| "Wayland compositor connection is unavailable")?;
        let (globals, mut queue) = registry_queue_init::<StreamState>(&connection)
            .map_err(|_| "Wayland window capture is unavailable")?;
        let qh = queue.handle();
        let supported = globals.contents().with_list(|globals| {
            globals
                .iter()
                .any(|global| global.interface == KWIN_SCREENCAST_IFACE && global.version >= 6)
        });
        if !supported {
            return Err("KWin native application-window capture is unavailable. Install the Weavelo KDE desktop entry and restart Weavelo.".into());
        }
        let screencast: kwin_wayland_bindings::zkde_screencast_unstable_v1::ZkdeScreencastUnstableV1 = globals
            .bind(&qh, 6..=6, ())
            .map_err(|_| "KWin did not authorize Weavelo's declared window-capture interface")?;
        let stream = screencast.stream_window(
            uuid,
            kwin_wayland_bindings::zkde_screencast_unstable_v1::Pointer::Hidden,
            &qh,
            (),
        );
        connection
            .flush()
            .map_err(|_| "KWin rejected the application-window stream")?;
        let deadline = std::time::Instant::now() + STREAM_TIMEOUT;
        let mut state = StreamState::default();
        loop {
            if *stop.borrow() {
                return Err("KWin application-window stream was stopped".into());
            }
            dispatch_until(&mut queue, &mut state, &mut stop, deadline)?;
            if let Some(error) = state.failed.take() {
                return Err(error);
            }
            if state.closed {
                return Err("KWin closed the selected application-window stream".into());
            }
            if let (Some(node), Some(serial)) = (state.node, state.serial) {
                if node == 0 || serial == 0 {
                    return Err("KWin returned an invalid PipeWire window identity".into());
                }
                let _ = started.send(Ok((node, serial)));
                let ongoing_deadline =
                    std::time::Instant::now() + Duration::from_secs(24 * 60 * 60);
                loop {
                    if *stop.borrow() {
                        break;
                    }
                    dispatch_until(&mut queue, &mut state, &mut stop, ongoing_deadline)?;
                    if state.closed || state.failed.is_some() {
                        break;
                    }
                }
                stream.close();
                screencast.destroy();
                let _ = connection.flush();
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err("KWin did not create the selected application-window stream".into());
            }
        }
    })();
    if let Err(error) = result {
        let _ = started.send(Err(error));
        revoked.send_replace(true);
    } else if !*stop.borrow() {
        // The stream can report `closed` or `failed` after its initial node
        // and serial have been delivered. A normal return without a requested
        // stop is therefore a lost window, not a successful long-lived stream.
        revoked.send_replace(true);
    }
    done.send_replace(true);
}

fn dispatch_until(
    queue: &mut EventQueue<StreamState>,
    state: &mut StreamState,
    stop: &mut watch::Receiver<bool>,
    deadline: std::time::Instant,
) -> Result<(), String> {
    loop {
        queue
            .dispatch_pending(state)
            .map_err(|_| "KWin window stream dispatch failed")?;
        if *stop.borrow() || state.closed || state.failed.is_some() {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err("KWin window stream timed out".into());
        }
        let Some(read) = queue.prepare_read() else {
            continue;
        };
        queue
            .flush()
            .map_err(|_| "KWin window stream flush failed")?;
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let timeout_ms = remaining.as_millis().clamp(1, STREAM_STOP_POLL as u128) as i32;
        let mut descriptor = libc::pollfd {
            fd: read.connection_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
        if ready < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err("KWin window stream socket failed".into());
        }
        if ready == 0 {
            return Ok(());
        }
        read.read().map_err(|_| "KWin window stream read failed")?;
        queue
            .dispatch_pending(state)
            .map_err(|_| "KWin window stream dispatch failed")?;
        return Ok(());
    }
}

#[derive(Default)]
struct RegistryState;
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for RegistryState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

#[derive(Default)]
struct StreamState {
    node: Option<u32>,
    serial: Option<u64>,
    failed: Option<String>,
    closed: bool,
}
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for StreamState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl
    Dispatch<
        kwin_wayland_bindings::zkde_screencast_stream_unstable_v1::ZkdeScreencastStreamUnstableV1,
        (),
    > for StreamState
{
    fn event(
        state: &mut Self,
        _: &kwin_wayland_bindings::zkde_screencast_stream_unstable_v1::ZkdeScreencastStreamUnstableV1,
        event: kwin_wayland_bindings::zkde_screencast_stream_unstable_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use kwin_wayland_bindings::zkde_screencast_stream_unstable_v1::Event;
        match event {
            Event::Serial {
                object_serial_hi,
                object_serial_low,
            } => {
                state.serial =
                    Some((u64::from(object_serial_hi) << 32) | u64::from(object_serial_low));
            }
            Event::Created { node } => state.node = Some(node),
            Event::Failed { error } => {
                state.failed = Some(
                    clean_label(&error)
                        .unwrap_or_else(|| "KWin could not capture this application window".into()),
                )
            }
            Event::Closed => state.closed = true,
        }
    }
}

impl Dispatch<kwin_wayland_bindings::zkde_screencast_unstable_v1::ZkdeScreencastUnstableV1, ()>
    for StreamState
{
    fn event(
        _: &mut Self,
        _: &kwin_wayland_bindings::zkde_screencast_unstable_v1::ZkdeScreencastUnstableV1,
        _: kwin_wayland_bindings::zkde_screencast_unstable_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kwin_runner_ids_keep_exact_uuid_and_windows_have_a_separate_source_range() {
        let uuid = uuid_from_runner_id("0_{a47dfed3-918c-4895-bb8d-46feac94db30}").unwrap();
        assert_eq!(normalize_uuid(&uuid), "a47dfed3918c4895bb8d46feac94db30");
        assert!(uuid_from_runner_id("1_{a47dfed3-918c-4895-bb8d-46feac94db30}").is_none());
        assert!(uuid_from_runner_id("0_not-a-uuid").is_none());
        let mut ids = HashSet::new();
        let id = source_id("a47dfed3918c4895bb8d46feac94db30", &mut ids).unwrap();
        assert_eq!(id & 0xc000_0000, SOURCE_ID_PREFIX);
        assert!(is_kwin_window(CaptureSource::Window(id)));
        assert!(!is_kwin_window(CaptureSource::Window(42)));
    }

    #[test]
    fn icon_data_is_bounded_and_converted_from_rgb_or_rgba_rows() {
        let properties: HashMap<String, OwnedValue> = HashMap::new();
        assert!(icon_rgba(properties.get("icon-data")).is_none());
        let icon_data = OwnedValue::try_from(zbus::zvariant::Structure::from((
            1_i32,
            1_i32,
            8_i32,
            true,
            8_i32,
            4_i32,
            vec![255_u8, 0, 0, 128, 99, 99, 99, 99],
        )))
        .unwrap();
        let decoded = icon_rgba(Some(&icon_data)).unwrap();
        assert_eq!(decoded.width, 1);
        assert_eq!(decoded.height, 1);
        assert_eq!(decoded.pixels, [255, 0, 0, 128]);
        let pixels = RgbaIcon {
            width: 2,
            height: 1,
            pixels: vec![255, 0, 0, 255, 0, 255, 0, 255],
        };
        let icon = remote_core::application_icons::encode_rgba(
            pixels.width,
            pixels.height,
            &pixels.pixels,
        )
        .unwrap();
        assert!(::protocol::interaction::validate_icon_container(&icon.png).is_ok());
    }

    #[test]
    fn kwin_fractional_double_dimensions_are_rounded_and_bounded() {
        let mut properties = HashMap::new();
        properties.insert("width".into(), OwnedValue::from(2648.28_f64));
        properties.insert("height".into(), OwnedValue::from(1444.14_f64));
        assert_eq!(value_dimension(&properties, "width"), Some(2648));
        assert_eq!(value_dimension(&properties, "height"), Some(1444));
        properties.insert("width".into(), OwnedValue::from(3840_i32));
        assert_eq!(value_dimension(&properties, "width"), Some(3840));
        for invalid in [
            f64::NAN,
            f64::INFINITY,
            -2.0,
            1.4,
            f64::from(u32::MAX) + 1.0,
        ] {
            properties.insert("width".into(), OwnedValue::from(invalid));
            assert_eq!(value_dimension(&properties, "width"), None, "{invalid}");
        }
    }

    #[test]
    fn named_icon_values_are_confined_to_real_icon_names_or_absolute_files() {
        assert_eq!(
            clean_icon_name("org.example.Editor"),
            Some("org.example.Editor".into())
        );
        assert_eq!(
            clean_icon_name("/usr/share/pixmaps/editor.svg"),
            Some("/usr/share/pixmaps/editor.svg".into())
        );
        for invalid in ["", "..", "../editor", "icons/editor", "editor\nExec=run"] {
            assert!(clean_icon_name(invalid).is_none(), "{invalid:?}");
        }
        assert!(safe_theme_directory("apps/48x48").is_some());
        assert!(safe_theme_directory("../outside").is_none());
    }

    #[test]
    fn labels_are_clean_and_source_ids_remain_distinct() {
        assert_eq!(
            clean_label("  Browser\nWindow  ").as_deref(),
            Some("BrowserWindow")
        );
        let mut ids = HashSet::new();
        let first = source_id("first-window", &mut ids).unwrap();
        let second = source_id("second-window", &mut ids).unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn source_ids_remain_bound_after_a_collision_and_window_disappearance() {
        let mut used = HashSet::new();
        let first = source_id_from_hash(7, &mut used).unwrap();
        let second = source_id_from_hash(7, &mut used).unwrap();
        assert_ne!(first, second);

        let mut ids_by_uuid = HashMap::from([
            ("firstwindow".to_owned(), first),
            ("secondwindow".to_owned(), second),
        ]);
        assert_eq!(
            source_id_for_uuid("first-window", &mut ids_by_uuid),
            Some(first)
        );
        assert_eq!(
            source_id_for_uuid("second-window", &mut ids_by_uuid),
            Some(second)
        );
        assert_eq!(ids_by_uuid.len(), 2);
    }
}
