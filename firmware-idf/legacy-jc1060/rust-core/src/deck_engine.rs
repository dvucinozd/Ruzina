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
use crate::deck_play::{RINGS, Reader};
use crate::deck_transport::{SEEK_REQUESTS, SEEK_RESULTS, SeekOutcome, SeekRequest};
use core::sync::atomic::{AtomicU32, Ordering};
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

use crate::deck_transport::{GATES, LOAD_REQUESTS, LOAD_RESULTS, LoadRequest, MEDIA};
use ruzina_deck::{DeckClaim, LoadPermit};
use ruzina_media_session::MediaLease;

static TRACK_INDEX: [AtomicU32; 2] = [AtomicU32::new(u32::MAX), AtomicU32::new(u32::MAX)];

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
    faulted: [bool; 2],
    /// The transport owner's confirmed playback, as the renderer runs it.
    playing: [bool; 2],
    pitch: [i16; 2],
    nudge: [TempoNudge; 2],
    /// The ring epoch when a seek was asked for; position reports wait for
    /// a newer one, so a stale position never overwrites the target.
    seeking: [Option<SeekRequest>; 2],
    unsent_seek: [Option<SeekRequest>; 2],
    seek_epoch: [Option<u32>; 2],
    source: [Option<LoadRequest>; 2],
    next_seek: u32,
    revision: u32,
    lease: Option<MediaLease>,
    pending_load: [Option<LoadPermit>; 2],
    unsupported: u32,
    refused_loads: u32,
}

