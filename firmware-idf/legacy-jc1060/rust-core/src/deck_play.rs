//! Deck 1 playback from the card to the controller's headphones.
//!
//! The SD load task is the producer: on a Load it opens the track's audio
//! file on the exFAT volume, decodes it with the shared `ruzina-audio-runtime`
//! `SourceDecoder` (MP3, FLAC or 16-bit WAV) and pushes PCM at the source rate
//! into a PSRAM ring. The USB client task is the consumer: `controller_audio`
//! pulls each packet's frames through the product's `ResamplerState` to the
//! device rate while deck 1's Play latch is on, scaled by the Headphone Level
//! knob. Cue returns the deck to the start of the track, paused.
//!
//! The ring is single-producer, single-consumer and lock-free. A new source
//! (Load or Cue) bumps the ring's epoch and records where its frames start;
//! the consumer jumps its read position there when it sees the new epoch, so
//! the producer never waits for the consumer to discard the old track.
use alloc::{boxed::Box, vec};
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};
use ruzina_audio_dsp::{PcmFrame, ResamplerState};
use ruzina_audio_runtime::decode::{
    AudioCodec, DecodeAction, DecoderStorage, ENCODED_BUFFER_BYTES, MP3_CHECKPOINTS,
    Mp3CheckpointStorage, SOURCE_READ_BYTES, SourceDecoder, StreamChange,
};
use ruzina_audio_runtime::pcm_stream::{PCM_STREAM_BLOCK_FRAMES, PcmStreamBlock};
use ruzina_audio_runtime::wav::{
    CHUNK_HEADER_BYTES, PCM_FORMAT_BYTES, RIFF_HEADER_BYTES, WavPcm16SourceDescriptor,
    parse_chunk_header, parse_pcm16_format, parse_riff_header,
};

/// Ring capacity in source frames: about 1.5 s at 44.1 kHz, 256 KiB of PSRAM.
pub(crate) const RING_FRAMES: u32 = 1 << 16;

/// Decoder actions one prime may take before its first block (ID3 tags
/// with artwork are read through 512 bytes at a time).
const PRIME_ACTIONS: u32 = 8192;

/// Decoder actions per producer poll, so a selection is served promptly.
const POLL_ACTIONS: u32 = 64;

/// Headphone gain at full Headphone Level, Q15 (-12 dBFS ceiling).
/// The single-deck consumer below (`Output`, `render`, the gain) is unused
/// with `deck-mix`, whose render graph replaces it.
#[cfg_attr(feature = "deck-mix", allow(dead_code))]
const MAX_GAIN_Q15: u64 = 8192;

/// Lock-free PCM ring between the SD load task and the USB client task.
pub(crate) struct Ring {
    frames: AtomicPtr<PcmFrame>,
    /// Next slot the producer writes; only the producer stores it.
    head: AtomicU32,
    /// Next slot the consumer reads; only the consumer stores it.
    tail: AtomicU32,
    epoch: AtomicU32,
    /// Where the current epoch's frames begin.
    start: AtomicU32,
    /// The current epoch's source rate; 0 holds the consumer silent.
    rate: AtomicU32,
    /// The source frame the current epoch starts at (a seek's target).
    origin: AtomicU32,
    /// The epoch whose source has delivered its last frame.
    ended: AtomicU32,
}

impl Ring {
    pub(crate) const fn new() -> Self {
        Self {
            frames: AtomicPtr::new(core::ptr::null_mut()),
            head: AtomicU32::new(0),
            tail: AtomicU32::new(0),
            epoch: AtomicU32::new(0),
            start: AtomicU32::new(0),
            rate: AtomicU32::new(0),
            origin: AtomicU32::new(0),
            ended: AtomicU32::new(0),
        }
    }

    /// The current epoch, as the producer last opened it.
    #[cfg_attr(not(feature = "deck-engine"), allow(dead_code))]
    pub(crate) fn epoch(&self) -> u32 {
        self.epoch.load(Ordering::Acquire)
    }

