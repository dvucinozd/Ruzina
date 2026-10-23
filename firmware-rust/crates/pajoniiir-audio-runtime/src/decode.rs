//! Bounded, pull-driven source contract shared by filesystems and host tests.
//! No filesystem, allocator, whole-track storage or output clock lives here.
//! Pair one active source with one exclusive codec context, checkpoint bank and
//! encoded window; the filesystem owner provisions and reuses these resources.
use crate::{
    mp3_index::{Mp3SeekIndex, Mp3TagKind},
    pcm_stream::{PCM_STREAM_BLOCK_FRAMES, PcmStreamBlock, frame_from_ms},
    timeline::{FrameLoop, PcmTimeline},
    wav::WavPcm16SourceDescriptor,
    wav_stream::WavStreamDecoder,
};
use nanoaudio_core::codec::{Decoder, Seek};
use nanoaudio_flac::FlacDecoder;
use nanoaudio_mp3::Mp3Decoder;
use pajoniiir_audio_dsp::PcmFrame;

pub const ENCODED_BUFFER_BYTES: usize = 98_304;
pub const SOURCE_READ_BYTES: usize = 512;
pub const CODEC_PCM_SAMPLES: usize = 2304;
pub const FLAC_MAX_BLOCK_FRAMES: usize = 9216;
pub const MP3_CHECKPOINTS: usize = 3; // loop origin + two recent exact seek points
const MP3_CHECKPOINT_INPUT: usize = 8192;
/// Prefer an indexed resume over a checkpoint further back than one index
/// stride plus a typical reservoir preroll.
const MP3_INDEX_SEEK_SLACK_FRAMES: u64 = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamChange {
    SeekMilliseconds(u32),
    Loop {
        position_frame: u64,
        region_ms: Option<(u32, u32)>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioCodec {
    WavPcm16,
    Mp3,
    Flac,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceDescriptor {
    Wav(WavPcm16SourceDescriptor),
    Compressed {
        codec: AudioCodec,
        sample_rate_hz: u32,
        channels: u8,
        frames: Option<u64>,
    },
}
impl SourceDescriptor {
    pub const fn sample_rate_hz(self) -> u32 {
        match self {
            Self::Wav(w) => w.format.sample_rate_hz,
            Self::Compressed { sample_rate_hz, .. } => sample_rate_hz,
        }
    }
    pub const fn frames(self) -> Option<u64> {
        match self {
            Self::Wav(w) => Some(w.frame_count()),
            Self::Compressed { frames, .. } => frames,
        }
    }
    pub const fn channels(self) -> usize {
        match self {
            Self::Wav(w) => w.format.channels as usize,
            Self::Compressed { channels, .. } => channels as usize,
        }
    }
}
impl From<WavPcm16SourceDescriptor> for SourceDescriptor {
    fn from(w: WavPcm16SourceDescriptor) -> Self {
        Self::Wav(w)
    }
}

/// Large decoder state must be placed by the caller (BSS or a provisioned heap),
/// never constructed on an embedded task stack. `new` is const for that purpose.
/// Where a source's length comes from (RP-08): codec metadata, a complete
/// MP3 frame walk, or nothing yet. Catalog length and analysis span are
/// never substituted for either.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceDuration {
    /// WAV size, FLAC STREAMINFO or an MP3 Xing/Info/VBRI frame count.
    Exact(u64),
    /// Untagged MP3 whose every frame header was walked.
    Scanned(u64),
    Unknown,
}

#[derive(Clone)]
pub struct DecoderStorage {
    mp3: Mp3Decoder<i16>,
    flac: FlacDecoder<i16, FLAC_MAX_BLOCK_FRAMES, 2>,
    pcm: [i16; CODEC_PCM_SAMPLES],
    mp3_index: Mp3SeekIndex,
}
#[derive(Clone)]
pub struct Mp3CheckpointStorage {
    decoder: Mp3Decoder<i16>,
    pcm: [i16; CODEC_PCM_SAMPLES],
    encoded: [u8; MP3_CHECKPOINT_INPUT],
}
impl Mp3CheckpointStorage {
    pub const fn new() -> Self {
        Self {
            decoder: Mp3Decoder::new(),
            pcm: [0; CODEC_PCM_SAMPLES],
            encoded: [0; MP3_CHECKPOINT_INPUT],
        }
    }
}
impl DecoderStorage {
    #[inline(always)]
    pub const fn new() -> Self {
        Self {
            mp3: Mp3Decoder::new(),
            flac: FlacDecoder::new(),
            pcm: [0; CODEC_PCM_SAMPLES],
            mp3_index: Mp3SeekIndex::new(),
        }
    }
    /// The frame walk of the current MP3 source.
    pub const fn mp3_index(&self) -> &Mp3SeekIndex {
        &self.mp3_index
    }
    fn reset(&mut self, codec: AudioCodec) {
        match codec {
            AudioCodec::Mp3 => self.mp3.reset(),
            AudioCodec::Flac => self.flac.reset(),
            AudioCodec::WavPcm16 => {}
        }
    }
    #[inline(never)]
    fn capture_mp3(&mut self, checkpoint: &mut Mp3CheckpointStorage, encoded: &[u8]) {
        checkpoint.decoder.clone_from(&self.mp3);
        checkpoint.pcm.copy_from_slice(&self.pcm);
        checkpoint.encoded[..encoded.len()].copy_from_slice(encoded);
    }
    #[inline(never)]
    fn restore_mp3(&mut self, checkpoint: &Mp3CheckpointStorage, encoded: &mut [u8]) {
        self.mp3.clone_from(&checkpoint.decoder);
        self.pcm.copy_from_slice(&checkpoint.pcm);
        encoded.copy_from_slice(&checkpoint.encoded[..encoded.len()]);
    }
}
impl Default for DecoderStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
#[allow(
    clippy::large_enum_variant,
    reason = "Fixed PCM blocks cross the owner boundary without heap allocation"
)]
pub enum DecodeAction {
    Read {
        offset: u64,
        maximum: usize,
    },
    Block(PcmStreamBlock),
    /// One bounded codec/preroll step completed; the owner may yield/cancel.
    Pending,
}

