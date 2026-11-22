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
use alloc::vec::Vec;
use core::fmt::Write;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU32, Ordering};
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
    /// When the snapshot was stored, in microseconds since boot (low 32 bits).
    at_us: AtomicU32,
}

impl Live {
    const fn new() -> Self {
        Self {
            position_ms: AtomicU32::new(0),
            flags: AtomicU32::new(0),
            pitch_centipercent: AtomicI32::new(0),
            bpm_x100: AtomicU32::new(0),
            at_us: AtomicU32::new(0),
        }
    }

    pub(crate) fn store(&self, deck: DeckSnapshot) {
        self.position_ms.store(deck.position_ms, Ordering::Relaxed);
        self.flags.store(deck.flags, Ordering::Relaxed);
        self.pitch_centipercent
            .store(deck.pitch_centipercent, Ordering::Relaxed);
        self.bpm_x100.store(deck.bpm_x100, Ordering::Relaxed);
        self.at_us.store(clock_us(), Ordering::Relaxed);
    }

    /// When the snapshot [`load`](Self::load) gives was stored, as [`clock_us`] counts.
    /// The screen's; host tests build without it.
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) fn stored_us(&self) -> u32 {
        self.at_us.load(Ordering::Relaxed)
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

/// Microseconds since boot, low 32 bits; 0 in host tests.
pub(crate) fn clock_us() -> u32 {
    #[cfg(all(feature = "idf-deck-engine", not(test)))]
    {
        unsafe extern "C" {
            fn esp_timer_get_time() -> i64;
        }
        // SAFETY: IDF monotonic task-safe clock.
        unsafe { esp_timer_get_time() as u32 }
    }
    #[cfg(not(all(feature = "idf-deck-engine", not(test))))]
    0
}

/// The furthest a snapshot's position is carried forward: a little over two refreshes.
const AHEAD_MAX_US: u32 = 40_000;

/// `deck` as it will be `ahead_us` after its snapshot: a playing deck moves on at its
/// pitched tempo, for at most [`AHEAD_MAX_US`]. A paused or looping deck stays where it
/// is (a loop would jump back), and so does one given a time before its snapshot.
///
/// The engine stores a position once per USB audio fill, and a frame reaches the panel
/// 20 to 40 ms after the screen read it, a different time each frame. Drawn as read, the
/// lanes stepped unevenly; at the largest zoom (about 30 px a frame) the owner saw them
/// stutter. Drawn where the deck will be when the frame shows, they move evenly.
pub(crate) fn ahead(deck: DeckSnapshot, ahead_us: u32) -> DeckSnapshot {
    if !deck.has(PLAYING) || deck.has(LOOPING) || ahead_us > AHEAD_MAX_US {
        return deck;
    }
    let rate = 10_000 + i64::from(deck.pitch_centipercent);
    let moved_ms = i64::from(ahead_us) * rate / 10_000_000;
    DeckSnapshot {
        position_ms: (i64::from(deck.position_ms) + moved_ms.max(0)).min(i64::from(u32::MAX))
            as u32,
        ..deck
    }
}

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
    /// LOAD on the Library page: the selected track onto this deck.
    Load,
    /// A pad on the Hot Cues page went down (`true`) or up. With `delete` it clears its
    /// cue, as Shift and the pad do.
    HotCue { pad: u8, delete: bool, down: bool },
    /// A Beat FX setting on the FX page; it belongs to no deck.
    Fx(FxTouch),
    /// MT tapped: Master Tempo on or off.
    MasterTempo,
    /// The Library page's selected folder or playlist tapped again: open it, as the
    /// browse knob's press does. It belongs to no deck.
    LibraryOpen,
    /// The Library page's BACK: one level up, as Shift and the browse knob's press do.
    LibraryBack,
    /// A memory cue on the Hot Cues page tapped (0 to 15): the deck calls it, as the
    /// SAMPLER pads do.
    Memory(u8),
}

