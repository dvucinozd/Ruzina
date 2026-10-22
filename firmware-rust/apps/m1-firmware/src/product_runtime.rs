use crate::transport_owner_state::{TransportCompletion, TransportOutcome};
use pajoniiir_audio_runtime::decode::SourceDescriptor;
#[cfg(feature = "library-deck-load")]
use pajoniiir_controller_core::{ControlValue, SemanticControl};
use pajoniiir_controller_session::{
    BeatFxTarget, ConnectionState, ControlDomain, ControllerLedFrame, DeckId, LedState, LedUpdate,
    PadMode, SemanticLedId, SessionDelivery, SessionDeliverySink,
};
use pajoniiir_deck::{
    DeckClaim, DeckEffect, DeckEffectBroker, DeckLoadGate, DeckProductState, JogMode, LoadPermit,
    LoopAdjustMode, PadFxBank,
};
use pajoniiir_hot_cues::HotCueBank;
use pajoniiir_media_catalog::CatalogLoadCandidate;
use pajoniiir_media_session::MediaLease;
use pajoniiir_mixer::MixerState;
use pajoniiir_track_analysis::TrackAnalysis;
#[cfg(feature = "library-deck-load")]
use pajoniiir_track_analysis::owned::{AnalysisIdentity, AnalysisPins, AnalysisPublication};

#[cfg(feature = "library-deck-load")]
#[derive(Debug)]
pub(crate) struct ProductAnalysisLoad {
    pub(crate) generation: u32,
    pub(crate) publication: Option<AnalysisPublication>,
}

#[cfg(feature = "library-deck-load")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BackgroundAnalysisRefusal {
    Deferred,
    Rejected,
}

const PRODUCT_LIBRARY_EFFECT_CAPACITY: usize = 8;
const PRODUCT_TRANSPORT_EFFECT_CAPACITY: usize = 16;
const PRODUCT_PERSISTENCE_EFFECT_CAPACITY: usize = 4;
const PRODUCT_AUDIO_DSP_EFFECT_CAPACITY: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LibraryEffect {
    Browse {
        delta: i16,
        multiplier: u16,
    },
    OpenLibrary,
    /// PLAYLISTS/BACK: track list -> playlist tree -> parent level (RP-06).
    Back,
    /// LOAD on a folder/playlist row opens it without load admission.
    Open,
    Load {
        deck: DeckId,
        permit: LoadPermit,
    },
}

/// Mirror of the Library worker's cursor: while it rests on a folder or
/// playlist row, LOAD opens that row (M2.5) instead of claiming a deck, so a
/// playing deck's LOAD LOCK does not block browsing. A stale mirror is safe:
/// the worker resolves the actual row, and an `Open` never loads a track.
static LIBRARY_CURSOR_ON_NODE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

pub(crate) fn set_library_cursor_on_node(on_node: bool) {
    LIBRARY_CURSOR_ON_NODE.store(on_node, core::sync::atomic::Ordering::Release);
}

fn library_cursor_on_node() -> bool {
    LIBRARY_CURSOR_ON_NODE.load(core::sync::atomic::Ordering::Acquire)
}

/// RP-04 single-flight load gates shared by the product control owner and the
/// asynchronous Library deck worker (see `pajoniiir_deck::DeckLoadGate`).
static LOAD_GATES: [DeckLoadGate; 2] = [DeckLoadGate::new(), DeckLoadGate::new()];

fn decoded_duration_ms(source: SourceDescriptor) -> Option<u32> {
    let rate = u64::from(source.sample_rate_hz());
    let frames = source.frames()?;
    (rate != 0).then(|| u32::try_from(frames.saturating_mul(1_000) / rate).unwrap_or(u32::MAX))
}

/// Worker-side access to the gates that `ProductRuntime::new` binds.
pub(crate) fn load_gate(deck: DeckId) -> &'static DeckLoadGate {
    &LOAD_GATES[deck_index(deck)]
}

