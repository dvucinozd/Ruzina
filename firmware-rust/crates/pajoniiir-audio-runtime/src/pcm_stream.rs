use crate::timeline::PcmTimeline;
use core::sync::atomic::{AtomicU32, Ordering};
use pajoniiir_audio_dsp::PcmFrame;

/// Preparation is not activation. The product owner commits the exact token
/// returned by prepare; seeks temporarily revoke it while replacing the stream.
pub struct StreamActivation {
    prepared: AtomicU32,
    active: AtomicU32,
}
impl StreamActivation {
    pub const fn new() -> Self {
        Self {
            prepared: AtomicU32::new(0),
            active: AtomicU32::new(0),
        }
    }
    pub fn invalidate(&self) {
        self.active.store(0, Ordering::Release);
        self.prepared.store(0, Ordering::Release);
    }
    pub fn prepared(&self, generation: u32) {
        self.prepared.store(generation, Ordering::Release);
    }
    pub fn current(&self) -> u32 {
        self.prepared.load(Ordering::Acquire)
    }
    pub fn active(&self) -> u32 {
        let active = self.active.load(Ordering::Acquire);
        if self.current() == active { active } else { 0 }
    }
    pub fn accepts_progress(&self, generation: u32) -> bool {
        generation != 0 && self.active() == generation
    }
    pub fn activate(&self, generation: u32) -> bool {
        if generation == 0 || self.current() != generation {
            return false;
        }
        self.active.store(generation, Ordering::Release);
        if self.current() != generation {
            self.active.store(0, Ordering::Release);
            return false;
        }
        true
    }
}
impl Default for StreamActivation {
    fn default() -> Self {
        Self::new()
    }
}

pub const PCM_STREAM_BLOCK_FRAMES: usize = 256;

#[derive(Clone, Debug, PartialEq)]
pub struct PcmStreamBlock {
    pub generation: u32,
    pub timeline: PcmTimeline,
    pub first_frame: u64,
    pub sample_rate_hz: u32,
    pub len: u16,
    pub eof: bool,
    pub faulted: bool,
    pub frames: [PcmFrame; PCM_STREAM_BLOCK_FRAMES],
}

impl PcmStreamBlock {
    pub const fn empty(generation: u32, first_frame: u64, sample_rate_hz: u32) -> Self {
        Self {
            generation,
            timeline: PcmTimeline::linear(),
            first_frame,
            sample_rate_hz,
            len: 0,
            eof: false,
            faulted: false,
            frames: [PcmFrame { left: 0, right: 0 }; PCM_STREAM_BLOCK_FRAMES],
        }
    }

    pub const fn frame_count(&self) -> usize {
        self.len as usize
    }

    pub fn frames(&self) -> &[PcmFrame] {
        &self.frames[..self.frame_count()]
    }

    pub fn frames_mut(&mut self) -> &mut [PcmFrame] {
        &mut self.frames[..]
    }

    pub const fn end_frame(&self) -> u64 {
        self.first_frame.saturating_add(self.len as u64)
    }

    pub fn set_len(&mut self, len: usize) -> bool {
        if len > PCM_STREAM_BLOCK_FRAMES {
            return false;
        }
        self.len = len as u16;
        true
    }

    pub fn mark_eof(&mut self) {
        self.eof = true;
    }

