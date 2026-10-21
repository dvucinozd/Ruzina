use alloc::vec::Vec;
use core::cell::RefCell;

use embassy_sync::blocking_mutex::CriticalSectionMutex;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::{Duration, Timer};
use pajoniiir_audio_dsp::PcmFrame;
use pajoniiir_audio_runtime::{
    fx::{FX_DECK_STORAGE_SAMPLES, FX_TOTAL_STORAGE_SAMPLES, product_fx_controls},
    pcm_stream::PcmStreamBlock,
    playback::PlaybackOutput,
    render::{
        FixedRateRenderGraph, RENDER_BLOCK_FRAMES, RENDER_SAMPLE_RATE_HZ, ReferenceSink,
        RenderControl, RenderDeckControl, RenderOutputBlock,
    },
    sink::{BlockProgress, SinkBlock, SinkRole},
    source::{CanonicalSource, IngestResult, SOURCE_HISTORY_FRAMES, SOURCE_PREFETCH_BLOCK_LIMIT},
};
use pajoniiir_controller_session::DeckId;
use static_cell::{ConstStaticCell, StaticCell};

use crate::{
    deck_audio_source::{
        active_stream_generation, current_stream_generation, try_receive_pcm_block,
    },
    deck_effect_owners::try_submit_render_progress,
    product_runtime::ProductRuntime,
};

const RENDER_BLOCK_PERIOD_US: u64 =
    ((RENDER_BLOCK_FRAMES as u64 * 1_000_000) + RENDER_SAMPLE_RATE_HZ as u64 - 1)
        / RENDER_SAMPLE_RATE_HZ as u64;

static RENDER_CONTROL: CriticalSectionMutex<RefCell<RenderControl>> =
    CriticalSectionMutex::new(RefCell::new(RenderControl::idle()));
static RENDER_STATS: CriticalSectionMutex<RefCell<AudioRenderOwnerStats>> =
    CriticalSectionMutex::new(RefCell::new(AudioRenderOwnerStats::new()));
static RENDER_OUTPUT: StaticCell<RenderOutputBlock> = StaticCell::new();
// BSS-backed buffers: no large array is constructed on an Embassy task stack.
static HISTORY_A: ConstStaticCell<[PcmFrame; SOURCE_HISTORY_FRAMES]> =
    ConstStaticCell::new([PcmFrame { left: 0, right: 0 }; SOURCE_HISTORY_FRAMES]);
static HISTORY_B: ConstStaticCell<[PcmFrame; SOURCE_HISTORY_FRAMES]> =
    ConstStaticCell::new([PcmFrame { left: 0, right: 0 }; SOURCE_HISTORY_FRAMES]);
static PLAYBACK_MAIN: ConstStaticCell<[SinkBlock; 2]> = ConstStaticCell::new([SinkBlock::EMPTY; 2]);
static PLAYBACK_PHONES: ConstStaticCell<[SinkBlock; 1]> =
    ConstStaticCell::new([SinkBlock::EMPTY; 1]);

#[derive(Clone, Copy)]
struct ScratchJog {
    deck: DeckId,
    generation: u32,
    ticks: i16,
    gesture_epoch: u32,
}
static SCRATCH_JOGS: Channel<CriticalSectionRawMutex, ScratchJog, 16> = Channel::new();

