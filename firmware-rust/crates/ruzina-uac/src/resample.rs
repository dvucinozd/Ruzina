//! Render-rate to device-rate conversion (M2.5 `controller_audio_resampler`).
//!
//! The engine renders at 48 kHz; a controller such as the FLX4 streams only
//! 44.1 kHz. Linear interpolation with exact rational phase: input frame `i`
//! sits at time `i * target` and output frame `j` at `j * source`, so
//! splitting the stream into arbitrary blocks changes nothing, there is no
//! drift over any length, and DC passes at unity. MAIN and headphones travel
//! in one multi-channel frame, so both keep the same phase.
//!
//! The UAC side pulls: [`Resampler::input_needed`] says how many render
//! frames the next packet's output frames require, the adapter issues exactly
//! that many from its sink lanes (so their receipts count render frames), and
//! [`Resampler::process`] then yields exactly the packet's frames.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResampleError {
    /// Rates must be non-zero and the source at least the target.
    Rates,
    Capacity,
}

#[derive(Clone, Copy, Debug)]
pub struct Resampler<const CH: usize> {
    source: u32,
    target: u32,
    seen: u64,
    next_output_time: u64,
    previous: Option<[i16; CH]>,
}

fn lerp(previous: i16, current: i16, offset: u64, interval: u32) -> i16 {
    let mixed = i64::from(previous) * (u64::from(interval) - offset) as i64
        + i64::from(current) * offset as i64;
    let half = i64::from(interval / 2);
    ((if mixed >= 0 {
        mixed + half
    } else {
        mixed - half
    }) / i64::from(interval)) as i16
}

impl<const CH: usize> Resampler<CH> {
    pub fn new(source: u32, target: u32) -> Result<Self, ResampleError> {
        if source == 0 || target == 0 || source < target {
            return Err(ResampleError::Rates);
        }
        Ok(Self {
            source,
            target,
            seen: 0,
            next_output_time: 0,
            previous: None,
        })
    }

    pub const fn is_passthrough(&self) -> bool {
        self.source == self.target
    }

    /// New input frames required to produce exactly `outputs` more frames.
    pub fn input_needed(&self, outputs: usize) -> usize {
        if outputs == 0 {
            return 0;
        }
        if self.is_passthrough() {
            return outputs;
        }
        // The last output lies at next + (outputs - 1) * source; it needs
        // the first input frame at or after that time.
        let last = self.next_output_time + (outputs as u64 - 1) * u64::from(self.source);
        let index = last.div_ceil(u64::from(self.target));
        (index + 1).saturating_sub(self.seen) as usize
    }

