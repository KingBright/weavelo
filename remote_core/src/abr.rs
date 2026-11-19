//! Closed-loop delay-gradient and loss-driven adaptive bitrate (ABR) controller.
//!
//! Designed primarily for real-time screen streaming over TCP/WebSocket relays
//! and variable Wi-Fi links:
//! - Detects queueing delay growth (`smoothed_rtt - baseline_rtt`) *before*
//!   socket buffers fill and drop fragments.
//! - Reacts rapidly to sequence loss, decode-queue drops, and keyframe storms
//!   via multiplicative decrease, capped to a readable floor.
//! - Smoothly probes back toward the user's configured bitrate ceiling once
//!   queueing delay and loss remain low, never exceeding `user_max_kbps`.

use std::time::{Duration, Instant};

const MIN_FLOOR_KBPS: u32 = 800;
const MAX_FLOOR_KBPS: u32 = 1_500;
const FPS_STEP_DOWN_KBPS: u32 = 3_000;
const FPS_STEP_UP_KBPS: u32 = 4_500;
const SEVERE_QUEUE_DELAY_MS: f32 = 120.0;
const MODERATE_QUEUE_DELAY_MS: f32 = 55.0;
const HEALTHY_QUEUE_DELAY_MS: f32 = 25.0;
const SEVERE_LOSS_PCT: f32 = 3.0;
const MODERATE_LOSS_PCT: f32 = 1.0;
const HEALTHY_LOSS_PCT: f32 = 0.3;

const BASELINE_DRIFT_INTERVAL: Duration = Duration::from_secs(30);
const SEVERE_DECREASE_COOLDOWN: Duration = Duration::from_millis(800);
const MODERATE_DECREASE_COOLDOWN: Duration = Duration::from_millis(1_500);
const RECOVERY_HOLD_AFTER_CONGESTION: Duration = Duration::from_secs(3);
const INCREASE_INTERVAL: Duration = Duration::from_secs(2);
const MIN_EVAL_INTERVAL: Duration = Duration::from_millis(450);

#[derive(Debug, Clone, Copy)]
pub struct AbrObservation {
    pub now: Instant,
    pub rtt_ms: Option<f32>,
    pub loss_rate_pct: f32,
    pub host_bitrate_kbps: u32,
    pub decode_queue_drops_total: usize,
    pub keyframe_requests_total: usize,
}

#[derive(Debug, Clone)]
pub struct AdaptiveBitrateController {
    width: u32,
    height: u32,
    user_fps: u32,
    applied_fps: u32,
    user_max_kbps: u32,
    min_kbps: u32,
    target_kbps: u32,
    applied_kbps: u32,
    baseline_rtt_ms: Option<f32>,
    baseline_updated_at: Option<Instant>,
    smoothed_rtt_ms: Option<f32>,
    elevated_delay_streak: u8,
    last_eval_at: Option<Instant>,
    last_decrease_at: Option<Instant>,
    last_increase_at: Option<Instant>,
    last_congestion_at: Option<Instant>,
    last_decode_queue_drops: usize,
    last_keyframe_requests: usize,
    counters_initialized: bool,
}

impl AdaptiveBitrateController {
    pub fn new(width: u32, height: u32, fps: u32, user_max_kbps: u32) -> Self {
        let user_max_kbps = user_max_kbps.max(500);
        let min_kbps = compute_floor_kbps(user_max_kbps);
        Self {
            width,
            height,
            user_fps: fps,
            applied_fps: fps,
            user_max_kbps,
            min_kbps,
            target_kbps: user_max_kbps,
            applied_kbps: user_max_kbps,
            baseline_rtt_ms: None,
            baseline_updated_at: None,
            smoothed_rtt_ms: None,
            elevated_delay_streak: 0,
            last_eval_at: None,
            last_decrease_at: None,
            last_increase_at: None,
            last_congestion_at: None,
            last_decode_queue_drops: 0,
            last_keyframe_requests: 0,
            counters_initialized: false,
        }
    }

