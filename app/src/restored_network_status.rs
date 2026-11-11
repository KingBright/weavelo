//! Session telemetry stays available when the remote-control toolbar is hidden.
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
                && s.updated_at.is_some_and(|at| at.elapsed() <= Duration::from_secs(5))
        });
        let rtt = live.and_then(|s| s.rtt).map(|rtt| format!("{:.0} ms", rtt.latest_ms));
        let route_text = route.filter(|r| !r.trim().is_empty()).map(localized_route_label);
        let base_label = rtt.clone().unwrap_or_else(|| quality.label().into());
        let label = match (&route_text, role.kind()) {
            (Some(r), remote_core::role::RoleKind::Viewing) => format!("{r} · {base_label}"),
            _ => base_label,
        };
        let rows = vec![
            (crate::i18n::tr("Connection").into(), quality.label().into()),
            (
                crate::i18n::tr("Route").into(),
                route_text.unwrap_or_else(|| "—".into()),
            ),
            (crate::i18n::tr("Network RTT").into(), rtt.unwrap_or_else(|| "—".into())),
            (
                crate::i18n::tr("Packet loss rate").into(),
                live.filter(|s| s.packet_loss_rate.is_finite() && (0.0..=100.0).contains(&s.packet_loss_rate))
                    .map(|s| format!("{:.1}%", s.packet_loss_rate))
                    .unwrap_or_else(|| "—".into()),
            ),
            (
                crate::i18n::tr("Bitrate").into(),
                live.map(|s| format_bitrate(s.bitrate_kbps)).unwrap_or_else(|| "—".into()),
            ),
        ];
        (label, quality, rows)
    }

    pub(super) fn network_status_chip(
        &self,
        role: &RoleState,
        route: Option<&str>,
        stats: Option<&UiHostStats>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let (label, quality, rows) = Self::network_status_details(role, route, stats);
        let detail = rows
            .iter()
            .map(|(name, value)| format!("{name}: {value}"))
            .collect::<Vec<_>>()
            .join("\n");
        div()
            .id("network-status-hit-area")
            .occlude()
            .on_any_mouse_down(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(
                command_button("network-status-chip", ActionVariantKind::Neutral, cx)
                    .h(px(28.))
                    .px_2()
                    .rounded_full()
                    .flex_none()
                    .bg(color_glass_card())
                    .border_1()
                    .border_color(color_border_fine())
                    .tooltip(tooltip(detail).build())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.network_details_open = !this.network_details_open;
                        this.reset_pointer_scroll();
                        this.owner.release_input();
                        this.pointer_input = PointerInputTracker::default();
                        window.focus(&this.root_focus);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_size(px(11.))
                            .whitespace_nowrap()
                            .child(network_quality_icon(quality, &cx.theme()).size(px(12.)))
                            .child(label),
                    ),
            )
    }

    pub(super) fn network_status_panel(
        &self,
        role: &RoleState,
        route: Option<&str>,
        stats: Option<&UiHostStats>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let (_, _, rows) = Self::network_status_details(role, route, stats);
        div()
            .id("network-status-details")
            .absolute()
            .right(px(12.))
            .bottom(px(48.))
            .w(px(250.))
            .max_w(relative(0.95))
            .p_3()
            .rounded(px(12.))
            .occlude()
            .bg(color_glass_card())
            .border_1()
            .border_color(color_border_fine())
            .shadow_lg()
            .on_any_mouse_down(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(crate::i18n::tr("Network details")),
                    )
                    .child(
                        command_button("close-network-status", ActionVariantKind::Neutral, cx)
                            .size(px(26.))
                            .child("×")
                            .tooltip(tooltip(crate::i18n::tr("Close")).build())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.network_details_open = false;
                                cx.notify();
                            })),
                    ),
            )
            .children(rows.into_iter().map(|(name, value)| {
                div()
                    .flex()
                    .justify_between()
                    .gap_2()
                    .mt_2()
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(cx.theme().content.secondary)
                            .child(name),
                    )
                    .child(div().text_size(px(11.)).child(value))
            }))
    }
}