pub(crate) async fn queue_scratch_jog(deck: DeckId, ticks: i16, gesture_epoch: u32) {
    // This is called by the same transport owner that serializes seeks, so a
    // queued pre-seek jog retains its old generation and is discarded on reset.
    let generation = current_stream_generation(deck);
    SCRATCH_JOGS
        .send(ScratchJog {
            deck,
            generation,
            ticks,
            gesture_epoch,
        })
        .await;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct AudioRenderOwnerStats {
    pub(crate) fx_ready: bool,
    pub(crate) fx_storage_failed: bool,
    pub(crate) rendered_blocks: u64,
    pub(crate) rendered_frames: u64,
    pub(crate) underflow_frames: [u64; 2],
    pub(crate) deferred_mode_frames: [u64; 2],
    pub(crate) progress_backpressure: u64,
    pub(crate) reference_digest: u64,
}

impl AudioRenderOwnerStats {
    const fn new() -> Self {
        Self {
            fx_ready: false,
            fx_storage_failed: false,
            rendered_blocks: 0,
            rendered_frames: 0,
            underflow_frames: [0, 0],
            deferred_mode_frames: [0, 0],
            progress_backpressure: 0,
            reference_digest: 0xcbf2_9ce4_8422_2325,
        }
    }
}

pub(crate) fn audio_render_owner_stats() -> AudioRenderOwnerStats {
    RENDER_STATS.lock(|cell| *cell.borrow())
}

pub(crate) fn publish_product_render_control(product: &ProductRuntime) {
    let mixer = product.mixer();
    let gains = mixer.stage_gains();
    let effects = product_fx_controls(product.deck());
    let mut control = RenderControl::idle();
    control.master_gain = gains.master;
    control.headphones = mixer.headphone_gains();

    for (index, deck) in [DeckId::One, DeckId::Two].into_iter().enumerate() {
        let deck_state = *product.deck().deck(deck);
        let mixer_deck = mixer.deck(deck);
        let source = product
            .loaded_track(deck)
            .and_then(|loaded| loaded.audio_source);
        let stream_generation = active_stream_generation(deck);
        control.decks[index] = RenderDeckControl {
            source_epoch: product.source_epoch(deck),
            fx: effects[index],
            playing: deck_state.playing && source.is_some() && stream_generation != 0,
            stream_generation,
            source_sample_rate_hz: source
                .map(|descriptor| descriptor.sample_rate_hz())
                .unwrap_or(RENDER_SAMPLE_RATE_HZ),
            pitch_centipercent: deck_state.pitch_centipercent,
            master_tempo: deck_state.master_tempo,
            scratch_active: deck_state.jog_touched && source.is_some() && stream_generation != 0,
            scratch_epoch: deck_state.scratch_epoch,
            censor_active: deck_state.censor_active,
            pre_gain: gains.pre[index],
            post_gain: gains.post[index],
            eq_low: mixer_deck.eq_low,
            eq_mid: mixer_deck.eq_mid,
            eq_high: mixer_deck.eq_high,
            filter_raw: mixer.effective_filter_raw(deck),
            pfl_enabled: mixer_deck.pfl_enabled,
        };
    }

    RENDER_CONTROL.lock(|cell| *cell.borrow_mut() = control);
}

fn current_render_control() -> RenderControl {
    RENDER_CONTROL.lock(|cell| *cell.borrow())
}

pub(crate) async fn run_audio_render_owner() -> ! {
    let output = RENDER_OUTPUT.init(RenderOutputBlock::silence());
    // Allocate once from the existing PSRAM heap before rendering. No buffer
    // growth or whole-delay-line clearing occurs in the render loop.
    let mut fx_memory = Vec::new();
    if fx_memory
        .try_reserve_exact(FX_TOTAL_STORAGE_SAMPLES)
        .is_err()
    {
        fx_memory_failed().await;
    }
    fx_memory.resize(FX_TOTAL_STORAGE_SAMPLES, 0.0f32);
    let (fx_a, fx_b) = fx_memory.split_at_mut(FX_DECK_STORAGE_SAMPLES);
    let mut graph = match FixedRateRenderGraph::with_fx([fx_a, fx_b]) {
        Ok(graph) => graph,
        Err(_) => fx_memory_failed().await,
    };
    RENDER_STATS.lock(|cell| cell.borrow_mut().fx_ready = true);
    let mut sink = ReferenceSink::new();
    let mut sources = [
        CanonicalSource::new(HISTORY_A.take()).expect("history A capacity"),
        CanonicalSource::new(HISTORY_B.take()).expect("history B capacity"),
    ];
    let mut pending_blocks: [Option<PcmStreamBlock>; 2] = [None, None];
    let mut observed_generation = [0u32; 2];
    // Explicit reference consumer: no physical DAC/UAC readiness is advertised.
    let mut playback = PlaybackOutput::new_reference(PLAYBACK_MAIN.take(), PLAYBACK_PHONES.take())
        .expect("reference delivery capacity");

    loop {
        for (index, deck) in [DeckId::One, DeckId::Two].into_iter().enumerate() {
            let generation = current_stream_generation(deck);
            if generation != observed_generation[index] {
                observed_generation[index] = generation;
                sources[index].reset(generation);
                pending_blocks[index] = None;
                graph.reset_deck(index);
            }
            for _ in 0..SOURCE_PREFETCH_BLOCK_LIMIT {
                let Some(block) = pending_blocks[index]
                    .take()
                    .or_else(|| try_receive_pcm_block(deck))
                else {
                    break;
                };
                match sources[index].ingest(&block) {
                    IngestResult::Backpressure => {
                        pending_blocks[index] = Some(block);
                        break;
                    }
                    IngestResult::Fault => break,
                    IngestResult::Accepted | IngestResult::Stale => {}
                }
            }
        }

        let mut control = current_render_control();
        let active_generations = [
            active_stream_generation(DeckId::One),
            active_stream_generation(DeckId::Two),
        ];
        for (index, active) in active_generations.into_iter().enumerate() {
            if control.decks[index].stream_generation != active {
                control.decks[index].stream_generation = 0;
                control.decks[index].playing = false;
                control.decks[index].scratch_active = false;
            }
        }
        let first_frame = playback.lane(SinkRole::Main).clock_frame();
        playback
            .sync_sources(active_generations, first_frame)
            .expect("reference playback epoch");
        let token = playback
            .lane(SinkRole::Main)
            .token()
            .expect("reference binding");
        playback
            .lane_mut(SinkRole::Main)
            .set_mute(token, false, 0)
            .expect("reference unmute");
        playback
            .lane_mut(SinkRole::Main)
            .expect_active(
                token,
                control
                    .decks
                    .iter()
                    .any(|deck| deck.playing || deck.scratch_active),
            )
            .expect("reference activity");
        for _ in 0..16 {
            let Ok(jog) = SCRATCH_JOGS.try_receive() else {
                break;
            };
            let index = if jog.deck == DeckId::One { 0 } else { 1 };
            if control.decks[index].scratch_active
                && control.decks[index].scratch_epoch == jog.gesture_epoch
                && control.decks[index].stream_generation == jog.generation
            {
                sources[index].jog(jog.generation, jog.gesture_epoch, jog.ticks);
            }
        }

        let permit = playback.render_permit();
        let first_frame = playback
            .next_block_frame()
            .expect("reference output demand");
        let report = graph.render_canonical_block(control, output, &mut sources);
        let mut block_progress = BlockProgress::default();

        for index in 0..2 {
            if control.decks[index].stream_generation == 0
                || control.decks[index].stream_generation != observed_generation[index]
            {
                continue;
            }
            if let Some(progress) =
                sources[index].take_progress(control.decks[index].source_control())
            {
                let frames = report.source_frames_consumed[index].min(u16::MAX as u32) as u16;
                block_progress.decks[index] = Some(progress);
                block_progress.source_frames[index] = frames;
            }
        }

        let submitted = playback.submit_render(permit, first_frame, output, block_progress);
        submitted[0].expect("reference block delivery");
        // The reference consumer accepts immediately. Physical drivers must
        // retain this receipt until DMA/UAC completion, never acknowledge fill.
        let receipt = playback
            .lane_mut(SinkRole::Main)
            .issue(token, first_frame, RENDER_BLOCK_FRAMES, &mut output.main)
            .expect("reference request");
        sink.write(output);
        playback.complete(receipt).expect("reference completion");
        for (index, deck) in [DeckId::One, DeckId::Two].into_iter().enumerate() {
            if let Some((progress, frames)) = playback.pending_progress(index) {
                if try_submit_render_progress(deck, progress, frames) {
                    playback.clear_progress(index);
                } else {
                    RENDER_STATS.lock(|cell| {
                        let mut stats = cell.borrow_mut();
                        stats.progress_backpressure = stats.progress_backpressure.saturating_add(1);
                    });
                }
            }
        }

        RENDER_STATS.lock(|cell| {
            let mut stats = cell.borrow_mut();
            stats.rendered_blocks = stats.rendered_blocks.saturating_add(1);
            stats.rendered_frames = stats
                .rendered_frames
                .saturating_add(u64::from(report.rendered_frames));
            for index in 0..2 {
                stats.underflow_frames[index] = stats.underflow_frames[index]
                    .saturating_add(u64::from(report.underflow_frames[index]));
                stats.deferred_mode_frames[index] = stats.deferred_mode_frames[index]
                    .saturating_add(u64::from(report.deferred_mode_frames[index]));
            }
            stats.reference_digest = sink.digest();
        });

        Timer::after(Duration::from_micros(RENDER_BLOCK_PERIOD_US)).await;
    }
}

async fn fx_memory_failed() -> ! {
    RENDER_STATS.lock(|cell| cell.borrow_mut().fx_storage_failed = true);
    // Fail closed: do not advance the reference consumer with missing effects.
    loop {
        Timer::after(Duration::from_millis(250)).await;
    }
}
