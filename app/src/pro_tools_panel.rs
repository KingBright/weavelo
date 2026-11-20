//! Desktop Pro Tools unified control panel state, spring animations, and persistence.
//!
//! Aggregates host privacy mode, video color profile (4:2:0 / 4:4:4, SDR / HDR10),
//! transport route preference, virtual display presets & profiles, multiplexed port
//! forwarding rules, searchable keymap catalog state, and one-click redacted
//! diagnostics export into a single testable desktop state container.

use host::privacy_mode::{PrivacyModeConfig, PrivacyModeError};
use protocol::color::{
    ChromaSubsampling, ColorBitDepth, ColorPrimaries, ColorProfile, ColorProfileError,
    HdrMasteringMetadata, HdrTransfer,
};
use protocol::keymap::{KeymapCatalog, KeymapCategory, KeymapProfile};
use protocol::virtual_display::{
    DisplayPlacement, MAX_SNAPSHOT_DISPLAYS, VirtualDisplayError, VirtualDisplayPreset,
    VirtualDisplaySpec,
};
use remote_core::diagnostics_bundle::{
    DIAGNOSTIC_BUNDLE_SCHEMA_VERSION, DiagnosticBundle, DiagnosticSessionSummary,
    DiagnosticTransportSummary,
};
use remote_core::port_forward::{
    MAX_ACTIVE_RULES, PortForwardRule, RuleValidationError, validate_rules,
};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Current persistence schema version for [`DesktopProToolsPreferences`].
pub const PRO_TOOLS_PREFS_SCHEMA_VERSION: u32 = 1;

/// Maximum integration substep (120 Hz) for spring simulation stability.
const MAX_SPRING_SUBSTEP_SECS: f32 = 1.0 / 120.0;

/// Maximum total `dt` accepted in a single [`SpringFloat::step`] call to avoid runaway work.
const MAX_SPRING_STEP_SECS: f32 = 0.25;

/// Quick virtual display presets exposed in the Desktop Pro Tools panel.
pub const QUICK_VIRTUAL_DISPLAY_PRESETS: [VirtualDisplayPreset; 5] = [
    VirtualDisplayPreset::Fhd1080p,
    VirtualDisplayPreset::Qhd1440p,
    VirtualDisplayPreset::Uhd4k,
    VirtualDisplayPreset::Ultrawide21x9,
    VirtualDisplayPreset::SuperUltrawide32x9,
];

/// Transport route selection preference for desktop sessions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopRouteMode {
    /// Automatically choose the best available route (LAN -> P2P -> Relay).
    #[default]
    Auto,
    /// Restrict connections strictly to local area network peers.
    LanOnly,
    /// Prefer/force direct peer-to-peer hole-punched transport.
    P2pOnly,
    /// Route traffic through the NAS UDP relay endpoint.
    NasUdpRelay,
    /// Route traffic through the NAS WebSocket Secure (WSS) relay endpoint.
    NasWssRelay,
}

impl DesktopRouteMode {
    /// All supported desktop route modes in display order.
    pub const fn all() -> &'static [Self] {
        &[
            Self::Auto,
            Self::LanOnly,
            Self::P2pOnly,
            Self::NasUdpRelay,
            Self::NasWssRelay,
        ]
    }

    /// Canonical snake_case identifier used in diagnostics and configuration.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::LanOnly => "lan_only",
            Self::P2pOnly => "p2p_only",
            Self::NasUdpRelay => "nas_udp_relay",
            Self::NasWssRelay => "nas_wss_relay",
        }
    }

    /// Human-readable label for the desktop control panel UI.
    pub const fn display_label(self) -> &'static str {
        match self {
            Self::Auto => "Auto (LAN / P2P / Relay)",
            Self::LanOnly => "LAN Only",
            Self::P2pOnly => "P2P Direct Only",
            Self::NasUdpRelay => "NAS UDP Relay",
            Self::NasWssRelay => "NAS WSS Relay",
        }
    }

    /// Localized label respecting the active desktop UI language.
    pub fn localized_label(self) -> &'static str {
        crate::i18n::tr(self.display_label())
    }
}

impl fmt::Display for DesktopRouteMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Active section tab within the unified Desktop Pro Tools panel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProToolsTab {
    #[default]
    Privacy,
    Color,
    Route,
    Displays,
    PortForward,
    Keymaps,
    Diagnostics,
}

/// Errors produced by Desktop Pro Tools state validation, mutations, or persistence.
#[derive(Debug)]
pub enum ProToolsError {
    Privacy(PrivacyModeError),
    Color(ColorProfileError),
    VirtualDisplay(VirtualDisplayError),
    PortForward(RuleValidationError),
    DuplicateDisplayName(String),
    TooManyVirtualDisplays { count: usize, max: usize },
    DisplayIndexOutOfBounds { index: usize, len: usize },
    PortForwardRuleNotFound(String),
    KeymapNotFound(String),
    UnsupportedPreset(VirtualDisplayPreset),
    UnsupportedSchemaVersion(u32),
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for ProToolsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Privacy(err) => write!(f, "privacy mode error: {err}"),
            Self::Color(err) => write!(f, "color profile error: {err}"),
            Self::VirtualDisplay(err) => write!(f, "virtual display error: {err}"),
            Self::PortForward(err) => write!(f, "port forward rule error: {err}"),
            Self::DuplicateDisplayName(name) => {
                write!(f, "virtual display profile named '{name}' already exists")
            }
            Self::TooManyVirtualDisplays { count, max } => {
                write!(f, "virtual display count ({count}) exceeds maximum ({max})")
            }
            Self::DisplayIndexOutOfBounds { index, len } => {
                write!(f, "virtual display index {index} out of bounds (len {len})")
            }
            Self::PortForwardRuleNotFound(id) => {
                write!(f, "port forwarding rule '{id}' not found")
            }
            Self::KeymapNotFound(id) => write!(f, "keymap preset '{id}' not found"),
            Self::UnsupportedPreset(preset) => {
                write!(f, "virtual display preset {preset:?} has no static dimensions")
            }
            Self::UnsupportedSchemaVersion(v) => write!(
                f,
                "unsupported pro tools preferences schema version {v} (expected {PRO_TOOLS_PREFS_SCHEMA_VERSION})"
            ),
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::Json(err) => write!(f, "JSON error: {err}"),
        }
    }
}

