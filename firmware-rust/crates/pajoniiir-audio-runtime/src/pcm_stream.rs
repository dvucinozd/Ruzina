use pajoniiir_audio_dsp::PcmFrame;

pub const PCM_STREAM_BLOCK_FRAMES: usize = 256;

#[derive(Clone, Debug, PartialEq)]
pub struct PcmStreamBlock {
    pub generation: u32,
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
        if let Some(generation) = self.generation {
            if generation != block.generation {
                self.stale_blocks = self.stale_blocks.saturating_add(1);
                return PcmBlockDisposition::Stale;
            }
        } else {
            self.generation = Some(block.generation);
        }

        if block.sample_rate_hz == 0 {
            self.ended = true;
            return PcmBlockDisposition::Fault {
                position_ms: self.position_ms(),
            };
        }

        self.sample_rate_hz = block.sample_rate_hz;
        self.position_frame = block.end_frame();
        self.consumed_blocks = self.consumed_blocks.saturating_add(1);

        if block.faulted {
            self.ended = true;
            return PcmBlockDisposition::Fault {
                position_ms: self.position_ms(),
            };
        }

        if block.eof {
            self.ended = true;
        }

        PcmBlockDisposition::Accepted {
            frames: block.len,
            position_ms: self.position_ms(),
            eof: block.eof,
        }
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
    let frame = u64::from(position_ms)
        .saturating_mul(u64::from(sample_rate_hz))
        / 1_000;
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

        assert_eq!(cursor.accept(&block(8, 0, 256, false)), PcmBlockDisposition::Stale);
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
    fn stream_generation_skips_zero_on_wrap() {
        assert_eq!(next_stream_generation(0), 1);
        assert_eq!(next_stream_generation(u32::MAX), 1);
    }
}
