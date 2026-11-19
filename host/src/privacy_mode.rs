//! Cross-platform Host Privacy Mode controller.
//!
//! Provides local display blanking or a custom privacy curtain, optional local
//! input blocking, an emergency hotkey registration hook, RAII [`PrivacyModeGuard`]
//! auto-restore on drop, and a disconnect watchdog that restores the host display
//! and input automatically if the remote session drops.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Component, Path};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Default disconnect grace period before privacy mode automatically restores (15 seconds).
pub const DEFAULT_AUTO_RESTORE_AFTER_DISCONNECT_MS: u64 = 15_000;

/// Maximum permitted disconnect auto-restore timeout (1 hour).
pub const MAX_AUTO_RESTORE_AFTER_DISCONNECT_MS: u64 = 3_600_000;

/// Maximum UTF-8 byte length for a custom curtain title.
pub const MAX_CURTAIN_TITLE_LEN: usize = 160;

/// Maximum UTF-8 byte length for a custom curtain subtitle.
pub const MAX_CURTAIN_SUBTITLE_LEN: usize = 512;

/// Default platform-neutral emergency hotkey chord to exit privacy mode locally.
pub const DEFAULT_EMERGENCY_HOTKEY: &str = "Ctrl+Alt+Shift+P";

const ALLOWED_CURTAIN_IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "svg"];

/// Visual privacy presentation applied to the host display.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PrivacyCurtainMode {
    /// Blank/fade the physical display(s) to black or turn off monitor power.
    BlankScreen,
    /// Display a custom fullscreen privacy curtain / screensaver overlay.
    CustomCurtain {
        title: String,
        #[serde(default)]
        subtitle: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        image_path: Option<String>,
    },
}

impl Default for PrivacyCurtainMode {
    fn default() -> Self {
        Self::BlankScreen
    }
}

/// Configuration for host privacy mode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacyModeConfig {
    /// Whether privacy mode should be active during the remote session.
    pub enabled: bool,
    /// Visual presentation mode for the local display.
    #[serde(default)]
    pub mode: PrivacyCurtainMode,
    /// Block local keyboard and mouse input while privacy mode is active.
    #[serde(default = "default_true")]
    pub block_local_input: bool,
    /// Delay after remote disconnect before automatically restoring local display/input.
    /// A value of `0` restores immediately upon disconnect notification.
    #[serde(default = "default_auto_restore_ms")]
    pub auto_restore_after_disconnect_ms: u64,
    /// Local emergency hotkey chord (e.g. `"Ctrl+Alt+Shift+P"`) to force-exit privacy mode.
    #[serde(default = "default_emergency_hotkey")]
    pub emergency_hotkey: String,
}

fn default_true() -> bool {
    true
}

fn default_auto_restore_ms() -> u64 {
    DEFAULT_AUTO_RESTORE_AFTER_DISCONNECT_MS
}

fn default_emergency_hotkey() -> String {
    DEFAULT_EMERGENCY_HOTKEY.to_string()
}

impl Default for PrivacyModeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: PrivacyCurtainMode::BlankScreen,
            block_local_input: true,
            auto_restore_after_disconnect_ms: DEFAULT_AUTO_RESTORE_AFTER_DISCONNECT_MS,
            emergency_hotkey: DEFAULT_EMERGENCY_HOTKEY.to_string(),
        }
    }
}

impl PrivacyModeConfig {
    /// Validate configuration invariants before applying privacy mode.
    pub fn validate(&self) -> Result<(), PrivacyModeError> {
        if self.auto_restore_after_disconnect_ms > MAX_AUTO_RESTORE_AFTER_DISCONNECT_MS {
            return Err(PrivacyModeError::InvalidConfig(format!(
                "auto_restore_after_disconnect_ms ({}) exceeds maximum ({})",
                self.auto_restore_after_disconnect_ms, MAX_AUTO_RESTORE_AFTER_DISCONNECT_MS
            )));
        }

        validate_hotkey(&self.emergency_hotkey)?;

        if let PrivacyCurtainMode::CustomCurtain {
            title,
            subtitle,
            image_path,
        } = &self.mode
        {
            if title.trim().is_empty() {
                return Err(PrivacyModeError::InvalidConfig(
                    "custom curtain title must not be empty".to_string(),
                ));
            }
            if title.len() > MAX_CURTAIN_TITLE_LEN {
                return Err(PrivacyModeError::InvalidConfig(format!(
                    "custom curtain title exceeds {MAX_CURTAIN_TITLE_LEN} bytes"
                )));
            }
            if subtitle.len() > MAX_CURTAIN_SUBTITLE_LEN {
                return Err(PrivacyModeError::InvalidConfig(format!(
                    "custom curtain subtitle exceeds {MAX_CURTAIN_SUBTITLE_LEN} bytes"
                )));
            }
            if let Some(raw_path) = image_path {
                validate_curtain_image_path(raw_path)?;
            }
        }

        Ok(())
    }
}

fn validate_hotkey(hotkey: &str) -> Result<(), PrivacyModeError> {
    let trimmed = hotkey.trim();
    if trimmed.is_empty() {
        return Err(PrivacyModeError::InvalidConfig(
            "emergency_hotkey must not be empty".to_string(),
        ));
    }
    let parts: Vec<&str> = trimmed.split('+').map(str::trim).collect();
    if parts.len() < 2 || parts.iter().any(|part| part.is_empty()) {
        return Err(PrivacyModeError::InvalidConfig(format!(
            "emergency_hotkey '{hotkey}' must be a modifier+key chord (e.g. '{DEFAULT_EMERGENCY_HOTKEY}')"
        )));
    }
    Ok(())
}

fn validate_curtain_image_path(raw_path: &str) -> Result<(), PrivacyModeError> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err(PrivacyModeError::InvalidConfig(
            "custom curtain image_path must not be empty when provided".to_string(),
        ));
    }
    if trimmed.contains('\0') {
        return Err(PrivacyModeError::InvalidConfig(
            "custom curtain image_path must not contain NUL bytes".to_string(),
        ));
    }
    let path = Path::new(trimmed);
    for component in path.components() {
        if matches!(component, Component::ParentDir) {
            return Err(PrivacyModeError::InvalidConfig(
                "custom curtain image_path must not contain parent-directory ('..') traversal"
                    .to_string(),
            ));
        }
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some(ext) if ALLOWED_CURTAIN_IMAGE_EXTENSIONS.contains(&ext) => Ok(()),
        _ => Err(PrivacyModeError::InvalidConfig(format!(
            "custom curtain image_path '{raw_path}' must use a supported image extension ({})",
            ALLOWED_CURTAIN_IMAGE_EXTENSIONS.join(", ")
        ))),
    }
}