    pub fn mark_faulted(&mut self) {
        self.faulted = true;
        self.eof = true;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcmStreamProgress {
    pub generation: u32,
    pub end_frame: u64,
    pub sample_rate_hz: u32,
    pub eof: bool,
    pub faulted: bool,
}

/// Bounded latest-progress publication. Congestion must not stop render.
#[derive(Clone, Copy, Debug, Default)]
pub struct PendingRenderProgress {
    pending: Option<(PcmStreamProgress, u16)>,
}

impl PendingRenderProgress {
    pub fn clear(&mut self) {
        self.pending = None;
    }
    pub const fn peek(&self) -> Option<(PcmStreamProgress, u16)> {
        self.pending
    }

    pub fn record(&mut self, mut progress: PcmStreamProgress, frames: u16) {
        let mut frames = frames;
        if let Some((previous, count)) = self.pending
            && previous.generation == progress.generation
        {
            // A vinyl release can move the audible cursor backwards within
            // the same retained source. Latest position/EOF are authoritative.
            progress.eof |= previous.faulted;
            progress.faulted |= previous.faulted;
            frames = frames.saturating_add(count);
        }
        self.pending = Some((progress, frames));
    }
}

pub struct PcmBlockFeed {
    current: Option<PcmStreamBlock>,
    offset: usize,
    generation: Option<u32>,
    next_frame: Option<u64>,
    sample_rate_hz: u32,
    progress_dirty: bool,
    terminal_eof: bool,
    terminal_faulted: bool,
    stale_blocks: u32,
    discontinuities: u32,
}

impl PcmBlockFeed {
    pub const fn new() -> Self {
        Self {
            current: None,
            offset: 0,
            generation: None,
            next_frame: None,
            sample_rate_hz: 0,
            progress_dirty: false,
            terminal_eof: false,
            terminal_faulted: false,
            stale_blocks: 0,
            discontinuities: 0,
        }
    }

    pub fn reset(&mut self, expected_generation: Option<u32>) {
        self.current = None;
        self.offset = 0;
        self.generation = expected_generation;
        self.next_frame = None;
        self.sample_rate_hz = 0;
        self.progress_dirty = false;
        self.terminal_eof = false;
        self.terminal_faulted = false;
    }

    pub const fn generation(&self) -> Option<u32> {
        self.generation
    }

    pub const fn stale_blocks(&self) -> u32 {
        self.stale_blocks
    }

    pub const fn discontinuities(&self) -> u32 {
        self.discontinuities
    }

    pub fn pop<F>(&mut self, mut next_block: F) -> Option<PcmFrame>
    where
        F: FnMut() -> Option<PcmStreamBlock>,
    {
        loop {
            if self.current.is_none() {
                let block = next_block()?;
                if block.generation == 0 {
                    self.discontinuities = self.discontinuities.saturating_add(1);
                    self.mark_fault(block.generation, block.first_frame, block.sample_rate_hz);
                    return None;
                }

                if let Some(expected) = self.generation {
                    if block.generation != expected {
                        self.stale_blocks = self.stale_blocks.saturating_add(1);
                        continue;
                    }
                } else {
                    self.generation = Some(block.generation);
                }

                if self.sample_rate_hz != 0 && self.sample_rate_hz != block.sample_rate_hz {
                    self.discontinuities = self.discontinuities.saturating_add(1);
                    self.mark_fault(block.generation, block.first_frame, block.sample_rate_hz);
                    return None;
                }
                self.sample_rate_hz = block.sample_rate_hz;

                if let Some(expected_frame) = self.next_frame
                    && block.first_frame != expected_frame
                {
                    self.discontinuities = self.discontinuities.saturating_add(1);
                    self.mark_fault(block.generation, expected_frame, block.sample_rate_hz);
                    return None;
                }

                if block.len == 0 {
                    self.next_frame = Some(block.first_frame);
                    self.progress_dirty = true;
                    self.terminal_eof = block.eof;
                    self.terminal_faulted = block.faulted;
                    return None;
                }

                self.offset = 0;
                self.current = Some(block);
            }

            let block = self.current.as_ref()?;
            let frame = block.frames[self.offset];
            self.offset += 1;
            self.next_frame = Some(block.first_frame.saturating_add(self.offset as u64));
            self.progress_dirty = true;

            if self.offset >= block.frame_count() {
                self.terminal_eof = block.eof;
                self.terminal_faulted = block.faulted;
                self.current = None;
                self.offset = 0;
            }
            return Some(frame);
        }
    }

    pub fn take_progress(&mut self) -> Option<PcmStreamProgress> {
        if !self.progress_dirty {
            return None;
        }
        self.progress_dirty = false;
        let progress = PcmStreamProgress {
            generation: self.generation?,
            end_frame: self.next_frame.unwrap_or(0),
            sample_rate_hz: self.sample_rate_hz,
            eof: self.terminal_eof,
            faulted: self.terminal_faulted,
        };
        if progress.eof || progress.faulted {
            self.terminal_eof = false;
            self.terminal_faulted = false;
        }
        Some(progress)
    }

    fn mark_fault(&mut self, generation: u32, end_frame: u64, sample_rate_hz: u32) {
        if generation != 0 {
            self.generation = Some(generation);
        }
        self.next_frame = Some(end_frame);
        self.sample_rate_hz = sample_rate_hz;
        self.current = None;
        self.offset = 0;
        self.progress_dirty = true;
        self.terminal_eof = true;
        self.terminal_faulted = true;
    }
}

impl Default for PcmBlockFeed {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PcmBlockDisposition {
    Accepted {
        frames: u16,
        position_ms: u32,
        eof: bool,
    },
    Stale,
    Fault {
        position_ms: u32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcmTransportCursor {
    generation: Option<u32>,
    position_frame: u64,
    sample_rate_hz: u32,
    consumed_blocks: u32,
    stale_blocks: u32,
    ended: bool,
}

impl PcmTransportCursor {
    pub const fn new() -> Self {
        Self {
            generation: None,
            position_frame: 0,
            sample_rate_hz: 0,
            consumed_blocks: 0,
            stale_blocks: 0,
            ended: false,
        }
    }

    pub fn begin_playback(&mut self) {
        self.generation = None;
        self.ended = false;
    }

    pub fn bind_seek(&mut self, generation: u32, frame: u64, sample_rate_hz: u32) {
        self.generation = Some(generation);
        self.position_frame = frame;
        self.sample_rate_hz = sample_rate_hz;
        self.ended = false;
    }

    pub fn accept(&mut self, block: &PcmStreamBlock) -> PcmBlockDisposition {
        self.accept_progress(
            PcmStreamProgress {
                generation: block.generation,
                end_frame: block.timeline.media_frame(block.end_frame()).unwrap_or(0),
                sample_rate_hz: block.sample_rate_hz,
                eof: block.eof,
                faulted: block.faulted || block.timeline.media_frame(block.end_frame()).is_none(),
            },
            block.len,
        )
    }

    pub fn accept_progress(
        &mut self,
        progress: PcmStreamProgress,
        frames: u16,
    ) -> PcmBlockDisposition {
        if let Some(generation) = self.generation {
            if generation != progress.generation {
                self.stale_blocks = self.stale_blocks.saturating_add(1);
                return PcmBlockDisposition::Stale;
            }
        } else {
            self.generation = Some(progress.generation);
        }

        if progress.sample_rate_hz == 0 {
            self.ended = true;
            return PcmBlockDisposition::Fault {
                position_ms: self.position_ms(),
            };
        }

        self.sample_rate_hz = progress.sample_rate_hz;
        self.position_frame = progress.end_frame;
        self.consumed_blocks = self.consumed_blocks.saturating_add(1);

        if progress.faulted {
            self.ended = true;
            return PcmBlockDisposition::Fault {
                position_ms: self.position_ms(),
            };
        }

        self.ended = progress.eof;

        PcmBlockDisposition::Accepted {
            frames,
            position_ms: self.position_ms(),
            eof: progress.eof,
        }
    }

    /// The product activation token, rather than a Play command, determines
    /// which source may publish time. This also supports a newly loaded deck
    /// being scratched while paused. Reject obsolete progress before rebinding.
    pub fn accept_active_progress(
        &mut self,
        progress: PcmStreamProgress,
        frames: u16,
        active_generation: u32,
    ) -> PcmBlockDisposition {
        if active_generation == 0 || progress.generation != active_generation {
            self.stale_blocks = self.stale_blocks.saturating_add(1);
            return PcmBlockDisposition::Stale;
        }
        if self.generation != Some(active_generation) {
            self.generation = Some(active_generation);
            self.ended = false;
        }
        self.accept_progress(progress, frames)
    }

    pub const fn generation(&self) -> Option<u32> {
        self.generation
    }

    pub const fn position_frame(&self) -> u64 {
        self.position_frame
    }

    pub const fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }

    pub const fn consumed_blocks(&self) -> u32 {
        self.consumed_blocks
    }

    pub const fn stale_blocks(&self) -> u32 {
        self.stale_blocks
    }

    pub const fn ended(&self) -> bool {
        self.ended
    }

    pub fn position_ms(&self) -> u32 {
        frame_to_ms(self.position_frame, self.sample_rate_hz)
    }
}

impl Default for PcmTransportCursor {
    fn default() -> Self {
        Self::new()
    }
}

pub fn frame_from_ms(position_ms: u32, sample_rate_hz: u32, frame_count: u64) -> u64 {
    if sample_rate_hz == 0 {
        return 0;
    }
    let frame = u64::from(position_ms).saturating_mul(u64::from(sample_rate_hz)) / 1_000;
    frame.min(frame_count)
}

pub fn frame_to_ms(frame: u64, sample_rate_hz: u32) -> u32 {
    if sample_rate_hz == 0 {
        return 0;
    }
    let millis = frame.saturating_mul(1_000) / u64::from(sample_rate_hz);
    millis.min(u64::from(u32::MAX)) as u32
}

pub const fn next_stream_generation(current: u32) -> u32 {
    let next = current.wrapping_add(1);
    if next == 0 { 1 } else { next }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(generation: u32, first: u64, len: usize, eof: bool) -> PcmStreamBlock {
        let mut block = PcmStreamBlock::empty(generation, first, 48_000);
        assert!(block.set_len(len));
        block.eof = eof;
        block
    }

    #[test]
    fn frame_time_conversion_is_bounded_and_deterministic() {
        assert_eq!(frame_from_ms(1_000, 48_000, 100_000), 48_000);
        assert_eq!(frame_to_ms(48_000, 48_000), 1_000);
        assert_eq!(frame_from_ms(10_000, 48_000, 12_000), 12_000);
        assert_eq!(frame_to_ms(u64::MAX, 48_000), u32::MAX);
        assert_eq!(frame_from_ms(1_000, 0, 100), 0);
        assert_eq!(frame_to_ms(100, 0), 0);
    }

    #[test]
    fn transport_adopts_first_generation_and_advances_position() {
        let mut cursor = PcmTransportCursor::new();
        cursor.begin_playback();

        assert_eq!(
            cursor.accept(&block(7, 0, 240, false)),
            PcmBlockDisposition::Accepted {
                frames: 240,
                position_ms: 5,
                eof: false,
            }
        );
        assert_eq!(cursor.generation(), Some(7));
        assert_eq!(cursor.position_frame(), 240);
        assert_eq!(cursor.consumed_blocks(), 1);
    }

    #[test]
    fn stale_generation_never_advances_transport() {
        let mut cursor = PcmTransportCursor::new();
        cursor.bind_seek(9, 4_800, 48_000);

        assert_eq!(
            cursor.accept(&block(8, 0, 256, false)),
            PcmBlockDisposition::Stale
        );
        assert_eq!(cursor.position_frame(), 4_800);
        assert_eq!(cursor.stale_blocks(), 1);
        assert_eq!(cursor.consumed_blocks(), 0);
    }

    #[test]
    fn seek_binding_accepts_only_new_generation() {
        let mut cursor = PcmTransportCursor::new();
        cursor.bind_seek(11, 24_000, 48_000);

        assert_eq!(
            cursor.accept(&block(11, 24_000, 240, false)),
            PcmBlockDisposition::Accepted {
                frames: 240,
                position_ms: 505,
                eof: false,
            }
        );
    }

    #[test]
    fn eof_and_fault_stop_cursor_at_reported_position() {
        let mut cursor = PcmTransportCursor::new();
        cursor.begin_playback();
        assert_eq!(
            cursor.accept(&block(1, 47_900, 100, true)),
            PcmBlockDisposition::Accepted {
                frames: 100,
                position_ms: 1_000,
                eof: true,
            }
        );
        assert!(cursor.ended());

        let mut cursor = PcmTransportCursor::new();
        let mut fault = block(2, 24_000, 0, true);
        fault.mark_faulted();
        assert_eq!(
            cursor.accept(&fault),
            PcmBlockDisposition::Fault { position_ms: 500 }
        );
        assert!(cursor.ended());
    }

    #[test]
    fn block_capacity_is_fail_closed() {
        let mut block = PcmStreamBlock::empty(1, 0, 44_100);
        assert!(block.set_len(PCM_STREAM_BLOCK_FRAMES));
        assert!(!block.set_len(PCM_STREAM_BLOCK_FRAMES + 1));
        assert_eq!(block.frame_count(), PCM_STREAM_BLOCK_FRAMES);
    }

    #[test]
    fn block_feed_preserves_contiguous_frame_coordinates() {
        let mut feed = PcmBlockFeed::new();
        let mut first = block(4, 100, 2, false);
        first.frames[0] = PcmFrame {
            left: 10,
            right: -10,
        };
        first.frames[1] = PcmFrame {
            left: 20,
            right: -20,
        };
        let mut second = block(4, 102, 1, true);
        second.frames[0] = PcmFrame {
            left: 30,
            right: -30,
        };
        let mut blocks = [Some(first), Some(second)].into_iter();

        assert_eq!(
            feed.pop(|| blocks.next().flatten()),
            Some(PcmFrame {
                left: 10,
                right: -10
            })
        );
        assert_eq!(
            feed.pop(|| blocks.next().flatten()),
            Some(PcmFrame {
                left: 20,
                right: -20
            })
        );
        let progress = feed.take_progress().unwrap();
        assert_eq!(progress.generation, 4);
        assert_eq!(progress.end_frame, 102);
        assert!(!progress.eof);

        assert_eq!(
            feed.pop(|| blocks.next().flatten()),
            Some(PcmFrame {
                left: 30,
                right: -30
            })
        );
        let progress = feed.take_progress().unwrap();
        assert_eq!(progress.end_frame, 103);
        assert!(progress.eof);
        assert!(!progress.faulted);
    }

    #[test]
    fn block_feed_rejects_stale_generation_and_discontinuity() {
        let mut feed = PcmBlockFeed::new();
        feed.reset(Some(7));

        let stale = block(6, 0, 1, false);
        let valid = block(7, 10, 1, false);
        let mut blocks = [Some(stale), Some(valid)].into_iter();
        assert!(feed.pop(|| blocks.next().flatten()).is_some());
        assert_eq!(feed.stale_blocks(), 1);
        assert_eq!(feed.take_progress().unwrap().end_frame, 11);

        let gap = block(7, 20, 1, false);
        assert_eq!(feed.pop(|| Some(gap.clone())), None);
        let progress = feed.take_progress().unwrap();
        assert!(progress.faulted);
        assert!(progress.eof);
        assert_eq!(feed.discontinuities(), 1);
    }

    #[test]
    fn transport_progress_rejects_old_render_generation() {
        let mut cursor = PcmTransportCursor::new();
        cursor.bind_seek(12, 4_800, 48_000);
        assert_eq!(
            cursor.accept_progress(
                PcmStreamProgress {
                    generation: 11,
                    end_frame: 5_000,
                    sample_rate_hz: 48_000,
                    eof: false,
                    faulted: false,
                },
                200,
            ),
            PcmBlockDisposition::Stale
        );
        assert_eq!(cursor.position_frame(), 4_800);
    }

    #[test]
    fn stream_generation_skips_zero_on_wrap() {
        assert_eq!(next_stream_generation(0), 1);
        assert_eq!(next_stream_generation(u32::MAX), 1);
    }

    #[test]
    fn paused_track_replacement_adopts_only_the_active_source_without_play() {
        let mut cursor = PcmTransportCursor::new();
        cursor.bind_seek(7, 48_000, 48_000);
        let progress = PcmStreamProgress {
            generation: 8,
            end_frame: 2_400,
            sample_rate_hz: 48_000,
            eof: false,
            faulted: false,
        };
        assert_eq!(
            cursor.accept_active_progress(progress, 0, 0),
            PcmBlockDisposition::Stale
        );
        assert_eq!(cursor.generation(), Some(7));
        assert!(matches!(
            cursor.accept_active_progress(progress, 0, 8),
            PcmBlockDisposition::Accepted {
                position_ms: 50,
                ..
            }
        ));
        assert_eq!(cursor.generation(), Some(8));
        assert_eq!(
            cursor.accept_active_progress(
                PcmStreamProgress {
                    generation: 7,
                    ..progress
                },
                0,
                8
            ),
            PcmBlockDisposition::Stale
        );
        assert_eq!(cursor.position_frame(), 2_400);
        assert_eq!(cursor.generation(), Some(8));
    }

    #[test]
    fn source_prepare_requires_exact_product_commit_and_invalidates_old_activation() {
        let gate = StreamActivation::new();
        gate.prepared(1);
        assert_eq!(gate.current(), 1);
        assert_eq!(gate.active(), 0);
        assert!(!gate.activate(0));
        assert!(gate.activate(1));
        gate.invalidate();
        assert_eq!(gate.active(), 0);
        assert!(!gate.accepts_progress(1));
        gate.prepared(2);
        assert!(!gate.activate(1));
        assert_eq!(gate.active(), 0);
        assert!(gate.activate(2));
        assert!(gate.accepts_progress(2));
        assert!(!gate.accepts_progress(1));
        gate.prepared(3);
        assert_eq!(gate.active(), 0);
    }

    #[test]
    fn latest_vinyl_position_can_rewind_without_stale_prefetched_eof() {
        let mut pending = PendingRenderProgress::default();
        let progress = PcmStreamProgress {
            generation: 1,
            end_frame: 1_000,
            sample_rate_hz: 48_000,
            eof: true,
            faulted: false,
        };
        pending.record(progress, 128);
        pending.record(
            PcmStreamProgress {
                end_frame: 800,
                eof: false,
                ..progress
            },
            0,
        );
        let (latest, _) = pending.peek().unwrap();
        assert_eq!(latest.end_frame, 800);
        assert!(!latest.eof);
        let mut transport = PcmTransportCursor::new();
        transport.accept_progress(progress, 128);
        assert!(matches!(
            transport.accept_progress(latest, 0),
            PcmBlockDisposition::Accepted { eof: false, .. }
        ));
        assert_eq!(transport.position_frame(), 800);
        assert!(!transport.ended());
    }

    #[test]
    fn congested_progress_coalesces_without_losing_terminal_state_or_crossing_generations() {
        let mut mailbox = PendingRenderProgress::default();
        let initial = PcmStreamProgress {
            generation: 7,
            end_frame: 128,
            sample_rate_hz: 48_000,
            eof: false,
            faulted: false,
        };
        mailbox.record(initial, 128);
        mailbox.record(
            PcmStreamProgress {
                end_frame: 256,
                eof: true,
                faulted: true,
                ..initial
            },
            128,
        );
        mailbox.record(
            PcmStreamProgress {
                end_frame: 256,
                ..initial
            },
            0,
        );
        let (terminal, frames) = mailbox.peek().unwrap();
        assert_eq!(frames, 256);
        assert!(terminal.eof && terminal.faulted);
        mailbox.record(
            PcmStreamProgress {
                generation: 8,
                end_frame: 10_000,
                ..initial
            },
            0,
        );
        let (new, frames) = mailbox.peek().unwrap();
        assert_eq!(new.generation, 8);
        assert!(!new.eof && !new.faulted);
        assert_eq!(frames, 0);
        mailbox.clear();
        assert!(mailbox.peek().is_none());
    }
}