/// Controls that can take a deck's audio ownership and therefore race a load:
/// PLAY, CUE (hold-preview) and VINYL platter touch (scratch). CDJ ignores
/// platter-top touch, so it never claims the deck.
fn claims_transport(
    event: pajoniiir_controller_core::ControlEvent,
    deck_state: &DeckProductState,
) -> Option<DeckId> {
    use pajoniiir_controller_core::{ControlValue, SemanticControl};
    let deck = event.deck?;
    match (event.control, event.value) {
        (SemanticControl::Play | SemanticControl::Cue, ControlValue::Pressed(true)) => Some(deck),
        (SemanticControl::JogTouch, ControlValue::Pressed(true))
            if deck_state.deck(deck).jog_mode == JogMode::Vinyl =>
        {
            Some(deck)
        }
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct LibraryEffectQueue<const N: usize> {
    items: [Option<LibraryEffect>; N],
    head: usize,
    tail: usize,
    count: usize,
}

impl<const N: usize> LibraryEffectQueue<N> {
    const fn new() -> Self {
        Self {
            items: [None; N],
            head: 0,
            tail: 0,
            count: 0,
        }
    }

    const fn len(&self) -> usize {
        self.count
    }

    fn front(&self) -> Option<LibraryEffect> {
        if self.count == 0 || N == 0 {
            None
        } else {
            self.items[self.head]
        }
    }

    fn try_push(&mut self, effect: LibraryEffect) -> bool {
        if self.count == N || N == 0 {
            return false;
        }
        self.items[self.tail] = Some(effect);
        self.tail = (self.tail + 1) % N;
        self.count += 1;
        true
    }

    fn complete_front(&mut self) -> bool {
        if self.count == 0 || N == 0 {
            return false;
        }
        self.items[self.head] = None;
        self.head = (self.head + 1) % N;
        self.count -= 1;
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProductLoadedTrack {
    pub(crate) candidate: CatalogLoadCandidate,
    pub(crate) source_cues_available: bool,
    pub(crate) audio_source: Option<SourceDescriptor>,
    #[cfg(feature = "library-deck-load")]
    pub(crate) analysis_identity: Option<AnalysisIdentity>,
}

use pajoniiir_service::handoff::{ProductCounters as ProductRuntimeStats, ProductObservation};

pub(crate) struct ProductRuntime {
    #[cfg(feature = "apta-usb0-bringup")]
    pub(crate) analysis_driver: crate::analysis_product::Driver<'static>,
    deck: DeckProductState,
    mixer: MixerState,
    library_effects: LibraryEffectQueue<PRODUCT_LIBRARY_EFFECT_CAPACITY>,
    loaded_tracks: [Option<ProductLoadedTrack>; 2],
    source_epochs: [u32; 2],
    persistence_faulted: bool,
    #[cfg(feature = "library-deck-load")]
    analyses: AnalysisPins,
    deck_effects: DeckEffectBroker<
        PRODUCT_TRANSPORT_EFFECT_CAPACITY,
        PRODUCT_PERSISTENCE_EFFECT_CAPACITY,
        PRODUCT_AUDIO_DSP_EFFECT_CAPACITY,
    >,
    leds: ControllerLedFrame,
    stats: ProductRuntimeStats,
    load_gates: &'static [DeckLoadGate; 2],
}

impl ProductRuntime {
    pub(crate) const fn new() -> Self {
        Self::with_load_gates(&LOAD_GATES)
    }

    /// Host fault-injection rigs bind private gates; firmware uses `new`.
    pub(crate) const fn with_load_gates(load_gates: &'static [DeckLoadGate; 2]) -> Self {
        Self {
            load_gates,
            #[cfg(feature = "apta-usb0-bringup")]
            analysis_driver: crate::analysis_product::Driver::new(),
            deck: DeckProductState::new(),
            mixer: MixerState::new(),
            library_effects: LibraryEffectQueue::new(),
            loaded_tracks: [None, None],
            source_epochs: [1; 2],
            persistence_faulted: false,
            #[cfg(feature = "library-deck-load")]
            analyses: AnalysisPins::new(),
            deck_effects: DeckEffectBroker::new(),
            leds: ControllerLedFrame::new(),
            stats: ProductRuntimeStats {
                accepted_connections: 0,
                accepted_deck_controls: 0,
                accepted_mixer_controls: 0,
                accepted_library_controls: 0,
                library_load_locked: 0,
                library_load_superseded: 0,
                transport_refused_loading: 0,
                library_failures: 0,
                effect_backpressure: 0,
                deferred_controls: 0,
            },
        }
    }

    pub(crate) const fn pending_library_effects(&self) -> usize {
        self.library_effects.len()
    }

    pub(crate) fn next_library_effect(&self) -> Option<LibraryEffect> {
        self.library_effects.front()
    }

    pub(crate) fn complete_library_effect(&mut self) -> bool {
        self.library_effects.complete_front()
    }

    pub(crate) fn note_library_failure(&mut self) {
        self.stats.library_failures = self.stats.library_failures.saturating_add(1);
    }

    pub(crate) fn fail_library_load(&mut self, source_revoked: Option<DeckId>, superseded: bool) {
        if let Some(deck) = source_revoked {
            self.invalidate_deck_source(deck);
        }
        if superseded {
            self.stats.library_load_superseded =
                self.stats.library_load_superseded.saturating_add(1);
        }
        self.note_library_failure();
    }

    pub(crate) fn note_persistence_fault(&mut self) {
        self.persistence_faulted = true;
    }

    pub(crate) const fn persistence_faulted(&self) -> bool {
        self.persistence_faulted
    }

    pub(crate) fn commit_library_load(
        &mut self,
        deck: DeckId,
        candidate: CatalogLoadCandidate,
        source: HotCueBank,
        persisted: Option<HotCueBank>,
        source_cues_available: bool,
        audio_source: Option<SourceDescriptor>,
        #[cfg(feature = "library-deck-load")] analysis: ProductAnalysisLoad,
    ) -> bool {
        if candidate.track.track_id != source.track_id()
            || persisted.is_some_and(|bank| bank.track_id() != candidate.track.track_id)
        {
            return false;
        }
        // A persisted read may have completed before a newly accepted edit was
        // flushed. Retain the live local bank even if that write just completed.
        let mut persisted = persisted;
        for other in [DeckId::One, DeckId::Two] {
            if self.loaded_track(other).is_some_and(|loaded| {
                loaded.candidate.track.track_id == candidate.track.track_id
                    && loaded.candidate.publication.lease == candidate.publication.lease
            }) {
                persisted = self.deck.local_hot_cues(other).or(persisted);
            }
        }
        let persisted = self
            .deck_effects
            .pending_hot_cues(candidate.track.track_id)
            .or(persisted);
        #[cfg(feature = "library-deck-load")]
        let analysis_identity = AnalysisIdentity {
            track: candidate.track.track_id,
            lease: candidate.publication.lease,
            catalog_revision: candidate.publication.revision,
            load_generation: analysis.generation,
        };
        #[cfg(feature = "library-deck-load")]
        if analysis.generation == 0
            || analysis
                .publication
                .as_ref()
                .is_some_and(|data| data.identity() != analysis_identity)
        {
            return false;
        }
        // Final publication check; the shared commit rechecks load admission.
        if self
            .deck
            .commit_track_load(
                deck,
                candidate.track.track_id,
                candidate.track.bpm_x100,
                source,
                persisted,
            )
            .is_err()
        {
            return false;
        }
        // Loop admission uses the source's decoded length, never catalog length.
        self.deck
            .set_decoded_duration_ms(deck, audio_source.and_then(decoded_duration_ms));

        self.loaded_tracks[deck_index(deck)] = Some(ProductLoadedTrack {
            candidate,
            source_cues_available,
            audio_source,
            #[cfg(feature = "library-deck-load")]
            analysis_identity: Some(analysis_identity),
        });
        self.advance_source_epoch(deck);
        #[cfg(feature = "library-deck-load")]
        {
            self.analyses.clear(deck_index(deck));
            if let Some(publication) = analysis.publication {
                let published =
                    self.analyses
                        .publish(deck_index(deck), analysis_identity, publication);
                debug_assert!(published);
            }
        }
        self.refresh_leds();
        true
    }

    pub(crate) fn loaded_track(&self, deck: DeckId) -> Option<ProductLoadedTrack> {
        self.loaded_tracks[deck_index(deck)]
    }

    /// Background copies use the current load identity and existing owned pins.
    /// The explicit USB0 bring-up caller supplies `transport_idle`, including actual
    /// downstream queues/in-flight work, not just accepted product commands.
    #[cfg(feature = "library-deck-load")]
    #[allow(clippy::result_large_err)]
    pub(crate) fn publish_background_analysis(
        &mut self,
        deck: DeckId,
        publication: AnalysisPublication,
        media: &pajoniiir_media_session::MediaSession,
        transport_idle: bool,
    ) -> Result<(), (BackgroundAnalysisRefusal, AnalysisPublication)> {
        let index = deck_index(deck);
        let identity = publication.identity();
        if !media.is_available()
            || !media.validate(identity.lease)
            || self.loaded_tracks[index].and_then(|t| t.analysis_identity) != Some(identity)
            || self.analyses.view(index, identity).is_some()
            || !matches!(
                publication.view().provider(),
                pajoniiir_track_analysis::AnalysisProvider::AptaNative
                    | pajoniiir_track_analysis::AnalysisProvider::AptaCache
            )
        {
            return Err((BackgroundAnalysisRefusal::Rejected, publication));
        }
        if !transport_idle
            || self.pending_transport_effects() != 0
            || !self.deck.deck(deck).can_replace_analysis()
        {
            return Err((BackgroundAnalysisRefusal::Deferred, publication));
        }
        let published = self.analyses.publish(index, identity, publication);
        debug_assert!(published);
        Ok(())
    }

    pub(crate) const fn source_epoch(&self, deck: DeckId) -> u32 {
        self.source_epochs[deck_index(deck)]
    }

    fn advance_source_epoch(&mut self, deck: DeckId) {
        let epoch = &mut self.source_epochs[deck_index(deck)];
        *epoch = epoch.wrapping_add(1).max(1);
        self.deck_effects.discard_source_effects(deck);
    }

    /// Apply only results from the current load and, for PCM, current stream.
    /// `stream_generation` is sampled by the dispatcher from the source owner.
    pub(crate) fn apply_transport_completion(
        &mut self,
        completion: TransportCompletion,
        stream_generation: u32,
    ) -> bool {
        let deck = completion.outcome.deck();
        if self.source_epoch(deck) != completion.source_epoch {
            return false;
        }
        match completion.outcome {
            TransportOutcome::Progress {
                generation,
                position_ms,
                ..
            } if generation != 0 && generation == stream_generation => {
                self.deck.set_position_ms(deck, position_ms);
                true
            }
            TransportOutcome::Playback {
                request_id,
                succeeded,
                ..
            } => self.resolve_playback_request(deck, request_id, succeeded),
            TransportOutcome::StreamEnded {
                generation,
                position_ms,
                ..
            } if generation != 0 && generation == stream_generation => {
                self.finish_stream_playback(deck, position_ms);
                true
            }
            TransportOutcome::SourceFailed { .. } if stream_generation == 0 => {
                self.invalidate_deck_source(deck);
                true
            }
            _ => false,
        }
    }

    pub(crate) fn invalidate_deck_source(&mut self, deck: DeckId) {
        self.advance_source_epoch(deck);
        #[cfg(feature = "library-deck-load")]
        self.analyses.clear(deck_index(deck));
        self.loaded_tracks[deck_index(deck)] = None;
        self.deck.unload_track(deck);
        self.refresh_leds();
    }

    pub(crate) fn invalidate_loaded_media(&mut self, current: Option<MediaLease>) -> u8 {
        let mut invalidated = 0u8;
        for deck in [DeckId::One, DeckId::Two] {
            let index = deck_index(deck);
            let stale = self.loaded_tracks[index]
                .is_some_and(|loaded| Some(loaded.candidate.publication.lease) != current);
            if stale {
                self.advance_source_epoch(deck);
                #[cfg(feature = "library-deck-load")]
                self.analyses.clear(index);
                self.loaded_tracks[index] = None;
                self.deck.unload_track(deck);
                invalidated = invalidated.saturating_add(1);
            }
        }
        if invalidated != 0 {
            self.refresh_leds();
        }
        invalidated
    }

    pub(crate) const fn pending_deck_effects(&self) -> usize {
        self.deck_effects.len()
    }

    pub(crate) const fn pending_transport_effects(&self) -> usize {
        self.deck_effects.transport_len()
    }

    pub(crate) const fn pending_persistence_effects(&self) -> usize {
        self.deck_effects.persistence_len()
    }

    pub(crate) const fn pending_audio_dsp_effects(&self) -> usize {
        self.deck_effects.audio_dsp_len()
    }

    pub(crate) fn next_transport_effect(&self) -> Option<DeckEffect> {
        self.deck_effects.transport_front()
    }

    pub(crate) fn complete_transport_effect(&mut self, succeeded: bool) -> bool {
        self.deck_effects.complete_transport(succeeded)
    }

    pub(crate) fn next_persistence_effect(&self) -> Option<DeckEffect> {
        self.deck_effects.persistence_front()
    }

    pub(crate) fn complete_persistence_effect(&mut self, succeeded: bool) -> bool {
        self.deck_effects.complete_persistence(succeeded)
    }

    pub(crate) fn next_audio_dsp_effect(&self) -> Option<DeckEffect> {
        self.deck_effects.audio_dsp_front()
    }

    pub(crate) fn complete_audio_dsp_effect(&mut self, succeeded: bool) -> bool {
        self.deck_effects.complete_audio_dsp(succeeded)
    }

    pub(crate) fn resolve_playback_request(
        &mut self,
        deck: DeckId,
        request_id: u32,
        succeeded: bool,
    ) -> bool {
        let resolved = self
            .deck
            .resolve_playback_request(deck, request_id, succeeded);
        if resolved {
            self.refresh_leds();
        }
        resolved
    }

    pub(crate) fn finish_stream_playback(&mut self, deck: DeckId, position_ms: u32) {
        self.deck.finish_playback(deck, position_ms);
        self.refresh_leds();
    }

    pub(crate) const fn has_dirty_leds(&self) -> bool {
        self.leds.has_dirty()
    }

    pub(crate) fn take_led_update(&mut self) -> Option<LedUpdate> {
        self.leds.take_next_dirty()
    }

    pub(crate) fn mark_led_resync(&mut self) {
        self.leds.mark_all_dirty();
    }

    pub(crate) const fn stats(&self) -> ProductRuntimeStats {
        self.stats
    }

    pub(crate) fn health_observation(&self, backpressured: bool) -> ProductObservation {
        ProductObservation {
            counters: self.stats(),
            queues: [
                self.pending_library_effects(),
                self.pending_transport_effects(),
                self.pending_persistence_effects(),
                self.pending_audio_dsp_effects(),
            ]
            .map(|v| v.min(u16::MAX as usize) as u16),
            playing: [
                self.deck.deck(DeckId::One).playing,
                self.deck.deck(DeckId::Two).playing,
            ],
            backpressured,
        }
    }

    pub(crate) const fn mixer(&self) -> &MixerState {
        &self.mixer
    }

    fn load_gate(&self, deck: DeckId) -> &'static DeckLoadGate {
        &self.load_gates[deck_index(deck)]
    }

    /// The controller owner mirrors the active profile's FILTER_ALWAYS here.
    pub(crate) fn set_channel_filter_always(&mut self, always: bool) {
        self.mixer.set_channel_filter_always(always);
    }

    pub(crate) const fn deck(&self) -> &DeckProductState {
        &self.deck
    }

    pub(crate) fn analysis_view(&self, deck: DeckId) -> Option<TrackAnalysis<'_>> {
        #[cfg(feature = "library-deck-load")]
        {
            let identity = self.loaded_tracks[deck_index(deck)]?.analysis_identity?;
            if Some(identity.lease) != crate::usb0_library::current_media_lease() {
                return None;
            }
            self.analyses.view(deck_index(deck), identity)
        }
        #[cfg(not(feature = "library-deck-load"))]
        {
            let _ = deck;
            None
        }
    }

    fn refresh_leds(&mut self) {
        for deck in [DeckId::One, DeckId::Two] {
            let deck_state = *self.deck.deck(deck);
            let mixer_state = self.mixer.deck(deck);

            self.set_deck_led(SemanticLedId::PLAY, deck, deck_state.playing);
            self.set_deck_led(
                if deck == DeckId::One {
                    SemanticLedId::TRACK_LOAD_DECK1
                } else {
                    SemanticLedId::TRACK_LOAD_DECK2
                },
                deck,
                self.loaded_tracks[deck_index(deck)].is_some(),
            );
            self.set_deck_led(SemanticLedId::PFL, deck, mixer_state.pfl_enabled);
            self.set_deck_led(SemanticLedId::SYNC, deck, deck_state.sync_enabled);
            self.set_deck_led(
                SemanticLedId::LOOP_IN,
                deck,
                deck_state.loop_state.pending_in_ms.is_some()
                    || deck_state.loop_state.active.is_some(),
            );
            self.set_deck_led(
                SemanticLedId::LOOP_OUT,
                deck,
                deck_state.loop_state.active.is_some(),
            );

            self.set_deck_led(
                SemanticLedId::PAD_MODE_HOT_CUE,
                deck,
                deck_state.pad_mode == PadMode::HotCue,
            );
            self.set_deck_led(
                SemanticLedId::PAD_MODE_BEAT_LOOP,
                deck,
                deck_state.pad_mode == PadMode::BeatLoop,
            );
            self.set_deck_led(
                SemanticLedId::PAD_MODE_BEAT_JUMP,
                deck,
                deck_state.pad_mode == PadMode::BeatJump,
            );
            self.set_deck_led(
                SemanticLedId::PAD_MODE_KEY_SHIFT,
                deck,
                deck_state.pad_mode == PadMode::KeyShift,
            );
            self.set_deck_led(
                SemanticLedId::PAD_MODE_KEYBOARD,
                deck,
                deck_state.pad_mode == PadMode::Keyboard,
            );
            self.set_deck_led(
                SemanticLedId::PAD_MODE_PAD_FX1,
                deck,
                deck_state.pad_mode == PadMode::PadFx1,
            );
            self.set_deck_led(
                SemanticLedId::PAD_MODE_PAD_FX2,
                deck,
                deck_state.pad_mode == PadMode::PadFx2,
            );
            self.set_deck_led(
                SemanticLedId::PAD_MODE_SAMPLER,
                deck,
                deck_state.pad_mode == PadMode::Sampler,
            );

            self.set_deck_led(SemanticLedId::CENSOR, deck, deck_state.censor_active);
            self.set_deck_led(
                SemanticLedId::LOOP_ADJUST_IN,
                deck,
                deck_state.loop_adjust_mode == LoopAdjustMode::In,
            );
            self.set_deck_led(
                SemanticLedId::LOOP_ADJUST_OUT,
                deck,
                deck_state.loop_adjust_mode == LoopAdjustMode::Out,
            );

            let hot_cue_mask = self
                .deck
                .hot_cues(deck)
                .map(|bank| bank.exists_mask())
                .unwrap_or(0);
            for pad in 0..8u8 {
                if let Some(led) = SemanticLedId::hot_cue_pad(pad) {
                    self.set_deck_led(led, deck, hot_cue_mask & (1u8 << pad) != 0);
                }
            }

            for pad in 0..8u8 {
                let active = deck_state.active_pad_fx;
                if let Some(led) = SemanticLedId::pad_fx1_pad(pad) {
                    self.set_deck_led(
                        led,
                        deck,
                        matches!(
                            active,
                            Some(active) if active.bank == PadFxBank::One && active.pad == pad
                        ),
                    );
                }
                if let Some(led) = SemanticLedId::pad_fx2_pad(pad) {
                    self.set_deck_led(
                        led,
                        deck,
                        matches!(
                            active,
                            Some(active) if active.bank == PadFxBank::Two && active.pad == pad
                        ),
                    );
                }
            }
        }

        let beat_fx = self.deck.beat_fx();
        self.set_deck_led(
            SemanticLedId::BEAT_FX_ON,
            DeckId::One,
            beat_fx.enabled
                && matches!(
                    beat_fx.target,
                    BeatFxTarget::ChannelOne | BeatFxTarget::Both
                ),
        );
        self.set_deck_led(
            SemanticLedId::BEAT_FX_ON,
            DeckId::Two,
            beat_fx.enabled
                && matches!(
                    beat_fx.target,
                    BeatFxTarget::ChannelTwo | BeatFxTarget::Both
                ),
        );

        self.set_global_led(SemanticLedId::SMART_CFX, self.mixer.smart_cfx_enabled());
        self.set_global_led(SemanticLedId::SMART_FADER, self.mixer.smart_fader_enabled());
        self.set_global_led(SemanticLedId::MASTER_CUE, self.mixer.master_cue_enabled());
    }

    fn set_deck_led(&mut self, led: SemanticLedId, deck: DeckId, enabled: bool) {
        let _ = self.leds.set(LedUpdate {
            led,
            deck: Some(deck),
            state: if enabled { LedState::On } else { LedState::Off },
        });
    }

    fn set_global_led(&mut self, led: SemanticLedId, enabled: bool) {
        let _ = self.leds.set(LedUpdate {
            led,
            deck: None,
            state: if enabled { LedState::On } else { LedState::Off },
        });
    }
}

impl SessionDeliverySink for ProductRuntime {
    fn try_accept(&mut self, delivery: SessionDelivery) -> bool {
        match delivery {
            SessionDelivery::Connection(state) => {
                // Disconnect releases an active CUE preview (pause + return to
                // cue). Publish those effects atomically or retry the delivery.
                let mut staged = self.deck;
                let releases = staged.set_controller_connected(state == ConnectionState::Connected);
                if !self.deck_effects.try_push_groups(&releases) {
                    self.stats.effect_backpressure =
                        self.stats.effect_backpressure.saturating_add(1);
                    return false;
                }
                self.deck = staged;
                self.refresh_leds();
                if state == ConnectionState::Connected {
                    self.mark_led_resync();
                }
                self.stats.accepted_connections = self.stats.accepted_connections.saturating_add(1);
                true
            }
            SessionDelivery::Control(event) => match event.control.domain() {
                ControlDomain::Deck => {
                    // With LOAD LOCK on, PLAY/jog withdraw a reserved load (the old
                    // session survives) and are refused while replacement runs.
                    if self.deck.load_lock()
                        && let Some(deck) = claims_transport(event, &self.deck)
                    {
                        match self.load_gate(deck).claim_for_transport() {
                            DeckClaim::Replacing => {
                                self.stats.transport_refused_loading =
                                    self.stats.transport_refused_loading.saturating_add(1);
                                return true;
                            }
                            DeckClaim::CancelledLoad | DeckClaim::Free => {}
                        }
                    }
                    let mut staged = self.deck;
                    let effects = staged.handle_control_with_analysis(
                        event,
                        [
                            self.analysis_view(DeckId::One),
                            self.analysis_view(DeckId::Two),
                        ],
                    );
                    if self.persistence_faulted
                        && effects.items.iter().flatten().any(|effect| {
                            effect.target() == pajoniiir_deck::DeckEffectTarget::Persistence
                        })
                    {
                        // Consume the rejected edit without changing its bank/LED
                        // or holding the controller FIFO ahead of other decks.
                        self.stats.deferred_controls =
                            self.stats.deferred_controls.saturating_add(1);
                        return true;
                    }
                    if !self.deck_effects.try_push_all(effects) {
                        self.stats.effect_backpressure =
                            self.stats.effect_backpressure.saturating_add(1);
                        return false;
                    }
                    // Accepted local edits belong to the track, not one deck's
                    // old snapshot. Keep the other deck current before its next
                    // edit, while preserving source and transport state.
                    for effect in effects.items.into_iter().flatten() {
                        if let DeckEffect::PersistHotCues { deck, bank } = effect
                            && let Some(origin) = self.loaded_tracks[deck_index(deck)]
                        {
                            for i in 0..2 {
                                if self.loaded_tracks[i].is_some_and(|other| {
                                    other.candidate.track.track_id == bank.track_id()
                                        && other.candidate.publication.lease
                                            == origin.candidate.publication.lease
                                }) {
                                    staged.update_local_hot_cues(
                                        if i == 0 { DeckId::One } else { DeckId::Two },
                                        bank,
                                    );
                                }
                            }
                        }
                    }
                    self.deck = staged;
                    self.refresh_leds();
                    self.stats.accepted_deck_controls =
                        self.stats.accepted_deck_controls.saturating_add(1);
                    true
                }
                ControlDomain::Mixer => {
                    let mut staged = self.mixer;
                    let _ = staged.handle_control(event);
                    self.mixer = staged;
                    self.refresh_leds();
                    self.stats.accepted_mixer_controls =
                        self.stats.accepted_mixer_controls.saturating_add(1);
                    true
                }
                ControlDomain::Library => {
                    #[cfg(feature = "library-deck-load")]
                    {
                        let effect = match (event.control, event.value) {
                            (SemanticControl::BrowseDelta, ControlValue::Relative(delta))
                                if delta != 0 =>
                            {
                                Some(LibraryEffect::Browse {
                                    delta,
                                    multiplier: 1,
                                })
                            }
                            (SemanticControl::ShiftBrowseDelta, ControlValue::Relative(delta))
                                if delta != 0 =>
                            {
                                Some(LibraryEffect::Browse {
                                    delta,
                                    multiplier: 10,
                                })
                            }
                            (SemanticControl::BrowsePress, ControlValue::Pressed(true)) => {
                                Some(LibraryEffect::OpenLibrary)
                            }
                            (SemanticControl::ShiftBrowsePress, ControlValue::Pressed(true)) => {
                                Some(LibraryEffect::Back)
                            }
                            (
                                SemanticControl::Load | SemanticControl::ShiftLoad,
                                ControlValue::Pressed(true),
                            ) => {
                                let Some(deck) = event.deck else {
                                    self.stats.accepted_library_controls =
                                        self.stats.accepted_library_controls.saturating_add(1);
                                    return true;
                                };
                                if library_cursor_on_node() {
                                    Some(LibraryEffect::Open)
                                } else {
                                    // One admission path for controller, touch and
                                    // future Web loads: verdict plus single-flight permit.
                                    match self
                                        .deck
                                        .load_admission(deck)
                                        .and_then(|_| self.load_gate(deck).reserve())
                                    {
                                        Ok(permit) => Some(LibraryEffect::Load { deck, permit }),
                                        Err(_) => {
                                            self.stats.library_load_locked =
                                                self.stats.library_load_locked.saturating_add(1);
                                            self.stats.accepted_library_controls = self
                                                .stats
                                                .accepted_library_controls
                                                .saturating_add(1);
                                            return true;
                                        }
                                    }
                                }
                            }
                            _ => None,
                        };

                        if let Some(effect) = effect
                            && !self.library_effects.try_push(effect)
                        {
                            if let LibraryEffect::Load { deck, permit } = effect {
                                self.load_gate(deck).finish(permit);
                            }
                            self.stats.effect_backpressure =
                                self.stats.effect_backpressure.saturating_add(1);
                            return false;
                        }

                        self.stats.accepted_library_controls =
                            self.stats.accepted_library_controls.saturating_add(1);
                        true
                    }

                    #[cfg(not(feature = "library-deck-load"))]
                    {
                        self.stats.deferred_controls =
                            self.stats.deferred_controls.saturating_add(1);
                        false
                    }
                }
                ControlDomain::Deferred => {
                    self.stats.deferred_controls = self.stats.deferred_controls.saturating_add(1);
                    false
                }
            },
        }
    }
}

const fn deck_index(deck: DeckId) -> usize {
    match deck {
        DeckId::One => 0,
        DeckId::Two => 1,
    }
}
