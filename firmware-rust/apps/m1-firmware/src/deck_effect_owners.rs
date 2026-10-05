use core::cell::RefCell;

use crate::product_runtime::ProductRuntime;
use embassy_sync::{
    blocking_mutex::{CriticalSectionMutex, raw::CriticalSectionRawMutex},
    channel::{Channel, TrySendError},
    signal::Signal,
};
use pajoniiir_audio_runtime::{AudioEffectCommand, map_deck_effect};
use pajoniiir_controller_session::DeckId;
use pajoniiir_deck::{DeckEffect, DeckEffectTarget};

const TRANSPORT_OWNER_QUEUE_DEPTH: usize = 16;
const AUDIO_DSP_OWNER_QUEUE_DEPTH: usize = 16;
const TRANSPORT_COMPLETION_DEPTH: usize = 16;

static TRANSPORT_COMMANDS: Channel<
    CriticalSectionRawMutex,
    DeckEffect,
    TRANSPORT_OWNER_QUEUE_DEPTH,
> = Channel::new();
static AUDIO_DSP_COMMANDS: Channel<
    CriticalSectionRawMutex,
    DeckEffect,
    AUDIO_DSP_OWNER_QUEUE_DEPTH,
> = Channel::new();
static TRANSPORT_COMPLETIONS: Channel<
    CriticalSectionRawMutex,
    TransportCompletion,
    TRANSPORT_COMPLETION_DEPTH,
> = Channel::new();
static EFFECT_OWNER_PROGRESS: Signal<CriticalSectionRawMutex, ()> = Signal::new();

static TRANSPORT_OWNER_STATE: CriticalSectionMutex<RefCell<TransportOwnerState>> =
    CriticalSectionMutex::new(RefCell::new(TransportOwnerState::new()));
static AUDIO_DSP_OWNER_STATE: CriticalSectionMutex<RefCell<AudioDspOwnerState>> =
    CriticalSectionMutex::new(RefCell::new(AudioDspOwnerState::new()));

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TransportDeckOwnerState {
    pub(crate) playing: bool,
    pub(crate) position_ms: u32,
    pub(crate) pitch_centipercent: i16,
    pub(crate) loop_region: Option<(u32, u32)>,
}

