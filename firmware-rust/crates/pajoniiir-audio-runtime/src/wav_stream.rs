//! Bounded WAV producer shared by the filesystem owner and host fault tests.
//! Reads may overshoot a loop end; only its exact PCM prefix is published.
use crate::{
    pcm_stream::{PCM_STREAM_BLOCK_FRAMES, PcmStreamBlock, frame_from_ms},
    timeline::{FrameLoop, PcmTimeline},
    wav::{WavPcm16Decoder, WavPcm16SourceDescriptor},
};
use pajoniiir_audio_dsp::PcmFrame;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WavStreamChange {
    SeekMilliseconds(u32),
    Loop {
        position_frame: u64,
        region_ms: Option<(u32, u32)>,
    },
}

pub struct WavStreamDecoder {
    descriptor: WavPcm16SourceDescriptor,
    decoder: WavPcm16Decoder,
    generation: u32,
    timeline: PcmTimeline,
    next_frame: u64,
    media_frame: u64,
    bytes_to_boundary: u64,
    short_loop: [PcmFrame; PCM_STREAM_BLOCK_FRAMES],
    short_valid: [bool; PCM_STREAM_BLOCK_FRAMES],
    short_filled: usize,
}

impl WavStreamDecoder {
    pub fn new(
        descriptor: WavPcm16SourceDescriptor,
        generation: u32,
        frame: u64,
        region: Option<FrameLoop>,
    ) -> Option<Self> {
        let _ = WavPcm16Decoder::new(descriptor.format, descriptor.data_bytes).ok()?;
        descriptor.data_offset.checked_add(descriptor.data_bytes)?;
        if generation == 0
            || frame > descriptor.frame_count()
            || region.is_some_and(|r| r.end() > descriptor.frame_count())
        {
            return None;
        }
        let timeline = PcmTimeline::at(frame, region);
        let media_frame = timeline.origin();
        let remaining = descriptor
            .data_bytes
            .checked_sub(media_frame.checked_mul(u64::from(descriptor.format.block_align))?)?;
        Some(Self {
            decoder: WavPcm16Decoder::new(descriptor.format, remaining).ok()?,
            descriptor,
            generation,
            timeline,
            next_frame: media_frame,
            media_frame,
            bytes_to_boundary: (region.map_or(descriptor.frame_count(), FrameLoop::end)
                - media_frame)
                * u64::from(descriptor.format.block_align),
            short_loop: [PcmFrame::default(); PCM_STREAM_BLOCK_FRAMES],
            short_valid: [false; PCM_STREAM_BLOCK_FRAMES],
            short_filled: 0,
        })
    }

    pub fn changed(&self, generation: u32, change: WavStreamChange) -> Option<Self> {
        let (frame, region) = match change {
            WavStreamChange::SeekMilliseconds(ms) => (
                frame_from_ms(
                    ms,
                    self.descriptor.format.sample_rate_hz,
                    self.descriptor.frame_count(),
                ),
                self.timeline.loop_region(),
            ),
            WavStreamChange::Loop {
                position_frame,
                region_ms,
            } => (
                position_frame.min(self.descriptor.frame_count()),
                match region_ms {
                    Some((start, end)) => Some(FrameLoop::from_ms(
                        start,
                        end,
                        self.descriptor.format.sample_rate_hz,
                        self.descriptor.frame_count(),
                    )?),
                    None => None,
                },
            ),
        };
        Self::new(self.descriptor, generation, frame, region)
    }

    pub const fn generation(&self) -> u32 {
        self.generation
    }
    pub const fn media_frame(&self) -> u64 {
        self.media_frame
    }
    pub const fn timeline(&self) -> PcmTimeline {
        self.timeline
    }

