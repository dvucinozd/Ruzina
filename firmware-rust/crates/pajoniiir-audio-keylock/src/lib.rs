#![no_std]
#![forbid(unsafe_code)]

use pajoniiir_audio_dsp::PcmFrame;

pub const KEYLOCK_SYNTH_HOP: u32 = 256;
pub const KEYLOCK_SEARCH_CACHE_FRAMES: usize = 640;

const REFERENCE_STRIDE: u32 = 32;
const REFERENCE_COUNT: usize = 2;
const COARSE_POINTS: i32 = 8;
const REFINE_RADIUS: i32 = 3;
const REBASE_THRESHOLD: f32 = 16_384.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeylockOutput {
    pub frame: PcmFrame,
    pub consumed: u32,
    pub play_seq: u64,
}

pub struct KeylockState {
    initialized: bool,
    initial_half: bool,
    phase: u32,
    origin_seq: u64,
    logical_seq: u64,
    grain_a: f32,
    grain_b: f32,
    logical_fraction: f32,
    tempo_factor: f32,
    rate_ratio: f32,
    last_search_candidates: u16,
    search_frames: [PcmFrame; KEYLOCK_SEARCH_CACHE_FRAMES],
    search_valid: [u8; KEYLOCK_SEARCH_CACHE_FRAMES],
}

impl KeylockState {
    pub fn new(start: u64) -> Self {
        Self {
            initialized: true,
            initial_half: true,
            phase: 0,
            origin_seq: start,
            logical_seq: start,
            grain_a: 0.0,
            grain_b: KEYLOCK_SYNTH_HOP as f32,
            logical_fraction: 0.0,
            tempo_factor: 1.0,
            rate_ratio: 1.0,
            last_search_candidates: 0,
            search_frames: [PcmFrame { left: 0, right: 0 }; KEYLOCK_SEARCH_CACHE_FRAMES],
            search_valid: [0; KEYLOCK_SEARCH_CACHE_FRAMES],
        }
    }

    pub fn reset(&mut self, start: u64) {
        self.initialized = true;
        self.initial_half = true;
        self.phase = 0;
        self.origin_seq = start;
        self.logical_seq = start;
        self.grain_a = 0.0;
        self.grain_b = KEYLOCK_SYNTH_HOP as f32;
        self.logical_fraction = 0.0;
        self.tempo_factor = 1.0;
        self.rate_ratio = 1.0;
        self.last_search_candidates = 0;
        self.search_valid.fill(0);
    }

    pub fn configure(&mut self, tempo_factor: f32, rate_ratio: f32) {
        let next_tempo = clamp_factor(tempo_factor, 0.50, 2.00);
        let next_ratio = clamp_factor(rate_ratio, 0.25, 4.00);
        if self.tempo_factor == next_tempo && self.rate_ratio == next_ratio {
            return;
        }
        self.tempo_factor = next_tempo;
        self.rate_ratio = next_ratio;
    }

    pub fn next<F>(&mut self, read: &mut F) -> Option<KeylockOutput>
    where
        F: FnMut(u64) -> Option<PcmFrame>,
    {
        if !self.initialized {
            return None;
        }

        let ratio = self.rate_ratio;
        let frame = if self.initial_half {
            read_fractional(
                read,
                self.origin_seq,
                self.grain_a + self.phase as f32 * ratio,
            )?
        } else {
            let pa = self.grain_a + (KEYLOCK_SYNTH_HOP + self.phase) as f32 * ratio;
            let pb = self.grain_b + self.phase as f32 * ratio;
            let a = read_fractional(read, self.origin_seq, pa)?;
            let b = read_fractional(read, self.origin_seq, pb)?;
            let fade = (self.phase + 1) as f32 / KEYLOCK_SYNTH_HOP as f32;
            PcmFrame {
                left: lerp_i16(a.left, b.left, fade),
                right: lerp_i16(a.right, b.right, fade),
            }
        };

        let before = self.logical_seq;
        self.logical_fraction += self.tempo_factor * ratio;
        let advance = self.logical_fraction as u32;
        self.logical_seq = self.logical_seq.wrapping_add(advance as u64);
        self.logical_fraction -= advance as f32;
        let consumed = self.logical_seq.wrapping_sub(before) as u32;
        let play_seq = self.logical_seq;

        self.phase += 1;
        if self.phase >= KEYLOCK_SYNTH_HOP {
            self.phase = 0;
            let nominal =
                self.logical_seq.wrapping_sub(self.origin_seq) as f32 + self.logical_fraction;
            if self.initial_half {
                self.initial_half = false;
            } else {
                self.grain_a = self.grain_b;
            }
            self.grain_b = self.select_grain_start(read, nominal);
            self.rebase_coordinates();
        }

        Some(KeylockOutput {
            frame,
            consumed,
            play_seq,
        })
    }