    /// Hands the ring its storage once; later calls keep the first.
    pub(crate) fn attach(&self, storage: &'static mut [PcmFrame]) -> bool {
        storage.len() == RING_FRAMES as usize
            && self
                .frames
                .compare_exchange(
                    core::ptr::null_mut(),
                    storage.as_mut_ptr(),
                    Ordering::Release,
                    Ordering::Relaxed,
                )
                .is_ok()
    }

    fn slot(&self, position: u32) -> Option<*mut PcmFrame> {
        let base = self.frames.load(Ordering::Acquire);
        // SAFETY: `position & mask` is below the attached length.
        (!base.is_null()).then(|| unsafe { base.add((position % RING_FRAMES) as usize) })
    }

    /// Producer: free slots.
    pub(crate) fn free(&self) -> u32 {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        RING_FRAMES - head.wrapping_sub(tail)
    }

    /// Producer: appends all of `frames`, or nothing when they do not fit.
    pub(crate) fn push(&self, frames: &[PcmFrame]) -> bool {
        if frames.len() > self.free() as usize {
            return false;
        }
        let head = self.head.load(Ordering::Relaxed);
        for (offset, frame) in frames.iter().enumerate() {
            let Some(slot) = self.slot(head.wrapping_add(offset as u32)) else {
                return false;
            };
            // SAFETY: slots from head up to the free count belong to the
            // producer until `head` is published below.
            unsafe { slot.write(*frame) };
        }
        self.head
            .store(head.wrapping_add(frames.len() as u32), Ordering::Release);
        true
    }

    /// Producer: starts a new source at `rate` (0 silences the consumer);
    /// frames pushed before this call are never played.
    pub(crate) fn restart(&self, rate: u32) {
        self.restart_at(rate, 0);
    }

    /// As `restart`, for a source whose first frame is `origin`.
    pub(crate) fn restart_at(&self, rate: u32, origin: u32) {
        self.start
            .store(self.head.load(Ordering::Relaxed), Ordering::Relaxed);
        self.rate.store(rate, Ordering::Relaxed);
        self.origin.store(origin, Ordering::Relaxed);
        self.epoch.fetch_add(1, Ordering::Release);
    }

    /// Producer: the current source has pushed its last frame.
    pub(crate) fn end(&self) {
        self.ended
            .store(self.epoch.load(Ordering::Relaxed), Ordering::Release);
    }
}

/// The consumer's view of the ring.
#[derive(Clone, Copy)]
pub(crate) struct Reader {
    epoch: u32,
    tail: u32,
    rate: u32,
    /// The epoch's first source frame plus the frames taken since.
    position: u32,
}

impl Reader {
    pub(crate) const fn new() -> Self {
        Self {
            epoch: 0,
            tail: 0,
            rate: 0,
            position: 0,
        }
    }

    /// Follows a new epoch; returns true when the source changed.
    pub(crate) fn sync(&mut self, ring: &Ring) -> bool {
        let epoch = ring.epoch.load(Ordering::Acquire);
        if epoch == self.epoch {
            return false;
        }
        self.epoch = epoch;
        self.tail = ring.start.load(Ordering::Relaxed);
        self.rate = ring.rate.load(Ordering::Relaxed);
        self.position = ring.origin.load(Ordering::Relaxed);
        ring.tail.store(self.tail, Ordering::Release);
        true
    }

    pub(crate) fn pop(&mut self, ring: &Ring) -> Option<PcmFrame> {
        if self.tail == ring.head.load(Ordering::Acquire) {
            return None;
        }
        // SAFETY: slots between tail and the published head hold frames
        // the producer wrote before publishing; it never touches them again
        // until `tail` passes them.
        let frame = unsafe { ring.slot(self.tail)?.read() };
        self.tail = self.tail.wrapping_add(1);
        self.position = self.position.wrapping_add(1);
        Some(frame)
    }

    /// Returns the consumed slots to the producer.
    pub(crate) fn commit(&self, ring: &Ring) {
        ring.tail.store(self.tail, Ordering::Release);
    }

    pub(crate) fn ended(&self, ring: &Ring) -> bool {
        ring.ended.load(Ordering::Acquire) == self.epoch
    }

