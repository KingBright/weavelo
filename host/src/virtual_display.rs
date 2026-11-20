//! Cross-platform Host Virtual Display Manager.
//!
//! Manages virtual displays created for headless or multi-monitor remote
//! sessions, enforcing resolution/refresh/DPI validation and a maximum active
//! display cap ([`MAX_VIRTUAL_DISPLAYS`]). Provides a platform-agnostic
//! [`VirtualDisplayBackend`] trait with macOS (`CGVirtualDisplay`), Windows
//! (VDD / IddSampleDriver), and Linux (KWin / PipeWire `CreateVirtualMonitor`)
//! hooks, a deterministic [`MockVirtualDisplayBackend`] for tests, per-session
//! disconnect teardown that preserves persistent displays, and an RAII
//! [`VirtualDisplayGuard`] that cleans up ephemeral displays on [`Drop`].

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Minimum permitted virtual display width in pixels.
pub const MIN_VIRTUAL_DISPLAY_WIDTH: u32 = 320;
/// Maximum permitted virtual display width in pixels (8K UHD).
pub const MAX_VIRTUAL_DISPLAY_WIDTH: u32 = 7680;

/// Minimum permitted virtual display height in pixels.
pub const MIN_VIRTUAL_DISPLAY_HEIGHT: u32 = 240;
/// Maximum permitted virtual display height in pixels (8K UHD).
pub const MAX_VIRTUAL_DISPLAY_HEIGHT: u32 = 4320;

/// Minimum permitted virtual display refresh rate in Hz.
pub const MIN_VIRTUAL_DISPLAY_REFRESH_HZ: u32 = 24;
/// Maximum permitted virtual display refresh rate in Hz.
pub const MAX_VIRTUAL_DISPLAY_REFRESH_HZ: u32 = 240;

/// Minimum permitted DPI scale factor.
pub const MIN_VIRTUAL_DISPLAY_DPI_SCALE: f32 = 0.5;
/// Maximum permitted DPI scale factor.
pub const MAX_VIRTUAL_DISPLAY_DPI_SCALE: f32 = 4.0;

/// Maximum number of concurrently active virtual displays per host.
pub const MAX_VIRTUAL_DISPLAYS: usize = 4;

/// Maximum UTF-8 byte length for a virtual display name.
pub const MAX_VIRTUAL_DISPLAY_NAME_LEN: usize = 128;

/// Request describing a virtual display to create or reconfigure.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VirtualDisplayRequest {
    /// Human-readable display label (e.g. `"Weavelo Virtual 1"`).
    pub name: String,
    /// Horizontal resolution in pixels (`320..=7680`).
    pub width: u32,
    /// Vertical resolution in pixels (`240..=4320`).
    pub height: u32,
    /// Refresh rate in Hz (`24..=240`).
    pub refresh_hz: u32,
    /// DPI scale factor (`0.5..=4.0`).
    pub dpi_scale: f32,
    /// Whether HDR output should be enabled on the virtual display.
    #[serde(default)]
    pub hdr: bool,
    /// If `true`, the display survives session disconnect and RAII guard drop.
    #[serde(default)]
    pub persist_after_disconnect: bool,
}

impl Default for VirtualDisplayRequest {
    fn default() -> Self {
        Self {
            name: "Weavelo Virtual Display".to_string(),
            width: 1920,
            height: 1080,
            refresh_hz: 60,
            dpi_scale: 1.0,
            hdr: false,
            persist_after_disconnect: false,
        }
    }
}

impl VirtualDisplayRequest {
    /// Validate display name, dimensions, refresh rate, and DPI scale bounds.
    pub fn validate(&self) -> Result<(), VirtualDisplayError> {
        let trimmed = self.name.trim();
        if trimmed.is_empty() {
            return Err(VirtualDisplayError::InvalidRequest(
                "virtual display name must not be empty".to_string(),
            ));
        }
        if self.name.len() > MAX_VIRTUAL_DISPLAY_NAME_LEN {
            return Err(VirtualDisplayError::InvalidRequest(format!(
                "virtual display name exceeds {MAX_VIRTUAL_DISPLAY_NAME_LEN} bytes"
            )));
        }
        if self.name.contains('\0') {
            return Err(VirtualDisplayError::InvalidRequest(
                "virtual display name must not contain NUL bytes".to_string(),
            ));
        }
        if !(MIN_VIRTUAL_DISPLAY_WIDTH..=MAX_VIRTUAL_DISPLAY_WIDTH).contains(&self.width) {
            return Err(VirtualDisplayError::InvalidRequest(format!(
                "width ({}) must be within {}..={}",
                self.width, MIN_VIRTUAL_DISPLAY_WIDTH, MAX_VIRTUAL_DISPLAY_WIDTH
            )));
        }
        if !(MIN_VIRTUAL_DISPLAY_HEIGHT..=MAX_VIRTUAL_DISPLAY_HEIGHT).contains(&self.height) {
            return Err(VirtualDisplayError::InvalidRequest(format!(
                "height ({}) must be within {}..={}",
                self.height, MIN_VIRTUAL_DISPLAY_HEIGHT, MAX_VIRTUAL_DISPLAY_HEIGHT
            )));
        }
        if !(MIN_VIRTUAL_DISPLAY_REFRESH_HZ..=MAX_VIRTUAL_DISPLAY_REFRESH_HZ)
            .contains(&self.refresh_hz)
        {
            return Err(VirtualDisplayError::InvalidRequest(format!(
                "refresh_hz ({}) must be within {}..={}",
                self.refresh_hz, MIN_VIRTUAL_DISPLAY_REFRESH_HZ, MAX_VIRTUAL_DISPLAY_REFRESH_HZ
            )));
        }
        if !self.dpi_scale.is_finite()
            || !(MIN_VIRTUAL_DISPLAY_DPI_SCALE..=MAX_VIRTUAL_DISPLAY_DPI_SCALE)
                .contains(&self.dpi_scale)
        {
            return Err(VirtualDisplayError::InvalidRequest(format!(
                "dpi_scale ({}) must be a finite value within {}..={}",
                self.dpi_scale, MIN_VIRTUAL_DISPLAY_DPI_SCALE, MAX_VIRTUAL_DISPLAY_DPI_SCALE
            )));
        }
        Ok(())
    }
}

/// Active virtual display tracked by [`VirtualDisplayManager`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ManagedVirtualDisplay {
    /// Manager-assigned logical identifier (e.g. `"vdisp-1"`).
    pub id: String,
    /// Platform/OS-assigned display identifier (e.g. CGDirectDisplayID, `\\.\DISPLAYx`, PipeWire node).
    pub os_display_id: String,
    /// Current display configuration.
    pub request: VirtualDisplayRequest,
    /// Optional remote session identifier that owns this display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_session_id: Option<String>,
    /// Creation timestamp in Unix epoch milliseconds.
    pub created_at: u64,
}

/// Feature and platform capability summary reported by a [`VirtualDisplayBackend`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualDisplayCapabilities {
    pub platform: String,
    pub supported: bool,
    pub supports_dynamic_resize: bool,
    pub supports_hdr: bool,
    pub supports_dpi_scale: bool,
    pub supports_persistence: bool,
    pub max_displays: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver_hint: Option<String>,
}