    pub const fn origin_seq(&self) -> u64 {
        self.origin_seq
    }

    pub const fn logical_seq(&self) -> u64 {
        self.logical_seq
    }

    pub const fn grain_a(&self) -> f32 {
        self.grain_a
    }

    pub const fn tempo_factor(&self) -> f32 {
        self.tempo_factor
    }

    pub const fn rate_ratio(&self) -> f32 {
        self.rate_ratio
    }

    pub const fn last_search_candidates(&self) -> u16 {
        self.last_search_candidates
    }

    fn select_grain_start<F>(&mut self, read: &mut F, nominal: f32) -> f32
    where
        F: FnMut(u64) -> Option<PcmFrame>,
    {
        if self.tempo_factor > 0.9999 && self.tempo_factor < 1.0001 {
            return nominal;
        }

        let reference = self.grain_a + KEYLOCK_SYNTH_HOP as f32 * self.rate_ratio;
        let mut best = nominal;
        let mut best_error = u32::MAX;
        let mut radius = (48.0 * self.rate_ratio + 0.5) as i32;
        radius = radius.max(12);

        let mut reference_frames = [PcmFrame { left: 0, right: 0 }; REFERENCE_COUNT];
        let mut reference_count = 0usize;
        for i in (0..64u32).step_by(REFERENCE_STRIDE as usize) {
            let offset = i as f32 * self.rate_ratio;
            let Some(frame) = read_fractional(read, self.origin_seq, reference + offset) else {
                return nominal;
            };
            reference_frames[reference_count] = frame;
            reference_count += 1;
        }

        let first = (nominal - radius as f32).max(0.0);
        let first_frame = first as u32;
        let end_frame = (nominal + radius as f32 + 60.0 * self.rate_ratio) as u32 + 2;
        let count = (end_frame - first_frame).min(KEYLOCK_SEARCH_CACHE_FRAMES as u32) as usize;
        self.search_valid[..count].fill(0);
        let cache_first = self.origin_seq + first_frame as u64;

        self.last_search_candidates = 0;
        let mut center = 0i32;
        for point in 0..COARSE_POINTS {
            let delta = -radius + (2 * radius * point) / (COARSE_POINTS - 1);
            let candidate = nominal + delta as f32;
            if candidate < 0.0 {
                continue;
            }
            if let Some(error) = self.candidate_sad(
                read,
                cache_first,
                count,
                &reference_frames,
                reference_count,
                candidate,
                best_error,
            ) && error < best_error
            {
                best_error = error;
                best = candidate;
                center = delta;
            }
        }

        let first_delta = (center - REFINE_RADIUS).max(-radius);
        let last_delta = (center + REFINE_RADIUS).min(radius);
        for delta in first_delta..=last_delta {
            let candidate = nominal + delta as f32;
            if candidate < 0.0 {
                continue;
            }
            if let Some(error) = self.candidate_sad(
                read,
                cache_first,
                count,
                &reference_frames,
                reference_count,
                candidate,
                best_error,
            ) && error < best_error
            {
                best_error = error;
                best = candidate;
            }
        }

        best
    }

    #[allow(clippy::too_many_arguments)]
    fn candidate_sad<F>(
        &mut self,
        read: &mut F,
        cache_first: u64,
        cache_count: usize,
        reference_frames: &[PcmFrame; REFERENCE_COUNT],
        reference_count: usize,
        candidate: f32,
        stop_at: u32,
    ) -> Option<u32>
    where
        F: FnMut(u64) -> Option<PcmFrame>,
    {
        let mut error = 0u32;
        self.last_search_candidates = self.last_search_candidates.saturating_add(1);

        for (sample, reference_frame) in reference_frames.iter().take(reference_count).enumerate() {
            let offset = (sample as u32 * REFERENCE_STRIDE) as f32 * self.rate_ratio;
            let origin = self.origin_seq;
            let mut cached = |seq| self.read_cached(read, cache_first, cache_count, seq);
            let b = read_fractional(&mut cached, origin, candidate + offset)?;
            let dl = reference_frame.left as i32 - b.left as i32;
            let dr = reference_frame.right as i32 - b.right as i32;
            error = error
                .saturating_add(dl.unsigned_abs())
                .saturating_add(dr.unsigned_abs());
            if error >= stop_at {
                break;
            }
        }

        Some(error)
    }

