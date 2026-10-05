use core::cell::RefCell;
#[cfg(feature = "hotcue-flash-storage")]
use core::sync::atomic::{AtomicBool, Ordering};

use crate::product_runtime::ProductRuntime;
#[cfg(feature = "hotcue-flash-storage")]
use crate::hotcue_flash::EspHotCueFlash;
use embassy_sync::{
    blocking_mutex::{CriticalSectionMutex, raw::CriticalSectionRawMutex},
    channel::{Channel, TrySendError},
    signal::Signal,
};
use pajoniiir_audio_runtime::{AudioEffectCommand, map_deck_effect};
use pajoniiir_controller_session::DeckId;
use pajoniiir_deck::{DeckEffect, DeckEffectTarget};
#[cfg(feature = "hotcue-flash-storage")]
use pajoniiir_hot_cues::{
    HotCueStore, RecordBackedHotCueStore,
    journal::HotCueJournalBackend,
};

const TRANSPORT_OWNER_QUEUE_DEPTH: usize = 16;
const AUDIO_DSP_OWNER_QUEUE_DEPTH: usize = 16;
const TRANSPORT_COMPLETION_DEPTH: usize = 16;
#[cfg(feature = "hotcue-flash-storage")]
const PERSISTENCE_OWNER_QUEUE_DEPTH: usize = 1;
#[cfg(feature = "hotcue-flash-storage")]
const PERSISTENCE_COMPLETION_DEPTH: usize = 1;

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

#[cfg(feature = "hotcue-flash-storage")]
static PERSISTENCE_COMMANDS: Channel<
    CriticalSectionRawMutex,
    DeckEffect,
    PERSISTENCE_OWNER_QUEUE_DEPTH,
> = Channel::new();
#[cfg(feature = "hotcue-flash-storage")]
static PERSISTENCE_COMPLETIONS: Channel<
    CriticalSectionRawMutex,
    PersistenceCompletion,
    PERSISTENCE_COMPLETION_DEPTH,
> = Channel::new();
#[cfg(feature = "hotcue-flash-storage")]
static PERSISTENCE_IN_FLIGHT: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "hotcue-flash-storage")]
static PERSISTENCE_FAULTED: AtomicBool = AtomicBool::new(false);

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

#[cfg(feature = "hotcue-flash-storage")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PersistenceCompletion {
    effect: DeckEffect,
    succeeded: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct EffectOwnerDispatchReport {
    pub(crate) transport_accepted: u8,
    pub(crate) audio_dsp_accepted: u8,
    pub(crate) playback_completions: u8,
    pub(crate) persistence_completions: u8,
    pub(crate) transport_blocked: bool,
    pub(crate) audio_dsp_blocked: bool,
    pub(crate) persistence_blocked: bool,
    pub(crate) persistence_faulted: bool,
    pub(crate) persistence_pending: bool,
}

impl EffectOwnerDispatchReport {
    pub(crate) const fn made_progress(self) -> bool {
        self.transport_accepted != 0
            || self.audio_dsp_accepted != 0
            || self.playback_completions != 0
            || self.persistence_completions != 0
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

    dispatch_persistence_effect(product, &mut report);
    report.persistence_pending = product.pending_persistence_effects() != 0;
    report
}


#[cfg(feature = "hotcue-flash-storage")]
fn dispatch_persistence_effect(
    product: &mut ProductRuntime,
    report: &mut EffectOwnerDispatchReport,
) {
    if let Ok(completion) = PERSISTENCE_COMPLETIONS.try_receive() {
        PERSISTENCE_IN_FLIGHT.store(false, Ordering::Release);
        if product.next_persistence_effect() != Some(completion.effect) {
            PERSISTENCE_FAULTED.store(true, Ordering::Release);
            report.persistence_faulted = true;
            return;
        }

        if completion.succeeded {
            let completed = product.complete_persistence_effect(true);
            debug_assert!(completed);
            report.persistence_completions = report.persistence_completions.saturating_add(1);
        } else {
            PERSISTENCE_FAULTED.store(true, Ordering::Release);
            report.persistence_faulted = true;
            return;
        }
    }

    if PERSISTENCE_FAULTED.load(Ordering::Acquire) {
        report.persistence_faulted = true;
        report.persistence_blocked = product.pending_persistence_effects() != 0;
        return;
    }

    if PERSISTENCE_IN_FLIGHT.load(Ordering::Acquire) {
        report.persistence_blocked = product.pending_persistence_effects() != 0;
        return;
    }

    let Some(effect) = product.next_persistence_effect() else {
        return;
    };
    if effect.target() != DeckEffectTarget::Persistence {
        PERSISTENCE_FAULTED.store(true, Ordering::Release);
        report.persistence_faulted = true;
        report.persistence_blocked = true;
        return;
    }

    match PERSISTENCE_COMMANDS.try_send(effect) {
        Ok(()) => PERSISTENCE_IN_FLIGHT.store(true, Ordering::Release),
        Err(TrySendError::Full(_)) => report.persistence_blocked = true,
    }
}

#[cfg(not(feature = "hotcue-flash-storage"))]
fn dispatch_persistence_effect(
    product: &mut ProductRuntime,
    report: &mut EffectOwnerDispatchReport,
) {
    report.persistence_blocked = product.pending_persistence_effects() != 0;
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

#[cfg(feature = "hotcue-flash-storage")]
pub(crate) async fn run_persistence_owner(
    flash: esp_hal::peripherals::FLASH<'static>,
) -> ! {
    let flash = match EspHotCueFlash::open(flash) {
        Ok(flash) => flash,
        Err(_) => {
            PERSISTENCE_FAULTED.store(true, Ordering::Release);
            EFFECT_OWNER_PROGRESS.signal(());
            return core::future::pending::<core::convert::Infallible>().await;
        }
    };
    let backend = HotCueJournalBackend::new(flash);
    let mut store = RecordBackedHotCueStore::new(backend);

    loop {
        let effect = PERSISTENCE_COMMANDS.receive().await;
        let succeeded = match effect {
            DeckEffect::PersistHotCues { bank, .. } => store.save(&bank).is_ok(),
            _ => false,
        };
        if !succeeded {
            PERSISTENCE_FAULTED.store(true, Ordering::Release);
        }
        PERSISTENCE_COMPLETIONS
            .send(PersistenceCompletion { effect, succeeded })
            .await;
        EFFECT_OWNER_PROGRESS.signal(());

        if !succeeded {
            return core::future::pending::<core::convert::Infallible>().await;
        }
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
