//! Presentation-only telemetry state. Never infer availability from legacy zeroes.
use remote_core::{
    HostStats, SharedHostStats,
    clock_sync::{ClockEstimate, ClockSyncState},
};
use std::ops::Deref;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NetworkQuality { Connecting, Disconnected, Unknown, Good, Fair, Poor }

impl NetworkQuality {
    pub(crate) fn from_session(role:remote_core::role::RoleKind,stats:Option<&HostStats>)->Self {
        use remote_core::role::RoleKind;
        match role {
            RoleKind::Connecting=>return Self::Connecting,
            RoleKind::Viewing=>{},
            _=>return Self::Disconnected,
        }
        let Some(stats)=stats else {return Self::Unknown;};
        if !stats.updated_at.is_some_and(|at|at.elapsed()<=std::time::Duration::from_secs(5))
            || stats.link_status.is_empty() || !stats.rtt_ms.is_finite() || stats.rtt_ms<0.
            || !stats.packet_loss_rate.is_finite() || !(0.0..=100.0).contains(&stats.packet_loss_rate) {
            return Self::Unknown;
        }
        // Host encode/decode time and desktop damage FPS do not measure the network.
        if stats.rtt_ms>150. || stats.packet_loss_rate>=5. {Self::Poor}
        else if stats.rtt_ms>80. || stats.packet_loss_rate>=1. {Self::Fair}
        else {Self::Good}
    }
    pub(crate) fn label(self)->&'static str {match self {
        Self::Connecting=>crate::i18n::tr("Connecting"),Self::Disconnected=>crate::i18n::tr("Not connected"),Self::Unknown=>crate::i18n::tr("Network unknown"),
        Self::Good=>crate::i18n::tr("Network good"),Self::Fair=>crate::i18n::tr("Network fair"),Self::Poor=>crate::i18n::tr("Network poor"),
    }}
}

/// Taking a decoded frame into the UI is only a submission estimate. No GPU
/// completion or VSync callback has occurred here, so presentation stays unknown.
pub(crate) fn mark_ui_frame_submission(
    timing: &mut protocol::FrameTimingCheckpoints,
    elapsed_since_decode: std::time::Duration,
) {
    timing.render_submit_ts_us = 0;
    timing.render_done_ts_us = 0;
    if timing.capture_ts_us == 0 || timing.recv_ts_us == 0 || timing.decode_done_ts_us == 0 {
        return;
    }
    let floor = timing.decode_done_ts_us;
    if floor < timing.recv_ts_us || floor < timing.jitter_exit_ts_us || floor < timing.decode_enter_ts_us {
        return;
    }
    timing.render_submit_ts_us = u32::try_from(elapsed_since_decode.as_micros())
        .ok()
        .and_then(|elapsed| floor.checked_add(elapsed))
        .filter(|submit| *submit < u32::MAX)
        .unwrap_or(0);
}

pub(crate) struct UiHostStats {
    host: HostStats,
    pub(crate) capture_receive: CaptureReceiveDisplay,
    pub(crate) capture_submit: CaptureReceiveDisplay,
    pub(crate) rtt: Option<remote_core::client_session::RttSummary>,
}

impl UiHostStats {
    pub(crate) fn read(shared: &SharedHostStats) -> Self {
        let host = shared.snapshot();
        let clock = shared.clock_estimate();
        Self {
            rtt: shared.rtt_summary(),
            capture_submit: CaptureReceiveDisplay::from_measurements(
                shared.latest_ui_submission()
                    .map(|timing|timing.render_submit_ts_us as f32/1000.0),
                clock.clone(),
            ),
            capture_receive: CaptureReceiveDisplay::from_measurements(
                shared.estimated_e2e_latency_ms(),
                clock,
            ),
            host,
        }
    }
    pub(crate) fn network_quality(role: remote_core::role::RoleKind, stats: Option<&Self>) -> NetworkQuality {
        if role == remote_core::role::RoleKind::Viewing && stats.is_none_or(|stats| stats.rtt.is_none()) {
            if let Some(stats) = stats
                && stats.updated_at.is_some_and(|at| at.elapsed() <= std::time::Duration::from_secs(5))
                && stats.packet_loss_rate.is_finite() && stats.packet_loss_rate <= 100.0 {
                if stats.packet_loss_rate >= 5.0 { return NetworkQuality::Poor; }
                if stats.packet_loss_rate >= 1.0 { return NetworkQuality::Fair; }
            }
            return NetworkQuality::Unknown;
        }
        NetworkQuality::from_session(role, stats.map(|stats| &stats.host))
    }