    fn read_cached<F>(
        &mut self,
        read: &mut F,
        first: u64,
        count: usize,
        seq: u64,
    ) -> Option<PcmFrame>
    where
        F: FnMut(u64) -> Option<PcmFrame>,
    {
        if seq < first || seq - first >= count as u64 {
            return read(seq);
        }

        let index = (seq - first) as usize;
        if self.search_valid[index] == 0 {
            match read(seq) {
                Some(frame) => {
                    self.search_frames[index] = frame;
                    self.search_valid[index] = 1;
                }
                None => self.search_valid[index] = 2,
            }
        }

        if self.search_valid[index] == 1 {
            Some(self.search_frames[index])
        } else {
            None
        }
    }

    fn rebase_coordinates(&mut self) {
        if self.grain_a < REBASE_THRESHOLD {
            return;
        }
        let shift = (self.grain_a - KEYLOCK_SYNTH_HOP as f32) as u32;
        self.origin_seq = self.origin_seq.wrapping_add(shift as u64);
        self.grain_a -= shift as f32;
        self.grain_b -= shift as f32;
    }
}

fn clamp_factor(value: f32, low: f32, high: f32) -> f32 {
    if !value.is_finite() {
        1.0
    } else {
        value.clamp(low, high)
    }
}

fn lerp_i16(a: i16, b: i16, fraction: f32) -> i16 {
    let value = a as f32 + (b as f32 - a as f32) * fraction;
    if value > i16::MAX as f32 {
        i16::MAX
    } else if value < i16::MIN as f32 {
        i16::MIN
    } else if value >= 0.0 {
        (value + 0.5) as i16
    } else {
        (value - 0.5) as i16
    }
}