/// What the FX page sets: an effect (0 Filter to 3 Delay), a beat (0 for 1/4 to 4 for 4
/// beats), a channel (0 both, 1 deck 1, 2 deck 2), a depth (0 to 127), or FX ON pressed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FxTouch {
    Effect(u8),
    Beat(u8),
    Target(u8),
    Depth(u8),
    On,
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
                TouchControl::Load => 8,
                TouchControl::HotCue { pad, delete, down } => {
                    16 | u32::from(down) << 2 | u32::from(pad & 7) << 5 | u32::from(delete) << 8
                }
                TouchControl::Fx(fx) => {
                    let (kind, value) = match fx {
                        FxTouch::Effect(value) => (0, value),
                        FxTouch::Beat(value) => (1, value),
                        FxTouch::Target(value) => (2, value),
                        FxTouch::Depth(value) => (3, value),
                        FxTouch::On => (4, 0),
                    };
                    512 | kind << 10 | u32::from(value & 0x7f) << 13
                }
                TouchControl::MasterTempo => 1 << 20,
                TouchControl::LibraryOpen => 1 << 21,
                TouchControl::LibraryBack => 1 << 21 | 1 << 22,
                TouchControl::Memory(n) => 1 << 23 | u32::from(n & 15) << 24,
            };
        self.slots[(head % TOUCH_SLOTS) as usize].store(word, Ordering::Relaxed);
        self.head.store(head.wrapping_add(1), Ordering::Release);
        true
    }

    /// The producer's side: how many requests still fit.
    #[cfg(feature = "web-remote")]
    pub(crate) fn free(&self) -> u32 {
        let head = self.head.load(Ordering::Relaxed);
        TOUCH_SLOTS - head.wrapping_sub(self.tail.load(Ordering::Acquire))
    }

    /// The USB task's side: the oldest request.
    pub(crate) fn pop(&self) -> Option<(usize, TouchControl)> {
        let tail = self.tail.load(Ordering::Relaxed);
        if tail == self.head.load(Ordering::Acquire) {
            return None;
        }
        let word = self.slots[(tail % TOUCH_SLOTS) as usize].load(Ordering::Relaxed);
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        let control = if word & 1 << 23 != 0 {
            TouchControl::Memory((word >> 24 & 15) as u8)
        } else if word & 1 << 21 != 0 {
            if word & 1 << 22 != 0 {
                TouchControl::LibraryBack
            } else {
                TouchControl::LibraryOpen
            }
        } else if word & 1 << 20 != 0 {
            TouchControl::MasterTempo
        } else if word & 512 != 0 {
            let value = (word >> 13 & 0x7f) as u8;
            TouchControl::Fx(match word >> 10 & 7 {
                0 => FxTouch::Effect(value),
                1 => FxTouch::Beat(value),
                2 => FxTouch::Target(value),
                3 => FxTouch::Depth(value),
                _ => FxTouch::On,
            })
        } else if word & 16 != 0 {
            TouchControl::HotCue {
                pad: (word >> 5 & 7) as u8,
                delete: word & 256 != 0,
                down: word & 4 != 0,
            }
        } else if word & 8 != 0 {
            TouchControl::Load
        } else if word & 2 == 0 {
            TouchControl::Play
        } else {
            TouchControl::Cue(word & 4 != 0)
        };
        Some(((word & 1) as usize, control))
    }
}

pub(crate) static TOUCH: TouchQueue = TouchQueue::new();

/// The Web Remote's requests, from the web task: a second queue, as each has one producer.
#[cfg(feature = "web-remote")]
pub(crate) static WEB: TouchQueue = TouchQueue::new();

/// One catalog track as the Library page lists it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct LibraryRow {
    pub(crate) title: String,
    pub(crate) artist: String,
    pub(crate) key: String,
    pub(crate) bpm_x100: u32,
    pub(crate) duration_ms: u32,
}

/// The catalog's tracks by catalog index.
pub(crate) struct Library {
    pub(crate) rows: Vec<LibraryRow>,
}

static LIBRARY: Mailbox<Library> = Mailbox::new();
static LIBRARY_LEN: AtomicU32 = AtomicU32::new(0);
/// The selected row of the level shown.
static SELECTED: AtomicU32 = AtomicU32::new(0);

/// One folder or playlist of the export's playlist tree.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ShelfNode {
    pub(crate) name: String,
    pub(crate) folder: bool,
    /// The parent's node index, or `None` at the top.
    pub(crate) parent: Option<u32>,
    /// A folder's children, as node indexes, in Rekordbox's order.
    pub(crate) children: Vec<u32>,
    /// A playlist's tracks, as catalog indexes, in Rekordbox's order.
    pub(crate) tracks: Vec<u32>,
}

/// The export's playlist tree: the top level's nodes, and every node by index.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Shelf {
    pub(crate) top: Vec<u32>,
    pub(crate) nodes: Vec<ShelfNode>,
}

