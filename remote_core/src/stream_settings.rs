//! Stream form state and effects; execution and persistence stay in the adapter.
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamSettingsValues {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
}
impl StreamSettingsValues {
    pub fn resolution(self) -> (u32, u32) {
        (self.width, self.height)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingsReceipt {
    command: u64,
    view: u64,
}

pub enum StreamSettingsAction {
    EditFps(String),
    EditBitrate(String),
    ApplyCustom,
    SelectResolution(u32, u32),
    SelectFps(u32),
    SelectBitrate(u32),
    UpdateFailed(SettingsReceipt, String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdatePolicy {
    Always,
    WhenConnected,
}

/// A commit requests the existing preferences write and optional Owner update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamSettingsEffect {
    pub values: StreamSettingsValues,
    pub update: UpdatePolicy,
    pub report_error: bool,
    pub receipt: SettingsReceipt,
}
impl StreamSettingsEffect {
    pub fn should_update(self, connected: bool) -> bool {
        self.update == UpdatePolicy::Always || connected
    }
}

pub struct StreamSettingsViewModel<'a> {
    pub values: StreamSettingsValues,
    pub fps_text: &'a str,
    pub bitrate_text: &'a str,
    pub error: Option<&'a str>,
}

pub struct StreamSettingsState {
    values: StreamSettingsValues,
    fps_text: String,
    bitrate_text: String,
    error: Option<String>,
    command: u64,
    view: u64,
}
impl StreamSettingsState {
    pub fn new(values: StreamSettingsValues) -> Self {
        Self {
            values,
            fps_text: values.fps.to_string(),
            bitrate_text: values.bitrate_kbps.to_string(),
            error: None,
            command: 0,
            view: 0,
        }
    }
    pub fn values(&self) -> StreamSettingsValues {
        self.values
    }
    pub fn project(&self) -> StreamSettingsViewModel<'_> {
        StreamSettingsViewModel {
            values: self.values,
            fps_text: &self.fps_text,
            bitrate_text: &self.bitrate_text,
            error: self.error.as_deref(),
        }
    }
    pub fn effect_is_current(&self, receipt: SettingsReceipt) -> bool {
        self.command == receipt.command
    }
    pub fn reduce(&mut self, action: StreamSettingsAction) -> Option<StreamSettingsEffect> {
        if let StreamSettingsAction::UpdateFailed(receipt, error) = action {
            if self.effect_is_current(receipt) && self.view == receipt.view {
                self.error = Some(error);
            }
            return None;
        }
        self.view = self.view.wrapping_add(1);
        let custom = matches!(action, StreamSettingsAction::ApplyCustom);
        match action {
            StreamSettingsAction::EditFps(text) => {
                self.fps_text = text;
                return None;
            }
            StreamSettingsAction::EditBitrate(text) => {
                self.bitrate_text = text;
                return None;
            }
            StreamSettingsAction::ApplyCustom => {
                let parsed = self
                    .fps_text
                    .trim()
                    .parse::<u32>()
                    .ok()
                    .zip(self.bitrate_text.trim().parse::<u32>().ok());
                let Some((fps, bitrate_kbps)) = parsed.filter(|&(fps, bitrate)| {
                    protocol::validate_video_settings(
                        self.values.width,
                        self.values.height,
                        fps,
                        bitrate,
                    )
                    .is_ok()
                }) else {
                    self.error = Some("Enter positive whole numbers for FPS and kbps.".into());
                    return None;
                };
                self.values.fps = fps;
                self.values.bitrate_kbps = bitrate_kbps;
                self.error = None;
            }
            StreamSettingsAction::SelectResolution(width, height) => {
                self.values.width = width;
                self.values.height = height;
            }
            StreamSettingsAction::SelectFps(fps) => {
                self.values.fps = fps;
                self.fps_text = fps.to_string();
            }
            StreamSettingsAction::SelectBitrate(bitrate) => {
                self.values.bitrate_kbps = bitrate;
                self.bitrate_text = bitrate.to_string();
            }
            StreamSettingsAction::UpdateFailed(_, _) => unreachable!(),
        }
        self.command = self.command.wrapping_add(1);
        Some(StreamSettingsEffect {
            values: self.values,
            update: if custom {
                UpdatePolicy::WhenConnected
            } else {
                UpdatePolicy::Always
            },
            report_error: custom,
            receipt: SettingsReceipt {
                command: self.command,
                view: self.view,
            },
        })
    }
}

/// Latest committed settings per in-flight connection, not a renderer or network identity.
/// The Owner consumes these before attaching a successful handshake's session.
#[derive(Default)]
pub struct PendingStreamSettingsState {
    requests: BTreeMap<String, (u64, StreamSettingsValues)>,
}
impl PendingStreamSettingsState {
    pub fn begin(&mut self, device: &str, attempt: u64, values: StreamSettingsValues) {
        self.requests.insert(device.to_owned(), (attempt, values));
    }
    pub fn update(&mut self, device: &str, attempt: u64, values: StreamSettingsValues) -> bool {
        let Some((generation, current)) = self.requests.get_mut(device) else {
            return false;
        };
        if *generation != attempt {
            return false;
        }
        *current = values;
        true
    }
    pub fn take(&mut self, device: &str, attempt: u64) -> Option<StreamSettingsValues> {
        if self
            .requests
            .get(device)
            .is_none_or(|(generation, _)| *generation != attempt)
        {
            return None;
        }
        self.requests.remove(device).map(|(_, values)| values)
    }
    pub fn clear(&mut self) {
        self.requests.clear();
    }
}

/// Universal per-subscription pixel ceiling accepted by both desktop hosts and
/// Android MediaCodec publishers (`<= 4096` per edge, `<= 3840 * 2160` area).
pub const UNIVERSAL_WINDOW_MAX_EDGE: u32 = 4096;
pub const UNIVERSAL_WINDOW_MAX_PIXELS: u64 = 3840 * 2160;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowClarityScale {
    #[default]
    Native100,
    High75,
    Standard50,
    Saver35,
}

impl WindowClarityScale {
    pub const ALL: [Self; 4] = [
        Self::Native100,
        Self::High75,
        Self::Standard50,
        Self::Saver35,
    ];