/// Errors produced by virtual display validation, capacity checks, or platform backends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VirtualDisplayError {
    /// Request failed validation invariants.
    InvalidRequest(String),
    /// Active virtual display limit was reached.
    MaxDisplaysReached { limit: usize },
    /// Referenced virtual display ID does not exist.
    NotFound(String),
    /// Platform backend failed to create, update, or destroy the display.
    Backend(String),
    /// Virtual display backend is not available or not supported on this host.
    UnsupportedPlatform(String),
}

impl fmt::Display for VirtualDisplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(msg) => write!(f, "invalid virtual display request: {msg}"),
            Self::MaxDisplaysReached { limit } => {
                write!(f, "maximum number of virtual displays reached ({limit})")
            }
            Self::NotFound(id) => write!(f, "virtual display '{id}' not found"),
            Self::Backend(msg) => write!(f, "virtual display backend error: {msg}"),
            Self::UnsupportedPlatform(msg) => {
                write!(f, "virtual display unsupported on this platform: {msg}")
            }
        }
    }
}

impl std::error::Error for VirtualDisplayError {}

/// Platform abstraction for creating, updating, and destroying OS virtual displays.
pub trait VirtualDisplayBackend: Send + Sync {
    /// Create a new virtual display and return its OS-level display identifier.
    fn create_display(
        &self,
        request: &VirtualDisplayRequest,
    ) -> Result<String, VirtualDisplayError>;

    /// Reconfigure an existing virtual display identified by `os_display_id`.
    fn update_display(
        &self,
        os_display_id: &str,
        request: &VirtualDisplayRequest,
    ) -> Result<(), VirtualDisplayError>;

    /// Destroy an existing virtual display identified by `os_display_id`.
    fn destroy_display(&self, os_display_id: &str) -> Result<(), VirtualDisplayError>;

    /// Report backend capabilities and platform support metadata.
    fn capabilities(&self) -> VirtualDisplayCapabilities;
}

/// Thread-safe manager tracking virtual displays, per-session ownership, and lifecycle cleanup.
pub struct VirtualDisplayManager {
    backend: Arc<dyn VirtualDisplayBackend>,
    displays: Mutex<BTreeMap<String, ManagedVirtualDisplay>>,
    next_id: AtomicU64,
}

impl VirtualDisplayManager {
    /// Create a manager backed by the current host OS implementation.
    pub fn with_platform_backend() -> Arc<Self> {
        Arc::new(Self::new(default_platform_backend()))
    }

