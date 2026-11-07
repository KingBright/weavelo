use gpui::{Hsla, Rgba, rgb, rgba};
use crate::product_components::theme::{Theme, ThemeSet};

/// Resolve a semantic font role without changing the original visual hierarchy.
/// DirectWrite does not interpret CSS generic family names as installed fonts.
pub(crate) const fn product_ui_font() -> &'static str {
    if cfg!(any(target_os="macos",target_os="windows")){".SystemUIFont"}else{"sans-serif"}
}
/// Keep numeric telemetry monospaced; never silently substitute proportional text.
pub(crate) const fn product_mono_font() -> &'static str {
    if cfg!(target_os="windows"){"Consolas"}else{"monospace"}
}

pub fn weavelo_themes() -> ThemeSet {
    ThemeSet::new(light_theme()).dark(dark_theme())
}

#[allow(dead_code)]
#[inline]
pub fn color_accent_cyan() -> Rgba {
    rgb(0x38bdf8)
}

#[allow(dead_code)]
#[inline]
pub fn color_accent_emerald() -> Rgba {
    rgb(0x10b981)
}

#[allow(dead_code)]
#[inline]
pub fn color_accent_purple() -> Rgba {
    rgb(0x818cf8)
}

#[allow(dead_code)]
#[inline]
pub fn color_accent_amber() -> Rgba {
    rgb(0xe5a93c)
}

#[allow(dead_code)]
#[inline]
pub fn color_accent_rose() -> Rgba {
    rgb(0xf43f5e)
}

#[allow(dead_code)]
#[inline]
pub fn color_glass_bg() -> Hsla {
    rgba(0x080b11eb).into()
}

#[allow(dead_code)]
#[inline]
pub fn color_glass_card() -> Hsla {
    rgba(0x111622e8).into()
}

#[allow(dead_code)]
#[inline]
pub fn color_border_fine() -> Rgba {
    rgb(0x263042)
}

pub(crate) fn dark_theme() -> Theme {
    let mut theme = Theme::default_dark();

    theme.surface.canvas = rgb(0x080b11).into();
    theme.surface.base = rgb(0x111622).into();
    theme.surface.raised = rgb(0x171f30).into();
    theme.surface.sunken = rgb(0x05070b).into();
    theme.surface.hover = rgb(0x1e293b).into();

    theme.content.primary = rgb(0xf8fafc).into();
    theme.content.secondary = rgb(0xcbd5e1).into();
    theme.content.tertiary = rgb(0x94a3b8).into();
    theme.content.disabled = rgb(0x475569).into();
    theme.content.on_primary = rgb(0x04070d).into();
    theme.content.on_status = rgb(0x04070d).into();

    theme.border.default = rgb(0x263042).into();
    theme.border.muted = rgb(0x1b2332).into();
    theme.border.focus = rgb(0x38bdf8).into();
    theme.border.divider = rgb(0x1b2332).into();

    theme.action.neutral.bg = rgb(0x171f30).into();
    theme.action.neutral.hover_bg = rgb(0x222d42).into();
    theme.action.neutral.active_bg = rgb(0x2c3a52).into();
    theme.action.neutral.fg = theme.content.primary;
    theme.action.neutral.disabled_bg = rgb(0x0f141f).into();
    theme.action.neutral.disabled_fg = theme.content.disabled;

    theme.action.primary.bg = rgb(0x38bdf8).into();
    theme.action.primary.hover_bg = rgb(0x7dd3fc).into();
    theme.action.primary.active_bg = rgb(0x0ea5e9).into();
    theme.action.primary.fg = theme.content.on_primary;
    theme.action.primary.disabled_bg = rgb(0x1e3a4c).into();
    theme.action.primary.disabled_fg = rgb(0x748a99).into();

    theme.action.danger.bg = rgb(0xff8f87).into();
    theme.action.danger.hover_bg = rgb(0xffa69f).into();
    theme.action.danger.active_bg = rgb(0xf37970).into();
    theme.action.danger.fg = rgb(0x1e0908).into();
    theme.action.danger.disabled_bg = rgb(0x54302e).into();
    theme.action.danger.disabled_fg = theme.content.disabled;

    theme.status.success.bg = rgb(0x6ee7b7).into();
    theme.status.success.fg = rgb(0x06281b).into();
    theme.status.warning.bg = rgb(0xfcd34d).into();
    theme.status.warning.fg = rgb(0x2e1c04).into();
    theme.status.error.bg = rgb(0xff928a).into();
    theme.status.error.fg = rgb(0x3b0d0a).into();
    theme.status.info.bg = rgb(0x7dd3fc).into();
    theme.status.info.fg = rgb(0x062235).into();

    theme
}

