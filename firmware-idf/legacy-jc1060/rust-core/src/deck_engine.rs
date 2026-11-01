//! The product deck engine, `ruzina-deck`'s `DeckProductState`, in place of
//! the diagnostic Play latch.
//!
//! Every controller event reaches `handle_control` on the USB client task.
//! This module is the transport owner for its effects, as the native
//! firmware's transport owner is:
//! - `PlaybackRequest` and `Pause` gate rendering;
//! - `Seek` reaches the deck's producer, which reopens the decoder at that
//!   position in a new ring epoch;
//! - `SetPitchCentipercent` is the render pitch, and `JogNudge` adds the
//!   product's decaying tempo bend.
//!
//! Scratch, loops, pad and beat FX effects are counted and not executed
//! yet. The renderer reports each deck's source position back with
//! `set_position_ms`, so CUE sets and returns to the real position; a seek
//! holds those reports until its new epoch arrives. A loaded track is
//! committed with `commit_track_load` when its producer has decoded the
//! first block, through the `LOADED` mailbox; Load is refused while the deck
//! plays (the product's load lock).
use crate::deck_play::{COMMANDS, RINGS, Reader};
use core::sync::atomic::{AtomicU32, Ordering, fence};
use ruzina_audio_runtime::nudge::TempoNudge;
use ruzina_audio_runtime::render::RENDER_SAMPLE_RATE_HZ;
use ruzina_controller_core::{ControlEvent, DeckId};
use ruzina_core::MediaTrackId;
use ruzina_deck::{DeckEffect, DeckEffects, DeckProductState};
use ruzina_hot_cues::HotCueBank;

const DECKS: [DeckId; 2] = [DeckId::One, DeckId::Two];

fn index(deck: DeckId) -> usize {
    usize::from(deck == DeckId::Two)
}

/// What a deck's producer last loaded, from the SD load task to the engine.
/// A sequence lock: the single writer makes `seq` odd while it writes.
pub(crate) struct Loaded {
    seq: AtomicU32,
    /// Ready (1) or failed (2), catalog index, BPM x100, track id words.
    words: [AtomicU32; 7],
}

/// One published load.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LoadedTrack {
    pub(crate) ready: bool,
    pub(crate) index: u32,
    pub(crate) bpm_x100: u32,
    pub(crate) track_id: [u8; 16],
}

impl Loaded {
    pub(crate) const fn new() -> Self {
        Self {
            seq: AtomicU32::new(0),
            words: [const { AtomicU32::new(0) }; 7],
        }
    }