    /// The current epoch's source rate; 0 while no source is ready.
    #[cfg_attr(not(feature = "deck-mix"), allow(dead_code))]
    pub(crate) const fn rate(&self) -> u32 {
        self.rate
    }

    /// The epoch this reader follows.
    #[cfg_attr(not(feature = "deck-engine"), allow(dead_code))]
    pub(crate) const fn epoch(&self) -> u32 {
        self.epoch
    }

    /// Source position of the next frame, in milliseconds; 0 without a rate.
    #[cfg_attr(not(feature = "deck-engine"), allow(dead_code))]
    pub(crate) fn position_ms(&self) -> u32 {
        if self.rate == 0 {
            return 0;
        }
        (u64::from(self.position) * 1000 / u64::from(self.rate)) as u32
    }

    pub(crate) fn buffered(&self, ring: &Ring) -> u32 {
        ring.head.load(Ordering::Acquire).wrapping_sub(self.tail)
    }
}

/// The audio file of one track, as the producer reads it. The legacy bridge
/// implements it over the exFAT volume; host tests over memory.
pub(crate) trait SourceFiles {
    type File;
    fn open(&mut self, path: &str) -> Option<Self::File>;
    fn len(&self, file: &Self::File) -> u64;
    /// Reads up to `out.len()` bytes at `offset`; `None` is an I/O failure.
    fn read_at(&mut self, file: &mut Self::File, offset: u64, out: &mut [u8]) -> Option<usize>;
    fn close(&mut self, file: Self::File);
}

/// What the producer reports; scalars only reach C.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Event {
    /// The track is decoding; codec 1 WAV, 2 MP3, 3 FLAC.
    Ready { index: u32, codec: u32, rate: u32 },
    /// Stage 1 path, 2 open, 3 format, 4 decode.
    Failed { index: u32, stage: u32 },
    /// The track pushed its last frame (faulted: a decode or read failure).
    Ended { index: u32, faulted: bool },
    /// A seek reopened the track at `position_ms` (ok: its first block
    /// decoded; otherwise the track ended as faulted).
    Seeked {
        index: u32,
        position_ms: u32,
        ok: bool,
    },
}

fn codec_number(codec: AudioCodec) -> u32 {
    match codec {
        AudioCodec::WavPcm16 => 1,
        AudioCodec::Mp3 => 2,
        AudioCodec::Flac => 3,
    }
}

struct Source<File> {
    file: File,
    index: u32,
    stream: Box<SourceDecoder>,
    /// A decoded block still waiting for ring space.
    pending: Option<Box<PcmStreamBlock>>,
    /// The decoder delivered its last block.
    ended: bool,
}

/// Pushes one block, ending the epoch after a final block.
fn deliver(ring: &Ring, block: &PcmStreamBlock) -> bool {
    if !ring.push(block.frames()) {
        return false;
    }
    if block.eof {
        ring.end();
    }
    true
}

/// Decoder state in the PSRAM heap; built once by the load task.
pub(crate) struct Producer<File> {
    storage: Box<DecoderStorage>,
    checkpoints: [Box<Mp3CheckpointStorage>; MP3_CHECKPOINTS],
    encoded: Box<[u8]>,
    read: [u8; SOURCE_READ_BYTES],
    source: Option<Source<File>>,
    generation: u32,
}

/// Initial decoder state, built at compile time. Building it as a value
/// puts a temporary of over 100 KiB on the 48 KiB task stack (the build's
/// frame guard caught 175 KiB), so each heap slot is filled by copying this.
static INITIAL_STORAGE: DecoderStorage = DecoderStorage::new();
static INITIAL_CHECKPOINT: Mp3CheckpointStorage = Mp3CheckpointStorage::new();

/// A heap copy of `initial`, never a stack temporary.
fn boxed_copy<T: Clone>(initial: &'static T) -> Box<T> {
    let mut slot = Box::<T>::new_uninit();
    // SAFETY: the decoder states are plain arrays and counters that own no
    // heap or pointers, so a bitwise copy equals their derived `Clone`; the
    // slot is fully written before it is assumed initialized.
    unsafe {
        core::ptr::copy_nonoverlapping(initial, slot.as_mut_ptr(), 1);
        slot.assume_init()
    }
}