impl Shelf {
    /// The tree of a playlist index, from its top level down; nodes the index cannot
    /// reach are left out.
    pub(crate) fn from_index<S: ruzina_media_catalog::playlists::PlaylistStore>(
        index: &ruzina_media_catalog::playlists::PlaylistIndex<S>,
    ) -> Self {
        use ruzina_media_catalog::playlists::PLAYLIST_ROOT;
        let mut shelf = Self::default();
        // Each folder still to walk: its id, and its node index (`None` at the top).
        let mut pending = alloc::vec![(PLAYLIST_ROOT, None)];
        while let Some((id, parent)) = pending.pop() {
            for node in index.children(id) {
                let at = shelf.nodes.len() as u32;
                match parent {
                    None => shelf.top.push(at),
                    Some(parent) => shelf.nodes[parent as usize].children.push(at),
                }
                shelf.nodes.push(ShelfNode {
                    name: node.name.as_str().into(),
                    folder: node.is_folder,
                    parent,
                    children: Vec::new(),
                    tracks: if node.is_folder {
                        Vec::new()
                    } else {
                        index
                            .tracks(node.id)
                            .iter()
                            .map(|entry| u32::from(entry.track_index))
                            .collect()
                    },
                });
                if node.is_folder && id != node.id {
                    pending.push((node.id, Some(at)));
                }
            }
        }
        shelf
    }
}

/// The playlist tree, once the catalog has one. Published once a card and kept: the
/// engine and the UI task both read it.
static SHELF: AtomicPtr<Shelf> = AtomicPtr::new(core::ptr::null_mut());

/// The level the Library shows: `ALL` the catalog's tracks, `TOP` the playlist tree's
/// top level, otherwise a node index. Written by the engine.
static LEVEL: AtomicU32 = AtomicU32::new(ALL);
const ALL: u32 = u32::MAX;
const TOP: u32 = u32::MAX - 1;
/// The selection in the track list, while the playlists are shown.
static ALL_SELECTED: AtomicU32 = AtomicU32::new(0);

/// A level of the Library.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Level {
    /// Every track in the catalog.
    All,
    /// The playlist tree's top level.
    Top,
    /// A folder's children or a playlist's tracks, by node index.
    Node(u32),
}

/// A row of the level shown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Row {
    /// A track, by catalog index.
    Track(u32),
    /// A folder or playlist, by node index.
    Node(u32),
}

/// Hands the export's playlist tree to the Library. It starts again on the track list.
pub(crate) fn publish_shelf(shelf: Shelf) {
    // The tree lives as long as the firmware, for any task that read it. A new card
    // would leave the old one behind, which the legacy image does not do.
    let old = SHELF.swap(Box::into_raw(Box::new(shelf)), Ordering::AcqRel);
    #[cfg(test)]
    if !old.is_null() {
        // SAFETY: the tests read the tree only between publications.
        drop(unsafe { Box::from_raw(old) });
    }
    #[cfg(not(test))]
    let _ = old;
    LEVEL.store(ALL, Ordering::Relaxed);
}

/// Takes the playlist tree back down, for the next test.
#[cfg(test)]
pub(crate) fn clear_shelf() {
    let old = SHELF.swap(core::ptr::null_mut(), Ordering::AcqRel);
    if !old.is_null() {
        // SAFETY: as in `publish_shelf`.
        drop(unsafe { Box::from_raw(old) });
    }
    LEVEL.store(ALL, Ordering::Relaxed);
}

/// The playlist tree, if the catalog has one.
pub(crate) fn shelf() -> Option<&'static Shelf> {
    // SAFETY: a published tree is never freed outside the tests (`publish_shelf`).
    unsafe { SHELF.load(Ordering::Acquire).as_ref() }
}

/// The level shown.
pub(crate) fn level() -> Level {
    match LEVEL.load(Ordering::Relaxed) {
        ALL => Level::All,
        TOP => Level::Top,
        node => Level::Node(node),
    }
}

/// The Library page's view of a track list, sorted and filtered (`library_view`): for one
/// level, the positions of the rows it shows, in order. The screen task writes it; the
/// engine reads it, on the other core, for the browse knob and LOAD. A sequence count
/// guards it: odd while the screen task writes, so a reader that saw it change reads again.
pub(crate) const VIEW_MAX: usize = 1_024;
const NO_VIEW: u32 = u32::MAX - 2;
static VIEW_SEQUENCE: AtomicU32 = AtomicU32::new(0);
static VIEW_LEVEL: AtomicU32 = AtomicU32::new(NO_VIEW);
static VIEW_LEN: AtomicU32 = AtomicU32::new(0);
static VIEW_ROWS: [AtomicU32; VIEW_MAX] = [const { AtomicU32::new(0) }; VIEW_MAX];

const fn level_code(level: Level) -> u32 {
    match level {
        Level::All => ALL,
        Level::Top => TOP,
        Level::Node(node) => node,
    }
}

