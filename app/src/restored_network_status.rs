//! Session telemetry lives as a minimalist signal-only icon in the stream header,
//! expanding into a floating diagnostic popover on click.
use super::*;

fn localized_route_label(route: &str) -> String {
    match route.trim() {
        "Lan" | "LAN" => crate::i18n::tr("LAN").to_owned(),
        "Relay" | "RELAY" => crate::i18n::tr("Relay").to_owned(),
        "P2P" | "Direct" => crate::i18n::tr("P2P Direct").to_owned(),
        other => crate::i18n::message(other),
    }
}

impl RestoredDashboard {
    fn network_status_details(
        role: &RoleState,
        route: Option<&str>,
        stats: Option<&UiHostStats>,
    ) -> (String, NetworkQuality, Vec<(String, String)>) {
        let quality = UiHostStats::network_quality(role.kind(), stats);
        let live = stats.filter(|s| {
            role.kind() == remote_core::role::RoleKind::Viewing
                && s.updated_at
                    .is_some_and(|at| at.elapsed() <= Duration::from_secs(5))
        });
        let rtt = stats
            .and_then(|s| s.rtt)
            .map(|rtt| format!("{:.0} ms", rtt.latest_ms));
        let route_text = route
            .filter(|r| !r.trim().is_empty())
            .map(localized_route_label);
        let base_label = rtt.clone().unwrap_or_else(|| quality.label().into());
        let label = match (&route_text, role.kind()) {
            (
                Some(r),
                remote_core::role::RoleKind::Viewing | remote_core::role::RoleKind::Connecting,
            ) => format!("{r} · {base_label}"),
            _ => base_label,
        };
        let capture_ui_value = live
            .map(|s| {
                let compact = s.capture_submit.compact_value();
                if compact != "—" {
                    compact
                } else if s.capture_receive.compact_value() != "—" {
                    s.capture_receive.compact_value()
                } else if s.latency.is_finite() && s.latency > 0.0 {
                    format!("{} {:.1} ms", crate::i18n::tr("Enc"), s.latency)
                } else if s.fps.is_finite() && s.fps == 0.0 {
                    crate::i18n::tr("Static").into()
                } else {
                    "—".into()
                }
            })
            .unwrap_or_else(|| "—".into());
        let fps_value = live
            .and_then(|s| {
                if !s.fps.is_finite() || s.fps < 0.0 {
                    None
                } else if s.fps > 0.0 {
                    Some(format!("{:.0} FPS", s.fps))
                } else {
                    Some(format!("0 FPS ({})", crate::i18n::tr("Static")))
                }
            })
            .unwrap_or_else(|| "—".into());
        let rows = vec![
            (crate::i18n::tr("Connection").into(), quality.label().into()),
            (
                crate::i18n::tr("Route").into(),
                route_text.unwrap_or_else(|| "—".into()),
            ),
            (crate::i18n::tr("Capture → UI").into(), capture_ui_value),
            (
                crate::i18n::tr("Network RTT").into(),
                rtt.unwrap_or_else(|| "—".into()),
            ),
            (crate::i18n::tr("Frame rate").into(), fps_value),
            (
                crate::i18n::tr("Packet loss rate").into(),
                live.filter(|s| {
                    s.packet_loss_rate.is_finite() && (0.0..=100.0).contains(&s.packet_loss_rate)
                })
                .map(|s| format!("{:.1}%", s.packet_loss_rate))
                .unwrap_or_else(|| "—".into()),
            ),
            (
                crate::i18n::tr("Bitrate").into(),
                live.map(|s| format_bitrate(s.bitrate_kbps))
                    .unwrap_or_else(|| "—".into()),
            ),
        ];
        (label, quality, rows)
    }

    pub(super) fn network_status_chip(
        &self,
        role: &RoleState,
        route: Option<&str>,
        stats: Option<&UiHostStats>,
        tiny: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let (label, quality, _) = Self::network_status_details(role, route, stats);
        let button_size = if tiny { 20.0 } else { 26.0 };
        let icon_size = if tiny { 11.0 } else { 13.0 };
        div()
            .id("network-status-hit-area")
            .occlude()
            .flex_none()
            .on_any_mouse_down(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(
                command_button(
                    "network-status-chip",
                    if self.network_details_open {
                        ActionVariantKind::Primary
                    } else {
                        ActionVariantKind::Neutral
                    },
                    cx,
                )
                .size(px(button_size))
                .rounded_full()
                .p_0()
                .tooltip(
                    tooltip(format!(
                        "{} · {}",
                        crate::i18n::tr("Network details"),
                        label
                    ))
                    .build(),
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.network_details_open = !this.network_details_open;
                    if this.network_details_open {
                        this.drawer_open = false;
                        this.show_apps_menu = false;
                    }
                    this.reset_pointer_scroll();
                    this.owner.release_input();
                    this.pointer_input = PointerInputTracker::default();
                    this.last_toolbar_activity = Instant::now();
                    window.focus(&this.root_focus);
                    cx.notify();
                }))
                .child(network_quality_icon(quality, &cx.theme()).size(px(icon_size))),
            )
    }

    pub(super) fn network_status_panel(
        &self,
        role: &RoleState,
        route: Option<&str>,
        stats: Option<&UiHostStats>,
        fullscreen: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let (_, _, rows) = Self::network_status_details(role, route, stats);
        let card_bg = crate::design_system::theme_glass_card(cx);
        let card_border = crate::design_system::theme_border_fine(cx);
        let divider = cx.theme().border.divider;
        let secondary = cx.theme().content.secondary;
        let top_offset = if fullscreen {
            46.0
        } else {
            window_fit::STREAM_HEADER_HEIGHT_PX + 6.0
        };
        div()
            .id("network-status-details")
            .absolute()
            .top(px(top_offset))
            .right(px(10.))
            .min_w(px(216.))
            .max_w(relative(0.95))
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2p5()
            .rounded(px(12.))
            .occlude()
            .bg(card_bg)
            .border_1()
            .border_color(card_border)
            .shadow_lg()
            .on_any_mouse_down(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .pb_1()
                    .mb_0p5()
                    .border_b_1()
                    .border_color(divider)
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .whitespace_nowrap()
                            .child(crate::i18n::tr("Network details")),
                    )
                    .child(
                        command_button("close-network-status", ActionVariantKind::Neutral, cx)
                            .size(px(22.))
                            .rounded(px(6.))
                            .p_0()
                            .child(icon(IconName::Close).size(px(12.)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.network_details_open = false;
                                cx.notify();
                            })),
                    ),
            )
            .children(rows.into_iter().map(move |(name, value)| {
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(secondary)
                            .whitespace_nowrap()
                            .child(name),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_family(crate::design_system::product_mono_font())
                            .font_weight(FontWeight::MEDIUM)
                            .whitespace_nowrap()
                            .child(value),
                    )
            }))
    }
}