    /// Converts `input`, appending to `output`; returns frames written.
    pub fn process(
        &mut self,
        input: &[[i16; CH]],
        output: &mut [[i16; CH]],
    ) -> Result<usize, ResampleError> {
        if self.is_passthrough() {
            let out = output
                .get_mut(..input.len())
                .ok_or(ResampleError::Capacity)?;
            out.copy_from_slice(input);
            self.seen += input.len() as u64;
            self.previous = input.last().copied().or(self.previous);
            return Ok(input.len());
        }
        let mut produced = 0;
        for current in input {
            let index = self.seen;
            let Some(previous) = self.previous else {
                // The first frame is output 0 at time 0.
                *output.get_mut(produced).ok_or(ResampleError::Capacity)? = *current;
                produced += 1;
                self.previous = Some(*current);
                self.seen = 1;
                self.next_output_time = u64::from(self.source);
                continue;
            };
            let previous_time = (index - 1) * u64::from(self.target);
            let current_time = index * u64::from(self.target);
            while self.next_output_time <= current_time {
                let offset = self.next_output_time - previous_time;
                let slot = output.get_mut(produced).ok_or(ResampleError::Capacity)?;
                *slot = core::array::from_fn(|channel| {
                    lerp(previous[channel], current[channel], offset, self.target)
                });
                produced += 1;
                self.next_output_time += u64::from(self.source);
            }
            self.previous = Some(*current);
            self.seen += 1;
        }
        Ok(produced)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    fn run(resampler: &mut Resampler<2>, input: &[[i16; 2]]) -> Vec<[i16; 2]> {
        let mut out = std::vec![[0; 2]; input.len() + 2];
        let n = resampler.process(input, &mut out).unwrap();
        out.truncate(n);
        out
    }

    fn sine(frames: usize, hz: f64, rate: f64) -> Vec<[i16; 2]> {
        (0..frames)
            .map(|i| {
                let v =
                    ((2.0 * core::f64::consts::PI * hz * i as f64 / rate).sin() * 20_000.0) as i16;
                [v, -v]
            })
            .collect()
    }

    #[test]
    fn one_second_converts_exactly_without_drift() {
        let mut resampler = Resampler::<2>::new(48_000, 44_100).unwrap();
        let input = sine(48_000, 1_000.0, 48_000.0);
        let out = run(&mut resampler, &input);
        assert_eq!(out.len(), 44_100);
        // Interpolation error of a 1 kHz tone stays far below one percent.
        for (j, frame) in out.iter().enumerate() {
            let want =
                (2.0 * core::f64::consts::PI * 1_000.0 * j as f64 / 44_100.0).sin() * 20_000.0;
            assert!(
                (f64::from(frame[0]) - want).abs() < 120.0,
                "frame {j}: {} vs {want}",
                frame[0]
            );
            assert!(
                (i32::from(frame[1]) + i32::from(frame[0])).abs() <= 1,
                "channels share the phase"
            );
        }
    }

    #[test]
    fn arbitrary_blocks_are_bit_identical_to_one_pass() {
        let input = sine(10_000, 997.0, 48_000.0);
        let whole = run(&mut Resampler::new(48_000, 44_100).unwrap(), &input);
        let mut split = Resampler::new(48_000, 44_100).unwrap();
        let mut pieces = Vec::new();
        let mut at = 0;
        for size in [1usize, 2, 47, 48, 49, 128, 3, 500].iter().cycle() {
            if at >= input.len() {
                break;
            }
            let end = (at + size).min(input.len());
            pieces.extend(run(&mut split, &input[at..end]));
            at = end;
        }
        assert_eq!(pieces, whole);
    }

    #[test]
    fn dc_and_full_scale_pass_at_unity() {
        let mut resampler = Resampler::<4>::new(48_000, 44_100).unwrap();
        let input = std::vec![[24_000, -24_000, i16::MAX, i16::MIN]; 960];
        let mut out = std::vec![[0; 4]; 900];
        let n = resampler.process(&input, &mut out).unwrap();
        assert_eq!(n, 882);
        assert!(
            out[..n]
                .iter()
                .all(|frame| *frame == [24_000, -24_000, i16::MAX, i16::MIN])
        );
    }

    #[test]
    fn pulling_gives_exactly_each_packets_frames() {
        let mut resampler = Resampler::<2>::new(48_000, 44_100).unwrap();
        let mut cadence = crate::packet::Cadence::new(44_100);
        let input = sine(48_000 * 2, 440.0, 48_000.0);
        let reference = run(&mut Resampler::new(48_000, 44_100).unwrap(), &input);
        let (mut consumed, mut produced) = (0, Vec::new());
        for _ in 0..2_000 {
            let frames = cadence.next_frames();
            let needed = resampler.input_needed(frames);
            assert!(needed <= 50, "a 1 ms packet needs at most 50 render frames");
            let mut out = [[0; 2]; 48];
            let n = resampler
                .process(&input[consumed..consumed + needed], &mut out)
                .unwrap();
            assert_eq!(n, frames);
            produced.extend_from_slice(&out[..n]);
            consumed += needed;
        }
        assert_eq!(produced.len(), 88_200);
        assert_eq!(produced[..], reference[..88_200]);
        assert!(
            (consumed as i64 - 96_000).abs() <= 1,
            "two seconds of render frames: {consumed}"
        );
    }

    #[test]
    fn equal_rates_pass_through_and_bad_rates_are_refused() {
        let mut same = Resampler::<2>::new(48_000, 48_000).unwrap();
        assert!(same.is_passthrough());
        assert_eq!(same.input_needed(48), 48);
        let input = sine(48, 440.0, 48_000.0);
        assert_eq!(run(&mut same, &input), input);
        assert_eq!(
            Resampler::<2>::new(44_100, 48_000).err(),
            Some(ResampleError::Rates)
        );
        assert_eq!(
            Resampler::<2>::new(0, 48_000).err(),
            Some(ResampleError::Rates)
        );
        let mut small = Resampler::<2>::new(48_000, 44_100).unwrap();
        assert_eq!(
            small.process(&input, &mut [[0; 2]; 10]),
            Err(ResampleError::Capacity)
        );
    }
}