    /// Update the user's explicit stream settings (e.g. from Subscribe or user Configure).
    /// Immediately aligns the ABR ceiling and applied bitrate with the user's choice.
    pub fn set_user_profile(&mut self, width: u32, height: u32, fps: u32, bitrate_kbps: u32) {
        let bitrate_kbps = bitrate_kbps.max(500);
        self.width = width;
        self.height = height;
        self.user_fps = fps;
        self.applied_fps = fps;
        self.user_max_kbps = bitrate_kbps;
        self.min_kbps = compute_floor_kbps(bitrate_kbps);
        self.target_kbps = bitrate_kbps;
        self.applied_kbps = bitrate_kbps;
        self.elevated_delay_streak = 0;
        self.last_decrease_at = None;
        self.last_increase_at = None;
    }

    pub fn profile(&self) -> (u32, u32, u32, u32) {
        (self.width, self.height, self.applied_fps, self.applied_kbps)
    }

    pub fn target_bitrate_kbps(&self) -> u32 {
        self.target_kbps
    }

    pub fn applied_bitrate_kbps(&self) -> u32 {
        self.applied_kbps
    }

    pub fn user_max_kbps(&self) -> u32 {
        self.user_max_kbps
    }

    /// Reset transient queue-delay and counter baselines (e.g. after pause/resume or source switch).
    pub fn reset_measurements(&mut self) {
        self.baseline_rtt_ms = None;
        self.baseline_updated_at = None;
        self.smoothed_rtt_ms = None;
        self.elevated_delay_streak = 0;
        self.last_eval_at = None;
        self.counters_initialized = false;
    }

