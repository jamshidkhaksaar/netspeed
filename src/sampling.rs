use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
struct Snapshot {
    luid: u64,
    incoming: u64,
    outgoing: u64,
    at: Instant,
    interval_ms: u32,
}

#[derive(Default)]
pub struct Sampler {
    previous: Option<Snapshot>,
    pub down: f64,
    pub up: f64,
    pub total_down: u64,
    pub total_up: u64,
}

impl Sampler {
    pub fn reset(&mut self) {
        self.previous = None;
        self.down = 0.0;
        self.up = 0.0;
    }

    /// True only when a fresh, comparable sample is available for CSV logging.
    pub fn update(
        &mut self,
        luid: u64,
        incoming: u64,
        outgoing: u64,
        now: Instant,
        interval_ms: u32,
    ) -> bool {
        let current = Snapshot {
            luid,
            incoming,
            outgoing,
            at: now,
            interval_ms: interval_ms.clamp(250, 60_000),
        };
        let Some(previous) = self.previous else {
            self.previous = Some(current);
            return false;
        };
        let elapsed = now.saturating_duration_since(previous.at);
        if previous.luid == luid && elapsed < Duration::from_millis(50) {
            // Display/theme messages can trigger an extra tick. Keep the original
            // baseline so bytes are not discarded by a too-short interval.
            return false;
        }
        self.previous = Some(current);
        // A long interval may have been shortened since the baseline was taken.
        // Honor either interval for this delta, then use the new one thereafter.
        let max_gap =
            Duration::from_millis(u64::from(previous.interval_ms.max(current.interval_ms)) * 3)
                .max(Duration::from_secs(30));
        if previous.luid != luid
            || incoming < previous.incoming
            || outgoing < previous.outgoing
            || elapsed > max_gap
        {
            self.down = 0.0;
            self.up = 0.0;
            return false;
        }
        let incoming = incoming - previous.incoming;
        let outgoing = outgoing - previous.outgoing;
        self.total_down = self.total_down.saturating_add(incoming);
        self.total_up = self.total_up.saturating_add(outgoing);
        self.down = incoming as f64 / elapsed.as_secs_f64();
        self.up = outgoing as f64 / elapsed.as_secs_f64();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_rates_and_totals_from_one_adapter() {
        let now = Instant::now();
        let mut s = Sampler::default();
        assert!(!s.update(1, 100, 200, now, 1000));
        assert!(s.update(1, 1100, 700, now + Duration::from_secs(2), 1000));
        assert_eq!((s.down, s.up), (500.0, 250.0));
        assert_eq!((s.total_down, s.total_up), (1000, 500));
    }

    #[test]
    fn route_change_rebaselines_instead_of_spiking() {
        let now = Instant::now();
        let mut s = Sampler::default();
        s.update(1, 100, 100, now, 1000);
        assert!(!s.update(2, 9_000_000, 8_000_000, now + Duration::from_secs(1), 1000));
        assert_eq!((s.down, s.up, s.total_down, s.total_up), (0.0, 0.0, 0, 0));
        assert!(s.update(2, 9_001_000, 8_000_100, now + Duration::from_secs(2), 1000));
        assert_eq!((s.down, s.up), (1000.0, 100.0));
    }

    #[test]
    fn counter_reset_clears_old_rates() {
        let now = Instant::now();
        let mut s = Sampler::default();
        s.update(1, 100, 100, now, 1000);
        s.update(1, 1100, 200, now + Duration::from_secs(1), 1000);
        assert!(!s.update(1, 0, 0, now + Duration::from_secs(2), 1000));
        assert_eq!((s.down, s.up), (0.0, 0.0));
        assert_eq!((s.total_down, s.total_up), (1000, 100));
    }

    #[test]
    fn reset_on_pause_disconnect_or_resume_clears_stale_rates() {
        let now = Instant::now();
        let mut s = Sampler::default();
        s.update(1, 0, 0, now, 1000);
        s.update(1, 1000, 100, now + Duration::from_secs(1), 1000);
        s.reset();
        assert_eq!((s.down, s.up), (0.0, 0.0));
        assert!(!s.update(1, 5000, 500, now + Duration::from_secs(2), 1000));
        assert_eq!((s.total_down, s.total_up), (1000, 100));
    }

    #[test]
    fn custom_sixty_second_interval_is_supported() {
        let now = Instant::now();
        let mut s = Sampler::default();
        s.update(1, 0, 0, now, 60_000);
        assert!(s.update(1, 61_000, 6100, now + Duration::from_secs(61), 60_000));
        assert_eq!((s.down, s.up), (1000.0, 100.0));
    }

    #[test]
    fn quick_extra_tick_does_not_discard_bytes() {
        let now = Instant::now();
        let mut s = Sampler::default();
        s.update(1, 0, 0, now, 1000);
        assert!(!s.update(1, 100, 10, now + Duration::from_millis(10), 1000));
        assert!(s.update(1, 1000, 100, now + Duration::from_secs(1), 1000));
        assert_eq!((s.total_down, s.total_up), (1000, 100));
    }

    #[test]
    fn interval_increase_preserves_rates_and_all_session_bytes() {
        let now = Instant::now();
        let mut s = Sampler::default();
        s.update(1, 0, 0, now, 1000);
        s.update(1, 1000, 100, now + Duration::from_secs(1), 1000);
        // An interval-only menu change at 1.5 seconds must not reset the sampler.
        assert!(s.update(1, 6500, 650, now + Duration::from_millis(6500), 5000));
        assert_eq!((s.down, s.up), (1000.0, 100.0));
        assert_eq!((s.total_down, s.total_up), (6500, 650));
        assert!(s.update(1, 11_500, 1150, now + Duration::from_millis(11_500), 5000));
        assert_eq!((s.total_down, s.total_up), (11_500, 1150));
    }

    #[test]
    fn shortening_a_custom_long_interval_keeps_the_original_baseline() {
        let now = Instant::now();
        let mut s = Sampler::default();
        s.update(1, 0, 0, now, 60_000);
        assert!(s.update(1, 50_250, 5025, now + Duration::from_millis(50_250), 250));
        assert_eq!((s.down, s.up), (1000.0, 100.0));
        assert_eq!((s.total_down, s.total_up), (50_250, 5025));
        // The old interval is only allowed for the transition, not future gaps.
        assert!(!s.update(1, 81_250, 8125, now + Duration::from_millis(81_250), 250));
        assert_eq!((s.down, s.up), (0.0, 0.0));
        assert_eq!((s.total_down, s.total_up), (50_250, 5025));
    }

    #[test]
    fn increasing_to_a_custom_long_interval_uses_the_new_gap_limit() {
        let now = Instant::now();
        let mut s = Sampler::default();
        s.update(1, 0, 0, now, 250);
        assert!(s.update(1, 60_000, 6000, now + Duration::from_secs(60), 60_000));
        assert_eq!((s.down, s.up), (1000.0, 100.0));
        assert_eq!((s.total_down, s.total_up), (60_000, 6000));
    }

    #[test]
    fn long_sleep_gap_rebaselines_without_stale_rates() {
        let now = Instant::now();
        let mut s = Sampler::default();
        s.update(1, 0, 0, now, 1000);
        s.update(1, 1000, 100, now + Duration::from_secs(1), 1000);
        assert!(!s.update(1, 20_000, 2000, now + Duration::from_secs(100), 1000));
        assert_eq!((s.down, s.up), (0.0, 0.0));
        assert_eq!((s.total_down, s.total_up), (1000, 100));
    }
}
