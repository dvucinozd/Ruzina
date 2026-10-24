//! Transient jog tempo bend (M2.5 B13a, `audio_engine` jog nudge).
//!
//! Each `JogNudge` tick adds 0.02 to a bend clamped to ±0.30; the effective
//! speed is `pitch × (1 + bend)`, and the bend springs back to zero once the
//! jog stops. M2.5 multiplies it by 0.88 per 256-frame block at 44.1 kHz
//! (about 5.8 ms); here the decay is expressed per elapsed output time, so a
//! 128-frame 48 kHz render block decays exactly as much as that much time
//! did in M2.5. Leaving a platter hold clears any leftover bend.

pub const JOG_BEND_PER_TICK: f32 = 0.02;
pub const JOG_BEND_MAX: f32 = 0.30;
pub const JOG_BEND_DECAY: f32 = 0.88;
/// M2.5 decay interval: one 256-frame block at 44.1 kHz.
pub const JOG_BEND_DECAY_SECONDS: f32 = 256.0 / 44_100.0;
const SNAP: f32 = 0.0005;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TempoNudge {
    bend: f32,
}

impl TempoNudge {
    pub const fn new() -> Self {
        Self { bend: 0.0 }
    }

    pub const fn bend(&self) -> f32 {
        self.bend
    }

    pub fn add(&mut self, ticks: i16) {
        let bend = if self.bend.is_finite() {
            self.bend
        } else {
            0.0
        };
        self.bend =
            (bend + f32::from(ticks) * JOG_BEND_PER_TICK).clamp(-JOG_BEND_MAX, JOG_BEND_MAX);
    }

    /// Decays for `frames` of output at `sample_rate_hz`.
    pub fn advance(&mut self, frames: u32, sample_rate_hz: u32) {
        if self.bend == 0.0 || sample_rate_hz == 0 {
            return;
        }
        let seconds = frames as f32 / sample_rate_hz as f32;
        self.bend *= libm::powf(JOG_BEND_DECAY, seconds / JOG_BEND_DECAY_SECONDS);
        if !self.bend.is_finite() || self.bend.abs() < SNAP {
            self.bend = 0.0;
        }
    }

    pub fn clear(&mut self) {
        self.bend = 0.0;
    }

    /// The fader pitch with the bend applied, for the existing
    /// `tempo_factor_from_centipercent` path.
    pub fn apply_centipercent(&self, pitch_centipercent: i16) -> i16 {
        let factor = (1.0 + f32::from(pitch_centipercent) / 10_000.0) * (1.0 + self.bend);
        libm::roundf((factor - 1.0) * 10_000.0).clamp(-10_000.0, f32::from(i16::MAX)) as i16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_bend_within_the_m25_limits() {
        let mut nudge = TempoNudge::new();
        nudge.add(5);
        assert!((nudge.bend() - 0.10).abs() < 1e-6);
        nudge.add(100);
        assert_eq!(nudge.bend(), JOG_BEND_MAX);
        nudge.add(i16::MIN);
        assert_eq!(nudge.bend(), -JOG_BEND_MAX);
        nudge.clear();
        assert_eq!(nudge.bend(), 0.0);
    }

    #[test]
    fn decay_matches_m25_per_elapsed_time_and_snaps_to_zero() {
        let mut m25 = TempoNudge::new();
        m25.add(10);
        m25.advance(256, 44_100);
        assert!((m25.bend() - 0.2 * 0.88).abs() < 1e-6, "one M2.5 block");
        // The same 5.8 ms in 48 kHz render blocks decays the same.
        let mut rust = TempoNudge::new();
        rust.add(10);
        let frames = 256.0 / 44_100.0 * 48_000.0;
        rust.advance(frames as u32, 48_000);
        rust.advance(1, 48_000);
        assert!((rust.bend() - m25.bend()).abs() < 0.002);
        // Released, it springs back within a fraction of a second.
        for _ in 0..200 {
            rust.advance(128, 48_000);
        }
        assert_eq!(rust.bend(), 0.0);
    }

    #[test]
    fn the_bend_rides_on_the_fader_pitch() {
        let mut nudge = TempoNudge::new();
        assert_eq!(nudge.apply_centipercent(-625), -625);
        nudge.add(5);
        // +10% on top of -6.25%: 0.9375 × 1.1 = 1.03125.
        assert_eq!(nudge.apply_centipercent(-625), 313);
        nudge.add(-20);
        assert_eq!(
            nudge.apply_centipercent(0),
            -3_000,
            "0.10 - 0.40 clamps to -0.30"
        );
    }
}
