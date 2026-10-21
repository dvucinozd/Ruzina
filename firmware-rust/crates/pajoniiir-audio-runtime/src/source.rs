//! Live source arbitration over generation-bound, bounded canonical PCM.
use pajoniiir_audio_dsp::PcmFrame;
use pajoniiir_audio_keylock::KeylockState;
use pajoniiir_audio_scratch::{ScratchConfig, ScratchState};

use crate::{
    history::{HistoryError, PcmHistory},
    pcm_stream::{PCM_STREAM_BLOCK_FRAMES, PcmStreamBlock, PcmStreamProgress},
};

pub const SOURCE_LOOKAHEAD_FRAMES: u64 = 2_048;
pub const SOURCE_HISTORY_FRAMES: usize = 16_384;
pub const SOURCE_PREFETCH_BLOCK_LIMIT: usize = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SourceControl {
    pub playing: bool,
    pub master_tempo: bool,
    pub scratch: bool,
    pub scratch_epoch: u32,
    pub censor: bool,
    pub tempo_factor: f32,
    pub output_sample_rate_hz: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SourceSample {
    pub frame: PcmFrame,
    pub advanced: u32,
    pub starved: bool,
    pub deferred: bool,
    pub silent: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IngestResult {
    Accepted,
    Stale,
    Backpressure,
    Fault,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Normal,
    Keylock,
    Scratch,
    Censor,
}

pub struct CanonicalSource<'a> {
    history: PcmHistory<'a>,
    generation: u32,
    sample_rate: u32,
    bound: bool,
    cursor: u64,
    fraction: f32,
    terminal: bool,
    faulted: bool,
    progress_dirty: bool,
    keylock: KeylockState,
    scratch: ScratchState,
    scratch_anchor: u64,
    scratch_expired: bool,
    pending_jog: i32,
    pending_jog_epoch: u32,
    scratch_epoch: u32,
    reverse_cursor: u64,
    reverse_fraction: f32,
    reverse_exhausted: bool,
    mode: Mode,
    stale_blocks: u32,
}

impl<'a> CanonicalSource<'a> {
    pub fn new(storage: &'a mut [PcmFrame]) -> Result<Self, HistoryError> {
        // Preserve lookahead plus a retained past. Small toy buffers are unsafe
        // for the keylock search window and are deliberately rejected.
        if storage.len() < SOURCE_LOOKAHEAD_FRAMES as usize * 2 {
            return Err(HistoryError::Capacity);
        }
        Ok(Self {
            history: PcmHistory::new(storage)?,
            generation: 0,
            sample_rate: 0,
            bound: false,
            cursor: 0,
            fraction: 0.0,
            terminal: false,
            faulted: false,
            progress_dirty: false,
            keylock: KeylockState::new(0),
            scratch: ScratchState::new(),
            scratch_anchor: 0,
            scratch_expired: false,
            pending_jog: 0,
            pending_jog_epoch: 0,
            scratch_epoch: 0,
            reverse_cursor: 0,
            reverse_fraction: 0.0,
            reverse_exhausted: false,
            mode: Mode::Normal,
            stale_blocks: 0,
        })
    }

    pub fn reset(&mut self, generation: u32) {
        self.history.reset(generation, 0);
        self.generation = generation;
        self.sample_rate = 0;
        self.bound = false;
        self.cursor = 0;
        self.fraction = 0.0;
        self.terminal = false;
        self.faulted = false;
        self.progress_dirty = false;
        self.keylock.reset(0);
        self.scratch.end();
        self.scratch_expired = false;
        self.pending_jog = 0;
        self.pending_jog_epoch = 0;
        self.scratch_epoch = 0;
        self.reverse_exhausted = false;
        self.mode = Mode::Normal;
    }

    pub fn ingest(&mut self, block: &PcmStreamBlock) -> IngestResult {
        if block.generation == 0 || block.generation != self.generation {
            self.stale_blocks = self.stale_blocks.saturating_add(1);
            return IngestResult::Stale;
        }
        if self.faulted {
            return IngestResult::Fault;
        }
        if block.frame_count() > PCM_STREAM_BLOCK_FRAMES
            || block.sample_rate_hz == 0
            || block.first_frame.checked_add(block.len as u64).is_none()
            || (self.bound
                && (block.first_frame != self.history.end()
                    || block.sample_rate_hz != self.sample_rate
                    || self.terminal))
            || (block.len == 0 && !block.eof && !block.faulted)
        {
            self.faulted = true;
            self.progress_dirty = true;
            return IngestResult::Fault;
        }
        if !self.bound {
            self.bound = true;
            self.cursor = block.first_frame;
            self.sample_rate = block.sample_rate_hz;
            self.history.reset(self.generation, block.first_frame);
            self.keylock.reset(self.cursor);
        }
        if block.end_frame().saturating_sub(self.cursor) > SOURCE_LOOKAHEAD_FRAMES {
            return IngestResult::Backpressure;
        }
        if self
            .history
            .append(self.generation, block.first_frame, block.frames())
            .is_err()
        {
            self.faulted = true;
        }
        self.terminal = block.eof || block.faulted;
        self.faulted |= block.faulted;
        // Terminal prefetch is not playback EOF. Faults, including empty-file
        // EOF, still need a control-plane completion without consuming samples.
        self.progress_dirty |= self.faulted || (self.terminal && self.cursor == self.history.end());
        if self.faulted {
            IngestResult::Fault
        } else {
            IngestResult::Accepted
        }
    }

    pub fn jog(&mut self, generation: u32, gesture_epoch: u32, ticks: i16) {
        if generation != 0 && generation == self.generation {
            if self.pending_jog_epoch != gesture_epoch {
                self.pending_jog = 0;
            }
            self.pending_jog_epoch = gesture_epoch;
            self.pending_jog = self.pending_jog.saturating_add(ticks as i32);
        }
    }

    pub fn configure_scratch(&mut self, config: ScratchConfig) {
        self.scratch.configure(config);
    }
    pub const fn position(&self) -> u64 {
        self.cursor
    }
    pub const fn generation(&self) -> u32 {
        self.generation
    }

    fn scratch_position(&self) -> Option<(u64, f32)> {
        if self.scratch_expired {
            return None;
        }
        let back = self.scratch.head_back();
        let whole = libm::ceilf(back) as u64;
        let frame = self.scratch_anchor.checked_sub(whole)?;
        (frame >= self.history.first() && frame < self.history.end())
            .then_some((frame, whole as f32 - back))
    }

    pub fn audible_position(&self) -> u64 {
        if self.mode == Mode::Scratch {
            self.scratch_position()
                .map(|head| head.0)
                .unwrap_or(self.cursor)
        } else {
            self.cursor
        }
    }

    fn finish_scratch(&mut self) {
        // Vinyl resumes at its read head, not at the background/slip cursor.
        // Censor has its own reverse cursor and returns to canonical time.
        if let Some((frame, fraction)) = self.scratch_position() {
            self.cursor = frame;
            self.fraction = fraction;
        }
        self.scratch.end();
        self.keylock.reset(self.cursor);
        self.pending_jog = 0;
        self.mode = Mode::Normal;
        self.progress_dirty = true;
    }
    pub const fn stale_blocks(&self) -> u32 {
        self.stale_blocks
    }

    pub fn next_sample(&mut self, control: SourceControl) -> SourceSample {
        if self.mode == Mode::Scratch
            && (!control.scratch || self.scratch_epoch != control.scratch_epoch)
        {
            self.finish_scratch();
        }
        let active = control.playing || control.scratch;
        if !active {
            if matches!(self.mode, Mode::Scratch | Mode::Censor) {
                self.scratch.end();
                self.keylock.reset(self.cursor);
                self.fraction = 0.0;
                self.mode = Mode::Normal;
                self.progress_dirty = true;
            }
            self.pending_jog = 0;
            return SourceSample::default();
        }
        if !self.bound || self.faulted || control.output_sample_rate_hz == 0 {
            return SourceSample {
                starved: active && !self.faulted && !self.terminal,
                silent: true,
                ..SourceSample::default()
            };
        }
        let ratio = self.sample_rate as f32 / control.output_sample_rate_hz as f32;
        let tempo = control.tempo_factor;
        let step = ratio * tempo;
        if !tempo.is_finite()
            || !(0.01..=16.0).contains(&step)
            || (control.master_tempo
                && !control.scratch
                && !control.censor
                && (!(0.50..=2.0).contains(&tempo) || !(0.25..=4.0).contains(&ratio)))
        {
            return SourceSample {
                deferred: true,
                silent: true,
                ..SourceSample::default()
            };
        }
        let mode = if control.scratch {
            Mode::Scratch
        } else if control.censor {
            Mode::Censor
        } else if control.master_tempo {
            Mode::Keylock
        } else {
            Mode::Normal
        };
        if mode != self.mode
            || (mode == Mode::Scratch && self.scratch_epoch != control.scratch_epoch)
        {
            if mode == Mode::Scratch {
                self.scratch_epoch = control.scratch_epoch;
                self.scratch_expired = false;
                self.scratch_anchor = self.history.end().saturating_sub(1);
                self.scratch
                    .seed(self.scratch_anchor.saturating_sub(self.cursor) as f32);
            } else {
                self.scratch.end();
                self.pending_jog = 0;
            }
            self.keylock.reset(self.cursor);
            self.fraction = 0.0;
            self.reverse_cursor = self.cursor;
            self.reverse_fraction = 0.0;
            self.reverse_exhausted = false;
            if mode == Mode::Censor {
                match self.cursor.checked_sub(1) {
                    Some(previous) => self.reverse_cursor = previous,
                    None => self.reverse_exhausted = true,
                }
            }
            self.mode = mode;
            self.progress_dirty = true;
        }
        let before = self.cursor;
        let audible_before = self.audible_position();
        if self.terminal
            && self.cursor == self.history.end()
            && !matches!(mode, Mode::Scratch | Mode::Censor)
        {
            return SourceSample {
                silent: true,
                ..SourceSample::default()
            };
        }
        let frame = match mode {
            Mode::Normal => {
                let frame = self.read_fractional(self.cursor, self.fraction);
                if frame.is_some() && self.can_advance(step) {
                    self.advance(step);
                    frame
                } else {
                    None
                }
            }
            Mode::Keylock => {
                if !self.can_advance(step) {
                    return SourceSample {
                        starved: true,
                        silent: true,
                        ..SourceSample::default()
                    };
                }
                self.keylock.configure(tempo, ratio);
                let history = &self.history;
                let generation = self.generation;
                let terminal = self.terminal;
                let end = history.end();
                self.keylock
                    .next(&mut |seq| {
                        history
                            .read(generation, seq)
                            .or_else(|| (terminal && seq >= end).then_some(PcmFrame::default()))
                    })
                    .map(|sample| {
                        self.cursor = sample.play_seq.min(self.history.end());
                        sample.frame
                    })
            }
            Mode::Scratch => {
                if self.pending_jog_epoch != control.scratch_epoch {
                    self.pending_jog = 0;
                }
                let end = self.history.end().saturating_sub(1);
                let shift = end.saturating_sub(self.scratch_anchor);
                self.scratch
                    .advance_reference(shift.min(u32::MAX as u64) as u32);
                self.scratch_anchor = end;
                let ticks = self.pending_jog.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                self.pending_jog -= ticks as i32;
                self.scratch.jog(ticks);
                let history = &self.history;
                let generation = self.generation;
                let first = history.first();
                let filled = end
                    .saturating_sub(first)
                    .saturating_add(1)
                    .min(u32::MAX as u64) as u32;
                self.scratch_expired |= self.scratch.head_back() > filled.saturating_sub(1) as f32;
                let frame = if self.scratch_expired {
                    None
                } else {
                    self.scratch.render(filled, |back| {
                        end.checked_sub(back as u64)
                            .and_then(|seq| history.read(generation, seq))
                    })
                };
                if control.playing && self.can_advance(step) {
                    self.advance(step);
                }
                frame
            }
            Mode::Censor => {
                let frame = if self.reverse_exhausted {
                    None
                } else {
                    self.read_fractional(self.reverse_cursor, self.reverse_fraction)
                };
                if frame.is_some() {
                    let movement = step - self.reverse_fraction;
                    if movement > 0.0 {
                        let whole = libm::ceilf(movement) as u64;
                        if let Some(next) = self.reverse_cursor.checked_sub(whole) {
                            self.reverse_cursor = next;
                            self.reverse_fraction = whole as f32 - movement;
                        } else {
                            self.reverse_exhausted = true;
                        }
                    } else {
                        self.reverse_fraction -= step;
                    }
                }
                if self.can_advance(step) {
                    self.advance(step);
                }
                frame
            }
        };
        self.progress_dirty |= self.cursor != before || self.audible_position() != audible_before;
        let at_end = self.terminal && self.cursor == self.history.end();
        SourceSample {
            frame: frame.unwrap_or_default(),
            advanced: self.cursor.saturating_sub(before).min(u32::MAX as u64) as u32,
            // A stationary scratch head intentionally outputs silence. Missing
            // canonical data and expired reads remain observable separately.
            starved: (mode == Mode::Scratch && self.scratch_expired)
                || (frame.is_none()
                    && !at_end
                    && !(mode == Mode::Scratch
                        && self.scratch.velocity().abs()
                            < pajoniiir_audio_scratch::SILENCE_VELOCITY)),
            deferred: false,
            silent: frame.is_none(),
        }
    }

    fn read_fractional(&self, frame: u64, fraction: f32) -> Option<PcmFrame> {
        if self.terminal && frame >= self.history.end() {
            return Some(PcmFrame::default());
        }
        let a = self.history.read(self.generation, frame)?;
        if fraction == 0.0 {
            return Some(a);
        }
        let next = frame.checked_add(1)?;
        let b = self.history.read(self.generation, next).or_else(|| {
            (self.terminal && next == self.history.end()).then_some(PcmFrame::default())
        })?;
        Some(PcmFrame {
            left: (a.left as f32 + (b.left as f32 - a.left as f32) * fraction) as i16,
            right: (a.right as f32 + (b.right as f32 - a.right as f32) * fraction) as i16,
        })
    }

    fn can_advance(&self, step: f32) -> bool {
        let advance = libm::ceilf(self.fraction + step) as u64;
        self.terminal
            || self
                .cursor
                .checked_add(advance)
                .is_some_and(|next| next <= self.history.end())
    }

    fn advance(&mut self, step: f32) {
        let fraction = self.fraction + step;
        let advance = fraction as u64;
        self.cursor = self.cursor.saturating_add(advance).min(self.history.end());
        self.fraction = fraction - advance as f32;
    }

    pub fn take_progress(&mut self, control: SourceControl) -> Option<PcmStreamProgress> {
        if !self.progress_dirty || self.generation == 0 {
            return None;
        }
        self.progress_dirty = false;
        Some(PcmStreamProgress {
            generation: self.generation,
            end_frame: self.audible_position(),
            sample_rate_hz: self.sample_rate,
            eof: self.faulted
                || (self.terminal
                    && self.cursor == self.history.end()
                    && !control.scratch
                    && !control.censor),
            faulted: self.faulted,
        })
    }
}