    /// SD load task only.
    pub(crate) fn post(&self, track: LoadedTrack) {
        let seq = self.seq.load(Ordering::Relaxed);
        self.seq.store(seq.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        let mut words = [
            if track.ready { 1 } else { 2 },
            track.index,
            track.bpm_x100,
            0,
            0,
            0,
            0,
        ];
        for (word, bytes) in words[3..].iter_mut().zip(track.track_id.chunks_exact(4)) {
            *word = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        for (slot, word) in self.words.iter().zip(words) {
            slot.store(word, Ordering::Relaxed);
        }
        self.seq.store(seq.wrapping_add(2), Ordering::Release);
    }

    /// A load newer than `seen`, read whole; `None` while none or mid-write.
    pub(crate) fn take(&self, seen: &mut u32) -> Option<LoadedTrack> {
        let seq = self.seq.load(Ordering::Acquire);
        if seq == *seen || seq % 2 == 1 {
            return None;
        }
        let words: [u32; 7] = core::array::from_fn(|i| self.words[i].load(Ordering::Relaxed));
        fence(Ordering::Acquire);
        if self.seq.load(Ordering::Relaxed) != seq {
            return None;
        }
        *seen = seq;
        let mut track_id = [0u8; 16];
        for (bytes, word) in track_id.chunks_exact_mut(4).zip(&words[3..]) {
            bytes.copy_from_slice(&word.to_le_bytes());
        }
        Some(LoadedTrack {
            ready: words[0] == 1,
            index: words[1],
            bpm_x100: words[2],
            track_id,
        })
    }
}

pub(crate) static LOADED: [Loaded; 2] = [Loaded::new(), Loaded::new()];

/// Per-deck status for the display task: position in milliseconds, and
/// flags (bit 0 loaded, bit 1 playing).
static STATUS: [[AtomicU32; 2]; 2] = [
    [AtomicU32::new(0), AtomicU32::new(0)],
    [AtomicU32::new(0), AtomicU32::new(0)],
];

pub(crate) struct Engine {
    deck: DeckProductState,
    connected: bool,
    loaded: [bool; 2],
    /// The transport owner's confirmed playback, as the renderer runs it.
    playing: [bool; 2],
    pitch: [i16; 2],
    nudge: [TempoNudge; 2],
    /// The ring epoch when a seek was asked for; position reports wait for
    /// a newer one, so a stale position never overwrites the target.
    seeking: [Option<u32>; 2],
    seen: [u32; 2],
    unsupported: u32,
    refused_loads: u32,
}

impl Engine {
    pub(crate) const fn new() -> Self {
        Self {
            deck: DeckProductState::new(),
            connected: false,
            loaded: [false; 2],
            playing: [false; 2],
            pitch: [0; 2],
            nudge: [TempoNudge::new(), TempoNudge::new()],
            seeking: [None; 2],
            seen: [0; 2],
            unsupported: 0,
            refused_loads: 0,
        }
    }

    /// One controller event through the deck engine.
    pub(crate) fn control(&mut self, event: &ControlEvent) {
        if !self.connected {
            self.connected = true;
            for effects in self.deck.set_controller_connected(true) {
                self.apply(effects);
            }
        }
        let effects = self.deck.handle_control(*event);
        self.apply(effects);
    }

    /// The product's load admission for `deck` (refused while it plays).
    pub(crate) fn load_allowed(&mut self, deck: DeckId) -> bool {
        let allowed = self.deck.load_admission(deck).is_ok();
        self.refused_loads = self.refused_loads.wrapping_add(u32::from(!allowed));
        allowed
    }

    fn apply(&mut self, effects: DeckEffects) {
        for effect in effects.items.into_iter().flatten() {
            match effect {
                DeckEffect::PlaybackRequest {
                    deck,
                    playing,
                    request_id,
                } => {
                    let i = index(deck);
                    let accepted = !playing || self.loaded[i];
                    if accepted {
                        self.playing[i] = playing;
                    }
                    self.deck
                        .resolve_playback_request(deck, request_id, accepted);
                }
                DeckEffect::Pause { deck } => self.playing[index(deck)] = false,
                DeckEffect::Seek { deck, position_ms } => {
                    let i = index(deck);
                    if self.loaded[i] {
                        COMMANDS[i].seek(position_ms);
                        self.seeking[i] = Some(RINGS[i].epoch());
                    }
                }
                DeckEffect::SetPitchCentipercent { deck, value } => {
                    self.pitch[index(deck)] = value;
                }
                DeckEffect::JogNudge { deck, ticks } => self.nudge[index(deck)].add(ticks),
                _ => self.unsupported = self.unsupported.wrapping_add(1),
            }
        }
    }

    /// Commits loads the producers published; returns each deck's playback
    /// and pitch (with the jog bend) for the next render block.
    pub(crate) fn transport(&mut self) -> ([bool; 2], [i16; 2]) {
        for deck in DECKS {
            let i = index(deck);
            let Some(track) = LOADED[i].take(&mut self.seen[i]) else {
                continue;
            };
            let id = MediaTrackId(track.track_id);
            let committed = track.ready
                && self
                    .deck
                    .commit_track_load(deck, id, track.bpm_x100, HotCueBank::empty(id), None)
                    .is_ok();
            if !committed {
                self.deck.unload_track(deck);
            }
            self.loaded[i] = committed;
            self.playing[i] = false;
            self.seeking[i] = None;
            self.nudge[i].clear();
        }
        let pitch = core::array::from_fn(|i| self.nudge[i].apply_centipercent(self.pitch[i]));
        (self.playing, pitch)
    }

    /// After a render block of `frames`: reports positions, decays the jog
    /// bend and stops a deck whose track has played out.
    pub(crate) fn progress(&mut self, readers: &[Reader; 2], frames: u32) {
        for deck in DECKS {
            let i = index(deck);
            let reader = &readers[i];
            self.nudge[i].advance(frames, RENDER_SAMPLE_RATE_HZ);
            if self.seeking[i].is_some_and(|epoch| reader.epoch() != epoch) {
                self.seeking[i] = None;
            }
            let position_ms = reader.position_ms();
            if self.loaded[i] && self.seeking[i].is_none() && reader.rate() != 0 {
                self.deck.set_position_ms(deck, position_ms);
                if self.playing[i] && reader.ended(&RINGS[i]) && reader.buffered(&RINGS[i]) == 0 {
                    self.deck.finish_playback(deck, position_ms);
                    self.playing[i] = false;
                }
            }
            let flags = u32::from(self.loaded[i]) | u32::from(self.playing[i]) << 1;
            STATUS[i][0].store(self.deck.deck(deck).position_ms, Ordering::Relaxed);
            STATUS[i][1].store(flags, Ordering::Relaxed);
        }
    }

    /// The confirmed playback, for the Play LEDs.
    pub(crate) const fn playing(&self) -> [bool; 2] {
        self.playing
    }

    /// The engine's own view of a deck, for tests and telemetry.
    #[cfg(test)]
    pub(crate) fn deck(&self, deck: DeckId) -> &ruzina_deck::DeckState {
        self.deck.deck(deck)
    }

    /// Unexecuted effects and refused loads.
    pub(crate) const fn counters(&self) -> [u32; 2] {
        [self.unsupported, self.refused_loads]
    }
}

/// Writes deck 1 and 2 position (ms) and flags (bit 0 loaded, bit 1
/// playing) to `out[0..4]`. Any task: the values are published atomically.
///
/// # Safety
/// `out` must be valid for four `u32` writes.
#[cfg(feature = "idf-deck-engine")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_engine_status(out: *mut u32) {
    if out.is_null() {
        return;
    }
    let values = [
        STATUS[0][0].load(Ordering::Relaxed),
        STATUS[0][1].load(Ordering::Relaxed),
        STATUS[1][0].load(Ordering::Relaxed),
        STATUS[1][1].load(Ordering::Relaxed),
    ];
    // SAFETY: the caller provides four writable u32 slots.
    unsafe { core::ptr::copy_nonoverlapping(values.as_ptr(), out, values.len()) };
}

#[cfg(test)]
mod tests;