    pub(crate) fn network_detail(quality: NetworkQuality, stats: Option<&Self>) -> String {
        let Some(stats) = stats else { return quality.label().into(); };
        match quality {
            NetworkQuality::Good | NetworkQuality::Fair | NetworkQuality::Poor => {
                let rtt = stats.rtt.map(|rtt| format!("{:.0} ms", rtt.latest_ms))
                    .unwrap_or_else(|| crate::i18n::tr("unavailable").into());
                crate::i18n::tr_format!("{} · RTT {} · loss {:.1}%", quality.label(), rtt, stats.packet_loss_rate)
            }
            _ => quality.label().into(),
        }
    }

    pub(crate) fn rtt_value(&self) -> String {
        self.rtt.map(|rtt| format!("{:.1} ms", rtt.median_ms)).unwrap_or_else(|| "—".into())
    }
    pub(crate) fn rtt_detail(&self) -> String {
        self.rtt.map(|rtt| crate::i18n::tr_format!(
            "Median of {} recent probes. Latest {:.1} ms; range {:.1}–{:.1} ms. Excludes measured host processing; includes transport and send scheduling. Expires after 10 s.",
            rtt.samples, rtt.latest_ms, rtt.min_ms, rtt.max_ms))
            .unwrap_or_else(|| crate::i18n::tr("No fresh round-trip measurement.").into())
    }

}

impl Deref for UiHostStats {
    type Target = HostStats;

    fn deref(&self) -> &Self::Target {
        &self.host
    }
}

pub(crate) struct CaptureReceiveDisplay {
    latency_ms: Option<f32>,
    uncertainty_ms: Option<f64>,
    clock_state: Option<ClockSyncState>,
}

impl CaptureReceiveDisplay {
    fn from_measurements(latency_ms: Option<f32>, clock: Option<ClockEstimate>) -> Self {
        let clock_state = clock.as_ref().map(|estimate| estimate.state);
        let latency_ms = latency_ms.filter(|latency| {
            clock_state == Some(ClockSyncState::Synchronized)
                && latency.is_finite()
                && *latency >= 0.0
        });
        let uncertainty_ms = clock
            .and_then(|estimate| estimate.uncertainty_us())
            .filter(|uncertainty| uncertainty.is_finite() && *uncertainty >= 0.0)
            .map(|uncertainty| uncertainty / 1000.0);
        Self {
            latency_ms,
            uncertainty_ms,
            clock_state,
        }
    }

    pub(crate) fn value(&self) -> String {
        match (self.latency_ms, self.uncertainty_ms) {
            (Some(latency), Some(uncertainty)) => {
                format!("≈ {latency:.1} ± {uncertainty:.1} ms")
            }
            (Some(latency), None) => format!("≈ {latency:.1} ms"),
            (None, _) => "—".into(),
        }
    }

    pub(crate) fn compact_value(&self) -> String {
        self.latency_ms.map(|latency| format!("≈ {latency:.1} ms"))
            .unwrap_or_else(|| "—".into())
    }

