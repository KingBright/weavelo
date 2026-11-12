use gpui::{App, Hsla, Rgba, rgb, rgba};
use crate::product_components::theme::{ActiveTheme, Theme, ThemeSet};

/// Resolve a semantic font role without changing the original visual hierarchy.
/// DirectWrite does not interpret CSS generic family names as installed fonts.
pub(crate) const fn product_ui_font() -> &'static str {
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        ".SystemUIFont"
    } else {
        "sans-serif"
    }
}

/// Keep numeric telemetry monospaced; never silently substitute proportional text.
pub(crate) const fn product_mono_font() -> &'static str {
    if cfg!(target_os = "windows") {
        "Consolas"
    } else {
        "monospace"
    }
}

pub fn weavelo_themes() -> ThemeSet {
    ThemeSet::new(light_theme()).dark(dark_theme())
}

#[inline]
pub fn is_dark_mode(cx: &App) -> bool {
    ely_gpui_component::theme::ActiveTheme::theme(cx).is_dark()
}

/// Theme-adaptive elevated glass card surface (Prism Air mist-white / Obsidian Craft micro-crystal).
#[inline]
pub fn theme_glass_card(cx: &App) -> Hsla {
    if is_dark_mode(cx) {
        rgba(0x171d28f2).into()
    } else {
        rgba(0xfffffff5).into()
    }
}

/// Theme-adaptive floating HUD / toolbar backdrop.
#[inline]
pub fn theme_glass_floating(cx: &App) -> Hsla {
    if is_dark_mode(cx) {
        rgba(0x131923eb).into()
    } else {
        rgba(0xf8fafcf2).into()
    }
}

/// Theme-adaptive 1px architectural hairline border.
#[inline]
pub fn theme_border_fine(cx: &App) -> Hsla {
    cx.theme().border.default
}

/// Theme-adaptive primary brand accent for text, badges, and highlights.
#[inline]
pub fn theme_accent_brand(cx: &App) -> Hsla {
    if is_dark_mode(cx) {
        rgb(0x38bdf8).into()
    } else {
        rgb(0x1b56d0).into()
    }
}

/// Theme-adaptive soft brand tint container fill for active rows and badges.
#[inline]
pub fn theme_brand_soft(cx: &App) -> Hsla {
    if is_dark_mode(cx) {
        rgba(0x38bdf824).into()
    } else {
        rgb(0xeaf2ff).into()
    }
}

/// Theme-adaptive prism cyan-teal secondary accent (telemetry & route pills).
#[inline]
pub fn theme_accent_cyan(cx: &App) -> Hsla {
    if is_dark_mode(cx) {
        rgb(0x38bdf8).into()
    } else {
        rgb(0x0c7792).into()
    }
}

/// Theme-adaptive online / live emerald accent with readable text contrast.
#[inline]
pub fn theme_accent_emerald(cx: &App) -> Hsla {
    if is_dark_mode(cx) {
        rgb(0x34d399).into()
    } else {
        rgb(0x057a55).into()
    }
}

/// Vibrant online indicator dot color (used strictly for status dots/bars, not body text).
#[inline]
pub fn theme_online_dot(cx: &App) -> Hsla {
    if is_dark_mode(cx) {
        rgb(0x34d399).into()
    } else {
        rgb(0x10b981).into()
    }
}

/// Theme-adaptive warm ochre warning accent with readable text contrast.
#[inline]
pub fn theme_accent_amber(cx: &App) -> Hsla {
    if is_dark_mode(cx) {
        rgb(0xfbbf24).into()
    } else {
        rgb(0xb45309).into()
    }
}

/// Theme-adaptive crimson-rose error/alert accent with readable text contrast.
#[inline]
pub fn theme_accent_rose(cx: &App) -> Hsla {
    if is_dark_mode(cx) {
        rgb(0xf87171).into()
    } else {
        rgb(0xc83a37).into()
    }
}

/// Theme-adaptive muted indigo-slate accent for secondary pipeline stage metrics.
#[inline]
pub fn theme_accent_indigo(cx: &App) -> Hsla {
    if is_dark_mode(cx) {
        rgb(0x93c5fd).into()
    } else {
        rgb(0x3b5bdb).into()
    }
}

