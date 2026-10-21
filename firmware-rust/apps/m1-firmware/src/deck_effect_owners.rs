use core::cell::RefCell;
#[cfg(feature = "hotcue-flash-storage")]
use core::sync::atomic::{AtomicBool, Ordering};

#[cfg(all(
    feature = "pcm-transport-bringup",
    not(feature = "render-graph-bringup")
))]
use crate::deck_audio_source::try_receive_pcm_block;
#[cfg(feature = "pcm-transport-bringup")]
use crate::deck_audio_source::{change_wav_source, seek_wav_source};
#[cfg(feature = "hotcue-flash-storage")]
use crate::hotcue_access::HotCueAccess;
#[cfg(feature = "hotcue-flash-storage")]
pub(crate) use crate::hotcue_access::HotCueLoadResult;
#[cfg(feature = "hotcue-flash-storage")]
use crate::hotcue_flash::EspHotCueFlash;
use crate::product_runtime::ProductRuntime;
use crate::transport_owner_state::{
    TransportCommand, TransportCompletion, TransportOutcome, TransportOwnerState,
};
#[cfg(any(feature = "hotcue-flash-storage", feature = "pcm-transport-bringup"))]
use embassy_futures::select::{Either, select};
use embassy_sync::{
    blocking_mutex::{CriticalSectionMutex, raw::CriticalSectionRawMutex},
    channel::{Channel, TrySendError},
    signal::Signal,
};
#[cfg(all(
    feature = "pcm-transport-bringup",
    not(feature = "render-graph-bringup")
))]
use embassy_time::{Duration, Timer};
#[cfg(feature = "render-graph-bringup")]
use pajoniiir_audio_runtime::pcm_stream::PcmStreamProgress;
#[cfg(feature = "pcm-transport-bringup")]
use pajoniiir_audio_runtime::wav_stream::WavStreamChange;
use pajoniiir_audio_runtime::{AudioEffectCommand, map_deck_effect};
use pajoniiir_controller_session::DeckId;
#[cfg(feature = "hotcue-flash-storage")]
use pajoniiir_core::MediaTrackId;
use pajoniiir_deck::{DeckEffect, DeckEffectTarget};
#[cfg(feature = "hotcue-flash-storage")]
use pajoniiir_hot_cues::{RecordBackedHotCueStore, journal::HotCueJournalBackend};

const TRANSPORT_OWNER_QUEUE_DEPTH: usize = 16;
const AUDIO_DSP_OWNER_QUEUE_DEPTH: usize = 16;
const TRANSPORT_COMPLETION_DEPTH: usize = 16;
#[cfg(feature = "render-graph-bringup")]
const RENDER_PROGRESS_DEPTH: usize = 8;
#[cfg(feature = "hotcue-flash-storage")]
const PERSISTENCE_OWNER_QUEUE_DEPTH: usize = 1;
#[cfg(feature = "hotcue-flash-storage")]
const PERSISTENCE_COMPLETION_DEPTH: usize = 1;
#[cfg(feature = "hotcue-flash-storage")]
const HOT_CUE_LOAD_QUEUE_DEPTH: usize = 1;

static TRANSPORT_COMMANDS: Channel<
    CriticalSectionRawMutex,
    TransportCommand,
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
#[cfg(feature = "render-graph-bringup")]
static RENDER_PROGRESS: Channel<
    CriticalSectionRawMutex,
    RenderTransportProgress,
    RENDER_PROGRESS_DEPTH,
> = Channel::new();

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
#[cfg(feature = "hotcue-flash-storage")]
static HOT_CUE_LOAD_REQUESTS: Channel<
    CriticalSectionRawMutex,
    HotCueLoadRequest,
    HOT_CUE_LOAD_QUEUE_DEPTH,
> = Channel::new();
#[cfg(feature = "hotcue-flash-storage")]
static HOT_CUE_LOAD_COMPLETIONS: Channel<
    CriticalSectionRawMutex,
    HotCueLoadCompletion,
    HOT_CUE_LOAD_QUEUE_DEPTH,
> = Channel::new();

static TRANSPORT_OWNER_STATE: CriticalSectionMutex<RefCell<TransportOwnerState>> =
    CriticalSectionMutex::new(RefCell::new(TransportOwnerState::new()));
