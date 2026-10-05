use pajoniiir_controller_session::{
    BeatFxTarget, ConnectionState, ControlDomain, ControllerLedFrame, DeckId, LedState, LedUpdate,
    PadMode, SemanticLedId, SessionDelivery, SessionDeliverySink,
};
use pajoniiir_deck::{
    DeckEffect, DeckEffectBroker, DeckProductState, LoopAdjustMode, PadFxBank,
};
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

pub(crate) struct ProductRuntime {
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

impl ProductRuntime {
    pub(crate) const fn new() -> Self {
        Self {
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
                && matches!(beat_fx.target, BeatFxTarget::ChannelOne | BeatFxTarget::Both),
        );
        self.set_deck_led(
            SemanticLedId::BEAT_FX_ON,
            DeckId::Two,
            beat_fx.enabled
                && matches!(beat_fx.target, BeatFxTarget::ChannelTwo | BeatFxTarget::Both),
        );

        self.set_global_led(SemanticLedId::SMART_CFX, self.mixer.smart_cfx_enabled());
        self.set_global_led(
            SemanticLedId::SMART_FADER,
            self.mixer.smart_fader_enabled(),
        );
        self.set_global_led(
            SemanticLedId::MASTER_CUE,
            self.mixer.master_cue_enabled(),
        );
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
                self.stats.accepted_connections =
                    self.stats.accepted_connections.saturating_add(1);
                true
            }
            SessionDelivery::Control(event) => match event.control.domain() {
                ControlDomain::Deck => {
                    let mut staged = self.deck;
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