/// Shows `level` through `rows`, positions of its rows (`None`: as the level has them),
/// with row `selected` of it selected. The selection is written with the mapping, so LOAD
/// never pairs a new view with the old view's selection. The screen task's side. Rows
/// past `VIEW_MAX` are left out.
pub(crate) fn publish_view(level: Level, rows: Option<&[u32]>, selected: u32) {
    VIEW_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    SELECTED.store(selected, Ordering::Relaxed);
    match rows {
        Some(rows) => {
            let rows = &rows[..rows.len().min(VIEW_MAX)];
            for (slot, &row) in VIEW_ROWS.iter().zip(rows) {
                slot.store(row, Ordering::Relaxed);
            }
            VIEW_LEN.store(rows.len() as u32, Ordering::Relaxed);
            VIEW_LEVEL.store(level_code(level), Ordering::Relaxed);
        }
        None => VIEW_LEVEL.store(NO_VIEW, Ordering::Relaxed),
    }
    VIEW_SEQUENCE.fetch_add(1, Ordering::AcqRel);
}

/// What a read through the view found.
#[derive(Debug, Eq, PartialEq)]
enum ViewRead<R> {
    /// The view of the level, read whole.
    Shown(R),
    /// No view of the level: it shows its rows as it has them.
    Absent,
    /// The screen task kept writing the view: nothing is known of it. Reading the level
    /// as it is would map a row shown in a sorted list to another track.
    Busy,
}

/// What the view says of `level`, read whole by `read`. A read the screen task keeps
/// overwriting gives up after a few tries.
fn through_view<R>(level: Level, read: impl Fn() -> R) -> ViewRead<R> {
    for _ in 0..64 {
        let before = VIEW_SEQUENCE.load(Ordering::Acquire);
        if before.is_multiple_of(2) {
            let seen = VIEW_LEVEL.load(Ordering::Relaxed) == level_code(level);
            let value = seen.then(&read);
            core::sync::atomic::fence(Ordering::Acquire);
            if VIEW_SEQUENCE.load(Ordering::Relaxed) == before {
                return value.map_or(ViewRead::Absent, ViewRead::Shown);
            }
        }
        core::hint::spin_loop();
    }
    ViewRead::Busy
}

/// How many rows `level` shows: its view's, or all of them; 0 while the view is being
/// written (so the knob and LOAD wait for the next turn).
pub(crate) fn level_len(level: Level) -> u32 {
    match through_view(level, || VIEW_LEN.load(Ordering::Relaxed)) {
        ViewRead::Shown(len) => len,
        ViewRead::Absent => base_len(level),
        ViewRead::Busy => 0,
    }
}

/// How many rows `level` has, before any view.
pub(crate) fn base_len(level: Level) -> u32 {
    match level {
        Level::All => LIBRARY_LEN.load(Ordering::Acquire),
        Level::Top => shelf().map_or(0, |shelf| shelf.top.len() as u32),
        Level::Node(node) => shelf()
            .and_then(|shelf| shelf.nodes.get(node as usize))
            .map_or(0, |node| (node.children.len() + node.tracks.len()) as u32),
    }
}

/// Row `index` of what `level` shows.
pub(crate) fn row_at(level: Level, index: u32) -> Option<Row> {
    let position = through_view(level, || {
        (index < VIEW_LEN.load(Ordering::Relaxed))
            .then(|| VIEW_ROWS[index as usize].load(Ordering::Relaxed))
    });
    match position {
        ViewRead::Shown(position) => base_row_at(level, position?),
        ViewRead::Absent => base_row_at(level, index),
        ViewRead::Busy => None,
    }
}

/// Row `index` of `level`, before any view.
pub(crate) fn base_row_at(level: Level, index: u32) -> Option<Row> {
    let index = index as usize;
    match level {
        Level::All => (index < LIBRARY_LEN.load(Ordering::Acquire) as usize)
            .then_some(Row::Track(index as u32)),
        Level::Top => shelf()?.top.get(index).map(|&node| Row::Node(node)),
        Level::Node(node) => {
            let node = shelf()?.nodes.get(node as usize)?;
            node.children
                .get(index)
                .map(|&child| Row::Node(child))
                .or_else(|| node.tracks.get(index).map(|&track| Row::Track(track)))
        }
    }
}

