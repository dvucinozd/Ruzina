//! MAIN level meter fed only by accepted final output.
//!
//! M2.5 drives the Web meter from the master limiter peak. Here a peak counts
//! only once the MAIN sink accepted the block (an output receipt), so audio
//! that was rendered but dropped never shows. The held peak decays at about
//! 20 dB/s after a short hold, in integer steps.

/// Full scale of a signed 16-bit peak.
pub const METER_FULL_SCALE: u16 = 32_767;
pub const METER_HOLD_MS: u64 = 100;
const DECAY_STEP_MS: u64 = 10;
/// 1001/1024 per 10 ms is -0.197 dB per step, ~19.7 dB per second.
const DECAY_NUMERATOR: u32 = 1001;
const DECAY_DENOMINATOR: u32 = 1024;
/// After 4 s of decay (~79 dB) the meter reads silence.
const DECAY_MAX_STEPS: u64 = 400;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MainMeter {
    peak: u16,
    at_ms: u64,
}

impl MainMeter {
    pub const fn new() -> Self {
        Self { peak: 0, at_ms: 0 }
    }

    /// One accepted MAIN block's absolute peak, at the receipt time.
    pub fn accept(&mut self, now_ms: u64, peak_abs: u16) {
        let peak_abs = peak_abs.min(METER_FULL_SCALE);
        if peak_abs >= self.level(now_ms) {
            self.peak = peak_abs;
            self.at_ms = now_ms;
        }
    }

    /// The displayed level, 0..=32767.
    pub fn level(&self, now_ms: u64) -> u16 {
        let elapsed = now_ms.saturating_sub(self.at_ms);
        if elapsed <= METER_HOLD_MS {
            return self.peak;
        }
        let steps = (elapsed - METER_HOLD_MS) / DECAY_STEP_MS;
        if steps >= DECAY_MAX_STEPS {
            return 0;
        }
        let mut level = u32::from(self.peak);
        for _ in 0..steps {
            level = level * DECAY_NUMERATOR / DECAY_DENOMINATOR;
            if level == 0 {
                break;
            }
        }
        level as u16
    }

    /// Output stopped or the sink changed: no stale level survives.
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_hold_then_decay_about_20_db_per_second() {
        let mut meter = MainMeter::new();
        assert_eq!(meter.level(0), 0);
        meter.accept(1_000, 32_767);
        assert_eq!(meter.level(1_050), 32_767);
        assert_eq!(meter.level(1_100), 32_767);
        // One second of decay is ~-19.7 dB, about a tenth of the peak.
        let after = meter.level(2_100);
        assert!((3_200..=3_450).contains(&after), "{after}");
        assert_eq!(meter.level(10_000), 0);
        // A louder block replaces a decayed peak; a quieter one does not.
        meter.accept(2_100, 20_000);
        assert_eq!(meter.level(2_100), 20_000);
        meter.accept(2_150, 1_000);
        assert_eq!(meter.level(2_150), 20_000);
        meter.accept(2_200, 40_000);
        assert_eq!(meter.level(2_200), METER_FULL_SCALE);
        meter.reset();
        assert_eq!(meter.level(2_200), 0);
    }
}
