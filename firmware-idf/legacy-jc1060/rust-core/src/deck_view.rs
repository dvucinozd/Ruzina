//! What the deck screen shows, handed from the deck engine and the load task to the
//! UI task.
//!
//! - **Live state:** the engine stores each deck's position, flags, pitch and tempo,
//!   and the Beat FX, in atomics after every render block. The UI reads them when it
//!   draws; a torn read between fields lasts one frame.
//! - **Loaded track:** the load task hands over the title, key and the track's analysis
//!   once per load, through a one-place mailbox the UI empties, and the beat grid to
//!   the deck engine through another, for the beat sync.
// Without the screen only the engine's side and the tests use this module.
#![cfg_attr(not(feature = "slint-ui"), allow(dead_code))]

use alloc::boxed::Box;
use alloc::string::String;
use core::fmt::Write;
use core::sync::atomic::{AtomicI32, AtomicPtr, AtomicU32, Ordering};
use ruzina_track_analysis::Beat;
use ruzina_track_analysis::owned::AnalysisPublication;

pub(crate) const LOADED: u32 = 1;
pub(crate) const PLAYING: u32 = 1 << 1;
pub(crate) const SYNC: u32 = 1 << 2;
pub(crate) const MASTER_TEMPO: u32 = 1 << 3;
pub(crate) const LOOPING: u32 = 1 << 4;
pub(crate) const QUANTIZE: u32 = 1 << 5;

/// One deck at one moment.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DeckSnapshot {
    pub(crate) position_ms: u32,
    pub(crate) flags: u32,
    pub(crate) pitch_centipercent: i32,
    /// The track's own tempo, before pitch.
    pub(crate) bpm_x100: u32,
}

impl DeckSnapshot {
    pub(crate) const fn has(&self, flag: u32) -> bool {
        self.flags & flag != 0
    }
}

pub(crate) struct Live {
    position_ms: AtomicU32,
    flags: AtomicU32,
    pitch_centipercent: AtomicI32,
    bpm_x100: AtomicU32,
}

impl Live {
    const fn new() -> Self {
        Self {
            position_ms: AtomicU32::new(0),
            flags: AtomicU32::new(0),
            pitch_centipercent: AtomicI32::new(0),
            bpm_x100: AtomicU32::new(0),
        }
    }

    pub(crate) fn store(&self, deck: DeckSnapshot) {
        self.position_ms.store(deck.position_ms, Ordering::Relaxed);
        self.flags.store(deck.flags, Ordering::Relaxed);
        self.pitch_centipercent
            .store(deck.pitch_centipercent, Ordering::Relaxed);
        self.bpm_x100.store(deck.bpm_x100, Ordering::Relaxed);
    }

    pub(crate) fn load(&self) -> DeckSnapshot {
        DeckSnapshot {
            position_ms: self.position_ms.load(Ordering::Relaxed),
            flags: self.flags.load(Ordering::Relaxed),
            pitch_centipercent: self.pitch_centipercent.load(Ordering::Relaxed),
            bpm_x100: self.bpm_x100.load(Ordering::Relaxed),
        }
    }
}

pub(crate) static LIVE: [Live; 2] = [Live::new(), Live::new()];

/// What the lane marks on a deck's track: the Hot Cues by pad, and the running loop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Marks {
    /// Each pad's cue in milliseconds; `None` for an empty pad.
    pub(crate) cues: [Option<u32>; 8],
    /// The running loop, `[start_ms, end_ms)`.
    pub(crate) looped: Option<(u32, u32)>,
    /// Loop In while the loop waits for its Out: the lane shades from it to the playhead.
    pub(crate) pending_in: Option<u32>,
}

impl Marks {
    #[cfg(test)]
    pub(crate) const NONE: Self = Self {
        cues: [None; 8],
        looped: None,
        pending_in: None,
    };
}

const NO_MARK: u32 = u32::MAX;

/// One deck's marks, written by the USB task when they change and read by the UI task.
/// The revision is odd while a write runs; a read that sees it change tries again.
pub(crate) struct MarksSlot {
    revision: AtomicU32,
    cues: [AtomicU32; 8],
    looped: [AtomicU32; 2],
    pending_in: AtomicU32,
}

impl MarksSlot {
    const fn new() -> Self {
        Self {
            revision: AtomicU32::new(0),
            cues: [const { AtomicU32::new(NO_MARK) }; 8],
            looped: [const { AtomicU32::new(NO_MARK) }; 2],
            pending_in: AtomicU32::new(NO_MARK),
        }
    }