impl TransportDeckOwnerState {
    const fn new() -> Self {
        Self {
            playing: false,
            position_ms: 0,
            pitch_centipercent: 0,
            loop_region: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TransportOwnerState {
    pub(crate) decks: [TransportDeckOwnerState; 2],
    pub(crate) accepted_commands: u32,
}

impl TransportOwnerState {
    const fn new() -> Self {
        Self {
            decks: [TransportDeckOwnerState::new(); 2],
            accepted_commands: 0,
        }
    }

    fn apply(&mut self, effect: DeckEffect) -> Option<TransportCompletion> {
        let completion = match effect {
            DeckEffect::PlaybackRequest {
                deck,
                playing,
                request_id,
            } => {
                self.deck_mut(deck).playing = playing;
                Some(TransportCompletion::Playback {
                    deck,
                    request_id,
                    succeeded: true,
                })
            }
            DeckEffect::Pause { deck } => {
                self.deck_mut(deck).playing = false;
                None
            }
            DeckEffect::Seek { deck, position_ms } => {
                self.deck_mut(deck).position_ms = position_ms;
                None
            }
            DeckEffect::SetPitchCentipercent { deck, value } => {
                self.deck_mut(deck).pitch_centipercent = value;
                None
            }
            DeckEffect::SetLoop {
                deck,
                start_ms,
                end_ms,
            } => {
                self.deck_mut(deck).loop_region = Some((start_ms, end_ms));
                None
            }
            DeckEffect::ClearLoop { deck } => {
                self.deck_mut(deck).loop_region = None;
                None
            }
            _ => return None,
        };
        self.accepted_commands = self.accepted_commands.saturating_add(1);
        completion
    }

    fn deck_mut(&mut self, deck: DeckId) -> &mut TransportDeckOwnerState {
        &mut self.decks[deck_index(deck)]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AudioDspOwnerState {
    pub(crate) last_command: Option<AudioEffectCommand>,
    pub(crate) accepted_commands: u32,
}

impl AudioDspOwnerState {
    const fn new() -> Self {
        Self {
            last_command: None,
            accepted_commands: 0,
        }
    }

    fn apply(&mut self, effect: DeckEffect) -> bool {
        let Some(command) = map_deck_effect(effect) else {
            return false;
        };
        self.last_command = Some(command);
        self.accepted_commands = self.accepted_commands.saturating_add(1);
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransportCompletion {
    Playback {
        deck: DeckId,
        request_id: u32,
        succeeded: bool,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct EffectOwnerDispatchReport {
    pub(crate) transport_accepted: u8,
    pub(crate) audio_dsp_accepted: u8,
    pub(crate) playback_completions: u8,
    pub(crate) transport_blocked: bool,
    pub(crate) audio_dsp_blocked: bool,
    pub(crate) persistence_pending: bool,
}

impl EffectOwnerDispatchReport {
    pub(crate) const fn made_progress(self) -> bool {
        self.transport_accepted != 0
            || self.audio_dsp_accepted != 0
            || self.playback_completions != 0
    }
}

pub(crate) fn transport_owner_state() -> TransportOwnerState {
    TRANSPORT_OWNER_STATE.lock(|cell| *cell.borrow())
}

pub(crate) fn audio_dsp_owner_state() -> AudioDspOwnerState {
    AUDIO_DSP_OWNER_STATE.lock(|cell| *cell.borrow())
}

pub(crate) async fn wait_for_effect_owner_progress() {
    EFFECT_OWNER_PROGRESS.wait().await;
}

pub(crate) fn dispatch_product_effects(
    product: &mut ProductRuntime,
) -> EffectOwnerDispatchReport {
    let mut report = EffectOwnerDispatchReport::default();

    while let Ok(completion) = TRANSPORT_COMPLETIONS.try_receive() {
        match completion {
            TransportCompletion::Playback {
                deck,
                request_id,
                succeeded,
            } => {
                if product.resolve_playback_request(deck, request_id, succeeded) {
                    report.playback_completions =
                        report.playback_completions.saturating_add(1);
                }
            }
        }
    }

    while let Some(effect) = product.next_transport_effect() {
        match try_submit(DeckEffectTarget::Transport, effect, &TRANSPORT_COMMANDS) {
            Ok(()) => {
                let completed = product.complete_transport_effect(true);
                debug_assert!(completed);
                report.transport_accepted = report.transport_accepted.saturating_add(1);
            }
            Err(()) => {
                report.transport_blocked = true;
                break;
            }
        }
    }

    while let Some(effect) = product.next_audio_dsp_effect() {
        match try_submit(DeckEffectTarget::AudioDsp, effect, &AUDIO_DSP_COMMANDS) {
            Ok(()) => {
                let completed = product.complete_audio_dsp_effect(true);
                debug_assert!(completed);
                report.audio_dsp_accepted = report.audio_dsp_accepted.saturating_add(1);
            }
            Err(()) => {
                report.audio_dsp_blocked = true;
                break;
            }
        }
    }

    // Do not acknowledge persistence without durable storage. The released C
    // product uses NVS; the native Rust firmware still needs an explicitly
    // reserved flash partition/backend before this lane can be consumed safely.
    report.persistence_pending = product.pending_persistence_effects() != 0;
    report
}

fn try_submit<const N: usize>(
    target: DeckEffectTarget,
    effect: DeckEffect,
    channel: &Channel<CriticalSectionRawMutex, DeckEffect, N>,
) -> Result<(), ()> {
    if effect.target() != target {
        return Err(());
    }
    match channel.try_send(effect) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => Err(()),
    }
}

pub(crate) async fn run_transport_owner() -> ! {
    loop {
        let effect = TRANSPORT_COMMANDS.receive().await;
        let completion =
            TRANSPORT_OWNER_STATE.lock(|cell| cell.borrow_mut().apply(effect));
        if let Some(completion) = completion {
            TRANSPORT_COMPLETIONS.send(completion).await;
        }
        EFFECT_OWNER_PROGRESS.signal(());
    }
}

pub(crate) async fn run_audio_dsp_owner() -> ! {
    loop {
        let effect = AUDIO_DSP_COMMANDS.receive().await;
        let accepted = AUDIO_DSP_OWNER_STATE.lock(|cell| cell.borrow_mut().apply(effect));
        debug_assert!(accepted);
        EFFECT_OWNER_PROGRESS.signal(());
    }
}

const fn deck_index(deck: DeckId) -> usize {
    match deck {
        DeckId::One => 0,
        DeckId::Two => 1,
    }
}