/// Errors produced by privacy mode validation or platform backends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrivacyModeError {
    /// Configuration failed validation.
    InvalidConfig(String),
    /// Backend operation failed on the host platform.
    Backend(String),
    /// Operation is not supported on the current host platform.
    UnsupportedPlatform(String),
}

impl fmt::Display for PrivacyModeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(msg) => write!(f, "invalid privacy mode config: {msg}"),
            Self::Backend(msg) => write!(f, "privacy mode backend error: {msg}"),
            Self::UnsupportedPlatform(msg) => {
                write!(f, "privacy mode unsupported on this platform: {msg}")
            }
        }
    }
}

impl std::error::Error for PrivacyModeError {}

/// Reason privacy mode transitioned back to `Inactive`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoreReason {
    Manual,
    ConfigDisabled,
    GuardDropped,
    DisconnectWatchdog,
    EmergencyHotkey,
}

/// Result of an activation request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivationOutcome {
    /// Privacy mode was newly activated.
    Activated,
    /// Privacy mode was already active with the exact same configuration (idempotent no-op).
    AlreadyActive,
    /// Privacy mode was already active and updated in place to the new configuration.
    Reconfigured,
    /// Config had `enabled == false`; privacy mode remained or became inactive.
    Disabled,
}

/// Lifecycle state of [`PrivacyModeController`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PrivacyModeState {
    Inactive,
    Active {
        mode: PrivacyCurtainMode,
        input_blocked: bool,
        emergency_hotkey: String,
        activated_at_ms: u64,
        disconnect_watchdog_armed: bool,
    },
}

/// Point-in-time diagnostic snapshot of the privacy mode controller.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacyModeSnapshot {
    pub platform: String,
    pub state: PrivacyModeState,
    pub last_restore_reason: Option<RestoreReason>,
    pub last_restore_at_ms: Option<u64>,
    pub activation_count: u64,
    pub restore_count: u64,
}

/// Platform abstraction for host display blanking, custom curtain, local input guard,
/// and emergency hotkey registration.
pub trait PrivacyBackend: Send + Sync {
    /// Human-readable backend/platform identifier.
    fn platform_name(&self) -> &'static str;

    /// Blank the physical display(s).
    fn apply_blank_screen(&self) -> Result<(), PrivacyModeError>;

    /// Show a custom fullscreen privacy curtain.
    fn apply_custom_curtain(
        &self,
        title: &str,
        subtitle: &str,
        image_path: Option<&str>,
    ) -> Result<(), PrivacyModeError>;

    /// Restore the physical display(s) and dismiss any curtain.
    fn restore_display(&self) -> Result<(), PrivacyModeError>;

    /// Block or unblock local keyboard/mouse input.
    fn set_local_input_blocked(&self, blocked: bool) -> Result<(), PrivacyModeError>;

    /// Register a local emergency hotkey chord that can force-restore privacy mode.
    fn register_emergency_hotkey(&self, _hotkey: &str) -> Result<(), PrivacyModeError> {
        Ok(())
    }

    /// Unregister the local emergency hotkey chord.
    fn unregister_emergency_hotkey(&self) -> Result<(), PrivacyModeError> {
        Ok(())
    }
}

/// Optional callback hook for native UI layers to render a custom curtain window.
pub type CurtainRenderHook =
    Arc<dyn Fn(Option<(&str, &str, Option<&str>)>) -> Result<(), PrivacyModeError> + Send + Sync>;

struct WatchdogHandle {
    generation: u64,
    cancelled: Arc<(Mutex<bool>, Condvar)>,
}

impl WatchdogHandle {
    fn cancel(&self) {
        let (lock, cvar) = &*self.cancelled;
        if let Ok(mut cancelled) = lock.lock() {
            *cancelled = true;
            cvar.notify_all();
        }
    }
}

struct ControllerInner {
    active_config: Option<PrivacyModeConfig>,
    activated_at_ms: Option<u64>,
    input_blocked: bool,
    hotkey_registered: bool,
    last_restore_reason: Option<RestoreReason>,
    last_restore_at_ms: Option<u64>,
    activation_count: u64,
    restore_count: u64,
    watchdog: Option<WatchdogHandle>,
}

impl Default for ControllerInner {
    fn default() -> Self {
        Self {
            active_config: None,
            activated_at_ms: None,
            input_blocked: false,
            hotkey_registered: false,
            last_restore_reason: None,
            last_restore_at_ms: None,
            activation_count: 0,
            restore_count: 0,
            watchdog: None,
        }
    }
}

/// Thread-safe state machine managing host privacy mode, RAII guards, and disconnect recovery.
pub struct PrivacyModeController {
    backend: Arc<dyn PrivacyBackend>,
    inner: Mutex<ControllerInner>,
    watchdog_generation: AtomicU64,
}

impl PrivacyModeController {
    /// Create a controller backed by the current host OS implementation.
    pub fn with_platform_backend() -> Arc<Self> {
        Arc::new(Self::new(default_platform_backend()))
    }

    /// Create a controller with a custom [`PrivacyBackend`] (e.g. a mock backend in tests).
    pub fn new(backend: Arc<dyn PrivacyBackend>) -> Self {
        Self {
            backend,
            inner: Mutex::new(ControllerInner::default()),
            watchdog_generation: AtomicU64::new(0),
        }
    }

    /// Activate or update privacy mode according to `config`.
    ///
    /// - Idempotent when already active with the exact same `config`.
    /// - If `config.enabled == false`, restores privacy mode if currently active.
    /// - Automatically rolls back partial backend state if a later step fails.
    pub fn activate(&self, config: PrivacyModeConfig) -> Result<ActivationOutcome, PrivacyModeError> {
        config.validate()?;

        if !config.enabled {
            let _ = self.restore(RestoreReason::ConfigDisabled)?;
            return Ok(ActivationOutcome::Disabled);
        }

        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());