/// The selected row of the level shown, read with the view in one go: the screen task
/// publishes a new view and its selection together (`publish_view`). `None` with nothing
/// selected, or while the view is being written: LOAD then does nothing rather than load
/// another track.
pub(crate) fn selected_row() -> Option<Row> {
    let level = level();
    let position = through_view(level, || {
        let len = VIEW_LEN.load(Ordering::Relaxed);
        (len != 0).then(|| {
            let index = SELECTED.load(Ordering::Relaxed).min(len - 1);
            VIEW_ROWS[index as usize].load(Ordering::Relaxed)
        })
    });
    match position {
        ViewRead::Shown(position) => base_row_at(level, position?),
        ViewRead::Absent => {
            let len = base_len(level);
            (len != 0)
                .then(|| SELECTED.load(Ordering::Relaxed).min(len - 1))
                .and_then(|index| base_row_at(level, index))
        }
        ViewRead::Busy => None,
    }
}

/// Opens the selected folder or playlist; false on a track or with nothing selected.
/// The engine's side.
pub(crate) fn open() -> bool {
    let Some(Row::Node(node)) = selected_row() else {
        return false;
    };
    LEVEL.store(node, Ordering::Relaxed);
    SELECTED.store(0, Ordering::Relaxed);
    true
}

/// One level up: the track list goes to the playlists' top level, the top level back
/// to the track list, and a folder or playlist to its parent. On the parent the
/// selection is the row of the folder or playlist it came from, however deep the tree.
/// False when there is nowhere to go. The engine's side.
pub(crate) fn back() -> bool {
    let (up, selected) = match level() {
        Level::All => {
            // The top level has no view.
            if base_len(Level::Top) == 0 {
                return false;
            }
            ALL_SELECTED.store(SELECTED.load(Ordering::Relaxed), Ordering::Relaxed);
            (TOP, 0)
        }
        Level::Top => (ALL, ALL_SELECTED.load(Ordering::Relaxed)),
        Level::Node(node) => {
            let shelf = shelf();
            let parent = shelf
                .and_then(|shelf| shelf.nodes.get(node as usize))
                .and_then(|node| node.parent);
            // The row this level was opened from: the node among its parent's rows.
            let row = shelf
                .and_then(|shelf| {
                    match parent {
                        Some(parent) => shelf.nodes.get(parent as usize)?.children.iter(),
                        None => shelf.top.iter(),
                    }
                    .position(|&child| child == node)
                })
                .unwrap_or(0);
            (parent.unwrap_or(TOP), row as u32)
        }
    };
    LEVEL.store(up, Ordering::Relaxed);
    let len = level_len(level());
    SELECTED.store(selected.min(len.saturating_sub(1)), Ordering::Relaxed);
    true
}

/// The names from the top level down to the level shown, for the page's heading.
pub(crate) fn path_names() -> Vec<&'static str> {
    let mut names = Vec::new();
    let mut at = match level() {
        Level::All | Level::Top => None,
        Level::Node(node) => Some(node),
    };
    // No path is longer than the tree has nodes; the bound only stops a broken parent loop.
    let bound = shelf().map_or(0, |shelf| shelf.nodes.len());
    while let Some(node) = at.and_then(|node| shelf()?.nodes.get(node as usize)) {
        names.push(node.name.as_str());
        at = node.parent;
        if names.len() >= bound {
            break;
        }
    }
    names.reverse();
    names
}
/// Whether the Library page is open: the browse knob then moves the selection rather
/// than the lanes' zoom. Written by the UI task.
pub(crate) static LIBRARY_OPEN: AtomicBool = AtomicBool::new(false);
/// The engine asks the UI task to show the Library page (the browse knob pressed on
/// another page).
pub(crate) static SHOW_LIBRARY: AtomicBool = AtomicBool::new(false);
/// The engine asks the UI task to go back to the Overview (the browse knob pressed on a
/// track in the Library).
pub(crate) static SHOW_OVERVIEW: AtomicBool = AtomicBool::new(false);

/// Rows one Shift + browse detent moves the selection.
pub(crate) const SHIFT_BROWSE_ROWS: i32 = 10;

/// Hands the catalog's tracks, by catalog index, to the Library page. The selection
/// stays, within the new list.
pub(crate) fn publish_library(rows: Vec<LibraryRow>) {
    LIBRARY_LEN.store(rows.len() as u32, Ordering::Release);
    #[cfg(feature = "web-remote")]
    WEB_LIBRARY.put(Box::new(WebLibrary {
        revision: WEB_REVISION
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1)
            .max(1),
        rows: rows.clone(),
    }));
    LIBRARY.put(Box::new(Library { rows }));
}

/// The catalog as the Web Remote lists it: its own copy, as the screen owns the other.
/// `revision` changes with each new list, so a page loads only what it listed.
#[cfg(feature = "web-remote")]
pub(crate) struct WebLibrary {
    pub(crate) revision: u32,
    pub(crate) rows: Vec<LibraryRow>,
}

