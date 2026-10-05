use pajoniiir_controller_session::{
    BeatFxTarget, ConnectionState, ControlDomain, ControllerLedFrame, DeckId, LedState, LedUpdate,
    PadMode, SemanticLedId, SessionDelivery, SessionDeliverySink,
};
use pajoniiir_deck::{DeckEffect, DeckEffectBroker, DeckProductState, LoopAdjustMode, PadFxBank};
use pajoniiir_mixer::MixerState;

const PRODUCT_TRANSPORT_EFFECT_CAPACITY: usize = 16;
const PRODUCT_PERSISTENCE_EFFECT_CAPACITY: usize = 4;
const PRODUCT_AUDIO_DSP_EFFECT_CAPACITY: usize = 16;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ProductRuntimeStats {
    pub(crate) accepted_connections: u32,
    pub(crate) accepted_deck_controls: u32,
    pub(crate) accepted_mixer_controls: u32,
    pub(crate) effect_backpressure: u32,
    pub(crate) deferred_controls: u32,
}

pub(crate) struct ProductRuntime<'a> {
    lifetime: core::marker::PhantomData<&'a mut ()>,
    #[cfg(any(feature = "apta-owner-compile", test))]
    analysis: AnalysisState<'a>,
    deck: DeckProductState,
    mixer: MixerState,
    deck_effects: DeckEffectBroker<
        PRODUCT_TRANSPORT_EFFECT_CAPACITY,
        PRODUCT_PERSISTENCE_EFFECT_CAPACITY,
        PRODUCT_AUDIO_DSP_EFFECT_CAPACITY,
    >,
    leds: ControllerLedFrame,
    stats: ProductRuntimeStats,
}

impl<'a> ProductRuntime<'a> {
    pub(crate) const fn new() -> Self {
        Self {
            lifetime: core::marker::PhantomData,
            #[cfg(any(feature = "apta-owner-compile", test))]
            analysis: AnalysisState::new(),
            deck: DeckProductState::new(),
            mixer: MixerState::new(),
            deck_effects: DeckEffectBroker::new(),
            leds: ControllerLedFrame::new(),
            stats: ProductRuntimeStats {
                accepted_connections: 0,
                accepted_deck_controls: 0,
                accepted_mixer_controls: 0,
                effect_backpressure: 0,
                deferred_controls: 0,
            },
        }
    }

    pub(crate) const fn pending_deck_effects(&self) -> usize {
        self.deck_effects.len()
    }

    pub(crate) fn next_transport_effect(&self) -> Option<DeckEffect> {
        self.deck_effects.transport_front()
    }

    pub(crate) fn complete_transport_effect(&mut self, succeeded: bool) -> bool {
        if !succeeded {
            return false;
        }
        if let Some(DeckEffect::PlaybackRequest {
            deck, request_id, ..
        }) = self.next_transport_effect()
        {
            self.deck.resolve_playback_request(deck, request_id, true);
            self.refresh_leds();
        }
        self.deck_effects.complete_transport(true)
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

    fn refresh_leds(&mut self) {
        for deck in [DeckId::One, DeckId::Two] {
            let deck_state = *self.deck.deck(deck);
            let mixer_state = self.mixer.deck(deck);

            self.set_deck_led(SemanticLedId::PLAY, deck, deck_state.playing);
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

impl SessionDeliverySink for ProductRuntime<'_> {
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
                    #[cfg(all(feature = "apta-owner-compile", not(test)))]
                    {
                        self.analysis.media = crate::usb0_msc::analysis_media_snapshot();
                    }
                    let mut staged = self.deck;
                    #[cfg(any(feature = "apta-owner-compile", test))]
                    let effects = staged.handle_control_with_analysis(
                        event,
                        [
                            self.analysis_track(DeckId::One, &self.analysis.media),
                            self.analysis_track(DeckId::Two, &self.analysis.media),
                        ],
                    );
                    #[cfg(not(any(feature = "apta-owner-compile", test)))]
                    let effects = staged.handle_control(event);
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
                ControlDomain::Library | ControlDomain::Deferred => {
                    self.stats.deferred_controls = self.stats.deferred_controls.saturating_add(1);
                    false
                }
            },
        }
    }
}