#[allow(dead_code)]
#[inline]
pub fn color_accent_cyan() -> Rgba {
    rgb(0x1b56d0)
}

#[allow(dead_code)]
#[inline]
pub fn color_accent_emerald() -> Rgba {
    rgb(0x10b981)
}

#[allow(dead_code)]
#[inline]
pub fn color_accent_purple() -> Rgba {
    rgb(0x3b5bdb)
}

#[allow(dead_code)]
#[inline]
pub fn color_accent_amber() -> Rgba {
    rgb(0xd97706)
}

#[allow(dead_code)]
#[inline]
pub fn color_accent_rose() -> Rgba {
    rgb(0xc83a37)
}

#[allow(dead_code)]
#[inline]
pub fn color_glass_bg() -> Hsla {
    rgba(0xf4f7faf2).into()
}

#[allow(dead_code)]
#[inline]
pub fn color_glass_card() -> Hsla {
    rgba(0xfffffff5).into()
}

#[allow(dead_code)]
#[inline]
pub fn color_border_fine() -> Rgba {
    rgb(0xdae3ee)
}

/// Obsidian Craft — Deep layered micro-crystal slate + luminous prism sky-cyan.
pub(crate) fn dark_theme() -> Theme {
    let mut theme = Theme::default_dark();

    theme.surface.canvas = rgb(0x0f131a).into();
    theme.surface.base = rgb(0x161c26).into();
    theme.surface.raised = rgb(0x1d2533).into();
    theme.surface.sunken = rgb(0x0a0d13).into();
    theme.surface.hover = rgb(0x242e3f).into();

    theme.content.primary = rgb(0xf1f5f9).into();
    theme.content.secondary = rgb(0x9aa8bc).into();
    theme.content.tertiary = rgb(0x6e7e94).into();
    theme.content.disabled = rgb(0x495567).into();
    theme.content.on_primary = rgb(0x06131f).into();
    theme.content.on_status = rgb(0x071921).into();

    theme.border.default = rgb(0x253041).into();
    theme.border.muted = rgb(0x1b2331).into();
    theme.border.focus = rgb(0x38bdf8).into();
    theme.border.divider = rgb(0x1e2736).into();

    theme.action.neutral.bg = rgb(0x1d2533).into();
    theme.action.neutral.hover_bg = rgb(0x263143).into();
    theme.action.neutral.active_bg = rgb(0x313e54).into();
    theme.action.neutral.fg = theme.content.primary;
    theme.action.neutral.disabled_bg = rgb(0x141923).into();
    theme.action.neutral.disabled_fg = theme.content.disabled;

    theme.action.primary.bg = rgb(0x38bdf8).into();
    theme.action.primary.hover_bg = rgb(0x5ccbfb).into();
    theme.action.primary.active_bg = rgb(0x1fa6e0).into();
    theme.action.primary.fg = theme.content.on_primary;
    theme.action.primary.disabled_bg = rgb(0x1e384b).into();
    theme.action.primary.disabled_fg = rgb(0x6e8da3).into();

    theme.action.danger.bg = rgb(0xf87171).into();
    theme.action.danger.hover_bg = rgb(0xfca5a5).into();
    theme.action.danger.active_bg = rgb(0xef4444).into();
    theme.action.danger.fg = rgb(0x1e0808).into();
    theme.action.danger.disabled_bg = rgb(0x482224).into();
    theme.action.danger.disabled_fg = theme.content.disabled;

    theme.status.success.bg = rgb(0x34d399).into();
    theme.status.success.fg = rgb(0x05291d).into();
    theme.status.warning.bg = rgb(0xfbbf24).into();
    theme.status.warning.fg = rgb(0x2c1c03).into();
    theme.status.error.bg = rgb(0xf87171).into();
    theme.status.error.fg = rgb(0x2d0a0a).into();
    theme.status.info.bg = rgb(0x38bdf8).into();
    theme.status.info.fg = rgb(0x061926).into();

    theme
}

