use core::cell::RefCell;

use embassy_sync::blocking_mutex::CriticalSectionMutex;
use embassy_time::{Duration, Timer};
use pajoniiir_audio_runtime::{
    pcm_stream::{PcmBlockFeed, PcmStreamProgress},
    render::{
        FixedRateRenderGraph, ReferenceSink, RenderControl, RenderDeckControl, RenderOutputBlock,
        RENDER_BLOCK_FRAMES, RENDER_SAMPLE_RATE_HZ,
    },
};
use pajoniiir_controller_session::DeckId;
use static_cell::StaticCell;

use crate::{
    deck_audio_source::{
        current_stream_generation, try_receive_pcm_block,
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
        let source = product.loaded_track(deck).and_then(|loaded| loaded.wav_source);
        control.decks[index] = RenderDeckControl {
            playing: deck_state.playing && source.is_some(),
            source_sample_rate_hz: source
                .map(|descriptor| descriptor.format.sample_rate_hz)
                .unwrap_or(RENDER_SAMPLE_RATE_HZ),
            pitch_centipercent: deck_state.pitch_centipercent,
            master_tempo: deck_state.master_tempo,
            scratch_active: deck_state.jog_touched || deck_state.censor_active,
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
    let mut feeds = [PcmBlockFeed::new(), PcmBlockFeed::new()];
    let mut observed_generation = [0u32; 2];
    let mut pending_progress: [Option<(PcmStreamProgress, u16)>; 2] = [None, None];

    loop {
        let mut blocked = false;
        for (index, deck) in [DeckId::One, DeckId::Two].into_iter().enumerate() {
            if let Some((progress, frames)) = pending_progress[index] {
                if try_submit_render_progress(deck, progress, frames) {
                    pending_progress[index] = None;
                } else {
                    blocked = true;
                    RENDER_STATS.lock(|cell| {
                        let mut stats = cell.borrow_mut();
                        stats.progress_backpressure = stats.progress_backpressure.saturating_add(1);
                    });
                }
            }
        }
        if blocked {
            Timer::after(Duration::from_millis(1)).await;
            continue;
        }

        for (index, deck) in [DeckId::One, DeckId::Two].into_iter().enumerate() {
            let generation = current_stream_generation(deck);
            if generation != observed_generation[index] {
                observed_generation[index] = generation;
                feeds[index].reset((generation != 0).then_some(generation));
                graph.reset_deck(index);
            }
        }

        let control = current_render_control();
        let report = graph.render_block(control, output, |index| {
            let deck = if index == 0 { DeckId::One } else { DeckId::Two };
            feeds[index].pop(|| try_receive_pcm_block(deck))
        });
        sink.write(output);

        for index in 0..2 {
            if let Some(progress) = feeds[index].take_progress() {
                let frames = report.source_frames_consumed[index].min(u16::MAX as u32) as u16;
                pending_progress[index] = Some((progress, frames));
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