// The same firmware owner is source-included by host integration tests. No second
// Deck state or runtime implementation is maintained for testing.
#[cfg(any(feature = "apta-owner-compile", test))]
use pajoniiir_apta_adapter::{
    owner::Output,
    service::{Authority, Command, Retained, Selection, Status},
};
#[cfg(any(feature = "apta-owner-compile", test))]
use pajoniiir_media_session::MediaSession;

#[cfg(any(feature = "apta-owner-compile", test))]
struct AnalysisState<'a> {
    selected: [Option<Selection>; 2],
    pending_catalog: Option<(Selection, DeckId)>,
    retained: [Option<Retained<'a>>; 2],
    recycle: [Option<Output<'a>>; 2],
    revision: u32,
    media: MediaSession,
    status: Option<Status>,
}
#[cfg(any(feature = "apta-owner-compile", test))]
impl AnalysisState<'_> {
    const fn new() -> Self {
        Self {
            selected: [None, None],
            pending_catalog: None,
            retained: [None, None],
            recycle: [None, None],
            revision: 0,
            media: MediaSession::new(),
            status: None,
        }
    }
}
#[cfg(any(feature = "apta-owner-compile", test))]
fn index(deck: DeckId) -> usize {
    match deck {
        DeckId::One => 0,
        DeckId::Two => 1,
    }
}

#[cfg(any(feature = "apta-owner-compile", test))]
impl<'a> ProductRuntime<'a> {
    /// Call at the owner boundary before control processing. Invalid media hides
    /// views immediately; it does not reclaim transport-protected banks.
    pub(crate) fn analysis_media(&mut self, media: MediaSession, status: Option<Status>) {
        self.analysis.media = media;
        self.analysis.status = status;
    }
    pub(crate) fn analysis_status(&self) -> Option<Status> {
        self.analysis.status
    }