        // Disarm any pending disconnect watchdog on fresh activation/reconfiguration.
        if let Some(watchdog) = inner.watchdog.take() {
            watchdog.cancel();
        }

        if let Some(current) = inner.active_config.clone() {
            if current == config {
                return Ok(ActivationOutcome::AlreadyActive);
            }

            // Reconfigure in place: update display mode, input blocking, and hotkey as needed.
            let mode_changed = current.mode != config.mode;
            let input_changed = current.block_local_input != config.block_local_input;
            let hotkey_changed = current.emergency_hotkey != config.emergency_hotkey;

            if mode_changed {
                self.apply_mode(&config.mode)?;
            }
            if input_changed {
                if let Err(err) = self.backend.set_local_input_blocked(config.block_local_input) {
                    if mode_changed {
                        let _ = self.apply_mode(&current.mode);
                    }
                    return Err(err);
                }
                inner.input_blocked = config.block_local_input;
            }
            if hotkey_changed {
                if inner.hotkey_registered {
                    let _ = self.backend.unregister_emergency_hotkey();
                    inner.hotkey_registered = false;
                }
                if let Err(err) = self.backend.register_emergency_hotkey(&config.emergency_hotkey) {
                    let _ = self
                        .backend
                        .register_emergency_hotkey(&current.emergency_hotkey);
                    inner.hotkey_registered = true;
                    if input_changed {
                        let _ = self
                            .backend
                            .set_local_input_blocked(current.block_local_input);
                        inner.input_blocked = current.block_local_input;
                    }
                    if mode_changed {
                        let _ = self.apply_mode(&current.mode);
                    }
                    return Err(err);
                }
                inner.hotkey_registered = true;
            }

            inner.active_config = Some(config);
            return Ok(ActivationOutcome::Reconfigured);
        }

        // Fresh activation with rollback on partial failure.
        self.apply_mode(&config.mode)?;

        if let Err(err) = self.backend.register_emergency_hotkey(&config.emergency_hotkey) {
            let _ = self.backend.restore_display();
            return Err(err);
        }

        if config.block_local_input {
            if let Err(err) = self.backend.set_local_input_blocked(true) {
                let _ = self.backend.unregister_emergency_hotkey();
                let _ = self.backend.restore_display();
                return Err(err);
            }
        }

        inner.active_config = Some(config.clone());
        inner.activated_at_ms = Some(now_unix_ms());
        inner.input_blocked = config.block_local_input;
        inner.hotkey_registered = true;
        inner.activation_count += 1;

        Ok(ActivationOutcome::Activated)
    }

    /// Activate privacy mode and return an RAII [`PrivacyModeGuard`] that restores
    /// local display and input automatically when dropped.
    pub fn acquire_guard(
        self: &Arc<Self>,
        config: PrivacyModeConfig,
    ) -> Result<PrivacyModeGuard, PrivacyModeError> {
        let outcome = self.activate(config)?;
        let active = !matches!(outcome, ActivationOutcome::Disabled);
        Ok(PrivacyModeGuard {
            controller: Arc::clone(self),
            active: AtomicBool::new(active),
        })
    }

    /// Restore host display and local input if privacy mode is currently active.
    ///
    /// Returns `Ok(true)` if a restore occurred, or `Ok(false)` if already inactive (idempotent).
    /// Always attempts to unblock input, unregister the hotkey, and restore the display
    /// even if one of the backend calls returns an error.
    pub fn restore(&self, reason: RestoreReason) -> Result<bool, PrivacyModeError> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(watchdog) = inner.watchdog.take() {
            watchdog.cancel();
        }
        if inner.active_config.is_none() {
            return Ok(false);
        }

        let mut first_err: Option<PrivacyModeError> = None;

        if inner.input_blocked {
            if let Err(err) = self.backend.set_local_input_blocked(false) {
                first_err.get_or_insert(err);
            }
        }

        if inner.hotkey_registered {
            if let Err(err) = self.backend.unregister_emergency_hotkey() {
                first_err.get_or_insert(err);
            }
        }

        if let Err(err) = self.backend.restore_display() {
            first_err.get_or_insert(err);
        }

        inner.active_config = None;
        inner.activated_at_ms = None;
        inner.input_blocked = false;
        inner.hotkey_registered = false;
        inner.last_restore_reason = Some(reason);
        inner.last_restore_at_ms = Some(now_unix_ms());
        inner.restore_count += 1;

        match first_err {
            Some(err) => Err(err),
            None => Ok(true),
        }
    }

    /// Convenience alias for `restore(RestoreReason::Manual)`.
    pub fn deactivate(&self) -> Result<bool, PrivacyModeError> {
        self.restore(RestoreReason::Manual)
    }

    /// Trigger an immediate local emergency restore (e.g. from the emergency hotkey handler).
    pub fn trigger_emergency_restore(&self) -> Result<bool, PrivacyModeError> {
        self.restore(RestoreReason::EmergencyHotkey)
    }

    /// Notify the controller that the remote session disconnected.
    ///
    /// - If privacy mode is inactive, this is a no-op (`Ok(false)`).
    /// - If `auto_restore_after_disconnect_ms == 0`, restores immediately (`Ok(true)`).
    /// - Otherwise arms a cancellable watchdog timer that restores privacy mode once
    ///   `auto_restore_after_disconnect_ms` elapses without a [`Self::notify_reconnect`].
    pub fn notify_disconnect(self: &Arc<Self>) -> Result<bool, PrivacyModeError> {
        let timeout_ms = {
            let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let Some(cfg) = &inner.active_config else {
                return Ok(false);
            };
            cfg.auto_restore_after_disconnect_ms
        };

        if timeout_ms == 0 {
            return self.restore(RestoreReason::DisconnectWatchdog);
        }

        let generation = self.watchdog_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let cancelled = Arc::new((Mutex::new(false), Condvar::new()));

        {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if inner.active_config.is_none() {
                return Ok(false);
            }
            if let Some(prev) = inner.watchdog.take() {
                prev.cancel();
            }
            inner.watchdog = Some(WatchdogHandle {
                generation,
                cancelled: Arc::clone(&cancelled),
            });
        }

        let controller = Arc::clone(self);
        let delay = Duration::from_millis(timeout_ms);

        std::thread::Builder::new()
            .name(format!("weavelo-privacy-watchdog-{generation}"))
            .spawn(move || {
                let (lock, cvar) = &*cancelled;
                let guard = lock.lock().unwrap_or_else(|e| e.into_inner());
                let (guard, _wait_result) = cvar
                    .wait_timeout_while(guard, delay, |is_cancelled| !*is_cancelled)
                    .unwrap_or_else(|e| e.into_inner());
                if *guard {
                    return;
                }
                drop(guard);

                // Verify our generation is still the active watchdog before restoring.
                let should_restore = {
                    let inner = controller.inner.lock().unwrap_or_else(|e| e.into_inner());
                    inner
                        .watchdog
                        .as_ref()
                        .is_some_and(|w| w.generation == generation)
                };
                if should_restore {
                    let _ = controller.restore(RestoreReason::DisconnectWatchdog);
                }
            })
            .map_err(|err| {
                PrivacyModeError::Backend(format!("failed to spawn disconnect watchdog: {err}"))
            })?;

        Ok(true)
    }

    /// Notify the controller that the remote session reconnected, cancelling any armed
    /// disconnect watchdog.
    ///
    /// Returns `true` if a pending watchdog was cancelled.
    pub fn notify_reconnect(&self) -> bool {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(watchdog) = inner.watchdog.take() {
            watchdog.cancel();
            true
        } else {
            false
        }
    }

    /// Return whether privacy mode is currently active.
    pub fn is_active(&self) -> bool {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.active_config.is_some()
    }

    /// Return a serializable snapshot of the current privacy mode state and counters.
    pub fn snapshot(&self) -> PrivacyModeSnapshot {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let state = match &inner.active_config {
            Some(cfg) => PrivacyModeState::Active {
                mode: cfg.mode.clone(),
                input_blocked: inner.input_blocked,
                emergency_hotkey: cfg.emergency_hotkey.clone(),
                activated_at_ms: inner.activated_at_ms.unwrap_or(0),
                disconnect_watchdog_armed: inner.watchdog.is_some(),
            },
            None => PrivacyModeState::Inactive,
        };
        PrivacyModeSnapshot {
            platform: self.backend.platform_name().to_string(),
            state,
            last_restore_reason: inner.last_restore_reason,
            last_restore_at_ms: inner.last_restore_at_ms,
            activation_count: inner.activation_count,
            restore_count: inner.restore_count,
        }
    }

    fn apply_mode(&self, mode: &PrivacyCurtainMode) -> Result<(), PrivacyModeError> {
        match mode {
            PrivacyCurtainMode::BlankScreen => self.backend.apply_blank_screen(),
            PrivacyCurtainMode::CustomCurtain {
                title,
                subtitle,
                image_path,
            } => self
                .backend
                .apply_custom_curtain(title, subtitle, image_path.as_deref()),
        }
    }
}