impl<File> Producer<File> {
    pub(crate) fn new() -> Self {
        Self {
            storage: boxed_copy(&INITIAL_STORAGE),
            checkpoints: core::array::from_fn(|_| boxed_copy(&INITIAL_CHECKPOINT)),
            encoded: vec![0u8; ENCODED_BUFFER_BYTES].into_boxed_slice(),
            read: [0; SOURCE_READ_BYTES],
            source: None,
            generation: 0,
        }
    }

    fn close<F: SourceFiles<File = File>>(&mut self, files: &mut F) {
        if let Some(source) = self.source.take() {
            files.close(source.file);
        }
    }

    /// Opens track `index` and decodes its first block into a new ring
    /// epoch. The ring is silenced first, so the old track stops at once.
    pub(crate) fn load<F: SourceFiles<File = File>>(
        &mut self,
        files: &mut F,
        ring: &Ring,
        index: u32,
        path: Option<&str>,
    ) -> Event {
        self.close(files);
        ring.restart(0);
        let failed = |stage| Event::Failed { index, stage };
        let Some(path) = path.filter(|path| !path.is_empty()) else {
            return failed(1);
        };
        let Some(mut file) = files.open(path) else {
            return failed(2);
        };
        let file_len = files.len(&file);
        let Some(stream) = self.open_stream(files, &mut file, file_len) else {
            files.close(file);
            return failed(3);
        };
        let codec = codec_number(stream.1);
        self.source = Some(Source {
            file,
            index,
            stream: Box::new(stream.0),
            pending: None,
            ended: false,
        });
        match self.decode(files, ring, PRIME_ACTIONS, true) {
            Some(rate) => Event::Ready { index, codec, rate },
            None => {
                self.close(files);
                ring.restart(0);
                failed(4)
            }
        }
    }

    fn open_stream<F: SourceFiles<File = File>>(
        &mut self,
        files: &mut F,
        file: &mut File,
        file_len: u64,
    ) -> Option<(SourceDecoder, AudioCodec)> {
        let head = files.read_at(file, 0, &mut self.read)?;
        if head < RIFF_HEADER_BYTES {
            return None;
        }
        self.generation = self.generation.wrapping_add(1).max(1);
        let magic = &self.read[..4];
        if magic == b"RIFF" {
            let descriptor = self.wav_descriptor(files, file, file_len)?;
            return SourceDecoder::wav(descriptor, file_len, self.generation)
                .map(|stream| (stream, AudioCodec::WavPcm16));
        }
        let codec = if magic == b"fLaC" {
            AudioCodec::Flac
        } else if &magic[..3] == b"ID3" || magic[0] == 0xff && magic[1] & 0xe0 == 0xe0 {
            AudioCodec::Mp3
        } else {
            return None;
        };
        SourceDecoder::compressed(codec, file_len, self.generation, &mut self.storage)
            .map(|stream| (stream, codec))
    }

    /// Walks the RIFF chunks to the PCM format and data chunk.
    fn wav_descriptor<F: SourceFiles<File = File>>(
        &mut self,
        files: &mut F,
        file: &mut File,
        file_len: u64,
    ) -> Option<WavPcm16SourceDescriptor> {
        let riff_end = u64::from(parse_riff_header(&self.read[..RIFF_HEADER_BYTES]).ok()?) + 8;
        if riff_end > file_len {
            return None;
        }
        let mut position = RIFF_HEADER_BYTES as u64;
        let mut format = None;
        while position + CHUNK_HEADER_BYTES as u64 <= riff_end {
            let bytes = files.read_at(file, position, &mut self.read)?;
            let chunk = parse_chunk_header(self.read.get(..CHUNK_HEADER_BYTES.min(bytes))?).ok()?;
            let data = position + CHUNK_HEADER_BYTES as u64;
            if data + u64::from(chunk.len) > riff_end {
                return None;
            }
            if chunk.is_format() {
                let bytes = files.read_at(file, data, &mut self.read)?;
                format =
                    Some(parse_pcm16_format(self.read.get(..PCM_FORMAT_BYTES.min(bytes))?).ok()?);
            } else if chunk.is_data() {
                return WavPcm16SourceDescriptor::new(format?, data, u64::from(chunk.len)).ok();
            }
            position = data + chunk.padded_len();
        }
        None
    }