static AUDIO_DSP_OWNER_STATE: CriticalSectionMutex<RefCell<AudioDspOwnerState>> =
    CriticalSectionMutex::new(RefCell::new(AudioDspOwnerState::new()));

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

#[cfg(feature = "render-graph-bringup")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RenderTransportProgress {
    deck: DeckId,
    progress: PcmStreamProgress,
    frames: u16,
}

#[cfg(feature = "hotcue-flash-storage")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PersistenceCompletion {
    effect: DeckEffect,
    succeeded: bool,
}

#[cfg(feature = "hotcue-flash-storage")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HotCueLoadRequest {
    request_id: u32,
    track_id: MediaTrackId,
}

#[cfg(feature = "hotcue-flash-storage")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HotCueLoadCompletion {
    request_id: u32,
    result: HotCueLoadResult,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct EffectOwnerDispatchReport {
    pub(crate) transport_accepted: u8,
    pub(crate) audio_dsp_accepted: u8,
    pub(crate) playback_completions: u8,
    pub(crate) stream_completions: u8,
    pub(crate) stream_progress: u8,
    pub(crate) stream_faults: u8,
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
            || self.stream_completions != 0
            || self.stream_progress != 0
            || self.stream_faults != 0
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

#[cfg(feature = "hotcue-flash-storage")]
pub(crate) async fn load_persisted_hot_cues(
    request_id: u32,
    track_id: MediaTrackId,
) -> HotCueLoadResult {
    if PERSISTENCE_FAULTED.load(Ordering::Acquire) {
        return HotCueLoadResult::Failed;
    }

    HOT_CUE_LOAD_REQUESTS
        .send(HotCueLoadRequest {
            request_id,
            track_id,
        })
        .await;
    let completion = HOT_CUE_LOAD_COMPLETIONS.receive().await;
    if completion.request_id != request_id {
        PERSISTENCE_FAULTED.store(true, Ordering::Release);
        EFFECT_OWNER_PROGRESS.signal(());
        HotCueLoadResult::Failed
    } else {
        completion.result
    }
}

pub(crate) fn dispatch_product_effects(product: &mut ProductRuntime) -> EffectOwnerDispatchReport {
    let mut report = EffectOwnerDispatchReport::default();

    sync_transport_epochs(product);
    while let Ok(completion) = TRANSPORT_COMPLETIONS.try_receive() {
        let deck = completion.outcome.deck();
        #[cfg(feature = "pcm-transport-bringup")]
        let generation = match completion.outcome {
            TransportOutcome::SourceFailed { .. } => {
                crate::deck_audio_source::current_stream_generation(deck)
            }
            _ => crate::deck_audio_source::active_stream_generation(deck),
        };
        #[cfg(not(feature = "pcm-transport-bringup"))]
        let generation = 0;
        let _ = deck;
        if !product.apply_transport_completion(completion, generation) {
            continue;
        }
        match completion.outcome {
            TransportOutcome::Progress { .. } => {
                report.stream_progress = report.stream_progress.saturating_add(1);
            }
            TransportOutcome::Playback { .. } => {
                report.playback_completions = report.playback_completions.saturating_add(1);
            }
            TransportOutcome::StreamEnded { faulted, .. } => {
                report.stream_completions = report.stream_completions.saturating_add(1);
                if faulted {
                    report.stream_faults = report.stream_faults.saturating_add(1);
                }
            }
            TransportOutcome::SourceFailed { .. } => {
                report.stream_faults = report.stream_faults.saturating_add(1);
            }
        }
    }
    sync_transport_epochs(product);

    while let Some(effect) = product.next_transport_effect() {
        let command = TransportCommand {
            effect,
            source_epoch: product.source_epoch(effect.source_deck().expect("transport deck")),
        };
        match TRANSPORT_COMMANDS.try_send(command) {
            Ok(()) => {
                let completed = product.complete_transport_effect(true);
                debug_assert!(completed);
                report.transport_accepted = report.transport_accepted.saturating_add(1);
            }
            Err(TrySendError::Full(_)) => {
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
    if report.persistence_faulted {
        product.note_persistence_fault();
    }
    report.persistence_pending = product.pending_persistence_effects() != 0;
    report
}

fn sync_transport_epochs(product: &ProductRuntime) {
    TRANSPORT_OWNER_STATE.lock(|cell| {
        cell.borrow_mut().sync_source_epochs([
            product.source_epoch(DeckId::One),
            product.source_epoch(DeckId::Two),
        ])
    });
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
pub(crate) async fn run_persistence_owner(flash: esp_hal::peripherals::FLASH<'static>) -> ! {
    let store = EspHotCueFlash::open(flash)
        .ok()
        .map(|flash| RecordBackedHotCueStore::new(HotCueJournalBackend::new(flash)));
    let mut access = HotCueAccess::new(store);
    if access.faulted() {
        PERSISTENCE_FAULTED.store(true, Ordering::Release);
        EFFECT_OWNER_PROGRESS.signal(());
    }

    loop {
        match select(
            PERSISTENCE_COMMANDS.receive(),
            HOT_CUE_LOAD_REQUESTS.receive(),
        )
        .await
        {
            Either::First(effect) => {
                if PERSISTENCE_FAULTED.load(Ordering::Acquire) {
                    access.latch_fault();
                }
                let succeeded = match effect {
                    DeckEffect::PersistHotCues { bank, .. } => access.save(&bank),
                    _ => false,
                };
                if !succeeded {
                    PERSISTENCE_FAULTED.store(true, Ordering::Release);
                }
                PERSISTENCE_COMPLETIONS
                    .send(PersistenceCompletion { effect, succeeded })
                    .await;
                EFFECT_OWNER_PROGRESS.signal(());
            }
            Either::Second(request) => {
                if PERSISTENCE_FAULTED.load(Ordering::Acquire) {
                    access.latch_fault();
                }
                let result = access.load(request.track_id);
                if access.faulted() {
                    PERSISTENCE_FAULTED.store(true, Ordering::Release);
                }
                HOT_CUE_LOAD_COMPLETIONS
                    .send(HotCueLoadCompletion {
                        request_id: request.request_id,
                        result,
                    })
                    .await;
                EFFECT_OWNER_PROGRESS.signal(());
            }
        }
    }
}

#[cfg(feature = "render-graph-bringup")]
pub(crate) fn try_submit_render_progress(
    deck: DeckId,
    progress: PcmStreamProgress,
    frames: u16,
) -> bool {
    match RENDER_PROGRESS.try_send(RenderTransportProgress {
        deck,
        progress,
        frames,
    }) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => false,
    }
}

#[cfg(feature = "pcm-transport-bringup")]
async fn execute_source_command(command: TransportCommand, request_id: &mut u32) {
    *request_id = request_id.wrapping_add(1).max(1);
    let deck = command.deck();
    let result = match command.effect {
        DeckEffect::Seek { position_ms, .. } => {
            seek_wav_source(deck, *request_id, position_ms).await
        }
        DeckEffect::SetLoop {
            start_ms, end_ms, ..
        } => {
            let position_frame = TRANSPORT_OWNER_STATE.lock(|cell| {
                cell.borrow().decks[deck_index(deck)]
                    .pcm_cursor
                    .position_frame()
            });
            change_wav_source(
                deck,
                *request_id,
                WavStreamChange::Loop {
                    position_frame,
                    region_ms: Some((start_ms, end_ms)),
                },
            )
            .await
        }
        DeckEffect::ClearLoop { .. } => {
            let position_frame = TRANSPORT_OWNER_STATE.lock(|cell| {
                cell.borrow().decks[deck_index(deck)]
                    .pcm_cursor
                    .position_frame()
            });
            change_wav_source(
                deck,
                *request_id,
                WavStreamChange::Loop {
                    position_frame,
                    region_ms: None,
                },
            )
            .await
        }
        _ => return,
    };
    match result {
        Ok(seek) => {
            TRANSPORT_OWNER_STATE.lock(|cell| {
                cell.borrow_mut().bind_source_control(
                    command,
                    seek.generation,
                    seek.frame,
                    seek.sample_rate_hz,
                )
            });
        }
        Err(()) => {
            let completion =
                TRANSPORT_OWNER_STATE.lock(|cell| cell.borrow_mut().fail_seek(command));
            if let Some(completion) = completion {
                TRANSPORT_COMPLETIONS.send(completion).await;
            }
        }
    }
}

pub(crate) async fn run_transport_owner() -> ! {
    #[cfg(feature = "render-graph-bringup")]
    {
        let mut source_request_id = 0u32;
        loop {
            match select(TRANSPORT_COMMANDS.receive(), RENDER_PROGRESS.receive()).await {
                Either::First(command) => {
                    if !TRANSPORT_OWNER_STATE.lock(|cell| cell.borrow().accepts(command)) {
                        EFFECT_OWNER_PROGRESS.signal(());
                        continue;
                    }
                    let effect = command.effect;
                    if matches!(
                        effect,
                        DeckEffect::Seek { .. }
                            | DeckEffect::SetLoop { .. }
                            | DeckEffect::ClearLoop { .. }
                    ) {
                        execute_source_command(command, &mut source_request_id).await;
                    } else if let DeckEffect::ScratchJog { deck, ticks, epoch } = effect {
                        crate::audio_render_owner::queue_scratch_jog(deck, ticks, epoch).await;
                    } else {
                        let completion =
                            TRANSPORT_OWNER_STATE.lock(|cell| cell.borrow_mut().apply(command));
                        if let Some(completion) = completion {
                            TRANSPORT_COMPLETIONS.send(completion).await;
                        }
                    }
                    EFFECT_OWNER_PROGRESS.signal(());
                }
                Either::Second(render) => {
                    let completion = TRANSPORT_OWNER_STATE.lock(|cell| {
                        cell.borrow_mut().consume_render_progress(
                            render.deck,
                            render.progress,
                            render.frames,
                            crate::deck_audio_source::active_stream_generation(render.deck),
                        )
                    });
                    if let Some(completion) = completion {
                        TRANSPORT_COMPLETIONS.send(completion).await;
                    }
                    EFFECT_OWNER_PROGRESS.signal(());
                }
            }
        }
    }

    #[cfg(all(
        feature = "pcm-transport-bringup",
        not(feature = "render-graph-bringup")
    ))]
    {
        let mut source_request_id = 0u32;
        loop {
            match select(
                TRANSPORT_COMMANDS.receive(),
                Timer::after(Duration::from_millis(1)),
            )
            .await
            {
                Either::First(command) => {
                    if !TRANSPORT_OWNER_STATE.lock(|cell| cell.borrow().accepts(command)) {
                        EFFECT_OWNER_PROGRESS.signal(());
                        continue;
                    }
                    let effect = command.effect;
                    if matches!(
                        effect,
                        DeckEffect::Seek { .. }
                            | DeckEffect::SetLoop { .. }
                            | DeckEffect::ClearLoop { .. }
                    ) {
                        execute_source_command(command, &mut source_request_id).await;
                    } else {
                        let completion =
                            TRANSPORT_OWNER_STATE.lock(|cell| cell.borrow_mut().apply(command));
                        if let Some(completion) = completion {
                            TRANSPORT_COMPLETIONS.send(completion).await;
                        }
                    }
                    EFFECT_OWNER_PROGRESS.signal(());
                }
                Either::Second(()) => {
                    for deck in [DeckId::One, DeckId::Two] {
                        let playing = TRANSPORT_OWNER_STATE
                            .lock(|cell| cell.borrow().decks[deck_index(deck)].playing);
                        if !playing {
                            continue;
                        }

                        for _ in 0..16 {
                            let Some(block) = try_receive_pcm_block(deck) else {
                                break;
                            };
                            let completion = TRANSPORT_OWNER_STATE
                                .lock(|cell| cell.borrow_mut().consume_pcm_block(deck, &block));
                            if let Some(completion) = completion {
                                let terminal = matches!(
                                    completion.outcome,
                                    TransportOutcome::StreamEnded { .. }
                                        | TransportOutcome::SourceFailed { .. }
                                );
                                TRANSPORT_COMPLETIONS.send(completion).await;
                                EFFECT_OWNER_PROGRESS.signal(());
                                if terminal {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[cfg(not(feature = "pcm-transport-bringup"))]
    loop {
        let command = TRANSPORT_COMMANDS.receive().await;
        let completion = TRANSPORT_OWNER_STATE.lock(|cell| cell.borrow_mut().apply(command));
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