#[derive(Clone, Copy)]
struct Mp3Checkpoint {
    frame: u64,
    offset: u64,
    input_len: usize,
    pcm_position: usize,
    pcm_len: usize,
    tag_checked: bool,
    trailing_checked: bool,
}

pub struct SourceDecoder {
    codec: AudioCodec,
    descriptor: Option<SourceDescriptor>,
    wav: Option<WavStreamDecoder>,
    file_bytes: u64,
    offset: u64,
    input_len: usize,
    requested: usize,
    input_ready: bool,
    pcm_position: usize,
    pcm_len: usize,
    decoded_frame: u64,
    media_frame: u64,
    next_frame: u64,
    generation: u32,
    timeline: PcmTimeline,
    failed: bool,
    ended: bool,
    short_loop: [PcmFrame; PCM_STREAM_BLOCK_FRAMES],
    short_valid: [bool; PCM_STREAM_BLOCK_FRAMES],
    short_filled: usize,
    mp3_tag_checked: bool,
    trailing_checked: bool,
    checkpoints: [Option<Mp3Checkpoint>; MP3_CHECKPOINTS],
    /// File offset of the first byte fed to the MP3 decoder (after ID3v2).
    audio_start: u64,
    /// Output frames the MP3 decoder trims at the start (gapless lead),
    /// learned once from its own seek arithmetic.
    mp3_lead: Option<u64>,
}
impl SourceDecoder {
    pub fn compressed(
        codec: AudioCodec,
        file_bytes: u64,
        generation: u32,
        storage: &mut DecoderStorage,
    ) -> Option<Self> {
        if codec == AudioCodec::WavPcm16 || file_bytes == 0 || generation == 0 {
            return None;
        }
        storage.reset(codec);
        storage.mp3_index.reset();
        if codec == AudioCodec::Flac {
            storage.flac.set_stream_length(file_bytes);
        }
        Some(Self::base(codec, file_bytes, generation))
    }
    fn base(codec: AudioCodec, file_bytes: u64, generation: u32) -> Self {
        Self {
            codec,
            file_bytes,
            generation,
            descriptor: None,
            wav: None,
            offset: 0,
            input_len: 0,
            requested: 0,
            input_ready: false,
            pcm_position: 0,
            pcm_len: 0,
            decoded_frame: 0,
            media_frame: 0,
            next_frame: 0,
            timeline: PcmTimeline::linear(),
            failed: false,
            ended: false,
            short_loop: [PcmFrame { left: 0, right: 0 }; PCM_STREAM_BLOCK_FRAMES],
            short_valid: [false; PCM_STREAM_BLOCK_FRAMES],
            short_filled: 0,
            mp3_tag_checked: false,
            trailing_checked: false,
            checkpoints: [None; MP3_CHECKPOINTS],
            audio_start: 0,
            mp3_lead: None,
        }
    }
    pub fn wav(
        descriptor: WavPcm16SourceDescriptor,
        file_bytes: u64,
        generation: u32,
    ) -> Option<Self> {
        if descriptor.data_offset.checked_add(descriptor.data_bytes)? > file_bytes {
            return None;
        }
        let mut s = Self::base(AudioCodec::WavPcm16, file_bytes, generation);
        s.wav = Some(WavStreamDecoder::new(descriptor, generation, 0, None)?);
        s.descriptor = Some(descriptor.into());
        s.offset = descriptor.data_offset;
        Some(s)
    }
    pub const fn descriptor(&self) -> Option<SourceDescriptor> {
        self.descriptor
    }
    pub const fn generation(&self) -> u32 {
        self.generation
    }
    /// Next read of the background MP3 frame walk, once the stream format
    /// and audio start are known. It is independent of `step`'s request:
    /// owners interleave it only while their PCM queue is full.
    pub fn index_read(
        &mut self,
        storage: &mut DecoderStorage,
        maximum: usize,
    ) -> Option<(u64, usize)> {
        if self.codec != AudioCodec::Mp3 || self.descriptor.is_none() || !self.mp3_tag_checked {
            return None;
        }
        if !storage.mp3_index.started() {
            storage.mp3_index.begin(self.audio_start, self.file_bytes);
        }
        storage.mp3_index.next_read(maximum)
    }
    /// Bytes for the last `index_read`; false for an unexpected range.
    pub fn provide_index(
        &mut self,
        storage: &mut DecoderStorage,
        offset: u64,
        bytes: &[u8],
    ) -> bool {
        self.codec == AudioCodec::Mp3 && storage.mp3_index.provide(offset, bytes)
    }
    pub fn duration(&self, storage: &DecoderStorage) -> SourceDuration {
        match self.descriptor.and_then(SourceDescriptor::frames) {
            Some(frames) => SourceDuration::Exact(frames),
            None => match storage.mp3_index.scanned_pcm_frames() {
                Some(frames) if self.codec == AudioCodec::Mp3 => SourceDuration::Scanned(frames),
                _ => SourceDuration::Unknown,
            },
        }
    }
    pub fn media_frame(&self) -> u64 {
        self.wav
            .as_ref()
            .map_or(self.media_frame, WavStreamDecoder::media_frame)
    }
    pub fn fault_block(&self) -> PcmStreamBlock {
        if let Some(wav) = &self.wav {
            return wav.fault_block();
        }
        let mut block = self.empty();
        block.mark_faulted();
        block
    }
    fn empty(&self) -> PcmStreamBlock {
        let mut b = PcmStreamBlock::empty(
            self.generation,
            self.next_frame,
            self.descriptor.map_or(0, SourceDescriptor::sample_rate_hz),
        );
        b.timeline = self.timeline;
        b
    }
    /// Supply exactly the last requested file range, with short reads allowed.
    /// A zero read before the advertised file end is an I/O failure, not EOF.
    pub fn provide(&mut self, encoded: &mut [u8], bytes: &[u8]) -> bool {
        if self.requested == 0
            || bytes.len() > self.requested
            || self.input_len + bytes.len() > encoded.len()
            || (bytes.is_empty() && self.offset < self.file_bytes)
        {
            self.failed = true;
            return false;
        }
        encoded[self.input_len..self.input_len + bytes.len()].copy_from_slice(bytes);
        self.input_len += bytes.len();
        self.offset += bytes.len() as u64;
        self.requested = 0;
        self.input_ready = true;
        true
    }
    fn read(&mut self, capacity: usize) -> DecodeAction {
        let remaining = (self.file_bytes - self.offset).min(SOURCE_READ_BYTES as u64) as usize;
        self.requested = remaining.min(capacity.saturating_sub(self.input_len));
        if self.requested == 0 {
            self.failed = true;
            return DecodeAction::Block(self.fault_block());
        }
        DecodeAction::Read {
            offset: self.offset,
            maximum: self.requested,
        }
    }
    pub fn change(
        &mut self,
        storage: &mut DecoderStorage,
        checkpoints: &mut [&mut Mp3CheckpointStorage; MP3_CHECKPOINTS],
        encoded: &mut [u8],
        generation: u32,
        change: StreamChange,
    ) -> bool {
        if generation == 0 || encoded.len() < ENCODED_BUFFER_BYTES {
            return false;
        }
        if let Some(wav) = &self.wav {
            let Some(next) = wav.changed(generation, change) else {
                return false;
            };
            let Some(SourceDescriptor::Wav(desc)) = self.descriptor else {
                return false;
            };
            self.offset =
                desc.data_offset + next.media_frame() * u64::from(desc.format.block_align);
            self.wav = Some(next);
            self.input_len = 0;
        } else {
            let Some(desc) = self.descriptor else {
                return false;
            };
            let total = desc.frames().unwrap_or(u64::MAX);
            let (frame, region) = match change {
                StreamChange::SeekMilliseconds(ms) => (
                    frame_from_ms(ms, desc.sample_rate_hz(), total),
                    self.timeline.loop_region(),
                ),
                StreamChange::Loop {
                    position_frame,
                    region_ms,
                } => (
                    position_frame.min(total),
                    match region_ms {
                        Some((a, b)) => {
                            match FrameLoop::from_ms(a, b, desc.sample_rate_hz(), total) {
                                Some(r) => Some(r),
                                None => return false,
                            }
                        }
                        None => None,
                    },
                ),
            };
            self.timeline = PcmTimeline::at(frame, region);
            self.media_frame = self.timeline.origin();
            self.next_frame = self.media_frame;
            if !self.restart(storage, checkpoints, encoded) {
                return false;
            }
        }
        self.generation = generation;
        self.failed = false;
        self.ended = false;
        self.input_ready = false;
        self.requested = 0;
        self.short_filled = 0;
        self.short_valid.fill(false);
        true
    }
    fn restart(
        &mut self,
        storage: &mut DecoderStorage,
        checkpoints: &mut [&mut Mp3CheckpointStorage; MP3_CHECKPOINTS],
        encoded: &mut [u8],
    ) -> bool {
        // FLAC headers carry exact sample numbers and the decoder refines its
        // search with bounded SeekTo requests. MP3 restores a complete decoder,
        // reservoir, PCM and input checkpoint, never an approximate VBR offset;
        // a target far from every checkpoint resumes at an exact indexed frame
        // through the codec's reservoir preroll instead (RP-08).
        let checkpoint = self
            .checkpoints
            .iter()
            .enumerate()
            .filter_map(|(i, c)| c.map(|c| (i, c)))
            .filter(|(_, c)| c.frame <= self.media_frame)
            .max_by_key(|(_, c)| c.frame);
        if self.codec == AudioCodec::Mp3 {
            let index = &storage.mp3_index;
            let reach = (u64::from(index.stride()) + MP3_INDEX_SEEK_SLACK_FRAMES)
                * index.samples_per_frame().unwrap_or(1152);
            if checkpoint.is_none_or(|(_, c)| self.media_frame - c.frame > reach)
                && self.indexed_mp3_seek(storage)
            {
                return true;
            }
        }
        if self.codec == AudioCodec::Mp3
            && let Some((slot, mark)) = checkpoint
        {
            storage.restore_mp3(checkpoints[slot], &mut encoded[..mark.input_len]);
            self.offset = mark.offset;
            self.input_len = mark.input_len;
            self.pcm_position = mark.pcm_position;
            self.pcm_len = mark.pcm_len;
            self.decoded_frame = mark.frame;
            self.requested = 0;
            self.mp3_tag_checked = mark.tag_checked;
            self.trailing_checked = mark.trailing_checked;
            return true;
        }
        if self.codec == AudioCodec::Flac && self.descriptor.is_some() {
            let Ok(request) = storage.flac.seek(self.media_frame) else {
                return false;
            };
            if request.byte_offset > self.file_bytes {
                return false;
            }
            self.offset = request.byte_offset;
            self.decoded_frame = self.media_frame;
        } else {
            storage.reset(self.codec);
            self.offset = 0;
            self.decoded_frame = 0;
        }
        self.input_len = 0;
        self.requested = 0;
        self.pcm_len = 0;
        self.pcm_position = 0;
        self.mp3_tag_checked = false;
        self.trailing_checked = false;
        true
    }
    /// Resumes the MP3 decoder near `media_frame` from an exact frame offset.
    ///
    /// Constant streams (a complete walk saw one bitrate) use the codec's own
    /// byte arithmetic, which is exact there. Xing/VBRI streams would land
    /// where the table of contents points, about 1 % off, so the codec is
    /// asked to resume at an indexed frame instead and fed from that frame's
    /// walked offset; the codec still drops its own reservoir preroll, and
    /// `step` skips the few output frames up to the target. Any inconsistency
    /// returns false and the caller restores a checkpoint or restarts.
    fn indexed_mp3_seek(&mut self, storage: &mut DecoderStorage) -> bool {
        let target = self.media_frame;
        let index = &storage.mp3_index;
        let (Some(samples), Some(first)) = (index.samples_per_frame(), index.first_offset()) else {
            return false;
        };
        if target == 0
            || first < self.audio_start
            || self
                .descriptor
                .and_then(SourceDescriptor::frames)
                .is_some_and(|frames| target >= frames)
        {
            return false;
        }
        let first = first - self.audio_start;
        let (offset, resume) = match (index.kind(), index.variable_bitrate()) {
            (Mp3TagKind::Plain | Mp3TagKind::Constant, false) if index.complete() => {
                let Ok(request) = storage.mp3.seek(target) else {
                    return false;
                };
                (self.audio_start + request.byte_offset, target)
            }
            (Mp3TagKind::Variable, _) => {
                let lead = match self.mp3_lead {
                    Some(lead) => lead,
                    None => {
                        // Resuming at 0 starts at the first frame and skips
                        // exactly the trimmed lead, when the lead is shorter
                        // than the codec's preroll.
                        let Ok(zero) = storage.mp3.seek(0) else {
                            return false;
                        };
                        if zero.byte_offset != first {
                            return false;
                        }
                        self.mp3_lead = Some(u64::from(zero.skip_frames));
                        u64::from(zero.skip_frames)
                    }
                };
                let Ok(request) = storage.mp3.seek(target) else {
                    return false;
                };
                let position = target + lead;
                let skip = u64::from(request.skip_frames);
                let within = position % samples;
                if request.skip_frames == u32::MAX
                    || skip < within
                    || skip > position
                    || !(position - skip).is_multiple_of(samples)
                {
                    return false;
                }
                let preroll = skip - within;
                let start = (position - skip) / samples;
                if start == 0 && request.byte_offset == first {
                    (self.audio_start + first, target)
                } else {
                    let Some((frame, offset)) = index.at_or_before(start) else {
                        return false;
                    };
                    let Some(resume) = (frame * samples + preroll).checked_sub(lead) else {
                        return false;
                    };
                    match storage.mp3.seek(resume) {
                        Ok(again) if u64::from(again.skip_frames) == preroll => (offset, resume),
                        _ => return false,
                    }
                }
            }
            _ => return false,
        };
        self.offset = offset;
        self.decoded_frame = resume;
        self.input_len = 0;
        self.requested = 0;
        self.pcm_len = 0;
        self.pcm_position = 0;
        self.mp3_tag_checked = true;
        self.trailing_checked = false;
        true
    }
    fn capture(
        &mut self,
        slot: usize,
        storage: &mut DecoderStorage,
        checkpoints: &mut [&mut Mp3CheckpointStorage; MP3_CHECKPOINTS],
        encoded: &[u8],
    ) {
        if self.input_len > MP3_CHECKPOINT_INPUT {
            return;
        }
        storage.capture_mp3(checkpoints[slot], &encoded[..self.input_len]);
        self.checkpoints[slot] = Some(Mp3Checkpoint {
            frame: self.decoded_frame,
            offset: self.offset,
            input_len: self.input_len,
            pcm_position: self.pcm_position,
            pcm_len: self.pcm_len,
            tag_checked: self.mp3_tag_checked,
            trailing_checked: self.trailing_checked,
        });
    }
    fn cached(&mut self) -> Option<PcmStreamBlock> {
        let region = self.timeline.loop_region()?;
        if region.end() - region.start() > PCM_STREAM_BLOCK_FRAMES as u64
            || self.short_filled != (region.end() - region.start()) as usize
        {
            return None;
        }
        let mut b = self.empty();
        for (i, sample) in b.frames_mut().iter_mut().enumerate() {
            let frame = self
                .timeline
                .media_frame(self.next_frame.checked_add(i as u64)?)?;
            *sample = self.short_loop[(frame - region.start()) as usize];
        }
        self.next_frame = self
            .next_frame
            .checked_add(PCM_STREAM_BLOCK_FRAMES as u64)?;
        self.media_frame = self.timeline.media_frame(self.next_frame)?;
        b.set_len(PCM_STREAM_BLOCK_FRAMES);
        Some(b)
    }
    pub fn step(
        &mut self,
        storage: &mut DecoderStorage,
        checkpoints: &mut [&mut Mp3CheckpointStorage; MP3_CHECKPOINTS],
        encoded: &mut [u8],
    ) -> DecodeAction {
        if self.failed || encoded.len() < ENCODED_BUFFER_BYTES {
            self.failed = true;
            return DecodeAction::Block(self.fault_block());
        }
        if let Some(wav) = &mut self.wav {
            if let Some(b) = wav.cached_block() {
                return DecodeAction::Block(b);
            }
            if self.input_ready {
                self.input_ready = false;
                let b = wav.decode(&encoded[..self.input_len]);
                self.input_len = 0;
                return b.map_or(DecodeAction::Pending, DecodeAction::Block);
            }
            if let Some(frame) = wav.rewind_frame() {
                let Some(SourceDescriptor::Wav(desc)) = self.descriptor else {
                    unreachable!()
                };
                self.offset = desc.data_offset + frame * u64::from(desc.format.block_align);
                if !wav.complete_rewind() {
                    self.failed = true;
                    return DecodeAction::Block(self.fault_block());
                }
            }
            if self.offset == self.file_bytes {
                return wav
                    .decode(&[])
                    .map_or(DecodeAction::Pending, DecodeAction::Block);
            }
            return self.read(encoded.len());
        }
        if let Some(b) = self.cached() {
            return DecodeAction::Block(b);
        }
        if self
            .timeline
            .loop_region()
            .is_some_and(|r| self.media_frame == r.end())
        {
            self.media_frame = self.timeline.loop_region().unwrap().start();
            if !self.restart(storage, checkpoints, encoded) {
                self.failed = true;
                return DecodeAction::Block(self.fault_block());
            }
        }
        if self.pcm_position < self.pcm_len {
            if self.codec == AudioCodec::Mp3 {
                let newest = self.checkpoints[1..]
                    .iter()
                    .flatten()
                    .map(|c| c.frame)
                    .max();
                let stride = u64::from(self.descriptor.unwrap().sample_rate_hz()) / 2;
                if newest.is_none_or(|frame| self.decoded_frame >= frame.saturating_add(stride)) {
                    let slot = if self.checkpoints[1].is_none_or(|c| {
                        self.checkpoints[2].is_some_and(|other| c.frame <= other.frame)
                    }) {
                        1
                    } else {
                        2
                    };
                    self.capture(slot, storage, checkpoints, encoded);
                }
            }
            let channels = self.descriptor.unwrap().channels();
            let skip = (self.media_frame.saturating_sub(self.decoded_frame))
                .min(((self.pcm_len - self.pcm_position) / channels) as u64)
                as usize;
            self.pcm_position += skip * channels;
            self.decoded_frame += skip as u64;
            if self.pcm_position == self.pcm_len {
                return DecodeAction::Pending;
            }
            if self.codec == AudioCodec::Mp3
                && self
                    .timeline
                    .loop_region()
                    .is_some_and(|r| r.start() == self.decoded_frame)
                && self.checkpoints[0].is_none_or(|c| c.frame != self.decoded_frame)
            {
                self.capture(0, storage, checkpoints, encoded);
            }
            let end = self.timeline.loop_region().map_or(u64::MAX, FrameLoop::end);
            let count = ((self.pcm_len - self.pcm_position) / channels)
                .min(PCM_STREAM_BLOCK_FRAMES)
                .min((end - self.media_frame).min(usize::MAX as u64) as usize);
            let mut b = self.empty();
            for (i, out) in b.frames_mut()[..count].iter_mut().enumerate() {
                let p = self.pcm_position + i * channels;
                *out = PcmFrame {
                    left: storage.pcm[p],
                    right: storage.pcm[p + channels - 1],
                };
                if let Some(region) = self.timeline.loop_region() {
                    let frame = self.media_frame + i as u64;
                    if region.end() - region.start() <= PCM_STREAM_BLOCK_FRAMES as u64
                        && frame >= region.start()
                    {
                        let index = (frame - region.start()) as usize;
                        self.short_loop[index] = *out;
                        if !self.short_valid[index] {
                            self.short_valid[index] = true;
                            self.short_filled += 1;
                        }
                    }
                }
            }
            self.pcm_position += count * channels;
            self.decoded_frame += count as u64;
            self.media_frame += count as u64;
            let Some(next) = self.next_frame.checked_add(count as u64) else {
                self.failed = true;
                return DecodeAction::Block(self.fault_block());
            };
            self.next_frame = next;
            b.set_len(count);
            return DecodeAction::Block(b);
        }
        if self.ended {
            let mut b = self.empty();
            b.mark_eof();
            return DecodeAction::Block(b);
        }
        // Metadata is not audio, even if embedded cover art happens to contain
        // a valid MP3 stream. Skip ID3 bodies by their checked byte length.
        if self.codec == AudioCodec::Mp3 && !self.mp3_tag_checked {
            if self.input_len < 10 && self.offset < self.file_bytes {
                return self.read(encoded.len());
            }
            if self.input_len >= 10 && &encoded[..3] == b"ID3" {
                if !(2..=4).contains(&encoded[3]) || encoded[6..10].iter().any(|v| v & 0x80 != 0) {
                    self.failed = true;
                    return DecodeAction::Block(self.fault_block());
                }
                let body = encoded[6..10]
                    .iter()
                    .fold(0u64, |size, byte| (size << 7) | u64::from(*byte));
                let footer = u64::from(encoded[3] == 4 && encoded[5] & 0x10 != 0) * 10;
                let total = 10 + body + footer;
                let start = self.offset - self.input_len as u64;
                if total > self.file_bytes - start {
                    self.failed = true;
                    return DecodeAction::Block(self.fault_block());
                }
                if total < self.input_len as u64 {
                    encoded.copy_within(total as usize..self.input_len, 0);
                    self.input_len -= total as usize;
                } else {
                    self.input_len = 0;
                    self.offset = start + total;
                }
                return DecodeAction::Pending;
            }
            self.audio_start = self.offset - self.input_len as u64;
            self.mp3_tag_checked = true;
        }
        if self.codec == AudioCodec::Mp3 && self.offset == self.file_bytes && !self.trailing_checked
        {
            if self.input_len >= 128
                && &encoded[self.input_len - 128..self.input_len - 125] == b"TAG"
            {
                self.input_len -= 128;
            }
            self.trailing_checked = true;
        }
        if self.offset == self.file_bytes {
            match self.codec {
                AudioCodec::Mp3 => storage.mp3.finish(),
                AudioCodec::Flac => storage.flac.finish(),
                _ => unreachable!(),
            }
        }
        let result = match self.codec {
            AudioCodec::Mp3 => storage
                .mp3
                .decode(&encoded[..self.input_len], &mut storage.pcm),
            AudioCodec::Flac => storage
                .flac
                .decode(&encoded[..self.input_len], &mut storage.pcm),
            _ => unreachable!(),
        };
        let progress = match result {
            Ok(progress) => progress,
            Err(nanoaudio_core::Error::SeekTo(offset))
                if self.codec == AudioCodec::Flac && offset <= self.file_bytes =>
            {
                self.offset = offset;
                self.input_len = 0;
                return DecodeAction::Pending;
            }
            Err(_) => {
                self.failed = true;
                return DecodeAction::Block(self.fault_block());
            }
        };
        let configuration = match self.codec {
            AudioCodec::Mp3 => storage.mp3.configuration(),
            AudioCodec::Flac => storage.flac.configuration(),
            _ => unreachable!(),
        };
        if let Some(c) = configuration {
            if c.channels == 0 || c.channels > 2 || c.sample_rate == 0 || c.sample_rate > 192_000 {
                self.failed = true;
                return DecodeAction::Block(self.fault_block());
            }
            if self.descriptor.is_some_and(|d| {
                d.channels() != c.channels as usize || d.sample_rate_hz() != c.sample_rate
            }) {
                self.failed = true;
                return DecodeAction::Block(self.fault_block());
            }
            let frames = match self.codec {
                AudioCodec::Mp3 => storage.mp3.frames(),
                AudioCodec::Flac => storage
                    .flac
                    .stream_information()
                    .and_then(|i| (i.total_samples != 0).then_some(i.total_samples)),
                _ => unreachable!(),
            };
            self.descriptor = Some(SourceDescriptor::Compressed {
                codec: self.codec,
                sample_rate_hz: c.sample_rate,
                channels: c.channels,
                frames,
            });
        }
        if progress.consumed > self.input_len
            || progress.produced > CODEC_PCM_SAMPLES
            || (progress.produced > 0
                && self
                    .descriptor
                    .is_none_or(|d| progress.produced % d.channels() != 0))
        {
            self.failed = true;
            return DecodeAction::Block(self.fault_block());
        }
        encoded.copy_within(progress.consumed..self.input_len, 0);
        self.input_len -= progress.consumed;
        self.pcm_position = 0;
        self.pcm_len = progress.produced;
        if progress.is_stalled() {
            if self.offset == self.file_bytes {
                if self.descriptor.is_none()
                    || self.decoded_frame < self.media_frame
                    || self
                        .descriptor
                        .and_then(SourceDescriptor::frames)
                        .is_some_and(|n| self.decoded_frame != n)
                    || self.timeline.loop_region().is_some()
                {
                    self.failed = true;
                    return DecodeAction::Block(self.fault_block());
                }
                self.ended = true;
                let mut b = self.empty();
                b.mark_eof();
                return DecodeAction::Block(b);
            }
            return self.read(encoded.len());
        }
        DecodeAction::Pending
    }
}

impl Default for Mp3CheckpointStorage {
    fn default() -> Self {
        Self::new()
    }
}
