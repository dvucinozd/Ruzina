//! Master Tempo for the deck mix: a time stretch that keeps the key while the tempo moves.
//!
//! WSOLA: the deck's frames are cut into grains of `GRAIN` frames, laid out one `HOP`
//! apart, and each output frame crossfades two of them with a raised cosine. The input
//! moves `HOP` times the tempo between grains, so the track plays at the tempo, while
//! each grain plays at its own speed, so the key stays. A grain's start moves up to
//! `SEEK` frames either way to where it best matches the grain it takes over from, so
//! the crossfade joins like waveforms.
//!
//! The fill must stay even, so that match is searched during the hop before the grain
//! starts, one candidate an output frame: first every 8th start, on a mono copy of every
//! 4th frame kept as frames arrive, then frame by frame around the best. A search the
//! deck cannot feed yet (just after a seek) leaves the grain where the tempo puts it.
//!
//! The stretch works at the track's own rate; the render graph then resamples to 48 kHz
//! as without Master Tempo. It reads the deck's frames in play order, counting positions
//! from its start, so loops, phase jumps and the history replay reach it exactly as they
//! reach the graph. A window keeps the frames it may still read.
use alloc::boxed::Box;
use ruzina_audio_dsp::PcmFrame;

/// Frames in a grain: about 23 ms at 44.1 kHz.
const GRAIN: u32 = 1024;
/// Output frames from one grain to the next: half a grain, so two always overlap.
const HOP: u32 = GRAIN / 2;
/// How far either way a grain's start may move to match the grain before it.
const SEEK: u32 = 256;
/// The mono copy keeps every `THIN`th frame.
const THIN: u32 = 4;
/// The coarse search tries every `COARSE`th start and compares every `COARSE`th frame
/// of the overlap.
const COARSE: u32 = 8;
/// Frames the fine search moves either way from the coarse pick.
const FINE: i32 = 3;
/// Frames the window keeps; a power of two, well over the reads' reach at tempo 2.
const WINDOW: u32 = 8192;
/// Frames pulled from the deck at most per output frame for a search, so that a search
/// right after a seek does not take a whole block's worth at once.
const PULL_BURST: u32 = 64;
/// Tempo factors the stretch follows; beyond them it holds the nearest.
const TEMPO_RANGE: (f32, f32) = (0.5, 2.0);

/// Where the search for the next grain's start stands.
#[derive(Clone, Copy)]
enum Search {
    /// Waiting for the frames: the overlap to match from `continuation`, around `nominal`.
    Waiting { continuation: u32, nominal: u32 },
    /// Every `COARSE`th start from `first`; `next` is the one to try now.
    Coarse {
        continuation: u32,
        first: u32,
        next: u32,
        best: (f32, u32),
    },
    /// Frame by frame around `center`; `offset` is the one to try now.
    Fine {
        continuation: u32,
        center: u32,
        offset: i32,
        best: (f32, u32),
    },
    /// The next grain's start.
    Done(u32),
}

pub(crate) struct Keylock {
    window: [PcmFrame; WINDOW as usize],
    /// Both channels as one, for every `THIN`th frame: the coarse search's frames.
    thin: [f32; (WINDOW / THIN) as usize],
    /// The raised-cosine fade-in over one hop, from 0 towards 1.
    fade: [f32; HOP as usize],
    /// The next frame's position: positions start at `HOP`, so the first grain's fading
    /// out partner lies before any frame.
    pulled: u32,
    /// The grain fading out (its second half plays now) and the grain fading in.
    out_start: u32,
    in_start: u32,
    /// Where the next grain would start at exactly the tempo: the stretch's anchor.
    nominal: f64,
    /// Output frames into the current hop.
    phase: u32,
    search: Search,
    running: bool,
}

