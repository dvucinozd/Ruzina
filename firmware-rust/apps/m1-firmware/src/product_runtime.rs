use pajoniiir_controller_session::{
    ConnectionState, ControlDomain, SessionDelivery, SessionDeliverySink,
};
use pajoniiir_deck::{DeckEffect, DeckEffectQueue, DeckProductState};
use pajoniiir_mixer::MixerState;

const PRODUCT_DECK_EFFECT_CAPACITY: usize = 16;

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
    deck_effects: DeckEffectQueue<PRODUCT_DECK_EFFECT_CAPACITY>,
    stats: ProductRuntimeStats,
}

impl ProductRuntime {
    pub(crate) const fn new() -> Self {
        Self {
            deck: DeckProductState::new(),
            mixer: MixerState::new(),
            deck_effects: DeckEffectQueue::new(),
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

    pub(crate) fn pop_deck_effect(&mut self) -> Option<DeckEffect> {
        self.deck_effects.pop()
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
}

impl SessionDeliverySink for ProductRuntime {
    fn try_accept(&mut self, delivery: SessionDelivery) -> bool {
        match delivery {
            SessionDelivery::Connection(state) => {
                self.deck
                    .set_controller_connected(state == ConnectionState::Connected);
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
                    self.stats.accepted_deck_controls =
                        self.stats.accepted_deck_controls.saturating_add(1);
                    true
                }
                ControlDomain::Mixer => {
                    let mut staged = self.mixer;
                    let _ = staged.handle_control(event);
                    self.mixer = staged;
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