    pub fn percent(self) -> u32 {
        match self {
            Self::Native100 => 100,
            Self::High75 => 75,
            Self::Standard50 => 50,
            Self::Saver35 => 35,
        }
    }

    pub fn from_percent(percent: u32) -> Self {
        match percent {
            0..=42 => Self::Saver35,
            43..=62 => Self::Standard50,
            63..=87 => Self::High75,
            _ => Self::Native100,
        }
    }

    /// Compute proportional even-aligned encode cap from the window's live source
    /// pixel dimensions without forcing portrait or square windows into 16:9.
    pub fn compute_cap(self, source: (u32, u32)) -> (u32, u32) {
        let (sw, sh) = if source.0 >= 2 && source.1 >= 2 {
            source
        } else {
            (1920, 1080)
        };
        let pct = f64::from(self.percent()) / 100.0;
        let raw_w = (f64::from(sw) * pct).round().max(2.0);
        let raw_h = (f64::from(sh) * pct).round().max(2.0);
        let edge_limit = f64::from(UNIVERSAL_WINDOW_MAX_EDGE);
        let edge_scale = (edge_limit / raw_w).min(edge_limit / raw_h).min(1.0);
        let area_scale = ((UNIVERSAL_WINDOW_MAX_PIXELS as f64) / (raw_w * raw_h))
            .sqrt()
            .min(1.0);
        let scale = edge_scale.min(area_scale);
        let w = (((raw_w * scale).floor() as u32) & !1).clamp(2, UNIVERSAL_WINDOW_MAX_EDGE);
        let h = (((raw_h * scale).floor() as u32) & !1).clamp(2, UNIVERSAL_WINDOW_MAX_EDGE);
        (w, h)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowLayoutPreset {
    #[default]
    KeepOriginal,
    FitLocalViewport,
    Fixed1280x800,
    Fixed1440x900,
    Fixed1600x1000,
    Portrait900x1440,
}

impl WindowLayoutPreset {
    pub const ALL: [Self; 6] = [
        Self::KeepOriginal,
        Self::FitLocalViewport,
        Self::Fixed1280x800,
        Self::Fixed1440x900,
        Self::Fixed1600x1000,
        Self::Portrait900x1440,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::KeepOriginal => "original",
            Self::FitLocalViewport => "fit_viewport",
            Self::Fixed1280x800 => "1280x800",
            Self::Fixed1440x900 => "1440x900",
            Self::Fixed1600x1000 => "1600x1000",
            Self::Portrait900x1440 => "900x1440",
        }
    }

    pub fn from_key(key: &str) -> Self {
        match key.trim() {
            "fit_viewport" => Self::FitLocalViewport,
            "1280x800" => Self::Fixed1280x800,
            "1440x900" => Self::Fixed1440x900,
            "1600x1000" => Self::Fixed1600x1000,
            "900x1440" => Self::Portrait900x1440,
            _ => Self::KeepOriginal,
        }
    }

    pub fn target_size(self, local_viewport: Option<(u32, u32)>) -> Option<(u32, u32)> {
        match self {
            Self::KeepOriginal => None,
            Self::FitLocalViewport => local_viewport.map(fit_remote_window_viewport),
            Self::Fixed1280x800 => Some((1280, 800)),
            Self::Fixed1440x900 => Some((1440, 900)),
            Self::Fixed1600x1000 => Some((1600, 1000)),
            Self::Portrait900x1440 => Some((900, 1440)),
        }
    }
}

/// Normalize a local viewer pane size into practical remote desktop/app logical
/// window bounds: preserve exact aspect ratio, ensure narrow mobile viewports
/// expand proportionally to at least 720 short-edge points so desktop apps do
/// not hit minimum-width refusal, and clamp within `320..=2560`.
pub fn fit_remote_window_viewport(viewport: (u32, u32)) -> (u32, u32) {
    let (vw, vh) = viewport;
    if vw < 64 || vh < 64 {
        return (1280, 800);
    }
    let short = vw.min(vh) as f64;
    let upscale = if short < 680.0 { 720.0 / short } else { 1.0 };
    let mut w = f64::from(vw) * upscale;
    let mut h = f64::from(vh) * upscale;
    let downscale = (2560.0 / w).min(2560.0 / h).min(1.0);
    w *= downscale;
    h *= downscale;
    let out_w = ((w.round() as u32) & !1).clamp(320, 2560);
    let out_h = ((h.round() as u32) & !1).clamp(320, 2560);
    (out_w, out_h)
}

/// Ephemeral session-scoped stream override for a single captured window,
/// strictly isolated from global full-screen `StreamSettingsValues`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowStreamOverride {
    pub clarity: WindowClarityScale,
    pub layout: WindowLayoutPreset,
    pub fps: u32,
    pub bitrate_kbps: u32,
}

impl WindowStreamOverride {
    pub fn from_baseline(baseline: StreamSettingsValues) -> Self {
        Self {
            clarity: WindowClarityScale::Native100,
            layout: WindowLayoutPreset::KeepOriginal,
            fps: baseline.fps.max(1),
            bitrate_kbps: baseline.bitrate_kbps.max(500),
        }
    }