    /// Observe latest session metrics; returns `Some(new_bitrate_kbps)` when a
    /// new bitrate should be applied to the host via `SessionCommand::Configure`.
    pub fn observe(&mut self, obs: AbrObservation) -> Option<u32> {
        if !self.counters_initialized {
            self.last_decode_queue_drops = obs.decode_queue_drops_total;
            self.last_keyframe_requests = obs.keyframe_requests_total;
            self.counters_initialized = true;
        }

        if let Some(last_eval) = self.last_eval_at
            && obs.now.saturating_duration_since(last_eval) < MIN_EVAL_INTERVAL
        {
            return None;
        }
        self.last_eval_at = Some(obs.now);

        let drop_delta = obs
            .decode_queue_drops_total
            .saturating_sub(self.last_decode_queue_drops);
        let kf_delta = obs
            .keyframe_requests_total
            .saturating_sub(self.last_keyframe_requests);
        self.last_decode_queue_drops = obs.decode_queue_drops_total;
        self.last_keyframe_requests = obs.keyframe_requests_total;

        let (queue_delay_ms, raw_queue_delay_ms) = self.update_rtt(obs.now, obs.rtt_ms);
        let loss = if obs.loss_rate_pct.is_finite() && obs.loss_rate_pct >= 0.0 {
            obs.loss_rate_pct
        } else {
            0.0
        };

        let severe = loss >= SEVERE_LOSS_PCT
            || queue_delay_ms >= SEVERE_QUEUE_DELAY_MS
            || drop_delta > 0
            || kf_delta >= 2;

        if queue_delay_ms >= MODERATE_QUEUE_DELAY_MS
            && raw_queue_delay_ms >= MODERATE_QUEUE_DELAY_MS
        {
            self.elevated_delay_streak = self.elevated_delay_streak.saturating_add(1);
        } else {
            self.elevated_delay_streak = 0;
        }

        let moderate = !severe
            && (self.elevated_delay_streak >= 2
                || loss >= MODERATE_LOSS_PCT
                || kf_delta >= 1);

        if severe {
            self.last_congestion_at = Some(obs.now);
            self.elevated_delay_streak = 0;
            let cooldown_ok = self.last_decrease_at.is_none_or(|t| {
                obs.now.saturating_duration_since(t) >= SEVERE_DECREASE_COOLDOWN
            });
            if cooldown_ok {
                let next = ((u64::from(self.target_kbps) * 65) / 100) as u32;
                let next = round_bitrate(next.clamp(self.min_kbps, self.user_max_kbps), self.min_kbps, self.user_max_kbps);
                self.target_kbps = next;
                self.last_decrease_at = Some(obs.now);
                return self.maybe_commit();
            }
            return None;
        }

        if moderate {
            self.last_congestion_at = Some(obs.now);
            let cooldown_ok = self.last_decrease_at.is_none_or(|t| {
                obs.now.saturating_duration_since(t) >= MODERATE_DECREASE_COOLDOWN
            });
            if cooldown_ok {
                let next = ((u64::from(self.target_kbps) * 82) / 100) as u32;
                let next = round_bitrate(next.clamp(self.min_kbps, self.user_max_kbps), self.min_kbps, self.user_max_kbps);
                self.target_kbps = next;
                self.last_decrease_at = Some(obs.now);
                self.elevated_delay_streak = 0;
                return self.maybe_commit();
            }
            return None;
        }

        // Healthy network: smoothly ramp back up toward user ceiling.
        let healthy = obs.rtt_ms.is_some()
            && queue_delay_ms < HEALTHY_QUEUE_DELAY_MS
            && loss < HEALTHY_LOSS_PCT
            && self.target_kbps < self.user_max_kbps;
        if !healthy {
            return None;
        }

        let quiet_long_enough = self.last_congestion_at.is_none_or(|t| {
            obs.now.saturating_duration_since(t) >= RECOVERY_HOLD_AFTER_CONGESTION
        });
        let increase_due = self.last_increase_at.is_none_or(|t| {
            obs.now.saturating_duration_since(t) >= INCREASE_INTERVAL
        });
        if !quiet_long_enough || !increase_due {
            return None;
        }

        // If stream is actively consuming bandwidth (or still below half ceiling),
        // step up conservatively so recovery never triggers another burst collapse.
        let active_usage = obs.host_bitrate_kbps >= (self.target_kbps / 3)
            || self.target_kbps < (self.user_max_kbps / 2);
        let step = if active_usage {
            ((u64::from(self.target_kbps) * 12) / 100).max(500) as u32
        } else {
            ((u64::from(self.target_kbps) * 8) / 100).max(300) as u32
        };
        let next = self.target_kbps.saturating_add(step).min(self.user_max_kbps);
        let next = round_bitrate(next, self.min_kbps, self.user_max_kbps);
        self.target_kbps = next;
        self.last_increase_at = Some(obs.now);
        self.maybe_commit()
    }

    fn update_rtt(&mut self, now: Instant, rtt_ms: Option<f32>) -> (f32, f32) {
        let Some(rtt) = rtt_ms.filter(|v| v.is_finite() && *v >= 0.0) else {
            return (0.0, 0.0);
        };

        // Asymmetric EWMA: smooth rising spikes to avoid jitter false alarms,
        // decay quickly when RTT falls so draining queues never double-penalize.
        let smoothed = match self.smoothed_rtt_ms {
            Some(prev) if rtt < prev => prev * 0.30 + rtt * 0.70,
            Some(prev) => prev * 0.55 + rtt * 0.45,
            None => rtt,
        };
        self.smoothed_rtt_ms = Some(smoothed);

        match self.baseline_rtt_ms {
            None => {
                self.baseline_rtt_ms = Some(rtt);
                self.baseline_updated_at = Some(now);
                (0.0, 0.0)
            }
            Some(base) => {
                if rtt <= base {
                    self.baseline_rtt_ms = Some(rtt);
                    self.baseline_updated_at = Some(now);
                    (0.0, 0.0)
                } else {
                    // Gently relax an old baseline after 30s in case the network route genuinely shifted.
                    let active_base = if self.baseline_updated_at.is_some_and(|t| {
                        now.saturating_duration_since(t) >= BASELINE_DRIFT_INTERVAL
                    }) {
                        let relaxed = base + (smoothed - base) * 0.15;
                        self.baseline_rtt_ms = Some(relaxed);
                        self.baseline_updated_at = Some(now);
                        relaxed
                    } else {
                        base
                    };
                    ((smoothed - active_base).max(0.0), (rtt - active_base).max(0.0))
                }
            }
        }
    }

