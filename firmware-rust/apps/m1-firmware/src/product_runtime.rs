use pajoniiir_audio_runtime::wav::WavPcm16SourceDescriptor;
use pajoniiir_controller_core::{ControlValue, SemanticControl};
use pajoniiir_controller_session::{
    BeatFxTarget, ConnectionState, ControlDomain, ControllerLedFrame, DeckId, LedState, LedUpdate,
    PadMode, SemanticLedId, SessionDelivery, SessionDeliverySink,
};
use pajoniiir_deck::{DeckEffect, DeckEffectBroker, DeckProductState, LoopAdjustMode, PadFxBank};
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

const PRODUCT_LIBRARY_EFFECT_CAPACITY: usize = 8;
const PRODUCT_TRANSPORT_EFFECT_CAPACITY: usize = 16;
const PRODUCT_PERSISTENCE_EFFECT_CAPACITY: usize = 4;
const PRODUCT_AUDIO_DSP_EFFECT_CAPACITY: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LibraryEffect {
    Browse { delta: i16, multiplier: u16 },
    OpenLibrary,
    Load { deck: DeckId },
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
    pub(crate) wav_source: Option<WavPcm16SourceDescriptor>,
    #[cfg(feature = "library-deck-load")]
    pub(crate) analysis_identity: Option<AnalysisIdentity>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ProductRuntimeStats {
    pub(crate) accepted_connections: u32,
    pub(crate) accepted_deck_controls: u32,
    pub(crate) accepted_mixer_controls: u32,
    pub(crate) accepted_library_controls: u32,
    pub(crate) library_load_locked: u32,
    pub(crate) library_failures: u32,
    pub(crate) effect_backpressure: u32,
    pub(crate) deferred_controls: u32,
}

pub(crate) struct ProductRuntime {
    deck: DeckProductState,
    mixer: MixerState,
    library_effects: LibraryEffectQueue<PRODUCT_LIBRARY_EFFECT_CAPACITY>,
    loaded_tracks: [Option<ProductLoadedTrack>; 2],
    #[cfg(feature = "library-deck-load")]
    analyses: AnalysisPins,
    deck_effects: DeckEffectBroker<
        PRODUCT_TRANSPORT_EFFECT_CAPACITY,
        PRODUCT_PERSISTENCE_EFFECT_CAPACITY,
        PRODUCT_AUDIO_DSP_EFFECT_CAPACITY,
    >,
    leds: ControllerLedFrame,
    stats: ProductRuntimeStats,
}

impl ProductRuntime {
    pub(crate) const fn new() -> Self {
        Self {
            deck: DeckProductState::new(),
            mixer: MixerState::new(),
            library_effects: LibraryEffectQueue::new(),
            loaded_tracks: [None, None],
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

    pub(crate) fn commit_library_load(
        &mut self,
        deck: DeckId,
        candidate: CatalogLoadCandidate,
        source: HotCueBank,
        persisted: Option<HotCueBank>,
        source_cues_available: bool,
        wav_source: Option<WavPcm16SourceDescriptor>,
        #[cfg(feature = "library-deck-load")] analysis: ProductAnalysisLoad,
    ) -> bool {
        if candidate.track.track_id != source.track_id()
            || persisted.is_some_and(|bank| bank.track_id() != candidate.track.track_id)
        {
            return false;
        }
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
        if !self.deck.commit_track_load(
            deck,
            candidate.track.track_id,
            candidate.track.bpm_x100,
            source,
            persisted,
        ) {
            return false;
        }

        self.loaded_tracks[deck_index(deck)] = Some(ProductLoadedTrack {
            candidate,
            source_cues_available,
            wav_source,
            #[cfg(feature = "library-deck-load")]
            analysis_identity: analysis.publication.as_ref().map(|_| analysis_identity),
        });
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

    pub(crate) fn invalidate_deck_source(&mut self, deck: DeckId) {
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

    pub(crate) const fn mixer(&self) -> &MixerState {
        &self.mixer
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
                self.deck
                    .set_controller_connected(state == ConnectionState::Connected);
                self.refresh_leds();
                if state == ConnectionState::Connected {
                    self.mark_led_resync();
                }
                self.stats.accepted_connections = self.stats.accepted_connections.saturating_add(1);
                true
            }
            SessionDelivery::Control(event) => match event.control.domain() {
                ControlDomain::Deck => {
                    let mut staged = self.deck;
                    let effects = staged.handle_control_with_analysis(
                        event,
                        [
                            self.analysis_view(DeckId::One),
                            self.analysis_view(DeckId::Two),
                        ],
                    );
                    if !self.deck_effects.try_push_all(effects) {
                        self.stats.effect_backpressure =
                            self.stats.effect_backpressure.saturating_add(1);
                        return false;
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
                            (
                                SemanticControl::BrowsePress | SemanticControl::ShiftBrowsePress,
                                ControlValue::Pressed(true),
                            ) => Some(LibraryEffect::OpenLibrary),
                            (
                                SemanticControl::Load | SemanticControl::ShiftLoad,
                                ControlValue::Pressed(true),
                            ) => {
                                let Some(deck) = event.deck else {
                                    self.stats.accepted_library_controls =
                                        self.stats.accepted_library_controls.saturating_add(1);
                                    return true;
                                };
                                if self.deck.deck(deck).playing {
                                    self.stats.library_load_locked =
                                        self.stats.library_load_locked.saturating_add(1);
                                    self.stats.accepted_library_controls =
                                        self.stats.accepted_library_controls.saturating_add(1);
                                    return true;
                                }
                                Some(LibraryEffect::Load { deck })
                            }
                            _ => None,
                        };

                        if let Some(effect) = effect
                            && !self.library_effects.try_push(effect)
                        {
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
