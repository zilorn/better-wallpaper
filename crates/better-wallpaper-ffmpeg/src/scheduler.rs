use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameDecision {
    Present,
    Wait(Duration),
    Drop,
}

#[derive(Debug, Clone)]
pub struct PlaybackClock {
    epoch: Instant,
    first_pts: Duration,
    drop_threshold: Duration,
}

impl PlaybackClock {
    pub fn new(now: Instant, first_pts: Duration, drop_threshold: Duration) -> Self {
        Self {
            epoch: now,
            first_pts,
            drop_threshold,
        }
    }

    pub fn reset(&mut self, now: Instant, first_pts: Duration) {
        self.epoch = now;
        self.first_pts = first_pts;
    }

    pub fn decide(&self, now: Instant, pts: Duration) -> FrameDecision {
        let media_elapsed = pts.saturating_sub(self.first_pts);
        let target = self.epoch + media_elapsed;
        if now < target {
            FrameDecision::Wait(target.duration_since(now))
        } else if now.duration_since(target) > self.drop_threshold {
            FrameDecision::Drop
        } else {
            FrameDecision::Present
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_presents_and_drops_against_monotonic_clock() {
        let start = Instant::now();
        let clock = PlaybackClock::new(start, Duration::ZERO, Duration::from_millis(50));
        assert_eq!(clock.decide(start, Duration::ZERO), FrameDecision::Present);
        assert_eq!(
            clock.decide(start, Duration::from_millis(20)),
            FrameDecision::Wait(Duration::from_millis(20))
        );
        assert_eq!(
            clock.decide(
                start + Duration::from_millis(100),
                Duration::from_millis(20)
            ),
            FrameDecision::Drop
        );
    }

    #[test]
    fn reset_starts_a_new_loop_without_old_drift() {
        let start = Instant::now();
        let mut clock = PlaybackClock::new(start, Duration::ZERO, Duration::from_millis(50));
        let next_loop = start + Duration::from_secs(5);
        clock.reset(next_loop, Duration::from_millis(100));
        assert_eq!(
            clock.decide(next_loop, Duration::from_millis(100)),
            FrameDecision::Present
        );
    }
}