impl Error for ProToolsError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Privacy(err) => Some(err),
            Self::Color(err) => Some(err),
            Self::VirtualDisplay(err) => Some(err),
            Self::PortForward(err) => Some(err),
            Self::Io(err) => Some(err),
            Self::Json(err) => Some(err),
            Self::DuplicateDisplayName(_)
            | Self::TooManyVirtualDisplays { .. }
            | Self::DisplayIndexOutOfBounds { .. }
            | Self::PortForwardRuleNotFound(_)
            | Self::KeymapNotFound(_)
            | Self::UnsupportedPreset(_)
            | Self::UnsupportedSchemaVersion(_) => None,
        }
    }
}

impl From<PrivacyModeError> for ProToolsError {
    fn from(value: PrivacyModeError) -> Self {
        Self::Privacy(value)
    }
}

impl From<ColorProfileError> for ProToolsError {
    fn from(value: ColorProfileError) -> Self {
        Self::Color(value)
    }
}

impl From<VirtualDisplayError> for ProToolsError {
    fn from(value: VirtualDisplayError) -> Self {
        Self::VirtualDisplay(value)
    }
}

impl From<RuleValidationError> for ProToolsError {
    fn from(value: RuleValidationError) -> Self {
        Self::PortForward(value)
    }
}

impl From<std::io::Error> for ProToolsError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for ProToolsError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

/// Damped harmonic spring scalar for fluid desktop drawer and floating capsule animations.
///
/// Supports both critically damped (`damping == 2 * sqrt(stiffness)`) and
/// under-damped (`damping < 2 * sqrt(stiffness)`) motion using sub-stepped
/// semi-implicit Euler integration for numerical stability across variable frame rates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringFloat {
    value: f32,
    target: f32,
    velocity: f32,
    stiffness: f32,
    damping: f32,
    epsilon: f32,
}

impl SpringFloat {
    /// Creates a spring initialized at `initial` with explicit `stiffness` (`k`) and `damping` (`c`).
    pub fn new(initial: f32, stiffness: f32, damping: f32) -> Self {
        let initial = if initial.is_finite() { initial } else { 0.0 };
        let stiffness = if stiffness.is_finite() {
            stiffness.max(1.0)
        } else {
            280.0
        };
        let damping = if damping.is_finite() {
            damping.max(0.1)
        } else {
            2.0 * stiffness.sqrt()
        };
        Self {
            value: initial,
            target: initial,
            velocity: 0.0,
            stiffness,
            damping,
            epsilon: 1e-4,
        }
    }

    /// Creates a critically damped spring (`zeta = 1.0`) for crisp, zero-overshoot drawer motion.
    pub fn critically_damped(initial: f32, stiffness: f32) -> Self {
        let k = if stiffness.is_finite() {
            stiffness.max(1.0)
        } else {
            320.0
        };
        let c = 2.0 * k.sqrt();
        Self::new(initial, k, c)
    }

    /// Creates an under-damped spring (`0 < zeta < 1.0`) for lively capsule/badge motion.
    pub fn under_damped(initial: f32, stiffness: f32, damping_ratio: f32) -> Self {
        let k = if stiffness.is_finite() {
            stiffness.max(1.0)
        } else {
            360.0
        };
        let zeta = if damping_ratio.is_finite() {
            damping_ratio.clamp(0.15, 0.99)
        } else {
            0.72
        };
        let c = 2.0 * zeta * k.sqrt();
        Self::new(initial, k, c)
    }

    /// Default critically damped spring tuned for the desktop Pro Tools side drawer.
    pub fn drawer_default() -> Self {
        Self::critically_damped(0.0, 340.0)
    }

    /// Default slightly under-damped spring tuned for the floating Pro Tools status capsule.
    pub fn capsule_default() -> Self {
        Self::under_damped(0.0, 380.0, 0.72)
    }

    /// Current interpolated scalar value.
    pub const fn value(&self) -> f32 {
        self.value
    }

    /// Current target scalar value.
    pub const fn target(&self) -> f32 {
        self.target
    }

    /// Current instantaneous velocity (units/sec).
    pub const fn velocity(&self) -> f32 {
        self.velocity
    }

    /// Spring stiffness `k`.
    pub const fn stiffness(&self) -> f32 {
        self.stiffness
    }

    /// Spring damping `c`.
    pub const fn damping(&self) -> f32 {
        self.damping
    }

    /// Damping ratio `zeta = c / (2 * sqrt(k))`.
    pub fn damping_ratio(&self) -> f32 {
        self.damping / (2.0 * self.stiffness.sqrt())
    }

    /// Updates the target value without resetting velocity.
    pub fn set_target(&mut self, target: f32) {
        if target.is_finite() {
            self.target = target;
        }
    }

    /// Immediately jumps to `value` and zeroes velocity.
    pub fn snap(&mut self, value: f32) {
        if value.is_finite() {
            self.value = value;
            self.target = value;
            self.velocity = 0.0;
        }
    }

    /// Returns `true` when both position error and velocity are within the settle epsilon.
    pub fn is_settled(&self) -> bool {
        (self.value - self.target).abs() <= self.epsilon && self.velocity.abs() <= self.epsilon
    }

    /// Advances the spring by `dt_secs` seconds and returns the updated value.
    pub fn step(&mut self, dt_secs: f32) -> f32 {
        if !dt_secs.is_finite() || dt_secs <= 0.0 {
            return self.value;
        }
        if self.is_settled() {
            self.value = self.target;
            self.velocity = 0.0;
            return self.value;
        }

        let mut remaining = dt_secs.min(MAX_SPRING_STEP_SECS);
        while remaining > 0.0 {
            let dt = remaining.min(MAX_SPRING_SUBSTEP_SECS);
            let displacement = self.value - self.target;
            let accel = -self.stiffness * displacement - self.damping * self.velocity;
            self.velocity += accel * dt;
            self.value += self.velocity * dt;
            remaining -= dt;
        }

        if self.is_settled() {
            self.value = self.target;
            self.velocity = 0.0;
        }
        self.value
    }
}

/// Live telemetry and runtime context passed into one-click redacted diagnostics export.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiagnosticsExportContext {
    pub app_version: String,
    pub platform: String,
    pub active_scope: String,
    pub relay_connected: bool,
    pub p2p_state: String,
    pub rtt_ms: f32,
    pub packet_loss_percent: f32,
    pub fps: f32,
    pub video_bitrate_kbps: u32,
    pub encoder_profile: String,
    pub recent_events: Vec<String>,
    pub generated_at_unix_ms: Option<u64>,
}