#[cfg(feature = "web-remote")]
static WEB_LIBRARY: Mailbox<WebLibrary> = Mailbox::new();
#[cfg(feature = "web-remote")]
static WEB_REVISION: AtomicU32 = AtomicU32::new(0);

/// The catalog for the Web Remote, if a new list arrived since the last call. Web task.
#[cfg(feature = "web-remote")]
pub(crate) fn take_web_library() -> Option<Box<WebLibrary>> {
    WEB_LIBRARY.take()
}

/// A deck's loaded track as the Web Remote shows it.
#[cfg(feature = "web-remote")]
pub(crate) struct WebTrack {
    /// Catalog index; `u32::MAX` for no track.
    pub(crate) index: u32,
    pub(crate) title: String,
    pub(crate) artist: String,
    pub(crate) duration_ms: u32,
    /// The overview, [`WEB_WAVE_COLUMNS`] columns of the loudest peak (0 to 255); empty
    /// without an analysis.
    pub(crate) wave: Vec<u8>,
}

/// Columns of the Web Remote's overview: as many as a wide phone shows.
#[cfg(feature = "web-remote")]
const WEB_WAVE_COLUMNS: usize = 400;

/// `preview` in `columns` columns, each its range's loudest peak on 0 to 255.
#[cfg(feature = "web-remote")]
fn web_wave(preview: &[ruzina_track_analysis::WaveformColumn], columns: usize) -> Vec<u8> {
    if preview.is_empty() {
        return Vec::new();
    }
    (0..columns)
        .map(|column| {
            let start = column * preview.len() / columns;
            let end = ((column + 1) * preview.len() / columns).max(start + 1);
            let peak = preview[start..end.min(preview.len())]
                .iter()
                .map(|entry| entry.peak)
                .max()
                .unwrap_or(0);
            (peak / 257) as u8
        })
        .collect()
}

#[cfg(feature = "web-remote")]
static WEB_TRACKS: [Mailbox<WebTrack>; 2] = [Mailbox::new(), Mailbox::new()];

/// Deck `deck`'s new track for the Web Remote, if it loaded one since the last call.
/// Web task.
#[cfg(feature = "web-remote")]
pub(crate) fn take_web_track(deck: usize) -> Option<Box<WebTrack>> {
    WEB_TRACKS[deck].take()
}

/// The revision of the newest catalog list, as [`WebLibrary::revision`] gives it. The
/// USB task checks a Web Remote load against it.
#[cfg(feature = "web-remote")]
pub(crate) fn web_revision() -> u32 {
    WEB_REVISION.load(Ordering::Relaxed).max(1)
}

/// The Web Remote's other controls (the mixer, SYNC, PFL, Tempo, loops), from the web task
/// to the USB task as the controller sends them, oldest first.
#[cfg(feature = "web-remote")]
pub(crate) struct EventQueue {
    slots: [core::cell::UnsafeCell<Option<ruzina_controller_core::ControlEvent>>;
        EVENT_SLOTS as usize],
    head: AtomicU32,
    tail: AtomicU32,
}

#[cfg(feature = "web-remote")]
const EVENT_SLOTS: u32 = 32;

// SAFETY: one producer (the web task) writes a slot before publishing `head`, and one
// consumer (the USB task) reads it before releasing `tail`; neither touches the other's
// slots.
#[cfg(feature = "web-remote")]
unsafe impl Sync for EventQueue {}

#[cfg(feature = "web-remote")]
impl EventQueue {
    const fn new() -> Self {
        Self {
            slots: [const { core::cell::UnsafeCell::new(None) }; EVENT_SLOTS as usize],
            head: AtomicU32::new(0),
            tail: AtomicU32::new(0),
        }
    }

    /// The web task's side: how many events still fit.
    pub(crate) fn free(&self) -> u32 {
        EVENT_SLOTS
            - self
                .head
                .load(Ordering::Relaxed)
                .wrapping_sub(self.tail.load(Ordering::Acquire))
    }

    /// The web task's side: queues `event`; false when full.
    pub(crate) fn push(&self, event: ruzina_controller_core::ControlEvent) -> bool {
        if self.free() == 0 {
            return false;
        }
        let head = self.head.load(Ordering::Relaxed);
        // SAFETY: the slot at `head` is the producer's until `head` moves past it.
        unsafe { *self.slots[(head % EVENT_SLOTS) as usize].get() = Some(event) };
        self.head.store(head.wrapping_add(1), Ordering::Release);
        true
    }