/// RAII guard that automatically restores host privacy mode when dropped.
pub struct PrivacyModeGuard {
    controller: Arc<PrivacyModeController>,
    active: AtomicBool,
}

impl PrivacyModeGuard {
    /// Whether this guard is still armed to restore privacy mode on drop.
    pub fn is_holding(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    /// Access the underlying controller.
    pub fn controller(&self) -> &Arc<PrivacyModeController> {
        &self.controller
    }

    /// Explicitly restore privacy mode and consume the guard, returning any backend error.
    pub fn release(self) -> Result<bool, PrivacyModeError> {
        if self.active.swap(false, Ordering::SeqCst) {
            self.controller.restore(RestoreReason::Manual)
        } else {
            Ok(false)
        }
    }

    /// Disarm the guard without restoring privacy mode.
    pub fn disarm(&self) {
        self.active.store(false, Ordering::SeqCst);
    }
}

impl Drop for PrivacyModeGuard {
    fn drop(&mut self) {
        if self.active.swap(false, Ordering::SeqCst) {
            let _ = self.controller.restore(RestoreReason::GuardDropped);
        }
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Construct the default [`PrivacyBackend`] for the current target OS.
pub fn default_platform_backend() -> Arc<dyn PrivacyBackend> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(macos::MacOsPrivacyBackend::new(None))
    }
    #[cfg(target_os = "windows")]
    {
        Arc::new(windows::WindowsPrivacyBackend::new(None))
    }
    #[cfg(target_os = "linux")]
    {
        Arc::new(linux::LinuxPrivacyBackend::new(None))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Arc::new(UnsupportedPrivacyBackend)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
struct UnsupportedPrivacyBackend;

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
impl PrivacyBackend for UnsupportedPrivacyBackend {
    fn platform_name(&self) -> &'static str {
        "unsupported"
    }

    fn apply_blank_screen(&self) -> Result<(), PrivacyModeError> {
        Err(PrivacyModeError::UnsupportedPlatform(
            "blank screen is not implemented on this OS".to_string(),
        ))
    }

    fn apply_custom_curtain(
        &self,
        _title: &str,
        _subtitle: &str,
        _image_path: Option<&str>,
    ) -> Result<(), PrivacyModeError> {
        Err(PrivacyModeError::UnsupportedPlatform(
            "custom curtain is not implemented on this OS".to_string(),
        ))
    }

    fn restore_display(&self) -> Result<(), PrivacyModeError> {
        Ok(())
    }

    fn set_local_input_blocked(&self, _blocked: bool) -> Result<(), PrivacyModeError> {
        Err(PrivacyModeError::UnsupportedPlatform(
            "local input guard is not implemented on this OS".to_string(),
        ))
    }
}

// ---------------------------------------------------------------------------
// macOS backend: CoreGraphics CGDisplayFade + curtain hook + local cursor/input guard
// ---------------------------------------------------------------------------
#[cfg(target_os = "macos")]
pub mod macos {
    use super::*;