/// Serializable user preference snapshot stored on disk by [`DesktopProToolsState`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesktopProToolsPreferences {
    #[serde(default = "default_prefs_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub privacy_config: PrivacyModeConfig,
    #[serde(default)]
    pub color_profile: ColorProfile,
    #[serde(default)]
    pub route_mode: DesktopRouteMode,
    #[serde(default)]
    pub virtual_displays: Vec<VirtualDisplaySpec>,
    #[serde(default)]
    pub port_forward_rules: Vec<PortForwardRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_keymap_id: Option<String>,
}

fn default_prefs_schema_version() -> u32 {
    PRO_TOOLS_PREFS_SCHEMA_VERSION
}

impl Default for DesktopProToolsPreferences {
    fn default() -> Self {
        Self {
            schema_version: PRO_TOOLS_PREFS_SCHEMA_VERSION,
            privacy_config: PrivacyModeConfig::default(),
            color_profile: ColorProfile::default(),
            route_mode: DesktopRouteMode::default(),
            virtual_displays: Vec::new(),
            port_forward_rules: Vec::new(),
            selected_keymap_id: None,
        }
    }
}

/// Unified Desktop Pro Tools control panel state.
#[derive(Debug, Clone)]
pub struct DesktopProToolsState {
    // Privacy mode
    pub privacy_config: PrivacyModeConfig,
    pub privacy_active: bool,

    // Video color profile (4:2:0 / 4:4:4 + SDR / HDR10)
    pub color_profile: ColorProfile,

    // Network route preference
    pub route_mode: DesktopRouteMode,

    // Virtual display profiles
    pub virtual_displays: Vec<VirtualDisplaySpec>,

    // Multiplexed port forwarding rules
    pub port_forward_rules: Vec<PortForwardRule>,

    // Keymap catalog & search state
    pub keymap_catalog: KeymapCatalog,
    pub keymap_query: String,
    pub keymap_category_filter: Option<KeymapCategory>,
    pub selected_keymap_id: Option<String>,

    // Unified panel UI & spring-interpolated animation state
    pub panel_open: bool,
    pub active_tab: ProToolsTab,
    pub drawer_motion: SpringFloat,
    pub capsule_motion: SpringFloat,
}

impl Default for DesktopProToolsState {
    fn default() -> Self {
        Self::new()
    }
}

impl DesktopProToolsState {
    /// Creates a new Desktop Pro Tools state preloaded with the built-in 500+ keymap catalog.
    pub fn new() -> Self {
        Self {
            privacy_config: PrivacyModeConfig::default(),
            privacy_active: false,
            color_profile: ColorProfile::default(),
            route_mode: DesktopRouteMode::Auto,
            virtual_displays: Vec::new(),
            port_forward_rules: Vec::new(),
            keymap_catalog: KeymapCatalog::builtin(),
            keymap_query: String::new(),
            keymap_category_filter: None,
            selected_keymap_id: None,
            panel_open: false,
            active_tab: ProToolsTab::Privacy,
            drawer_motion: SpringFloat::drawer_default(),
            capsule_motion: SpringFloat::capsule_default(),
        }
    }

    // -----------------------------------------------------------------------
    // Panel & spring animation controls
    // -----------------------------------------------------------------------

    /// Opens or closes the Pro Tools drawer and updates spring targets accordingly.
    pub fn set_panel_open(&mut self, open: bool) {
        self.panel_open = open;
        self.drawer_motion.set_target(if open { 1.0 } else { 0.0 });
        self.capsule_motion.set_target(if open { 1.0 } else { 0.0 });
    }

    /// Toggles the Pro Tools drawer open/closed state.
    pub fn toggle_panel(&mut self) -> bool {
        let next = !self.panel_open;
        self.set_panel_open(next);
        next
    }

    /// Advances all panel spring animations by `dt_secs` seconds.
    pub fn step_animations(&mut self, dt_secs: f32) {
        self.drawer_motion.step(dt_secs);
        self.capsule_motion.step(dt_secs);
    }

    /// Returns `true` when all panel springs have settled at their targets.
    pub fn animations_settled(&self) -> bool {
        self.drawer_motion.is_settled() && self.capsule_motion.is_settled()
    }

    // -----------------------------------------------------------------------
    // Privacy mode controls
    // -----------------------------------------------------------------------

    /// Validates and updates the stored [`PrivacyModeConfig`].
    ///
    /// If `config.enabled` is `false`, live `privacy_active` is also cleared.
    pub fn set_privacy_config(&mut self, config: PrivacyModeConfig) -> Result<(), ProToolsError> {
        config.validate()?;
        if !config.enabled {
            self.privacy_active = false;
        }
        self.privacy_config = config;
        Ok(())
    }

    /// Sets live privacy mode active state after validating the current configuration.
    pub fn set_privacy_active(&mut self, active: bool) -> Result<bool, ProToolsError> {
        if active {
            self.privacy_config.validate()?;
            self.privacy_config.enabled = true;
            self.privacy_active = true;
        } else {
            self.privacy_active = false;
        }
        Ok(self.privacy_active)
    }

    /// Toggles live privacy mode active state.
    pub fn toggle_privacy_active(&mut self) -> Result<bool, ProToolsError> {
        self.set_privacy_active(!self.privacy_active)
    }

    // -----------------------------------------------------------------------
    // Color profile controls (4:2:0 / 4:4:4 + SDR / HDR10)
    // -----------------------------------------------------------------------

    /// Validates and applies a complete [`ColorProfile`].
    pub fn set_color_profile(&mut self, profile: ColorProfile) -> Result<(), ProToolsError> {
        profile.validate()?;
        self.color_profile = profile;
        Ok(())
    }

    /// Switches chroma subsampling between YUV 4:2:0 and YUV 4:4:4 while preserving SDR/HDR10.
    pub fn set_chroma_subsampling(&mut self, chroma: ChromaSubsampling) {
        self.color_profile.chroma = chroma;
    }

    /// Convenience toggle for YUV 4:4:4 true-color mode.
    pub fn set_yuv444(&mut self, enabled: bool) {
        self.set_chroma_subsampling(if enabled {
            ChromaSubsampling::Yuv444
        } else {
            ChromaSubsampling::Yuv420
        });
    }

