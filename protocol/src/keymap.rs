//! Open `keymap-v1` virtual touch-to-keyboard/mouse overlay schema and built-in 500+ preset catalog.
//!
//! Designed for cross-platform sharing between desktop/mobile clients and hosts in JSON format.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::error::Error;
use std::fmt;

/// Current open keymap schema version (`keymap-v1`).
pub const KEYMAP_SCHEMA_VERSION: u32 = 1;

/// Minimum allowed normalized control radius.
pub const MIN_CONTROL_RADIUS_NORM: f32 = 0.02;
/// Maximum allowed normalized control radius.
pub const MAX_CONTROL_RADIUS_NORM: f32 = 0.45;
/// Minimum allowed control opacity.
pub const MIN_CONTROL_OPACITY: f32 = 0.1;
/// Maximum allowed control opacity.
pub const MAX_CONTROL_OPACITY: f32 = 1.0;

/// High-level genre or workflow category for a keymap profile.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum KeymapCategory {
    Fps,
    Moba,
    Rpg,
    Action,
    Racing,
    Simulator,
    Productivity,
    Media,
    Custom,
}

impl KeymapCategory {
    /// Returns all built-in categories.
    pub const fn all() -> &'static [KeymapCategory] {
        &[
            Self::Fps,
            Self::Moba,
            Self::Rpg,
            Self::Action,
            Self::Racing,
            Self::Simulator,
            Self::Productivity,
            Self::Media,
            Self::Custom,
        ]
    }

    /// Lowercase canonical identifier string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fps => "fps",
            Self::Moba => "moba",
            Self::Rpg => "rpg",
            Self::Action => "action",
            Self::Racing => "racing",
            Self::Simulator => "simulator",
            Self::Productivity => "productivity",
            Self::Media => "media",
            Self::Custom => "custom",
        }
    }
}

impl fmt::Display for KeymapCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Interactive behavior of an on-screen virtual control.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum VirtualControlKind {
    /// Directional virtual stick mapped to WASD or arrow keys.
    JoystickWasd,
    /// Relative pointer-look / camera drag region.
    LookPad,
    /// Single key press/hold button.
    KeyButton,
    /// Simultaneous chord / modifier combo (e.g., `Ctrl+S`, `Ctrl+Shift+P`).
    ComboButton,
    /// Repeating macro key burst triggered at `macro_interval_ms`.
    MacroBurst,
    /// Vertical/horizontal virtual scroll wheel strip.
    ScrollWheel,
    /// Mouse button press/hold (`MouseLeft`, `MouseRight`, `MouseMiddle`).
    MouseButton,
}

impl VirtualControlKind {
    /// Whether this control kind requires at least one non-empty key binding.
    pub const fn requires_keys(self) -> bool {
        matches!(self, Self::KeyButton | Self::ComboButton | Self::MacroBurst)
    }
}

/// A single virtual overlay control positioned in normalized `[0.0, 1.0]` screen coordinates.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct KeymapControl {
    pub id: String,
    pub label: String,
    pub kind: VirtualControlKind,
    pub norm_x: f32,
    pub norm_y: f32,
    pub radius_norm: f32,
    pub opacity: f32,
    pub keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macro_interval_ms: Option<u32>,
}

/// Complete `keymap-v1` overlay preset definition.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct KeymapProfile {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub category: KeymapCategory,
    pub process_hints: Vec<String>,
    pub controls: Vec<KeymapControl>,
}

/// Validation failure returned by [`KeymapProfile::validate`].
#[derive(Debug, Clone, PartialEq)]
pub enum KeymapValidationError {
    UnsupportedSchemaVersion(u32),
    EmptyProfileId,
    EmptyProfileName,
    EmptyControlId { index: usize },
    DuplicateControlId(String),
    InvalidCoordinate {
        control_id: String,
        axis: &'static str,
        value: f32,
    },
    InvalidRadius {
        control_id: String,
        value: f32,
    },
    InvalidOpacity {
        control_id: String,
        value: f32,
    },
    MissingKeys {
        control_id: String,
        kind: VirtualControlKind,
    },
    EmptyKeyBinding {
        control_id: String,
    },
    InvalidMacroInterval {
        control_id: String,
        interval_ms: u32,
    },
}

impl fmt::Display for KeymapValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchemaVersion(v) => {
                write!(
                    f,
                    "unsupported keymap schema_version {v} (expected {KEYMAP_SCHEMA_VERSION})"
                )
            }
            Self::EmptyProfileId => f.write_str("keymap profile id must not be empty"),
            Self::EmptyProfileName => f.write_str("keymap profile name must not be empty"),
            Self::EmptyControlId { index } => {
                write!(f, "control at index {index} has an empty id")
            }
            Self::DuplicateControlId(id) => {
                write!(f, "duplicate control id '{id}' in keymap profile")
            }
            Self::InvalidCoordinate {
                control_id,
                axis,
                value,
            } => write!(
                f,
                "control '{control_id}' has out-of-range {axis}={value} (expected 0.0..=1.0)"
            ),
            Self::InvalidRadius { control_id, value } => write!(
                f,
                "control '{control_id}' has out-of-range radius_norm={value} (expected {MIN_CONTROL_RADIUS_NORM}..={MAX_CONTROL_RADIUS_NORM})"
            ),
            Self::InvalidOpacity { control_id, value } => write!(
                f,
                "control '{control_id}' has out-of-range opacity={value} (expected {MIN_CONTROL_OPACITY}..={MAX_CONTROL_OPACITY})"
            ),
            Self::MissingKeys { control_id, kind } => write!(
                f,
                "control '{control_id}' of kind {kind:?} must specify at least one key"
            ),
            Self::EmptyKeyBinding { control_id } => {
                write!(f, "control '{control_id}' contains an empty key binding")
            }
            Self::InvalidMacroInterval {
                control_id,
                interval_ms,
            } => write!(
                f,
                "control '{control_id}' has invalid macro_interval_ms={interval_ms} (must be > 0)"
            ),
        }
    }
}

impl Error for KeymapValidationError {}

/// Combined error type for JSON serialization, deserialization, and validation.
#[derive(Debug)]
pub enum KeymapError {
    Validation(KeymapValidationError),
    Json(serde_json::Error),
}

impl fmt::Display for KeymapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(err) => write!(f, "keymap validation error: {err}"),
            Self::Json(err) => write!(f, "keymap json error: {err}"),
        }
    }
}

impl Error for KeymapError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Validation(err) => Some(err),
            Self::Json(err) => Some(err),
        }
    }
}

impl From<KeymapValidationError> for KeymapError {
    fn from(value: KeymapValidationError) -> Self {
        Self::Validation(value)
    }
}

impl From<serde_json::Error> for KeymapError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl KeymapProfile {
    /// Strictly validates schema version, metadata, geometry bounds, unique control IDs, and key bindings.
    pub fn validate(&self) -> Result<(), KeymapValidationError> {
        if self.schema_version != KEYMAP_SCHEMA_VERSION {
            return Err(KeymapValidationError::UnsupportedSchemaVersion(
                self.schema_version,
            ));
        }
        if self.id.trim().is_empty() {
            return Err(KeymapValidationError::EmptyProfileId);
        }
        if self.name.trim().is_empty() {
            return Err(KeymapValidationError::EmptyProfileName);
        }

        let mut seen_ids = HashSet::with_capacity(self.controls.len());
        for (index, ctrl) in self.controls.iter().enumerate() {
            let cid = ctrl.id.trim();
            if cid.is_empty() {
                return Err(KeymapValidationError::EmptyControlId { index });
            }
            if !seen_ids.insert(cid) {
                return Err(KeymapValidationError::DuplicateControlId(ctrl.id.clone()));
            }
            if !ctrl.norm_x.is_finite() || !(0.0..=1.0).contains(&ctrl.norm_x) {
                return Err(KeymapValidationError::InvalidCoordinate {
                    control_id: ctrl.id.clone(),
                    axis: "norm_x",
                    value: ctrl.norm_x,
                });
            }
            if !ctrl.norm_y.is_finite() || !(0.0..=1.0).contains(&ctrl.norm_y) {
                return Err(KeymapValidationError::InvalidCoordinate {
                    control_id: ctrl.id.clone(),
                    axis: "norm_y",
                    value: ctrl.norm_y,
                });
            }
            if !ctrl.radius_norm.is_finite()
                || !(MIN_CONTROL_RADIUS_NORM..=MAX_CONTROL_RADIUS_NORM).contains(&ctrl.radius_norm)
            {
                return Err(KeymapValidationError::InvalidRadius {
                    control_id: ctrl.id.clone(),
                    value: ctrl.radius_norm,
                });
            }
            if !ctrl.opacity.is_finite()
                || !(MIN_CONTROL_OPACITY..=MAX_CONTROL_OPACITY).contains(&ctrl.opacity)
            {
                return Err(KeymapValidationError::InvalidOpacity {
                    control_id: ctrl.id.clone(),
                    value: ctrl.opacity,
                });
            }
            if ctrl.kind.requires_keys() && ctrl.keys.is_empty() {
                return Err(KeymapValidationError::MissingKeys {
                    control_id: ctrl.id.clone(),
                    kind: ctrl.kind,
                });
            }
            if ctrl.keys.iter().any(|k| k.trim().is_empty()) {
                return Err(KeymapValidationError::EmptyKeyBinding {
                    control_id: ctrl.id.clone(),
                });
            }
            if let Some(0) = ctrl.macro_interval_ms {
                return Err(KeymapValidationError::InvalidMacroInterval {
                    control_id: ctrl.id.clone(),
                    interval_ms: 0,
                });
            }
        }

        Ok(())
    }

    /// Validates and serializes this profile to compact `keymap-v1` JSON.
    pub fn to_json(&self) -> Result<String, KeymapError> {
        self.validate()?;
        Ok(serde_json::to_string(self)?)
    }

    /// Validates and serializes this profile to pretty-printed `keymap-v1` JSON.
    pub fn to_json_pretty(&self) -> Result<String, KeymapError> {
        self.validate()?;
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Parses a `keymap-v1` JSON string and enforces [`KeymapProfile::validate`].
    pub fn from_json(raw: &str) -> Result<Self, KeymapError> {
        let profile: Self = serde_json::from_str(raw)?;
        profile.validate()?;
        Ok(profile)
    }
}

/// Searchable catalog of keymap presets.
#[derive(Debug, Clone)]
pub struct KeymapCatalog {
    profiles: Vec<KeymapProfile>,
}

impl KeymapCatalog {
    /// Builds the full built-in catalog containing 500+ curated game and application presets.
    pub fn builtin() -> Self {
        let mut profiles = Vec::with_capacity(BUILTIN_PRESET_SPECS.len());
        for spec in BUILTIN_PRESET_SPECS {
            profiles.push(build_profile(spec));
        }
        Self { profiles }
    }

    /// Creates a catalog from an explicit list of profiles.
    pub fn from_profiles(profiles: Vec<KeymapProfile>) -> Self {
        Self { profiles }
    }

    /// Total number of profiles in the catalog.
    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    /// Whether the catalog contains zero profiles.
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    /// Slice of all profiles in catalog order.
    pub fn profiles(&self) -> &[KeymapProfile] {
        &self.profiles
    }

    /// Looks up a single profile by exact ID.
    pub fn get(&self, id: &str) -> Option<&KeymapProfile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    /// Filters profiles by [`KeymapCategory`].
    pub fn by_category(&self, category: KeymapCategory) -> Vec<&KeymapProfile> {
        self.profiles
            .iter()
            .filter(|p| p.category == category)
            .collect()
    }

    /// Case-insensitive multi-token search across profile ID, name, category, and process hints.
    pub fn search(&self, query: &str) -> Vec<&KeymapProfile> {
        let trimmed = query.trim().to_ascii_lowercase();
        if trimmed.is_empty() {
            return self.profiles.iter().collect();
        }
        let tokens: Vec<&str> = trimmed.split_whitespace().collect();

        let mut scored: Vec<(u8, &KeymapProfile)> = self
            .profiles
            .iter()
            .filter_map(|profile| {
                let id_lc = profile.id.to_ascii_lowercase();
                let name_lc = profile.name.to_ascii_lowercase();
                let cat_lc = profile.category.as_str();
                let hints_lc: Vec<String> = profile
                    .process_hints
                    .iter()
                    .map(|h| h.to_ascii_lowercase())
                    .collect();

                let all_match = tokens.iter().all(|tok| {
                    id_lc.contains(tok)
                        || name_lc.contains(tok)
                        || cat_lc.contains(tok)
                        || hints_lc.iter().any(|h| h.contains(tok))
                });
                if !all_match {
                    return None;
                }

                let rank = if id_lc == trimmed || name_lc == trimmed {
                    0
                } else if name_lc.starts_with(&trimmed) || id_lc.starts_with(&trimmed) {
                    1
                } else if hints_lc.iter().any(|h| h == &trimmed) {
                    2
                } else {
                    3
                };
                Some((rank, profile))
            })
            .collect();

        scored.sort_by_key(|(rank, _)| *rank);
        scored.into_iter().map(|(_, p)| p).collect()
    }

    /// Matches a running process name or full executable path against `process_hints`
    /// using case-insensitive basename and substring matching, ranked by match specificity.
    pub fn match_process(&self, process_name: &str) -> Vec<&KeymapProfile> {
        let Some((base, stem)) = normalize_process_name(process_name) else {
            return Vec::new();
        };

        let mut scored: Vec<(u8, &KeymapProfile)> = self
            .profiles
            .iter()
            .filter_map(|profile| {
                let mut best_rank: Option<u8> = None;
                for hint in &profile.process_hints {
                    let Some((hint_base, hint_stem)) = normalize_process_name(hint) else {
                        continue;
                    };
                    let rank = if base == hint_base || stem == hint_stem {
                        Some(0)
                    } else if hint_stem.len() >= 3 && stem.contains(&hint_stem) {
                        Some(1)
                    } else if stem.len() >= 3 && hint_stem.contains(&stem) {
                        Some(2)
                    } else if hint_base.len() >= 3 && base.contains(&hint_base) {
                        Some(3)
                    } else {
                        None
                    };
                    if let Some(r) = rank {
                        best_rank = Some(best_rank.map_or(r, |cur| cur.min(r)));
                    }
                }
                best_rank.map(|r| (r, profile))
            })
            .collect();

        scored.sort_by_key(|(rank, _)| *rank);
        scored.into_iter().map(|(_, p)| p).collect()
    }
}

/// Extracts lowercase `(basename, stem_without_exec_ext)` from a process name or path.
fn normalize_process_name(raw: &str) -> Option<(String, String)> {
    let trimmed = raw.trim().trim_matches('"').trim_matches('\'');
    if trimmed.is_empty() {
        return None;
    }
    let trimmed = trimmed.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return None;
    }

    // Handle macOS `.app/Contents/MacOS/<bin>` paths as well as Windows/Unix paths.
    let basename = trimmed
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(trimmed)
        .to_ascii_lowercase();
    if basename.is_empty() {
        return None;
    }

    let stem = basename
        .strip_suffix(".exe")
        .or_else(|| basename.strip_suffix(".app"))
        .or_else(|| basename.strip_suffix(".bin"))
        .or_else(|| basename.strip_suffix(".sh"))
        .unwrap_or(&basename)
        .to_string();

    Some((basename, stem))
}

// ---------------------------------------------------------------------------
// Archetype builders & compact static catalog table (500+ curated presets)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Archetype {
    FpsTactical,
    FpsHero,
    FpsBattleRoyale,
    MobaClassic,
    RpgAction,
    RpgArpg,
    RpgMmo,
    ActionBrawler,
    ActionSurvival,
    ActionPlatformer,
    RacingArcade,
    RacingSim,
    SimFlight,
    SimStrategy,
    SimLife,
    ProdIde,
    ProdDesign,
    ProdVideo,
    ProdOffice,
    MediaPlayer,
    MediaDaw,
    CustomUniversal,
}

#[derive(Debug, Clone, Copy)]
struct PresetSpec {
    id: &'static str,
    name: &'static str,
    category: KeymapCategory,
    archetype: Archetype,
    hints: &'static [&'static str],
}

const fn spec(
    id: &'static str,
    name: &'static str,
    category: KeymapCategory,
    archetype: Archetype,
    hints: &'static [&'static str],
) -> PresetSpec {
    PresetSpec {
        id,
        name,
        category,
        archetype,
        hints,
    }
}

fn ctrl(
    id: &str,
    label: &str,
    kind: VirtualControlKind,
    norm_x: f32,
    norm_y: f32,
    radius_norm: f32,
    opacity: f32,
    keys: &[&str],
    macro_interval_ms: Option<u32>,
) -> KeymapControl {
    KeymapControl {
        id: id.to_string(),
        label: label.to_string(),
        kind,
        norm_x,
        norm_y,
        radius_norm,
        opacity,
        keys: keys.iter().map(|k| (*k).to_string()).collect(),
        macro_interval_ms,
    }
}