    /// Enqueue first, then commit selection and existing Hot Cue track identity.
    /// No catalog borrow survives this synchronous boundary. Queue refusal leaves
    /// product state unchanged. The caller can retry the copied selection.
    pub(crate) fn select_analysis(
        &mut self,
        selection: Selection,
        deck: DeckId,
        media: &MediaSession,
        mut send: impl FnMut(Command<'a>) -> Result<(), Command<'a>>,
    ) -> bool {
        if !media.is_available()
            || !media.validate(selection.lease())
            || !self.deck.deck(deck).can_replace_analysis()
            || self.next_transport_effect().is_some()
        {
            return false;
        }
        let Some(revision) = self.analysis.revision.checked_add(1) else {
            return false;
        };
        let selection = selection.with_revision(revision);
        if send(Command::Analyze(selection, deck)).is_err() {
            return false;
        }
        self.analysis.revision = revision;
        self.analysis.selected[index(deck)] = Some(selection);
        self.analysis.media = *media;
        if self.deck.hot_cues(deck).map(|b| b.track_id()) != Some(selection.track_id()) {
            self.deck.load_hot_cues(deck, selection.track_id(), None);
        }
        true
    }

    pub(crate) fn needs_catalog_selection(&self) -> bool {
        self.analysis.pending_catalog.is_none()
    }
    pub(crate) fn queue_catalog_selection(&mut self, selection: Selection, deck: DeckId) -> bool {
        if self.analysis.pending_catalog.is_some() {
            return false;
        }
        self.analysis.pending_catalog = Some((selection, deck));
        true
    }
    /// Retain an enqueue-refused catalog intent for the next owner turn. Stale
    /// media is an explicit rejection; blocked transport retains the intent.
    pub(crate) fn advance_analysis(
        &mut self,
        media: &MediaSession,
        mut send: impl FnMut(Command<'a>) -> Result<(), Command<'a>>,
    ) {
        self.analysis.media = *media;
        if let Some((selection, deck)) = self.analysis.pending_catalog {
            if !media.is_available() || !media.validate(selection.lease()) {
                self.analysis.pending_catalog = None;
                self.analysis.status = Some(Status::Rejected);
            } else if self.select_analysis(selection, deck, media, &mut send) {
                self.analysis.pending_catalog = None;
            }
        }
        for deck in [DeckId::One, DeckId::Two] {
            if self.analysis_track(deck, media).is_none() {
                self.retire_analysis(deck);
            }
        }
        self.flush_analysis_recycle(send);
    }

    /// Refusal returns the bank unchanged. The bounded output queue/caller owns
    /// it until rejection can move it into a retained recycle slot.
    #[allow(clippy::result_large_err)]
    pub(crate) fn accept_analysis(
        &mut self,
        mut bank: Retained<'a>,
        authority: Authority,
        media: &MediaSession,
    ) -> Result<(), Retained<'a>> {
        let deck = bank.target();
        let i = index(deck);
        if !self.analysis.selected[i].is_some_and(|s| bank.matches(s))
            || self.deck.hot_cues(deck).map(|b| b.track_id()) != Some(bank.track_id())
            || self.analysis.retained[i].is_some()
            || self.next_transport_effect().is_some()
            || bank.accept(authority, media, &self.deck).is_err()
        {
            return Err(bank);
        }
        self.analysis.media = *media;
        self.analysis.retained[i] = Some(bank);
        Ok(())
    }
    pub(crate) fn analysis_track(
        &self,
        deck: DeckId,
        media: &MediaSession,
    ) -> Option<pajoniiir_track_analysis::TrackAnalysis<'_>> {
        let bank = self.analysis.retained[index(deck)].as_ref()?;
        if !self.analysis.selected[index(deck)].is_some_and(|s| bank.matches(s)) {
            return None;
        }
        bank.track(media)
    }
    pub(crate) fn analysis_waveform(
        &self,
        deck: DeckId,
        media: &MediaSession,
    ) -> Option<pajoniiir_waveform::WaveformView<'_>> {
        self.analysis_track(deck, media)?;
        self.analysis.retained[index(deck)]
            .as_ref()?
            .waveform(media)
    }
    pub(crate) fn retire_analysis(&mut self, deck: DeckId) -> bool {
        let i = index(deck);
        // Cue/ToStart update logical state before their Pause/Seek effects are
        // acknowledged. Conservatively wait for the entire transport lane.
        if self.next_transport_effect().is_some()
            || self.analysis.recycle[i].is_some()
            || !self.deck.deck(deck).can_replace_analysis()
        {
            return false;
        }
        let Some(bank) = self.analysis.retained[i].take() else {
            return true;
        };
        match bank.retire(&self.deck) {
            Ok(output) => {
                self.analysis.recycle[i] = Some(output);
                true
            }
            Err(bank) => {
                self.analysis.retained[i] = Some(bank);
                false
            }
        }
    }
    #[allow(clippy::result_large_err)]
    pub(crate) fn reject_analysis(&mut self, bank: Retained<'a>) -> Result<(), Retained<'a>> {
        let Some(slot) = self.analysis.recycle.iter_mut().find(|s| s.is_none()) else {
            return Err(bank);
        };
        *slot = Some(bank.reject()?);
        Ok(())
    }
    pub(crate) fn flush_analysis_recycle(
        &mut self,
        mut send: impl FnMut(Command<'a>) -> Result<(), Command<'a>>,
    ) -> bool {
        for slot in &mut self.analysis.recycle {
            if let Some(output) = slot.take()
                && let Err(command) = send(Command::Recycle(output))
            {
                let Command::Recycle(output) = command else {
                    panic!("enqueue must return original command")
                };
                *slot = Some(output);
                return false;
            }
        }
        true
    }
    pub(crate) fn cancel_analysis(
        &mut self,
        mut send: impl FnMut(Command<'a>) -> Result<(), Command<'a>>,
    ) -> bool {
        if send(Command::Cancel).is_err() {
            return false;
        }
        self.analysis.pending_catalog = None;
        for i in 0..2 {
            if !self.analysis.retained[i]
                .as_ref()
                .is_some_and(|b| self.analysis.selected[i].is_some_and(|s| b.matches(s)))
            {
                self.analysis.selected[i] = None;
            }
        }
        true
    }
}