    /// Switches between SDR (8-bit BT.709) and HDR10 (10-bit PQ ST 2084 + BT.2020)
    /// while preserving the currently selected chroma subsampling (4:2:0 or 4:4:4).
    pub fn set_hdr10_enabled(&mut self, enabled: bool) {
        let chroma = self.color_profile.chroma;
        self.color_profile = if enabled {
            ColorProfile {
                chroma,
                bit_depth: ColorBitDepth::Bit10,
                transfer: HdrTransfer::PqSt2084,
                primaries: ColorPrimaries::Bt2020,
                hdr_metadata: Some(HdrMasteringMetadata::hdr10_default()),
            }
        } else {
            ColorProfile {
                chroma,
                bit_depth: ColorBitDepth::Bit8,
                transfer: HdrTransfer::Sdr,
                primaries: ColorPrimaries::Bt709,
                hdr_metadata: None,
            }
        };
    }

    // -----------------------------------------------------------------------
    // Route mode preference
    // -----------------------------------------------------------------------

    /// Updates the preferred desktop transport route mode.
    pub fn set_route_mode(&mut self, mode: DesktopRouteMode) {
        self.route_mode = mode;
    }

    // -----------------------------------------------------------------------
    // Virtual display profiles & quick presets
    // -----------------------------------------------------------------------

