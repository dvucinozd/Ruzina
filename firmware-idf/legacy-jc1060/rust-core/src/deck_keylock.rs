//! Master Tempo for the deck mix: a time stretch that keeps the key while the tempo moves.
//!
//! WSOLA: the deck's frames are cut into grains of `GRAIN` frames, laid out one `HOP`
//! apart, and each output frame crossfades two of them with a raised cosine. The input
//! moves `HOP` times the tempo between grains, so the track plays at the tempo, while
//! each grain plays at its own speed, so the key stays. Before a grain starts, its start
//! moves up to `SEEK` frames either way to where it best matches the grain it takes over
//! from, so the crossfade joins like waveforms.
//!
//! The stretch works at the track's own rate; the render graph then resamples to 48 kHz
//! as without Master Tempo. It reads the deck's frames in play order, counting positions
//! from its start, so loops, phase jumps and the history replay reach it exactly as they
//! reach the graph. A window keeps the frames it may still read.
use alloc::boxed::Box;
use ruzina_audio_dsp::PcmFrame;

/// Frames in a grain: about 23 ms at 44.1 kHz.
const GRAIN: usize = 1024;
/// Output frames from one grain to the next: half a grain, so two always overlap.
const HOP: usize = GRAIN / 2;
/// How far either way a grain's start may move to match the grain before it.
const SEEK: usize = 256;
/// The coarse search compares every `STRIDE`th frame, at every `STRIDE`th start.
const STRIDE: usize = 4;
/// The coarse search's frames: the overlap of a hop, thinned.
const TEMPLATE: usize = HOP / STRIDE;
/// The frames every coarse candidate's overlap reaches, thinned.
const SPAN: usize = (2 * SEEK + HOP) / STRIDE;
/// Frames the window keeps; a power of two, well over the reads' reach at tempo 2.
const WINDOW: usize = 8192;
/// Tempo factors the stretch follows; beyond them it holds the nearest.
const TEMPO_RANGE: (f32, f32) = (0.5, 2.0);

pub(crate) struct Keylock {
    window: [PcmFrame; WINDOW],
    /// The raised-cosine fade-in over one hop, from 0 towards 1.
    fade: [f32; HOP],
    /// The next frame's position: the frames pulled from the reader since the start.
    pulled: u64,
    /// The grain fading out (its second half plays now) and the grain fading in.
    out_start: i64,
    in_start: i64,
    /// Where the fading-in grain would start at exactly the tempo: the stretch's anchor.
    nominal: f64,
    /// Output frames into the current hop.
    phase: usize,
    running: bool,
    template: [f32; TEMPLATE],
    span: [f32; SPAN],
}

impl Keylock {
    /// Both decks' keylocks on the heap, never as a stack temporary: each is about
    /// 36 KiB, far over the USB task's free stack.
    pub(crate) fn boxed_pair() -> Box<[Keylock; 2]> {
        // SAFETY: every field of `Keylock` is an integer, a float, a bool or an array of
        // them, for which all-zero bytes are a valid value.
        let mut pair = unsafe { Box::<[Keylock; 2]>::new_zeroed().assume_init() };
        for keylock in pair.iter_mut() {
            keylock.fade = raised_cosine();
            keylock.stop();
        }
        pair
    }

    /// The next frame starts over: after a seek, a scratch or Master Tempo going off.
    pub(crate) fn stop(&mut self) {
        self.running = false;
    }

    /// Frames pulled from the reader but not heard yet: the stretch's lead on the deck's
    /// position.
    pub(crate) fn lag(&self) -> u32 {
        if !self.running {
            return 0;
        }
        let heard = self.in_start + self.phase as i64;
        (self.pulled as i64 - heard).clamp(0, WINDOW as i64) as u32
    }

    /// The next stretched frame at the track's rate, with `tempo` the deck's tempo
    /// factor. `pull` hands out the reader's next frame, `None` while the deck has none;
    /// then this returns `None`, and the next call takes up the same frame.
    pub(crate) fn next(
        &mut self,
        tempo: f32,
        mut pull: impl FnMut() -> Option<PcmFrame>,
    ) -> Option<PcmFrame> {
        if !self.running {
            // The first hop plays straight from the start: both grains read the same frames.
            self.pulled = 0;
            self.in_start = 0;
            self.out_start = -(HOP as i64);
            self.nominal = 0.0;
            self.phase = 0;
            self.running = true;
        }
        if self.phase == HOP {
            self.next_grain(tempo.clamp(TEMPO_RANGE.0, TEMPO_RANGE.1), &mut pull);
        }
        let phase = self.phase as i64;
        let outgoing = self.read(self.out_start + HOP as i64 + phase, &mut pull)?;
        let incoming = self.read(self.in_start + phase, &mut pull)?;
        let fade = self.fade[self.phase];
        self.phase += 1;
        let mix = |a: i16, b: i16| (f32::from(a) + (f32::from(b) - f32::from(a)) * fade) as i16;
        Some(PcmFrame {
            left: mix(outgoing.left, incoming.left),
            right: mix(outgoing.right, incoming.right),
        })
    }