    const CG_ERROR_SUCCESS: i32 = 0;
    const CG_DISPLAY_FADE_RESERVATION_INVALID_TOKEN: u32 = 0;
    const CG_DISPLAY_BLEND_NORMAL: f32 = 0.0;
    const CG_DISPLAY_BLEND_SOLID_COLOR: f32 = 1.0;
    const FADE_RESERVATION_SECONDS: f32 = 15.0;
    const FADE_DURATION_SECONDS: f32 = 0.15;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGAcquireDisplayFadeReservation(seconds: f32, token: *mut u32) -> i32;
        fn CGDisplayFade(
            token: u32,
            duration: f32,
            start_blend: f32,
            end_blend: f32,
            red_blend: f32,
            green_blend: f32,
            blue_blend: f32,
            synchronous: u32,
        ) -> i32;
        fn CGReleaseDisplayFadeReservation(token: u32) -> i32;
        fn CGAssociateMouseAndMouseCursorPosition(connected: u32) -> i32;
    }

    pub struct MacOsPrivacyBackend {
        fade_token: Mutex<Option<u32>>,
        curtain_hook: Option<CurtainRenderHook>,
    }

    impl MacOsPrivacyBackend {
        pub fn new(curtain_hook: Option<CurtainRenderHook>) -> Self {
            Self {
                fade_token: Mutex::new(None),
                curtain_hook,
            }
        }

        fn acquire_token(&self) -> Result<u32, PrivacyModeError> {
            let mut guard = self.fade_token.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(token) = *guard {
                return Ok(token);
            }
            let mut token = CG_DISPLAY_FADE_RESERVATION_INVALID_TOKEN;
            let status = unsafe { CGAcquireDisplayFadeReservation(FADE_RESERVATION_SECONDS, &mut token) };
            if status != CG_ERROR_SUCCESS || token == CG_DISPLAY_FADE_RESERVATION_INVALID_TOKEN {
                return Err(PrivacyModeError::Backend(format!(
                    "CGAcquireDisplayFadeReservation failed with CGError {status}"
                )));
            }
            *guard = Some(token);
            Ok(token)
        }
    }

    impl PrivacyBackend for MacOsPrivacyBackend {
        fn platform_name(&self) -> &'static str {
            "macos"
        }

        fn apply_blank_screen(&self) -> Result<(), PrivacyModeError> {
            let token = self.acquire_token()?;
            let status = unsafe {
                CGDisplayFade(
                    token,
                    FADE_DURATION_SECONDS,
                    CG_DISPLAY_BLEND_NORMAL,
                    CG_DISPLAY_BLEND_SOLID_COLOR,
                    0.0,
                    0.0,
                    0.0,
                    1,
                )
            };
            if status != CG_ERROR_SUCCESS {
                return Err(PrivacyModeError::Backend(format!(
                    "CGDisplayFade (blank) failed with CGError {status}"
                )));
            }
            Ok(())
        }

        fn apply_custom_curtain(
            &self,
            title: &str,
            subtitle: &str,
            image_path: Option<&str>,
        ) -> Result<(), PrivacyModeError> {
            if let Some(hook) = &self.curtain_hook {
                hook(Some((title, subtitle, image_path)))
            } else {
                // Fallback to CoreGraphics solid-black fade when no native NSWindow curtain is attached.
                self.apply_blank_screen()
            }
        }

        fn restore_display(&self) -> Result<(), PrivacyModeError> {
            let mut first_err = None;
            if let Some(hook) = &self.curtain_hook {
                if let Err(err) = hook(None) {
                    first_err = Some(err);
                }
            }
            let token = self
                .fade_token
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            if let Some(token) = token {
                let fade_status = unsafe {
                    CGDisplayFade(
                        token,
                        FADE_DURATION_SECONDS,
                        CG_DISPLAY_BLEND_SOLID_COLOR,
                        CG_DISPLAY_BLEND_NORMAL,
                        0.0,
                        0.0,
                        0.0,
                        1,
                    )
                };
                let release_status = unsafe { CGReleaseDisplayFadeReservation(token) };
                if fade_status != CG_ERROR_SUCCESS {
                    first_err.get_or_insert_with(|| {
                        PrivacyModeError::Backend(format!(
                            "CGDisplayFade (restore) failed with CGError {fade_status}"
                        ))
                    });
                }
                if release_status != CG_ERROR_SUCCESS {
                    first_err.get_or_insert_with(|| {
                        PrivacyModeError::Backend(format!(
                            "CGReleaseDisplayFadeReservation failed with CGError {release_status}"
                        ))
                    });
                }
            }
            match first_err {
                Some(err) => Err(err),
                None => Ok(()),
            }
        }