/// Prism Air — Warm mist alabaster + signature Weavelo prism cobalt-indigo.
pub(crate) fn light_theme() -> Theme {
    let mut theme = Theme::default_light();

    theme.surface.canvas = rgb(0xf4f7fa).into();
    theme.surface.base = rgb(0xffffff).into();
    theme.surface.raised = rgb(0xffffff).into();
    theme.surface.sunken = rgb(0xeaeff6).into();
    theme.surface.hover = rgb(0xe1e8f2).into();

    theme.content.primary = rgb(0x0f172a).into();
    theme.content.secondary = rgb(0x475569).into();
    theme.content.tertiary = rgb(0x64748b).into();
    theme.content.disabled = rgb(0x94a3b8).into();
    theme.content.on_primary = rgb(0xffffff).into();
    theme.content.on_status = rgb(0x091926).into();

    theme.border.default = rgb(0xdae3ee).into();
    theme.border.muted = rgb(0xe6edf5).into();
    theme.border.focus = rgb(0x1b56d0).into();
    theme.border.divider = rgb(0xe2e9f2).into();

    theme.action.neutral.bg = rgb(0xeef3f8).into();
    theme.action.neutral.hover_bg = rgb(0xe2eaf3).into();
    theme.action.neutral.active_bg = rgb(0xd3dfec).into();
    theme.action.neutral.fg = theme.content.primary;
    theme.action.neutral.disabled_bg = rgb(0xf1f5f9).into();
    theme.action.neutral.disabled_fg = theme.content.disabled;

    theme.action.primary.bg = rgb(0x1b56d0).into();
    theme.action.primary.hover_bg = rgb(0x1648b2).into();
    theme.action.primary.active_bg = rgb(0x123b94).into();
    theme.action.primary.fg = theme.content.on_primary;
    theme.action.primary.disabled_bg = rgb(0xa3b8e0).into();
    theme.action.primary.disabled_fg = rgb(0xf8fafc).into();

    theme.action.danger.bg = rgb(0xc83a37).into();
    theme.action.danger.hover_bg = rgb(0xb22f2c).into();
    theme.action.danger.active_bg = rgb(0x992624).into();
    theme.action.danger.fg = rgb(0xffffff).into();
    theme.action.danger.disabled_bg = rgb(0xdfa8a6).into();
    theme.action.danger.disabled_fg = rgb(0xfaf0ef).into();

    theme.status.success.bg = rgb(0xdcfce7).into();
    theme.status.success.fg = rgb(0x065f46).into();
    theme.status.warning.bg = rgb(0xfef2d6).into();
    theme.status.warning.fg = rgb(0x784905).into();
    theme.status.error.bg = rgb(0xfee6e4).into();
    theme.status.error.fg = rgb(0x8e2420).into();
    theme.status.info.bg = rgb(0xe6f1ff).into();
    theme.status.info.fg = rgb(0x1446a8).into();

    theme
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relative_luminance(color: Hsla) -> f32 {
        let color = Rgba::from(color);
        let linear = |value: f32| {
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
    }

    fn contrast_ratio(a: Hsla, b: Hsla) -> f32 {
        let a = relative_luminance(a);
        let b = relative_luminance(b);
        let (lighter, darker) = if a > b { (a, b) } else { (b, a) };
        (lighter + 0.05) / (darker + 0.05)
    }

    #[test]
    fn themes_keep_text_and_actions_readable() {
        let themes = [dark_theme(), light_theme()];
        for theme in themes {
            assert!(
                contrast_ratio(theme.content.primary, theme.surface.canvas) >= 7.0,
                "primary text must meet enhanced contrast"
            );
            assert!(
                contrast_ratio(theme.action.primary.fg, theme.action.primary.bg) >= 4.5,
                "primary actions must meet normal text contrast"
            );
            assert!(
                contrast_ratio(theme.action.danger.fg, theme.action.danger.bg) >= 4.5,
                "danger actions must meet normal text contrast"
            );
        }
    }
}

#[cfg(test)]
mod platform_font_tests {
    use super::*;
    #[test]
    fn font_roles_are_explicit_and_distinct() {
        assert!(!product_ui_font().is_empty());
        assert!(!product_mono_font().is_empty());
        assert_ne!(product_ui_font(), product_mono_font());
    }
    #[test]
    fn windows_does_not_request_css_names_from_directwrite() {
        if cfg!(target_os = "windows") {
            assert_eq!(product_ui_font(), ".SystemUIFont");
            assert_eq!(product_mono_font(), "Consolas");
        }
    }
}
