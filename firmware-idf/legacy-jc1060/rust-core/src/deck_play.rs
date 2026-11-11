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
use alloc::boxed::Box;
#[cfg(any(test, not(feature = "deck-engine")))]
use alloc::vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};
use ruzina_audio_dsp::{PcmFrame, ResamplerState};
use ruzina_audio_runtime::decode::{
    AudioCodec, DecodeAction, DecoderStorage, ENCODED_BUFFER_BYTES, MP3_CHECKPOINTS,
    Mp3CheckpointStorage, SOURCE_READ_BYTES, SourceDecoder, StreamChange,
};
use ruzina_audio_runtime::index_scan::IndexScanPolicy;
use ruzina_audio_runtime::pcm_stream::{PCM_STREAM_BLOCK_FRAMES, PcmStreamBlock};
use ruzina_audio_runtime::wav::{
    CHUNK_HEADER_BYTES, PCM_FORMAT_BYTES, RIFF_HEADER_BYTES, WavPcm16Format,
    WavPcm16SourceDescriptor, parse_chunk_header, parse_pcm16_format, parse_riff_header,
};
use ruzina_audio_scratch::{ScratchConfig, ScratchState};

/// Ring capacity in source frames: about 1.5 s at 44.1 kHz, 256 KiB of PSRAM.
pub(crate) const RING_FRAMES: u32 = 1 << 16;
/// Source frames of played audio each deck keeps for loops: about 23.8 s at
/// 44.1 kHz, 4 MiB of PSRAM, behind the ring in the same allocation.
pub(crate) const HISTORY_FRAMES: u32 = 1 << 20;
/// Source frames a loop start may lie before the history after a seek to it
/// (millisecond rounding).
const LOOP_START_SLACK: u32 = 2;
/// The shortest loop that may wrap at the end of a track, in source frames
/// (about 23 ms): a loop set in the last moments lets the track finish
/// instead of buzzing on a few frames.
const LOOP_MIN_FRAMES_AT_END: u32 = 1024;
/// Source frames a loop wrap crossfades over (about 5.8 ms at 44.1 kHz). The
/// wrap is sample-exact, so without this the waveform steps and clicks.
const LOOP_CROSSFADE_FRAMES: u16 = 256;
/// Jog ticks in one turn of the FLX4 platter, as the device log suggests.
const JOG_TICKS_PER_TURN: f32 = 1024.0;
/// One platter turn at normal speed: a record at 33 1/3 rpm.
const SECONDS_PER_TURN: f32 = 1.8;
/// Frames a forward scratch keeps ready past its head, taken from the ring.
const SCRATCH_LOOKAHEAD_FRAMES: f32 = 64.0;
/// One deck's ring and history storage, in frames.
pub(crate) const RING_STORAGE_FRAMES: usize = (RING_FRAMES + HISTORY_FRAMES) as usize;

/// Decoder actions one prime may take before its first block (ID3 tags
/// with artwork are read through 512 bytes at a time).
#[cfg(all(test, feature = "deck-engine"))]
const PRIME_ACTIONS: u32 = 8192;

/// Decoder actions per producer poll, so a selection is served promptly.
const POLL_ACTIONS: u32 = 16;
const POLL_BUDGET_US: u64 = 1000;

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
    /// Single-writer descriptor sequence. All sequence and descriptor accesses
    /// use SeqCst: a validated read cannot straddle any writer mutation in
    /// the single total order. PCM slot ownership still uses head/tail.
    descriptor_seq: AtomicU32,
    epoch: AtomicU32,
    /// Where the current epoch's frames begin.
    start: AtomicU32,
    /// The current epoch's source rate; 0 holds the consumer silent.
    rate: AtomicU32,
    /// The source frame the current epoch starts at (a seek's target).
    origin: AtomicU32,
    /// The epoch whose source has delivered its last frame.
    ended: AtomicU32,
    low_supply_us: AtomicU32,
    prime_max_us: AtomicU32,
}

impl Ring {
    #[cfg(feature = "producer-profile")]
    pub(crate) fn profile_head(&self) -> u32 {
        self.head.load(Ordering::Acquire)
    }
    #[cfg(feature = "deck-engine")]
    pub(crate) fn has_storage(&self) -> bool {
        !self.frames.load(Ordering::Acquire).is_null()
    }
    pub(crate) const fn new() -> Self {
        Self {
            frames: AtomicPtr::new(core::ptr::null_mut()),
            head: AtomicU32::new(0),
            tail: AtomicU32::new(0),
            descriptor_seq: AtomicU32::new(0),
            epoch: AtomicU32::new(0),
            start: AtomicU32::new(0),
            rate: AtomicU32::new(0),
            origin: AtomicU32::new(0),
            ended: AtomicU32::new(0),
            low_supply_us: AtomicU32::new(u32::MAX),
            prime_max_us: AtomicU32::new(0),
        }
    }

    /// The current epoch, as the producer last opened it.
    #[cfg_attr(not(feature = "deck-engine"), allow(dead_code))]
    pub(crate) fn epoch(&self) -> u32 {
        self.epoch.load(Ordering::Acquire)
    }

    /// Producer-only receipt snapshot: no descriptor writer can overlap it.
    #[cfg(feature = "deck-engine")]
    pub(crate) fn seek_position(&self) -> (u32, u32, bool) {
        let d = self
            .snapshot()
            .expect("single producer owns descriptor writes");
        (
            d.epoch,
            if d.rate == 0 {
                0
            } else {
                (u64::from(d.origin) * 1000 / u64::from(d.rate)) as u32
            },
            d.ended == d.epoch && self.head.load(Ordering::Acquire) == d.start,
        )
    }