    fn desired_fps(&self) -> u32 {
        if self.user_fps <= 30 {
            return self.user_fps;
        }
        if self.target_kbps <= FPS_STEP_DOWN_KBPS {
            30
        } else if self.target_kbps >= FPS_STEP_UP_KBPS {
            self.user_fps
        } else {
            self.applied_fps
        }
    }

    fn maybe_commit(&mut self) -> Option<u32> {
        let next_fps = self.desired_fps();
        let fps_changed = next_fps != self.applied_fps;
        if self.target_kbps == self.applied_kbps && !fps_changed {
            return None;
        }
        let diff = self.target_kbps.abs_diff(self.applied_kbps);
        let min_step = (self.applied_kbps / 10).max(400);
        let hits_bound = self.target_kbps == self.user_max_kbps || self.target_kbps == self.min_kbps;
        if diff >= min_step || hits_bound || fps_changed {
            self.applied_kbps = self.target_kbps;
            self.applied_fps = next_fps;
            Some(self.applied_kbps)
        } else {
            None
        }
    }
}

fn compute_floor_kbps(user_max_kbps: u32) -> u32 {
    (user_max_kbps / 10)
        .clamp(MIN_FLOOR_KBPS, MAX_FLOOR_KBPS)
        .min(user_max_kbps)
        .max(500)
}

