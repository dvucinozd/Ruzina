use core::cell::RefCell;

use embassy_sync::blocking_mutex::CriticalSectionMutex;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::{Duration, Timer};
use pajoniiir_audio_dsp::PcmFrame;
use pajoniiir_audio_runtime::{
    pcm_stream::{PcmStreamBlock, PendingRenderProgress},
    render::{
        FixedRateRenderGraph, RENDER_BLOCK_FRAMES, RENDER_SAMPLE_RATE_HZ, ReferenceSink,
        RenderControl, RenderDeckControl, RenderOutputBlock,
    },
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
    let mut control = RenderControl::idle();
    control.master_gain = gains.master;

    for (index, deck) in [DeckId::One, DeckId::Two].into_iter().enumerate() {
        let deck_state = *product.deck().deck(deck);
        let mixer_deck = mixer.deck(deck);
        let source = product
            .loaded_track(deck)
            .and_then(|loaded| loaded.wav_source);
        let stream_generation = active_stream_generation(deck);
        control.decks[index] = RenderDeckControl {
            playing: deck_state.playing && source.is_some() && stream_generation != 0,
            stream_generation,
            source_sample_rate_hz: source
                .map(|descriptor| descriptor.format.sample_rate_hz)
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
    let mut graph = FixedRateRenderGraph::new();
    let mut sink = ReferenceSink::new();
    let mut sources = [
        CanonicalSource::new(HISTORY_A.take()).expect("history A capacity"),
        CanonicalSource::new(HISTORY_B.take()).expect("history B capacity"),
    ];
    let mut pending_blocks: [Option<PcmStreamBlock>; 2] = [None, None];
    let mut observed_generation = [0u32; 2];
    let mut pending_progress = [PendingRenderProgress::default(); 2];

    loop {
        for (index, deck) in [DeckId::One, DeckId::Two].into_iter().enumerate() {
            let generation = current_stream_generation(deck);
            if generation != observed_generation[index] {
                observed_generation[index] = generation;
                sources[index].reset(generation);
                pending_blocks[index] = None;
                pending_progress[index].clear();
                graph.reset_deck(index);
            }
            if let Some((progress, frames)) = pending_progress[index].peek() {
                if try_submit_render_progress(deck, progress, frames) {
                    pending_progress[index].clear();
                } else {
                    RENDER_STATS.lock(|cell| {
                        let mut stats = cell.borrow_mut();
                        stats.progress_backpressure = stats.progress_backpressure.saturating_add(1);
                    });
                }
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

        let control = current_render_control();
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

        let report = graph.render_canonical_block(control, output, &mut sources);
        sink.write(output);

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
                pending_progress[index].record(progress, frames);
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