    pub(crate) fn submission_detail(&self) -> String {
        self.explain(crate::i18n::tr("Latest completed host capture to UI scene submission sample (expires after 5 s), including transport, decode and UI waiting. GPU completion and physical display remain unmeasured."))
    }
    pub(crate) fn detail(&self) -> String {
        self.explain(crate::i18n::tr("Host capture to client receive estimate; excludes decoding and display."))
    }
    fn explain(&self,scope:&str) -> String {
        if self.latency_ms.is_some() {
            let uncertainty = self.uncertainty_ms.map_or_else(
                || crate::i18n::tr("Model clock uncertainty unavailable.").to_string(),
                |uncertainty| crate::i18n::tr_format!(
                    "Model clock uncertainty ± {uncertainty:.1} ms; not a guaranteed accuracy bound.", uncertainty=uncertainty
                ),
            );
            crate::i18n::tr_format!("{scope} {uncertainty}", scope=scope, uncertainty=uncertainty)
        } else {
            let reason = match self.clock_state {
                None => crate::i18n::tr("awaiting clock measurements"),
                Some(ClockSyncState::Acquiring) => crate::i18n::tr("acquiring clock mapping"),
                Some(ClockSyncState::Stale) => crate::i18n::tr("clock mapping expired"),
                Some(ClockSyncState::Synchronized) => crate::i18n::tr("awaiting a valid capture/receive sample"),
            };
            crate::i18n::tr_format!("{scope} Unavailable: {reason}.", scope=scope, reason=reason)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_indicator_obeys_connection_and_measurement_freshness() {
        use remote_core::role::RoleKind;
        let mut stats=HostStats {updated_at:Some(std::time::Instant::now()),..Default::default()};
        assert_eq!(NetworkQuality::from_session(RoleKind::Viewing,Some(&stats)),NetworkQuality::Unknown);
        stats.link_status="measured";stats.rtt_ms=12.;
        assert_eq!(NetworkQuality::from_session(RoleKind::Viewing,Some(&stats)),NetworkQuality::Good);
        assert_eq!(NetworkQuality::from_session(RoleKind::Connecting,Some(&stats)),NetworkQuality::Connecting);
        assert_eq!(NetworkQuality::from_session(RoleKind::Idle,Some(&stats)),NetworkQuality::Disconnected);
        stats.updated_at=Some(std::time::Instant::now()-std::time::Duration::from_secs(6));
        assert_eq!(NetworkQuality::from_session(RoleKind::Viewing,Some(&stats)),NetworkQuality::Unknown);
    }

    #[test]
    fn network_quality_uses_rtt_and_percentage_loss_instead_of_encode_or_damage_fps() {
        use remote_core::role::RoleKind;
        let mut stats=HostStats {updated_at:Some(std::time::Instant::now()),link_status:"measured",rtt_ms:20.,
            fps:0.,latency:300.,jitter:150.,..Default::default()};
        assert_eq!(NetworkQuality::from_session(RoleKind::Viewing,Some(&stats)),NetworkQuality::Good);
        stats.rtt_ms=90.;
        assert_eq!(NetworkQuality::from_session(RoleKind::Viewing,Some(&stats)),NetworkQuality::Fair);
        stats.rtt_ms=20.;stats.packet_loss_rate=6.;
        assert_eq!(NetworkQuality::from_session(RoleKind::Viewing,Some(&stats)),NetworkQuality::Poor);
        stats.packet_loss_rate=f32::NAN;
        assert_eq!(NetworkQuality::from_session(RoleKind::Viewing,Some(&stats)),NetworkQuality::Unknown);
    }
    use remote_core::clock_sync::ClockSynchronizer;

    #[test]
    fn ui_submission_never_fabricates_presentation_or_wraps_invalid_timing() {
        let mut timing = protocol::FrameTimingCheckpoints::new(1_000_000);
        timing.recv_ts_us = 10_000;
        timing.jitter_exit_ts_us = 11_000;
        timing.decode_done_ts_us = 13_000;
        timing.render_done_ts_us = 99_000;
        mark_ui_frame_submission(&mut timing, std::time::Duration::from_micros(2_000));
        assert_eq!(timing.render_submit_ts_us, 15_000);
        assert_eq!(timing.render_done_ts_us, 0);
        assert_eq!(timing.total_e2e_duration_us(), None);
        assert_eq!(timing.render_presentation_duration_us(), None);
        for (capture, decoded, elapsed) in [
            (0, 13_000, 2_000),
            (1_000_000, 0, 2_000),
            (1_000_000, 9_000, 2_000),
            (1_000_000, u32::MAX, 2_000),
            (1_000_000, u32::MAX, 0),
            (1_000_000, 13_000, u64::MAX),
        ] {
            timing.capture_ts_us = capture;
            timing.decode_done_ts_us = decoded;
            mark_ui_frame_submission(&mut timing, std::time::Duration::from_micros(elapsed));
            assert_eq!(timing.render_submit_ts_us, 0);
            assert_eq!(timing.render_done_ts_us, 0);
        }
        timing.capture_ts_us = 1_000_000;
        timing.decode_done_ts_us = 13_000;
        timing.decode_enter_ts_us = 14_000;
        mark_ui_frame_submission(&mut timing, std::time::Duration::ZERO);
        assert_eq!(timing.render_submit_ts_us, 0);
        assert_eq!(timing.render_done_ts_us, 0);
    }

    fn synchronized_clock() -> ClockEstimate {
        let mut clock = ClockSynchronizer::new();
        for probe_id in 1..=4 {
            let sent = probe_id * 1_000_000;
            clock.begin_probe(probe_id, sent).unwrap();
            clock
                .complete_probe(probe_id, sent, sent + 1500, sent + 1600, sent + 1100)
                .unwrap();
        }
        let estimate = clock.estimate(4_001_100).unwrap();
        assert_eq!(estimate.state, ClockSyncState::Synchronized);
        estimate
    }

    #[test]
    fn missing_acquiring_and_expired_measurements_are_unavailable() {
        let missing = CaptureReceiveDisplay::from_measurements(Some(12.5), None);
        assert_eq!(missing.value(), "—");
        assert!(missing.detail().contains("awaiting clock measurements"));
        for (state, reason) in [
            (ClockSyncState::Acquiring, "acquiring clock mapping"),
            (ClockSyncState::Stale, "clock mapping expired"),
        ] {
            let mut clock = synchronized_clock();
            clock.state = state;
            let display = CaptureReceiveDisplay::from_measurements(Some(12.5), Some(clock));
            assert_eq!(display.value(), "—");
            assert!(display.detail().contains(reason));
        }
    }

    #[test]
    fn fresh_estimate_shows_model_uncertainty_and_limited_scope() {
        let clock = synchronized_clock();
        let uncertainty_ms = clock.uncertainty_us().unwrap() / 1000.0;
        let display = CaptureReceiveDisplay::from_measurements(Some(12.5), Some(clock));
        assert_eq!(display.value(), format!("≈ 12.5 ± {uncertainty_ms:.1} ms"));
        assert!(display.detail().contains("excludes decoding and display"));
        assert!(display.detail().contains("not a guaranteed accuracy bound"));
    }

    #[test]
    fn valid_zero_is_distinct_from_missing_without_clamping_invalid_values() {
        let clock = synchronized_clock();
        let zero = CaptureReceiveDisplay::from_measurements(Some(0.0), Some(clock));
        assert!(zero.value().starts_with("≈ 0.0"));
        for latency in [None, Some(-1.0), Some(f32::NAN), Some(f32::INFINITY)] {
            let display = CaptureReceiveDisplay::from_measurements(latency, Some(clock));
            assert_eq!(display.value(), "—");
        }
    }

    #[test]
    fn unknown_uncertainty_is_not_reported_as_zero_error() {
        let mut clock = synchronized_clock();
        clock.fit_ci95_us = None;
        let display = CaptureReceiveDisplay::from_measurements(Some(12.5), Some(clock));
        assert_eq!(display.value(), "≈ 12.5 ms");
        assert!(display.detail().contains("uncertainty unavailable"));
    }

    #[test]
    fn missing_rtt_is_unknown_without_hiding_fresh_loss_warnings() {
        let shared = SharedHostStats::default();
        let mut display = UiHostStats::read(&shared);
        display.host.updated_at = Some(std::time::Instant::now());
        display.host.link_status = "healthy";
        assert_eq!(display.rtt_value(), "—");
        assert_eq!(UiHostStats::network_quality(remote_core::role::RoleKind::Viewing, Some(&display)), NetworkQuality::Unknown);
        display.host.packet_loss_rate = 6.;
        assert!(UiHostStats::network_detail(NetworkQuality::Poor, Some(&display)).contains("RTT unavailable"));
        assert_eq!(UiHostStats::network_quality(remote_core::role::RoleKind::Viewing, Some(&display)), NetworkQuality::Poor);
        display.host.updated_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(6));
        assert_eq!(UiHostStats::network_quality(remote_core::role::RoleKind::Viewing, Some(&display)), NetworkQuality::Unknown);
    }

    #[test]
    fn default_shared_stats_do_not_turn_unknown_into_zero_latency() {
        let shared = SharedHostStats::default();
        let display = UiHostStats::read(&shared);
        assert_eq!(display.capture_receive.value(), "—");
        assert!(display.capture_receive.detail().contains("Unavailable"));
    }
}