impl Keylock {
    /// Both decks' keylocks on the heap, never as a stack temporary: each is about
    /// 44 KiB, far over the USB task's free stack.
    pub(crate) fn boxed_pair() -> Box<[Keylock; 2]> {
        // SAFETY: every field of `Keylock` but `search` is an integer, a float, a bool
        // or an array of them, for which all-zero bytes are a valid value; `search` is
        // written before the pair is handed out.
        let mut pair = unsafe {
            let mut pair = Box::<[Keylock; 2]>::new_zeroed();
            let base = pair.as_mut_ptr().cast::<Keylock>();
            for i in 0..2 {
                core::ptr::addr_of_mut!((*base.add(i)).search).write(Search::Done(0));
            }
            pair.assume_init()
        };
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
        self.pulled
            .wrapping_sub(self.in_start.wrapping_add(self.phase))
            .min(WINDOW)
    }

    /// The next stretched frame at the track's rate, with `tempo` the deck's tempo
    /// factor. `pull` hands out the reader's next frame, `None` while the deck has none;
    /// then this returns `None`, and the next call takes up the same frame.
    pub(crate) fn next(
        &mut self,
        tempo: f32,
        mut pull: impl FnMut() -> Option<PcmFrame>,
    ) -> Option<PcmFrame> {
        let tempo = tempo.clamp(TEMPO_RANGE.0, TEMPO_RANGE.1);
        if !self.running {
            // The first hop plays straight from the start: both grains read the same frames.
            self.pulled = HOP;
            self.out_start = 0;
            self.in_start = HOP;
            self.nominal = f64::from(HOP);
            self.phase = 0;
            self.running = true;
            self.plan(tempo);
        }
        if self.phase == HOP {
            let start = match self.search {
                Search::Done(start) => start,
                // The search did not finish: the grain goes where the tempo puts it.
                _ => self.nominal as u32,
            };
            self.out_start = self.in_start;
            self.in_start = start;
            self.phase = 0;
            self.plan(tempo);
        }
        let outgoing = self.out_start.wrapping_add(HOP + self.phase);
        let incoming = self.in_start.wrapping_add(self.phase);
        if !self.fill(outgoing, u32::MAX, &mut pull) || !self.fill(incoming, u32::MAX, &mut pull) {
            return None;
        }
        self.work(&mut pull);
        let (a, b) = (self.frame(outgoing), self.frame(incoming));
        let fade = self.fade[self.phase as usize];
        self.phase += 1;
        let mix = |a: i16, b: i16| (f32::from(a) + (f32::from(b) - f32::from(a)) * fade) as i16;
        Some(PcmFrame {
            left: mix(a.left, b.left),
            right: mix(a.right, b.right),
        })
    }

    /// A hop starts: the grain after the one fading in is placed `HOP` times the tempo
    /// on, and its search starts.
    fn plan(&mut self, tempo: f32) {
        let continuation = self.in_start.wrapping_add(HOP);
        self.nominal += f64::from(HOP) * f64::from(tempo);
        self.search = if (tempo - 1.0).abs() < 1.0e-4 {
            // At the track's own tempo the next grain simply continues this one, so the
            // output is the input, frame for frame.
            self.nominal = f64::from(continuation);
            Search::Done(continuation)
        } else {
            Search::Waiting {
                continuation,
                nominal: self.nominal as u32,
            }
        };
    }

    /// One step of the search for the next grain's start.
    fn work(&mut self, pull: &mut impl FnMut() -> Option<PcmFrame>) {
        self.search = match self.search {
            Search::Waiting {
                continuation,
                nominal,
            } => {
                // Starts a multiple of `THIN` apart, so that the mono copy holds them.
                let oldest = self.pulled.saturating_sub(WINDOW - GRAIN).max(HOP);
                let first = nominal.saturating_sub(SEEK).max(oldest) & !(THIN - 1);
                let need = (continuation + HOP).max(first + 2 * SEEK + HOP) + COARSE;
                if self.fill(need, PULL_BURST, pull) {
                    Search::Coarse {
                        continuation,
                        first,
                        next: first,
                        best: (f32::MIN, first),
                    }
                } else {
                    self.search
                }
            }
            Search::Coarse {
                continuation,
                first,
                next,
                best,
            } => {
                // The template starts on the mono copy's grid; a start found against it
                // keeps the same offset from the grid.
                let template = continuation & !(THIN - 1);
                let score = self.coarse_score(template, next);
                let best = if score > best.0 { (score, next) } else { best };
                if next + COARSE <= first + 2 * SEEK {
                    Search::Coarse {
                        continuation,
                        first,
                        next: next + COARSE,
                        best,
                    }
                } else {
                    let center = best.1 + (continuation - template);
                    Search::Fine {
                        continuation,
                        center,
                        offset: -FINE,
                        best: (f32::MIN, center),
                    }
                }
            }
            Search::Fine {
                continuation,
                center,
                offset,
                best,
            } => {
                let start = center.wrapping_add_signed(offset);
                let score = self.fine_score(continuation, start);
                let best = if score > best.0 { (score, start) } else { best };
                if offset < FINE {
                    Search::Fine {
                        continuation,
                        center,
                        offset: offset + 1,
                        best,
                    }
                } else {
                    Search::Done(best.1)
                }
            }
            done @ Search::Done(_) => done,
        };
    }