impl Engine {
    pub(crate) const fn new() -> Self {
        Self {
            deck: DeckProductState::new(),
            connected: false,
            loaded: [false; 2],
            faulted: [false; 2],
            playing: [false; 2],
            pitch: [0; 2],
            nudge: [TempoNudge::new(), TempoNudge::new()],
            seeking: [None; 2],
            unsent_seek: [None; 2],
            seek_epoch: [None; 2],
            source: [None; 2],
            next_seek: 0,
            revision: 0,
            lease: None,
            pending_load: [None; 2],
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
        if let Some(deck) = event.deck
            && matches!(
                event.control,
                ruzina_controller_core::SemanticControl::JogTouch
            )
            && event.value == ruzina_controller_core::ControlValue::Pressed(true)
        {
            match GATES[index(deck)].claim_for_transport() {
                DeckClaim::Replacing => return,
                DeckClaim::CancelledLoad => self.pending_load[index(deck)] = None,
                DeckClaim::Free => {}
            }
        }
        let effects = self.deck.handle_control(*event);
        self.apply(effects);
    }

    /// Admission and reservation run on the USB control task. Producer must
    /// begin replacement with this exact permit before closing the old file.
    pub(crate) fn request_load(&mut self, deck: DeckId, catalog_index: u32) -> bool {
        self.transport();
        let i = index(deck);
        if self.deck.load_admission(deck).is_err() || self.pending_load[i].is_some() {
            self.refused_loads = self.refused_loads.wrapping_add(1);
            return false;
        }
        let Some(lease) = self.lease.filter(|l| crate::media_signal::current(*l)) else {
            return false;
        };
        let Ok(permit) = GATES[i].reserve() else {
            return false;
        };
        let request = LoadRequest {
            lease,
            permit,
            index: catalog_index,
        };
        if LOAD_REQUESTS[i].put(request).is_err() {
            GATES[i].finish(permit);
            return false;
        }
        self.pending_load[i] = Some(permit);
        true
    }

    #[cfg(test)]
    pub(crate) fn load_allowed(&mut self, deck: DeckId) -> bool {
        let allowed = self.deck.load_admission(deck).is_ok();
        self.refused_loads += u32::from(!allowed);
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
                    let claim = if playing {
                        GATES[i].claim_for_transport()
                    } else {
                        DeckClaim::Free
                    };
                    if claim == DeckClaim::CancelledLoad {
                        self.pending_load[i] = None;
                    }
                    let accepted = !playing
                        || self.loaded[i] && !self.faulted[i] && claim != DeckClaim::Replacing;
                    if accepted {
                        self.playing[i] = playing;
                    }
                    self.deck
                        .resolve_playback_request(deck, request_id, accepted);
                }
                DeckEffect::Pause { deck } => self.playing[index(deck)] = false,
                DeckEffect::Seek { deck, position_ms } => {
                    let i = index(deck);
                    if let Some(source) = self.source[i] {
                        self.next_seek = self.next_seek.wrapping_add(1).max(1);
                        let request = SeekRequest {
                            source,
                            id: self.next_seek,
                            target_ms: position_ms,
                        };
                        self.seeking[i] = Some(request);
                        self.unsent_seek[i] = Some(request);
                        self.seek_epoch[i] = None;
                        self.revision = self.revision.wrapping_add(1);
                        self.publish_seek(i);
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
        if let Some(lease) = MEDIA.take() {
            self.lease = Some(lease);
        }
        if self.lease.is_some_and(|l| !crate::media_signal::current(l)) {
            for (i, deck) in DECKS.into_iter().enumerate() {
                if let Some(permit) = self.pending_load[i].take() {
                    GATES[i].finish(permit);
                }
                self.source[i] = None;
                self.loaded[i] = false;
                self.faulted[i] = false;
                self.playing[i] = false;
                self.seeking[i] = None;
                self.unsent_seek[i] = None;
                self.seek_epoch[i] = None;
                self.nudge[i].clear();
                self.deck.unload_track(deck);
                TRACK_INDEX[i].store(u32::MAX, Ordering::Release);
                STATUS[i][0].store(0, Ordering::Relaxed);
                STATUS[i][1].store(0, Ordering::Relaxed);
            }
            self.lease = None;
            self.revision = self.revision.wrapping_add(1);
        }
        for deck in DECKS {
            let i = index(deck);
            let Some(result) = LOAD_RESULTS[i].take() else {
                continue;
            };
            if self.pending_load[i] != Some(result.request.permit)
                || self.lease != Some(result.request.lease)
            {
                GATES[i].finish(result.request.permit);
                continue;
            }
            let id = MediaTrackId(result.track_id);
            let committed = result.ready
                && self
                    .deck
                    .commit_track_load(deck, id, result.bpm_x100, HotCueBank::empty(id), None)
                    .is_ok();
            if !committed {
                self.deck.unload_track(deck);
            }
            self.loaded[i] = committed;
            self.faulted[i] = false;
            self.source[i] = committed.then_some(result.request);
            self.revision = self.revision.wrapping_add(1);
            self.playing[i] = false;
            self.seeking[i] = None;
            self.unsent_seek[i] = None;
            self.seek_epoch[i] = None;
            self.nudge[i].clear();
            TRACK_INDEX[i].store(
                if committed {
                    result.request.index
                } else {
                    u32::MAX
                },
                Ordering::Release,
            );
            self.pending_load[i] = None;
            GATES[i].finish(result.request.permit);
        }
        for i in 0..2 {
            self.publish_seek(i);
            if let Some(result) = SEEK_RESULTS[i].take()
                && self.seeking[i] == Some(result.request)
                && self.source[i] == Some(result.request.source)
            {
                self.seeking[i] = None;
                self.faulted[i] =
                    matches!(result.outcome, SeekOutcome::Failed | SeekOutcome::Cancelled);
                self.seek_epoch[i] = Some(result.epoch);
                self.deck.set_position_ms(DECKS[i], result.position_ms);
                if result.outcome != SeekOutcome::Ready {
                    self.playing[i] = false;
                    self.deck.finish_playback(DECKS[i], result.position_ms);
                    self.seek_epoch[i] = None;
                }
                self.revision = self.revision.wrapping_add(1);
            }
        }
        let pitch = core::array::from_fn(|i| self.nudge[i].apply_centipercent(self.pitch[i]));
        crate::media_signal::playback(self.playing, pitch);
        (
            core::array::from_fn(|i| self.playing[i] && self.seeking[i].is_none()),
            pitch,
        )
    }

    pub(crate) fn connection(&mut self, connected: bool) {
        self.connected = connected;
        for effects in self.deck.set_controller_connected(connected) {
            self.apply(effects);
        }
    }

    fn publish_seek(&mut self, i: usize) {
        if let Some(request) = self.unsent_seek[i]
            && SEEK_REQUESTS[i].put(request).is_ok()
        {
            self.unsent_seek[i] = None;
        }
    }
    pub(crate) fn revision(&self) -> u32 {
        self.revision
    }

    /// After a render block of `frames`: reports positions, decays the jog
    /// bend and stops a deck whose track has played out.
    pub(crate) fn progress(&mut self, readers: &[Reader; 2], frames: u32) {
        for deck in DECKS {
            let i = index(deck);
            let reader = &readers[i];
            self.nudge[i].advance(frames, RENDER_SAMPLE_RATE_HZ);
            if self.seek_epoch[i] == Some(reader.epoch()) {
                self.seek_epoch[i] = None;
            }
            let position_ms = reader.position_ms();
            if self.loaded[i]
                && self.seeking[i].is_none()
                && self.seek_epoch[i].is_none()
                && reader.rate() != 0
            {
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

/// Confirmed catalog index, or -1. Snapshot only; safe on the display task.
#[cfg(feature = "idf-deck-engine")]
#[unsafe(no_mangle)]
pub extern "C" fn ruzina_legacy_engine_track(deck: u32) -> i32 {
    if !(1..=2).contains(&deck) {
        return -1;
    }
    TRACK_INDEX[deck as usize - 1].load(Ordering::Acquire) as i32
}