    /// Create a manager with a custom [`VirtualDisplayBackend`] (e.g. [`MockVirtualDisplayBackend`]).
    pub fn new(backend: Arc<dyn VirtualDisplayBackend>) -> Self {
        Self {
            backend,
            displays: Mutex::new(BTreeMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Return the underlying backend's capabilities.
    pub fn capabilities(&self) -> VirtualDisplayCapabilities {
        self.backend.capabilities()
    }

    /// Effective maximum number of concurrent virtual displays enforced by this manager.
    pub fn max_displays(&self) -> usize {
        self.backend
            .capabilities()
            .max_displays
            .min(MAX_VIRTUAL_DISPLAYS)
    }

    /// Create a new virtual display owned optionally by `owner_session_id`.
    pub fn create_display(
        &self,
        owner_session_id: Option<String>,
        request: VirtualDisplayRequest,
    ) -> Result<ManagedVirtualDisplay, VirtualDisplayError> {
        request.validate()?;

        let mut displays = self.displays.lock().unwrap_or_else(|e| e.into_inner());
        let limit = self.max_displays();
        if displays.len() >= limit {
            return Err(VirtualDisplayError::MaxDisplaysReached { limit });
        }

        let os_display_id = self.backend.create_display(&request)?;
        let seq = self.next_id.fetch_add(1, Ordering::SeqCst);
        let id = format!("vdisp-{seq}");
        let managed = ManagedVirtualDisplay {
            id: id.clone(),
            os_display_id,
            request,
            owner_session_id,
            created_at: now_unix_ms(),
        };
        displays.insert(id, managed.clone());
        Ok(managed)
    }

    /// Create a virtual display and wrap it in an RAII [`VirtualDisplayGuard`].
    ///
    /// Ephemeral displays (`persist_after_disconnect == false`) are destroyed
    /// automatically when the guard is dropped; persistent displays survive drop.
    pub fn create_guarded(
        self: &Arc<Self>,
        owner_session_id: Option<String>,
        request: VirtualDisplayRequest,
    ) -> Result<VirtualDisplayGuard, VirtualDisplayError> {
        let display = self.create_display(owner_session_id, request)?;
        Ok(VirtualDisplayGuard {
            manager: Arc::clone(self),
            display,
            active: AtomicBool::new(true),
        })
    }

    /// Replace the configuration of an existing virtual display.
    pub fn update_display(
        &self,
        display_id: &str,
        request: VirtualDisplayRequest,
    ) -> Result<ManagedVirtualDisplay, VirtualDisplayError> {
        request.validate()?;

        let mut displays = self.displays.lock().unwrap_or_else(|e| e.into_inner());
        let Some(entry) = displays.get_mut(display_id) else {
            return Err(VirtualDisplayError::NotFound(display_id.to_string()));
        };

        self.backend.update_display(&entry.os_display_id, &request)?;
        entry.request = request;
        Ok(entry.clone())
    }

    /// Convenience helper to resize an existing virtual display while preserving
    /// its name, HDR, and persistence flags (and optionally updating refresh/DPI).
    pub fn resize_display(
        &self,
        display_id: &str,
        width: u32,
        height: u32,
        refresh_hz: Option<u32>,
        dpi_scale: Option<f32>,
    ) -> Result<ManagedVirtualDisplay, VirtualDisplayError> {
        let mut displays = self.displays.lock().unwrap_or_else(|e| e.into_inner());
        let Some(entry) = displays.get_mut(display_id) else {
            return Err(VirtualDisplayError::NotFound(display_id.to_string()));
        };

        let mut updated = entry.request.clone();
        updated.width = width;
        updated.height = height;
        if let Some(hz) = refresh_hz {
            updated.refresh_hz = hz;
        }
        if let Some(scale) = dpi_scale {
            updated.dpi_scale = scale;
        }
        updated.validate()?;

        self.backend.update_display(&entry.os_display_id, &updated)?;
        entry.request = updated;
        Ok(entry.clone())
    }

    /// Destroy a specific virtual display by logical `display_id`.
    pub fn destroy_display(
        &self,
        display_id: &str,
    ) -> Result<ManagedVirtualDisplay, VirtualDisplayError> {
        let mut displays = self.displays.lock().unwrap_or_else(|e| e.into_inner());
        let Some(os_display_id) = displays.get(display_id).map(|d| d.os_display_id.clone()) else {
            return Err(VirtualDisplayError::NotFound(display_id.to_string()));
        };

        self.backend.destroy_display(&os_display_id)?;
        let removed = displays
            .remove(display_id)
            .expect("display existed under lock");
        Ok(removed)
    }

    /// Tear down all ephemeral displays owned by `session_id` on disconnect.
    ///
    /// Displays with `persist_after_disconnect == true` are preserved. Displays
    /// owned by other sessions (or unowned) are untouched. Attempts to destroy
    /// every matching ephemeral display even if one backend call fails.
    pub fn teardown_session(&self, session_id: &str) -> Result<Vec<String>, VirtualDisplayError> {
        let mut displays = self.displays.lock().unwrap_or_else(|e| e.into_inner());
        let targets: Vec<(String, String)> = displays
            .values()
            .filter(|d| {
                d.owner_session_id.as_deref() == Some(session_id)
                    && !d.request.persist_after_disconnect
            })
            .map(|d| (d.id.clone(), d.os_display_id.clone()))
            .collect();

        let mut destroyed = Vec::with_capacity(targets.len());
        let mut first_err: Option<VirtualDisplayError> = None;

        for (id, os_display_id) in targets {
            match self.backend.destroy_display(&os_display_id) {
                Ok(()) => {
                    displays.remove(&id);
                    destroyed.push(id);
                }
                Err(err) => {
                    first_err.get_or_insert(err);
                }
            }
        }

        match first_err {
            Some(err) => Err(err),
            None => Ok(destroyed),
        }
    }

    /// Destroy all managed virtual displays regardless of persistence or session ownership.
    pub fn destroy_all(&self) -> Result<Vec<String>, VirtualDisplayError> {
        let mut displays = self.displays.lock().unwrap_or_else(|e| e.into_inner());
        let targets: Vec<(String, String)> = displays
            .values()
            .map(|d| (d.id.clone(), d.os_display_id.clone()))
            .collect();

        let mut destroyed = Vec::with_capacity(targets.len());
        let mut first_err: Option<VirtualDisplayError> = None;

        for (id, os_display_id) in targets {
            match self.backend.destroy_display(&os_display_id) {
                Ok(()) => {
                    displays.remove(&id);
                    destroyed.push(id);
                }
                Err(err) => {
                    first_err.get_or_insert(err);
                }
            }
        }

        match first_err {
            Some(err) => Err(err),
            None => Ok(destroyed),
        }
    }

    /// Look up a managed virtual display by logical ID.
    pub fn get_display(&self, display_id: &str) -> Option<ManagedVirtualDisplay> {
        let displays = self.displays.lock().unwrap_or_else(|e| e.into_inner());
        displays.get(display_id).cloned()
    }

    /// List all currently active virtual displays in deterministic ID order.
    pub fn list_displays(&self) -> Vec<ManagedVirtualDisplay> {
        let displays = self.displays.lock().unwrap_or_else(|e| e.into_inner());
        displays.values().cloned().collect()
    }

    /// List all currently active virtual displays owned by `session_id`.
    pub fn list_session_displays(&self, session_id: &str) -> Vec<ManagedVirtualDisplay> {
        let displays = self.displays.lock().unwrap_or_else(|e| e.into_inner());
        displays
            .values()
            .filter(|d| d.owner_session_id.as_deref() == Some(session_id))
            .cloned()
            .collect()
    }

    /// Number of currently active virtual displays.
    pub fn active_count(&self) -> usize {
        let displays = self.displays.lock().unwrap_or_else(|e| e.into_inner());
        displays.len()
    }
}

/// RAII guard that automatically destroys an ephemeral virtual display on [`Drop`].
///
/// If the managed display has `persist_after_disconnect == true`, dropping the
/// guard leaves the display intact.
pub struct VirtualDisplayGuard {
    manager: Arc<VirtualDisplayManager>,
    display: ManagedVirtualDisplay,
    active: AtomicBool,
}

impl VirtualDisplayGuard {
    /// Snapshot of the display at guard creation time.
    pub fn display(&self) -> &ManagedVirtualDisplay {
        &self.display
    }

    /// Logical display identifier managed by [`VirtualDisplayManager`].
    pub fn display_id(&self) -> &str {
        &self.display.id
    }

    /// OS-level display identifier assigned by the platform backend.
    pub fn os_display_id(&self) -> &str {
        &self.display.os_display_id
    }

    /// Whether this guard is still armed to evaluate cleanup on drop.
    pub fn is_holding(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    /// Disarm the guard so dropping it will not destroy the virtual display.
    pub fn disarm(&self) {
        self.active.store(false, Ordering::SeqCst);
    }

    /// Explicitly release the guard, destroying the display if it is still
    /// present and ephemeral (`persist_after_disconnect == false`).
    ///
    /// Returns `Ok(true)` if the display was destroyed, `Ok(false)` if already
    /// disarmed, already removed, or marked persistent.
    pub fn release(self) -> Result<bool, VirtualDisplayError> {
        if !self.active.swap(false, Ordering::SeqCst) {
            return Ok(false);
        }
        let Some(current) = self.manager.get_display(&self.display.id) else {
            return Ok(false);
        };
        if current.request.persist_after_disconnect {
            return Ok(false);
        }
        self.manager.destroy_display(&self.display.id)?;
        Ok(true)
    }
}

impl Drop for VirtualDisplayGuard {
    fn drop(&mut self) {
        if !self.active.swap(false, Ordering::SeqCst) {
            return;
        }
        if let Some(current) = self.manager.get_display(&self.display.id) {
            if !current.request.persist_after_disconnect {
                let _ = self.manager.destroy_display(&self.display.id);
            }
        }
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Construct the default [`VirtualDisplayBackend`] for the current target OS.
pub fn default_platform_backend() -> Arc<dyn VirtualDisplayBackend> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(macos::MacOsVirtualDisplayBackend::new(None))
    }
    #[cfg(target_os = "windows")]
    {
        Arc::new(windows::WindowsVirtualDisplayBackend::new(None))
    }
    #[cfg(target_os = "linux")]
    {
        Arc::new(linux::LinuxVirtualDisplayBackend::new(None))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Arc::new(UnsupportedVirtualDisplayBackend)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
struct UnsupportedVirtualDisplayBackend;

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
impl VirtualDisplayBackend for UnsupportedVirtualDisplayBackend {
    fn create_display(
        &self,
        _request: &VirtualDisplayRequest,
    ) -> Result<String, VirtualDisplayError> {
        Err(VirtualDisplayError::UnsupportedPlatform(
            "virtual displays are not implemented on this OS".to_string(),
        ))
    }

    fn update_display(
        &self,
        _os_display_id: &str,
        _request: &VirtualDisplayRequest,
    ) -> Result<(), VirtualDisplayError> {
        Err(VirtualDisplayError::UnsupportedPlatform(
            "virtual displays are not implemented on this OS".to_string(),
        ))
    }

    fn destroy_display(&self, _os_display_id: &str) -> Result<(), VirtualDisplayError> {
        Err(VirtualDisplayError::UnsupportedPlatform(
            "virtual displays are not implemented on this OS".to_string(),
        ))
    }

    fn capabilities(&self) -> VirtualDisplayCapabilities {
        VirtualDisplayCapabilities {
            platform: "unsupported".to_string(),
            supported: false,
            supports_dynamic_resize: false,
            supports_hdr: false,
            supports_dpi_scale: false,
            supports_persistence: false,
            max_displays: 0,
            driver_hint: None,
        }
    }
}

// ---------------------------------------------------------------------------
// macOS backend: CGVirtualDisplay descriptor/mode builder hook
// ---------------------------------------------------------------------------
#[cfg(target_os = "macos")]
pub mod macos {
    use super::*;
    use std::sync::atomic::AtomicU32;

    /// Default synthetic vendor ID used for Weavelo `CGVirtualDisplayDescriptor` instances.
    pub const WEAVELO_CG_VENDOR_ID: u32 = 0x5756;
    /// Default synthetic product ID used for Weavelo `CGVirtualDisplayDescriptor` instances.
    pub const WEAVELO_CG_PRODUCT_ID: u32 = 0x0001;

    /// Mode parameters mirroring Apple's `CGVirtualDisplayMode`.
    #[derive(Clone, Debug, PartialEq)]
    pub struct CgVirtualDisplayMode {
        pub width: u32,
        pub height: u32,
        pub refresh_rate: f64,
        pub hi_dpi: bool,
    }

    /// Descriptor parameters mirroring Apple's `CGVirtualDisplayDescriptor` and
    /// `CGVirtualDisplaySettings`.
    #[derive(Clone, Debug, PartialEq)]
    pub struct CgVirtualDisplayDescriptor {
        pub name: String,
        pub max_pixels_wide: u32,
        pub max_pixels_high: u32,
        pub size_in_millimeters: (f64, f64),
        pub vendor_id: u32,
        pub product_id: u32,
        pub serial_num: u32,
        pub hdr: bool,
        pub mode: CgVirtualDisplayMode,
    }

    impl CgVirtualDisplayDescriptor {
        /// Build a `CGVirtualDisplay` descriptor and initial mode from a [`VirtualDisplayRequest`].
        pub fn from_request(request: &VirtualDisplayRequest, serial_num: u32) -> Self {
            let hi_dpi = request.dpi_scale > 1.0;
            let scale = f64::from(request.dpi_scale.max(MIN_VIRTUAL_DISPLAY_DPI_SCALE));
            // Compute backing pixel dimensions for HiDPI displays (clamped to valid u32).
            let max_pixels_wide = ((f64::from(request.width) * scale).round() as u32)
                .clamp(request.width, MAX_VIRTUAL_DISPLAY_WIDTH);
            let max_pixels_high = ((f64::from(request.height) * scale).round() as u32)
                .clamp(request.height, MAX_VIRTUAL_DISPLAY_HEIGHT);

            // Approximate physical panel size using a 96-DPI baseline scaled by dpi_scale.
            let effective_dpi = 96.0 * scale;
            let width_mm = (f64::from(max_pixels_wide) / effective_dpi) * 25.4;
            let height_mm = (f64::from(max_pixels_high) / effective_dpi) * 25.4;

            Self {
                name: request.name.clone(),
                max_pixels_wide,
                max_pixels_high,
                size_in_millimeters: (width_mm, height_mm),
                vendor_id: WEAVELO_CG_VENDOR_ID,
                product_id: WEAVELO_CG_PRODUCT_ID,
                serial_num,
                hdr: request.hdr,
                mode: CgVirtualDisplayMode {
                    width: request.width,
                    height: request.height,
                    refresh_rate: f64::from(request.refresh_hz),
                    hi_dpi,
                },
            }
        }
    }

    /// Hook implemented by a native Objective-C / AppKit bridge to drive `CGVirtualDisplay`.
    pub trait MacOsVirtualDisplayHook: Send + Sync {
        fn create_virtual_display(
            &self,
            descriptor: &CgVirtualDisplayDescriptor,
        ) -> Result<String, VirtualDisplayError>;

        fn apply_settings(
            &self,
            os_display_id: &str,
            descriptor: &CgVirtualDisplayDescriptor,
        ) -> Result<(), VirtualDisplayError>;

        fn destroy_virtual_display(&self, os_display_id: &str) -> Result<(), VirtualDisplayError>;
    }

    /// macOS virtual display backend using `CGVirtualDisplay` descriptor/mode hooks.
    pub struct MacOsVirtualDisplayBackend {
        hook: Option<Arc<dyn MacOsVirtualDisplayHook>>,
        next_serial: AtomicU32,
    }

    impl MacOsVirtualDisplayBackend {
        pub fn new(hook: Option<Arc<dyn MacOsVirtualDisplayHook>>) -> Self {
            Self {
                hook,
                next_serial: AtomicU32::new(1),
            }
        }
    }

    impl VirtualDisplayBackend for MacOsVirtualDisplayBackend {
        fn create_display(
            &self,
            request: &VirtualDisplayRequest,
        ) -> Result<String, VirtualDisplayError> {
            let Some(hook) = &self.hook else {
                return Err(VirtualDisplayError::UnsupportedPlatform(
                    "macOS CGVirtualDisplay requires an attached native CoreGraphics hook"
                        .to_string(),
                ));
            };
            let serial = self.next_serial.fetch_add(1, Ordering::SeqCst);
            let descriptor = CgVirtualDisplayDescriptor::from_request(request, serial);
            hook.create_virtual_display(&descriptor)
        }

        fn update_display(
            &self,
            os_display_id: &str,
            request: &VirtualDisplayRequest,
        ) -> Result<(), VirtualDisplayError> {
            let Some(hook) = &self.hook else {
                return Err(VirtualDisplayError::UnsupportedPlatform(
                    "macOS CGVirtualDisplay requires an attached native CoreGraphics hook"
                        .to_string(),
                ));
            };
            let descriptor = CgVirtualDisplayDescriptor::from_request(request, 0);
            hook.apply_settings(os_display_id, &descriptor)
        }

        fn destroy_display(&self, os_display_id: &str) -> Result<(), VirtualDisplayError> {
            let Some(hook) = &self.hook else {
                return Err(VirtualDisplayError::UnsupportedPlatform(
                    "macOS CGVirtualDisplay requires an attached native CoreGraphics hook"
                        .to_string(),
                ));
            };
            hook.destroy_virtual_display(os_display_id)
        }

        fn capabilities(&self) -> VirtualDisplayCapabilities {
            VirtualDisplayCapabilities {
                platform: "macos".to_string(),
                supported: self.hook.is_some(),
                supports_dynamic_resize: true,
                supports_hdr: true,
                supports_dpi_scale: true,
                supports_persistence: true,
                max_displays: MAX_VIRTUAL_DISPLAYS,
                driver_hint: Some("CoreGraphics CGVirtualDisplay".to_string()),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Windows backend: VDD / IddSampleDriver control hook
// ---------------------------------------------------------------------------
#[cfg(target_os = "windows")]
pub mod windows {
    use super::*;

    /// Default named-pipe control endpoint for a Windows Virtual Display Driver bridge.
    pub const DEFAULT_VDD_CONTROL_PIPE: &str = r"\\.\pipe\WeaveloVddControl";

    /// Mode specification passed to a Windows VDD / IddSampleDriver control hook.
    #[derive(Clone, Debug, PartialEq)]
    pub struct WindowsVddModeSpec {
        pub control_pipe: String,
        pub name: String,
        pub width: u32,
        pub height: u32,
        pub refresh_hz: u32,
        pub dpi_scale_percent: u32,
        pub hdr: bool,
    }

    impl WindowsVddModeSpec {
        pub fn from_request(control_pipe: &str, request: &VirtualDisplayRequest) -> Self {
            Self {
                control_pipe: control_pipe.to_string(),
                name: request.name.clone(),
                width: request.width,
                height: request.height,
                refresh_hz: request.refresh_hz,
                dpi_scale_percent: (request.dpi_scale * 100.0).round() as u32,
                hdr: request.hdr,
            }
        }
    }

    /// Hook implemented by a Windows Virtual Display Driver (VDD / IddSampleDriver) controller.
    pub trait WindowsVddHook: Send + Sync {
        fn add_monitor(&self, spec: &WindowsVddModeSpec) -> Result<String, VirtualDisplayError>;
        fn update_monitor(
            &self,
            os_display_id: &str,
            spec: &WindowsVddModeSpec,
        ) -> Result<(), VirtualDisplayError>;
        fn remove_monitor(&self, os_display_id: &str) -> Result<(), VirtualDisplayError>;
    }

    /// Windows virtual display backend using VDD / IddSampleDriver control hooks.
    pub struct WindowsVirtualDisplayBackend {
        hook: Option<Arc<dyn WindowsVddHook>>,
        control_pipe: String,
    }

    impl WindowsVirtualDisplayBackend {
        pub fn new(hook: Option<Arc<dyn WindowsVddHook>>) -> Self {
            Self {
                hook,
                control_pipe: DEFAULT_VDD_CONTROL_PIPE.to_string(),
            }
        }

        pub fn with_control_pipe(
            hook: Option<Arc<dyn WindowsVddHook>>,
            control_pipe: impl Into<String>,
        ) -> Self {
            Self {
                hook,
                control_pipe: control_pipe.into(),
            }
        }
    }

    impl VirtualDisplayBackend for WindowsVirtualDisplayBackend {
        fn create_display(
            &self,
            request: &VirtualDisplayRequest,
        ) -> Result<String, VirtualDisplayError> {
            let Some(hook) = &self.hook else {
                return Err(VirtualDisplayError::UnsupportedPlatform(
                    "Windows virtual display requires an installed VDD / IddSampleDriver control hook"
                        .to_string(),
                ));
            };
            let spec = WindowsVddModeSpec::from_request(&self.control_pipe, request);
            hook.add_monitor(&spec)
        }

        fn update_display(
            &self,
            os_display_id: &str,
            request: &VirtualDisplayRequest,
        ) -> Result<(), VirtualDisplayError> {
            let Some(hook) = &self.hook else {
                return Err(VirtualDisplayError::UnsupportedPlatform(
                    "Windows virtual display requires an installed VDD / IddSampleDriver control hook"
                        .to_string(),
                ));
            };
            let spec = WindowsVddModeSpec::from_request(&self.control_pipe, request);
            hook.update_monitor(os_display_id, &spec)
        }

        fn destroy_display(&self, os_display_id: &str) -> Result<(), VirtualDisplayError> {
            let Some(hook) = &self.hook else {
                return Err(VirtualDisplayError::UnsupportedPlatform(
                    "Windows virtual display requires an installed VDD / IddSampleDriver control hook"
                        .to_string(),
                ));
            };
            hook.remove_monitor(os_display_id)
        }

        fn capabilities(&self) -> VirtualDisplayCapabilities {
            VirtualDisplayCapabilities {
                platform: "windows".to_string(),
                supported: self.hook.is_some(),
                supports_dynamic_resize: true,
                supports_hdr: true,
                supports_dpi_scale: true,
                supports_persistence: true,
                max_displays: MAX_VIRTUAL_DISPLAYS,
                driver_hint: Some("Windows VDD / IddSampleDriver".to_string()),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Linux backend: KWin / PipeWire `CreateVirtualMonitor` D-Bus hook
// ---------------------------------------------------------------------------
#[cfg(target_os = "linux")]
pub mod linux {
    use super::*;

    /// KWin D-Bus service name for virtual output / PipeWire monitor creation.
    pub const KWIN_DBUS_SERVICE: &str = "org.kde.KWin";
    /// KWin D-Bus interface exposing `CreateVirtualMonitor`.
    pub const KWIN_VIRTUAL_OUTPUT_INTERFACE: &str = "org.kde.KWin.VirtualOutput";
    /// KWin D-Bus method name used to request a virtual monitor.
    pub const KWIN_CREATE_VIRTUAL_MONITOR_METHOD: &str = "CreateVirtualMonitor";

    /// D-Bus specification for KWin / PipeWire `CreateVirtualMonitor`.
    #[derive(Clone, Debug, PartialEq)]
    pub struct KwinVirtualMonitorSpec {
        pub dbus_service: String,
        pub dbus_interface: String,
        pub method: String,
        pub name: String,
        pub width: u32,
        pub height: u32,
        pub refresh_hz: u32,
        pub scale: f64,
        pub hdr: bool,
    }

    impl KwinVirtualMonitorSpec {
        pub fn from_request(request: &VirtualDisplayRequest) -> Self {
            Self {
                dbus_service: KWIN_DBUS_SERVICE.to_string(),
                dbus_interface: KWIN_VIRTUAL_OUTPUT_INTERFACE.to_string(),
                method: KWIN_CREATE_VIRTUAL_MONITOR_METHOD.to_string(),
                name: request.name.clone(),
                width: request.width,
                height: request.height,
                refresh_hz: request.refresh_hz,
                scale: f64::from(request.dpi_scale),
                hdr: request.hdr,
            }
        }
    }

    /// Hook implemented by a Linux KWin / PipeWire D-Bus client.
    pub trait LinuxVirtualDisplayHook: Send + Sync {
        fn create_virtual_monitor(
            &self,
            spec: &KwinVirtualMonitorSpec,
        ) -> Result<String, VirtualDisplayError>;

        fn update_virtual_monitor(
            &self,
            os_display_id: &str,
            spec: &KwinVirtualMonitorSpec,
        ) -> Result<(), VirtualDisplayError>;

        fn destroy_virtual_monitor(&self, os_display_id: &str) -> Result<(), VirtualDisplayError>;
    }

    /// Linux virtual display backend using KWin / PipeWire `CreateVirtualMonitor` D-Bus hooks.
    pub struct LinuxVirtualDisplayBackend {
        hook: Option<Arc<dyn LinuxVirtualDisplayHook>>,
    }

    impl LinuxVirtualDisplayBackend {
        pub fn new(hook: Option<Arc<dyn LinuxVirtualDisplayHook>>) -> Self {
            Self { hook }
        }
    }

    impl VirtualDisplayBackend for LinuxVirtualDisplayBackend {
        fn create_display(
            &self,
            request: &VirtualDisplayRequest,
        ) -> Result<String, VirtualDisplayError> {
            let Some(hook) = &self.hook else {
                return Err(VirtualDisplayError::UnsupportedPlatform(
                    "Linux virtual display requires an attached KWin/PipeWire CreateVirtualMonitor D-Bus hook"
                        .to_string(),
                ));
            };
            let spec = KwinVirtualMonitorSpec::from_request(request);
            hook.create_virtual_monitor(&spec)
        }

        fn update_display(
            &self,
            os_display_id: &str,
            request: &VirtualDisplayRequest,
        ) -> Result<(), VirtualDisplayError> {
            let Some(hook) = &self.hook else {
                return Err(VirtualDisplayError::UnsupportedPlatform(
                    "Linux virtual display requires an attached KWin/PipeWire CreateVirtualMonitor D-Bus hook"
                        .to_string(),
                ));
            };
            let spec = KwinVirtualMonitorSpec::from_request(request);
            hook.update_virtual_monitor(os_display_id, &spec)
        }

        fn destroy_display(&self, os_display_id: &str) -> Result<(), VirtualDisplayError> {
            let Some(hook) = &self.hook else {
                return Err(VirtualDisplayError::UnsupportedPlatform(
                    "Linux virtual display requires an attached KWin/PipeWire CreateVirtualMonitor D-Bus hook"
                        .to_string(),
                ));
            };
            hook.destroy_virtual_monitor(os_display_id)
        }

        fn capabilities(&self) -> VirtualDisplayCapabilities {
            VirtualDisplayCapabilities {
                platform: "linux".to_string(),
                supported: self.hook.is_some(),
                supports_dynamic_resize: true,
                supports_hdr: false,
                supports_dpi_scale: true,
                supports_persistence: true,
                max_displays: MAX_VIRTUAL_DISPLAYS,
                driver_hint: Some("KWin / PipeWire CreateVirtualMonitor D-Bus".to_string()),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Deterministic mock backend for unit & integration tests
// ---------------------------------------------------------------------------

/// Recorded operation observed by [`MockVirtualDisplayBackend`].
#[derive(Clone, Debug, PartialEq)]
pub enum MockVirtualDisplayCall {
    Create {
        os_display_id: String,
        request: VirtualDisplayRequest,
    },
    Update {
        os_display_id: String,
        request: VirtualDisplayRequest,
    },
    Destroy {
        os_display_id: String,
    },
}

struct MockBackendState {
    next_os_id: u64,
    active: HashMap<String, VirtualDisplayRequest>,
    calls: Vec<MockVirtualDisplayCall>,
    fail_create: bool,
    fail_update: bool,
    fail_destroy: bool,
    fail_destroy_ids: Vec<String>,
    capabilities: VirtualDisplayCapabilities,
}

impl Default for MockBackendState {
    fn default() -> Self {
        Self {
            next_os_id: 1,
            active: HashMap::new(),
            calls: Vec::new(),
            fail_create: false,
            fail_update: false,
            fail_destroy: false,
            fail_destroy_ids: Vec::new(),
            capabilities: VirtualDisplayCapabilities {
                platform: "mock".to_string(),
                supported: true,
                supports_dynamic_resize: true,
                supports_hdr: true,
                supports_dpi_scale: true,
                supports_persistence: true,
                max_displays: MAX_VIRTUAL_DISPLAYS,
                driver_hint: Some("MockVirtualDisplayBackend".to_string()),
            },
        }
    }
}

/// Deterministic in-memory [`VirtualDisplayBackend`] for unit and integration testing.
#[derive(Default)]
pub struct MockVirtualDisplayBackend {
    state: Mutex<MockBackendState>,
}

impl MockVirtualDisplayBackend {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn calls(&self) -> Vec<MockVirtualDisplayCall> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .calls
            .clone()
    }

    pub fn active_os_displays(&self) -> HashMap<String, VirtualDisplayRequest> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active
            .clone()
    }

    pub fn set_fail_create(&self, fail: bool) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .fail_create = fail;
    }

    pub fn set_fail_update(&self, fail: bool) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .fail_update = fail;
    }

    pub fn set_fail_destroy(&self, fail: bool) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .fail_destroy = fail;
    }

    pub fn set_fail_destroy_os_id(&self, os_display_id: impl Into<String>) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .fail_destroy_ids
            .push(os_display_id.into());
    }

    pub fn set_capabilities(&self, capabilities: VirtualDisplayCapabilities) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .capabilities = capabilities;
    }
}

impl VirtualDisplayBackend for MockVirtualDisplayBackend {
    fn create_display(
        &self,
        request: &VirtualDisplayRequest,
    ) -> Result<String, VirtualDisplayError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.fail_create {
            return Err(VirtualDisplayError::Backend(
                "mock create_display failure".to_string(),
            ));
        }
        let seq = state.next_os_id;
        state.next_os_id += 1;
        let os_display_id = format!("mock-os-display-{seq}");
        state.active.insert(os_display_id.clone(), request.clone());
        state.calls.push(MockVirtualDisplayCall::Create {
            os_display_id: os_display_id.clone(),
            request: request.clone(),
        });
        Ok(os_display_id)
    }

    fn update_display(
        &self,
        os_display_id: &str,
        request: &VirtualDisplayRequest,
    ) -> Result<(), VirtualDisplayError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.fail_update {
            return Err(VirtualDisplayError::Backend(
                "mock update_display failure".to_string(),
            ));
        }
        if !state.active.contains_key(os_display_id) {
            return Err(VirtualDisplayError::NotFound(os_display_id.to_string()));
        }
        state
            .active
            .insert(os_display_id.to_string(), request.clone());
        state.calls.push(MockVirtualDisplayCall::Update {
            os_display_id: os_display_id.to_string(),
            request: request.clone(),
        });
        Ok(())
    }

    fn destroy_display(&self, os_display_id: &str) -> Result<(), VirtualDisplayError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.fail_destroy || state.fail_destroy_ids.iter().any(|id| id == os_display_id) {
            return Err(VirtualDisplayError::Backend(format!(
                "mock destroy_display failure for {os_display_id}"
            )));
        }
        if state.active.remove(os_display_id).is_none() {
            return Err(VirtualDisplayError::NotFound(os_display_id.to_string()));
        }
        state.calls.push(MockVirtualDisplayCall::Destroy {
            os_display_id: os_display_id.to_string(),
        });
        Ok(())
    }

    fn capabilities(&self) -> VirtualDisplayCapabilities {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .capabilities
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request(name: &str, persist_after_disconnect: bool) -> VirtualDisplayRequest {
        VirtualDisplayRequest {
            name: name.to_string(),
            width: 2560,
            height: 1440,
            refresh_hz: 120,
            dpi_scale: 2.0,
            hdr: true,
            persist_after_disconnect,
        }
    }

    #[test]
    fn create_and_destroy_virtual_display() {
        let backend = MockVirtualDisplayBackend::new();
        let manager = VirtualDisplayManager::new(backend.clone());

        let req = sample_request("Studio Virtual", false);
        let created = manager
            .create_display(Some("sess-1".to_string()), req.clone())
            .unwrap();

        assert_eq!(created.id, "vdisp-1");
        assert_eq!(created.os_display_id, "mock-os-display-1");
        assert_eq!(created.request, req);
        assert_eq!(created.owner_session_id.as_deref(), Some("sess-1"));
        assert!(created.created_at > 0);
        assert_eq!(manager.active_count(), 1);
        assert_eq!(manager.get_display("vdisp-1"), Some(created.clone()));
        assert_eq!(backend.active_os_displays().len(), 1);

        let removed = manager.destroy_display("vdisp-1").unwrap();
        assert_eq!(removed, created);
        assert_eq!(manager.active_count(), 0);
        assert!(manager.get_display("vdisp-1").is_none());
        assert!(backend.active_os_displays().is_empty());

        // Destroying an unknown ID fails cleanly with NotFound.
        assert_eq!(
            manager.destroy_display("vdisp-1"),
            Err(VirtualDisplayError::NotFound("vdisp-1".to_string()))
        );
    }

    #[test]
    fn request_validation_enforces_bounds() {
        // Boundary extremes are valid.
        let min_req = VirtualDisplayRequest {
            name: "Min".to_string(),
            width: MIN_VIRTUAL_DISPLAY_WIDTH,
            height: MIN_VIRTUAL_DISPLAY_HEIGHT,
            refresh_hz: MIN_VIRTUAL_DISPLAY_REFRESH_HZ,
            dpi_scale: MIN_VIRTUAL_DISPLAY_DPI_SCALE,
            hdr: false,
            persist_after_disconnect: false,
        };
        assert!(min_req.validate().is_ok());

        let max_req = VirtualDisplayRequest {
            name: "Max".to_string(),
            width: MAX_VIRTUAL_DISPLAY_WIDTH,
            height: MAX_VIRTUAL_DISPLAY_HEIGHT,
            refresh_hz: MAX_VIRTUAL_DISPLAY_REFRESH_HZ,
            dpi_scale: MAX_VIRTUAL_DISPLAY_DPI_SCALE,
            hdr: true,
            persist_after_disconnect: true,
        };
        assert!(max_req.validate().is_ok());

        // Empty name, oversized name, NUL byte.
        for bad_name in [
            "   ".to_string(),
            "a".repeat(MAX_VIRTUAL_DISPLAY_NAME_LEN + 1),
            "bad\0name".to_string(),
        ] {
            let mut req = VirtualDisplayRequest::default();
            req.name = bad_name;
            assert!(matches!(
                req.validate(),
                Err(VirtualDisplayError::InvalidRequest(_))
            ));
        }

        // Invalid widths.
        for bad_width in [0, MIN_VIRTUAL_DISPLAY_WIDTH - 1, MAX_VIRTUAL_DISPLAY_WIDTH + 1] {
            let mut req = VirtualDisplayRequest::default();
            req.width = bad_width;
            assert!(matches!(
                req.validate(),
                Err(VirtualDisplayError::InvalidRequest(_))
            ));
        }

        // Invalid heights.
        for bad_height in [
            0,
            MIN_VIRTUAL_DISPLAY_HEIGHT - 1,
            MAX_VIRTUAL_DISPLAY_HEIGHT + 1,
        ] {
            let mut req = VirtualDisplayRequest::default();
            req.height = bad_height;
            assert!(matches!(
                req.validate(),
                Err(VirtualDisplayError::InvalidRequest(_))
            ));
        }

        // Invalid refresh rates.
        for bad_hz in [
            0,
            MIN_VIRTUAL_DISPLAY_REFRESH_HZ - 1,
            MAX_VIRTUAL_DISPLAY_REFRESH_HZ + 1,
        ] {
            let mut req = VirtualDisplayRequest::default();
            req.refresh_hz = bad_hz;
            assert!(matches!(
                req.validate(),
                Err(VirtualDisplayError::InvalidRequest(_))
            ));
        }

        // Invalid DPI scales.
        for bad_dpi in [0.0, 0.49, 4.01, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut req = VirtualDisplayRequest::default();
            req.dpi_scale = bad_dpi;
            assert!(matches!(
                req.validate(),
                Err(VirtualDisplayError::InvalidRequest(_))
            ));
        }
    }

    #[test]
    fn max_virtual_displays_cap_enforced_and_recovers_after_destroy() {
        let backend = MockVirtualDisplayBackend::new();
        let manager = VirtualDisplayManager::new(backend.clone());

        let mut ids = Vec::new();
        for idx in 0..MAX_VIRTUAL_DISPLAYS {
            let display = manager
                .create_display(
                    Some("sess-cap".to_string()),
                    sample_request(&format!("Display {idx}"), false),
                )
                .unwrap();
            ids.push(display.id);
        }
        assert_eq!(manager.active_count(), MAX_VIRTUAL_DISPLAYS);

        // Fifth display must be rejected with MaxDisplaysReached.
        let err = manager
            .create_display(
                Some("sess-cap".to_string()),
                sample_request("Overflow", false),
            )
            .unwrap_err();
        assert_eq!(
            err,
            VirtualDisplayError::MaxDisplaysReached {
                limit: MAX_VIRTUAL_DISPLAYS
            }
        );

        // Destroying one frees a slot for a new display.
        manager.destroy_display(&ids[1]).unwrap();
        assert_eq!(manager.active_count(), MAX_VIRTUAL_DISPLAYS - 1);

        let replacement = manager
            .create_display(
                Some("sess-cap".to_string()),
                sample_request("Replacement", false),
            )
            .unwrap();
        assert_eq!(manager.active_count(), MAX_VIRTUAL_DISPLAYS);
        assert!(manager.get_display(&replacement.id).is_some());
    }

    #[test]
    fn resize_and_update_virtual_display() {
        let backend = MockVirtualDisplayBackend::new();
        let manager = VirtualDisplayManager::new(backend.clone());

        let created = manager
            .create_display(
                Some("sess-resize".to_string()),
                VirtualDisplayRequest::default(),
            )
            .unwrap();

        let resized = manager
            .resize_display(&created.id, 3840, 2160, Some(144), Some(1.5))
            .unwrap();
        assert_eq!(resized.request.width, 3840);
        assert_eq!(resized.request.height, 2160);
        assert_eq!(resized.request.refresh_hz, 144);
        assert_eq!(resized.request.dpi_scale, 1.5);
        assert_eq!(
            backend
                .active_os_displays()
                .get(&created.os_display_id)
                .unwrap()
                .width,
            3840
        );

        // Invalid resize dimensions are rejected without mutating state.
        let err = manager
            .resize_display(&created.id, 100, 100, None, None)
            .unwrap_err();
        assert!(matches!(err, VirtualDisplayError::InvalidRequest(_)));
        assert_eq!(manager.get_display(&created.id).unwrap().request.width, 3840);

        // Backend update failure leaves previous display configuration intact.
        backend.set_fail_update(true);
        let mut updated_req = resized.request.clone();
        updated_req.hdr = true;
        updated_req.persist_after_disconnect = true;
        let err = manager.update_display(&created.id, updated_req.clone()).unwrap_err();
        assert!(matches!(err, VirtualDisplayError::Backend(_)));
        assert!(!manager.get_display(&created.id).unwrap().request.hdr);

        // Clearing backend failure allows full update_display to succeed.
        backend.set_fail_update(false);
        let updated = manager.update_display(&created.id, updated_req).unwrap();
        assert!(updated.request.hdr);
        assert!(updated.request.persist_after_disconnect);
    }

    #[test]
    fn teardown_session_destroys_ephemeral_and_preserves_persistent_and_other_sessions() {
        let backend = MockVirtualDisplayBackend::new();
        let manager = VirtualDisplayManager::new(backend.clone());

        let s1_ephemeral_a = manager
            .create_display(Some("sess-1".to_string()), sample_request("S1 Eph A", false))
            .unwrap();
        let s1_persistent = manager
            .create_display(Some("sess-1".to_string()), sample_request("S1 Persist", true))
            .unwrap();
        let s1_ephemeral_b = manager
            .create_display(Some("sess-1".to_string()), sample_request("S1 Eph B", false))
            .unwrap();
        let s2_ephemeral = manager
            .create_display(Some("sess-2".to_string()), sample_request("S2 Eph", false))
            .unwrap();

        assert_eq!(manager.active_count(), 4);
        assert_eq!(manager.list_session_displays("sess-1").len(), 3);

        let destroyed = manager.teardown_session("sess-1").unwrap();
        assert_eq!(destroyed, vec![s1_ephemeral_a.id.clone(), s1_ephemeral_b.id.clone()]);

        // Session 1's persistent display and session 2's display remain active.
        assert_eq!(manager.active_count(), 2);
        assert!(manager.get_display(&s1_ephemeral_a.id).is_none());
        assert!(manager.get_display(&s1_ephemeral_b.id).is_none());
        assert!(manager.get_display(&s1_persistent.id).is_some());
        assert!(manager.get_display(&s2_ephemeral.id).is_some());
        assert_eq!(backend.active_os_displays().len(), 2);

        // Repeating teardown on sess-1 is a clean no-op.
        assert!(manager.teardown_session("sess-1").unwrap().is_empty());
    }

    #[test]
    fn teardown_session_attempts_remaining_ephemeral_displays_on_partial_failure() {
        let backend = MockVirtualDisplayBackend::new();
        let manager = VirtualDisplayManager::new(backend.clone());

        let d1 = manager
            .create_display(Some("sess-err".to_string()), sample_request("D1", false))
            .unwrap();
        let d2 = manager
            .create_display(Some("sess-err".to_string()), sample_request("D2", false))
            .unwrap();

        // Inject failure only for the first display's OS ID.
        backend.set_fail_destroy_os_id(d1.os_display_id.clone());

        let err = manager.teardown_session("sess-err").unwrap_err();
        assert!(matches!(err, VirtualDisplayError::Backend(_)));
        // d2 was still destroyed while d1 stayed tracked because backend destroy failed.
        assert!(manager.get_display(&d1.id).is_some());
        assert!(manager.get_display(&d2.id).is_none());
    }

    #[test]
    fn raii_guard_destroys_ephemeral_on_drop_and_preserves_persistent() {
        let backend = MockVirtualDisplayBackend::new();
        let manager = Arc::new(VirtualDisplayManager::new(backend.clone()));

        // Ephemeral display is automatically destroyed when guard drops.
        let ephemeral_id;
        {
            let guard = manager
                .create_guarded(Some("sess-guard".to_string()), sample_request("Ephemeral", false))
                .unwrap();
            ephemeral_id = guard.display_id().to_string();
            assert!(guard.is_holding());
            assert_eq!(manager.active_count(), 1);
        }
        assert_eq!(manager.active_count(), 0);
        assert!(manager.get_display(&ephemeral_id).is_none());
        assert!(backend.active_os_displays().is_empty());

        // Persistent display survives guard drop.
        let persistent_id;
        {
            let guard = manager
                .create_guarded(Some("sess-guard".to_string()), sample_request("Persistent", true))
                .unwrap();
            persistent_id = guard.display_id().to_string();
            assert_eq!(manager.active_count(), 1);
        }
        assert_eq!(manager.active_count(), 1);
        assert!(manager.get_display(&persistent_id).is_some());

        // Disarmed ephemeral guard also preserves its display on drop.
        let disarmed_id;
        {
            let guard = manager
                .create_guarded(Some("sess-guard".to_string()), sample_request("Disarmed", false))
                .unwrap();
            disarmed_id = guard.display_id().to_string();
            guard.disarm();
            assert!(!guard.is_holding());
        }
        assert_eq!(manager.active_count(), 2);
        assert!(manager.get_display(&disarmed_id).is_some());

        // Explicit release destroys once without double-destroy on Drop.
        let guard = manager
            .create_guarded(Some("sess-guard".to_string()), sample_request("Explicit", false))
            .unwrap();
        let explicit_os_id = guard.os_display_id().to_string();
        assert!(guard.release().unwrap());
        let destroy_count = backend
            .calls()
            .iter()
            .filter(|call| {
                matches!(call, MockVirtualDisplayCall::Destroy { os_display_id } if os_display_id == &explicit_os_id)
            })
            .count();
        assert_eq!(destroy_count, 1);
    }

    #[test]
    fn serde_roundtrip_and_platform_backend_smoke() {
        let req = sample_request("4K HDR", true);
        let json = serde_json::to_string(&req).unwrap();
        let decoded: VirtualDisplayRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, req);

        let backend = default_platform_backend();
        let caps = backend.capabilities();
        assert!(!caps.platform.is_empty());
        assert!(caps.max_displays <= MAX_VIRTUAL_DISPLAYS);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_cg_descriptor_builder_and_hook() {
        use std::sync::atomic::AtomicUsize;

        struct RecordingMacHook {
            created: AtomicUsize,
            updated: AtomicUsize,
            destroyed: AtomicUsize,
        }

        impl macos::MacOsVirtualDisplayHook for RecordingMacHook {
            fn create_virtual_display(
                &self,
                descriptor: &macos::CgVirtualDisplayDescriptor,
            ) -> Result<String, VirtualDisplayError> {
                assert_eq!(descriptor.mode.width, 1920);
                assert_eq!(descriptor.mode.height, 1080);
                assert!(descriptor.mode.hi_dpi);
                assert_eq!(descriptor.max_pixels_wide, 3840);
                assert_eq!(descriptor.max_pixels_high, 2160);
                self.created.fetch_add(1, Ordering::SeqCst);
                Ok(format!("cg-display-{}", descriptor.serial_num))
            }

            fn apply_settings(
                &self,
                _os_display_id: &str,
                descriptor: &macos::CgVirtualDisplayDescriptor,
            ) -> Result<(), VirtualDisplayError> {
                assert_eq!(descriptor.mode.width, 2560);
                self.updated.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }

            fn destroy_virtual_display(
                &self,
                _os_display_id: &str,
            ) -> Result<(), VirtualDisplayError> {
                self.destroyed.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }

        let hook = Arc::new(RecordingMacHook {
            created: AtomicUsize::new(0),
            updated: AtomicUsize::new(0),
            destroyed: AtomicUsize::new(0),
        });
        let backend = Arc::new(macos::MacOsVirtualDisplayBackend::new(Some(hook.clone())));
        assert!(backend.capabilities().supported);

        let manager = VirtualDisplayManager::new(backend);
        let req = VirtualDisplayRequest {
            name: "Mac HiDPI".to_string(),
            width: 1920,
            height: 1080,
            refresh_hz: 60,
            dpi_scale: 2.0,
            hdr: false,
            persist_after_disconnect: false,
        };
        let display = manager.create_display(Some("mac-sess".into()), req).unwrap();
        assert_eq!(display.os_display_id, "cg-display-1");
        manager
            .resize_display(&display.id, 2560, 1440, Some(120), None)
            .unwrap();
        manager.destroy_display(&display.id).unwrap();

        assert_eq!(hook.created.load(Ordering::SeqCst), 1);
        assert_eq!(hook.updated.load(Ordering::SeqCst), 1);
        assert_eq!(hook.destroyed.load(Ordering::SeqCst), 1);
    }
}