    /// The USB task's side: the oldest event.
    pub(crate) fn pop(&self) -> Option<ruzina_controller_core::ControlEvent> {
        let tail = self.tail.load(Ordering::Relaxed);
        if tail == self.head.load(Ordering::Acquire) {
            return None;
        }
        // SAFETY: the producer published this slot before `head`; it is the consumer's
        // until `tail` moves past it.
        let event = unsafe { (*self.slots[(tail % EVENT_SLOTS) as usize].get()).take() };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        event
    }
}

#[cfg(feature = "web-remote")]
pub(crate) static WEB_EVENTS: EventQueue = EventQueue::new();

/// The mix as the Web Remote shows it, stored by the USB task after every turn:
/// [`MixView`]'s fields in order. A reader may see two turns mixed.
#[cfg(feature = "web-remote")]
pub(crate) static MIX_VIEW: [AtomicU32; MIX_VIEW_LEN] = [const { AtomicU32::new(0) }; MIX_VIEW_LEN];

#[cfg(feature = "web-remote")]
const MIX_VIEW_LEN: usize = 15;

/// The mixer and SYNC as the Web Remote reports them.
#[cfg(feature = "web-remote")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct MixView {
    pub(crate) volume: [u16; 2],
    pub(crate) crossfader: u16,
    pub(crate) master_volume: u16,
    pub(crate) headphone_mix: u16,
    /// Low, mid, high per channel.
    pub(crate) eq: [[u16; 3]; 2],
    pub(crate) filter: [u16; 2],
    pub(crate) pfl: [bool; 2],
    pub(crate) smart_cfx: bool,
    pub(crate) smart_fader: bool,
    pub(crate) sync: [bool; 2],
    pub(crate) sync_master: [bool; 2],
    /// Each deck's Hot Cue pads that hold a cue, as bits.
    pub(crate) hot_cues: [u8; 2],
}

#[cfg(feature = "web-remote")]
impl MixView {
    pub(crate) fn store(&self) {
        let flags = u32::from(self.pfl[0])
            | u32::from(self.pfl[1]) << 1
            | u32::from(self.smart_cfx) << 2
            | u32::from(self.smart_fader) << 3
            | u32::from(self.sync[0]) << 4
            | u32::from(self.sync[1]) << 5
            | u32::from(self.sync_master[0]) << 6
            | u32::from(self.sync_master[1]) << 7
            | u32::from(self.hot_cues[0]) << 8
            | u32::from(self.hot_cues[1]) << 16;
        let values = [
            u32::from(self.volume[0]),
            u32::from(self.volume[1]),
            u32::from(self.crossfader),
            u32::from(self.master_volume),
            u32::from(self.headphone_mix),
            u32::from(self.eq[0][0]),
            u32::from(self.eq[0][1]),
            u32::from(self.eq[0][2]),
            u32::from(self.eq[1][0]),
            u32::from(self.eq[1][1]),
            u32::from(self.eq[1][2]),
            u32::from(self.filter[0]),
            u32::from(self.filter[1]),
            flags,
            1, // published
        ];
        for (slot, value) in MIX_VIEW.iter().zip(values) {
            slot.store(value, Ordering::Relaxed);
        }
    }

    /// The latest view, or `None` before the USB task stored one.
    pub(crate) fn load() -> Option<Self> {
        let v: [u32; MIX_VIEW_LEN] = core::array::from_fn(|i| MIX_VIEW[i].load(Ordering::Relaxed));
        if v[14] == 0 {
            return None;
        }
        let bit = |n: u32| v[13] & 1 << n != 0;
        Some(Self {
            volume: [v[0] as u16, v[1] as u16],
            crossfader: v[2] as u16,
            master_volume: v[3] as u16,
            headphone_mix: v[4] as u16,
            eq: [
                [v[5] as u16, v[6] as u16, v[7] as u16],
                [v[8] as u16, v[9] as u16, v[10] as u16],
            ],
            filter: [v[11] as u16, v[12] as u16],
            pfl: [bit(0), bit(1)],
            smart_cfx: bit(2),
            smart_fader: bit(3),
            sync: [bit(4), bit(5)],
            sync_master: [bit(6), bit(7)],
            hot_cues: [(v[13] >> 8) as u8, (v[13] >> 16) as u8],
        })
    }
}

/// The catalog's tracks, if a new list arrived since the last call.
pub(crate) fn take_library() -> Option<Box<Library>> {
    LIBRARY.take()
}

/// The selected row of the level shown; `None` while it has no rows.
pub(crate) fn selected() -> Option<u32> {
    let len = level_len(level());
    (len != 0).then(|| SELECTED.load(Ordering::Relaxed).min(len - 1))
}