    /// Hands the ring its storage once; later calls keep the first.
    #[cfg(test)]
    pub(crate) fn attach(&self, storage: &'static mut [PcmFrame]) -> bool {
        storage.len() == RING_STORAGE_FRAMES
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

    /// Consumer-only history slot of source frame `position`; the producer
    /// never touches the storage past the ring.
    fn history_slot(&self, position: u32) -> Option<*mut PcmFrame> {
        let base = self.frames.load(Ordering::Acquire);
        // SAFETY: the offset stays inside the attached RING_STORAGE_FRAMES.
        (!base.is_null()).then(|| unsafe {
            base.add(RING_FRAMES as usize + (position % HISTORY_FRAMES) as usize)
        })
    }

    /// Producer: free slots.
    pub(crate) fn free(&self) -> u32 {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        RING_FRAMES - head.wrapping_sub(tail)
    }

    pub(crate) fn supply_us(&self, pitch: i16) -> u64 {
        let Some(d) = self.snapshot() else { return 0 };
        if d.rate == 0 {
            return 0;
        }
        let frames = self
            .head
            .load(Ordering::Acquire)
            .wrapping_sub(self.tail.load(Ordering::Acquire))
            .min(RING_FRAMES);
        let speed = (10_000i32 + i32::from(pitch)).max(1) as u64;
        u64::from(frames) * 1_000_000 * 10_000 / (u64::from(d.rate) * speed)
    }

    pub(crate) fn record_supply(&self, pitch: i16) {
        self.low_supply_us.fetch_min(
            self.supply_us(pitch).min(u64::from(u32::MAX)) as u32,
            Ordering::Relaxed,
        );
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
        self.descriptor_seq.fetch_add(1, Ordering::SeqCst);
        self.start
            .store(self.head.load(Ordering::Relaxed), Ordering::SeqCst);
        self.rate.store(rate, Ordering::SeqCst);
        self.origin.store(origin, Ordering::SeqCst);
        // Zero is reserved for "no source" and never matches a terminal epoch.
        let epoch = self.epoch.load(Ordering::SeqCst).wrapping_add(1).max(1);
        self.epoch.store(epoch, Ordering::SeqCst);
        self.ended.store(0, Ordering::SeqCst);
        self.descriptor_seq.fetch_add(1, Ordering::SeqCst);
    }

    /// Producer: the current source has pushed its last frame.
    pub(crate) fn end(&self) {
        self.descriptor_seq.fetch_add(1, Ordering::SeqCst);
        self.ended
            .store(self.epoch.load(Ordering::SeqCst), Ordering::SeqCst);
        self.descriptor_seq.fetch_add(1, Ordering::SeqCst);
    }

    /// One bounded attempt. The producer may be preempted mid-publication;
    /// the audio consumer must not spin waiting for that lower-priority task.
    fn snapshot(&self) -> Option<Descriptor> {
        self.snapshot_with(|| {})
    }

    fn snapshot_with(&self, after_fields: impl FnOnce()) -> Option<Descriptor> {
        let seq = self.descriptor_seq.load(Ordering::SeqCst);
        if !seq.is_multiple_of(2) {
            return None;
        }
        let descriptor = Descriptor {
            epoch: self.epoch.load(Ordering::SeqCst),
            start: self.start.load(Ordering::SeqCst),
            rate: self.rate.load(Ordering::SeqCst),
            origin: self.origin.load(Ordering::SeqCst),
            ended: self.ended.load(Ordering::SeqCst),
        };
        after_fields();
        (self.descriptor_seq.load(Ordering::SeqCst) == seq).then_some(descriptor)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Descriptor {
    epoch: u32,
    start: u32,
    rate: u32,
    origin: u32,
    ended: u32,
}

/// A loop wrap's crossfade: the audio that would have followed the wrap point
/// fades out while the loop start fades in.
#[derive(Clone, Copy)]
struct LoopFade {
    /// The source frame that would have played next without the wrap;
    /// `None` fades the held frame instead.
    from: Option<u32>,
    /// Frames left, and the fade's length.
    left: u16,
    total: u16,
    /// The last outgoing frame found, held when none follows (track end, or
    /// a fade that starts from whatever was playing).
    held: PcmFrame,
}

/// The consumer's view of the ring.
///
/// Every frame taken from the ring is also kept in the deck's history,
/// indexed by source frame. A loop plays its first pass from the ring; at
/// the loop end the reader replays the history from the loop start while the
/// ring waits at the loop end, so both the wrap and the exit are seamless.
#[derive(Clone, Copy)]
pub(crate) struct Reader {
    epoch: u32,
    tail: u32,
    rate: u32,
    /// The epoch's first source frame plus the frames taken from the ring.
    position: u32,
    /// The first source frame of this epoch the history holds.
    history_from: u32,
    /// The next source frame replayed from the history, while it trails
    /// `position` (a loop, or the rest of one after its exit).
    replay: Option<u32>,
    /// The active loop, in source frames: `[start, end)`.
    loop_frames: Option<(u32, u32)>,
    /// A loop end was reached without its start in the history.
    loop_missed: bool,
    /// The crossfade of the latest wrap, while it runs.
    fade: Option<LoopFade>,
    /// The last frame played, for a fade with nothing to fade out.
    last: PcmFrame,
    /// The platter gesture while the jog is touched: frames then come from
    /// the history around this head, and the ring only feeds it going forward.
    scratch: Option<ScratchState>,
    /// Whether the gesture's last frame was audio rather than silence.
    scratch_audible: bool,
}

impl Reader {
    pub(crate) const fn new() -> Self {
        Self {
            epoch: 0,
            tail: 0,
            rate: 0,
            position: 0,
            history_from: 0,
            replay: None,
            loop_frames: None,
            loop_missed: false,
            fade: None,
            last: PcmFrame { left: 0, right: 0 },
            scratch: None,
            scratch_audible: false,
        }
    }

    /// Whether the jog gesture owns this deck's audio.
    #[cfg_attr(not(feature = "deck-mix"), allow(dead_code))]
    pub(crate) const fn scratching(&self) -> bool {
        self.scratch.is_some()
    }

    /// Whether playback comes from the history (after a loop or a scratch).
    #[cfg(feature = "uac-profile")]
    pub(crate) const fn replaying(&self) -> bool {
        self.replay.is_some()
    }

    /// The jog was touched: playback holds where it is, until the platter
    /// moves. A deck that was `playing` fades out. Does nothing without a
    /// source, or during a gesture.
    #[cfg_attr(not(feature = "deck-engine"), allow(dead_code))]
    pub(crate) fn begin_scratch(&mut self, playing: bool) {
        if self.rate == 0 || self.scratch.is_some() {
            return;
        }
        let mut scratch = ScratchState::new();
        scratch.configure(ScratchConfig {
            frames_per_tick: self.rate as f32 * SECONDS_PER_TURN / JOG_TICKS_PER_TURN,
            ..ScratchConfig::default()
        });
        let cursor = self.cursor();
        let newest = self.position.saturating_sub(1);
        scratch.seed(newest.saturating_sub(cursor) as f32);
        self.scratch = Some(scratch);
        self.scratch_audible = false;
        self.fade = playing.then_some(LoopFade {
            from: Some(cursor),
            left: LOOP_CROSSFADE_FRAMES,
            total: LOOP_CROSSFADE_FRAMES,
            held: self.last,
        });
    }

    /// Fades from the last frame played to whatever plays next.
    fn fade_from_last(&mut self) {
        self.fade = Some(LoopFade {
            from: None,
            left: LOOP_CROSSFADE_FRAMES,
            total: LOOP_CROSSFADE_FRAMES,
            held: self.last,
        });
    }

    /// Platter movement in jog ticks; backward is negative.
    #[cfg_attr(not(feature = "deck-engine"), allow(dead_code))]
    pub(crate) fn scratch_jog(&mut self, ticks: i16) {
        if let Some(scratch) = &mut self.scratch {
            scratch.jog(ticks);
        }
    }

    /// The jog is not touched: playback continues from the head, through the
    /// history and then the ring, as after a loop. A deck that will not play
    /// on stops its platter here and ends once that faded out, because its
    /// frames stop at once when the gesture ends. Called for every block.
    #[cfg_attr(not(feature = "deck-engine"), allow(dead_code))]
    pub(crate) fn end_scratch(&mut self, playing: bool) {
        let Some(scratch) = &mut self.scratch else {
            return;
        };
        if !playing && (self.scratch_audible || self.fade.is_some()) {
            scratch.seed(scratch.head_back());
            return;
        }
        let at = self.cursor().max(self.history_from);
        self.scratch = None;
        self.replay = (at < self.position).then_some(at);
        self.fade_from_last();
    }

    /// The source frame the gesture's head is on.
    fn scratch_cursor(&self, scratch: &ScratchState) -> u32 {
        self.position
            .saturating_sub(1)
            .saturating_sub(scratch.head_back() as u32)
    }

    /// The next frame of the gesture; silence while the platter holds or
    /// stands at an edge of the audio.
    fn scratch_frame(&mut self, ring: &Ring, mut scratch: ScratchState) -> PcmFrame {
        if scratch.velocity() > 0.0 || scratch.velocity_target() > 0.0 {
            while scratch.head_back() < SCRATCH_LOOKAHEAD_FRAMES && self.ring_frame(ring).is_some()
            {
                scratch.advance_reference(1);
            }
        }
        let filled = self
            .position
            .saturating_sub(self.history_from)
            .min(HISTORY_FRAMES);
        let newest = self.position.wrapping_sub(1);
        let frame = scratch.render(filled, |back| {
            let slot = ring.history_slot(newest.wrapping_sub(back))?;
            // SAFETY: `back` is below `filled`, so the frame is in the
            // history, which only this consumer reads and writes.
            Some(unsafe { slot.read() })
        });
        self.scratch = Some(scratch);
        if frame.is_some() != self.scratch_audible {
            self.scratch_audible = frame.is_some();
            self.fade_from_last();
        }
        frame.unwrap_or_default()
    }

    /// The next frame from the ring, kept in the history.
    fn ring_frame(&mut self, ring: &Ring) -> Option<PcmFrame> {
        if self.tail == ring.head.load(Ordering::Acquire) {
            return None;
        }
        // SAFETY: slots between tail and the published head hold frames
        // the producer wrote before publishing; it never touches them again
        // until `tail` passes them.
        let frame = unsafe { ring.slot(self.tail)?.read() };
        if let Some(slot) = ring.history_slot(self.position) {
            // SAFETY: the history is consumer-only storage.
            unsafe { slot.write(frame) };
        }
        self.tail = self.tail.wrapping_add(1);
        self.position = self.position.wrapping_add(1);
        Some(frame)
    }

    /// The source frame playing next: at the jog's head, in the history, or
    /// from the ring.
    fn cursor(&self) -> u32 {
        match &self.scratch {
            Some(scratch) => self.scratch_cursor(scratch),
            None => self.replay.unwrap_or(self.position),
        }
    }

    /// Whether every frame from `at` up to `position` is in the history.
    fn holds(&self, at: u32) -> bool {
        at >= self.history_from && at <= self.position && self.position - at <= HISTORY_FRAMES
    }

    /// Goes back to the loop start. A start the history does not hold (it
    /// lies before this epoch, or too far back) is reported once through
    /// `take_loop_miss` and playback runs on; the loop stays set, so after the
    /// caller's seek to the start every later wrap replays the history.
    fn wrap(&mut self, start: u32) {
        // A seek to the loop start may land a frame or two after it.
        let start = if start < self.history_from && self.history_from - start <= LOOP_START_SLACK {
            self.history_from
        } else {
            start
        };
        if self.holds(start) {
            let from = self.cursor();
            self.replay = (start != self.position).then_some(start);
            let length = self
                .loop_frames
                .map_or(0, |(_, end)| end.saturating_sub(start));
            let total = u32::from(LOOP_CROSSFADE_FRAMES).min(length / 2) as u16;
            self.fade = (total > 0).then_some(LoopFade {
                from: Some(from),
                left: total,
                total,
                held: self.last,
            });
        } else {
            self.loop_missed = true;
        }
    }

    /// The frame that would have played at source frame `at` without the
    /// wrap: from the history, or ahead in the ring, which waits at the loop
    /// end. `None` past the audio available, as at the end of the track.
    fn outgoing(&self, ring: &Ring, at: u32) -> Option<PcmFrame> {
        let slot = if at < self.position {
            ring.history_slot(at)?
        } else {
            let ahead = at - self.position;
            if ahead >= ring.head.load(Ordering::Acquire).wrapping_sub(self.tail) {
                return None;
            }
            ring.slot(self.tail.wrapping_add(ahead))?
        };
        // SAFETY: frames before `position` are in the consumer's history;
        // slots from tail up to the published head are the producer's written
        // frames, which it does not touch until `tail` passes them.
        Some(unsafe { slot.read() })
    }

    /// Mixes the running wrap crossfade into `frame`.
    fn crossfade(&mut self, ring: &Ring, frame: PcmFrame) -> PcmFrame {
        let Some(mut fade) = self.fade else {
            return frame;
        };
        if let Some(from) = fade.from {
            fade.held = self.outgoing(ring, from).unwrap_or(fade.held);
            fade.from = Some(from.wrapping_add(1));
        }
        let (out, total) = (i32::from(fade.left), i32::from(fade.total));
        let mix = |outgoing: i16, incoming: i16| {
            ((i32::from(outgoing) * out + i32::from(incoming) * (total - out)) / total) as i16
        };
        let mixed = PcmFrame {
            left: mix(fade.held.left, frame.left),
            right: mix(fade.held.right, frame.right),
        };
        fade.left -= 1;
        self.fade = (fade.left > 0).then_some(fade);
        mixed
    }

    /// Sets the active loop in source frames. A loop whose end is already
    /// behind (a Loop Out lands up to a millisecond behind the reported
    /// position) wraps at once.
    #[cfg_attr(not(feature = "deck-engine"), allow(dead_code))]
    pub(crate) fn set_loop(&mut self, start: u32, end: u32) {
        if end <= start {
            return;
        }
        self.loop_frames = Some((start, end));
        self.loop_missed = false;
        // During a gesture the wrap waits for the release.
        if self.scratch.is_none() && self.cursor() > end {
            self.wrap(start);
        }
    }

    /// Ends the loop. Playback continues from where it is, through the rest
    /// of the history and then the ring.
    #[cfg_attr(not(feature = "deck-engine"), allow(dead_code))]
    pub(crate) fn clear_loop(&mut self) {
        self.loop_frames = None;
    }

    /// Moves playback `frames` source frames, the beat sync's jump to the other deck's
    /// phase: backward through the history, or forward through it and then the ring,
    /// with a crossfade. A jump past the history's newest frame skips ring frames
    /// without keeping them, so the history starts again there. Returns false,
    /// changing nothing, during a gesture or a loop, or when the history does not hold
    /// the frame or the ring has not buffered it.
    #[cfg_attr(not(feature = "deck-engine"), allow(dead_code))]
    pub(crate) fn shift(&mut self, ring: &Ring, frames: i32) -> bool {
        if self.rate == 0 || self.scratch.is_some() || self.loop_frames.is_some() {
            return false;
        }
        let from = self.cursor();
        let Some(target) = from.checked_add_signed(frames) else {
            return false;
        };
        if self.holds(target) {
            self.replay = (target != self.position).then_some(target);
            self.fade = Some(LoopFade {
                from: Some(from),
                left: LOOP_CROSSFADE_FRAMES,
                total: LOOP_CROSSFADE_FRAMES,
                held: self.last,
            });
            return true;
        }
        let ahead = target.wrapping_sub(self.position);
        if target < self.position || ahead > self.buffered(ring) {
            return false;
        }
        self.tail = self.tail.wrapping_add(ahead);
        self.position = target;
        self.history_from = target;
        self.replay = None;
        // The skipped slots are the producer's again, so the fade holds the last frame.
        self.fade_from_last();
        true
    }

    /// Whether a loop can go on at the end of the track: its start lies far
    /// enough before the last frame.
    fn loop_wraps_at_end(&self, start: u32) -> bool {
        start.saturating_add(LOOP_MIN_FRAMES_AT_END) <= self.position
    }

    /// While audio remains after the ring ran out: the jog holds the deck, the
    /// history is replaying, or a loop will wrap. A loop that cannot wrap lets
    /// the track finish.
    #[cfg_attr(not(feature = "deck-engine"), allow(dead_code))]
    pub(crate) fn looping(&self) -> bool {
        self.scratch.is_some()
            || self.replay.is_some()
            || self
                .loop_frames
                .is_some_and(|(start, _)| self.loop_wraps_at_end(start))
    }

    /// The loop start in milliseconds, once, after a loop end was reached
    /// without its start in the history; the caller seeks there.
    #[cfg_attr(not(feature = "deck-engine"), allow(dead_code))]
    pub(crate) fn take_loop_miss(&mut self, start_ms: u32) -> Option<u32> {
        core::mem::take(&mut self.loop_missed).then_some(start_ms)
    }

    /// `ms` in this epoch's source frames; 0 without a rate.
    #[cfg_attr(not(feature = "deck-engine"), allow(dead_code))]
    pub(crate) fn frame_at_ms(&self, ms: u32) -> u32 {
        (u64::from(ms) * u64::from(self.rate) / 1000).min(u64::from(u32::MAX)) as u32
    }

    /// Follows a new epoch; returns true when the source changed.
    pub(crate) fn sync(&mut self, ring: &Ring) -> bool {
        let Some(descriptor) = ring.snapshot() else {
            return false;
        };
        if descriptor.epoch == self.epoch {
            return false;
        }
        self.epoch = descriptor.epoch;
        self.tail = descriptor.start;
        self.rate = descriptor.rate;
        self.position = descriptor.origin;
        // A new source (load or seek) starts a new history; the loop stays,
        // and a touched jog starts a new gesture from it.
        self.history_from = descriptor.origin;
        self.scratch = None;
        self.replay = None;
        self.loop_missed = false;
        self.fade = None;
        ring.tail.store(self.tail, Ordering::Release);
        true
    }

    /// The next frame to play, with a loop wrap's crossfade.
    pub(crate) fn pop(&mut self, ring: &Ring) -> Option<PcmFrame> {
        let frame = self.next_source_frame(ring)?;
        let mixed = self.crossfade(ring, frame);
        self.last = mixed;
        Some(mixed)
    }

    /// The next source frame, exact: from the history or the ring.
    fn next_source_frame(&mut self, ring: &Ring) -> Option<PcmFrame> {
        if let Some(scratch) = self.scratch {
            return Some(self.scratch_frame(ring, scratch));
        }
        // At the loop end, or past it after a seek there; or at the end of the
        // track, when the loop end lies beyond it and can never be reached.
        if let Some((start, end)) = self.loop_frames
            && (self.cursor() >= end
                || (self.replay.is_none()
                    && self.tail == ring.head.load(Ordering::Acquire)
                    && self.loop_wraps_at_end(start)
                    && self.ended(ring)))
        {
            self.wrap(start);
        }
        if let Some(at) = self.replay {
            // SAFETY: `holds` kept every frame from `at` up to `position` in
            // the history, which only this consumer reads and writes.
            let frame = unsafe { ring.history_slot(at)?.read() };
            let next = at.wrapping_add(1);
            self.replay = (next != self.position).then_some(next);
            return Some(frame);
        }
        self.ring_frame(ring)
    }

    /// Returns the consumed slots to the producer.
    pub(crate) fn commit(&self, ring: &Ring) {
        ring.tail.store(self.tail, Ordering::Release);
    }

    pub(crate) fn ended(&self, ring: &Ring) -> bool {
        ring.snapshot()
            .is_some_and(|d| d.epoch == self.epoch && d.ended == self.epoch && self.epoch != 0)
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
        (u64::from(self.cursor()) * 1000 / u64::from(self.rate)) as u32
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
    /// Monotonic clock; default zero disables only the elapsed-time budget.
    fn now_us(&self) -> u64 {
        0
    }
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
    /// The MP3 frame walk reached the end of the file, `elapsed_ms` after
    /// the load began.
    Indexed { index: u32, elapsed_ms: u32 },
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
    #[cfg(feature = "producer-profile")]
    pub(crate) profile: crate::producer_profile::Stats,
    storage: Box<DecoderStorage>,
    checkpoints: [Box<Mp3CheckpointStorage>; MP3_CHECKPOINTS],
    encoded: Box<[u8]>,
    read: [u8; SOURCE_READ_BYTES],
    source: Option<Source<File>>,
    generation: u32,
    prime: Option<PrimeJob>,
    opening: Option<Opening<File>>,
    prime_started_us: u64,
    /// When the current track's load began, for the frame walk's report.
    loaded_at_us: u64,
    /// The background MP3 frame walk of the loaded track; stops after three
    /// failed reads in a row, until the next load.
    index_policy: IndexScanPolicy,
}
struct Opening<File> {
    file: File,
    index: u32,
    len: u64,
    phase: OpenPhase,
}
#[derive(Clone, Copy)]
enum OpenPhase {
    Header,
    Chunk {
        position: u64,
        end: u64,
        format: Option<WavPcm16Format>,
    },
    Format {
        position: u64,
        next: u64,
        end: u64,
    },
}

#[derive(Clone, Copy)]
enum PrimeJob {
    Load { index: u32, codec: u32 },
    Seek { index: u32, position_ms: u32 },
}

/// Initial decoder state, built at compile time. Building it as a value
/// puts a temporary of over 100 KiB on the 48 KiB task stack (the build's
/// frame guard caught 175 KiB), so each heap slot is filled by copying this.
static INITIAL_STORAGE: DecoderStorage = DecoderStorage::new();
static INITIAL_CHECKPOINT: Mp3CheckpointStorage = Mp3CheckpointStorage::new();

/// A heap copy of `initial`, never a stack temporary.
/// # Safety
/// Only use for audited inline storage with no ownership or self references;
/// a byte copy must be a valid independent value, including its destructor.
#[cfg(any(test, not(feature = "deck-engine")))]
unsafe fn boxed_copy<T>(initial: &'static T) -> Box<T> {
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
    #[cfg(feature = "deck-engine")]
    pub(crate) fn try_new() -> Option<Self> {
        use crate::deck_resources::{try_copy, try_zeros};
        Some(Self {
            #[cfg(feature = "producer-profile")]
            profile: crate::producer_profile::Stats::default(),
            // SAFETY: same audited inline, non-owning storage as new().
            storage: unsafe { try_copy(&INITIAL_STORAGE)? },
            checkpoints: [
                unsafe { try_copy(&INITIAL_CHECKPOINT)? },
                unsafe { try_copy(&INITIAL_CHECKPOINT)? },
                unsafe { try_copy(&INITIAL_CHECKPOINT)? },
            ],
            encoded: try_zeros(ENCODED_BUFFER_BYTES)?,
            read: [0; SOURCE_READ_BYTES],
            source: None,
            generation: 0,
            prime: None,
            opening: None,
            prime_started_us: 0,
            loaded_at_us: 0,
            index_policy: IndexScanPolicy::default(),
        })
    }

    #[cfg(any(test, not(feature = "deck-engine")))]
    pub(crate) fn new() -> Self {
        Self {
            #[cfg(feature = "producer-profile")]
            profile: crate::producer_profile::Stats::default(),
            // SAFETY: these concrete audited storage types contain only inline
            // decoder arrays/scalars, with no owning pointers or destructors.
            storage: unsafe { boxed_copy(&INITIAL_STORAGE) },
            checkpoints: core::array::from_fn(|_| unsafe { boxed_copy(&INITIAL_CHECKPOINT) }),
            encoded: vec![0u8; ENCODED_BUFFER_BYTES].into_boxed_slice(),
            read: [0; SOURCE_READ_BYTES],
            source: None,
            generation: 0,
            prime: None,
            opening: None,
            prime_started_us: 0,
            loaded_at_us: 0,
            index_policy: IndexScanPolicy::default(),
        }
    }

    fn close<F: SourceFiles<File = File>>(&mut self, files: &mut F) {
        if let Some(opening) = self.opening.take() {
            files.close(opening.file);
        }
        if let Some(source) = self.source.take() {
            files.close(source.file);
        }
    }

    /// Opens track `index` and decodes its first block into a new ring
    /// epoch. The ring is silenced first, so the old track stops at once.
    pub(crate) fn begin_load<F: SourceFiles<File = File>>(
        &mut self,
        files: &mut F,
        ring: &Ring,
        index: u32,
        path: Option<&str>,
    ) -> Option<Event> {
        self.prime = None;
        self.prime_started_us = files.now_us();
        self.loaded_at_us = self.prime_started_us;
        self.index_policy = IndexScanPolicy::default();
        self.close(files);
        ring.restart(0);
        let failed = |stage| Some(Event::Failed { index, stage });
        let Some(path) = path.filter(|path| !path.is_empty()) else {
            return failed(1);
        };
        let Some(file) = files.open(path) else {
            return failed(2);
        };
        let len = files.len(&file);
        self.generation = self.generation.wrapping_add(1).max(1);
        self.opening = Some(Opening {
            file,
            index,
            len,
            phase: OpenPhase::Header,
        });
        None
    }

    pub(crate) fn poll_prime<F: SourceFiles<File = File>>(
        &mut self,
        files: &mut F,
        ring: &Ring,
    ) -> Option<Event> {
        if self.opening.is_some() {
            return self.poll_open(files);
        }
        let job = self.prime?;
        match self.decode(files, ring, POLL_ACTIONS, true) {
            Some(0) => None, // Budget exhausted, keep decoder state for next turn.
            Some(rate) => {
                ring.prime_max_us.fetch_max(
                    files
                        .now_us()
                        .saturating_sub(self.prime_started_us)
                        .min(u64::from(u32::MAX)) as u32,
                    Ordering::Relaxed,
                );
                self.prime = None;
                Some(match job {
                    PrimeJob::Load { index, codec } => Event::Ready { index, codec, rate },
                    PrimeJob::Seek { index, position_ms } => Event::Seeked {
                        index,
                        position_ms,
                        ok: true,
                    },
                })
            }
            None => {
                self.prime = None;
                ring.restart(0);
                ring.end();
                Some(match job {
                    PrimeJob::Load { index, .. } => {
                        self.close(files);
                        Event::Failed { index, stage: 4 }
                    }
                    PrimeJob::Seek { index, position_ms } => {
                        if let Some(s) = self.source.as_mut() {
                            s.ended = true;
                        }
                        Event::Seeked {
                            index,
                            position_ms,
                            ok: false,
                        }
                    }
                })
            }
        }
    }

    /// One read of the MP3 frame walk (a no-op for other codecs and once the
    /// walk is complete). With the walk, a seek far from every checkpoint
    /// resumes at an exact indexed frame instead of decoding from the start
    /// of the file, one 512-byte card read at a time.
    fn index_step<F: SourceFiles<File = File>>(&mut self, files: &mut F) -> (bool, Option<Event>) {
        let Some(source) = self.source.as_mut() else {
            return (false, None);
        };
        if !self.index_policy.can_attempt() {
            return (false, None);
        }
        let Some((offset, len)) = source
            .stream
            .index_read(&mut self.storage, SOURCE_READ_BYTES)
        else {
            return (false, None);
        };
        let read = &mut self.read[..len];
        let accepted = files
            .read_at(&mut source.file, offset, read)
            .is_some_and(|bytes| {
                bytes != 0
                    && source
                        .stream
                        .provide_index(&mut self.storage, offset, &read[..bytes])
            });
        self.index_policy.record_result(accepted);
        let finished = accepted && self.storage.mp3_index().complete();
        (
            true,
            finished.then(|| Event::Indexed {
                index: source.index,
                elapsed_ms: (files.now_us().saturating_sub(self.loaded_at_us) / 1000)
                    .min(u64::from(u32::MAX)) as u32,
            }),
        )
    }

    /// The loaded MP3's frame walk has reached the end of the file.
    #[cfg(test)]
    pub(crate) fn indexed(&self) -> bool {
        self.storage.mp3_index().complete()
    }

    pub(crate) fn priming(&self) -> bool {
        self.prime.is_some() || self.opening.is_some()
    }

    /// One metadata read per turn, including long RIFF chunk chains.
    fn poll_open<F: SourceFiles<File = File>>(&mut self, files: &mut F) -> Option<Event> {
        let mut opening = self.opening.take()?;
        let index = opening.index;
        let offset = match opening.phase {
            OpenPhase::Header => 0,
            OpenPhase::Chunk { position, .. } | OpenPhase::Format { position, .. } => position,
        };
        let result = files.read_at(&mut opening.file, offset, &mut self.read);
        let mut stream = None;
        let valid = (|| -> Option<()> {
            let bytes = result?;
            match opening.phase {
                OpenPhase::Header => {
                    if bytes < RIFF_HEADER_BYTES {
                        return None;
                    }
                    let magic = &self.read[..4];
                    if magic == b"RIFF" {
                        let end =
                            u64::from(parse_riff_header(&self.read[..RIFF_HEADER_BYTES]).ok()?) + 8;
                        if end > opening.len {
                            return None;
                        }
                        opening.phase = OpenPhase::Chunk {
                            position: RIFF_HEADER_BYTES as u64,
                            end,
                            format: None,
                        };
                    } else {
                        let codec = if magic == b"fLaC" {
                            AudioCodec::Flac
                        } else if &magic[..3] == b"ID3"
                            || magic[0] == 0xff && magic[1] & 0xe0 == 0xe0
                        {
                            AudioCodec::Mp3
                        } else {
                            return None;
                        };
                        stream = Some((
                            SourceDecoder::compressed(
                                codec,
                                opening.len,
                                self.generation,
                                &mut self.storage,
                            )?,
                            codec,
                        ));
                    }
                }
                OpenPhase::Chunk {
                    position,
                    end,
                    format,
                } => {
                    if position + CHUNK_HEADER_BYTES as u64 > end {
                        return None;
                    }
                    let chunk =
                        parse_chunk_header(self.read.get(..CHUNK_HEADER_BYTES.min(bytes))?).ok()?;
                    let data = position + CHUNK_HEADER_BYTES as u64;
                    if data + u64::from(chunk.len) > end {
                        return None;
                    }
                    let next = data + chunk.padded_len();
                    if chunk.is_format() {
                        if chunk.len < PCM_FORMAT_BYTES as u32 {
                            return None;
                        }
                        opening.phase = OpenPhase::Format {
                            position: data,
                            next,
                            end,
                        };
                    } else if chunk.is_data() {
                        let desc =
                            WavPcm16SourceDescriptor::new(format?, data, u64::from(chunk.len))
                                .ok()?;
                        stream = Some((
                            SourceDecoder::wav(desc, opening.len, self.generation)?,
                            AudioCodec::WavPcm16,
                        ));
                    } else {
                        opening.phase = OpenPhase::Chunk {
                            position: next,
                            end,
                            format,
                        };
                    }
                }
                OpenPhase::Format { next, end, .. } => {
                    let format =
                        parse_pcm16_format(self.read.get(..PCM_FORMAT_BYTES.min(bytes))?).ok()?;
                    opening.phase = OpenPhase::Chunk {
                        position: next,
                        end,
                        format: Some(format),
                    };
                }
            }
            Some(())
        })()
        .is_some();
        if !valid {
            files.close(opening.file);
            return Some(Event::Failed { index, stage: 3 });
        }
        if let Some((stream, codec)) = stream {
            self.source = Some(Source {
                file: opening.file,
                index,
                stream: Box::new(stream),
                pending: None,
                ended: false,
            });
            self.prime = Some(PrimeJob::Load {
                index,
                codec: codec_number(codec),
            });
        } else {
            self.opening = Some(opening);
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
        #[cfg(feature = "producer-profile")]
        let (started, children) = (files.now_us(), self.profile.children());
        let result = self.decode_inner(files, ring, actions, prime);
        #[cfg(feature = "producer-profile")]
        self.profile
            .finish_decode(started, files.now_us(), children);
        result
    }

    fn decode_inner<F: SourceFiles<File = File>>(
        &mut self,
        files: &mut F,
        ring: &Ring,
        actions: u32,
        prime: bool,
    ) -> Option<u32> {
        let started = files.now_us();
        let source = self.source.as_mut()?;
        if let Some(block) = source.pending.take() {
            #[cfg(feature = "producer-profile")]
            let before = files.now_us();
            let delivered = deliver(ring, &block);
            #[cfg(feature = "producer-profile")]
            self.profile
                .record(crate::producer_profile::PUSH, before, files.now_us());
            if !delivered {
                source.pending = Some(block);
                return Some(0);
            }
        }
        for action in 0..actions {
            if action != 0 && files.now_us().saturating_sub(started) >= POLL_BUDGET_US {
                break;
            }
            if source.ended || !prime && ring.free() < PCM_STREAM_BLOCK_FRAMES as u32 {
                break;
            }
            let mut checkpoints = self.checkpoints.each_mut().map(|slot| &mut **slot);
            #[cfg(feature = "producer-profile")]
            let before = files.now_us();
            let action = source
                .stream
                .step(&mut self.storage, &mut checkpoints, &mut self.encoded);
            #[cfg(feature = "producer-profile")]
            self.profile
                .record(crate::producer_profile::STEP, before, files.now_us());
            match action {
                DecodeAction::Read { offset, maximum } => {
                    let read = &mut self.read[..maximum.min(SOURCE_READ_BYTES)];
                    #[cfg(feature = "producer-profile")]
                    let before = files.now_us();
                    let bytes = files.read_at(&mut source.file, offset, read);
                    #[cfg(feature = "producer-profile")]
                    {
                        self.profile
                            .record(crate::producer_profile::READ, before, files.now_us());
                        self.profile.read_bytes(bytes.unwrap_or(0));
                    }
                    let provided = bytes.is_some_and(|bytes| {
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
                    if block.faulted || prime && block.len == 0 && !block.eof {
                        source.ended = true;
                        ring.end();
                        return None;
                    }
                    let rate = block.sample_rate_hz;
                    source.ended = block.eof;
                    if prime {
                        ring.restart_at(rate, block.first_frame as u32);
                    }
                    #[cfg(feature = "producer-profile")]
                    let before = files.now_us();
                    let delivered = deliver(ring, &block);
                    #[cfg(feature = "producer-profile")]
                    self.profile
                        .record(crate::producer_profile::PUSH, before, files.now_us());
                    if !delivered {
                        source.pending = Some(Box::new(block));
                    }
                    if prime {
                        return Some(rate);
                    }
                }
            }
        }
        Some(0)
    }

    #[cfg(feature = "producer-profile")]
    pub(crate) fn profile_poll(
        &mut self,
        ring: &Ring,
        start: u64,
        end: u64,
        busy: bool,
        head: u32,
    ) {
        self.profile
            .poll(start, end, busy, ring.profile_head().wrapping_sub(head));
    }

    #[cfg(feature = "producer-profile")]
    pub(crate) fn profile_publish(
        &self,
        deck: usize,
        ring: &Ring,
        now: u64,
        playing: bool,
        pitch: i16,
    ) {
        let mut words = self.profile.words();
        words[2..10].copy_from_slice(&[
            now as u32,
            (now >> 32) as u32,
            ring.epoch(),
            self.source.as_ref().map_or(u32::MAX, |s| s.index),
            ring.rate.load(Ordering::Acquire),
            u32::from(playing),
            i32::from(pitch) as u32,
            ring.supply_us(pitch).min(u64::from(u32::MAX)) as u32,
        ]);
        crate::producer_profile::publish(deck, words);
    }

    /// Moves the loaded track to `position_ms` and decodes its first block
    /// there into a new ring epoch; the old epoch plays on until then.
    pub(crate) fn begin_seek<F: SourceFiles<File = File>>(
        &mut self,
        files: &mut F,
        ring: &Ring,
        position_ms: u32,
    ) -> Option<Event> {
        self.prime_started_us = files.now_us();
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
        if !changed {
            source.ended = true;
            self.prime = None;
            ring.restart(0);
            ring.end();
            return Some(Event::Seeked {
                index,
                position_ms,
                ok: false,
            });
        }
        self.prime = Some(PrimeJob::Seek { index, position_ms });
        let _ = files; // Further I/O occurs only in bounded poll_prime turns.
        None
    }

    #[cfg(all(test, feature = "deck-engine"))]
    pub(crate) fn seek<F: SourceFiles<File = File>>(
        &mut self,
        files: &mut F,
        ring: &Ring,
        position_ms: u32,
    ) -> Option<Event> {
        if let Some(event) = self.begin_seek(files, ring, position_ms) {
            return Some(event);
        }
        if !self.priming() {
            return None;
        }
        for _ in 0..PRIME_ACTIONS {
            if let Some(event) = self.poll_prime(files, ring) {
                return Some(event);
            }
        }
        panic!("test seek never primed");
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
                self.begin_load(files, ring, index, path.as_ref().map(AsRef::as_ref)),
            );
        }
        if let Some(position_ms) = commands.take_seek() {
            let event = self.begin_seek(files, ring, position_ms);
            return (self.priming() || event.is_some(), event);
        }
        if self.priming() {
            return (true, self.poll_prime(files, ring));
        }
        let Some(source) = &self.source else {
            return (false, None);
        };
        if source.ended && source.pending.is_none() || ring.free() < PCM_STREAM_BLOCK_FRAMES as u32
        {
            // Nothing to decode: spend the turn on one read of the MP3 frame
            // walk. A read counts as work, so the owner polls again without
            // waiting a 10 ms tick and the walk runs at card speed, one read
            // per deck per turn; decoding always comes first.
            return self.index_step(files);
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
    #[cfg(test)]
    pub(crate) fn seek(&self, position_ms: u32) {
        self.seek
            .store(position_ms.saturating_add(1), Ordering::Release);
    }

    pub(crate) fn take_seek(&self) -> Option<u32> {
        self.seek.swap(0, Ordering::Acquire).checked_sub(1)
    }

    #[cfg(any(test, not(feature = "deck-engine")))]
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
    #[cfg_attr(feature = "deck-engine", allow(dead_code))]
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
        ring.record_supply(0);
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
    if crate::media_signal::muted() {
        output.resampler.reset();
        out.fill(PcmFrame::default());
        return;
    }
    output.render(&RINGS[0], out, device_rate, playing, gain_q15);
}

/// Allocates deck `deck`'s ring storage once, in the PSRAM heap.
#[cfg(any(test, not(feature = "deck-engine")))]
pub(crate) fn attach_ring(deck: usize) -> bool {
    if !RINGS[deck].frames.load(Ordering::Acquire).is_null() {
        return true;
    }
    let storage = vec![PcmFrame::default(); RING_STORAGE_FRAMES].into_boxed_slice();
    install_ring(&RINGS[deck], storage)
}

pub(crate) fn install_ring(ring: &Ring, mut storage: Box<[PcmFrame]>) -> bool {
    if storage.len() != RING_STORAGE_FRAMES {
        return false;
    }
    if ring
        .frames
        .compare_exchange(
            core::ptr::null_mut(),
            storage.as_mut_ptr(),
            Ordering::Release,
            Ordering::Relaxed,
        )
        .is_err()
    {
        return false;
    }
    let _ = Box::leak(storage);
    true
}

/// Decks that decode: deck 1, and deck 2 with `deck-mix`.
pub(crate) const DECKS: usize = if cfg!(feature = "deck-mix") { 2 } else { 1 };

/// Load the given catalog track on deck 1 or 2 (1-based), when that deck
/// decodes. Called by the display task when a Load button assigns a track.
#[cfg(feature = "idf-deck-play")]
#[unsafe(no_mangle)]
pub extern "C" fn ruzina_legacy_play_load(deck: u32, index: u32) {
    if (1..=DECKS as u32).contains(&deck) {
        #[cfg(feature = "deck-engine")]
        crate::deck_mix::with(|mix| {
            mix.engine().request_load(
                if deck == 2 {
                    ruzina_controller_core::DeckId::Two
                } else {
                    ruzina_controller_core::DeckId::One
                },
                index,
            );
        });
        #[cfg(not(feature = "deck-engine"))]
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

/// Minimum buffered time while rendering and maximum prime duration, per deck.
/// u32::MAX low-water means no active rendering sample has been recorded.
/// # Safety
/// `out` must be valid for four u32 writes; values are atomic snapshots.
#[cfg(feature = "idf-deck-play")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_play_diagnostics(out: *mut u32) {
    if out.is_null() {
        return;
    }
    let values = [
        RINGS[0].low_supply_us.load(Ordering::Relaxed),
        RINGS[1].low_supply_us.load(Ordering::Relaxed),
        RINGS[0].prime_max_us.load(Ordering::Relaxed),
        RINGS[1].prime_max_us.load(Ordering::Relaxed),
    ];
    // SAFETY: caller lends four writable scalar slots.
    unsafe { core::ptr::copy_nonoverlapping(values.as_ptr(), out, 4) };
}

#[cfg(test)]
pub(crate) mod tests;