    /// Runs the decoder for up to `actions` steps, pushing every block into
    /// the ring while a whole block fits. With `prime`, it stops at the first
    /// block, opening the ring epoch at that block's rate, which it returns;
    /// that block waits in `pending` while the consumer still holds the old
    /// epoch's frames. `None` is a read or decode failure.
    fn decode<F: SourceFiles<File = File>>(
        &mut self,
        files: &mut F,
        ring: &Ring,
        actions: u32,
        prime: bool,
    ) -> Option<u32> {
        let source = self.source.as_mut()?;
        if let Some(block) = source.pending.take()
            && !deliver(ring, &block)
        {
            source.pending = Some(block);
            return Some(0);
        }
        for _ in 0..actions {
            if source.ended || !prime && ring.free() < PCM_STREAM_BLOCK_FRAMES as u32 {
                break;
            }
            let mut checkpoints = self.checkpoints.each_mut().map(|slot| &mut **slot);
            match source
                .stream
                .step(&mut self.storage, &mut checkpoints, &mut self.encoded)
            {
                DecodeAction::Read { offset, maximum } => {
                    let read = &mut self.read[..maximum.min(SOURCE_READ_BYTES)];
                    let provided =
                        files
                            .read_at(&mut source.file, offset, read)
                            .is_some_and(|bytes| {
                                source.stream.provide(&mut self.encoded, &read[..bytes])
                            });
                    if !provided {
                        source.ended = true;
                        ring.end();
                        return None;
                    }
                }
                DecodeAction::Pending => {}
                DecodeAction::Block(block) => {
                    if block.faulted || prime && block.len == 0 {
                        source.ended = true;
                        ring.end();
                        return None;
                    }
                    let rate = block.sample_rate_hz;
                    source.ended = block.eof;
                    if prime {
                        ring.restart_at(rate, block.first_frame as u32);
                    }
                    if !deliver(ring, &block) {
                        source.pending = Some(Box::new(block));
                    }
                    if prime {
                        return Some(rate);
                    }
                }
            }
        }
        (!prime).then_some(0)
    }

    /// Moves the loaded track to `position_ms` and decodes its first block
    /// there into a new ring epoch; the old epoch plays on until then.
    pub(crate) fn seek<F: SourceFiles<File = File>>(
        &mut self,
        files: &mut F,
        ring: &Ring,
        position_ms: u32,
    ) -> Option<Event> {
        let source = self.source.as_mut()?;
        let index = source.index;
        self.generation = self.generation.wrapping_add(1).max(1);
        let mut checkpoints = self.checkpoints.each_mut().map(|slot| &mut **slot);
        let changed = source.stream.change(
            &mut self.storage,
            &mut checkpoints,
            &mut self.encoded,
            self.generation,
            StreamChange::SeekMilliseconds(position_ms),
        );
        source.pending = None;
        source.ended = false;
        let ok = changed && self.decode(files, ring, PRIME_ACTIONS, true).is_some();
        if !ok && let Some(source) = self.source.as_mut() {
            source.ended = true;
            ring.end();
        }
        Some(Event::Seeked {
            index,
            position_ms,
            ok,
        })
    }