    /// Publishes `marks` when they differ from what the slot holds. One writer.
    pub(crate) fn store(&self, marks: Marks) {
        if self.load().1 == marks {
            return;
        }
        self.revision.fetch_add(1, Ordering::AcqRel);
        for (slot, cue) in self.cues.iter().zip(marks.cues) {
            slot.store(cue.unwrap_or(NO_MARK), Ordering::Relaxed);
        }
        let (start, end) = marks.looped.unwrap_or((NO_MARK, NO_MARK));
        self.looped[0].store(start, Ordering::Relaxed);
        self.looped[1].store(end, Ordering::Relaxed);
        self.pending_in
            .store(marks.pending_in.unwrap_or(NO_MARK), Ordering::Relaxed);
        self.revision.fetch_add(1, Ordering::Release);
    }

    /// The marks and their revision, which changes with each change of the marks.
    pub(crate) fn load(&self) -> (u32, Marks) {
        loop {
            let before = self.revision.load(Ordering::Acquire);
            let mark = |slot: &AtomicU32| {
                let value = slot.load(Ordering::Relaxed);
                (value != NO_MARK).then_some(value)
            };
            let marks = Marks {
                cues: core::array::from_fn(|i| mark(&self.cues[i])),
                looped: mark(&self.looped[0]).zip(mark(&self.looped[1])),
                pending_in: mark(&self.pending_in),
            };
            core::sync::atomic::fence(Ordering::Acquire);
            if before & 1 == 0 && self.revision.load(Ordering::Relaxed) == before {
                return (before, marks);
            }
        }
    }
}

pub(crate) static MARKS: [MarksSlot; 2] = [MarksSlot::new(), MarksSlot::new()];

/// The Beat FX panel: effect, beat, target, depth and on, packed in one word so the UI
/// never sees half a change.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct FxSnapshot {
    /// 0 Filter, 1 Echo, 2 Flanger, 3 Delay.
    pub(crate) effect: u8,
    /// 0 for 1/4 up to 4 for 4 beats.
    pub(crate) beat: u8,
    /// 0 both decks, 1 deck 1, 2 deck 2.
    pub(crate) target: u8,
    /// 0 to 127.
    pub(crate) depth: u8,
    pub(crate) on: bool,
}

impl FxSnapshot {
    pub(crate) const fn pack(self) -> u32 {
        (self.effect as u32 & 3)
            | (self.beat as u32 & 7) << 2
            | (self.target as u32 & 3) << 5
            | (self.depth as u32 & 0x7f) << 7
            | (self.on as u32) << 14
    }

    pub(crate) const fn unpack(word: u32) -> Self {
        Self {
            effect: (word & 3) as u8,
            beat: (word >> 2 & 7) as u8,
            target: (word >> 5 & 3) as u8,
            depth: (word >> 7 & 0x7f) as u8,
            on: word >> 14 & 1 != 0,
        }
    }

    pub(crate) const fn effect_name(self) -> &'static str {
        match self.effect {
            0 => "FILTER",
            1 => "ECHO",
            2 => "FLANGER",
            _ => "DELAY",
        }
    }

    pub(crate) const fn beat_name(self) -> &'static str {
        match self.beat {
            0 => "1/4",
            1 => "1/2",
            2 => "1",
            3 => "2",
            _ => "4",
        }
    }

    /// Depth in percent, as the panel shows it.
    pub(crate) const fn depth_percent(self) -> i32 {
        (self.depth as i32 * 100 + 63) / 127
    }
}

pub(crate) static FX: AtomicU32 = AtomicU32::new(0);

/// What a touch on the deck screen asks of a deck.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TouchControl {
    /// The PLAY button was pressed.
    Play,
    /// The CUE button went down (`true`) or up.
    Cue(bool),
}

const TOUCH_SLOTS: u32 = 16;

/// Touch requests from the UI task to the USB task, oldest first. A full queue drops the
/// newest request.
pub(crate) struct TouchQueue {
    slots: [AtomicU32; TOUCH_SLOTS as usize],
    head: AtomicU32,
    tail: AtomicU32,
}

impl TouchQueue {
    pub(crate) const fn new() -> Self {
        Self {
            slots: [const { AtomicU32::new(0) }; TOUCH_SLOTS as usize],
            head: AtomicU32::new(0),
            tail: AtomicU32::new(0),
        }
    }