fn round_bitrate(kbps: u32, min_kbps: u32, max_kbps: u32) -> u32 {
    if kbps <= min_kbps {
        return min_kbps;
    }
    if kbps >= max_kbps {
        return max_kbps;
    }
    let rounded = ((kbps + 50) / 100) * 100;
    rounded.clamp(min_kbps, max_kbps)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(
        now: Instant,
        rtt_ms: Option<f32>,
        loss_rate_pct: f32,
        host_bitrate_kbps: u32,
        drops: usize,
        kfs: usize,
    ) -> AbrObservation {
        AbrObservation {
            now,
            rtt_ms,
            loss_rate_pct,
            host_bitrate_kbps,
            decode_queue_drops_total: drops,
            keyframe_requests_total: kfs,
        }
    }

    #[test]
    fn healthy_link_emits_no_configure_churn() {
        let mut abr = AdaptiveBitrateController::new(1920, 1080, 60, 20_000);
        let start = Instant::now();
        for step in 0..20 {
            let t = start + Duration::from_millis(step * 500);
            assert_eq!(abr.observe(obs(t, Some(22.0), 0.0, 14_000, 0, 0)), None);
        }
        assert_eq!(abr.applied_bitrate_kbps(), 20_000);
    }

    #[test]
    fn tcp_queueing_delay_reduces_bitrate_before_packet_loss_occurs() {
        let mut abr = AdaptiveBitrateController::new(1920, 1080, 60, 20_000);
        let start = Instant::now();
        // Establish baseline RTT ~25ms
        assert_eq!(abr.observe(obs(start, Some(25.0), 0.0, 18_000, 0, 0)), None);
        // Queue builds on TCP relay: RTT jumps to 200ms (0% loss!)
        let t1 = start + Duration::from_millis(600);
        assert_eq!(abr.observe(obs(t1, Some(200.0), 0.0, 18_000, 0, 0)), None); // smoothed ~103.75 -> queue delay ~78.75ms (streak 1)
        let t2 = start + Duration::from_millis(1200);
        let reduced = abr
            .observe(obs(t2, Some(210.0), 0.0, 18_000, 0, 0))
            .expect("sustained queue delay should cut bitrate before loss");
        assert!(reduced < 18_000, "expected drop, got {reduced}");
        assert!(reduced >= 1_500);
    }

    #[test]
    fn severe_loss_and_drops_cut_rapidly_and_respect_floor() {
        let mut abr = AdaptiveBitrateController::new(1920, 1080, 60, 10_000);
        let mut now = Instant::now();
        let mut drops = 0;
        let mut last = 10_000;
        for _ in 0..10 {
            now += Duration::from_secs(1);
            drops += 1;
            if let Some(next) = abr.observe(obs(now, Some(300.0), 8.0, last, drops, 2)) {
                assert!(next < last);
                last = next;
            }
        }
        assert_eq!(abr.applied_bitrate_kbps(), 1_000);
    }

    #[test]
    fn recovers_smoothly_toward_user_ceiling_after_congestion_clears() {
        let mut abr = AdaptiveBitrateController::new(1920, 1080, 60, 10_000);
        let mut now = Instant::now();
        abr.observe(obs(now, Some(20.0), 0.0, 9_000, 0, 0));
        now += Duration::from_secs(1);
        let dropped = abr
            .observe(obs(now, Some(320.0), 5.0, 9_000, 1, 2))
            .expect("severe congestion must drop bitrate");
        assert!(dropped <= 7_000);

        // Link becomes healthy again
        let mut saw_increase = false;
        for _ in 0..15 {
            now += Duration::from_secs(2);
            if let Some(next) = abr.observe(obs(now, Some(22.0), 0.0, abr.applied_bitrate_kbps(), 1, 2)) {
                assert!(next > dropped);
                assert!(next <= 10_000);
                saw_increase = true;
            }
        }
        assert!(saw_increase);
        assert_eq!(abr.applied_bitrate_kbps(), 10_000);
    }

    #[test]
    fn user_profile_update_resets_ceiling_and_target() {
        let mut abr = AdaptiveBitrateController::new(1920, 1080, 60, 20_000);
        let mut now = Instant::now();
        abr.observe(obs(now, Some(20.0), 0.0, 18_000, 0, 0));
        now += Duration::from_secs(1);
        assert!(abr.observe(obs(now, Some(400.0), 10.0, 18_000, 1, 2)).is_some());
        assert!(abr.applied_bitrate_kbps() < 20_000);

        abr.set_user_profile(1280, 720, 30, 6_000);
        assert_eq!(abr.profile(), (1280, 720, 30, 6_000));
        assert_eq!(abr.user_max_kbps(), 6_000);
    }

    #[test]
    fn severe_congestion_steps_60fps_down_to_30fps_and_restores_on_recovery() {
        let mut abr = AdaptiveBitrateController::new(1920, 1080, 60, 10_000);
        let mut now = Instant::now();
        abr.observe(obs(now, Some(20.0), 0.0, 9_000, 0, 0));
        let mut drops = 0;
        for _ in 0..5 {
            now += Duration::from_secs(1);
            drops += 1;
            abr.observe(obs(now, Some(300.0), 6.0, abr.applied_bitrate_kbps(), drops, 2));
        }
        assert!(abr.applied_bitrate_kbps() <= FPS_STEP_DOWN_KBPS);
        assert_eq!(abr.profile().2, 30, "expected 60fps -> 30fps under severe weak relay");

        for _ in 0..25 {
            now += Duration::from_secs(2);
            abr.observe(obs(now, Some(22.0), 0.0, abr.applied_bitrate_kbps(), drops, 2));
        }
        assert_eq!(abr.applied_bitrate_kbps(), 10_000);
        assert_eq!(abr.profile().2, 60, "expected fps to restore to 60 after recovery");
    }
}