fn build_controls(archetype: Archetype) -> Vec<KeymapControl> {
    use VirtualControlKind::*;
    match archetype {
        Archetype::FpsTactical => vec![
            ctrl("move", "WASD", JoystickWasd, 0.14, 0.76, 0.12, 0.65, &["W", "A", "S", "D"], None),
            ctrl("look", "Aim", LookPad, 0.72, 0.50, 0.24, 0.20, &[], None),
            ctrl("fire", "Fire", MouseButton, 0.88, 0.72, 0.08, 0.78, &["MouseLeft"], None),
            ctrl("ads", "ADS", MouseButton, 0.88, 0.54, 0.065, 0.72, &["MouseRight"], None),
            ctrl("reload", "R", KeyButton, 0.78, 0.82, 0.055, 0.72, &["R"], None),
            ctrl("jump", "Jump", KeyButton, 0.90, 0.86, 0.06, 0.72, &["Space"], None),
            ctrl("crouch", "Crouch", KeyButton, 0.78, 0.66, 0.055, 0.68, &["ControlLeft"], None),
            ctrl("walk", "Walk", KeyButton, 0.08, 0.54, 0.05, 0.62, &["ShiftLeft"], None),
            ctrl("use", "Use", KeyButton, 0.68, 0.74, 0.05, 0.68, &["E"], None),
            ctrl("grenade", "Nade", KeyButton, 0.68, 0.58, 0.05, 0.68, &["G"], None),
            ctrl("primary", "1", KeyButton, 0.42, 0.90, 0.045, 0.60, &["Digit1"], None),
            ctrl("secondary", "2", KeyButton, 0.50, 0.90, 0.045, 0.60, &["Digit2"], None),
            ctrl("tab", "Score", KeyButton, 0.50, 0.08, 0.045, 0.55, &["Tab"], None),
        ],
        Archetype::FpsHero => vec![
            ctrl("move", "WASD", JoystickWasd, 0.14, 0.76, 0.12, 0.65, &["W", "A", "S", "D"], None),
            ctrl("look", "Aim", LookPad, 0.72, 0.48, 0.24, 0.20, &[], None),
            ctrl("fire", "Fire", MouseButton, 0.88, 0.72, 0.08, 0.80, &["MouseLeft"], None),
            ctrl("alt_fire", "Alt", MouseButton, 0.88, 0.54, 0.065, 0.74, &["MouseRight"], None),
            ctrl("ability_1", "Shift", KeyButton, 0.76, 0.64, 0.06, 0.74, &["ShiftLeft"], None),
            ctrl("ability_2", "E", KeyButton, 0.76, 0.78, 0.06, 0.74, &["E"], None),
            ctrl("ultimate", "Ult Q", KeyButton, 0.66, 0.82, 0.06, 0.78, &["Q"], None),
            ctrl("reload", "R", KeyButton, 0.80, 0.40, 0.05, 0.68, &["R"], None),
            ctrl("jump", "Jump", KeyButton, 0.90, 0.86, 0.06, 0.72, &["Space"], None),
            ctrl("melee", "Melee", KeyButton, 0.66, 0.66, 0.05, 0.68, &["V"], None),
            ctrl("crouch", "Slide", KeyButton, 0.08, 0.56, 0.05, 0.65, &["ControlLeft"], None),
        ],
        Archetype::FpsBattleRoyale => vec![
            ctrl("move", "WASD", JoystickWasd, 0.14, 0.76, 0.12, 0.65, &["W", "A", "S", "D"], None),
            ctrl("look", "Look", LookPad, 0.70, 0.48, 0.24, 0.20, &[], None),
            ctrl("fire", "Fire", MouseButton, 0.88, 0.70, 0.08, 0.80, &["MouseLeft"], None),
            ctrl("ads", "Scope", MouseButton, 0.88, 0.52, 0.065, 0.74, &["MouseRight"], None),
            ctrl("loot", "Loot", KeyButton, 0.68, 0.72, 0.055, 0.72, &["F"], None),
            ctrl("reload", "R", KeyButton, 0.78, 0.82, 0.055, 0.72, &["R"], None),
            ctrl("jump", "Jump", KeyButton, 0.90, 0.86, 0.06, 0.72, &["Space"], None),
            ctrl("crouch", "Crouch", KeyButton, 0.78, 0.64, 0.05, 0.68, &["C"], None),
            ctrl("prone", "Prone", KeyButton, 0.68, 0.56, 0.05, 0.65, &["Z"], None),
            ctrl("heal", "Heal", KeyButton, 0.36, 0.88, 0.05, 0.70, &["Digit4"], None),
            ctrl("bag", "Bag", KeyButton, 0.46, 0.88, 0.05, 0.62, &["Tab"], None),
            ctrl("map", "Map", KeyButton, 0.88, 0.14, 0.05, 0.60, &["M"], None),
        ],
        Archetype::MobaClassic => vec![
            ctrl("move", "Move", MouseButton, 0.14, 0.76, 0.09, 0.68, &["MouseRight"], None),
            ctrl("select", "Target", MouseButton, 0.88, 0.76, 0.08, 0.78, &["MouseLeft"], None),
            ctrl("skill_q", "Q", KeyButton, 0.74, 0.84, 0.06, 0.80, &["Q"], None),
            ctrl("skill_w", "W", KeyButton, 0.76, 0.68, 0.06, 0.80, &["W"], None),
            ctrl("skill_e", "E", KeyButton, 0.84, 0.56, 0.06, 0.80, &["E"], None),
            ctrl("skill_r", "R", KeyButton, 0.92, 0.46, 0.065, 0.84, &["R"], None),
            ctrl("spell_d", "D", KeyButton, 0.64, 0.86, 0.05, 0.72, &["D"], None),
            ctrl("spell_f", "F", KeyButton, 0.64, 0.72, 0.05, 0.72, &["F"], None),
            ctrl("attack_move", "A-Move", KeyButton, 0.88, 0.90, 0.05, 0.70, &["A"], None),
            ctrl("recall", "Recall", KeyButton, 0.52, 0.88, 0.05, 0.66, &["B"], None),
            ctrl("center", "Cam", KeyButton, 0.26, 0.86, 0.05, 0.66, &["Space"], None),
            ctrl("item_1", "1", KeyButton, 0.42, 0.88, 0.045, 0.64, &["Digit1"], None),
            ctrl("tab", "Tab", KeyButton, 0.50, 0.08, 0.045, 0.55, &["Tab"], None),
        ],
        Archetype::RpgAction => vec![
            ctrl("move", "WASD", JoystickWasd, 0.14, 0.76, 0.12, 0.65, &["W", "A", "S", "D"], None),
            ctrl("look", "Camera", LookPad, 0.70, 0.46, 0.24, 0.18, &[], None),
            ctrl("light_atk", "Attack", MouseButton, 0.88, 0.74, 0.08, 0.80, &["MouseLeft"], None),
            ctrl("heavy_blk", "Block", MouseButton, 0.88, 0.56, 0.065, 0.74, &["MouseRight"], None),
            ctrl("dodge", "Dodge", KeyButton, 0.78, 0.84, 0.06, 0.76, &["Space"], None),
            ctrl("skill_e", "Skill", KeyButton, 0.76, 0.68, 0.058, 0.76, &["E"], None),
            ctrl("burst_q", "Burst", KeyButton, 0.68, 0.56, 0.058, 0.76, &["Q"], None),
            ctrl("heal", "Flask", KeyButton, 0.66, 0.84, 0.055, 0.72, &["R"], None),
            ctrl("interact", "Use", KeyButton, 0.66, 0.70, 0.05, 0.70, &["F"], None),
            ctrl("sprint", "Sprint", KeyButton, 0.08, 0.54, 0.05, 0.64, &["ShiftLeft"], None),
            ctrl("inv", "Bag", KeyButton, 0.44, 0.90, 0.045, 0.58, &["I"], None),
            ctrl("map", "Map", KeyButton, 0.54, 0.90, 0.045, 0.58, &["M"], None),
        ],
        Archetype::RpgArpg => vec![
            ctrl("primary", "Move/Atk", MouseButton, 0.88, 0.76, 0.08, 0.80, &["MouseLeft"], None),
            ctrl("secondary", "Core", MouseButton, 0.88, 0.58, 0.065, 0.76, &["MouseRight"], None),
            ctrl("skill_q", "Q", KeyButton, 0.76, 0.84, 0.058, 0.78, &["Q"], None),
            ctrl("skill_w", "W", KeyButton, 0.76, 0.68, 0.058, 0.78, &["W"], None),
            ctrl("skill_e", "E", KeyButton, 0.76, 0.52, 0.058, 0.78, &["E"], None),
            ctrl("skill_r", "R", KeyButton, 0.86, 0.42, 0.06, 0.80, &["R"], None),
            ctrl("dash", "Dash", KeyButton, 0.14, 0.78, 0.075, 0.76, &["Space"], None),
            ctrl("flask_1", "HP", KeyButton, 0.10, 0.58, 0.052, 0.74, &["Digit1"], None),
            ctrl("flask_2", "MP", KeyButton, 0.20, 0.58, 0.052, 0.74, &["Digit2"], None),
            ctrl("inv", "Inv", KeyButton, 0.44, 0.90, 0.045, 0.60, &["I"], None),
            ctrl("map", "Map", KeyButton, 0.54, 0.90, 0.045, 0.60, &["Tab"], None),
        ],
        Archetype::RpgMmo => vec![
            ctrl("move", "WASD", JoystickWasd, 0.14, 0.76, 0.12, 0.65, &["W", "A", "S", "D"], None),
            ctrl("look", "Camera", LookPad, 0.70, 0.42, 0.22, 0.18, &[], None),
            ctrl("bar_1", "1", KeyButton, 0.64, 0.86, 0.052, 0.74, &["Digit1"], None),
            ctrl("bar_2", "2", KeyButton, 0.74, 0.86, 0.052, 0.74, &["Digit2"], None),
            ctrl("bar_3", "3", KeyButton, 0.84, 0.86, 0.052, 0.74, &["Digit3"], None),
            ctrl("bar_4", "4", KeyButton, 0.64, 0.72, 0.052, 0.74, &["Digit4"], None),
            ctrl("bar_5", "5", KeyButton, 0.74, 0.72, 0.052, 0.74, &["Digit5"], None),
            ctrl("bar_6", "6", KeyButton, 0.84, 0.72, 0.052, 0.74, &["Digit6"], None),
            ctrl("burst", "Auto", MacroBurst, 0.92, 0.78, 0.055, 0.78, &["Digit1", "Digit2"], Some(180)),
            ctrl("target", "Tab", KeyButton, 0.90, 0.58, 0.052, 0.70, &["Tab"], None),
            ctrl("jump", "Jump", KeyButton, 0.28, 0.86, 0.055, 0.70, &["Space"], None),
            ctrl("interact", "F", KeyButton, 0.54, 0.86, 0.048, 0.68, &["F"], None),
        ],
        Archetype::ActionBrawler => vec![
            ctrl("move", "WASD", JoystickWasd, 0.14, 0.76, 0.12, 0.65, &["W", "A", "S", "D"], None),
            ctrl("look", "Camera", LookPad, 0.70, 0.46, 0.24, 0.18, &[], None),
            ctrl("attack", "Strike", MouseButton, 0.88, 0.74, 0.08, 0.80, &["MouseLeft"], None),
            ctrl("parry", "Parry", MouseButton, 0.88, 0.56, 0.065, 0.76, &["MouseRight"], None),
            ctrl("jump", "Jump", KeyButton, 0.78, 0.84, 0.06, 0.74, &["Space"], None),
            ctrl("interact", "Act", KeyButton, 0.76, 0.68, 0.055, 0.72, &["E"], None),
            ctrl("finisher", "Grab", KeyButton, 0.68, 0.76, 0.055, 0.72, &["F"], None),
            ctrl("gadget", "Gadget", KeyButton, 0.68, 0.60, 0.052, 0.70, &["Q"], None),
            ctrl("sprint", "Run", KeyButton, 0.08, 0.54, 0.05, 0.64, &["ShiftLeft"], None),
            ctrl("stealth", "Stealth", KeyButton, 0.24, 0.88, 0.05, 0.62, &["ControlLeft"], None),
        ],
        Archetype::ActionSurvival => vec![
            ctrl("move", "WASD", JoystickWasd, 0.14, 0.76, 0.12, 0.65, &["W", "A", "S", "D"], None),
            ctrl("look", "Look", LookPad, 0.70, 0.46, 0.24, 0.18, &[], None),
            ctrl("use_lmb", "Mine/Hit", MouseButton, 0.88, 0.74, 0.08, 0.78, &["MouseLeft"], None),
            ctrl("place_rmb", "Place", MouseButton, 0.88, 0.56, 0.065, 0.74, &["MouseRight"], None),
            ctrl("jump", "Jump", KeyButton, 0.78, 0.84, 0.06, 0.74, &["Space"], None),
            ctrl("interact", "Use", KeyButton, 0.76, 0.68, 0.055, 0.72, &["E"], None),
            ctrl("inv", "Craft", KeyButton, 0.66, 0.84, 0.052, 0.70, &["Tab"], None),
            ctrl("drop", "Drop", KeyButton, 0.66, 0.68, 0.05, 0.66, &["Q"], None),
            ctrl("hotbar", "Scroll", ScrollWheel, 0.50, 0.88, 0.05, 0.62, &["WheelUp", "WheelDown"], None),
            ctrl("crouch", "Sneak", KeyButton, 0.08, 0.54, 0.05, 0.64, &["ShiftLeft"], None),
        ],
        Archetype::ActionPlatformer => vec![
            ctrl("move", "Move", JoystickWasd, 0.15, 0.76, 0.12, 0.68, &["W", "A", "S", "D"], None),
            ctrl("jump", "Jump", KeyButton, 0.88, 0.78, 0.075, 0.82, &["Space"], None),
            ctrl("attack", "Attack", KeyButton, 0.76, 0.72, 0.07, 0.80, &["X"], None),
            ctrl("dash", "Dash", KeyButton, 0.88, 0.58, 0.065, 0.78, &["ShiftLeft"], None),
            ctrl("special", "Cast", KeyButton, 0.76, 0.56, 0.06, 0.74, &["C"], None),
            ctrl("interact", "Use", KeyButton, 0.64, 0.76, 0.055, 0.70, &["E"], None),
            ctrl("menu", "Esc", KeyButton, 0.08, 0.10, 0.045, 0.58, &["Escape"], None),
        ],
        Archetype::RacingArcade => vec![
            ctrl("steer", "Steer", JoystickWasd, 0.16, 0.76, 0.13, 0.72, &["W", "A", "S", "D"], None),
            ctrl("throttle", "Gas", KeyButton, 0.90, 0.76, 0.08, 0.82, &["W"], None),
            ctrl("brake", "Brake", KeyButton, 0.78, 0.82, 0.07, 0.78, &["S"], None),
            ctrl("handbrake", "Drift", KeyButton, 0.78, 0.64, 0.06, 0.76, &["Space"], None),
            ctrl("nitro", "Boost", KeyButton, 0.90, 0.56, 0.065, 0.80, &["ShiftLeft"], None),
            ctrl("cam", "Cam", KeyButton, 0.88, 0.14, 0.05, 0.60, &["C"], None),
            ctrl("reset", "Reset", KeyButton, 0.10, 0.14, 0.05, 0.60, &["R"], None),
        ],
        Archetype::RacingSim => vec![
            ctrl("steer", "Steer", JoystickWasd, 0.16, 0.76, 0.13, 0.72, &["W", "A", "S", "D"], None),
            ctrl("throttle", "Gas", KeyButton, 0.90, 0.76, 0.08, 0.82, &["W"], None),
            ctrl("brake", "Brake", KeyButton, 0.78, 0.82, 0.07, 0.78, &["S"], None),
            ctrl("shift_up", "Shift+", KeyButton, 0.90, 0.54, 0.06, 0.76, &["E"], None),
            ctrl("shift_dn", "Shift-", KeyButton, 0.78, 0.58, 0.06, 0.76, &["Q"], None),
            ctrl("handbrake", "Hand", KeyButton, 0.68, 0.78, 0.055, 0.72, &["Space"], None),
            ctrl("look_back", "Rear", KeyButton, 0.50, 0.10, 0.048, 0.58, &["B"], None),
            ctrl("cam", "Cam", KeyButton, 0.88, 0.14, 0.048, 0.58, &["C"], None),
        ],
        Archetype::SimFlight => vec![
            ctrl("stick", "Pitch/Roll", JoystickWasd, 0.16, 0.76, 0.13, 0.68, &["W", "A", "S", "D"], None),
            ctrl("look", "Cockpit", LookPad, 0.70, 0.44, 0.22, 0.18, &[], None),
            ctrl("thr_up", "Thr+", KeyButton, 0.90, 0.62, 0.065, 0.76, &["ShiftLeft"], None),
            ctrl("thr_dn", "Thr-", KeyButton, 0.90, 0.80, 0.065, 0.76, &["ControlLeft"], None),
            ctrl("yaw_l", "Yaw L", KeyButton, 0.68, 0.84, 0.055, 0.70, &["Q"], None),
            ctrl("yaw_r", "Yaw R", KeyButton, 0.78, 0.84, 0.055, 0.70, &["E"], None),
            ctrl("gear", "Gear", KeyButton, 0.76, 0.66, 0.05, 0.68, &["G"], None),
            ctrl("flaps", "Flaps", KeyButton, 0.66, 0.66, 0.05, 0.68, &["F"], None),
            ctrl("brake", "Fire/Brk", KeyButton, 0.88, 0.44, 0.06, 0.74, &["Space"], None),
            ctrl("map", "Map", KeyButton, 0.88, 0.14, 0.048, 0.58, &["M"], None),
        ],
        Archetype::SimStrategy => vec![
            ctrl("pan", "Pan", JoystickWasd, 0.14, 0.78, 0.11, 0.62, &["W", "A", "S", "D"], None),
            ctrl("select", "Select", MouseButton, 0.88, 0.76, 0.075, 0.76, &["MouseLeft"], None),
            ctrl("order", "Order", MouseButton, 0.88, 0.58, 0.07, 0.76, &["MouseRight"], None),
            ctrl("zoom", "Zoom", ScrollWheel, 0.94, 0.34, 0.055, 0.64, &["WheelUp", "WheelDown"], None),
            ctrl("pause", "Pause", KeyButton, 0.50, 0.90, 0.055, 0.72, &["Space"], None),
            ctrl("group_1", "Grp 1", KeyButton, 0.36, 0.90, 0.045, 0.64, &["Digit1"], None),
            ctrl("group_2", "Grp 2", KeyButton, 0.43, 0.90, 0.045, 0.64, &["Digit2"], None),
            ctrl("build", "Build", KeyButton, 0.76, 0.82, 0.055, 0.70, &["B"], None),
            ctrl("esc", "Esc", KeyButton, 0.08, 0.10, 0.045, 0.58, &["Escape"], None),
        ],
        Archetype::SimLife => vec![
            ctrl("pan", "Pan", JoystickWasd, 0.14, 0.78, 0.11, 0.62, &["W", "A", "S", "D"], None),
            ctrl("click", "Click", MouseButton, 0.88, 0.76, 0.075, 0.76, &["MouseLeft"], None),
            ctrl("rclick", "Menu", MouseButton, 0.88, 0.58, 0.065, 0.72, &["MouseRight"], None),
            ctrl("zoom", "Zoom", ScrollWheel, 0.94, 0.36, 0.055, 0.64, &["WheelUp", "WheelDown"], None),
            ctrl("speed_pause", "Pause", KeyButton, 0.42, 0.90, 0.05, 0.70, &["Space"], None),
            ctrl("speed_1", "1x", KeyButton, 0.50, 0.90, 0.045, 0.66, &["Digit1"], None),
            ctrl("speed_3", "3x", KeyButton, 0.58, 0.90, 0.045, 0.66, &["Digit3"], None),
            ctrl("rotate", "Rot", KeyButton, 0.76, 0.80, 0.05, 0.68, &["R"], None),
        ],
        Archetype::ProdIde => vec![
            ctrl("save", "Save", ComboButton, 0.10, 0.88, 0.055, 0.74, &["ControlLeft", "S"], None),
            ctrl("undo", "Undo", ComboButton, 0.20, 0.88, 0.055, 0.74, &["ControlLeft", "Z"], None),
            ctrl("redo", "Redo", ComboButton, 0.30, 0.88, 0.055, 0.74, &["ControlLeft", "ShiftLeft", "Z"], None),
            ctrl("palette", "Cmd", ComboButton, 0.42, 0.88, 0.06, 0.78, &["ControlLeft", "ShiftLeft", "P"], None),
            ctrl("find", "Find", ComboButton, 0.54, 0.88, 0.055, 0.74, &["ControlLeft", "F"], None),
            ctrl("copy", "Copy", ComboButton, 0.66, 0.88, 0.055, 0.74, &["ControlLeft", "C"], None),
            ctrl("paste", "Paste", ComboButton, 0.76, 0.88, 0.055, 0.74, &["ControlLeft", "V"], None),
            ctrl("comment", "//", ComboButton, 0.86, 0.88, 0.055, 0.74, &["ControlLeft", "Slash"], None),
            ctrl("scroll", "Scroll", ScrollWheel, 0.94, 0.50, 0.06, 0.64, &["WheelUp", "WheelDown"], None),
            ctrl("esc", "Esc", KeyButton, 0.08, 0.12, 0.045, 0.60, &["Escape"], None),
        ],
        Archetype::ProdDesign => vec![
            ctrl("undo", "Undo", ComboButton, 0.10, 0.88, 0.058, 0.78, &["ControlLeft", "Z"], None),
            ctrl("redo", "Redo", ComboButton, 0.20, 0.88, 0.058, 0.76, &["ControlLeft", "ShiftLeft", "Z"], None),
            ctrl("save", "Save", ComboButton, 0.30, 0.88, 0.055, 0.74, &["ControlLeft", "S"], None),
            ctrl("pan_space", "Pan", KeyButton, 0.10, 0.72, 0.06, 0.76, &["Space"], None),
            ctrl("shift_mod", "Shift", KeyButton, 0.10, 0.56, 0.055, 0.72, &["ShiftLeft"], None),
            ctrl("alt_mod", "Alt", KeyButton, 0.10, 0.42, 0.055, 0.72, &["AltLeft"], None),
            ctrl("brush", "Brush", KeyButton, 0.82, 0.86, 0.055, 0.74, &["B"], None),
            ctrl("eraser", "Erase", KeyButton, 0.90, 0.86, 0.055, 0.74, &["E"], None),
            ctrl("zoom", "Zoom", ScrollWheel, 0.94, 0.48, 0.06, 0.66, &["WheelUp", "WheelDown"], None),
        ],
        Archetype::ProdVideo => vec![
            ctrl("play", "Play", KeyButton, 0.50, 0.88, 0.065, 0.80, &["Space"], None),
            ctrl("rev", "J", KeyButton, 0.38, 0.88, 0.055, 0.76, &["J"], None),
            ctrl("stop", "K", KeyButton, 0.62, 0.88, 0.055, 0.76, &["K"], None),
            ctrl("fwd", "L", KeyButton, 0.72, 0.88, 0.055, 0.76, &["L"], None),
            ctrl("cut", "Cut", ComboButton, 0.84, 0.88, 0.058, 0.78, &["ControlLeft", "B"], None),
            ctrl("undo", "Undo", ComboButton, 0.10, 0.88, 0.055, 0.76, &["ControlLeft", "Z"], None),
            ctrl("save", "Save", ComboButton, 0.22, 0.88, 0.055, 0.74, &["ControlLeft", "S"], None),
            ctrl("mark_in", "In", KeyButton, 0.84, 0.72, 0.05, 0.70, &["I"], None),
            ctrl("mark_out", "Out", KeyButton, 0.92, 0.72, 0.05, 0.70, &["O"], None),
            ctrl("scrub", "Timeline", ScrollWheel, 0.94, 0.44, 0.06, 0.64, &["WheelUp", "WheelDown"], None),
        ],
        Archetype::ProdOffice => vec![
            ctrl("copy", "Copy", ComboButton, 0.12, 0.88, 0.055, 0.74, &["ControlLeft", "C"], None),
            ctrl("paste", "Paste", ComboButton, 0.24, 0.88, 0.055, 0.74, &["ControlLeft", "V"], None),
            ctrl("undo", "Undo", ComboButton, 0.36, 0.88, 0.055, 0.74, &["ControlLeft", "Z"], None),
            ctrl("save", "Save", ComboButton, 0.48, 0.88, 0.055, 0.74, &["ControlLeft", "S"], None),
            ctrl("find", "Find", ComboButton, 0.60, 0.88, 0.055, 0.74, &["ControlLeft", "F"], None),
            ctrl("new_tab", "New", ComboButton, 0.72, 0.88, 0.055, 0.72, &["ControlLeft", "T"], None),
            ctrl("close_tab", "Close", ComboButton, 0.84, 0.88, 0.055, 0.72, &["ControlLeft", "W"], None),
            ctrl("scroll", "Scroll", ScrollWheel, 0.94, 0.50, 0.06, 0.64, &["WheelUp", "WheelDown"], None),
        ],
        Archetype::MediaPlayer => vec![
            ctrl("play_pause", "Play", KeyButton, 0.50, 0.86, 0.075, 0.82, &["Space"], None),
            ctrl("seek_back", "-10s", KeyButton, 0.34, 0.86, 0.06, 0.76, &["ArrowLeft"], None),
            ctrl("seek_fwd", "+10s", KeyButton, 0.66, 0.86, 0.06, 0.76, &["ArrowRight"], None),
            ctrl("vol_up", "Vol+", KeyButton, 0.90, 0.58, 0.055, 0.74, &["ArrowUp"], None),
            ctrl("vol_dn", "Vol-", KeyButton, 0.90, 0.74, 0.055, 0.74, &["ArrowDown"], None),
            ctrl("mute", "Mute", KeyButton, 0.90, 0.88, 0.052, 0.72, &["M"], None),
            ctrl("fullscreen", "Full", KeyButton, 0.80, 0.86, 0.055, 0.72, &["F"], None),
            ctrl("subtitles", "Sub", KeyButton, 0.20, 0.86, 0.052, 0.68, &["V"], None),
            ctrl("esc", "Esc", KeyButton, 0.08, 0.12, 0.048, 0.60, &["Escape"], None),
        ],
        Archetype::MediaDaw => vec![
            ctrl("play", "Play", KeyButton, 0.50, 0.88, 0.07, 0.82, &["Space"], None),
            ctrl("record", "Rec", KeyButton, 0.64, 0.88, 0.06, 0.80, &["R"], None),
            ctrl("loop", "Loop", KeyButton, 0.36, 0.88, 0.055, 0.74, &["L"], None),
            ctrl("undo", "Undo", ComboButton, 0.12, 0.88, 0.055, 0.74, &["ControlLeft", "Z"], None),
            ctrl("save", "Save", ComboButton, 0.24, 0.88, 0.055, 0.74, &["ControlLeft", "S"], None),
            ctrl("split", "Split", ComboButton, 0.78, 0.88, 0.055, 0.74, &["ControlLeft", "E"], None),
            ctrl("scroll", "Zoom", ScrollWheel, 0.94, 0.48, 0.06, 0.64, &["WheelUp", "WheelDown"], None),
        ],
        Archetype::CustomUniversal => vec![
            ctrl("move", "WASD", JoystickWasd, 0.14, 0.76, 0.12, 0.65, &["W", "A", "S", "D"], None),
            ctrl("look", "Trackpad", LookPad, 0.70, 0.48, 0.24, 0.18, &[], None),
            ctrl("lmb", "LMB", MouseButton, 0.88, 0.74, 0.075, 0.76, &["MouseLeft"], None),
            ctrl("rmb", "RMB", MouseButton, 0.88, 0.56, 0.065, 0.72, &["MouseRight"], None),
            ctrl("space", "Space", KeyButton, 0.76, 0.84, 0.06, 0.72, &["Space"], None),
            ctrl("enter", "Enter", KeyButton, 0.64, 0.84, 0.055, 0.70, &["Enter"], None),
            ctrl("esc", "Esc", KeyButton, 0.08, 0.12, 0.048, 0.60, &["Escape"], None),
        ],
    }
}