    pub fn is_default(&self, baseline: StreamSettingsValues) -> bool {
        self.clarity == WindowClarityScale::Native100
            && self.layout == WindowLayoutPreset::KeepOriginal
            && self.fps == baseline.fps
            && self.bitrate_kbps == baseline.bitrate_kbps
    }

    pub fn effective_stream_values(&self, live_source: (u32, u32)) -> StreamSettingsValues {
        let (width, height) = self.clarity.compute_cap(live_source);
        StreamSettingsValues {
            width,
            height,
            fps: self.fps.max(1),
            bitrate_kbps: self.bitrate_kbps.max(500),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn values() -> StreamSettingsValues {
        StreamSettingsValues {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_kbps: 10_000,
        }
    }
    fn state() -> StreamSettingsState {
        StreamSettingsState::new(values())
    }

    #[test]
    fn drafts_survive_projection_without_changing_committed_values_or_emitting_effects() {
        let mut state = state();
        assert!(
            state
                .reduce(StreamSettingsAction::EditFps(" 75 ".into()))
                .is_none()
        );
        assert!(
            state
                .reduce(StreamSettingsAction::EditBitrate("12345".into()))
                .is_none()
        );
        let view = state.project();
        assert_eq!(view.fps_text, " 75 ");
        assert_eq!(view.bitrate_text, "12345");
        assert_eq!(view.values, values());
        assert_eq!(view.error, None);
        let effect = state.reduce(StreamSettingsAction::ApplyCustom).unwrap();
        assert_eq!(effect.values.fps, 75);
        assert_eq!(effect.values.bitrate_kbps, 12345);
        assert!(!effect.should_update(false));
        assert!(effect.should_update(true));
        assert!(effect.report_error);
    }
    #[test]
    fn invalid_custom_values_preserve_committed_settings_and_have_no_side_effects() {
        for text in ["", "0", "-1", "1.5", "4294967296", "214748365"] {
            let mut state = state();
            state.reduce(StreamSettingsAction::EditFps(text.into()));
            assert!(
                state.reduce(StreamSettingsAction::ApplyCustom).is_none(),
                "{text}"
            );
            assert_eq!(state.values(), values());
            assert!(state.project().error.is_some());
        }
        let mut state = state();
        state.reduce(StreamSettingsAction::EditBitrate("0".into()));
        assert!(state.reduce(StreamSettingsAction::ApplyCustom).is_none());
        assert_eq!(state.values(), values());
        state.reduce(StreamSettingsAction::EditBitrate(u32::MAX.to_string()));
        assert_eq!(
            state
                .reduce(StreamSettingsAction::ApplyCustom)
                .unwrap()
                .values
                .bitrate_kbps,
            u32::MAX
        );
    }
    #[test]
    fn presets_preserve_other_values_and_sync_only_the_matching_draft() {
        let mut state = state();
        state.reduce(StreamSettingsAction::EditFps("75".into()));
        let resolution = state
            .reduce(StreamSettingsAction::SelectResolution(3840, 2160))
            .unwrap();
        assert_eq!(resolution.values.resolution(), (3840, 2160));
        assert_eq!(resolution.values.fps, 60);
        assert_eq!(state.project().fps_text, "75");
        let fps = state.reduce(StreamSettingsAction::SelectFps(120)).unwrap();
        assert_eq!(state.project().fps_text, "120");
        assert_eq!(fps.values.bitrate_kbps, 10_000);
        let bitrate = state
            .reduce(StreamSettingsAction::SelectBitrate(80_000))
            .unwrap();
        assert_eq!(state.project().bitrate_text, "80000");
        assert_eq!(bitrate.values.fps, 120);
        assert!(bitrate.should_update(false));
        assert!(!bitrate.report_error);
    }
    #[test]
    fn editing_next_draft_keeps_committed_effect_valid_but_rejects_its_late_error() {
        let mut state = state();
        let effect = state.reduce(StreamSettingsAction::ApplyCustom).unwrap();
        state.reduce(StreamSettingsAction::EditFps("75".into()));
        assert!(state.effect_is_current(effect.receipt));
        state.reduce(StreamSettingsAction::UpdateFailed(
            effect.receipt,
            "old error".into(),
        ));
        assert_eq!(state.project().error, None);
        assert_eq!(state.project().fps_text, "75");
        let latest = state.reduce(StreamSettingsAction::ApplyCustom).unwrap();
        assert!(!state.effect_is_current(effect.receipt));
        state.reduce(StreamSettingsAction::UpdateFailed(
            latest.receipt,
            "current error".into(),
        ));
        assert_eq!(state.project().error, Some("current error"));
        state.reduce(StreamSettingsAction::EditFps("bad".into()));
        state.reduce(StreamSettingsAction::ApplyCustom);
        state.reduce(StreamSettingsAction::UpdateFailed(
            latest.receipt,
            "late error".into(),
        ));
        assert_eq!(
            state.project().error,
            Some("Enter positive whole numbers for FPS and kbps.")
        );
    }

    #[test]
    fn connecting_request_consumes_latest_commit_without_consuming_unapplied_drafts() {
        let mut pending = PendingStreamSettingsState::default();
        let mut form = state();
        pending.begin("a", 1, form.values());
        form.reduce(StreamSettingsAction::EditFps("75".into()));
        let resolution = form
            .reduce(StreamSettingsAction::SelectResolution(3840, 2160))
            .unwrap();
        assert!(pending.update("a", 1, resolution.values));
        let custom = form.reduce(StreamSettingsAction::ApplyCustom).unwrap();
        assert!(pending.update("a", 1, custom.values));
        form.reduce(StreamSettingsAction::EditFps("invalid next draft".into()));
        let committed = pending.take("a", 1).unwrap();
        assert_eq!(committed, custom.values);
        assert_eq!(committed.resolution(), (3840, 2160));
        assert_eq!(committed.fps, 75);
        assert!(!pending.update("a", 1, values()));
    }

    #[test]
    fn pending_settings_are_isolated_by_device_and_reconnect_generation() {
        let mut pending = PendingStreamSettingsState::default();
        let latest = StreamSettingsValues {
            fps: 120,
            ..values()
        };
        pending.begin("a", 1, values());
        pending.begin("b", 2, values());
        assert!(pending.update("b", 2, latest));
        assert_eq!(pending.take("a", 1), Some(values()));
        pending.begin("b", 3, values());
        assert!(!pending.update("b", 2, latest));
        assert_eq!(pending.take("b", 2), None);
        assert_eq!(pending.take("b", 3), Some(values()));
    }

    #[test]
    fn cancelled_failed_and_cleared_requests_cannot_retain_or_create_settings() {
        let mut pending = PendingStreamSettingsState::default();
        assert!(!pending.update("missing", 1, values()));
        pending.begin("a", 1, values());
        assert_eq!(pending.take("a", 1), Some(values()));
        assert_eq!(pending.take("a", 1), None);
        pending.begin("a", 2, values());
        pending.begin("b", 3, values());
        pending.clear();
        assert!(!pending.update("a", 2, values()));
        assert_eq!(pending.take("a", 2), None);
        assert_eq!(pending.take("b", 3), None);
    }

    #[test]
    fn window_clarity_scale_preserves_aspect_and_respects_universal_ceiling() {
        assert_eq!(
            WindowClarityScale::Native100.compute_cap((1200, 800)),
            (1200, 800)
        );
        assert_eq!(
            WindowClarityScale::High75.compute_cap((1200, 800)),
            (900, 600)
        );
        assert_eq!(
            WindowClarityScale::Standard50.compute_cap((1200, 800)),
            (600, 400)
        );
        assert_eq!(
            WindowClarityScale::Saver35.compute_cap((1200, 800)),
            (420, 280)
        );
        // Portrait window is never squished into landscape 16:9.
        let (pw, ph) = WindowClarityScale::High75.compute_cap((1080, 2400));
        assert_eq!((pw, ph), (810, 1800));
        // 5K Retina window clamps cleanly within 4096 edge and 3840x2160 pixel area.
        let (big_w, big_h) = WindowClarityScale::Native100.compute_cap((5120, 2880));
        assert!(big_w <= UNIVERSAL_WINDOW_MAX_EDGE && big_h <= UNIVERSAL_WINDOW_MAX_EDGE);
        assert!(u64::from(big_w) * u64::from(big_h) <= UNIVERSAL_WINDOW_MAX_PIXELS);
        assert_eq!(big_w % 2, 0);
        assert_eq!(big_h % 2, 0);
    }

    #[test]
    fn window_layout_preset_and_override_stay_isolated_from_global_baseline() {
        let baseline = values();
        let mut ov = WindowStreamOverride::from_baseline(baseline);
        assert!(ov.is_default(baseline));
        assert_eq!(ov.layout.target_size(Some((1400, 900))), None);

        ov.clarity = WindowClarityScale::Standard50;
        ov.layout = WindowLayoutPreset::FitLocalViewport;
        ov.fps = 30;
        assert!(!ov.is_default(baseline));

        // Narrow phone portrait viewport expands proportionally above desktop min-width.
        let fitted_mobile = ov.layout.target_size(Some((390, 844))).unwrap();
        assert_eq!(fitted_mobile.0, 720);
        assert!(fitted_mobile.1 > 1500 && fitted_mobile.1 <= 2560);

        // Desktop viewer pane maps 1:1 when already comfortable.
        assert_eq!(ov.layout.target_size(Some((1360, 880))), Some((1360, 880)));

        let stream = ov.effective_stream_values((1360, 880));
        assert_eq!((stream.width, stream.height), (680, 440));
        assert_eq!(stream.fps, 30);
        assert_eq!(baseline.resolution(), (1920, 1080));
        assert_eq!(baseline.fps, 60);
    }
}