    /// The hop is over: the grain that faded in fades out, and the next one starts
    /// where it best continues it, near where the tempo puts it.
    fn next_grain(&mut self, tempo: f32, pull: &mut impl FnMut() -> Option<PcmFrame>) {
        let previous = self.in_start;
        let continuation = previous + HOP as i64;
        self.nominal += HOP as f64 * f64::from(tempo);
        let start = if (tempo - 1.0).abs() < 1.0e-4 {
            // At the track's own tempo the next grain simply continues the last one, so
            // the output is the input, frame for frame.
            self.nominal = continuation as f64;
            continuation
        } else {
            let nominal = self.nominal as i64;
            self.search(continuation, nominal, pull).unwrap_or(nominal)
        };
        self.out_start = previous;
        self.in_start = start;
        self.phase = 0;
    }

    /// The start within `SEEK` of `nominal` whose frames best match the frames from
    /// `continuation`: coarsely on every `STRIDE`th frame, then frame by frame around the
    /// best. `None` when the deck cannot yet supply the frames to compare.
    fn search(
        &mut self,
        continuation: i64,
        nominal: i64,
        pull: &mut impl FnMut() -> Option<PcmFrame>,
    ) -> Option<i64> {
        let oldest = self.pulled as i64 + (STRIDE as i64) - WINDOW as i64;
        let first = (nominal - SEEK as i64).max(oldest).max(0);
        for j in 0..TEMPLATE {
            self.template[j] = mono(self.read(continuation + (j * STRIDE) as i64, pull)?);
        }
        for j in 0..SPAN {
            self.span[j] = mono(self.read(first + (j * STRIDE) as i64, pull)?);
        }
        // Normalised cross-correlation, compared as its signed square so that no root is
        // taken; the candidate's energy slides along with it.
        let mut energy: f32 = self.span[..TEMPLATE].iter().map(|s| s * s).sum();
        let mut best = (f32::MIN, 0usize);
        for c in 0..=SPAN - TEMPLATE {
            if c > 0 {
                let (gone, came) = (self.span[c - 1], self.span[c + TEMPLATE - 1]);
                energy = (energy - gone * gone + came * came).max(0.0);
            }
            let correlation: f32 = self
                .template
                .iter()
                .zip(&self.span[c..c + TEMPLATE])
                .map(|(t, s)| t * s)
                .sum();
            let score = correlation * correlation.abs() / (energy + 1.0);
            if score > best.0 {
                best = (score, c);
            }
        }
        let coarse = first + (best.1 * STRIDE) as i64;
        // Frame by frame around the coarse pick, over the whole overlap at every second
        // frame; all of it lies within the frames the coarse search pulled.
        let mut refined = (f32::MIN, coarse);
        for start in (coarse - STRIDE as i64 + 1).max(first)..coarse + STRIDE as i64 {
            let (mut correlation, mut energy) = (0.0f32, 1.0f32);
            for j in (0..HOP as i64).step_by(2) {
                let t = mono(self.read(continuation + j, pull)?);
                let s = mono(self.read(start + j, pull)?);
                correlation += t * s;
                energy += s * s;
            }
            let score = correlation * correlation.abs() / energy;
            if score > refined.0 {
                refined = (score, start);
            }
        }
        Some(refined.1)
    }

    /// The deck's frame at `position`, pulling from the reader up to it. A position the
    /// window no longer holds reads as silence.
    fn read(
        &mut self,
        position: i64,
        pull: &mut impl FnMut() -> Option<PcmFrame>,
    ) -> Option<PcmFrame> {
        let Ok(position) = u64::try_from(position) else {
            return Some(PcmFrame::default());
        };
        while self.pulled <= position {
            self.window[(self.pulled % WINDOW as u64) as usize] = pull()?;
            self.pulled += 1;
        }
        if position + (WINDOW as u64) <= self.pulled {
            return Some(PcmFrame::default());
        }
        Some(self.window[(position % WINDOW as u64) as usize])
    }

    /// Frames pulled from the reader since the start.
    #[cfg(test)]
    pub(crate) const fn pulled(&self) -> u64 {
        self.pulled
    }
}

/// Both channels as one, for matching grains.
fn mono(frame: PcmFrame) -> f32 {
    f32::from(frame.left) + f32::from(frame.right)
}

/// `0.5 - 0.5 cos(pi p / HOP)` for each frame `p` of a hop: with the same curve falling,
/// the two grains' weights add up to one at every frame. The cosine comes from a rotation
/// by small steps, so no libm is needed.
fn raised_cosine() -> [f32; HOP] {
    let step = core::f64::consts::PI / HOP as f64;
    let square = step * step;
    let (step_cos, step_sin) = (
        1.0 - square / 2.0 + square * square / 24.0,
        step * (1.0 - square / 6.0 + square * square / 120.0),
    );
    let (mut cos, mut sin) = (1.0f64, 0.0f64);
    let mut fade = [0.0; HOP];
    for value in fade.iter_mut() {
        *value = (0.5 - 0.5 * cos) as f32;
        (cos, sin) = (
            cos * step_cos - sin * step_sin,
            sin * step_cos + cos * step_sin,
        );
    }
    fade
}

#[cfg(test)]
mod tests;