fn build_profile(s: &PresetSpec) -> KeymapProfile {
    KeymapProfile {
        schema_version: KEYMAP_SCHEMA_VERSION,
        id: s.id.to_string(),
        name: s.name.to_string(),
        category: s.category,
        process_hints: s.hints.iter().map(|h| (*h).to_string()).collect(),
        controls: build_controls(s.archetype),
    }
}

const BUILTIN_PRESET_SPECS: &[PresetSpec] = &[
    // ==================== FPS (100 presets) ====================
    spec("fps_cs2", "Counter-Strike 2", KeymapCategory::Fps, Archetype::FpsTactical, &["cs2.exe", "cs2"]),
    spec("fps_csgo", "Counter-Strike: Global Offensive", KeymapCategory::Fps, Archetype::FpsTactical, &["csgo.exe", "csgo_linux64"]),
    spec("fps_valorant", "VALORANT", KeymapCategory::Fps, Archetype::FpsTactical, &["VALORANT-Win64-Shipping.exe", "valorant.exe"]),
    spec("fps_r6siege", "Rainbow Six Siege", KeymapCategory::Fps, Archetype::FpsTactical, &["RainbowSix.exe", "RainbowSix_Vulkan.exe"]),
    spec("fps_tarkov", "Escape from Tarkov", KeymapCategory::Fps, Archetype::FpsTactical, &["EscapeFromTarkov.exe"]),
    spec("fps_ready_or_not", "Ready or Not", KeymapCategory::Fps, Archetype::FpsTactical, &["ReadyOrNot-Win64-Shipping.exe"]),
    spec("fps_insurgency_sandstorm", "Insurgency: Sandstorm", KeymapCategory::Fps, Archetype::FpsTactical, &["InsurgencyClient-Win64-Shipping.exe"]),
    spec("fps_squad", "Squad", KeymapCategory::Fps, Archetype::FpsTactical, &["SquadGame.exe"]),
    spec("fps_arma3", "Arma 3", KeymapCategory::Fps, Archetype::FpsTactical, &["arma3_x64.exe", "arma3.exe"]),
    spec("fps_arma_reforger", "Arma Reforger", KeymapCategory::Fps, Archetype::FpsTactical, &["ArmaReforgerSteam.exe"]),
    spec("fps_hell_let_loose", "Hell Let Loose", KeymapCategory::Fps, Archetype::FpsTactical, &["HLL-Win64-Shipping.exe"]),
    spec("fps_post_scriptum", "Squad 44", KeymapCategory::Fps, Archetype::FpsTactical, &["PostScriptum.exe"]),
    spec("fps_ground_branch", "Ground Branch", KeymapCategory::Fps, Archetype::FpsTactical, &["GroundBranch-Win64-Shipping.exe"]),
    spec("fps_zero_hour", "Zero Hour", KeymapCategory::Fps, Archetype::FpsTactical, &["ZeroHour.exe"]),
    spec("fps_gray_zone", "Gray Zone Warfare", KeymapCategory::Fps, Archetype::FpsTactical, &["GZWClientSteam-Win64-Shipping.exe"]),
    spec("fps_arena_breakout", "Arena Breakout: Infinite", KeymapCategory::Fps, Archetype::FpsTactical, &["UAGame.exe", "ABInfinite.exe"]),
    spec("fps_delta_force", "Delta Force", KeymapCategory::Fps, Archetype::FpsTactical, &["DeltaForceClient-Win64-Shipping.exe"]),
    spec("fps_crossfire", "CrossFire", KeymapCategory::Fps, Archetype::FpsTactical, &["crossfire.exe"]),
    spec("fps_sudden_attack", "Sudden Attack", KeymapCategory::Fps, Archetype::FpsTactical, &["suddenattack.exe"]),
    spec("fps_point_blank", "Point Blank", KeymapCategory::Fps, Archetype::FpsTactical, &["PointBlank.exe"]),
    spec("fps_overwatch2", "Overwatch 2", KeymapCategory::Fps, Archetype::FpsHero, &["Overwatch.exe"]),
    spec("fps_apex_legends", "Apex Legends", KeymapCategory::Fps, Archetype::FpsHero, &["r5apex.exe", "r5apex_dx12.exe"]),
    spec("fps_tf2", "Team Fortress 2", KeymapCategory::Fps, Archetype::FpsHero, &["tf_win64.exe", "hl2.exe"]),
    spec("fps_the_finals", "THE FINALS", KeymapCategory::Fps, Archetype::FpsHero, &["Discovery.exe"]),
    spec("fps_marvel_rivals", "Marvel Rivals", KeymapCategory::Fps, Archetype::FpsHero, &["Marvel-Win64-Shipping.exe"]),
    spec("fps_paladins", "Paladins", KeymapCategory::Fps, Archetype::FpsHero, &["Paladins.exe"]),
    spec("fps_destiny2", "Destiny 2", KeymapCategory::Fps, Archetype::FpsHero, &["destiny2.exe"]),
    spec("fps_halo_infinite", "Halo Infinite", KeymapCategory::Fps, Archetype::FpsHero, &["HaloInfinite.exe"]),
    spec("fps_halo_mcc", "Halo: The Master Chief Collection", KeymapCategory::Fps, Archetype::FpsHero, &["MCC-Win64-Shipping.exe"]),
    spec("fps_doom_eternal", "DOOM Eternal", KeymapCategory::Fps, Archetype::FpsHero, &["DOOMEternalx64vk.exe"]),
    spec("fps_doom_2016", "DOOM (2016)", KeymapCategory::Fps, Archetype::FpsHero, &["DOOMx64.exe", "DOOMx64vk.exe"]),
    spec("fps_doom_dark_ages", "DOOM: The Dark Ages", KeymapCategory::Fps, Archetype::FpsHero, &["DOOMTheDarkAges.exe"]),
    spec("fps_titanfall2", "Titanfall 2", KeymapCategory::Fps, Archetype::FpsHero, &["Titanfall2.exe"]),
    spec("fps_ultrakill", "ULTRAKILL", KeymapCategory::Fps, Archetype::FpsHero, &["ULTRAKILL.exe"]),
    spec("fps_quake_champions", "Quake Champions", KeymapCategory::Fps, Archetype::FpsHero, &["QuakeChampions.exe"]),
    spec("fps_quake_live", "Quake Live", KeymapCategory::Fps, Archetype::FpsHero, &["quakelive_steam.exe"]),
    spec("fps_splitgate", "Splitgate", KeymapCategory::Fps, Archetype::FpsHero, &["PortalWars-Win64-Shipping.exe"]),
    spec("fps_splitgate2", "Splitgate 2", KeymapCategory::Fps, Archetype::FpsHero, &["Splitgate2-Win64-Shipping.exe"]),
    spec("fps_xdefiant", "XDefiant", KeymapCategory::Fps, Archetype::FpsHero, &["XDefiant.exe"]),
    spec("fps_fragpunk", "FragPunk", KeymapCategory::Fps, Archetype::FpsHero, &["FragPunk.exe"]),
    spec("fps_strinova", "Strinova", KeymapCategory::Fps, Archetype::FpsHero, &["Strinova-Win64-Shipping.exe"]),
    spec("fps_borderlands2", "Borderlands 2", KeymapCategory::Fps, Archetype::FpsHero, &["Borderlands2.exe"]),
    spec("fps_borderlands3", "Borderlands 3", KeymapCategory::Fps, Archetype::FpsHero, &["Borderlands3.exe"]),
    spec("fps_wonderlands", "Tiny Tina's Wonderlands", KeymapCategory::Fps, Archetype::FpsHero, &["Wonderlands.exe"]),
    spec("fps_bioshock_infinite", "BioShock Infinite", KeymapCategory::Fps, Archetype::FpsHero, &["BioShockInfinite.exe"]),
    spec("fps_dishonored2", "Dishonored 2", KeymapCategory::Fps, Archetype::FpsHero, &["Dishonored2.exe"]),
    spec("fps_deathloop", "DEATHLOOP", KeymapCategory::Fps, Archetype::FpsHero, &["Deathloop.exe"]),
    spec("fps_prey", "Prey (2017)", KeymapCategory::Fps, Archetype::FpsHero, &["Prey.exe"]),
    spec("fps_ghostrunner", "Ghostrunner", KeymapCategory::Fps, Archetype::FpsHero, &["Ghostrunner-Win64-Shipping.exe"]),
    spec("fps_ghostrunner2", "Ghostrunner 2", KeymapCategory::Fps, Archetype::FpsHero, &["Ghostrunner2-Win64-Shipping.exe"]),
    spec("fps_neon_white", "Neon White", KeymapCategory::Fps, Archetype::FpsHero, &["Neon White.exe"]),
    spec("fps_roboquest", "Roboquest", KeymapCategory::Fps, Archetype::FpsHero, &["RoboQuest-Win64-Shipping.exe"]),
    spec("fps_gunfire_reborn", "Gunfire Reborn", KeymapCategory::Fps, Archetype::FpsHero, &["Gunfire Reborn.exe"]),
    spec("fps_risk_of_rain2", "Risk of Rain 2", KeymapCategory::Fps, Archetype::FpsHero, &["Risk of Rain 2.exe"]),
    spec("fps_deep_rock_galactic", "Deep Rock Galactic", KeymapCategory::Fps, Archetype::FpsHero, &["FSD-Win64-Shipping.exe"]),
    spec("fps_left4dead2", "Left 4 Dead 2", KeymapCategory::Fps, Archetype::FpsHero, &["left4dead2.exe"]),
    spec("fps_back4blood", "Back 4 Blood", KeymapCategory::Fps, Archetype::FpsHero, &["Back4Blood.exe"]),
    spec("fps_killing_floor2", "Killing Floor 2", KeymapCategory::Fps, Archetype::FpsHero, &["KFGame.exe"]),
    spec("fps_payday2", "PAYDAY 2", KeymapCategory::Fps, Archetype::FpsTactical, &["payday2_win32_release.exe"]),
    spec("fps_payday3", "PAYDAY 3", KeymapCategory::Fps, Archetype::FpsTactical, &["PAYDAY3Client-Win64-Shipping.exe"]),
    spec("fps_pubg", "PUBG: BATTLEGROUNDS", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["TslGame.exe"]),
    spec("fps_fortnite", "Fortnite", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["FortniteClient-Win64-Shipping.exe"]),
    spec("fps_cod_warzone", "Call of Duty: Warzone", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["cod.exe", "ModernWarfare.exe"]),
    spec("fps_cod_bo6", "Call of Duty: Black Ops 6", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["BlackOps6.exe", "cod.exe"]),
    spec("fps_cod_mw3", "Call of Duty: Modern Warfare III", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["sp23-cod.exe"]),
    spec("fps_bf2042", "Battlefield 2042", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["BF2042.exe"]),
    spec("fps_bf1", "Battlefield 1", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["bf1.exe"]),
    spec("fps_bfv", "Battlefield V", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["bfv.exe"]),
    spec("fps_bf4", "Battlefield 4", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["bf4.exe"]),
    spec("fps_battlebit", "BattleBit Remastered", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["BattleBit.exe"]),
    spec("fps_hunt_showdown", "Hunt: Showdown 1896", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["HuntGame.exe"]),
    spec("fps_rust", "Rust", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["RustClient.exe"]),
    spec("fps_dayz", "DayZ", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["DayZ_x64.exe"]),
    spec("fps_scum", "SCUM", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["SCUM.exe"]),
    spec("fps_super_people", "Super People", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["BravoHotelClient-Win64-Shipping.exe"]),
    spec("fps_bloodhunt", "Vampire: The Masquerade - Bloodhunt", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["Tiger-Win64-Shipping.exe"]),
    spec("fps_naraka", "NARAKA: BLADEPOINT", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["NarakaBladepoint.exe"]),
    spec("fps_far_cry6", "Far Cry 6", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["FarCry6.exe"]),
    spec("fps_far_cry5", "Far Cry 5", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["FarCry5.exe"]),
    spec("fps_metro_exodus", "Metro Exodus", KeymapCategory::Fps, Archetype::FpsTactical, &["MetroExodus.exe"]),
    spec("fps_stalker2", "S.T.A.L.K.E.R. 2: Heart of Chornobyl", KeymapCategory::Fps, Archetype::FpsTactical, &["Stalker2-Win64-Shipping.exe"]),
    spec("fps_stalker_anomaly", "S.T.A.L.K.E.R. Anomaly", KeymapCategory::Fps, Archetype::FpsTactical, &["AnomalyDX11AVX.exe"]),
    spec("fps_crysis_remastered", "Crysis Remastered", KeymapCategory::Fps, Archetype::FpsHero, &["CrysisRemastered.exe"]),
    spec("fps_wolfenstein2", "Wolfenstein II: The New Colossus", KeymapCategory::Fps, Archetype::FpsHero, &["NewColossus_x64vk.exe"]),
    spec("fps_half_life2", "Half-Life 2", KeymapCategory::Fps, Archetype::FpsTactical, &["hl2.exe", "hl2_linux"]),
    spec("fps_black_mesa", "Black Mesa", KeymapCategory::Fps, Archetype::FpsTactical, &["bms.exe"]),
    spec("fps_portal2", "Portal 2", KeymapCategory::Fps, Archetype::FpsHero, &["portal2.exe"]),
    spec("fps_atomic_heart", "Atomic Heart", KeymapCategory::Fps, Archetype::FpsHero, &["AtomicHeart-Win64-Shipping.exe"]),
    spec("fps_dying_light2", "Dying Light 2", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["DyingLightGame_x64_rwdi.exe"]),
    spec("fps_dead_island2", "Dead Island 2", KeymapCategory::Fps, Archetype::FpsHero, &["DeadIsland-Win64-Shipping.exe"]),
    spec("fps_warhammer_darktide", "Warhammer 40,000: Darktide", KeymapCategory::Fps, Archetype::FpsHero, &["Darktide.exe"]),
    spec("fps_vermintide2", "Warhammer: Vermintide 2", KeymapCategory::Fps, Archetype::FpsHero, &["vermintide2.exe"]),
    spec("fps_helldivers2", "HELLDIVERS 2", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["helldivers2.exe"]),
    spec("fps_space_marine2", "Warhammer 40,000: Space Marine 2", KeymapCategory::Fps, Archetype::FpsHero, &["Warhammer 40000 Space Marine 2.exe"]),
    spec("fps_gtfo", "GTFO", KeymapCategory::Fps, Archetype::FpsTactical, &["GTFO.exe"]),
    spec("fps_six_days_fallujah", "Six Days in Fallujah", KeymapCategory::Fps, Archetype::FpsTactical, &["SixDays-Win64-Shipping.exe"]),
    spec("fps_enlisted", "Enlisted", KeymapCategory::Fps, Archetype::FpsTactical, &["enlisted.exe"]),
    spec("fps_planetside2", "PlanetSide 2", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["PlanetSide2_x64.exe"]),
    spec("fps_ironsight", "Ironsight", KeymapCategory::Fps, Archetype::FpsTactical, &["ironsight.exe"]),
    spec("fps_krunker", "Krunker", KeymapCategory::Fps, Archetype::FpsHero, &["Krunker.exe", "krunker"]),

    // ==================== MOBA (45 presets) ====================
    spec("moba_lol", "League of Legends", KeymapCategory::Moba, Archetype::MobaClassic, &["League of Legends.exe", "LeagueClientUx.exe"]),
    spec("moba_dota2", "Dota 2", KeymapCategory::Moba, Archetype::MobaClassic, &["dota2.exe", "dota2"]),
    spec("moba_hots", "Heroes of the Storm", KeymapCategory::Moba, Archetype::MobaClassic, &["HeroesOfTheStorm_x64.exe"]),
    spec("moba_smite", "SMITE", KeymapCategory::Moba, Archetype::MobaClassic, &["Smite.exe"]),
    spec("moba_smite2", "SMITE 2", KeymapCategory::Moba, Archetype::MobaClassic, &["Hemingway-Win64-Shipping.exe"]),
    spec("moba_eternal_return", "Eternal Return", KeymapCategory::Moba, Archetype::MobaClassic, &["EternalReturn.exe"]),
    spec("moba_predecessor", "Predecessor", KeymapCategory::Moba, Archetype::MobaClassic, &["PredecessorClient-Win64-Shipping.exe"]),
    spec("moba_supervive", "SUPERVIVE", KeymapCategory::Moba, Archetype::MobaClassic, &["SUPERVIVE.exe"]),
    spec("moba_deadlock", "Deadlock", KeymapCategory::Moba, Archetype::FpsHero, &["project8.exe", "deadlock.exe"]),
    spec("moba_pokemon_unite", "Pokemon UNITE (Emulator)", KeymapCategory::Moba, Archetype::MobaClassic, &["pokemonunite"]),
    spec("moba_honor_of_kings", "Honor of Kings (PC)", KeymapCategory::Moba, Archetype::MobaClassic, &["HonorOfKings.exe", "sgame"]),
    spec("moba_wild_rift", "League of Legends: Wild Rift (PC)", KeymapCategory::Moba, Archetype::MobaClassic, &["wildrift"]),
    spec("moba_mobile_legends", "Mobile Legends: Bang Bang (PC)", KeymapCategory::Moba, Archetype::MobaClassic, &["mobilelegends"]),
    spec("moba_arena_of_valor", "Arena of Valor (PC)", KeymapCategory::Moba, Archetype::MobaClassic, &["arenaofvalor"]),
    spec("moba_battlerite", "Battlerite", KeymapCategory::Moba, Archetype::MobaClassic, &["Battlerite.exe"]),
    spec("moba_battlerite_royale", "Battlerite Royale", KeymapCategory::Moba, Archetype::MobaClassic, &["BattleriteRoyale.exe"]),
    spec("moba_vainglory", "Vainglory", KeymapCategory::Moba, Archetype::MobaClassic, &["Vainglory.exe"]),
    spec("moba_awesomenauts", "Awesomenauts", KeymapCategory::Moba, Archetype::ActionPlatformer, &["Awesomenauts.exe"]),
    spec("moba_gigantic", "Gigantic: Rampage Edition", KeymapCategory::Moba, Archetype::FpsHero, &["RxGame-Win64-Shipping.exe"]),
    spec("moba_overprime", "Paragon: The Overprime", KeymapCategory::Moba, Archetype::MobaClassic, &["Overprime-Win64-Shipping.exe"]),
    spec("moba_heroes_of_newerth", "Heroes of Newerth", KeymapCategory::Moba, Archetype::MobaClassic, &["hon_x64.exe"]),
    spec("moba_strife", "Strife", KeymapCategory::Moba, Archetype::MobaClassic, &["strife.exe"]),
    spec("moba_300_heroes", "300 Heroes", KeymapCategory::Moba, Archetype::MobaClassic, &["300.exe"]),
    spec("moba_onmyoji_arena", "Onmyoji Arena (PC)", KeymapCategory::Moba, Archetype::MobaClassic, &["onmyojiarena"]),
    spec("moba_extraordinary_ones", "Extraordinary Ones", KeymapCategory::Moba, Archetype::MobaClassic, &["extraordinaryones"]),
    spec("moba_bleeding_edge", "Bleeding Edge", KeymapCategory::Moba, Archetype::ActionBrawler, &["BleedingEdge.exe"]),
    spec("moba_omega_strikers", "Omega Strikers", KeymapCategory::Moba, Archetype::MobaClassic, &["OmegaStrikers-Win64-Shipping.exe"]),
    spec("moba_fangs", "FANGS", KeymapCategory::Moba, Archetype::MobaClassic, &["Fangs.exe"]),
    spec("moba_fault", "Fault: Elder Orb", KeymapCategory::Moba, Archetype::MobaClassic, &["Fault-Win64-Shipping.exe"]),
    spec("moba_ethereal", "Ethereal: Clash of Souls", KeymapCategory::Moba, Archetype::MobaClassic, &["Ethereal.exe"]),
    spec("moba_bapbap", "BAPBAP", KeymapCategory::Moba, Archetype::MobaClassic, &["bapbap"]),
    spec("moba_seek_and_destroy", "Seekers of Skyveil", KeymapCategory::Moba, Archetype::MobaClassic, &["Skyveil.exe"]),
    spec("moba_project_loki", "Project Loki", KeymapCategory::Moba, Archetype::MobaClassic, &["ProjectLoki.exe"]),
    spec("moba_minion_masters", "Minion Masters", KeymapCategory::Moba, Archetype::MobaClassic, &["MinionMasters.exe"]),
    spec("moba_bloodline_champions", "Bloodline Champions", KeymapCategory::Moba, Archetype::MobaClassic, &["BloodlineChampions.exe"]),
    spec("moba_demigod", "Demigod", KeymapCategory::Moba, Archetype::MobaClassic, &["Demigod.exe"]),
    spec("moba_monday_night_combat", "Super Monday Night Combat", KeymapCategory::Moba, Archetype::FpsHero, &["SuperMNC.exe"]),
    spec("moba_mxm", "Master X Master", KeymapCategory::Moba, Archetype::MobaClassic, &["MXM.exe"]),
    spec("moba_warlander", "Warlander", KeymapCategory::Moba, Archetype::ActionBrawler, &["Warlander.exe"]),
    spec("moba_hyper_universe", "Hyper Universe", KeymapCategory::Moba, Archetype::ActionPlatformer, &["HyperUniverse.exe"]),
    spec("moba_genesia", "Genesis MOBA", KeymapCategory::Moba, Archetype::MobaClassic, &["GenesisMOBA.exe"]),
    spec("moba_dota_underlords", "Dota Underlords", KeymapCategory::Moba, Archetype::SimStrategy, &["underlords.exe"]),
    spec("moba_tft", "Teamfight Tactics", KeymapCategory::Moba, Archetype::SimStrategy, &["tft", "League of Legends.exe"]),
    spec("moba_legion_td2", "Legion TD 2", KeymapCategory::Moba, Archetype::SimStrategy, &["Legion TD 2.exe"]),
    spec("moba_mechabellum", "Mechabellum", KeymapCategory::Moba, Archetype::SimStrategy, &["Mechabellum.exe"]),

    // ==================== RPG (95 presets) ====================
    spec("rpg_elden_ring", "Elden Ring", KeymapCategory::Rpg, Archetype::RpgAction, &["eldenring.exe"]),
    spec("rpg_genshin_impact", "Genshin Impact", KeymapCategory::Rpg, Archetype::RpgAction, &["GenshinImpact.exe", "YuanShen.exe"]),
    spec("rpg_wuthering_waves", "Wuthering Waves", KeymapCategory::Rpg, Archetype::RpgAction, &["Client-Win64-Shipping.exe", "Wuthering Waves.exe"]),
    spec("rpg_honkai_star_rail", "Honkai: Star Rail", KeymapCategory::Rpg, Archetype::RpgAction, &["StarRail.exe"]),
    spec("rpg_zzz", "Zenless Zone Zero", KeymapCategory::Rpg, Archetype::RpgAction, &["ZenlessZoneZero.exe"]),
    spec("rpg_cyberpunk2077", "Cyberpunk 2077", KeymapCategory::Rpg, Archetype::RpgAction, &["Cyberpunk2077.exe"]),
    spec("rpg_witcher3", "The Witcher 3: Wild Hunt", KeymapCategory::Rpg, Archetype::RpgAction, &["witcher3.exe"]),
    spec("rpg_skyrim", "The Elder Scrolls V: Skyrim", KeymapCategory::Rpg, Archetype::RpgAction, &["SkyrimSE.exe", "TESV.exe"]),
    spec("rpg_oblivion", "The Elder Scrolls IV: Oblivion", KeymapCategory::Rpg, Archetype::RpgAction, &["Oblivion.exe"]),
    spec("rpg_fallout4", "Fallout 4", KeymapCategory::Rpg, Archetype::RpgAction, &["Fallout4.exe"]),
    spec("rpg_fallout_nv", "Fallout: New Vegas", KeymapCategory::Rpg, Archetype::RpgAction, &["FalloutNV.exe"]),
    spec("rpg_fallout76", "Fallout 76", KeymapCategory::Rpg, Archetype::RpgAction, &["Fallout76.exe"]),
    spec("rpg_starfield", "Starfield", KeymapCategory::Rpg, Archetype::RpgAction, &["Starfield.exe"]),
    spec("rpg_bg3", "Baldur's Gate 3", KeymapCategory::Rpg, Archetype::RpgArpg, &["bg3.exe", "bg3_dx11.exe"]),
    spec("rpg_dos2", "Divinity: Original Sin 2", KeymapCategory::Rpg, Archetype::RpgArpg, &["EoCApp.exe"]),
    spec("rpg_black_myth_wukong", "Black Myth: Wukong", KeymapCategory::Rpg, Archetype::RpgAction, &["b1-Win64-Shipping.exe"]),
    spec("rpg_mh_world", "Monster Hunter: World", KeymapCategory::Rpg, Archetype::RpgAction, &["MonsterHunterWorld.exe"]),
    spec("rpg_mh_rise", "Monster Hunter Rise", KeymapCategory::Rpg, Archetype::RpgAction, &["MonsterHunterRise.exe"]),
    spec("rpg_mh_wilds", "Monster Hunter Wilds", KeymapCategory::Rpg, Archetype::RpgAction, &["MonsterHunterWilds.exe"]),
    spec("rpg_dark_souls3", "Dark Souls III", KeymapCategory::Rpg, Archetype::RpgAction, &["DarkSoulsIII.exe"]),
    spec("rpg_dark_souls_remastered", "Dark Souls: Remastered", KeymapCategory::Rpg, Archetype::RpgAction, &["DarkSoulsRemastered.exe"]),
    spec("rpg_sekiro", "Sekiro: Shadows Die Twice", KeymapCategory::Rpg, Archetype::RpgAction, &["sekiro.exe"]),
    spec("rpg_lies_of_p", "Lies of P", KeymapCategory::Rpg, Archetype::RpgAction, &["LOP-Win64-Shipping.exe"]),
    spec("rpg_lords_of_fallen", "Lords of the Fallen", KeymapCategory::Rpg, Archetype::RpgAction, &["LOTF2-Win64-Shipping.exe"]),
    spec("rpg_nioh2", "Nioh 2", KeymapCategory::Rpg, Archetype::RpgAction, &["nioh2.exe"]),
    spec("rpg_wo_long", "Wo Long: Fallen Dynasty", KeymapCategory::Rpg, Archetype::RpgAction, &["WoLong.exe"]),
    spec("rpg_hogwarts_legacy", "Hogwarts Legacy", KeymapCategory::Rpg, Archetype::RpgAction, &["HogwartsLegacy.exe"]),
    spec("rpg_ff16", "Final Fantasy XVI", KeymapCategory::Rpg, Archetype::RpgAction, &["ffxvi.exe"]),
    spec("rpg_ff7_remake", "Final Fantasy VII Remake", KeymapCategory::Rpg, Archetype::RpgAction, &["ff7remake_.exe"]),
    spec("rpg_ff7_rebirth", "Final Fantasy VII Rebirth", KeymapCategory::Rpg, Archetype::RpgAction, &["ff7rebirth_.exe"]),
    spec("rpg_dragons_dogma2", "Dragon's Dogma 2", KeymapCategory::Rpg, Archetype::RpgAction, &["DD2.exe"]),
    spec("rpg_dragon_age_veilguard", "Dragon Age: The Veilguard", KeymapCategory::Rpg, Archetype::RpgAction, &["Dragon Age The Veilguard.exe"]),
    spec("rpg_mass_effect_le", "Mass Effect Legendary Edition", KeymapCategory::Rpg, Archetype::RpgAction, &["MassEffect1.exe", "MassEffect2.exe", "MassEffect3.exe"]),
    spec("rpg_persona5_royal", "Persona 5 Royal", KeymapCategory::Rpg, Archetype::RpgAction, &["P5R.exe"]),
    spec("rpg_persona3_reload", "Persona 3 Reload", KeymapCategory::Rpg, Archetype::RpgAction, &["P3R.exe"]),
    spec("rpg_metaphor_refantazio", "Metaphor: ReFantazio", KeymapCategory::Rpg, Archetype::RpgAction, &["METAPHOR.exe"]),
    spec("rpg_yakuza_lad", "Yakuza: Like a Dragon", KeymapCategory::Rpg, Archetype::RpgAction, &["YakuzaLikeADragon.exe"]),
    spec("rpg_infinite_wealth", "Like a Dragon: Infinite Wealth", KeymapCategory::Rpg, Archetype::RpgAction, &["likeadragon8.exe"]),
    spec("rpg_kingdom_come", "Kingdom Come: Deliverance", KeymapCategory::Rpg, Archetype::RpgAction, &["KingdomCome.exe"]),
    spec("rpg_kingdom_come2", "Kingdom Come: Deliverance II", KeymapCategory::Rpg, Archetype::RpgAction, &["KingdomCome2.exe"]),
    spec("rpg_mount_blade2", "Mount & Blade II: Bannerlord", KeymapCategory::Rpg, Archetype::RpgAction, &["Bannerlord.exe"]),
    spec("rpg_kenshi", "Kenshi", KeymapCategory::Rpg, Archetype::SimStrategy, &["kenshi_x64.exe"]),
    spec("rpg_disco_elysium", "Disco Elysium", KeymapCategory::Rpg, Archetype::RpgArpg, &["disco.exe"]),
    spec("rpg_pillars2", "Pillars of Eternity II: Deadfire", KeymapCategory::Rpg, Archetype::RpgArpg, &["PillarsOfEternityII.exe"]),
    spec("rpg_pathfinder_wotr", "Pathfinder: Wrath of the Righteous", KeymapCategory::Rpg, Archetype::RpgArpg, &["Wrath.exe"]),
    spec("rpg_rogue_trader", "Warhammer 40,000: Rogue Trader", KeymapCategory::Rpg, Archetype::RpgArpg, &["WH40KRT.exe"]),
    spec("rpg_wasteland3", "Wasteland 3", KeymapCategory::Rpg, Archetype::RpgArpg, &["WL3.exe"]),
    spec("rpg_outer_worlds", "The Outer Worlds", KeymapCategory::Rpg, Archetype::RpgAction, &["IndianaEpicGameStore-Win64-Shipping.exe"]),
    spec("rpg_avowed", "Avowed", KeymapCategory::Rpg, Archetype::RpgAction, &["Avowed-Win64-Shipping.exe"]),
    spec("rpg_greedfall", "GreedFall", KeymapCategory::Rpg, Archetype::RpgAction, &["GreedFall.exe"]),
    spec("rpg_diablo4", "Diablo IV", KeymapCategory::Rpg, Archetype::RpgArpg, &["Diablo IV.exe"]),
    spec("rpg_diablo3", "Diablo III", KeymapCategory::Rpg, Archetype::RpgArpg, &["Diablo III64.exe"]),
    spec("rpg_diablo2_resurrected", "Diablo II: Resurrected", KeymapCategory::Rpg, Archetype::RpgArpg, &["D2R.exe"]),
    spec("rpg_poe", "Path of Exile", KeymapCategory::Rpg, Archetype::RpgArpg, &["PathOfExileSteam.exe", "PathOfExile_x64.exe"]),
    spec("rpg_poe2", "Path of Exile 2", KeymapCategory::Rpg, Archetype::RpgArpg, &["PathOfExile2Steam.exe", "PathOfExile2_x64.exe"]),
    spec("rpg_last_epoch", "Last Epoch", KeymapCategory::Rpg, Archetype::RpgArpg, &["Last Epoch.exe"]),
    spec("rpg_grim_dawn", "Grim Dawn", KeymapCategory::Rpg, Archetype::RpgArpg, &["Grim Dawn.exe"]),
    spec("rpg_torchlight_inf", "Torchlight: Infinite", KeymapCategory::Rpg, Archetype::RpgArpg, &["torchlight_infinite.exe"]),
    spec("rpg_lost_ark", "Lost Ark", KeymapCategory::Rpg, Archetype::RpgArpg, &["LOSTARK.exe"]),
    spec("rpg_hades", "Hades", KeymapCategory::Rpg, Archetype::RpgArpg, &["Hades.exe"]),
    spec("rpg_hades2", "Hades II", KeymapCategory::Rpg, Archetype::RpgArpg, &["Hades2.exe"]),
    spec("rpg_no_rest_wicked", "No Rest for the Wicked", KeymapCategory::Rpg, Archetype::RpgArpg, &["NoRestForTheWicked.exe"]),
    spec("rpg_v_rising", "V Rising", KeymapCategory::Rpg, Archetype::RpgArpg, &["VRising.exe"]),
    spec("rpg_albion_online", "Albion Online", KeymapCategory::Rpg, Archetype::RpgArpg, &["Albion-Online.exe"]),
    spec("rpg_undecember", "UNDECEMBER", KeymapCategory::Rpg, Archetype::RpgArpg, &["RZGame-Win64-Shipping.exe"]),
    spec("rpg_wolcen", "Wolcen: Lords of Mayhem", KeymapCategory::Rpg, Archetype::RpgArpg, &["wolcen.exe"]),
    spec("rpg_titan_quest", "Titan Quest Anniversary", KeymapCategory::Rpg, Archetype::RpgArpg, &["TQ.exe"]),
    spec("rpg_wow", "World of Warcraft", KeymapCategory::Rpg, Archetype::RpgMmo, &["Wow.exe", "WowClassic.exe"]),
    spec("rpg_ff14", "Final Fantasy XIV", KeymapCategory::Rpg, Archetype::RpgMmo, &["ffxiv_dx11.exe"]),
    spec("rpg_gw2", "Guild Wars 2", KeymapCategory::Rpg, Archetype::RpgMmo, &["Gw2-64.exe"]),
    spec("rpg_eso", "The Elder Scrolls Online", KeymapCategory::Rpg, Archetype::RpgMmo, &["eso64.exe"]),
    spec("rpg_new_world", "New World: Aeternum", KeymapCategory::Rpg, Archetype::RpgMmo, &["NewWorld.exe"]),
    spec("rpg_bdo", "Black Desert Online", KeymapCategory::Rpg, Archetype::RpgMmo, &["BlackDesert64.exe"]),
    spec("rpg_throne_liberty", "Throne and Liberty", KeymapCategory::Rpg, Archetype::RpgMmo, &["TL.exe"]),
    spec("rpg_osrs", "Old School RuneScape", KeymapCategory::Rpg, Archetype::RpgMmo, &["RuneLite.exe", "osclient.exe"]),
    spec("rpg_rs3", "RuneScape 3", KeymapCategory::Rpg, Archetype::RpgMmo, &["rs2client.exe"]),
    spec("rpg_maplestory", "MapleStory", KeymapCategory::Rpg, Archetype::RpgMmo, &["MapleStory.exe"]),
    spec("rpg_dnf", "Dungeon Fighter Online", KeymapCategory::Rpg, Archetype::ActionPlatformer, &["DNF.exe"]),
    spec("rpg_warframe", "Warframe", KeymapCategory::Rpg, Archetype::RpgMmo, &["Warframe.x64.exe"]),
    spec("rpg_first_descendant", "The First Descendant", KeymapCategory::Rpg, Archetype::RpgMmo, &["M1-Win64-Shipping.exe"]),
    spec("rpg_swtor", "Star Wars: The Old Republic", KeymapCategory::Rpg, Archetype::RpgMmo, &["swtor.exe"]),
    spec("rpg_eve_online", "EVE Online", KeymapCategory::Rpg, Archetype::RpgMmo, &["exefile.exe"]),
    spec("rpg_pso2_ngs", "Phantasy Star Online 2: NGS", KeymapCategory::Rpg, Archetype::RpgMmo, &["pso2.exe"]),
    spec("rpg_tower_of_fantasy", "Tower of Fantasy", KeymapCategory::Rpg, Archetype::RpgAction, &["QRSL.exe"]),
    spec("rpg_once_human", "Once Human", KeymapCategory::Rpg, Archetype::RpgAction, &["ONCE_HUMAN.exe"]),
    spec("rpg_tales_of_arise", "Tales of Arise", KeymapCategory::Rpg, Archetype::RpgAction, &["Tales of Arise.exe"]),
    spec("rpg_granblue_relink", "Granblue Fantasy: Relink", KeymapCategory::Rpg, Archetype::RpgAction, &["granblue_fantasy_relink.exe"]),
    spec("rpg_octopath2", "Octopath Traveler II", KeymapCategory::Rpg, Archetype::RpgAction, &["Octopath_Traveler2-Win64-Shipping.exe"]),
    spec("rpg_dq11", "Dragon Quest XI S", KeymapCategory::Rpg, Archetype::RpgAction, &["DRAGON QUEST XI S.exe"]),
    spec("rpg_sea_of_stars", "Sea of Stars", KeymapCategory::Rpg, Archetype::ActionPlatformer, &["SeaOfStars.exe"]),
    spec("rpg_undertale", "Undertale", KeymapCategory::Rpg, Archetype::ActionPlatformer, &["UNDERTALE.exe"]),
    spec("rpg_omori", "OMORI", KeymapCategory::Rpg, Archetype::ActionPlatformer, &["OMORI.exe"]),
    spec("rpg_darkest_dungeon2", "Darkest Dungeon II", KeymapCategory::Rpg, Archetype::SimStrategy, &["Darkest Dungeon II.exe"]),
    spec("rpg_remnant2", "Remnant II", KeymapCategory::Rpg, Archetype::RpgAction, &["Remnant2-Win64-Shipping.exe"]),
    spec("rpg_banishers", "Banishers: Ghosts of New Eden", KeymapCategory::Rpg, Archetype::RpgAction, &["Banishers-Win64-Shipping.exe"]),

    // ==================== ACTION (90 presets) ====================
    spec("act_gta5", "Grand Theft Auto V", KeymapCategory::Action, Archetype::ActionBrawler, &["GTA5.exe", "PlayGTAV.exe"]),
    spec("act_rdr2", "Red Dead Redemption 2", KeymapCategory::Action, Archetype::ActionBrawler, &["RDR2.exe"]),
    spec("act_god_of_war", "God of War", KeymapCategory::Action, Archetype::ActionBrawler, &["GoW.exe"]),
    spec("act_gow_ragnarok", "God of War Ragnarok", KeymapCategory::Action, Archetype::ActionBrawler, &["GoWR.exe"]),
    spec("act_spiderman", "Marvel's Spider-Man Remastered", KeymapCategory::Action, Archetype::ActionBrawler, &["Spider-Man.exe"]),
    spec("act_spiderman_mm", "Spider-Man: Miles Morales", KeymapCategory::Action, Archetype::ActionBrawler, &["MilesMorales.exe"]),
    spec("act_spiderman2", "Marvel's Spider-Man 2", KeymapCategory::Action, Archetype::ActionBrawler, &["Spider-Man2.exe"]),
    spec("act_ghost_tsushima", "Ghost of Tsushima", KeymapCategory::Action, Archetype::ActionBrawler, &["GhostOfTsushima.exe"]),
    spec("act_dmc5", "Devil May Cry 5", KeymapCategory::Action, Archetype::ActionBrawler, &["DevilMayCry5.exe"]),
    spec("act_nier_automata", "NieR:Automata", KeymapCategory::Action, Archetype::ActionBrawler, &["NieRAutomata.exe"]),
    spec("act_nier_replicant", "NieR Replicant", KeymapCategory::Action, Archetype::ActionBrawler, &["NieR Replicant ver.1.22474487139.exe"]),
    spec("act_batman_arkham_knight", "Batman: Arkham Knight", KeymapCategory::Action, Archetype::ActionBrawler, &["BatmanAK.exe"]),
    spec("act_ac_mirage", "Assassin's Creed Mirage", KeymapCategory::Action, Archetype::ActionBrawler, &["ACMirage.exe"]),
    spec("act_ac_valhalla", "Assassin's Creed Valhalla", KeymapCategory::Action, Archetype::ActionBrawler, &["ACValhalla.exe"]),
    spec("act_ac_odyssey", "Assassin's Creed Odyssey", KeymapCategory::Action, Archetype::ActionBrawler, &["ACOdyssey.exe"]),
    spec("act_ac_shadows", "Assassin's Creed Shadows", KeymapCategory::Action, Archetype::ActionBrawler, &["ACShadows.exe"]),
    spec("act_horizon_zd", "Horizon Zero Dawn", KeymapCategory::Action, Archetype::ActionBrawler, &["HorizonZeroDawn.exe"]),
    spec("act_horizon_fw", "Horizon Forbidden West", KeymapCategory::Action, Archetype::ActionBrawler, &["HorizonForbiddenWest.exe"]),
    spec("act_tlou1", "The Last of Us Part I", KeymapCategory::Action, Archetype::ActionBrawler, &["tlou-i.exe"]),
    spec("act_uncharted4", "UNCHARTED: Legacy of Thieves", KeymapCategory::Action, Archetype::ActionBrawler, &["u4.exe"]),
    spec("act_shadow_tomb_raider", "Shadow of the Tomb Raider", KeymapCategory::Action, Archetype::ActionBrawler, &["SOTTR.exe"]),
    spec("act_mgsv", "Metal Gear Solid V: TPP", KeymapCategory::Action, Archetype::ActionBrawler, &["mgsvtpp.exe"]),
    spec("act_death_stranding", "Death Stranding", KeymapCategory::Action, Archetype::ActionBrawler, &["ds.exe"]),
    spec("act_control", "Control", KeymapCategory::Action, Archetype::ActionBrawler, &["Control_DX12.exe", "Control_DX11.exe"]),
    spec("act_alan_wake2", "Alan Wake 2", KeymapCategory::Action, Archetype::ActionBrawler, &["AlanWake2.exe"]),
    spec("act_re4_remake", "Resident Evil 4 Remake", KeymapCategory::Action, Archetype::ActionBrawler, &["re4.exe"]),
    spec("act_re2_remake", "Resident Evil 2 Remake", KeymapCategory::Action, Archetype::ActionBrawler, &["re2.exe"]),
    spec("act_re_village", "Resident Evil Village", KeymapCategory::Action, Archetype::ActionBrawler, &["re8.exe"]),
    spec("act_silent_hill2", "Silent Hill 2 Remake", KeymapCategory::Action, Archetype::ActionBrawler, &["SHProto-Win64-Shipping.exe"]),
    spec("act_dead_space", "Dead Space Remake", KeymapCategory::Action, Archetype::ActionBrawler, &["Dead Space.exe"]),
    spec("act_jedi_survivor", "Star Wars Jedi: Survivor", KeymapCategory::Action, Archetype::ActionBrawler, &["JediSurvivor.exe"]),
    spec("act_jedi_fallen", "Star Wars Jedi: Fallen Order", KeymapCategory::Action, Archetype::ActionBrawler, &["starwarsjedifallenorder.exe"]),
    spec("act_sifu", "Sifu", KeymapCategory::Action, Archetype::ActionBrawler, &["Sifu-Win64-Shipping.exe"]),
    spec("act_hi_fi_rush", "Hi-Fi RUSH", KeymapCategory::Action, Archetype::ActionBrawler, &["Hi-Fi-RUSH.exe"]),
    spec("act_bayonetta", "Bayonetta", KeymapCategory::Action, Archetype::ActionBrawler, &["Bayonetta.exe"]),
    spec("act_armored_core6", "Armored Core VI", KeymapCategory::Action, Archetype::ActionBrawler, &["armoredcore6.exe"]),
    spec("act_for_honor", "For Honor", KeymapCategory::Action, Archetype::ActionBrawler, &["forhonor.exe"]),
    spec("act_chivalry2", "Chivalry 2", KeymapCategory::Action, Archetype::ActionBrawler, &["Chivalry2-Win64-Shipping.exe"]),
    spec("act_mordhau", "MORDHAU", KeymapCategory::Action, Archetype::ActionBrawler, &["Mordhau-Win64-Shipping.exe"]),
    spec("act_street_fighter6", "Street Fighter 6", KeymapCategory::Action, Archetype::ActionPlatformer, &["StreetFighter6.exe"]),
    spec("act_tekken8", "TEKKEN 8", KeymapCategory::Action, Archetype::ActionPlatformer, &["Polaris-Win64-Shipping.exe"]),
    spec("act_guilty_gear_strive", "Guilty Gear -Strive-", KeymapCategory::Action, Archetype::ActionPlatformer, &["GGST-Win64-Shipping.exe"]),
    spec("act_mortal_kombat1", "Mortal Kombat 1", KeymapCategory::Action, Archetype::ActionPlatformer, &["MK12.exe"]),
    spec("act_dbz_sparking_zero", "Dragon Ball: Sparking! ZERO", KeymapCategory::Action, Archetype::ActionBrawler, &["SparkingZERO-Win64-Shipping.exe"]),
    spec("act_hitman3", "HITMAN World of Assassination", KeymapCategory::Action, Archetype::ActionBrawler, &["HITMAN3.exe"]),
    spec("act_watch_dogs2", "Watch Dogs 2", KeymapCategory::Action, Archetype::ActionBrawler, &["WatchDogs2.exe"]),
    spec("act_sleeping_dogs", "Sleeping Dogs", KeymapCategory::Action, Archetype::ActionBrawler, &["SDHDShip.exe"]),
    spec("act_just_cause4", "Just Cause 4", KeymapCategory::Action, Archetype::ActionBrawler, &["JustCause4.exe"]),
    spec("act_mafia_de", "Mafia: Definitive Edition", KeymapCategory::Action, Archetype::ActionBrawler, &["mafiadefinitiveedition.exe"]),
    spec("act_ratchet_rift_apart", "Ratchet & Clank: Rift Apart", KeymapCategory::Action, Archetype::ActionBrawler, &["RiftApart.exe"]),
    spec("act_returnal", "Returnal", KeymapCategory::Action, Archetype::ActionBrawler, &["Returnal-Win64-Shipping.exe"]),
    spec("act_hollow_knight", "Hollow Knight", KeymapCategory::Action, Archetype::ActionPlatformer, &["hollow_knight.exe"]),
    spec("act_celeste", "Celeste", KeymapCategory::Action, Archetype::ActionPlatformer, &["Celeste.exe"]),
    spec("act_dead_cells", "Dead Cells", KeymapCategory::Action, Archetype::ActionPlatformer, &["deadcells.exe"]),
    spec("act_ori_wotw", "Ori and the Will of the Wisps", KeymapCategory::Action, Archetype::ActionPlatformer, &["oriwotw.exe"]),
    spec("act_cuphead", "Cuphead", KeymapCategory::Action, Archetype::ActionPlatformer, &["Cuphead.exe"]),
    spec("act_nine_sols", "Nine Sols", KeymapCategory::Action, Archetype::ActionPlatformer, &["NineSols.exe"]),
    spec("act_blasphemous2", "Blasphemous 2", KeymapCategory::Action, Archetype::ActionPlatformer, &["Blasphemous 2.exe"]),
    spec("act_prince_of_persia_lc", "Prince of Persia: The Lost Crown", KeymapCategory::Action, Archetype::ActionPlatformer, &["TheLostCrown.exe"]),
    spec("act_shovel_knight", "Shovel Knight", KeymapCategory::Action, Archetype::ActionPlatformer, &["ShovelKnight.exe"]),
    spec("act_katana_zero", "Katana ZERO", KeymapCategory::Action, Archetype::ActionPlatformer, &["Katana ZERO.exe"]),
    spec("act_hotline_miami", "Hotline Miami", KeymapCategory::Action, Archetype::ActionPlatformer, &["HotlineMiami.exe"]),
    spec("act_enter_the_gungeon", "Enter the Gungeon", KeymapCategory::Action, Archetype::ActionPlatformer, &["EtG.exe"]),
    spec("act_binding_of_isaac", "The Binding of Isaac: Repentance", KeymapCategory::Action, Archetype::ActionPlatformer, &["isaac-ng.exe"]),
    spec("act_vampire_survivors", "Vampire Survivors", KeymapCategory::Action, Archetype::ActionPlatformer, &["VampireSurvivors.exe"]),
    spec("act_brotato", "Brotato", KeymapCategory::Action, Archetype::ActionPlatformer, &["Brotato.exe"]),
    spec("act_cult_of_the_lamb", "Cult of the Lamb", KeymapCategory::Action, Archetype::ActionPlatformer, &["Cult Of The Lamb.exe"]),
    spec("act_dave_the_diver", "DAVE THE DIVER", KeymapCategory::Action, Archetype::ActionPlatformer, &["DaveTheDiver.exe"]),
    spec("act_minecraft", "Minecraft", KeymapCategory::Action, Archetype::ActionSurvival, &["javaw.exe", "Minecraft.Windows.exe", "minecraft"]),
    spec("act_terraria", "Terraria", KeymapCategory::Action, Archetype::ActionSurvival, &["Terraria.exe"]),
    spec("act_palworld", "Palworld", KeymapCategory::Action, Archetype::ActionSurvival, &["Palworld-Win64-Shipping.exe"]),
    spec("act_valheim", "Valheim", KeymapCategory::Action, Archetype::ActionSurvival, &["valheim.exe"]),
    spec("act_enshrouded", "Enshrouded", KeymapCategory::Action, Archetype::ActionSurvival, &["enshrouded.exe"]),
    spec("act_subnautica", "Subnautica", KeymapCategory::Action, Archetype::ActionSurvival, &["Subnautica.exe"]),
    spec("act_no_mans_sky", "No Man's Sky", KeymapCategory::Action, Archetype::ActionSurvival, &["NMS.exe"]),
    spec("act_ark_ascended", "ARK: Survival Ascended", KeymapCategory::Action, Archetype::ActionSurvival, &["ArkAscended.exe"]),
    spec("act_sons_of_forest", "Sons Of The Forest", KeymapCategory::Action, Archetype::ActionSurvival, &["SonsOfTheForest.exe"]),
    spec("act_project_zomboid", "Project Zomboid", KeymapCategory::Action, Archetype::ActionSurvival, &["ProjectZomboid64.exe"]),
    spec("act_7days_to_die", "7 Days to Die", KeymapCategory::Action, Archetype::ActionSurvival, &["7DaysToDie.exe"]),
    spec("act_dont_starve", "Don't Starve Together", KeymapCategory::Action, Archetype::ActionSurvival, &["dontstarve_steam_x64.exe"]),
    spec("act_raft", "Raft", KeymapCategory::Action, Archetype::ActionSurvival, &["Raft.exe"]),
    spec("act_grounded", "Grounded", KeymapCategory::Action, Archetype::ActionSurvival, &["Maine-Win64-Shipping.exe"]),
    spec("act_sea_of_thieves", "Sea of Thieves", KeymapCategory::Action, Archetype::ActionSurvival, &["SoTGame.exe"]),
    spec("act_lethal_company", "Lethal Company", KeymapCategory::Action, Archetype::ActionSurvival, &["Lethal Company.exe"]),
    spec("act_phasmophobia", "Phasmophobia", KeymapCategory::Action, Archetype::ActionSurvival, &["Phasmophobia.exe"]),
    spec("act_dead_by_daylight", "Dead by Daylight", KeymapCategory::Action, Archetype::ActionSurvival, &["DeadByDaylight-Win64-Shipping.exe"]),
    spec("act_among_us", "Among Us", KeymapCategory::Action, Archetype::ActionPlatformer, &["Among Us.exe"]),
    spec("act_fall_guys", "Fall Guys", KeymapCategory::Action, Archetype::ActionPlatformer, &["FallGuys_client_game.exe"]),
    spec("act_it_takes_two", "It Takes Two", KeymapCategory::Action, Archetype::ActionBrawler, &["ItTakesTwo.exe"]),
    spec("act_roblox", "Roblox", KeymapCategory::Action, Archetype::ActionSurvival, &["RobloxPlayerBeta.exe"]),

    // ==================== RACING (50 presets) ====================
    spec("race_fh5", "Forza Horizon 5", KeymapCategory::Racing, Archetype::RacingArcade, &["ForzaHorizon5.exe"]),
    spec("race_fh4", "Forza Horizon 4", KeymapCategory::Racing, Archetype::RacingArcade, &["ForzaHorizon4.exe"]),
    spec("race_fm2023", "Forza Motorsport", KeymapCategory::Racing, Archetype::RacingSim, &["forza_gaming.desktop.x64_release_final.exe", "ForzaMotorsport.exe"]),
    spec("race_nfs_unbound", "Need for Speed Unbound", KeymapCategory::Racing, Archetype::RacingArcade, &["NeedForSpeedUnbound.exe"]),
    spec("race_nfs_heat", "Need for Speed Heat", KeymapCategory::Racing, Archetype::RacingArcade, &["NeedForSpeedHeat.exe"]),
    spec("race_nfs_hot_pursuit", "Need for Speed Hot Pursuit Remastered", KeymapCategory::Racing, Archetype::RacingArcade, &["NFS11Remastered.exe"]),
    spec("race_crew_motorfest", "The Crew Motorfest", KeymapCategory::Racing, Archetype::RacingArcade, &["TheCrewMotorfest.exe"]),
    spec("race_crew2", "The Crew 2", KeymapCategory::Racing, Archetype::RacingArcade, &["TheCrew2.exe"]),
    spec("race_f1_24", "F1 24", KeymapCategory::Racing, Archetype::RacingSim, &["F1_24.exe"]),
    spec("race_f1_23", "F1 23", KeymapCategory::Racing, Archetype::RacingSim, &["F1_23.exe"]),
    spec("race_f1_22", "F1 22", KeymapCategory::Racing, Archetype::RacingSim, &["F1_22.exe"]),
    spec("race_assetto_corsa", "Assetto Corsa", KeymapCategory::Racing, Archetype::RacingSim, &["acs.exe"]),
    spec("race_acc", "Assetto Corsa Competizione", KeymapCategory::Racing, Archetype::RacingSim, &["AC2-Win64-Shipping.exe"]),
    spec("race_ac_evo", "Assetto Corsa EVO", KeymapCategory::Racing, Archetype::RacingSim, &["AssettoCorsaEVO.exe"]),
    spec("race_iracing", "iRacing", KeymapCategory::Racing, Archetype::RacingSim, &["iRacingSim64DX11.exe"]),
    spec("race_rfactor2", "rFactor 2", KeymapCategory::Racing, Archetype::RacingSim, &["rFactor2.exe"]),
    spec("race_ams2", "Automobilista 2", KeymapCategory::Racing, Archetype::RacingSim, &["AMS2AVX.exe"]),
    spec("race_raceroom", "RaceRoom Racing Experience", KeymapCategory::Racing, Archetype::RacingSim, &["RRRE64.exe"]),
    spec("race_le_mans_ultimate", "Le Mans Ultimate", KeymapCategory::Racing, Archetype::RacingSim, &["Le Mans Ultimate.exe"]),
    spec("race_dirt_rally2", "DiRT Rally 2.0", KeymapCategory::Racing, Archetype::RacingSim, &["dirtrally2.exe"]),
    spec("race_ea_wrc", "EA SPORTS WRC", KeymapCategory::Racing, Archetype::RacingSim, &["WRC.exe"]),
    spec("race_dirt5", "DIRT 5", KeymapCategory::Racing, Archetype::RacingArcade, &["DIRT5.exe"]),
    spec("race_beamng", "BeamNG.drive", KeymapCategory::Racing, Archetype::RacingSim, &["BeamNG.drive.x64.exe"]),
    spec("race_ets2", "Euro Truck Simulator 2", KeymapCategory::Racing, Archetype::RacingSim, &["eurotrucks2.exe"]),
    spec("race_ats", "American Truck Simulator", KeymapCategory::Racing, Archetype::RacingSim, &["amtrucks.exe"]),
    spec("race_snowrunner", "SnowRunner", KeymapCategory::Racing, Archetype::RacingSim, &["SnowRunner.exe"]),
    spec("race_mudrunner", "MudRunner", KeymapCategory::Racing, Archetype::RacingSim, &["MudRunner.exe"]),
    spec("race_expeditions", "Expeditions: A MudRunner Game", KeymapCategory::Racing, Archetype::RacingSim, &["Expeditions.exe"]),
    spec("race_rocket_league", "Rocket League", KeymapCategory::Racing, Archetype::RacingArcade, &["RocketLeague.exe"]),
    spec("race_trackmania", "Trackmania", KeymapCategory::Racing, Archetype::RacingArcade, &["Trackmania.exe"]),
    spec("race_wreckfest", "Wreckfest", KeymapCategory::Racing, Archetype::RacingArcade, &["Wreckfest_x64.exe"]),
    spec("race_burnout_paradise", "Burnout Paradise Remastered", KeymapCategory::Racing, Archetype::RacingArcade, &["BurnoutPR.exe"]),
    spec("race_grid_legends", "GRID Legends", KeymapCategory::Racing, Archetype::RacingArcade, &["GridLegends.exe"]),
    spec("race_project_cars2", "Project CARS 2", KeymapCategory::Racing, Archetype::RacingSim, &["pCARS2AVX.exe"]),
    spec("race_carx_drift", "CarX Drift Racing Online", KeymapCategory::Racing, Archetype::RacingArcade, &["Drift Racing Online.exe"]),
    spec("race_carx_street", "CarX Street", KeymapCategory::Racing, Archetype::RacingArcade, &["CarX Street.exe"]),
    spec("race_test_drive_sc", "Test Drive Unlimited Solar Crown", KeymapCategory::Racing, Archetype::RacingArcade, &["TDUSC.exe"]),
    spec("race_asphalt_unite", "Asphalt Legends Unite", KeymapCategory::Racing, Archetype::RacingArcade, &["Asphalt9_Steam_x64_rtl.exe"]),
    spec("race_hot_wheels2", "Hot Wheels Unleashed 2", KeymapCategory::Racing, Archetype::RacingArcade, &["hotwheels2-Win64-Shipping.exe"]),
    spec("race_sonic_allstars", "Sonic & All-Stars Racing Transformed", KeymapCategory::Racing, Archetype::RacingArcade, &["ASN_App_PcDx9_Final.exe"]),
    spec("race_kartrider_drift", "KartRider: Drift", KeymapCategory::Racing, Archetype::RacingArcade, &["KartDrift-Win64-Shipping.exe"]),
    spec("race_art_of_rally", "art of rally", KeymapCategory::Racing, Archetype::RacingArcade, &["artofrally.exe"]),
    spec("race_horizon_chase", "Horizon Chase Turbo", KeymapCategory::Racing, Archetype::RacingArcade, &["HorizonChase.exe"]),
    spec("race_redout2", "Redout 2", KeymapCategory::Racing, Archetype::RacingArcade, &["Redout2-Win64-Shipping.exe"]),
    spec("race_ride5", "RIDE 5", KeymapCategory::Racing, Archetype::RacingSim, &["ride5-Win64-Shipping.exe"]),
    spec("race_motogp24", "MotoGP 24", KeymapCategory::Racing, Archetype::RacingSim, &["motogp24-Win64-Shipping.exe"]),
    spec("race_mx_bikes", "MX Bikes", KeymapCategory::Racing, Archetype::RacingSim, &["mxbikes.exe"]),
    spec("race_my_summer_car", "My Summer Car", KeymapCategory::Racing, Archetype::RacingSim, &["mysummercar.exe"]),
    spec("race_pacific_drive", "Pacific Drive", KeymapCategory::Racing, Archetype::RacingArcade, &["PenDriverPro-Win64-Shipping.exe"]),
    spec("race_nascar_heat5", "NASCAR Heat 5", KeymapCategory::Racing, Archetype::RacingSim, &["NASCARHeat5.exe"]),

    // ==================== SIMULATOR (65 presets) ====================
    spec("sim_msfs2020", "Microsoft Flight Simulator", KeymapCategory::Simulator, Archetype::SimFlight, &["FlightSimulator.exe"]),
    spec("sim_msfs2024", "Microsoft Flight Simulator 2024", KeymapCategory::Simulator, Archetype::SimFlight, &["FlightSimulator2024.exe"]),
    spec("sim_xplane12", "X-Plane 12", KeymapCategory::Simulator, Archetype::SimFlight, &["X-Plane.exe", "X-Plane-x86_64"]),
    spec("sim_dcs_world", "DCS World", KeymapCategory::Simulator, Archetype::SimFlight, &["DCS.exe"]),
    spec("sim_war_thunder", "War Thunder", KeymapCategory::Simulator, Archetype::SimFlight, &["aces.exe"]),
    spec("sim_il2_sturmovik", "IL-2 Sturmovik: Great Battles", KeymapCategory::Simulator, Archetype::SimFlight, &["Il-2.exe"]),
    spec("sim_star_citizen", "Star Citizen", KeymapCategory::Simulator, Archetype::SimFlight, &["StarCitizen.exe"]),
    spec("sim_elite_dangerous", "Elite Dangerous", KeymapCategory::Simulator, Archetype::SimFlight, &["EliteDangerous64.exe"]),
    spec("sim_ksp", "Kerbal Space Program", KeymapCategory::Simulator, Archetype::SimFlight, &["KSP_x64.exe"]),
    spec("sim_ksp2", "Kerbal Space Program 2", KeymapCategory::Simulator, Archetype::SimFlight, &["KSP2_x64.exe"]),
    spec("sim_ace_combat7", "Ace Combat 7", KeymapCategory::Simulator, Archetype::SimFlight, &["Ace7Game.exe"]),
    spec("sim_project_wingman", "Project Wingman", KeymapCategory::Simulator, Archetype::SimFlight, &["ProjectWingman-Win64-Shipping.exe"]),
    spec("sim_x4_foundations", "X4: Foundations", KeymapCategory::Simulator, Archetype::SimFlight, &["X4.exe"]),
    spec("sim_world_of_tanks", "World of Tanks", KeymapCategory::Simulator, Archetype::SimFlight, &["WorldOfTanks.exe"]),
    spec("sim_world_of_warships", "World of Warships", KeymapCategory::Simulator, Archetype::SimFlight, &["WorldOfWarships64.exe"]),
    spec("sim_civ6", "Sid Meier's Civilization VI", KeymapCategory::Simulator, Archetype::SimStrategy, &["CivilizationVI.exe", "CivilizationVI_DX12.exe"]),
    spec("sim_civ7", "Sid Meier's Civilization VII", KeymapCategory::Simulator, Archetype::SimStrategy, &["Civ7_Win64_DX12_FinalRelease.exe"]),
    spec("sim_cities_skylines", "Cities: Skylines", KeymapCategory::Simulator, Archetype::SimStrategy, &["Cities.exe"]),
    spec("sim_cities_skylines2", "Cities: Skylines II", KeymapCategory::Simulator, Archetype::SimStrategy, &["Cities2.exe"]),
    spec("sim_stellaris", "Stellaris", KeymapCategory::Simulator, Archetype::SimStrategy, &["stellaris.exe"]),
    spec("sim_hoi4", "Hearts of Iron IV", KeymapCategory::Simulator, Archetype::SimStrategy, &["hoi4.exe"]),
    spec("sim_eu4", "Europa Universalis IV", KeymapCategory::Simulator, Archetype::SimStrategy, &["eu4.exe"]),
    spec("sim_ck3", "Crusader Kings III", KeymapCategory::Simulator, Archetype::SimStrategy, &["ck3.exe"]),
    spec("sim_victoria3", "Victoria 3", KeymapCategory::Simulator, Archetype::SimStrategy, &["victoria3.exe"]),
    spec("sim_aoe4", "Age of Empires IV", KeymapCategory::Simulator, Archetype::SimStrategy, &["RelicCardinal.exe"]),
    spec("sim_aoe2_de", "Age of Empires II: Definitive Edition", KeymapCategory::Simulator, Archetype::SimStrategy, &["AoE2DE_s.exe"]),
    spec("sim_aom_retold", "Age of Mythology: Retold", KeymapCategory::Simulator, Archetype::SimStrategy, &["AoMRT_s.exe"]),
    spec("sim_starcraft2", "StarCraft II", KeymapCategory::Simulator, Archetype::SimStrategy, &["SC2_x64.exe"]),
    spec("sim_warcraft3", "Warcraft III: Reforged", KeymapCategory::Simulator, Archetype::SimStrategy, &["Warcraft III.exe"]),
    spec("sim_total_war_wh3", "Total War: WARHAMMER III", KeymapCategory::Simulator, Archetype::SimStrategy, &["Warhammer3.exe"]),
    spec("sim_total_war_3k", "Total War: THREE KINGDOMS", KeymapCategory::Simulator, Archetype::SimStrategy, &["Three_Kingdoms.exe"]),
    spec("sim_coh3", "Company of Heroes 3", KeymapCategory::Simulator, Archetype::SimStrategy, &["RelicCoH3.exe"]),
    spec("sim_xcom2", "XCOM 2", KeymapCategory::Simulator, Archetype::SimStrategy, &["XCom2.exe"]),
    spec("sim_factorio", "Factorio", KeymapCategory::Simulator, Archetype::SimStrategy, &["factorio.exe", "factorio"]),
    spec("sim_satisfactory", "Satisfactory", KeymapCategory::Simulator, Archetype::ActionSurvival, &["FactoryGameSteam-Win64-Shipping.exe"]),
    spec("sim_dyson_sphere", "Dyson Sphere Program", KeymapCategory::Simulator, Archetype::SimStrategy, &["DSPGAME.exe"]),
    spec("sim_rimworld", "RimWorld", KeymapCategory::Simulator, Archetype::SimStrategy, &["RimWorldWin64.exe"]),
    spec("sim_dwarf_fortress", "Dwarf Fortress", KeymapCategory::Simulator, Archetype::SimStrategy, &["Dwarf Fortress.exe"]),
    spec("sim_oxygen_not_inc", "Oxygen Not Included", KeymapCategory::Simulator, Archetype::SimStrategy, &["OxygenNotIncluded.exe"]),
    spec("sim_anno1800", "Anno 1800", KeymapCategory::Simulator, Archetype::SimStrategy, &["Anno1800.exe"]),
    spec("sim_frostpunk", "Frostpunk", KeymapCategory::Simulator, Archetype::SimStrategy, &["Frostpunk.exe"]),
    spec("sim_frostpunk2", "Frostpunk 2", KeymapCategory::Simulator, Archetype::SimStrategy, &["Frostpunk2-Win64-Shipping.exe"]),
    spec("sim_manor_lords", "Manor Lords", KeymapCategory::Simulator, Archetype::SimStrategy, &["ManorLords-Win64-Shipping.exe"]),
    spec("sim_timberborn", "Timberborn", KeymapCategory::Simulator, Archetype::SimStrategy, &["Timberborn.exe"]),
    spec("sim_against_the_storm", "Against the Storm", KeymapCategory::Simulator, Archetype::SimStrategy, &["Against the Storm.exe"]),
    spec("sim_sims4", "The Sims 4", KeymapCategory::Simulator, Archetype::SimLife, &["TS4_x64.exe"]),
    spec("sim_stardew_valley", "Stardew Valley", KeymapCategory::Simulator, Archetype::ActionPlatformer, &["Stardew Valley.exe"]),
    spec("sim_coral_island", "Coral Island", KeymapCategory::Simulator, Archetype::SimLife, &["ProjectCoral-Win64-Shipping.exe"]),
    spec("sim_farming25", "Farming Simulator 25", KeymapCategory::Simulator, Archetype::SimLife, &["FarmingSimulator2025Game.exe"]),
    spec("sim_farming22", "Farming Simulator 22", KeymapCategory::Simulator, Archetype::SimLife, &["FarmingSimulator2022Game.exe"]),
    spec("sim_football_mgr24", "Football Manager 2024", KeymapCategory::Simulator, Archetype::SimLife, &["fm.exe"]),
    spec("sim_planet_zoo", "Planet Zoo", KeymapCategory::Simulator, Archetype::SimLife, &["PlanetZoo.exe"]),
    spec("sim_planet_coaster2", "Planet Coaster 2", KeymapCategory::Simulator, Archetype::SimLife, &["PlanetCoaster2.exe"]),
    spec("sim_jurassic_evo2", "Jurassic World Evolution 2", KeymapCategory::Simulator, Archetype::SimLife, &["JWE2.exe"]),
    spec("sim_two_point_campus", "Two Point Campus", KeymapCategory::Simulator, Archetype::SimLife, &["TPC.exe"]),
    spec("sim_powerwash_sim", "PowerWash Simulator", KeymapCategory::Simulator, Archetype::ActionSurvival, &["PowerWashSimulator.exe"]),
    spec("sim_house_flipper2", "House Flipper 2", KeymapCategory::Simulator, Archetype::ActionSurvival, &["HouseFlipper2.exe"]),
    spec("sim_supermarket_sim", "Supermarket Simulator", KeymapCategory::Simulator, Archetype::SimLife, &["Supermarket Simulator.exe"]),
    spec("sim_tcg_card_shop", "TCG Card Shop Simulator", KeymapCategory::Simulator, Archetype::SimLife, &["Card Shop Simulator.exe"]),
    spec("sim_balatro", "Balatro", KeymapCategory::Simulator, Archetype::SimLife, &["Balatro.exe"]),
    spec("sim_slay_the_spire", "Slay the Spire", KeymapCategory::Simulator, Archetype::SimLife, &["SlayTheSpire.exe"]),
    spec("sim_hearthstone", "Hearthstone", KeymapCategory::Simulator, Archetype::SimLife, &["Hearthstone.exe"]),
    spec("sim_mtg_arena", "MTG Arena", KeymapCategory::Simulator, Archetype::SimLife, &["MTGA.exe"]),
    spec("sim_master_duel", "Yu-Gi-Oh! Master Duel", KeymapCategory::Simulator, Archetype::SimLife, &["masterduel.exe"]),
    spec("sim_tabletop_sim", "Tabletop Simulator", KeymapCategory::Simulator, Archetype::SimLife, &["Tabletop Simulator.exe"]),

    // ==================== PRODUCTIVITY (95 presets: IDE, Design, Video, Office) ====================
    spec("prod_vscode", "Visual Studio Code", KeymapCategory::Productivity, Archetype::ProdIde, &["Code.exe", "code", "Electron"]),
    spec("prod_cursor", "Cursor IDE", KeymapCategory::Productivity, Archetype::ProdIde, &["Cursor.exe", "Cursor"]),
    spec("prod_windsurf", "Windsurf IDE", KeymapCategory::Productivity, Archetype::ProdIde, &["Windsurf.exe", "Windsurf"]),
    spec("prod_zed", "Zed Editor", KeymapCategory::Productivity, Archetype::ProdIde, &["zed", "Zed"]),
    spec("prod_intellij", "IntelliJ IDEA", KeymapCategory::Productivity, Archetype::ProdIde, &["idea64.exe", "idea"]),
    spec("prod_rustrover", "RustRover", KeymapCategory::Productivity, Archetype::ProdIde, &["rustrover64.exe", "rustrover"]),
    spec("prod_pycharm", "PyCharm", KeymapCategory::Productivity, Archetype::ProdIde, &["pycharm64.exe", "pycharm"]),
    spec("prod_webstorm", "WebStorm", KeymapCategory::Productivity, Archetype::ProdIde, &["webstorm64.exe", "webstorm"]),
    spec("prod_clion", "CLion", KeymapCategory::Productivity, Archetype::ProdIde, &["clion64.exe", "clion"]),
    spec("prod_goland", "GoLand", KeymapCategory::Productivity, Archetype::ProdIde, &["goland64.exe", "goland"]),
    spec("prod_rider", "JetBrains Rider", KeymapCategory::Productivity, Archetype::ProdIde, &["rider64.exe", "rider"]),
    spec("prod_phpstorm", "PhpStorm", KeymapCategory::Productivity, Archetype::ProdIde, &["phpstorm64.exe", "phpstorm"]),
    spec("prod_datagrip", "DataGrip", KeymapCategory::Productivity, Archetype::ProdIde, &["datagrip64.exe", "datagrip"]),
    spec("prod_android_studio", "Android Studio", KeymapCategory::Productivity, Archetype::ProdIde, &["studio64.exe", "studio"]),
    spec("prod_xcode", "Xcode", KeymapCategory::Productivity, Archetype::ProdIde, &["Xcode"]),
    spec("prod_visual_studio", "Visual Studio", KeymapCategory::Productivity, Archetype::ProdIde, &["devenv.exe"]),
    spec("prod_neovim", "Neovim", KeymapCategory::Productivity, Archetype::ProdIde, &["nvim.exe", "nvim"]),
    spec("prod_vim", "Vim", KeymapCategory::Productivity, Archetype::ProdIde, &["vim.exe", "vim"]),
    spec("prod_emacs", "GNU Emacs", KeymapCategory::Productivity, Archetype::ProdIde, &["emacs.exe", "emacs"]),
    spec("prod_sublime", "Sublime Text", KeymapCategory::Productivity, Archetype::ProdIde, &["sublime_text.exe", "subl"]),
    spec("prod_notepadpp", "Notepad++", KeymapCategory::Productivity, Archetype::ProdIde, &["notepad++.exe"]),
    spec("prod_helix", "Helix Editor", KeymapCategory::Productivity, Archetype::ProdIde, &["hx.exe", "hx"]),
    spec("prod_windows_terminal", "Windows Terminal", KeymapCategory::Productivity, Archetype::ProdIde, &["WindowsTerminal.exe"]),
    spec("prod_iterm2", "iTerm2", KeymapCategory::Productivity, Archetype::ProdIde, &["iTerm2"]),
    spec("prod_warp", "Warp Terminal", KeymapCategory::Productivity, Archetype::ProdIde, &["Warp", "warp-terminal"]),
    spec("prod_alacritty", "Alacritty", KeymapCategory::Productivity, Archetype::ProdIde, &["alacritty.exe", "alacritty"]),
    spec("prod_kitty", "Kitty Terminal", KeymapCategory::Productivity, Archetype::ProdIde, &["kitty"]),
    spec("prod_wezterm", "WezTerm", KeymapCategory::Productivity, Archetype::ProdIde, &["wezterm-gui.exe", "wezterm-gui"]),
    spec("prod_ghostty", "Ghostty Terminal", KeymapCategory::Productivity, Archetype::ProdIde, &["ghostty"]),
    spec("prod_github_desktop", "GitHub Desktop", KeymapCategory::Productivity, Archetype::ProdIde, &["GitHubDesktop.exe"]),
    spec("prod_gitkraken", "GitKraken", KeymapCategory::Productivity, Archetype::ProdIde, &["gitkraken.exe", "gitkraken"]),
    spec("prod_sourcetree", "Sourcetree", KeymapCategory::Productivity, Archetype::ProdIde, &["SourceTree.exe", "Sourcetree"]),
    spec("prod_postman", "Postman", KeymapCategory::Productivity, Archetype::ProdIde, &["Postman.exe", "Postman"]),
    spec("prod_insomnia", "Insomnia", KeymapCategory::Productivity, Archetype::ProdIde, &["Insomnia.exe", "Insomnia"]),
    spec("prod_docker_desktop", "Docker Desktop", KeymapCategory::Productivity, Archetype::ProdIde, &["Docker Desktop.exe", "Docker"]),
    spec("prod_dbeaver", "DBeaver", KeymapCategory::Productivity, Archetype::ProdIde, &["dbeaver.exe", "dbeaver"]),
    spec("prod_tableplus", "TablePlus", KeymapCategory::Productivity, Archetype::ProdIde, &["TablePlus.exe", "TablePlus"]),
    spec("prod_wireshark", "Wireshark", KeymapCategory::Productivity, Archetype::ProdIde, &["Wireshark.exe", "wireshark"]),
    spec("prod_photoshop", "Adobe Photoshop", KeymapCategory::Productivity, Archetype::ProdDesign, &["Photoshop.exe", "Adobe Photoshop"]),
    spec("prod_illustrator", "Adobe Illustrator", KeymapCategory::Productivity, Archetype::ProdDesign, &["Illustrator.exe", "Adobe Illustrator"]),
    spec("prod_indesign", "Adobe InDesign", KeymapCategory::Productivity, Archetype::ProdDesign, &["InDesign.exe"]),
    spec("prod_lightroom", "Adobe Lightroom Classic", KeymapCategory::Productivity, Archetype::ProdDesign, &["Lightroom.exe"]),
    spec("prod_xd", "Adobe XD", KeymapCategory::Productivity, Archetype::ProdDesign, &["XD.exe"]),
    spec("prod_figma", "Figma", KeymapCategory::Productivity, Archetype::ProdDesign, &["Figma.exe", "Figma"]),
    spec("prod_sketch", "Sketch", KeymapCategory::Productivity, Archetype::ProdDesign, &["Sketch"]),
    spec("prod_blender", "Blender", KeymapCategory::Productivity, Archetype::ProdDesign, &["blender.exe", "Blender"]),
    spec("prod_maya", "Autodesk Maya", KeymapCategory::Productivity, Archetype::ProdDesign, &["maya.exe", "Maya"]),
    spec("prod_3dsmax", "Autodesk 3ds Max", KeymapCategory::Productivity, Archetype::ProdDesign, &["3dsmax.exe"]),
    spec("prod_cinema4d", "Cinema 4D", KeymapCategory::Productivity, Archetype::ProdDesign, &["Cinema 4D.exe"]),
    spec("prod_houdini", "SideFX Houdini", KeymapCategory::Productivity, Archetype::ProdDesign, &["houdini.exe", "houdinifx.exe"]),
    spec("prod_zbrush", "Maxon ZBrush", KeymapCategory::Productivity, Archetype::ProdDesign, &["ZBrush.exe"]),
    spec("prod_substance_painter", "Substance 3D Painter", KeymapCategory::Productivity, Archetype::ProdDesign, &["Adobe Substance 3D Painter.exe"]),
    spec("prod_unity", "Unity Editor", KeymapCategory::Productivity, Archetype::ProdDesign, &["Unity.exe", "Unity"]),
    spec("prod_unreal", "Unreal Editor", KeymapCategory::Productivity, Archetype::ProdDesign, &["UnrealEditor.exe", "UnrealEditor"]),
    spec("prod_godot", "Godot Engine", KeymapCategory::Productivity, Archetype::ProdDesign, &["Godot_v4.exe", "godot"]),
    spec("prod_autocad", "AutoCAD", KeymapCategory::Productivity, Archetype::ProdDesign, &["acad.exe"]),
    spec("prod_solidworks", "SOLIDWORKS", KeymapCategory::Productivity, Archetype::ProdDesign, &["SLDWORKS.exe"]),
    spec("prod_fusion360", "Autodesk Fusion", KeymapCategory::Productivity, Archetype::ProdDesign, &["Fusion360.exe"]),
    spec("prod_rhino", "Rhinoceros 3D", KeymapCategory::Productivity, Archetype::ProdDesign, &["Rhino.exe"]),
    spec("prod_sketchup", "SketchUp", KeymapCategory::Productivity, Archetype::ProdDesign, &["SketchUp.exe"]),
    spec("prod_affinity_photo", "Affinity Photo 2", KeymapCategory::Productivity, Archetype::ProdDesign, &["Photo.exe", "Affinity Photo 2"]),
    spec("prod_affinity_designer", "Affinity Designer 2", KeymapCategory::Productivity, Archetype::ProdDesign, &["Designer.exe", "Affinity Designer 2"]),
    spec("prod_gimp", "GIMP", KeymapCategory::Productivity, Archetype::ProdDesign, &["gimp-2.10.exe", "gimp"]),
    spec("prod_inkscape", "Inkscape", KeymapCategory::Productivity, Archetype::ProdDesign, &["inkscape.exe", "inkscape"]),
    spec("prod_krita", "Krita", KeymapCategory::Productivity, Archetype::ProdDesign, &["krita.exe", "krita"]),
    spec("prod_clip_studio", "Clip Studio Paint", KeymapCategory::Productivity, Archetype::ProdDesign, &["CLIPStudioPaint.exe"]),
    spec("prod_aseprite", "Aseprite", KeymapCategory::Productivity, Archetype::ProdDesign, &["Aseprite.exe", "aseprite"]),
    spec("prod_premiere_pro", "Adobe Premiere Pro", KeymapCategory::Productivity, Archetype::ProdVideo, &["Adobe Premiere Pro.exe", "Adobe Premiere Pro"]),
    spec("prod_after_effects", "Adobe After Effects", KeymapCategory::Productivity, Archetype::ProdVideo, &["AfterFX.exe", "After Effects"]),
    spec("prod_davinci_resolve", "DaVinci Resolve", KeymapCategory::Productivity, Archetype::ProdVideo, &["Resolve.exe", "Resolve"]),
    spec("prod_final_cut", "Final Cut Pro", KeymapCategory::Productivity, Archetype::ProdVideo, &["Final Cut Pro"]),
    spec("prod_capcut", "CapCut Desktop", KeymapCategory::Productivity, Archetype::ProdVideo, &["CapCut.exe", "CapCut"]),
    spec("prod_jianying", "Jianying Pro", KeymapCategory::Productivity, Archetype::ProdVideo, &["JianyingPro.exe"]),
    spec("prod_vegas_pro", "VEGAS Pro", KeymapCategory::Productivity, Archetype::ProdVideo, &["vegas210.exe", "vegas"]),
    spec("prod_kdenlive", "Kdenlive", KeymapCategory::Productivity, Archetype::ProdVideo, &["kdenlive.exe", "kdenlive"]),
    spec("prod_shotcut", "Shotcut", KeymapCategory::Productivity, Archetype::ProdVideo, &["shotcut.exe", "shotcut"]),
    spec("prod_ms_word", "Microsoft Word", KeymapCategory::Productivity, Archetype::ProdOffice, &["WINWORD.EXE", "Microsoft Word"]),
    spec("prod_ms_excel", "Microsoft Excel", KeymapCategory::Productivity, Archetype::ProdOffice, &["EXCEL.EXE", "Microsoft Excel"]),
    spec("prod_ms_powerpoint", "Microsoft PowerPoint", KeymapCategory::Productivity, Archetype::ProdOffice, &["POWERPNT.EXE", "Microsoft PowerPoint"]),
    spec("prod_ms_outlook", "Microsoft Outlook", KeymapCategory::Productivity, Archetype::ProdOffice, &["OUTLOOK.EXE", "olk.exe"]),
    spec("prod_ms_onenote", "Microsoft OneNote", KeymapCategory::Productivity, Archetype::ProdOffice, &["ONENOTE.EXE"]),
    spec("prod_libreoffice", "LibreOffice", KeymapCategory::Productivity, Archetype::ProdOffice, &["soffice.bin", "soffice.exe"]),
    spec("prod_wps_office", "WPS Office", KeymapCategory::Productivity, Archetype::ProdOffice, &["wps.exe", "et.exe", "wpp.exe"]),
    spec("prod_notion", "Notion", KeymapCategory::Productivity, Archetype::ProdOffice, &["Notion.exe", "Notion"]),
    spec("prod_obsidian", "Obsidian", KeymapCategory::Productivity, Archetype::ProdOffice, &["Obsidian.exe", "Obsidian"]),
    spec("prod_logseq", "Logseq", KeymapCategory::Productivity, Archetype::ProdOffice, &["Logseq.exe", "Logseq"]),
    spec("prod_slack", "Slack", KeymapCategory::Productivity, Archetype::ProdOffice, &["slack.exe", "Slack"]),
    spec("prod_discord", "Discord", KeymapCategory::Productivity, Archetype::ProdOffice, &["Discord.exe", "Discord"]),
    spec("prod_teams", "Microsoft Teams", KeymapCategory::Productivity, Archetype::ProdOffice, &["ms-teams.exe", "Teams.exe"]),
    spec("prod_zoom", "Zoom Workplace", KeymapCategory::Productivity, Archetype::ProdOffice, &["Zoom.exe", "zoom.us"]),
    spec("prod_chrome", "Google Chrome", KeymapCategory::Productivity, Archetype::ProdOffice, &["chrome.exe", "Google Chrome"]),
    spec("prod_firefox", "Mozilla Firefox", KeymapCategory::Productivity, Archetype::ProdOffice, &["firefox.exe", "firefox"]),
    spec("prod_edge", "Microsoft Edge", KeymapCategory::Productivity, Archetype::ProdOffice, &["msedge.exe", "Microsoft Edge"]),
    spec("prod_arc", "Arc Browser", KeymapCategory::Productivity, Archetype::ProdOffice, &["Arc.exe", "Arc"]),
    spec("prod_brave", "Brave Browser", KeymapCategory::Productivity, Archetype::ProdOffice, &["brave.exe", "Brave Browser"]),

    // ==================== MEDIA (45 presets) ====================
    spec("media_vlc", "VLC Media Player", KeymapCategory::Media, Archetype::MediaPlayer, &["vlc.exe", "vlc", "VLC"]),
    spec("media_mpv", "mpv Player", KeymapCategory::Media, Archetype::MediaPlayer, &["mpv.exe", "mpv"]),
    spec("media_iina", "IINA", KeymapCategory::Media, Archetype::MediaPlayer, &["IINA"]),
    spec("media_potplayer", "PotPlayer", KeymapCategory::Media, Archetype::MediaPlayer, &["PotPlayerMini64.exe", "PotPlayer64.exe"]),
    spec("media_mpc_hc", "MPC-HC", KeymapCategory::Media, Archetype::MediaPlayer, &["mpc-hc64.exe"]),
    spec("media_mpc_be", "MPC-BE", KeymapCategory::Media, Archetype::MediaPlayer, &["mpc-be64.exe"]),
    spec("media_plex", "Plex Desktop", KeymapCategory::Media, Archetype::MediaPlayer, &["Plex.exe", "Plex"]),
    spec("media_jellyfin", "Jellyfin Media Player", KeymapCategory::Media, Archetype::MediaPlayer, &["JellyfinMediaPlayer.exe", "jellyfinmediaplayer"]),
    spec("media_kodi", "Kodi", KeymapCategory::Media, Archetype::MediaPlayer, &["kodi.exe", "kodi"]),
    spec("media_infuse", "Infuse", KeymapCategory::Media, Archetype::MediaPlayer, &["Infuse"]),
    spec("media_emby", "Emby Theater", KeymapCategory::Media, Archetype::MediaPlayer, &["Emby.Theater.exe"]),
    spec("media_spotify", "Spotify", KeymapCategory::Media, Archetype::MediaPlayer, &["Spotify.exe", "Spotify"]),
    spec("media_apple_music", "Apple Music", KeymapCategory::Media, Archetype::MediaPlayer, &["AppleMusic.exe", "Music"]),
    spec("media_tidal", "TIDAL", KeymapCategory::Media, Archetype::MediaPlayer, &["TIDAL.exe", "TIDAL"]),
    spec("media_foobar2000", "foobar2000", KeymapCategory::Media, Archetype::MediaPlayer, &["foobar2000.exe"]),
    spec("media_musicbee", "MusicBee", KeymapCategory::Media, Archetype::MediaPlayer, &["MusicBee.exe"]),
    spec("media_netease_music", "NetEase Cloud Music", KeymapCategory::Media, Archetype::MediaPlayer, &["cloudmusic.exe"]),
    spec("media_qq_music", "QQ Music", KeymapCategory::Media, Archetype::MediaPlayer, &["QQMusic.exe"]),
    spec("media_youtube_tv", "YouTube Leanback", KeymapCategory::Media, Archetype::MediaPlayer, &["youtube", "youtubetv"]),
    spec("media_netflix", "Netflix", KeymapCategory::Media, Archetype::MediaPlayer, &["netflix"]),
    spec("media_bilibili", "Bilibili Desktop", KeymapCategory::Media, Archetype::MediaPlayer, &["bilibili.exe", "bilibili"]),
    spec("media_twitch", "Twitch Desktop", KeymapCategory::Media, Archetype::MediaPlayer, &["Twitch.exe", "twitch"]),
    spec("media_obs", "OBS Studio", KeymapCategory::Media, Archetype::ProdVideo, &["obs64.exe", "obs"]),
    spec("media_streamlabs", "Streamlabs Desktop", KeymapCategory::Media, Archetype::ProdVideo, &["Streamlabs OBS.exe"]),
    spec("media_handbrake", "HandBrake", KeymapCategory::Media, Archetype::MediaPlayer, &["HandBrake.exe", "HandBrake"]),
    spec("media_audacity", "Audacity", KeymapCategory::Media, Archetype::MediaDaw, &["Audacity.exe", "Audacity"]),
    spec("media_ableton", "Ableton Live", KeymapCategory::Media, Archetype::MediaDaw, &["Ableton Live 12 Suite.exe", "Live"]),
    spec("media_fl_studio", "FL Studio", KeymapCategory::Media, Archetype::MediaDaw, &["FL64.exe", "OsxFL"]),
    spec("media_logic_pro", "Logic Pro", KeymapCategory::Media, Archetype::MediaDaw, &["Logic Pro"]),
    spec("media_reaper", "REAPER", KeymapCategory::Media, Archetype::MediaDaw, &["reaper.exe", "REAPER"]),
    spec("media_cubase", "Steinberg Cubase", KeymapCategory::Media, Archetype::MediaDaw, &["Cubase13.exe", "Cubase"]),
    spec("media_studio_one", "PreSonus Studio One", KeymapCategory::Media, Archetype::MediaDaw, &["Studio One.exe"]),
    spec("media_pro_tools", "Avid Pro Tools", KeymapCategory::Media, Archetype::MediaDaw, &["ProTools.exe", "Pro Tools"]),
    spec("media_bitwig", "Bitwig Studio", KeymapCategory::Media, Archetype::MediaDaw, &["Bitwig Studio.exe", "bitwig-studio"]),
    spec("media_garageband", "GarageBand", KeymapCategory::Media, Archetype::MediaDaw, &["GarageBand"]),
    spec("media_rekordbox", "rekordbox", KeymapCategory::Media, Archetype::MediaDaw, &["rekordbox.exe", "rekordbox"]),
    spec("media_serato", "Serato DJ Pro", KeymapCategory::Media, Archetype::MediaDaw, &["Serato DJ Pro.exe"]),
    spec("media_traktor", "Traktor Pro", KeymapCategory::Media, Archetype::MediaDaw, &["Traktor.exe"]),
    spec("media_virtualdj", "VirtualDJ", KeymapCategory::Media, Archetype::MediaDaw, &["virtualdj.exe"]),
    spec("media_steam_big_picture", "Steam Big Picture", KeymapCategory::Media, Archetype::MediaPlayer, &["steam.exe", "steamwebhelper"]),
    spec("media_playnite", "Playnite Fullscreen", KeymapCategory::Media, Archetype::MediaPlayer, &["Playnite.FullscreenApp.exe"]),
    spec("media_retroarch", "RetroArch", KeymapCategory::Media, Archetype::ActionPlatformer, &["retroarch.exe", "retroarch"]),
    spec("media_dolphin_emu", "Dolphin Emulator", KeymapCategory::Media, Archetype::ActionPlatformer, &["Dolphin.exe", "dolphin-emu"]),
    spec("media_pcsx2", "PCSX2", KeymapCategory::Media, Archetype::ActionBrawler, &["pcsx2-qt.exe", "pcsx2"]),
    spec("media_rpcs3", "RPCS3", KeymapCategory::Media, Archetype::ActionBrawler, &["rpcs3.exe", "rpcs3"]),

    // ==================== EXTRA POPULAR TITLES & APPS (to exceed 520+) ====================
    spec("fps_stalker_soc", "S.T.A.L.K.E.R.: Shadow of Chernobyl", KeymapCategory::Fps, Archetype::FpsTactical, &["XR_3DA.exe"]),
    spec("fps_bioshock_remastered", "BioShock Remastered", KeymapCategory::Fps, Archetype::FpsHero, &["BioshockHD.exe"]),
    spec("fps_metro2033", "Metro 2033 Redux", KeymapCategory::Fps, Archetype::FpsTactical, &["metro.exe"]),
    spec("fps_titanfall", "Titanfall", KeymapCategory::Fps, Archetype::FpsHero, &["Titanfall.exe"]),
    spec("fps_deus_ex_md", "Deus Ex: Mankind Divided", KeymapCategory::Fps, Archetype::FpsTactical, &["DXMD.exe"]),
    spec("fps_superhot", "SUPERHOT", KeymapCategory::Fps, Archetype::FpsHero, &["SH.exe"]),
    spec("fps_dusk", "DUSK", KeymapCategory::Fps, Archetype::FpsHero, &["Dusk.exe"]),
    spec("fps_prodeus", "Prodeus", KeymapCategory::Fps, Archetype::FpsHero, &["Prodeus.exe"]),
    spec("fps_boltgun", "Warhammer 40,000: Boltgun", KeymapCategory::Fps, Archetype::FpsHero, &["Boltgun-Win64-Shipping.exe"]),
    spec("fps_trepang2", "Trepang2", KeymapCategory::Fps, Archetype::FpsHero, &["CPP_FPS-Win64-Shipping.exe"]),
    spec("fps_system_shock", "System Shock Remake", KeymapCategory::Fps, Archetype::FpsTactical, &["SystemReShock-Win64-Shipping.exe"]),
    spec("fps_robocop", "RoboCop: Rogue City", KeymapCategory::Fps, Archetype::FpsTactical, &["RoboCop-Win64-Shipping.exe"]),
    spec("fps_sniper_elite5", "Sniper Elite 5", KeymapCategory::Fps, Archetype::FpsTactical, &["Sniper5_dx12.exe"]),
    spec("fps_ghost_recon_bp", "Ghost Recon Breakpoint", KeymapCategory::Fps, Archetype::FpsTactical, &["GRB.exe"]),
    spec("fps_division2", "Tom Clancy's The Division 2", KeymapCategory::Fps, Archetype::FpsBattleRoyale, &["TheDivision2.exe"]),
    spec("rpg_morrowind", "The Elder Scrolls III: Morrowind", KeymapCategory::Rpg, Archetype::RpgAction, &["Morrowind.exe", "openmw.exe"]),
    spec("rpg_fallout3", "Fallout 3", KeymapCategory::Rpg, Archetype::RpgAction, &["Fallout3.exe"]),
    spec("rpg_dragon_age_inq", "Dragon Age: Inquisition", KeymapCategory::Rpg, Archetype::RpgAction, &["DragonAgeInquisition.exe"]),
    spec("rpg_kotor", "Star Wars: KOTOR", KeymapCategory::Rpg, Archetype::RpgAction, &["swkotor.exe"]),
    spec("rpg_persona4_golden", "Persona 4 Golden", KeymapCategory::Rpg, Archetype::RpgAction, &["P4G.exe"]),
    spec("rpg_smt5_vengeance", "Shin Megami Tensei V: Vengeance", KeymapCategory::Rpg, Archetype::RpgAction, &["SMT5V-Win64-Shipping.exe"]),
    spec("rpg_chrono_trigger", "Chrono Trigger", KeymapCategory::Rpg, Archetype::ActionPlatformer, &["Chrono Trigger.exe"]),
    spec("rpg_ff10", "Final Fantasy X/X-2 HD", KeymapCategory::Rpg, Archetype::RpgAction, &["FFX.exe"]),
    spec("rpg_ff15", "Final Fantasy XV", KeymapCategory::Rpg, Archetype::RpgAction, &["ffxv_s.exe"]),
    spec("rpg_kingdom_hearts3", "Kingdom Hearts III", KeymapCategory::Rpg, Archetype::RpgAction, &["KINGDOM HEARTS III.exe"]),
    spec("rpg_code_vein", "CODE VEIN", KeymapCategory::Rpg, Archetype::RpgAction, &["CodeVein-Win64-Shipping.exe"]),
    spec("rpg_mortal_shell", "Mortal Shell", KeymapCategory::Rpg, Archetype::RpgAction, &["Dungeonhaven-Win64-Shipping.exe"]),
    spec("rpg_surge2", "The Surge 2", KeymapCategory::Rpg, Archetype::RpgAction, &["TheSurge2.exe"]),
    spec("rpg_another_crabs_treasure", "Another Crab's Treasure", KeymapCategory::Rpg, Archetype::RpgAction, &["AnotherCrabsTreasure.exe"]),
    spec("rpg_elex2", "ELEX II", KeymapCategory::Rpg, Archetype::RpgAction, &["ELEX2.exe"]),
    spec("act_yakuza0", "Yakuza 0", KeymapCategory::Action, Archetype::ActionBrawler, &["Yakuza0.exe"]),
    spec("act_judgment", "Judgment", KeymapCategory::Action, Archetype::ActionBrawler, &["Judgment.exe"]),
    spec("act_lost_judgment", "Lost Judgment", KeymapCategory::Action, Archetype::ActionBrawler, &["LostJudgment.exe"]),
    spec("act_shadow_of_war", "Middle-earth: Shadow of War", KeymapCategory::Action, Archetype::ActionBrawler, &["ShadowOfWar.exe"]),
    spec("act_mad_max", "Mad Max", KeymapCategory::Action, Archetype::ActionBrawler, &["MadMax.exe"]),
    spec("act_days_gone", "Days Gone", KeymapCategory::Action, Archetype::ActionBrawler, &["DaysGone.exe"]),
    spec("act_plague_tale_req", "A Plague Tale: Requiem", KeymapCategory::Action, Archetype::ActionBrawler, &["APlagueTaleRequiem_x64.exe"]),
    spec("act_hellblade2", "Senua's Saga: Hellblade II", KeymapCategory::Action, Archetype::ActionBrawler, &["Hellblade2-Win64-Shipping.exe"]),
    spec("act_stellar_blade", "Stellar Blade", KeymapCategory::Action, Archetype::ActionBrawler, &["SB-Win64-Shipping.exe"]),
    spec("act_kunitsu_gami", "Kunitsu-Gami: Path of the Goddess", KeymapCategory::Action, Archetype::ActionBrawler, &["KunitsuGami.exe"]),
    spec("act_animal_well", "ANIMAL WELL", KeymapCategory::Action, Archetype::ActionPlatformer, &["Animal Well.exe"]),
    spec("act_tunic", "TUNIC", KeymapCategory::Action, Archetype::ActionPlatformer, &["Tunic.exe"]),
    spec("act_deaths_door", "Death's Door", KeymapCategory::Action, Archetype::ActionPlatformer, &["DeathsDoor.exe"]),
    spec("act_outer_wilds", "Outer Wilds", KeymapCategory::Action, Archetype::ActionSurvival, &["OuterWilds.exe"]),
    spec("act_stray", "Stray", KeymapCategory::Action, Archetype::ActionBrawler, &["Stray-Win64-Shipping.exe"]),
    spec("act_content_warning", "Content Warning", KeymapCategory::Action, Archetype::ActionSurvival, &["Content Warning.exe"]),
    spec("act_buckshot_roulette", "Buckshot Roulette", KeymapCategory::Action, Archetype::SimLife, &["Buckshot Roulette.exe"]),
    spec("act_chained_together", "Chained Together", KeymapCategory::Action, Archetype::ActionPlatformer, &["ChainedTogether-Win64-Shipping.exe"]),
    spec("act_overcooked2", "Overcooked! 2", KeymapCategory::Action, Archetype::ActionPlatformer, &["Overcooked2.exe"]),
    spec("act_gang_beasts", "Gang Beasts", KeymapCategory::Action, Archetype::ActionPlatformer, &["Gang Beasts.exe"]),
    spec("sim_barotrauma", "Barotrauma", KeymapCategory::Simulator, Archetype::SimStrategy, &["Barotrauma.exe"]),
    spec("sim_ftl", "FTL: Faster Than Light", KeymapCategory::Simulator, Archetype::SimStrategy, &["FTLGame.exe"]),
    spec("sim_into_the_breach", "Into the Breach", KeymapCategory::Simulator, Archetype::SimStrategy, &["Breach.exe"]),
    spec("sim_humankind", "HUMANKIND", KeymapCategory::Simulator, Archetype::SimStrategy, &["Humankind.exe"]),
    spec("sim_endless_space2", "Endless Space 2", KeymapCategory::Simulator, Archetype::SimStrategy, &["EndlessSpace2.exe"]),
    spec("sim_northgard", "Northgard", KeymapCategory::Simulator, Archetype::SimStrategy, &["Northgard.exe"]),
    spec("sim_dune_spice_wars", "Dune: Spice Wars", KeymapCategory::Simulator, Archetype::SimStrategy, &["D4X.exe"]),
    spec("sim_homeworld3", "Homeworld 3", KeymapCategory::Simulator, Archetype::SimStrategy, &["Homeworld3-Win64-Shipping.exe"]),
    spec("sim_sins_solar2", "Sins of a Solar Empire II", KeymapCategory::Simulator, Archetype::SimStrategy, &["sins2.exe"]),
    spec("sim_command_conquer", "C&C Remastered Collection", KeymapCategory::Simulator, Archetype::SimStrategy, &["ClientG.exe"]),
    spec("prod_safari", "Safari", KeymapCategory::Productivity, Archetype::ProdOffice, &["Safari"]),
    spec("prod_opera", "Opera GX", KeymapCategory::Productivity, Archetype::ProdOffice, &["opera.exe", "Opera GX"]),
    spec("prod_vivaldi", "Vivaldi Browser", KeymapCategory::Productivity, Archetype::ProdOffice, &["vivaldi.exe", "Vivaldi"]),
    spec("prod_thunderbird", "Mozilla Thunderbird", KeymapCategory::Productivity, Archetype::ProdOffice, &["thunderbird.exe", "thunderbird"]),
    spec("prod_telegram", "Telegram Desktop", KeymapCategory::Productivity, Archetype::ProdOffice, &["Telegram.exe", "Telegram"]),
    spec("prod_wechat", "WeChat", KeymapCategory::Productivity, Archetype::ProdOffice, &["WeChat.exe", "WeChat"]),
    spec("prod_feishu", "Feishu / Lark", KeymapCategory::Productivity, Archetype::ProdOffice, &["Feishu.exe", "Lark"]),
    spec("prod_acrobat", "Adobe Acrobat", KeymapCategory::Productivity, Archetype::ProdOffice, &["Acrobat.exe", "AcroRd32.exe"]),
    spec("prod_typora", "Typora", KeymapCategory::Productivity, Archetype::ProdOffice, &["Typora.exe", "Typora"]),
    spec("prod_miro", "Miro", KeymapCategory::Productivity, Archetype::ProdDesign, &["Miro.exe", "Miro"]),
    spec("prod_excalidraw", "Excalidraw", KeymapCategory::Productivity, Archetype::ProdDesign, &["excalidraw"]),
    spec("prod_capture_one", "Capture One", KeymapCategory::Productivity, Archetype::ProdDesign, &["CaptureOne.exe", "Capture One"]),
    spec("prod_darktable", "darktable", KeymapCategory::Productivity, Archetype::ProdDesign, &["darktable.exe", "darktable"]),
    spec("prod_kicad", "KiCad", KeymapCategory::Productivity, Archetype::ProdDesign, &["kicad.exe", "kicad"]),
    spec("prod_freecad", "FreeCAD", KeymapCategory::Productivity, Archetype::ProdDesign, &["FreeCAD.exe", "FreeCAD"]),
    spec("prod_matlab", "MATLAB", KeymapCategory::Productivity, Archetype::ProdIde, &["MATLAB.exe", "MATLAB"]),
    spec("prod_rstudio", "RStudio", KeymapCategory::Productivity, Archetype::ProdIde, &["rstudio.exe", "RStudio"]),
    spec("prod_jupyter", "JupyterLab Desktop", KeymapCategory::Productivity, Archetype::ProdIde, &["JupyterLab.exe", "JupyterLab"]),
    spec("custom_universal_desktop", "Universal Desktop & Gaming", KeymapCategory::Custom, Archetype::CustomUniversal, &["universal", "desktop"]),
    spec("custom_minimal_touchpad", "Minimal Touchpad & Shortcuts", KeymapCategory::Custom, Archetype::ProdOffice, &["minimal", "touchpad"]),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn builtin_catalog_has_at_least_500_valid_unique_presets() {
        let catalog = KeymapCatalog::builtin();
        assert!(
            catalog.len() >= 500,
            "expected at least 500 built-in presets, got {}",
            catalog.len()
        );

        let mut ids = HashSet::with_capacity(catalog.len());
        for profile in catalog.profiles() {
            assert!(
                ids.insert(profile.id.as_str()),
                "duplicate built-in preset id: {}",
                profile.id
            );
            profile
                .validate()
                .unwrap_or_else(|err| panic!("preset '{}' failed validation: {err}", profile.id));
            assert!(
                !profile.controls.is_empty(),
                "preset '{}' should have at least one control",
                profile.id
            );
            assert!(
                !profile.process_hints.is_empty(),
                "preset '{}' should have at least one process hint",
                profile.id
            );
        }

        // Ensure every core category is well represented.
        for cat in [
            KeymapCategory::Fps,
            KeymapCategory::Moba,
            KeymapCategory::Rpg,
            KeymapCategory::Action,
            KeymapCategory::Racing,
            KeymapCategory::Simulator,
            KeymapCategory::Productivity,
            KeymapCategory::Media,
        ] {
            let count = catalog.by_category(cat).len();
            assert!(
                count >= 40,
                "category {cat} should have >= 40 presets, got {count}"
            );
        }
    }

    #[test]
    fn match_process_handles_paths_extensions_and_case() {
        let catalog = KeymapCatalog::builtin();

        let val = catalog.match_process(r"C:\Riot Games\VALORANT\live\VALORANT-Win64-Shipping.exe");
        assert_eq!(val.first().map(|p| p.id.as_str()), Some("fps_valorant"));

        let cs2 = catalog.match_process("CS2.EXE");
        assert_eq!(cs2.first().map(|p| p.id.as_str()), Some("fps_cs2"));

        let blender = catalog.match_process("/Applications/Blender.app/Contents/MacOS/Blender");
        assert_eq!(blender.first().map(|p| p.id.as_str()), Some("prod_blender"));

        let vlc = catalog.match_process("/usr/bin/vlc");
        assert_eq!(vlc.first().map(|p| p.id.as_str()), Some("media_vlc"));

        let lol = catalog.match_process("League of Legends.exe");
        assert_eq!(lol.first().map(|p| p.id.as_str()), Some("moba_lol"));

        assert!(catalog.match_process("   ").is_empty());
    }

    #[test]
    fn search_supports_name_id_category_and_process_hints() {
        let catalog = KeymapCatalog::builtin();

        let by_name = catalog.search("Elden Ring");
        assert_eq!(by_name.first().map(|p| p.id.as_str()), Some("rpg_elden_ring"));

        let by_hint = catalog.search("r5apex");
        assert_eq!(by_hint.first().map(|p| p.id.as_str()), Some("fps_apex_legends"));

        let by_multi_token = catalog.search("productivity resolve");
        assert!(by_multi_token.iter().any(|p| p.id == "prod_davinci_resolve"));

        let all = catalog.search("   ");
        assert_eq!(all.len(), catalog.len());
    }

    #[test]
    fn json_roundtrip_and_strict_validation() {
        let catalog = KeymapCatalog::builtin();
        let original = catalog.get("fps_valorant").expect("valorant preset exists");

        let json = original.to_json().expect("serialize valid profile");
        let parsed = KeymapProfile::from_json(&json).expect("deserialize valid profile");
        assert_eq!(*original, parsed);

        let pretty = original.to_json_pretty().expect("pretty serialize");
        let parsed_pretty = KeymapProfile::from_json(&pretty).expect("deserialize pretty");
        assert_eq!(*original, parsed_pretty);

        // Schema version check
        let mut bad_ver = original.clone();
        bad_ver.schema_version = 2;
        assert_eq!(
            bad_ver.validate(),
            Err(KeymapValidationError::UnsupportedSchemaVersion(2))
        );

        // Empty profile id / name
        let mut empty_id = original.clone();
        empty_id.id = "  ".into();
        assert_eq!(empty_id.validate(), Err(KeymapValidationError::EmptyProfileId));

        let mut empty_name = original.clone();
        empty_name.name = "".into();
        assert_eq!(
            empty_name.validate(),
            Err(KeymapValidationError::EmptyProfileName)
        );

        // Out-of-range coordinate
        let mut bad_coord = original.clone();
        bad_coord.controls[0].norm_x = 1.25;
        assert!(matches!(
            bad_coord.validate(),
            Err(KeymapValidationError::InvalidCoordinate { axis: "norm_x", .. })
        ));

        // Out-of-range radius
        let mut bad_radius = original.clone();
        bad_radius.controls[0].radius_norm = 0.01;
        assert!(matches!(
            bad_radius.validate(),
            Err(KeymapValidationError::InvalidRadius { .. })
        ));

        // Out-of-range opacity
        let mut bad_opacity = original.clone();
        bad_opacity.controls[0].opacity = 0.05;
        assert!(matches!(
            bad_opacity.validate(),
            Err(KeymapValidationError::InvalidOpacity { .. })
        ));

        // Duplicate control ID
        let mut dup_ctrl = original.clone();
        dup_ctrl.controls[1].id = dup_ctrl.controls[0].id.clone();
        assert!(matches!(
            dup_ctrl.validate(),
            Err(KeymapValidationError::DuplicateControlId(_))
        ));

        // Missing keys on KeyButton / ComboButton / MacroBurst
        let mut missing_keys = original.clone();
        let key_btn = missing_keys
            .controls
            .iter_mut()
            .find(|c| c.kind == VirtualControlKind::KeyButton)
            .expect("has key button");
        key_btn.keys.clear();
        assert!(matches!(
            missing_keys.validate(),
            Err(KeymapValidationError::MissingKeys {
                kind: VirtualControlKind::KeyButton,
                ..
            })
        ));

        // Invalid JSON import with schema violation must fail
        let invalid_json = json.replace("\"schema_version\":1", "\"schema_version\":99");
        assert!(matches!(
            KeymapProfile::from_json(&invalid_json),
            Err(KeymapError::Validation(
                KeymapValidationError::UnsupportedSchemaVersion(99)
            ))
        ));
    }
}