fn read_fractional<F>(read: &mut F, origin_seq: u64, seq: f32) -> Option<PcmFrame>
where
    F: FnMut(u64) -> Option<PcmFrame>,
{
    if seq < 0.0 {
        return None;
    }
    let whole = seq as u32;
    let fraction = seq - whole as f32;
    let absolute = origin_seq.wrapping_add(whole as u64);
    let a = read(absolute)?;
    if fraction <= 0.000001 {
        return Some(a);
    }
    let b = match read(absolute.wrapping_add(1)) {
        Some(frame) => frame,
        None => return Some(a),
    };
    Some(PcmFrame {
        left: lerp_i16(a.left, b.left, fraction),
        right: lerp_i16(a.right, b.right, fraction),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE_FRAMES: usize = 8_192;

    fn sine_fixture() -> [PcmFrame; SOURCE_FRAMES] {
        let mut source = [PcmFrame { left: 0, right: 0 }; SOURCE_FRAMES];
        for (i, frame) in source.iter_mut().enumerate() {
            let phase = 2.0 * core::f64::consts::PI * 1_000.0 * i as f64 / 48_000.0;
            let value = (libm::sin(phase) * 12_000.0) as i16;
            *frame = PcmFrame {
                left: value,
                right: value,
            };
        }
        source
    }

    fn count_positive_crossings(samples: &[i16]) -> usize {
        samples
            .windows(2)
            .filter(|pair| pair[0] <= 0 && pair[1] > 0)
            .count()
    }

    #[test]
    fn state_is_bounded_and_keeps_search_cache_out_of_hot_stack() {
        assert!(core::mem::size_of::<KeylockState>() < 4_096);
        assert_eq!(KEYLOCK_SEARCH_CACHE_FRAMES, 640);
    }

    #[test]
    fn keylock_preserves_pitch_while_advancing_tempo() {
        let source = sine_fixture();
        let mut state = KeylockState::new(0);
        state.configure(1.10, 1.0);
        let mut output = [0i16; 4_096];
        let mut consumed_total = 0u32;
        let mut reader = |seq: u64| source.get(seq as usize).copied();

        for sample in &mut output {
            let next = state.next(&mut reader).expect("source runway");
            *sample = next.frame.left;
            consumed_total += next.consumed;
        }

        assert!((4_504..=4_507).contains(&consumed_total));
        let crossings = count_positive_crossings(&output[512..3_584]);
        assert!(
            (60..=68).contains(&crossings),
            "crossings={crossings} consumed={consumed_total} grain_a={} grain_b={} origin={} logical={}",
            state.grain_a,
            state.grain_b,
            state.origin_seq,
            state.logical_seq,
        );
    }

    #[test]
    fn unity_tempo_is_sample_exact_and_long_position_rebases() {
        let source = sine_fixture();
        let mut state = KeylockState::new(0);
        state.configure(1.0, 1.0);
        let mut reader = |seq: u64| source.get(seq as usize).copied();

        for expected in source.iter().take(1_024) {
            assert_eq!(state.next(&mut reader).unwrap().frame, *expected);
        }

        let long_start = 100_000_000u64;
        state.reset(long_start);
        state.configure(1.0, 1.0);
        let mut repeating = |seq: u64| Some(source[seq as usize % SOURCE_FRAMES]);
        let mut play_seq = long_start;

        for i in 0..20_000u64 {
            let next = state.next(&mut repeating).unwrap();
            play_seq = next.play_seq;
            assert_eq!(
                next.frame,
                source[((long_start + i) as usize) % SOURCE_FRAMES]
            );
        }

        assert_eq!(play_seq, long_start + 20_000);
        assert!(state.origin_seq() > long_start);
        assert!(state.grain_a() < REBASE_THRESHOLD);
    }

    #[test]
    fn invalid_factors_fail_safe_to_unity() {
        let mut state = KeylockState::new(0);
        state.configure(f32::NAN, f32::INFINITY);
        assert_eq!(state.tempo_factor(), 1.0);
        assert_eq!(state.rate_ratio(), 1.0);
        state.configure(0.1, 9.0);
        assert_eq!(state.tempo_factor(), 0.5);
        assert_eq!(state.rate_ratio(), 4.0);
    }

    fn run_search_budget(ratio: f32, tempo: f32, missing: bool) {
        const FIXTURE: usize = 8_192;
        let mut frames = [PcmFrame { left: 0, right: 0 }; FIXTURE];
        for (i, frame) in frames.iter_mut().enumerate() {
            let phase = 2.0 * core::f32::consts::PI * i as f32 / FIXTURE as f32;
            *frame = PcmFrame {
                left: (10_000.0 * libm::sinf(phase * 71.0) + 4_000.0 * libm::sinf(phase * 179.0))
                    as i16,
                right: (9_000.0 * libm::sinf(phase * 83.0) + 3_000.0 * libm::cosf(phase * 197.0))
                    as i16,
            };
        }

        let mut state = KeylockState::new(100_000_000);
        let mut max_calls = 0u32;
        let mut max_candidates = 0u16;
        let mut failures = 0u32;

        for i in 0..48_000u32 {
            let current = if i < 24_000 {
                tempo
            } else if i < 36_000 {
                if (i / 73) % 2 != 0 { 1.05 } else { 0.95 }
            } else {
                1.0
            };
            state.configure(current, ratio);

            let mut calls = 0u32;
            let mut reader = |seq: u64| {
                calls += 1;
                let index = seq as usize % FIXTURE;
                if missing && index >= FIXTURE - 3 {
                    None
                } else {
                    Some(frames[index])
                }
            };

            if state.next(&mut reader).is_none() {
                assert!(missing);
                failures += 1;
                state.reset(state.logical_seq().wrapping_add(1_024));
            }
            max_calls = max_calls.max(calls);
            max_candidates = max_candidates.max(state.last_search_candidates());
        }

        let mut radius = (48.0 * ratio + 0.5) as u32;
        radius = radius.max(12);
        let limit = 2 * radius + libm::ceilf(60.0 * ratio) as u32 + 40;
        assert!(max_calls <= limit);
        assert!(max_calls <= 64);
        assert!(max_candidates <= 16);
        if !missing {
            assert_eq!(failures, 0);
        }
    }

    #[test]
    fn wsola_search_keeps_released_read_and_candidate_budgets() {
        for ratio in [
            0.25,
            44_100.0 / 48_000.0,
            1.0,
            48_000.0 / 44_100.0,
            96_000.0 / 44_100.0,
            4.0,
        ] {
            run_search_budget(ratio, 0.95, false);
            run_search_budget(ratio, 1.05, false);
            run_search_budget(ratio, 0.8, true);
        }
    }
}