        fn set_local_input_blocked(&self, blocked: bool) -> Result<(), PrivacyModeError> {
            let connected = if blocked { 0 } else { 1 };
            let status = unsafe { CGAssociateMouseAndMouseCursorPosition(connected) };
            if status != CG_ERROR_SUCCESS {
                return Err(PrivacyModeError::Backend(format!(
                    "CGAssociateMouseAndMouseCursorPosition({connected}) failed with CGError {status}"
                )));
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Windows backend: SC_MONITORPOWER + BlockInput + curtain hook
// ---------------------------------------------------------------------------
#[cfg(target_os = "windows")]
pub mod windows {
    use super::*;
    use windows_sys::Win32::Foundation::{LPARAM, WPARAM};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::BlockInput;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, PostMessageW, SC_MONITORPOWER, WM_SYSCOMMAND,
    };

    const MONITOR_POWER_ON: LPARAM = -1;
    const MONITOR_POWER_OFF: LPARAM = 2;

    pub struct WindowsPrivacyBackend {
        curtain_hook: Option<CurtainRenderHook>,
    }

    impl WindowsPrivacyBackend {
        pub fn new(curtain_hook: Option<CurtainRenderHook>) -> Self {
            Self { curtain_hook }
        }

        fn post_monitor_power(power_state: LPARAM) -> Result<(), PrivacyModeError> {
            let ok = unsafe {
                PostMessageW(
                    HWND_BROADCAST,
                    WM_SYSCOMMAND,
                    SC_MONITORPOWER as WPARAM,
                    power_state,
                )
            };
            if ok == 0 {
                return Err(PrivacyModeError::Backend(format!(
                    "PostMessageW(SC_MONITORPOWER, {power_state}) failed"
                )));
            }
            Ok(())
        }
    }

    impl PrivacyBackend for WindowsPrivacyBackend {
        fn platform_name(&self) -> &'static str {
            "windows"
        }

        fn apply_blank_screen(&self) -> Result<(), PrivacyModeError> {
            Self::post_monitor_power(MONITOR_POWER_OFF)
        }

        fn apply_custom_curtain(
            &self,
            title: &str,
            subtitle: &str,
            image_path: Option<&str>,
        ) -> Result<(), PrivacyModeError> {
            if let Some(hook) = &self.curtain_hook {
                hook(Some((title, subtitle, image_path)))
            } else {
                self.apply_blank_screen()
            }
        }

        fn restore_display(&self) -> Result<(), PrivacyModeError> {
            let mut first_err = None;
            if let Some(hook) = &self.curtain_hook {
                if let Err(err) = hook(None) {
                    first_err = Some(err);
                }
            }
            if let Err(err) = Self::post_monitor_power(MONITOR_POWER_ON) {
                first_err.get_or_insert(err);
            }
            match first_err {
                Some(err) => Err(err),
                None => Ok(()),
            }
        }

        fn set_local_input_blocked(&self, blocked: bool) -> Result<(), PrivacyModeError> {
            let flag = if blocked { 1 } else { 0 };
            let ok = unsafe { BlockInput(flag) };
            if ok == 0 {
                return Err(PrivacyModeError::Backend(format!(
                    "BlockInput({flag}) failed (administrator elevation may be required)"
                )));
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Linux backend: X11 DPMS (`xset`) + curtain hook + local input hook
// ---------------------------------------------------------------------------
#[cfg(target_os = "linux")]
pub mod linux {
    use super::*;
    use std::process::Command;

    pub type LinuxInputHook = Arc<dyn Fn(bool) -> Result<(), PrivacyModeError> + Send + Sync>;

    pub struct LinuxPrivacyBackend {
        curtain_hook: Option<CurtainRenderHook>,
        input_hook: Option<LinuxInputHook>,
    }

    impl LinuxPrivacyBackend {
        pub fn new(curtain_hook: Option<CurtainRenderHook>) -> Self {
            Self {
                curtain_hook,
                input_hook: None,
            }
        }

        pub fn with_hooks(
            curtain_hook: Option<CurtainRenderHook>,
            input_hook: Option<LinuxInputHook>,
        ) -> Self {
            Self {
                curtain_hook,
                input_hook,
            }
        }

        fn run_dpms(action: &str) -> Result<(), PrivacyModeError> {
            let output = Command::new("xset")
                .args(["dpms", "force", action])
                .output()
                .map_err(|err| {
                    PrivacyModeError::Backend(format!(
                        "failed to execute 'xset dpms force {action}': {err}"
                    ))
                })?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(PrivacyModeError::Backend(format!(
                    "xset dpms force {action} exited with {}: {}",
                    output.status,
                    stderr.trim()
                )));
            }
            Ok(())
        }
    }

    impl PrivacyBackend for LinuxPrivacyBackend {
        fn platform_name(&self) -> &'static str {
            "linux"
        }

        fn apply_blank_screen(&self) -> Result<(), PrivacyModeError> {
            Self::run_dpms("off")
        }

        fn apply_custom_curtain(
            &self,
            title: &str,
            subtitle: &str,
            image_path: Option<&str>,
        ) -> Result<(), PrivacyModeError> {
            if let Some(hook) = &self.curtain_hook {
                hook(Some((title, subtitle, image_path)))
            } else {
                self.apply_blank_screen()
            }
        }

        fn restore_display(&self) -> Result<(), PrivacyModeError> {
            let mut first_err = None;
            if let Some(hook) = &self.curtain_hook {
                if let Err(err) = hook(None) {
                    first_err = Some(err);
                }
            }
            if let Err(err) = Self::run_dpms("on") {
                first_err.get_or_insert(err);
            }
            match first_err {
                Some(err) => Err(err),
                None => Ok(()),
            }
        }

        fn set_local_input_blocked(&self, blocked: bool) -> Result<(), PrivacyModeError> {
            if let Some(hook) = &self.input_hook {
                hook(blocked)
            } else {
                // Without an XInput/evdev grab hook, report unsupported cleanly so callers can
                // choose blank-screen-only privacy mode or attach a platform grabber.
                if blocked {
                    Err(PrivacyModeError::UnsupportedPlatform(
                        "Linux local input blocking requires an attached input grab hook"
                            .to_string(),
                    ))
                } else {
                    Ok(())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum BackendCall {
        BlankScreen,
        CustomCurtain {
            title: String,
            subtitle: String,
            image_path: Option<String>,
        },
        RestoreDisplay,
        SetInputBlocked(bool),
        RegisterHotkey(String),
        UnregisterHotkey,
    }

    #[derive(Default)]
    struct MockState {
        calls: Vec<BackendCall>,
        fail_blank: bool,
        fail_curtain: bool,
        fail_block_input: bool,
        fail_unblock_input: bool,
        fail_register_hotkey: bool,
        fail_restore_display: bool,
    }

    #[derive(Default)]
    struct MockPrivacyBackend {
        state: Mutex<MockState>,
    }

    impl MockPrivacyBackend {
        fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }

        fn calls(&self) -> Vec<BackendCall> {
            self.state.lock().unwrap().calls.clone()
        }

        fn set_fail_block_input(&self, fail: bool) {
            self.state.lock().unwrap().fail_block_input = fail;
        }

        fn set_fail_unblock_input(&self, fail: bool) {
            self.state.lock().unwrap().fail_unblock_input = fail;
        }
    }

    impl PrivacyBackend for MockPrivacyBackend {
        fn platform_name(&self) -> &'static str {
            "mock"
        }

        fn apply_blank_screen(&self) -> Result<(), PrivacyModeError> {
            let mut state = self.state.lock().unwrap();
            state.calls.push(BackendCall::BlankScreen);
            if state.fail_blank {
                return Err(PrivacyModeError::Backend("mock blank failure".into()));
            }
            Ok(())
        }

        fn apply_custom_curtain(
            &self,
            title: &str,
            subtitle: &str,
            image_path: Option<&str>,
        ) -> Result<(), PrivacyModeError> {
            let mut state = self.state.lock().unwrap();
            state.calls.push(BackendCall::CustomCurtain {
                title: title.to_string(),
                subtitle: subtitle.to_string(),
                image_path: image_path.map(str::to_string),
            });
            if state.fail_curtain {
                return Err(PrivacyModeError::Backend("mock curtain failure".into()));
            }
            Ok(())
        }

        fn restore_display(&self) -> Result<(), PrivacyModeError> {
            let mut state = self.state.lock().unwrap();
            state.calls.push(BackendCall::RestoreDisplay);
            if state.fail_restore_display {
                return Err(PrivacyModeError::Backend("mock restore failure".into()));
            }
            Ok(())
        }

        fn set_local_input_blocked(&self, blocked: bool) -> Result<(), PrivacyModeError> {
            let mut state = self.state.lock().unwrap();
            state.calls.push(BackendCall::SetInputBlocked(blocked));
            if blocked && state.fail_block_input {
                return Err(PrivacyModeError::Backend("mock block input failure".into()));
            }
            if !blocked && state.fail_unblock_input {
                return Err(PrivacyModeError::Backend(
                    "mock unblock input failure".into(),
                ));
            }
            Ok(())
        }

        fn register_emergency_hotkey(&self, hotkey: &str) -> Result<(), PrivacyModeError> {
            let mut state = self.state.lock().unwrap();
            state
                .calls
                .push(BackendCall::RegisterHotkey(hotkey.to_string()));
            if state.fail_register_hotkey {
                return Err(PrivacyModeError::Backend("mock hotkey failure".into()));
            }
            Ok(())
        }

        fn unregister_emergency_hotkey(&self) -> Result<(), PrivacyModeError> {
            let mut state = self.state.lock().unwrap();
            state.calls.push(BackendCall::UnregisterHotkey);
            Ok(())
        }
    }

    fn blank_config() -> PrivacyModeConfig {
        PrivacyModeConfig {
            enabled: true,
            mode: PrivacyCurtainMode::BlankScreen,
            block_local_input: true,
            auto_restore_after_disconnect_ms: 2_000,
            emergency_hotkey: "Ctrl+Alt+Esc".to_string(),
        }
    }

    fn curtain_config() -> PrivacyModeConfig {
        PrivacyModeConfig {
            enabled: true,
            mode: PrivacyCurtainMode::CustomCurtain {
                title: "Remote Session Active".to_string(),
                subtitle: "Screen hidden for privacy".to_string(),
                image_path: Some("/assets/curtain.png".to_string()),
            },
            block_local_input: true,
            auto_restore_after_disconnect_ms: 5_000,
            emergency_hotkey: "Ctrl+Alt+Shift+P".to_string(),
        }
    }

    #[test]
    fn activation_and_manual_restore_sequence() {
        let backend = MockPrivacyBackend::new();
        let controller = PrivacyModeController::new(backend.clone());

        assert!(!controller.is_active());
        assert_eq!(
            controller.activate(blank_config()).unwrap(),
            ActivationOutcome::Activated
        );
        assert!(controller.is_active());

        let snap = controller.snapshot();
        assert_eq!(snap.platform, "mock");
        assert_eq!(snap.activation_count, 1);
        assert_eq!(snap.restore_count, 0);
        assert!(matches!(
            snap.state,
            PrivacyModeState::Active {
                mode: PrivacyCurtainMode::BlankScreen,
                input_blocked: true,
                disconnect_watchdog_armed: false,
                ..
            }
        ));

        assert!(controller.deactivate().unwrap());
        assert!(!controller.is_active());
        assert!(!controller.deactivate().unwrap()); // idempotent second restore

        let snap = controller.snapshot();
        assert_eq!(snap.state, PrivacyModeState::Inactive);
        assert_eq!(snap.last_restore_reason, Some(RestoreReason::Manual));
        assert_eq!(snap.restore_count, 1);

        assert_eq!(
            backend.calls(),
            vec![
                BackendCall::BlankScreen,
                BackendCall::RegisterHotkey("Ctrl+Alt+Esc".into()),
                BackendCall::SetInputBlocked(true),
                BackendCall::SetInputBlocked(false),
                BackendCall::UnregisterHotkey,
                BackendCall::RestoreDisplay,
            ]
        );
    }

    #[test]
    fn custom_curtain_config_validation_rejects_invalid_inputs() {
        // Valid custom curtain passes.
        assert!(curtain_config().validate().is_ok());

        // Empty title rejected.
        let mut cfg = curtain_config();
        cfg.mode = PrivacyCurtainMode::CustomCurtain {
            title: "   ".into(),
            subtitle: "ok".into(),
            image_path: None,
        };
        assert!(matches!(
            cfg.validate(),
            Err(PrivacyModeError::InvalidConfig(_))
        ));

        // Oversized title rejected.
        cfg.mode = PrivacyCurtainMode::CustomCurtain {
            title: "a".repeat(MAX_CURTAIN_TITLE_LEN + 1),
            subtitle: String::new(),
            image_path: None,
        };
        assert!(matches!(
            cfg.validate(),
            Err(PrivacyModeError::InvalidConfig(_))
        ));

        // Path traversal rejected.
        cfg.mode = PrivacyCurtainMode::CustomCurtain {
            title: "Private".into(),
            subtitle: String::new(),
            image_path: Some("../secret.png".into()),
        };
        assert!(matches!(
            cfg.validate(),
            Err(PrivacyModeError::InvalidConfig(_))
        ));

        // Unsupported image extension rejected.
        cfg.mode = PrivacyCurtainMode::CustomCurtain {
            title: "Private".into(),
            subtitle: String::new(),
            image_path: Some("/tmp/curtain.exe".into()),
        };
        assert!(matches!(
            cfg.validate(),
            Err(PrivacyModeError::InvalidConfig(_))
        ));

        // Empty or non-chord emergency hotkey rejected.
        let mut bad_hotkey = blank_config();
        bad_hotkey.emergency_hotkey = "Esc".into();
        assert!(matches!(
            bad_hotkey.validate(),
            Err(PrivacyModeError::InvalidConfig(_))
        ));

        // Excessive auto-restore timeout rejected.
        let mut bad_timeout = blank_config();
        bad_timeout.auto_restore_after_disconnect_ms = MAX_AUTO_RESTORE_AFTER_DISCONNECT_MS + 1;
        assert!(matches!(
            bad_timeout.validate(),
            Err(PrivacyModeError::InvalidConfig(_))
        ));
    }

    #[test]
    fn activation_idempotency_and_in_place_reconfigure() {
        let backend = MockPrivacyBackend::new();
        let controller = PrivacyModeController::new(backend.clone());

        let cfg = blank_config();
        assert_eq!(
            controller.activate(cfg.clone()).unwrap(),
            ActivationOutcome::Activated
        );
        // Repeating the exact same config must not invoke backend again.
        assert_eq!(
            controller.activate(cfg).unwrap(),
            ActivationOutcome::AlreadyActive
        );
        assert_eq!(backend.calls().len(), 3);

        // Switching to custom curtain reconfigures in place.
        let next = curtain_config();
        assert_eq!(
            controller.activate(next).unwrap(),
            ActivationOutcome::Reconfigured
        );
        assert_eq!(
            backend.calls(),
            vec![
                BackendCall::BlankScreen,
                BackendCall::RegisterHotkey("Ctrl+Alt+Esc".into()),
                BackendCall::SetInputBlocked(true),
                BackendCall::CustomCurtain {
                    title: "Remote Session Active".into(),
                    subtitle: "Screen hidden for privacy".into(),
                    image_path: Some("/assets/curtain.png".into()),
                },
                BackendCall::UnregisterHotkey,
                BackendCall::RegisterHotkey("Ctrl+Alt+Shift+P".into()),
            ]
        );
    }

    #[test]
    fn partial_activation_failure_rolls_back_display_and_hotkey() {
        let backend = MockPrivacyBackend::new();
        backend.set_fail_block_input(true);
        let controller = PrivacyModeController::new(backend.clone());

        let err = controller.activate(blank_config()).unwrap_err();
        assert!(matches!(err, PrivacyModeError::Backend(_)));
        assert!(!controller.is_active());
        assert_eq!(
            backend.calls(),
            vec![
                BackendCall::BlankScreen,
                BackendCall::RegisterHotkey("Ctrl+Alt+Esc".into()),
                BackendCall::SetInputBlocked(true),
                BackendCall::UnregisterHotkey,
                BackendCall::RestoreDisplay,
            ]
        );
    }

    #[test]
    fn restore_attempts_display_restore_even_if_input_unblock_fails() {
        let backend = MockPrivacyBackend::new();
        let controller = PrivacyModeController::new(backend.clone());
        controller.activate(blank_config()).unwrap();

        backend.set_fail_unblock_input(true);
        let err = controller.deactivate().unwrap_err();
        assert!(matches!(err, PrivacyModeError::Backend(_)));
        assert!(!controller.is_active());
        assert!(backend.calls().contains(&BackendCall::RestoreDisplay));
    }

    #[test]
    fn raii_guard_auto_restores_on_drop_and_does_not_double_restore() {
        let backend = MockPrivacyBackend::new();
        let controller = Arc::new(PrivacyModeController::new(backend.clone()));

        {
            let guard = controller.acquire_guard(curtain_config()).unwrap();
            assert!(guard.is_holding());
            assert!(controller.is_active());
        }

        assert!(!controller.is_active());
        let snap = controller.snapshot();
        assert_eq!(snap.last_restore_reason, Some(RestoreReason::GuardDropped));
        assert_eq!(snap.restore_count, 1);

        // Explicit release consumes the guard without triggering a second restore in Drop.
        let guard = controller.acquire_guard(blank_config()).unwrap();
        assert!(guard.release().unwrap());
        assert!(!controller.is_active());
        assert_eq!(controller.snapshot().restore_count, 2);
        assert_eq!(
            controller.snapshot().last_restore_reason,
            Some(RestoreReason::Manual)
        );
    }

    #[test]
    fn disconnect_watchdog_restores_after_timeout_and_can_be_cancelled_by_reconnect() {
        let backend = MockPrivacyBackend::new();
        let controller = Arc::new(PrivacyModeController::new(backend.clone()));

        let mut cfg = blank_config();
        cfg.auto_restore_after_disconnect_ms = 80;
        controller.activate(cfg.clone()).unwrap();

        // Disconnect arms the watchdog; reconnecting before timeout cancels it.
        assert!(controller.notify_disconnect().unwrap());
        assert!(matches!(
            controller.snapshot().state,
            PrivacyModeState::Active {
                disconnect_watchdog_armed: true,
                ..
            }
        ));
        assert!(controller.notify_reconnect());
        std::thread::sleep(Duration::from_millis(120));
        assert!(
            controller.is_active(),
            "privacy mode must remain active after reconnect cancelled watchdog"
        );

        // Disconnect again and let the watchdog expire.
        assert!(controller.notify_disconnect().unwrap());
        let deadline = Instant::now() + Duration::from_secs(2);
        while controller.is_active() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !controller.is_active(),
            "disconnect watchdog should have restored privacy mode"
        );
        assert_eq!(
            controller.snapshot().last_restore_reason,
            Some(RestoreReason::DisconnectWatchdog)
        );
    }

    #[test]
    fn disconnect_with_zero_timeout_restores_immediately() {
        let backend = MockPrivacyBackend::new();
        let controller = Arc::new(PrivacyModeController::new(backend));

        let mut cfg = blank_config();
        cfg.auto_restore_after_disconnect_ms = 0;
        controller.activate(cfg).unwrap();

        assert!(controller.notify_disconnect().unwrap());
        assert!(!controller.is_active());
        assert_eq!(
            controller.snapshot().last_restore_reason,
            Some(RestoreReason::DisconnectWatchdog)
        );
    }

    #[test]
    fn config_serde_roundtrip_and_platform_backend_smoke() {
        let cfg = curtain_config();
        let json = serde_json::to_string(&cfg).unwrap();
        let decoded: PrivacyModeConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, decoded);

        let backend = default_platform_backend();
        assert!(!backend.platform_name().is_empty());
    }
}