pub(crate) fn light_theme() -> Theme {
    let mut theme = Theme::default_light();

    theme.surface.canvas = rgb(0xf8fafc).into();
    theme.surface.base = rgb(0xffffff).into();
    theme.surface.raised = rgb(0xf1f5f9).into();
    theme.surface.sunken = rgb(0xe2e8f0).into();
    theme.surface.hover = rgb(0xe2e8f0).into();

    theme.content.primary = rgb(0x0f172a).into();
    theme.content.secondary = rgb(0x334155).into();
    theme.content.tertiary = rgb(0x64748b).into();
    theme.content.disabled = rgb(0x94a3b8).into();
    theme.content.on_primary = rgb(0xffffff).into();
    theme.content.on_status = rgb(0x0f172a).into();

    theme.border.default = rgb(0xcbd5e1).into();
    theme.border.muted = rgb(0xe2e8f0).into();
    theme.border.focus = rgb(0x0284c7).into();
    theme.border.divider = rgb(0xe2e8f0).into();

    theme.action.neutral.bg = rgb(0xf1f5f9).into();
    theme.action.neutral.hover_bg = rgb(0xe2e8f0).into();
    theme.action.neutral.active_bg = rgb(0xcbd5e1).into();
    theme.action.neutral.fg = theme.content.primary;
    theme.action.neutral.disabled_bg = rgb(0xf8fafc).into();
    theme.action.neutral.disabled_fg = theme.content.disabled;

    theme.action.primary.bg = rgb(0x0f172a).into();
    theme.action.primary.hover_bg = rgb(0x1e293b).into();
    theme.action.primary.active_bg = rgb(0x020617).into();
    theme.action.primary.fg = theme.content.on_primary;
    theme.action.primary.disabled_bg = rgb(0x94a3b8).into();
    theme.action.primary.disabled_fg = rgb(0xf1f5f9).into();

    theme.action.danger.bg = rgb(0xc94843).into();
    theme.action.danger.hover_bg = rgb(0xb83d38).into();
    theme.action.danger.active_bg = rgb(0xa93430).into();
    theme.action.danger.fg = rgb(0xffffff).into();
    theme.action.danger.disabled_bg = rgb(0xd9aaa7).into();
    theme.action.danger.disabled_fg = rgb(0xf7e9e8).into();

    theme.status.success.bg = rgb(0xd7f5e4).into();
    theme.status.success.fg = rgb(0x145f3c).into();
    theme.status.warning.bg = rgb(0xffedc1).into();
    theme.status.warning.fg = rgb(0x704a00).into();
    theme.status.error.bg = rgb(0xffd8d4).into();
    theme.status.error.fg = rgb(0x8e2924).into();
    theme.status.info.bg = rgb(0xd5efff).into();
    theme.status.info.fg = rgb(0x165b82).into();

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

#[cfg(test)] mod platform_font_tests {
    use super::*;
    #[test] fn font_roles_are_explicit_and_distinct() {
        assert!(!product_ui_font().is_empty());assert!(!product_mono_font().is_empty());
        assert_ne!(product_ui_font(),product_mono_font());
    }
    #[test] fn windows_does_not_request_css_names_from_directwrite() {
        if cfg!(target_os="windows") {
            assert_eq!(product_ui_font(),".SystemUIFont");assert_eq!(product_mono_font(),"Consolas");
        }
    }
}
