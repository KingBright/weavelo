//! Shared desktop stage & status capsule surfaces, adaptive across Prism Air & Obsidian Craft.
//! The production GPUI view and platform probe use these same functions.
use crate::design_system::{theme_accent_brand, theme_border_fine, theme_glass_card};
use crate::product_components::{
    IconName, icon,
    theme::{ActiveTheme, Theme},
};
use gpui::prelude::FluentBuilder;
use gpui::*;

pub(crate) fn full_idle_canvas_stage<T: 'static>(cx: &mut Context<T>) -> Div {
    let theme = cx.theme().clone();
    let accent = theme_accent_brand(cx);
    div()
        .w_full()
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.surface.canvas)
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_4()
                .p_8()
                .rounded(px(16.0))
                .bg(theme_glass_card(cx))
                .border_1()
                .border_color(theme_border_fine(cx))
                .shadow_lg()
                .child(
                    div()
                        .size(px(60.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(theme.surface.sunken)
                        .border_1()
                        .border_color(accent)
                        .child(
                            icon(IconName::Server)
                                .size(px(26.0))
                                .color(accent),
                        ),
                )
                .child(
                    div()
                        .text_size(px(18.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.content.primary)
                        .child("Weavelo"),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(theme.content.secondary)
                        .child(crate::i18n::tr("Ready for incoming and outgoing sessions")),
                ),
        )
}

pub(crate) fn stream_status_capsule_card(
    title: String,
    is_live: bool,
    compact_stats: String,
    compact: bool,
    theme: &Theme,
    action_slot: Option<AnyElement>,
) -> Div {
    let is_dark = Rgba::from(theme.surface.canvas).r < 0.3;
    let capsule_bg: Hsla = if is_dark {
        rgba(0x171d28f2).into()
    } else {
        rgba(0xfffffff5).into()
    };
    let live_dot: Hsla = if is_dark {
        rgb(0x34d399).into()
    } else {
        rgb(0x10b981).into()
    };
    div()
        .flex()
        .items_center()
        .gap_3()
        .px_4()
        .py_2()
        .bg(capsule_bg)
        .border_1()
        .border_color(theme.border.default)
        .rounded_full()
        .shadow_lg()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_1()
                .child(
                    div()
                        .size(px(8.0))
                        .flex_none()
                        .rounded_full()
                        .bg(if is_live {
                            live_dot
                        } else {
                            theme.content.disabled
                        }),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.content.primary)
                        .whitespace_nowrap()
                        .max_w(if compact { px(120.0) } else { px(220.0) })
                        .truncate()
                        .child(title),
                ),
        )
        .child(div().w(px(1.0)).h(px(16.0)).bg(theme.border.divider))
        .child(
            div()
                .text_size(px(10.0))
                // Preserve fixed-width telemetry on DirectWrite, which does not
                // interpret the CSS generic name as an installed font family.
                .font_family(crate::design_system::product_mono_font())
                .text_color(theme.content.secondary)
                .whitespace_nowrap()
                .child(compact_stats),
        )
        .when_some(action_slot, |this, slot| this.child(slot))
}