    /// One producer turn: serves a pending Load, Cue or seek, else tops up
    /// the ring. Returns whether it did work and what to report.
    pub(crate) fn poll<F: SourceFiles<File = File>, P: AsRef<str>>(
        &mut self,
        files: &mut F,
        ring: &Ring,
        commands: &Commands,
        path: impl Fn(u32) -> Option<P>,
    ) -> (bool, Option<Event>) {
        let (load, cue) = commands.take();
        let index = match (load, &self.source) {
            (0, Some(source)) if cue => Some(source.index),
            (0, _) => None,
            (load, _) => Some(load - 1),
        };
        if let Some(index) = index {
            commands.take_seek();
            let path = path(index);
            return (
                true,
                Some(self.load(files, ring, index, path.as_ref().map(AsRef::as_ref))),
            );
        }
        if let Some(position_ms) = commands.take_seek() {
            let event = self.seek(files, ring, position_ms);
            return (event.is_some(), event);
        }
        let Some(source) = &self.source else {
            return (false, None);
        };
        if source.ended && source.pending.is_none() || ring.free() < PCM_STREAM_BLOCK_FRAMES as u32
        {
            return (false, None);
        }
        let (index, was_ended) = (source.index, source.ended);
        let failed = self.decode(files, ring, POLL_ACTIONS, false).is_none();
        let ended = !was_ended && self.source.as_ref().is_some_and(|source| source.ended);
        (
            true,
            ended.then_some(Event::Ended {
                index,
                faulted: failed,
            }),
        )
    }
}

/// Requests from the display and controller tasks to the producer.
pub(crate) struct Commands {
    /// Track index + 1 to load on deck 1; 0 none.
    load: AtomicU32,
    /// Return deck 1 to the start of its track.
    cue: AtomicBool,
    /// Seek target in milliseconds + 1; 0 none. A newer seek replaces it.
    seek: AtomicU32,
}

impl Commands {
    pub(crate) const fn new() -> Self {
        Self {
            load: AtomicU32::new(0),
            cue: AtomicBool::new(false),
            seek: AtomicU32::new(0),
        }
    }

    /// Move the loaded track to `position_ms` (from the deck engine).
    #[cfg_attr(not(feature = "deck-engine"), allow(dead_code))]
    pub(crate) fn seek(&self, position_ms: u32) {
        self.seek
            .store(position_ms.saturating_add(1), Ordering::Release);
    }

    pub(crate) fn take_seek(&self) -> Option<u32> {
        self.seek.swap(0, Ordering::Acquire).checked_sub(1)
    }

    pub(crate) fn load(&self, index: u32) {
        self.load.store(index.saturating_add(1), Ordering::Release);
    }

    /// Restart deck 1 from its beginning (without the deck engine, which
    /// seeks to its cue point instead).
    #[cfg_attr(feature = "deck-engine", allow(dead_code))]
    pub(crate) fn cue(&self) {
        self.cue.store(true, Ordering::Release);
    }

    /// A Load is waiting.
    pub(crate) fn pending_load(&self) -> bool {
        self.load.load(Ordering::Acquire) != 0
    }

    /// Takes the pending Load (index + 1, 0 none) and Cue.
    pub(crate) fn take(&self) -> (u32, bool) {
        (
            self.load.swap(0, Ordering::Acquire),
            self.cue.swap(false, Ordering::Acquire),
        )
    }
}

/// The consumer side: resampler, read position and counters.
#[cfg_attr(feature = "deck-mix", allow(dead_code))]
pub(crate) struct Output {
    reader: Reader,
    resampler: ResamplerState,
    played: u32,
    underruns: u32,
}

#[cfg_attr(feature = "deck-mix", allow(dead_code))]
impl Output {
    pub(crate) const fn new() -> Self {
        Self {
            reader: Reader::new(),
            resampler: ResamplerState::new(),
            played: 0,
            underruns: 0,
        }
    }

    /// Fills `out` with deck 1 at `device_rate`, silent while paused, before
    /// a source is ready or after it ended. A packet that runs dry while
    /// playing counts one underrun.
    pub(crate) fn render(
        &mut self,
        ring: &Ring,
        out: &mut [PcmFrame],
        device_rate: u32,
        playing: bool,
        gain_q15: u32,
    ) {
        if self.reader.sync(ring) {
            self.resampler.reset();
        }
        if !playing || self.reader.rate == 0 || device_rate == 0 {
            out.fill(PcmFrame::default());
            return;
        }
        let factor = self.reader.rate as f32 / device_rate as f32;
        let (reader, mut starved) = (&mut self.reader, false);
        for frame in out.iter_mut() {
            let (pcm, _) = self.resampler.next(factor, || {
                let next = reader.pop(ring);
                starved |= next.is_none();
                next
            });
            *frame = PcmFrame {
                left: scale(pcm.left, gain_q15),
                right: scale(pcm.right, gain_q15),
            };
        }
        reader.commit(ring);
        if starved {
            self.resampler.silence_history();
            if !reader.ended(ring) {
                self.underruns = self.underruns.wrapping_add(1);
            }
        }
        self.played = self.played.wrapping_add(out.len() as u32);
    }
}

