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

/// The Beat Loop pad whose length is 4 beats.
const FOUR_BEAT_LOOP_PAD: u8 = 7;

/// A loop change for a deck's reader, applied at the next render block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LoopCommand {
    /// The loop `[start_ms, end_ms)`.
    Set(u32, u32),
    Clear,
}

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

/// After a seek, a playing deck resumes only once its new epoch holds this
/// many source frames (about 93 ms at 44.1 kHz), or its track has ended. A
/// seek's first block alone is a few milliseconds, and resuming on it
/// underran the next render blocks on the device.
const SEEK_CUSHION_FRAMES: u32 = 4096;
/// The longest a cushion holds a deck, in 128-frame render blocks (1 s).
const SEEK_CUSHION_MAX_BLOCKS: u16 = 375;

/// Tracks whose new Hot Cues can wait for flash at once.
const HOT_CUE_SAVES_QUEUED: usize = 16;

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
    /// The epoch a completed seek waits to buffer before playing resumes,
    /// and the render blocks it has waited.
    cushion: [Option<(u32, u16)>; 2],
    source: [Option<LoadRequest>; 2],
    next_seek: u32,
    revision: [u32; 2],
    lease: Option<MediaLease>,
    pending_load: [Option<LoadPermit>; 2],
    unsupported: u32,
    refused_loads: u32,
    /// Loop changes waiting for the next render block.
    loop_command: [Option<LoopCommand>; 2],
    /// The deck's active loop, in milliseconds.
    loop_ms: [Option<(u32, u32)>; 2],
    /// Jog ticks waiting for the next render block.
    scratch_ticks: [i32; 2],
    /// Banks the flash journal has not taken yet, one per track, oldest first.
    /// Flash waits until no deck plays, which a long mix can put off.
    hot_cue_saves: [Option<HotCueBank>; HOT_CUE_SAVES_QUEUED],
    #[cfg(feature = "soak-autoplay")]
    auto: crate::soak_autoplay::AutoPlay,
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
            cushion: [None; 2],
            source: [None; 2],
            next_seek: 0,
            revision: [0; 2],
            lease: None,
            pending_load: [None; 2],
            unsupported: 0,
            refused_loads: 0,
            loop_command: [None; 2],
            loop_ms: [None; 2],
            scratch_ticks: [0; 2],
            hot_cue_saves: [None; HOT_CUE_SAVES_QUEUED],
            #[cfg(feature = "soak-autoplay")]
            auto: crate::soak_autoplay::AutoPlay::new(),
        }
    }

    /// The soak DJ's turn, once per USB client loop turn after `tick`: loads
    /// the next track on a deck whose track played out and presses Play.
    #[cfg(feature = "soak-autoplay")]
    pub(crate) fn autoplay(&mut self) {
        use crate::soak_autoplay::{Action, View};
        let tracks = crate::soak_autoplay::catalog();
        for deck in DECKS {
            let i = index(deck);
            let view = View {
                media: self.lease.is_some_and(crate::media_signal::current),
                loaded: self.loaded[i] && !self.faulted[i],
                playing: self.playing[i],
                pending_load: self.pending_load[i].is_some(),
            };
            match self.auto.step(i, view, tracks) {
                Action::Load(track) => {
                    if !self.request_load(deck, track) {
                        self.auto.load_refused(i);
                    }
                }
                Action::Play => self.control(&ControlEvent {
                    deck: Some(deck),
                    control: ruzina_controller_core::SemanticControl::Play,
                    value: ruzina_controller_core::ControlValue::Pressed(true),
                }),
                Action::None => {}
            }
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
        let effects = self.deck.handle_control(self.flx4_loop_button(event));
        self.apply(effects);
    }

    /// The FLX4's loop buttons when no loop runs; with one running, both go
    /// to the deck state unchanged (exit, and stop and forget).
    ///
    /// 4 BEAT/EXIT arrives as Reloop/Exit and starts the 4-beat loop the button
    /// is labelled with, through the Beat Loop pad of that length (beat grid,
    /// or the BPM, or the deck state's 120 BPM). Reloop Stop, the Shift layer,
    /// then brings back the last loop, as Reloop/Exit does in the deck state.
    fn flx4_loop_button(&self, event: &ControlEvent) -> ControlEvent {
        use ruzina_controller_core::{
            ControlValue, DeckExtAction, DeckExtActionValue, PadAction, PadMode, SemanticControl,
        };
        let Some(deck) = event.deck else {
            return *event;
        };
        let loop_state = self.deck.deck(deck).loop_state;
        if loop_state.active.is_some() {
            return *event;
        }
        match (event.control, event.value) {
            (SemanticControl::ReloopExit, ControlValue::Pressed(true)) => ControlEvent {
                deck: event.deck,
                control: SemanticControl::PadAction,
                value: ControlValue::PadAction(PadAction {
                    pad: FOUR_BEAT_LOOP_PAD,
                    mode: PadMode::BeatLoop,
                    shifted: false,
                    pressed: true,
                }),
            },
            (
                SemanticControl::DeckExtAction,
                ControlValue::DeckExtAction(DeckExtActionValue {
                    action: DeckExtAction::ReloopStop,
                    pressed: true,
                }),
            ) if loop_state.last.is_some() => ControlEvent {
                deck: event.deck,
                control: SemanticControl::ReloopExit,
                value: ControlValue::Pressed(true),
            },
            _ => *event,
        }
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
                DeckEffect::Seek { deck, position_ms } => self.seek(index(deck), position_ms),
                DeckEffect::SetLoop {
                    deck,
                    start_ms,
                    end_ms,
                } => {
                    let i = index(deck);
                    self.loop_command[i] = Some(LoopCommand::Set(start_ms, end_ms));
                    self.loop_ms[i] = Some((start_ms, end_ms));
                }
                DeckEffect::ClearLoop { deck } => {
                    let i = index(deck);
                    self.loop_command[i] = Some(LoopCommand::Clear);
                    self.loop_ms[i] = None;
                }
                DeckEffect::SetPitchCentipercent { deck, value } => {
                    self.pitch[index(deck)] = value;
                }
                DeckEffect::JogNudge { deck, ticks } => self.nudge[index(deck)].add(ticks),
                // The render graph reads the FX from the deck state per block.
                DeckEffect::ApplyBeatFx { .. } | DeckEffect::ApplyPadFx { .. } => {}
                DeckEffect::PersistHotCues { bank, .. } => self.queue_hot_cues(bank),
                DeckEffect::ScratchJog { deck, ticks, epoch } => {
                    if epoch == self.deck.deck(deck).scratch_epoch {
                        let i = index(deck);
                        self.scratch_ticks[i] =
                            self.scratch_ticks[i].saturating_add(i32::from(ticks));
                    }
                }
            }
        }
    }

    /// Asks deck `i`'s producer to reopen its source at `position_ms`.
    fn seek(&mut self, i: usize, position_ms: u32) {
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
            self.cushion[i] = None;
            self.revision[i] = self.revision[i].wrapping_add(1);
            self.publish_seek(i);
        }
    }

    /// Deck `i`'s jog ticks for the next render block while its loaded track's
    /// jog is touched, `None` when no gesture may run.
    pub(crate) fn take_scratch(&mut self, i: usize) -> Option<i16> {
        let touched = self.deck.deck(DECKS[i]).jog_touched
            && self.loaded[i]
            && !self.faulted[i]
            && self.seeking[i].is_none();
        if !touched {
            self.scratch_ticks[i] = 0;
            return None;
        }
        let ticks = self.scratch_ticks[i].clamp(i32::from(i16::MIN), i32::from(i16::MAX));
        self.scratch_ticks[i] -= ticks;
        Some(ticks as i16)
    }

    /// The loop change for deck `i`'s reader, once.
    pub(crate) fn take_loop_command(&mut self, i: usize) -> Option<LoopCommand> {
        self.loop_command[i].take()
    }

    /// Keeps `bank` for the flash journal: it replaces the track's waiting
    /// bank, or joins the queue, dropping the oldest when the queue is full.
    fn queue_hot_cues(&mut self, bank: HotCueBank) {
        let queue = &mut self.hot_cue_saves;
        if let Some(waiting) = queue
            .iter_mut()
            .flatten()
            .find(|waiting| waiting.track_id() == bank.track_id())
        {
            *waiting = bank;
            return;
        }
        if queue[HOT_CUE_SAVES_QUEUED - 1].is_some() {
            queue.copy_within(1.., 0);
            queue[HOT_CUE_SAVES_QUEUED - 1] = None;
        }
        if let Some(free) = queue.iter_mut().find(|slot| slot.is_none()) {
            *free = Some(bank);
        }
    }

    /// Offers the oldest waiting bank to the SD owner, outside the audio fill.
    pub(crate) fn flush_hot_cues(&mut self) {
        let Some(bank) = self.hot_cue_saves[0] else {
            return;
        };
        if crate::deck_transport::HOT_CUE_SAVES.put(bank).is_ok() {
            self.hot_cue_saves.copy_within(1.., 0);
            self.hot_cue_saves[HOT_CUE_SAVES_QUEUED - 1] = None;
        }
    }

    /// Each deck's Hot Cue pads that hold a cue, as bits.
    pub(crate) fn hot_cue_masks(&self) -> [u8; 2] {
        DECKS.map(|deck| {
            self.deck
                .hot_cues(deck)
                .map_or(0, |bank| bank.exists_mask())
        })
    }

    /// Each deck's Beat FX and Pad FX for the next render block.
    pub(crate) fn fx_controls(&self) -> [ruzina_audio_runtime::fx::DeckFxControl; 2] {
        ruzina_audio_runtime::fx::product_fx_controls(&self.deck)
    }

    /// The Beat FX On LED.
    pub(crate) fn beat_fx_on(&self) -> bool {
        self.deck.beat_fx().enabled
    }

    /// Each deck's Loop In and Loop Out LEDs: In from the press that marks
    /// the start, Out once the loop runs.
    pub(crate) fn loop_leds(&self) -> [(bool, bool); 2] {
        DECKS.map(|deck| {
            let active = self.loop_ms[index(deck)].is_some();
            let marked = self.deck.deck(deck).loop_state.pending_in_ms.is_some();
            (active || marked, active)
        })
    }

    /// Deck `i`'s active loop start, in milliseconds.
    pub(crate) fn loop_start_ms(&self, i: usize) -> Option<u32> {
        self.loop_ms[i].map(|(start, _)| start)
    }

    /// The reader reached the loop end without the loop start in its
    /// history: one seek there, after which the loop replays seamlessly.
    pub(crate) fn seek_loop_start(&mut self, i: usize, start_ms: u32) {
        if self.seeking[i].is_none_or(|request| request.target_ms != start_ms) {
            self.seek(i, start_ms);
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
                self.cushion[i] = None;
                self.nudge[i].clear();
                self.loop_command[i] = Some(LoopCommand::Clear);
                self.loop_ms[i] = None;
                self.deck.unload_track(deck);
                self.revision[i] = self.revision[i].wrapping_add(1);
                TRACK_INDEX[i].store(u32::MAX, Ordering::Release);
                STATUS[i][0].store(0, Ordering::Relaxed);
                STATUS[i][1].store(0, Ordering::Relaxed);
            }
            self.lease = None;
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
            let source = if result.source_cues.track_id() == id {
                result.source_cues
            } else {
                HotCueBank::empty(id)
            };
            // A bank still waiting for flash is newer than what flash holds.
            let persisted = self
                .hot_cue_saves
                .iter()
                .flatten()
                .find(|waiting| waiting.track_id() == id)
                .copied()
                .or(result.persisted_cues.filter(|bank| bank.track_id() == id));
            let committed = result.ready
                && self
                    .deck
                    .commit_track_load(deck, id, result.bpm_x100, source, persisted)
                    .is_ok();
            if !committed {
                self.deck.unload_track(deck);
            }
            self.loaded[i] = committed;
            self.faulted[i] = false;
            self.source[i] = committed.then_some(result.request);
            self.revision[i] = self.revision[i].wrapping_add(1);
            self.playing[i] = false;
            self.seeking[i] = None;
            self.unsent_seek[i] = None;
            self.seek_epoch[i] = None;
            self.loop_command[i] = Some(LoopCommand::Clear);
            self.loop_ms[i] = None;
            // A new track starts like a seek: its producer published Ready
            // after opening this epoch with one block, so a Play right after
            // the load waits for the same buffered cushion. A later Play
            // finds the ring already full and does not wait.
            self.cushion[i] = committed.then(|| (RINGS[i].epoch(), 0));
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
                self.cushion[i] = Some((result.epoch, 0));
                self.deck.set_position_ms(DECKS[i], result.position_ms);
                if result.outcome != SeekOutcome::Ready {
                    self.playing[i] = false;
                    self.deck.finish_playback(DECKS[i], result.position_ms);
                    self.seek_epoch[i] = None;
                    self.cushion[i] = None;
                }
                self.revision[i] = self.revision[i].wrapping_add(1);
            }
        }
        let pitch = core::array::from_fn(|i| self.nudge[i].apply_centipercent(self.pitch[i]));
        crate::media_signal::playback(self.playing, pitch);
        (
            core::array::from_fn(|i| {
                self.playing[i] && self.seeking[i].is_none() && self.cushion[i].is_none()
            }),
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
    pub(crate) fn revision(&self) -> [u32; 2] {
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
            if let Some((epoch, waited)) = self.cushion[i] {
                let ready = reader.epoch() == epoch
                    && (reader.buffered(&RINGS[i]) >= SEEK_CUSHION_FRAMES
                        || reader.ended(&RINGS[i]));
                self.cushion[i] =
                    (!ready && waited < SEEK_CUSHION_MAX_BLOCKS).then_some((epoch, waited + 1));
            }
            let position_ms = reader.position_ms();
            if self.loaded[i]
                && self.seeking[i].is_none()
                && self.seek_epoch[i].is_none()
                && reader.rate() != 0
            {
                self.deck.set_position_ms(deck, position_ms);
                if self.playing[i]
                    && !reader.looping()
                    && reader.ended(&RINGS[i])
                    && reader.buffered(&RINGS[i]) == 0
                {
                    self.deck.finish_playback(deck, position_ms);
                    self.playing[i] = false;
                    #[cfg(feature = "soak-autoplay")]
                    self.auto.track_ended(i);
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