    /// The UI task's side: queues `control` for deck `deck` (0 or 1).
    pub(crate) fn push(&self, deck: usize, control: TouchControl) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        if head.wrapping_sub(self.tail.load(Ordering::Acquire)) >= TOUCH_SLOTS {
            return false;
        }
        let word = (deck as u32 & 1)
            | match control {
                TouchControl::Play => 0,
                TouchControl::Cue(down) => 2 | u32::from(down) << 2,
            };
        self.slots[(head % TOUCH_SLOTS) as usize].store(word, Ordering::Relaxed);
        self.head.store(head.wrapping_add(1), Ordering::Release);
        true
    }

    /// The USB task's side: the oldest request.
    pub(crate) fn pop(&self) -> Option<(usize, TouchControl)> {
        let tail = self.tail.load(Ordering::Relaxed);
        if tail == self.head.load(Ordering::Acquire) {
            return None;
        }
        let word = self.slots[(tail % TOUCH_SLOTS) as usize].load(Ordering::Relaxed);
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        let control = if word & 2 == 0 {
            TouchControl::Play
        } else {
            TouchControl::Cue(word & 4 != 0)
        };
        Some(((word & 1) as usize, control))
    }
}

pub(crate) static TOUCH: TouchQueue = TouchQueue::new();

/// Each channel's loudest pre-fader sample since the UI last looked, 32 767 for full
/// scale.
pub(crate) struct Peaks([AtomicU32; 2]);

impl Peaks {
    const fn new() -> Self {
        Self([AtomicU32::new(0), AtomicU32::new(0)])
    }

    /// Adds a render block's channel peaks.
    pub(crate) fn add(&self, peaks: [u16; 2]) {
        for (slot, peak) in self.0.iter().zip(peaks) {
            slot.fetch_max(u32::from(peak), Ordering::Relaxed);
        }
    }

    /// Channel `deck`'s loudest sample since the last call.
    pub(crate) fn take(&self, deck: usize) -> u32 {
        self.0[deck].swap(0, Ordering::Relaxed)
    }
}

/// The screen's channel meters.
pub(crate) static PEAKS: Peaks = Peaks::new();
/// The controller's channel meters, read on their own so neither takes the other's
/// peaks.
pub(crate) static LED_PEAKS: Peaks = Peaks::new();

/// The controller's channel meter level, as the donor drives the FLX4's: the loudest
/// sample on a linear 0 to 127 scale, rising at once and falling 12 an update.
pub(crate) fn meter_led(previous: u8, peak: u32) -> u8 {
    let level = (peak * 127 / 32_768).min(127) as u8;
    level.max(previous.saturating_sub(12))
}

/// The channel meter's lit segments for `peak`, out of ten: green to -9 dBFS, amber at
/// -6 and -3, red from -0.5.
pub(crate) fn meter_segments(peak: u32) -> i32 {
    // -40, -32, -26, -20, -16, -12, -9, -6, -3 and -0.5 dBFS.
    const STEPS: [u32; 10] = [
        328, 823, 1_641, 3_277, 5_193, 8_231, 11_627, 16_423, 23_197, 30_934,
    ];
    STEPS.iter().filter(|&&step| peak >= step).count() as i32
}

/// The lane zoom: how many beats the lane shows, the donor's steps with one more at each
/// end. The browse knob moves it.
const ZOOM_BEATS: [u32; 7] = [2, 4, 8, 12, 16, 24, 32];
static ZOOM_STEP: AtomicU32 = AtomicU32::new(4);

/// Steps one Shift + browse detent moves the zoom.
pub(crate) const SHIFT_ZOOM_STEPS: i32 = 3;

/// Moves the zoom `delta` steps; clockwise shows more beats.
pub(crate) fn zoom_by(delta: i32) {
    let last = ZOOM_BEATS.len() as i32 - 1;
    let step = (ZOOM_STEP.load(Ordering::Relaxed) as i32)
        .saturating_add(delta)
        .clamp(0, last);
    ZOOM_STEP.store(step as u32, Ordering::Relaxed);
}

/// The time the lane shows for a track at `bpm_x100`: the zoom's beats, 500 ms each
/// without a tempo, between 0.5 s and 30 s.
pub(crate) fn window_ms(bpm_x100: u32) -> u32 {
    let beat_ms = 6_000_000u32.checked_div(bpm_x100).unwrap_or(500);
    let step = (ZOOM_STEP.load(Ordering::Relaxed) as usize).min(ZOOM_BEATS.len() - 1);
    beat_ms.saturating_mul(ZOOM_BEATS[step]).clamp(500, 30_000)
}

/// What the load task knows about a track a deck has just loaded.
pub(crate) struct LoadedTrack {
    /// Catalog index; `u32::MAX` for no track.
    pub(crate) index: u32,
    pub(crate) title: String,
    pub(crate) artist: String,
    pub(crate) key: String,
    pub(crate) duration_ms: u32,
    /// Beat grid, waveforms and cues, when the export has an analysis.
    pub(crate) analysis: Option<AnalysisPublication>,
}