    /// How well the frames from `start` match those from `template`, on every
    /// `COARSE`th frame of the overlap: normalised cross-correlation, as its signed
    /// square so that no root is taken. Both are multiples of `THIN`.
    fn coarse_score(&self, template: u32, start: u32) -> f32 {
        let thin = |position: u32| self.thin[((position / THIN) & (WINDOW / THIN - 1)) as usize];
        let (mut correlation, mut energy) = (0.0f32, 1.0f32);
        for j in (0..HOP).step_by(COARSE as usize) {
            let (t, s) = (thin(template + j), thin(start + j));
            correlation += t * s;
            energy += s * s;
        }
        correlation * correlation.abs() / energy
    }

    /// As `coarse_score`, frame by frame from any start, on every `THIN`th frame.
    fn fine_score(&self, template: u32, start: u32) -> f32 {
        let (mut correlation, mut energy) = (0.0f32, 1.0f32);
        for j in (0..HOP).step_by(THIN as usize) {
            let t = mono(self.frame(template.wrapping_add(j)));
            let s = mono(self.frame(start.wrapping_add(j)));
            correlation += t * s;
            energy += s * s;
        }
        correlation * correlation.abs() / energy
    }

    /// Pulls from the reader until it holds `position`, at most `limit` frames now;
    /// whether it does.
    fn fill(
        &mut self,
        position: u32,
        limit: u32,
        pull: &mut impl FnMut() -> Option<PcmFrame>,
    ) -> bool {
        let mut left = limit;
        while (position.wrapping_sub(self.pulled) as i32) >= 0 {
            if left == 0 {
                return false;
            }
            let Some(frame) = pull() else {
                return false;
            };
            self.window[(self.pulled & (WINDOW - 1)) as usize] = frame;
            if self.pulled.is_multiple_of(THIN) {
                self.thin[((self.pulled / THIN) & (WINDOW / THIN - 1)) as usize] = mono(frame);
            }
            self.pulled = self.pulled.wrapping_add(1);
            left -= 1;
        }
        true
    }

    fn frame(&self, position: u32) -> PcmFrame {
        self.window[(position & (WINDOW - 1)) as usize]
    }

    /// Frames pulled from the reader since the start.
    #[cfg(test)]
    pub(crate) const fn pulled(&self) -> u64 {
        (self.pulled - HOP) as u64
    }
}

/// Both channels as one, for matching grains.
fn mono(frame: PcmFrame) -> f32 {
    f32::from(frame.left) + f32::from(frame.right)
}

/// `0.5 - 0.5 cos(pi p / HOP)` for each frame `p` of a hop: with the same curve falling,
/// the two grains' weights add up to one at every frame. The cosine comes from a rotation
/// by small steps, so no libm is needed.
fn raised_cosine() -> [f32; HOP as usize] {
    let step = core::f64::consts::PI / f64::from(HOP);
    let square = step * step;
    let (step_cos, step_sin) = (
        1.0 - square / 2.0 + square * square / 24.0,
        step * (1.0 - square / 6.0 + square * square / 120.0),
    );
    let (mut cos, mut sin) = (1.0f64, 0.0f64);
    let mut fade = [0.0; HOP as usize];
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