/// Selects row `index` of the level shown, within it.
pub(crate) fn select(index: u32) {
    let len = level_len(level());
    if len != 0 {
        SELECTED.store(index.min(len - 1), Ordering::Relaxed);
    }
}

/// Moves the selection `delta` rows, stopping at both ends.
pub(crate) fn browse(delta: i32) {
    if let Some(index) = selected() {
        select(index.saturating_add_signed(delta));
    }
}

/// Tracks in the catalog.
#[cfg_attr(test, allow(dead_code))]
pub(crate) fn library_len() -> u32 {
    LIBRARY_LEN.load(Ordering::Acquire)
}

/// The mix's figures for the Settings page, as `Mix::stats` gives them: rendered frames,
/// underrun blocks per deck, buffered frames per deck, source rate per deck, limited
/// MAIN samples. Written by the USB task outside the fill; a reader may see a mix of
/// two refreshes, which the page does not mind.
pub(crate) static STATS: [AtomicU32; 8] = [const { AtomicU32::new(0) }; 8];

/// Hands the Settings page the mix's latest figures.
pub(crate) fn publish_stats(stats: [u32; 8]) {
    for (slot, value) in STATS.iter().zip(stats) {
        slot.store(value, Ordering::Relaxed);
    }
}

/// Whether the Library page is open.
pub(crate) fn library_open() -> bool {
    LIBRARY_OPEN.load(Ordering::Relaxed)
}

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
/// The Web Remote's channel meters, read at each status request.
#[cfg(feature = "web-remote")]
pub(crate) static WEB_PEAKS: Peaks = Peaks::new();

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

/// What the load task knows about a track a deck has just loaded. Host tests have no
/// deck screen to read the texts.
#[cfg_attr(test, allow(dead_code))]
pub(crate) struct LoadedTrack {
    /// Catalog index; `u32::MAX` for no track.
    pub(crate) index: u32,
    pub(crate) title: String,
    pub(crate) artist: String,
    pub(crate) key: String,
    pub(crate) duration_ms: u32,
    /// Beat grid, waveforms and cues, when the export has an analysis.
    pub(crate) analysis: Option<AnalysisPublication>,
    /// The cover, `COVER_EDGE` pixels square in RGB565, when the export has a readable
    /// one.
    pub(crate) cover: Option<Vec<u16>>,
}

/// The deck screen's cover edge in pixels.
pub(crate) const COVER_EDGE: u16 = 56;

impl LoadedTrack {
    pub(crate) fn none() -> Self {
        Self {
            index: u32::MAX,
            title: String::new(),
            artist: String::new(),
            key: String::new(),
            duration_ms: 0,
            analysis: None,
            cover: None,
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

/// Hands deck `deck`'s new track to the UI, and its beat grid and memory cues to the
/// deck engine.
pub(crate) fn publish_track(deck: usize, track: Box<LoadedTrack>) {
    let view = track.analysis.as_ref().map(|analysis| analysis.view());
    let beats = view
        .as_ref()
        .and_then(|view| view.beat_grid())
        .map_or(&[][..], |grid| grid.beats());
    let memory = view.as_ref().map_or(&[][..], |view| view.memory_cues());
    GRIDS[deck].put(Box::new(Grid {
        index: track.index,
        beats: beats.into(),
        memory: memory
            .iter()
            .map(|cue| (cue.start_ms, cue.end_ms))
            .collect(),
    }));
    #[cfg(feature = "web-remote")]
    WEB_TRACKS[deck].put(Box::new(WebTrack {
        index: track.index,
        title: track.title.clone(),
        artist: track.artist.clone(),
        duration_ms: track.duration_ms,
        wave: web_wave(
            view.as_ref().map_or(&[][..], |view| view.preview()),
            WEB_WAVE_COLUMNS,
        ),
    }));
    TRACKS[deck].put(track);
}

/// Deck `deck`'s track, if a new one arrived since the last call.
pub(crate) fn take_track(deck: usize) -> Option<Box<LoadedTrack>> {
    TRACKS[deck].take()
}

/// The beat grid of the track a deck loaded, for the deck engine's beat sync, and its
/// memory cues, for their calls.
pub(crate) struct Grid {
    /// Catalog index; `u32::MAX` for no track.
    pub(crate) index: u32,
    /// Empty when the track has no analysis.
    pub(crate) beats: Box<[Beat]>,
    /// Rekordbox's memory cues in export order: each start, and a memory loop's end.
    pub(crate) memory: Box<[(u32, Option<u32>)]>,
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