impl LoadedTrack {
    pub(crate) fn none() -> Self {
        Self {
            index: u32::MAX,
            title: String::new(),
            artist: String::new(),
            key: String::new(),
            duration_ms: 0,
            analysis: None,
        }
    }
}

/// A one-place mailbox: the newest value waits for its reader, and one the reader never
/// took is dropped by the writer.
struct Mailbox<T>(AtomicPtr<T>);

impl<T> Mailbox<T> {
    const fn new() -> Self {
        Self(AtomicPtr::new(core::ptr::null_mut()))
    }

    fn put(&self, value: Box<T>) {
        let old = self.0.swap(Box::into_raw(value), Ordering::AcqRel);
        if !old.is_null() {
            // SAFETY: only `put` stores into the slot, from `Box::into_raw`, and the swap
            // gave this caller sole ownership of the pointer it replaced.
            drop(unsafe { Box::from_raw(old) });
        }
    }

    fn take(&self) -> Option<Box<T>> {
        let value = self.0.swap(core::ptr::null_mut(), Ordering::AcqRel);
        // SAFETY: as in `put`: the swap hands the pointer to this caller alone.
        (!value.is_null()).then(|| unsafe { Box::from_raw(value) })
    }
}

static TRACKS: [Mailbox<LoadedTrack>; 2] = [Mailbox::new(), Mailbox::new()];

/// Hands deck `deck`'s new track to the UI, and its beat grid to the deck engine.
pub(crate) fn publish_track(deck: usize, track: Box<LoadedTrack>) {
    let beats = track
        .analysis
        .as_ref()
        .and_then(|analysis| analysis.view().beat_grid())
        .map_or(&[][..], |grid| grid.beats());
    GRIDS[deck].put(Box::new(Grid {
        index: track.index,
        beats: beats.into(),
    }));
    TRACKS[deck].put(track);
}

/// Deck `deck`'s track, if a new one arrived since the last call.
pub(crate) fn take_track(deck: usize) -> Option<Box<LoadedTrack>> {
    TRACKS[deck].take()
}

/// The beat grid of the track a deck loaded, for the deck engine's beat sync.
pub(crate) struct Grid {
    /// Catalog index; `u32::MAX` for no track.
    pub(crate) index: u32,
    /// Empty when the track has no analysis.
    pub(crate) beats: Box<[Beat]>,
}

static GRIDS: [Mailbox<Grid>; 2] = [Mailbox::new(), Mailbox::new()];

/// Deck `deck`'s beat grid, if a new track arrived since the last call.
pub(crate) fn take_grid(deck: usize) -> Option<Box<Grid>> {
    GRIDS[deck].take()
}

/// `mm:ss`; an hour or more shows as `h:mm:ss`.
pub(crate) fn clock(ms: u32) -> String {
    let seconds = ms / 1_000;
    let mut text = String::new();
    if seconds >= 3_600 {
        let _ = write!(
            text,
            "{}:{:02}:{:02}",
            seconds / 3_600,
            seconds / 60 % 60,
            seconds % 60
        );
    } else {
        let _ = write!(text, "{:02}:{:02}", seconds / 60, seconds % 60);
    }
    text
}

/// Time left as `-mm:ss`, rounded up so it reaches `-00:00` only at the end.
pub(crate) fn remaining(position_ms: u32, duration_ms: u32) -> String {
    let left = duration_ms.saturating_sub(position_ms);
    let mut text = String::from("-");
    text.push_str(&clock(left.div_ceil(1_000) * 1_000));
    text
}

/// Pitch as `+1.56%`.
pub(crate) fn pitch(centipercent: i32) -> String {
    let mut text = String::new();
    let sign = if centipercent < 0 { '-' } else { '+' };
    let value = centipercent.unsigned_abs();
    let _ = write!(text, "{sign}{}.{:02}%", value / 100, value % 100);
    text
}

/// The tempo the deck plays at: the track's BPM with the pitch, two decimals.
pub(crate) fn bpm(bpm_x100: u32, centipercent: i32) -> String {
    let mut text = String::new();
    if bpm_x100 == 0 {
        text.push_str("--.--");
        return text;
    }
    let played = (i64::from(bpm_x100) * (10_000 + i64::from(centipercent)) + 5_000) / 10_000;
    let played = played.max(0) as u64;
    let _ = write!(text, "{}.{:02}", played / 100, played % 100);
    text
}

#[cfg(test)]
mod tests;
