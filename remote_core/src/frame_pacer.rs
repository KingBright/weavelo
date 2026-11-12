/// Select frames on a target timeline instead of rounding every interval up to
/// a whole display refresh. This permits e.g. 37 FPS from a 60 Hz capture source.
pub struct FramePacer {
    period_us: u64,
    next_due_us: Option<u64>,
}

impl FramePacer {
    pub fn new(fps: u32) -> Self {
        Self {
            period_us: (1_000_000 / u64::from(fps.max(1))).max(1),
            next_due_us: None,
        }
    }

    pub fn reset(&mut self, fps: u32) {
        *self = Self::new(fps);
    }

    /// A conditional redraw must not consume a slot until it is submitted.
    pub fn ready(&self, timestamp_us: u64) -> bool {
        self.next_due_us
            .is_none_or(|due| timestamp_us.saturating_add(self.period_us / 2) >= due)
    }

    pub fn admit(&mut self, timestamp_us: u64) -> bool {
        let due = self.next_due_us.unwrap_or(timestamp_us);
        let latest = timestamp_us.saturating_add(self.period_us / 2);
        if latest < due {
            return false;
        }
        // Select the nearest available capture frame. Advance the timeline past
        // late frames rather than producing a catch-up burst after a stall.
        let periods = (latest - due) / self.period_us + 1;
        self.next_due_us = Some(due.saturating_add(periods.saturating_mul(self.period_us)));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn viewport_redraws_and_capture_share_one_budget() {
        for fps in [7, 30, 47] {
            let mut pacer = FramePacer::new(fps);
            let mut frames = 0u32;
            // 60 Hz capture plus 20 Hz viewport/keyframe refresh. Neither path
            // may create an extra encoder budget; checking idle redraws is free.
            for step in 0..6000 {
                let now = step * 1_000_000 / 600;
                if step % 10 == 0 && pacer.admit(now) {
                    frames += 1;
                }
                if step % 30 == 0 && pacer.ready(now) {
                    assert!(pacer.ready(now));
                    assert!(pacer.admit(now));
                    frames += 1;
                }
            }
            assert!(frames.abs_diff(fps * 10) <= 1, "{fps}: {frames}");
        }
    }
    #[test]
    fn arbitrary_rates_do_not_round_down_to_display_divisors() {
        for fps in [1, 7, 24, 29, 37, 47, 59, 60, 120] {
            let mut pacer = FramePacer::new(fps);
            let emitted = (0..600).filter(|n| pacer.admit(n * 1_000_000 / 60)).count();
            let expected = fps.min(60) as usize * 10;
            assert!(
                emitted.abs_diff(expected) <= 1,
                "{fps}: {emitted}, expected {expected}"
            );
        }
    }
    #[test]
    fn gaps_and_rate_changes_do_not_create_a_catch_up_burst() {
        let mut pacer = FramePacer::new(30);
        assert!(pacer.admit(0));
        assert!(pacer.admit(20_000_000));
        assert!(!pacer.admit(20_000_001));
        pacer.reset(37);
        assert!(pacer.admit(20_000_002));
    }
    #[test]
    fn delayed_camera_callbacks_are_paced_by_source_time() {
        // A 20 Hz camera may deliver two samples together. Their callback
        // arrival times are not the source's frame intervals.
        let mut source = FramePacer::new(30);
        let mut arrival = FramePacer::new(30);
        let emitted_source = (0..200).filter(|n| source.admit(n * 50_000)).count();
        let emitted_arrival = (0..200)
            .filter(|n| arrival.admit((n / 2) * 100_000))
            .count();
        assert_eq!(emitted_source, 200);
        assert_eq!(emitted_arrival, 100);
    }
}
