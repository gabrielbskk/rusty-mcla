//! Rendering may run freely; gameplay advances at the verified 60 Hz cadence.
pub const DT: f32 = 1.0 / 60.0;
const STEP: f64 = 1.0 / 60.0;

#[derive(Default)]
pub struct Clock {
    remainder: f64,
}

impl Clock {
    pub fn reset(&mut self) { self.remainder = 0.0; }

    /// Bound recovery from a stalled window to the existing 100 ms limit.
    pub fn advance(&mut self, elapsed: f64) -> usize {
        self.remainder += elapsed.clamp(0.0, 0.1);
        let ticks = ((self.remainder + 1e-10) / STEP).floor() as usize;
        self.remainder = (self.remainder - ticks as f64 * STEP).max(0.0);
        ticks
    }

    pub fn alpha(&self) -> f32 {
        (self.remainder / STEP).clamp(0.0, 1.0) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_deliver_the_same_ticks() {
        for fps in [30, 60, 144, 240, 400] {
            let mut clock = Clock::default();
            let count: usize = (0..fps * 12).map(|_| clock.advance(1.0 / fps as f64)).sum();
            assert_eq!(count, 720, "{fps} fps");
            assert!(clock.alpha() < 1e-6);
        }
        let mut clock = Clock::default();
        let count: usize = (0..360).flat_map(|_| [0.001, 0.029, 1.0 / 300.0]).map(|d| clock.advance(d)).sum();
        assert_eq!(count, 720);
    }

    #[test]
    fn stall_is_bounded_and_fraction_is_retained() {
        let mut clock = Clock::default();
        assert_eq!(clock.advance(STEP * 0.25), 0);
        assert_eq!(clock.advance(STEP * 0.25), 0);
        assert!((clock.alpha() - 0.5).abs() < 1e-6);
        assert_eq!(clock.advance(3.0), 6);
        assert!((clock.alpha() - 0.5).abs() < 1e-6);
        assert_eq!(clock.advance(STEP * 0.5), 1);
        assert!(clock.alpha() < 1e-6);
    }

    #[test]
    fn loading_does_not_leave_motion_to_catch_up() {
        let mut clock = Clock::default();
        assert_eq!(clock.advance(STEP * 0.75), 0);
        clock.reset();
        assert_eq!(clock.advance(STEP * 0.5), 0);
        assert_eq!(clock.advance(STEP * 0.5), 1);
    }
}