#[cfg_attr(feature = "deck-mix", allow(dead_code))]
fn scale(sample: i16, gain_q15: u32) -> i16 {
    ((i64::from(sample) * i64::from(gain_q15)) >> 15) as i16
}

/// Headphone gain from the Headphone Level knob, squared for an audio taper;
/// a quarter turn until the knob first moves (-36 dBFS).
#[cfg_attr(feature = "deck-mix", allow(dead_code))]
pub(crate) fn headphone_gain_q15(level: Option<(u16, u16)>) -> u32 {
    let (value, max) = match level {
        Some((value, max)) if max != 0 => (u64::from(value.min(max)), u64::from(max)),
        _ => (1, 4),
    };
    (MAX_GAIN_Q15 * value * value / (max * max)) as u32
}

/// One ring and one command slot per deck; `deck-play` uses deck 1 only.
pub(crate) static RINGS: [Ring; 2] = [Ring::new(), Ring::new()];
pub(crate) static COMMANDS: [Commands; 2] = [Commands::new(), Commands::new()];

struct Shared(UnsafeCell<Output>);
// SAFETY: only the C USB client task renders and reads the counters.
unsafe impl Sync for Shared {}
static OUTPUT: Shared = Shared(UnsafeCell::new(Output::new()));

/// Consumer entry for `controller_audio`, on the USB client task only.
#[cfg_attr(feature = "deck-mix", allow(dead_code))]
pub(crate) fn render(out: &mut [PcmFrame], device_rate: u32, playing: bool, gain_q15: u32) {
    // SAFETY: single caller task (see `Shared`); no reference escapes.
    let output = unsafe { &mut *OUTPUT.0.get() };
    output.render(&RINGS[0], out, device_rate, playing, gain_q15);
}

/// Allocates deck `deck`'s ring storage once, in the PSRAM heap.
pub(crate) fn attach_ring(deck: usize) -> bool {
    let storage = vec![PcmFrame::default(); RING_FRAMES as usize].into_boxed_slice();
    RINGS[deck].attach(Box::leak(storage))
}

/// Decks that decode: deck 1, and deck 2 with `deck-mix`.
pub(crate) const DECKS: usize = if cfg!(feature = "deck-mix") { 2 } else { 1 };

/// Load the given catalog track on deck 1 or 2 (1-based), when that deck
/// decodes. Called by the display task when a Load button assigns a track.
#[cfg(feature = "idf-deck-play")]
#[unsafe(no_mangle)]
pub extern "C" fn ruzina_legacy_play_load(deck: u32, index: u32) {
    if (1..=DECKS as u32).contains(&deck) {
        COMMANDS[deck as usize - 1].load(index);
    }
}

/// Writes frames played (low 32 bits), underrun packets, buffered source
/// frames and the source rate to `out[0..4]`.
///
/// # Safety
/// `out` must be valid for four `u32` writes. USB client task only.
#[cfg(feature = "idf-deck-play")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_play_stats(out: *mut u32) {
    if out.is_null() {
        return;
    }
    // SAFETY: single caller task (see `Shared`).
    let output = unsafe { &*OUTPUT.0.get() };
    let values = [
        output.played,
        output.underruns,
        output.reader.buffered(&RINGS[0]),
        output.reader.rate,
    ];
    // SAFETY: the caller provides four writable u32 slots.
    unsafe { core::ptr::copy_nonoverlapping(values.as_ptr(), out, values.len()) };
}

#[cfg(test)]
mod tests;