    /// A complete loop of at most one PCM block is retained in a fixed cache.
    /// This prevents one-frame loops from filling every queue slot with a
    /// one-frame block or requiring a filesystem seek for every output sample.
    pub fn cached_block(&mut self) -> Option<PcmStreamBlock> {
        let region = self.timeline.loop_region()?;
        let length = region.end() - region.start();
        if length > PCM_STREAM_BLOCK_FRAMES as u64 || self.short_filled != length as usize {
            return None;
        }
        let mut block = self.empty_block();
        let Some(end) = self.next_frame.checked_add(PCM_STREAM_BLOCK_FRAMES as u64) else {
            return Some(self.fault_block());
        };
        for (offset, sample) in block.frames_mut().iter_mut().enumerate() {
            let frame = self.timeline.media_frame(self.next_frame + offset as u64)?;
            *sample = self.short_loop[(frame - region.start()) as usize];
        }
        block.set_len(PCM_STREAM_BLOCK_FRAMES);
        self.next_frame = end;
        self.media_frame = self.timeline.media_frame(end)?;
        Some(block)
    }

    /// Seek the existing file to this frame before the next read. No generation
    /// changes at a steady wrap, and no decoder read crosses the loop boundary.
    pub fn rewind_frame(&self) -> Option<u64> {
        self.timeline
            .loop_region()
            .filter(|region| self.media_frame == region.end())
            .map(FrameLoop::start)
    }

    /// Called only after a successful filesystem seek to rewind_frame().
    pub fn complete_rewind(&mut self) -> bool {
        let Some(frame) = self.rewind_frame() else {
            return false;
        };
        let remaining =
            self.descriptor.data_bytes - frame * u64::from(self.descriptor.format.block_align);
        let Ok(decoder) = WavPcm16Decoder::new(self.descriptor.format, remaining) else {
            return false;
        };
        self.decoder = decoder;
        self.media_frame = frame;
        self.bytes_to_boundary = (self.timeline.loop_region().expect("rewind region").end()
            - frame)
            * u64::from(self.descriptor.format.block_align);
        true
    }

    pub fn fault_block(&self) -> PcmStreamBlock {
        let mut block = self.empty_block();
        block.mark_faulted();
        block
    }

    fn empty_block(&self) -> PcmStreamBlock {
        let mut block = PcmStreamBlock::empty(
            self.generation,
            self.next_frame,
            self.descriptor.format.sample_rate_hz,
        );
        block.timeline = self.timeline;
        block
    }

    pub fn decode(&mut self, input: &[u8]) -> Option<PcmStreamBlock> {
        if self.rewind_frame().is_some() {
            return Some(self.fault_block());
        }
        let mut block = self.empty_block();
        if self.decoder.is_finished() {
            block.mark_eof();
            return Some(block);
        }
        let bytes = input
            .len()
            .min(usize::try_from(self.bytes_to_boundary).unwrap_or(usize::MAX));
        if bytes == 0 {
            return Some(self.fault_block());
        }
        let progress = self.decoder.decode(&input[..bytes], block.frames_mut());
        if !block.set_len(progress.produced_frames) || progress.consumed_bytes != bytes {
            return Some(self.fault_block());
        }
        self.bytes_to_boundary -= bytes as u64;
        let Some(next) = self.next_frame.checked_add(progress.produced_frames as u64) else {
            return Some(self.fault_block());
        };
        self.next_frame = next;
        if let Some(region) = self.timeline.loop_region()
            && region.end() - region.start() <= PCM_STREAM_BLOCK_FRAMES as u64
        {
            for (offset, sample) in block.frames().iter().enumerate() {
                let frame = self.media_frame + offset as u64;
                if frame >= region.start() && frame < region.end() {
                    let index = (frame - region.start()) as usize;
                    self.short_loop[index] = *sample;
                    if !self.short_valid[index] {
                        self.short_valid[index] = true;
                        self.short_filled += 1;
                    }
                }
            }
        }
        self.media_frame += progress.produced_frames as u64;
        if progress.finished && self.timeline.loop_region().is_none() {
            block.mark_eof();
        }
        if block.len != 0 || block.eof {
            Some(block)
        } else {
            None
        }
    }
}