    /// Returns the quick presets (1080p / 2K / 4K / 21:9 / 32:9).
    pub const fn quick_virtual_display_presets() -> &'static [VirtualDisplayPreset] {
        &QUICK_VIRTUAL_DISPLAY_PRESETS
    }

    /// Validates and adds a custom virtual display profile.
    pub fn add_virtual_display(&mut self, spec: VirtualDisplaySpec) -> Result<(), ProToolsError> {
        spec.validate()?;
        if self.virtual_displays.len() >= MAX_SNAPSHOT_DISPLAYS {
            return Err(ProToolsError::TooManyVirtualDisplays {
                count: self.virtual_displays.len() + 1,
                max: MAX_SNAPSHOT_DISPLAYS,
            });
        }
        let target_name = spec.name.trim().to_ascii_lowercase();
        if self
            .virtual_displays
            .iter()
            .any(|existing| existing.name.trim().to_ascii_lowercase() == target_name)
        {
            return Err(ProToolsError::DuplicateDisplayName(spec.name));
        }
        self.virtual_displays.push(spec);
        Ok(())
    }

    /// Adds a quick virtual display preset (1080p / 2K / 4K / 21:9 / 32:9).
    ///
    /// If a display with the preset's default name already exists, automatically appends
    /// a numeric suffix (`" #2"`, `" #3"`, ...) so repeated quick-preset clicks succeed.
    pub fn add_virtual_display_preset(
        &mut self,
        preset: VirtualDisplayPreset,
        refresh_hz: u16,
        placement: DisplayPlacement,
    ) -> Result<&VirtualDisplaySpec, ProToolsError> {
        let mut spec = preset
            .to_spec(refresh_hz, placement)
            .ok_or(ProToolsError::UnsupportedPreset(preset))?;
        let base_name = spec.name.clone();
        if self.has_display_name(&base_name) {
            let mut suffix = 2usize;
            loop {
                let candidate = format!("{base_name} #{suffix}");
                if !self.has_display_name(&candidate) {
                    spec.name = candidate;
                    break;
                }
                suffix += 1;
            }
        }
        self.add_virtual_display(spec)?;
        Ok(self.virtual_displays.last().expect("just pushed"))
    }

    /// Validates and replaces the virtual display profile at `index`.
    pub fn update_virtual_display(
        &mut self,
        index: usize,
        spec: VirtualDisplaySpec,
    ) -> Result<(), ProToolsError> {
        if index >= self.virtual_displays.len() {
            return Err(ProToolsError::DisplayIndexOutOfBounds {
                index,
                len: self.virtual_displays.len(),
            });
        }
        spec.validate()?;
        let target_name = spec.name.trim().to_ascii_lowercase();
        if self
            .virtual_displays
            .iter()
            .enumerate()
            .any(|(i, existing)| {
                i != index && existing.name.trim().to_ascii_lowercase() == target_name
            })
        {
            return Err(ProToolsError::DuplicateDisplayName(spec.name));
        }
        self.virtual_displays[index] = spec;
        Ok(())
    }

    /// Removes the virtual display profile at `index`, returning it if present.
    pub fn remove_virtual_display(&mut self, index: usize) -> Option<VirtualDisplaySpec> {
        if index < self.virtual_displays.len() {
            Some(self.virtual_displays.remove(index))
        } else {
            None
        }
    }

    /// Removes the first virtual display profile matching `name` (case-insensitive).
    pub fn remove_virtual_display_by_name(&mut self, name: &str) -> Option<VirtualDisplaySpec> {
        let needle = name.trim().to_ascii_lowercase();
        let pos = self
            .virtual_displays
            .iter()
            .position(|d| d.name.trim().to_ascii_lowercase() == needle)?;
        Some(self.virtual_displays.remove(pos))
    }

    fn has_display_name(&self, name: &str) -> bool {
        let needle = name.trim().to_ascii_lowercase();
        self.virtual_displays
            .iter()
            .any(|d| d.name.trim().to_ascii_lowercase() == needle)
    }

    // -----------------------------------------------------------------------
    // Port forwarding rules CRUD & validation
    // -----------------------------------------------------------------------

    /// Validates and adds a [`PortForwardRule`], enforcing rule-set uniqueness, loop guard,
    /// active bind address conflict checks, and `MAX_ACTIVE_RULES`.
    pub fn add_port_forward_rule(&mut self, rule: PortForwardRule) -> Result<(), ProToolsError> {
        let mut candidate = self.port_forward_rules.clone();
        candidate.push(rule);
        validate_rules(&candidate, MAX_ACTIVE_RULES)?;
        self.port_forward_rules = candidate;
        Ok(())
    }

    /// Toggles the `enabled` flag of the rule with `rule_id`, validating the full rule set
    /// before committing. Returns the new `enabled` state.
    pub fn toggle_port_forward_rule(&mut self, rule_id: &str) -> Result<bool, ProToolsError> {
        let current = self
            .port_forward_rules
            .iter()
            .find(|r| r.id == rule_id)
            .map(|r| r.enabled)
            .ok_or_else(|| ProToolsError::PortForwardRuleNotFound(rule_id.to_string()))?;
        self.set_port_forward_rule_enabled(rule_id, !current)
    }

    /// Explicitly sets `enabled` on `rule_id`, validating the resulting rule set first.
    pub fn set_port_forward_rule_enabled(
        &mut self,
        rule_id: &str,
        enabled: bool,
    ) -> Result<bool, ProToolsError> {
        let mut candidate = self.port_forward_rules.clone();
        let Some(target) = candidate.iter_mut().find(|r| r.id == rule_id) else {
            return Err(ProToolsError::PortForwardRuleNotFound(rule_id.to_string()));
        };
        target.enabled = enabled;
        validate_rules(&candidate, MAX_ACTIVE_RULES)?;
        self.port_forward_rules = candidate;
        Ok(enabled)
    }

    /// Removes the port forwarding rule with `rule_id`, returning it if found.
    pub fn remove_port_forward_rule(&mut self, rule_id: &str) -> Option<PortForwardRule> {
        let pos = self.port_forward_rules.iter().position(|r| r.id == rule_id)?;
        Some(self.port_forward_rules.remove(pos))
    }

    /// Returns the number of currently enabled port forwarding rules.
    pub fn active_port_forward_count(&self) -> usize {
        self.port_forward_rules.iter().filter(|r| r.enabled).count()
    }

    // -----------------------------------------------------------------------
    // Keymap catalog search & selection
    // -----------------------------------------------------------------------

    /// Updates the live keymap search query.
    pub fn set_keymap_query(&mut self, query: impl Into<String>) {
        self.keymap_query = query.into();
    }

    /// Updates the optional keymap category filter.
    pub fn set_keymap_category_filter(&mut self, category: Option<KeymapCategory>) {
        self.keymap_category_filter = category;
    }

    /// Returns keymap presets matching the current search query and optional category filter.
    pub fn filtered_keymaps(&self) -> Vec<&KeymapProfile> {
        let results = self.keymap_catalog.search(&self.keymap_query);
        match self.keymap_category_filter {
            Some(cat) => results.into_iter().filter(|p| p.category == cat).collect(),
            None => results,
        }
    }

    /// Selects a keymap preset by exact ID from the catalog.
    pub fn select_keymap(&mut self, id: &str) -> Result<&KeymapProfile, ProToolsError> {
        if self.keymap_catalog.get(id).is_none() {
            return Err(ProToolsError::KeymapNotFound(id.to_string()));
        }
        self.selected_keymap_id = Some(id.to_string());
        Ok(self.keymap_catalog.get(id).expect("verified exists"))
    }

    /// Returns the currently selected keymap preset, if any.
    pub fn selected_keymap(&self) -> Option<&KeymapProfile> {
        self.selected_keymap_id
            .as_deref()
            .and_then(|id| self.keymap_catalog.get(id))
    }

    /// Matches a running process name or path against the keymap catalog.
    pub fn match_keymaps_for_process(&self, process_name: &str) -> Vec<&KeymapProfile> {
        self.keymap_catalog.match_process(process_name)
    }

    // -----------------------------------------------------------------------
    // One-click redacted diagnostics builder
    // -----------------------------------------------------------------------

    /// Builds a structured [`DiagnosticBundle`] combining current Pro Tools state and live telemetry.
    pub fn build_diagnostic_bundle(&self, ctx: &DiagnosticsExportContext) -> DiagnosticBundle {
        let generated_at_unix_ms = ctx.generated_at_unix_ms.unwrap_or_else(now_unix_ms);
        let app_version = if ctx.app_version.trim().is_empty() {
            env!("CARGO_PKG_VERSION").to_string()
        } else {
            ctx.app_version.clone()
        };
        let platform = if ctx.platform.trim().is_empty() {
            format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
        } else {
            ctx.platform.clone()
        };

        DiagnosticBundle {
            schema_version: DIAGNOSTIC_BUNDLE_SCHEMA_VERSION,
            generated_at_unix_ms,
            app_version,
            platform,
            transport: DiagnosticTransportSummary {
                route_mode: self.route_mode.as_str().to_string(),
                active_scope: ctx.active_scope.clone(),
                relay_connected: ctx.relay_connected,
                p2p_state: ctx.p2p_state.clone(),
                rtt_ms: ctx.rtt_ms,
                packet_loss_percent: ctx.packet_loss_percent,
                fps: ctx.fps,
                video_bitrate_kbps: ctx.video_bitrate_kbps,
            },
            session: DiagnosticSessionSummary {
                encoder_profile: ctx.encoder_profile.clone(),
                chroma: self.color_profile.chroma.to_string(),
                hdr_mode: format!(
                    "{} {} {}",
                    self.color_profile.transfer,
                    self.color_profile.bit_depth,
                    self.color_profile.primaries
                ),
                virtual_displays_active: self.virtual_displays.len() as u32,
                port_forwards_active: self.active_port_forward_count() as u32,
                privacy_mode_active: self.privacy_active,
            },
            recent_events: ctx.recent_events.clone(),
        }
    }

    /// One-click redacted diagnostics JSON export powered by [`DiagnosticBundle::to_redacted_json`].
    ///
    /// Guarantees all credentials, tokens, PEM blocks, and non-loopback IP addresses are
    /// scrubbed and the payload is bounded to `MAX_DIAGNOSTIC_BUNDLE_BYTES`.
    pub fn build_redacted_diagnostics_json(&self, ctx: &DiagnosticsExportContext) -> String {
        let bundle = self.build_diagnostic_bundle(ctx);
        bundle
            .to_redacted_json()
            .unwrap_or_else(|_| r#"{"schema_version":1,"error":"serialization_failed"}"#.to_string())
    }

    // -----------------------------------------------------------------------
    // JSON persistence
    // -----------------------------------------------------------------------

    /// Extracts the persistent user preferences snapshot from this state.
    pub fn to_preferences(&self) -> DesktopProToolsPreferences {
        DesktopProToolsPreferences {
            schema_version: PRO_TOOLS_PREFS_SCHEMA_VERSION,
            privacy_config: self.privacy_config.clone(),
            color_profile: self.color_profile,
            route_mode: self.route_mode,
            virtual_displays: self.virtual_displays.clone(),
            port_forward_rules: self.port_forward_rules.clone(),
            selected_keymap_id: self.selected_keymap_id.clone(),
        }
    }

    /// Reconstructs a validated [`DesktopProToolsState`] from a [`DesktopProToolsPreferences`] snapshot.
    pub fn from_preferences(prefs: DesktopProToolsPreferences) -> Result<Self, ProToolsError> {
        if prefs.schema_version != PRO_TOOLS_PREFS_SCHEMA_VERSION {
            return Err(ProToolsError::UnsupportedSchemaVersion(prefs.schema_version));
        }
        prefs.privacy_config.validate()?;
        prefs.color_profile.validate()?;
        validate_rules(&prefs.port_forward_rules, MAX_ACTIVE_RULES)?;

        let mut state = Self::new();
        state.privacy_config = prefs.privacy_config;
        state.color_profile = prefs.color_profile;
        state.route_mode = prefs.route_mode;
        for spec in prefs.virtual_displays {
            state.add_virtual_display(spec)?;
        }
        state.port_forward_rules = prefs.port_forward_rules;
        if let Some(keymap_id) = prefs.selected_keymap_id
            && state.keymap_catalog.get(&keymap_id).is_some()
        {
            state.selected_keymap_id = Some(keymap_id);
        }
        Ok(state)
    }

    /// Persists user preferences as pretty-printed JSON to `path` (creating parent directories).
    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<(), ProToolsError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let prefs = self.to_preferences();
        let json = serde_json::to_string_pretty(&prefs)?;
        fs::write(path, json)?;
        Ok(())
    }

    /// Loads and validates user preferences from JSON at `path`.
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, ProToolsError> {
        let raw = fs::read_to_string(path.as_ref())?;
        let prefs: DesktopProToolsPreferences = serde_json::from_str(&raw)?;
        Self::from_preferences(prefs)
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use host::privacy_mode::PrivacyCurtainMode;
    use remote_core::diagnostics_bundle::MAX_DIAGNOSTIC_BUNDLE_BYTES;
    use remote_core::port_forward::{TunnelDirection, TunnelProtocol};
    use tempfile::tempdir;

    fn sample_rule(id: &str, bind_port: u16, target_port: u16, enabled: bool) -> PortForwardRule {
        PortForwardRule {
            id: id.to_string(),
            name: format!("Rule {id}"),
            protocol: TunnelProtocol::Tcp,
            direction: TunnelDirection::LocalToRemote,
            bind_host: "127.0.0.1".to_string(),
            bind_port,
            target_host: "127.0.0.1".to_string(),
            target_port,
            enabled,
        }
    }

    #[test]
    fn privacy_config_validation_and_live_toggle() {
        let mut state = DesktopProToolsState::new();
        assert!(!state.privacy_active);

        // Invalid hotkey rejected without mutating stored config.
        let bad_hotkey = PrivacyModeConfig {
            enabled: true,
            emergency_hotkey: "Escape".to_string(),
            ..PrivacyModeConfig::default()
        };
        assert!(matches!(
            state.set_privacy_config(bad_hotkey),
            Err(ProToolsError::Privacy(PrivacyModeError::InvalidConfig(_)))
        ));

        // Invalid custom curtain (empty title) rejected.
        let bad_curtain = PrivacyModeConfig {
            enabled: true,
            mode: PrivacyCurtainMode::CustomCurtain {
                title: "   ".to_string(),
                subtitle: "sub".to_string(),
                image_path: None,
            },
            ..PrivacyModeConfig::default()
        };
        assert!(matches!(
            state.set_privacy_config(bad_curtain),
            Err(ProToolsError::Privacy(PrivacyModeError::InvalidConfig(_)))
        ));

        // Valid custom curtain accepted and live active toggle works.
        let valid_curtain = PrivacyModeConfig {
            enabled: true,
            mode: PrivacyCurtainMode::CustomCurtain {
                title: "Remote Session Active".to_string(),
                subtitle: "Press Ctrl+Alt+Shift+P to exit".to_string(),
                image_path: Some("curtain.png".to_string()),
            },
            block_local_input: true,
            auto_restore_after_disconnect_ms: 10_000,
            emergency_hotkey: "Ctrl+Alt+Shift+P".to_string(),
        };
        state.set_privacy_config(valid_curtain.clone()).unwrap();
        assert_eq!(state.privacy_config, valid_curtain);

        assert!(state.toggle_privacy_active().unwrap());
        assert!(state.privacy_active);

        // Disabling via config automatically clears live active state.
        let mut disabled = valid_curtain;
        disabled.enabled = false;
        state.set_privacy_config(disabled).unwrap();
        assert!(!state.privacy_active);
    }

    #[test]
    fn color_profile_switching_420_444_and_sdr_hdr10() {
        let mut state = DesktopProToolsState::new();
        assert_eq!(state.color_profile, ColorProfile::sdr_420());

        // Toggle 4:4:4 in SDR mode.
        state.set_yuv444(true);
        assert_eq!(state.color_profile, ColorProfile::sdr_444());
        assert!(state.color_profile.validate().is_ok());

        // Enable HDR10 -> preserves 4:4:4 and upgrades to 10-bit PQ BT.2020 with metadata.
        state.set_hdr10_enabled(true);
        assert_eq!(state.color_profile.chroma, ChromaSubsampling::Yuv444);
        assert_eq!(state.color_profile.bit_depth, ColorBitDepth::Bit10);
        assert_eq!(state.color_profile.transfer, HdrTransfer::PqSt2084);
        assert_eq!(state.color_profile.primaries, ColorPrimaries::Bt2020);
        assert!(state.color_profile.hdr_metadata.is_some());
        assert!(state.color_profile.validate().is_ok());

        // Switch back to 4:2:0 while staying in HDR10.
        state.set_yuv444(false);
        assert_eq!(state.color_profile.chroma, ChromaSubsampling::Yuv420);
        assert!(state.color_profile.is_hdr());
        assert!(state.color_profile.validate().is_ok());

        // Disable HDR10 -> returns to 8-bit SDR BT.709 4:2:0 with no HDR metadata.
        state.set_hdr10_enabled(false);
        assert_eq!(state.color_profile, ColorProfile::sdr_420());

        // Invalid custom ColorProfile (8-bit HDR) is rejected.
        let invalid = ColorProfile {
            chroma: ChromaSubsampling::Yuv444,
            bit_depth: ColorBitDepth::Bit8,
            transfer: HdrTransfer::PqSt2084,
            primaries: ColorPrimaries::Bt2020,
            hdr_metadata: None,
        };
        assert!(matches!(
            state.set_color_profile(invalid),
            Err(ProToolsError::Color(ColorProfileError::HdrRequires10Bit { .. }))
        ));
    }

    #[test]
    fn virtual_display_crud_and_quick_presets() {
        let mut state = DesktopProToolsState::new();
        assert_eq!(DesktopProToolsState::quick_virtual_display_presets().len(), 5);

        // Add all 5 quick presets (1080p / 2K / 4K / 21:9 / 32:9).
        for &preset in DesktopProToolsState::quick_virtual_display_presets() {
            let spec = state
                .add_virtual_display_preset(preset, 120, DisplayPlacement::RightOf)
                .expect("quick preset should succeed");
            assert_eq!(spec.refresh_hz, 120);
            assert!(spec.validate().is_ok());
        }
        assert_eq!(state.virtual_displays.len(), 5);
        assert_eq!(state.virtual_displays[0].width, 1920);
        assert_eq!(state.virtual_displays[1].width, 2560);
        assert_eq!(state.virtual_displays[2].width, 3840);
        assert_eq!(state.virtual_displays[3].width, 3440);
        assert_eq!(state.virtual_displays[4].width, 5120);

        // Repeated quick preset auto-disambiguates the display name.
        let second_4k = state
            .add_virtual_display_preset(VirtualDisplayPreset::Uhd4k, 60, DisplayPlacement::LeftOf)
            .unwrap()
            .clone();
        assert_eq!(second_4k.name, "Weavelo 4K #2");

        // MatchClient has no static dimensions -> UnsupportedPreset error.
        assert!(matches!(
            state.add_virtual_display_preset(
                VirtualDisplayPreset::MatchClient,
                60,
                DisplayPlacement::RightOf
            ),
            Err(ProToolsError::UnsupportedPreset(
                VirtualDisplayPreset::MatchClient
            ))
        ));

        // Reject odd dimensions and duplicate names on custom add.
        let odd_spec = VirtualDisplaySpec {
            name: "Odd".to_string(),
            width: 1921,
            height: 1080,
            refresh_hz: 60,
            dpi_scale: 1.0,
            hdr: false,
            placement: DisplayPlacement::Standalone,
        };
        assert!(matches!(
            state.add_virtual_display(odd_spec),
            Err(ProToolsError::VirtualDisplay(
                VirtualDisplayError::OddDimensions { .. }
            ))
        ));

        let dup_spec = state.virtual_displays[0].clone();
        assert!(matches!(
            state.add_virtual_display(dup_spec),
            Err(ProToolsError::DuplicateDisplayName(_))
        ));

        // Update and remove.
        let mut updated = state.virtual_displays[0].clone();
        updated.refresh_hz = 144;
        updated.hdr = true;
        state.update_virtual_display(0, updated.clone()).unwrap();
        assert_eq!(state.virtual_displays[0], updated);

        let removed = state.remove_virtual_display_by_name("Weavelo 4K #2");
        assert!(removed.is_some());
        assert_eq!(state.virtual_displays.len(), 5);

        assert!(state.remove_virtual_display(0).is_some());
        assert_eq!(state.virtual_displays.len(), 4);
    }

    #[test]
    fn port_forward_rule_crud_and_validation() {
        let mut state = DesktopProToolsState::new();

        // Loopback self-forward (same bind & target port on 127.0.0.1) is rejected.
        let loop_rule = sample_rule("loop", 8080, 8080, true);
        assert!(matches!(
            state.add_port_forward_rule(loop_rule),
            Err(ProToolsError::PortForward(
                RuleValidationError::LoopbackSelfForward { .. }
            ))
        ));

        // Add two valid rules sharing a bind port, with the second disabled.
        state
            .add_port_forward_rule(sample_rule("web-a", 18080, 8080, true))
            .unwrap();
        state
            .add_port_forward_rule(sample_rule("web-b", 18080, 8081, false))
            .unwrap();
        assert_eq!(state.port_forward_rules.len(), 2);
        assert_eq!(state.active_port_forward_count(), 1);

        // Duplicate rule ID rejected.
        assert!(matches!(
            state.add_port_forward_rule(sample_rule("web-a", 18082, 8082, true)),
            Err(ProToolsError::PortForward(
                RuleValidationError::DuplicateRuleId(_)
            ))
        ));

        // Enabling web-b while web-a is active on 127.0.0.1:18080 fails without mutating state.
        assert!(matches!(
            state.toggle_port_forward_rule("web-b"),
            Err(ProToolsError::PortForward(
                RuleValidationError::DuplicateBindAddress { .. }
            ))
        ));
        assert_eq!(state.active_port_forward_count(), 1);

        // Disable web-a, then enable web-b -> succeeds.
        assert!(!state.toggle_port_forward_rule("web-a").unwrap());
        assert!(state.toggle_port_forward_rule("web-b").unwrap());
        assert_eq!(state.active_port_forward_count(), 1);

        // Remove rule.
        let removed = state.remove_port_forward_rule("web-a").unwrap();
        assert_eq!(removed.id, "web-a");
        assert_eq!(state.port_forward_rules.len(), 1);
        assert!(state.remove_port_forward_rule("nonexistent").is_none());
    }

    #[test]
    fn keymap_catalog_search_filter_and_selection() {
        let mut state = DesktopProToolsState::new();
        assert!(state.keymap_catalog.len() >= 500);

        // Search by game name.
        state.set_keymap_query("valorant");
        let results = state.filtered_keymaps();
        assert!(!results.is_empty());
        let first_id = results[0].id.clone();

        // Category filter narrows results.
        state.set_keymap_category_filter(Some(KeymapCategory::Racing));
        assert!(state.filtered_keymaps().is_empty());

        state.set_keymap_category_filter(Some(KeymapCategory::Fps));
        assert!(!state.filtered_keymaps().is_empty());

        // Selecting a valid preset stores it; invalid ID returns error.
        let selected = state.select_keymap(&first_id).unwrap();
        assert_eq!(selected.id, first_id);
        assert_eq!(state.selected_keymap().map(|p| p.id.as_str()), Some(first_id.as_str()));
        assert!(matches!(
            state.select_keymap("does-not-exist-keymap"),
            Err(ProToolsError::KeymapNotFound(_))
        ));
    }

    #[test]
    fn redacted_diagnostics_export_scrubs_secrets_and_reflects_state() {
        let mut state = DesktopProToolsState::new();
        state.set_route_mode(DesktopRouteMode::NasWssRelay);
        state.set_yuv444(true);
        state.set_hdr10_enabled(true);
        state.set_privacy_active(true).unwrap();
        state
            .add_virtual_display_preset(VirtualDisplayPreset::Uhd4k, 60, DisplayPlacement::RightOf)
            .unwrap();
        state
            .add_port_forward_rule(sample_rule("ssh", 2222, 22, true))
            .unwrap();

        let ctx = DiagnosticsExportContext {
            app_version: "2.0.0-alpha.9".to_string(),
            platform: "macos-aarch64".to_string(),
            active_scope: "relay".to_string(),
            relay_connected: true,
            p2p_state: "fallback".to_string(),
            rtt_ms: 18.5,
            packet_loss_percent: 0.1,
            fps: 119.8,
            video_bitrate_kbps: 24_000,
            encoder_profile: "hevc_main10_444".to_string(),
            recent_events: vec![
                "connected via https://alice:supersecretpass@203.0.113.42:8443/v1/relay"
                    .to_string(),
                "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcdef123456".to_string(),
                "loopback health check 127.0.0.1:39494 ok".to_string(),
            ],
            generated_at_unix_ms: Some(1_735_000_000_000),
        };

        let json = state.build_redacted_diagnostics_json(&ctx);
        assert!(json.len() <= MAX_DIAGNOSTIC_BUNDLE_BYTES);
        assert!(!json.contains("supersecretpass"));
        assert!(!json.contains("203.0.113.42"));
        assert!(!json.contains("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"));
        // Loopback IP is preserved.
        assert!(json.contains("127.0.0.1"));

        let parsed: DiagnosticBundle = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.transport.route_mode, "nas_wss_relay");
        assert_eq!(parsed.session.chroma, "4:4:4");
        assert!(parsed.session.hdr_mode.contains("PQ (ST 2084)"));
        assert_eq!(parsed.session.virtual_displays_active, 1);
        assert_eq!(parsed.session.port_forwards_active, 1);
        assert!(parsed.session.privacy_mode_active);
    }

    #[test]
    fn spring_animation_convergence_critically_and_under_damped() {
        // Critically damped drawer spring should converge to 1.0 without overshoot.
        let mut drawer = SpringFloat::drawer_default();
        assert!((drawer.damping_ratio() - 1.0).abs() < 1e-4);
        drawer.set_target(1.0);
        let mut max_val = 0.0_f32;
        for _ in 0..240 {
            let v = drawer.step(1.0 / 60.0);
            max_val = max_val.max(v);
        }
        assert!(drawer.is_settled());
        assert_eq!(drawer.value(), 1.0);
        assert!(
            max_val <= 1.001,
            "critically damped spring should not noticeably overshoot, got {max_val}"
        );

        // Under-damped capsule spring should overshoot slightly then settle cleanly.
        let mut capsule = SpringFloat::capsule_default();
        assert!(capsule.damping_ratio() < 1.0);
        capsule.set_target(1.0);
        let mut peak = 0.0_f32;
        for _ in 0..240 {
            let v = capsule.step(1.0 / 60.0);
            peak = peak.max(v);
        }
        assert!(peak > 1.01, "under-damped spring should exhibit gentle overshoot, got {peak}");
        assert!(capsule.is_settled());
        assert_eq!(capsule.value(), 1.0);

        // Panel helper drives both springs in sync.
        let mut state = DesktopProToolsState::new();
        assert!(state.toggle_panel());
        for _ in 0..240 {
            state.step_animations(1.0 / 60.0);
        }
        assert!(state.animations_settled());
        assert_eq!(state.drawer_motion.value(), 1.0);
        assert_eq!(state.capsule_motion.value(), 1.0);
    }

    #[test]
    fn json_persistence_roundtrip_and_validation() {
        let dir = tempdir().unwrap();
        let prefs_path = dir.path().join("nested").join("pro_tools_prefs.json");

        let mut original = DesktopProToolsState::new();
        original
            .set_privacy_config(PrivacyModeConfig {
                enabled: true,
                mode: PrivacyCurtainMode::CustomCurtain {
                    title: "Working Remotely".to_string(),
                    subtitle: "Display shielded".to_string(),
                    image_path: Some("shield.webp".to_string()),
                },
                block_local_input: true,
                auto_restore_after_disconnect_ms: 20_000,
                emergency_hotkey: "Ctrl+Alt+Shift+P".to_string(),
            })
            .unwrap();
        original.set_yuv444(true);
        original.set_hdr10_enabled(true);
        original.set_route_mode(DesktopRouteMode::P2pOnly);
        original
            .add_virtual_display_preset(
                VirtualDisplayPreset::Ultrawide21x9,
                120,
                DisplayPlacement::RightOf,
            )
            .unwrap();
        original
            .add_port_forward_rule(sample_rule("dev-db", 15432, 5432, true))
            .unwrap();

        let keymap_id = original.keymap_catalog.profiles()[0].id.clone();
        original.select_keymap(&keymap_id).unwrap();

        original.save_to_path(&prefs_path).unwrap();

        let loaded = DesktopProToolsState::load_from_path(&prefs_path).unwrap();
        assert_eq!(loaded.privacy_config, original.privacy_config);
        // Transient live active flag resets to false on cold load.
        assert!(!loaded.privacy_active);
        assert_eq!(loaded.color_profile, original.color_profile);
        assert_eq!(loaded.route_mode, DesktopRouteMode::P2pOnly);
        assert_eq!(loaded.virtual_displays, original.virtual_displays);
        assert_eq!(loaded.port_forward_rules, original.port_forward_rules);
        assert_eq!(loaded.selected_keymap_id, Some(keymap_id));
        assert!(loaded.keymap_catalog.len() >= 500);

        // Tampered schema version is rejected on load.
        let bad_path = dir.path().join("bad_version.json");
        fs::write(&bad_path, r#"{"schema_version": 99}"#).unwrap();
        assert!(matches!(
            DesktopProToolsState::load_from_path(&bad_path),
            Err(ProToolsError::UnsupportedSchemaVersion(99))
        ));
    }
}
