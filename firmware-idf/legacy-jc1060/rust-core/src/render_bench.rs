//! Profile builds only: times the render graph at boot, before USB audio and the media
//! load start, to tell its computing from its memory traffic.
//!
//! The C side runs each case on a task of its own on core 0, at the USB client's priority,
//! with the graph on that task's stack in internal RAM, as `deck_mix` keeps it. Each
//! case renders `FILLS` fills of three 96-frame blocks with both decks playing, deck 2
//! off its track's tempo, and reports the mean, shortest and longest fill:
//! 0. the graph alone, from a source that is a register;
//! 1. the same with Beat FX echo on both decks;
//! 2. a source read from a ring in internal RAM, each frame copied to a history there,
//!    as `deck_play::Reader` does;
//! 3. the same ring and history in PSRAM;
//! 4. the PSRAM ring with the reader's three acquire loads a frame;
//! 5. to 8. the graph's parts alone, for both decks a frame: the resampler, the EQ, the
//!    channel filter at its center, and MAIN's limiter with the headphones' conversion;
//! 9. the graph with deck 2 stopped;
//! 10. the graph without effect storage.
use crate::deck_mix::MIX_BLOCK_FRAMES;
use alloc::vec;
use core::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
use ruzina_audio_dsp::{DelayMode, DspFrame, EqState, FilterState, PcmFrame, ResamplerState};
use ruzina_audio_runtime::fx::FX_DECK_STORAGE_SAMPLES;
use ruzina_audio_runtime::render::{
    FixedRateRenderGraph, RenderControl, RenderDeckControl, RenderOutputBlock,
};

const CASES: u32 = 11;
const FILLS: u32 = 300;
const WARMUP: u32 = 20;
/// Source frames per deck in a PSRAM ring: 2 MiB, larger than every cache.
const PSRAM_RING: usize = 1 << 19;
/// History frames per deck, as `deck_play` keeps.
const HISTORY: usize = 1 << 16;
/// Internal ring and history, per deck; small enough for internal RAM.
const INTERNAL_RING: usize = 1024;

static INTERNAL: [[[AtomicU32; INTERNAL_RING]; 2]; 2] =
    [const { [const { [const { AtomicU32::new(0) }; INTERNAL_RING] }; 2] }; 2];

unsafe extern "C" {
    fn esp_timer_get_time() -> i64;
}

fn now() -> i64 {
    // SAFETY: a plain IDF timer read.
    unsafe { esp_timer_get_time() }
}

fn control(echo: bool) -> RenderControl {
    let deck = |pitch| {
        let mut deck = RenderDeckControl {
            source_epoch: 1,
            stream_generation: 1,
            playing: true,
            source_sample_rate_hz: 44_100,
            pitch_centipercent: pitch,
            ..RenderDeckControl::idle()
        };
        if echo {
            deck.fx.beat_selected = true;
            deck.fx.beat.time.enabled = true;
            deck.fx.beat.time.mode = DelayMode::Echo;
            deck.fx.beat.time.delay_ms = 500;
            deck.fx.beat.time.wet_q15 = 20_000;
            deck.fx.beat.time.feedback_q15 = 16_000;
        }
        deck
    };
    RenderControl {
        decks: [deck(0), deck(250)],
        ..RenderControl::idle()
    }
}

/// Times `FILLS` fills of `graph` after a warmup, pulling source frames from `source`.
fn time(
    graph: &mut FixedRateRenderGraph<'_>,
    control: RenderControl,
    out: &mut [u32; 3],
    mut source: impl FnMut(usize) -> Option<PcmFrame>,
) {
    let mut block = RenderOutputBlock::silence();
    let (mut total, mut shortest, mut longest) = (0i64, i64::MAX, 0i64);
    for fill in 0..WARMUP + FILLS {
        let start = now();
        for _ in 0..288 / MIX_BLOCK_FRAMES {
            graph.render_block_frames(control, &mut block, MIX_BLOCK_FRAMES, &mut source);
        }
        let took = now() - start;
        if fill >= WARMUP {
            total += took;
            shortest = shortest.min(took);
            longest = longest.max(took);
        }
    }
    core::hint::black_box(&block);
    out[0] = (total / i64::from(FILLS)) as u32;
    out[1] = shortest as u32;
    out[2] = longest as u32;
}

/// Times `FILLS` fills of 288 calls of `frame`, after a warmup.
fn time_frames(out: &mut [u32; 3], mut frame: impl FnMut()) {
    let (mut total, mut shortest, mut longest) = (0i64, i64::MAX, 0i64);
    for fill in 0..WARMUP + FILLS {
        let start = now();
        for _ in 0..288 {
            frame();
        }
        let took = now() - start;
        if fill >= WARMUP {
            total += took;
            shortest = shortest.min(took);
            longest = longest.max(took);
        }
    }
    out[0] = (total / i64::from(FILLS)) as u32;
    out[1] = shortest as u32;
    out[2] = longest as u32;
}

fn dsp(state: &mut [u32; 2], deck: usize) -> DspFrame {
    DspFrame::from(noise(state, deck).unwrap_or_default())
}

/// The graph's parts alone, for both decks a frame (cases 5 to 8).
fn parts(case: u32, out: &mut [u32; 3]) {
    let mut state = [1u32, 7];
    match case {
        5 => {
            let mut resamplers = [ResamplerState::new(), ResamplerState::new()];
            let factors = [44_100.0 / 48_000.0, 44_100.0 * 1.025 / 48_000.0];
            time_frames(out, || {
                for (deck, resampler) in resamplers.iter_mut().enumerate() {
                    core::hint::black_box(
                        resampler.next(factors[deck], || noise(&mut state, deck)),
                    );
                }
            });
        }
        6 => {
            let mut eqs = [EqState::new(48_000), EqState::new(48_000)];
            time_frames(out, || {
                for (deck, eq) in eqs.iter_mut().enumerate() {
                    core::hint::black_box(eq.process_frame(dsp(&mut state, deck)));
                }
            });
        }
        7 => {
            let mut filters = [FilterState::new(48_000), FilterState::new(48_000)];
            time_frames(out, || {
                for (deck, filter) in filters.iter_mut().enumerate() {
                    core::hint::black_box(filter.process_frame(true, dsp(&mut state, deck)));
                }
            });
        }
        _ => {
            let mut stats = ruzina_mixer::LimiterStats::default();
            time_frames(out, || {
                let a = dsp(&mut state, 0);
                let b = dsp(&mut state, 1);
                let main = [
                    ruzina_mixer::limit_main_sample(a.left + b.left, &mut stats),
                    ruzina_mixer::limit_main_sample(a.right + b.right, &mut stats),
                ];
                core::hint::black_box((
                    main,
                    PcmFrame::from(DspFrame {
                        left: a.left * 0.5,
                        right: b.right * 0.5,
                    }),
                ));
            });
        }
    }
}

/// A frame from a register: a noise generator per deck.
fn noise(state: &mut [u32; 2], deck: usize) -> Option<PcmFrame> {
    state[deck] = state[deck]
        .wrapping_mul(1_664_525)
        .wrapping_add(1_013_904_223);
    let sample = (state[deck] >> 16) as i16 / 4;
    Some(PcmFrame {
        left: sample,
        right: sample.wrapping_neg(),
    })
}

fn pack(frame: PcmFrame) -> u32 {
    u32::from(frame.left as u16) | u32::from(frame.right as u16) << 16
}

fn unpack(word: u32) -> PcmFrame {
    PcmFrame {
        left: word as u16 as i16,
        right: (word >> 16) as u16 as i16,
    }
}

/// Runs `case` into `out`: the mean, shortest and longest fill in µs. Each case has a
/// graph of its own, so that no state carries over. Returns false for an unknown case,
/// or when PSRAM is short.
fn run(case: u32, out: &mut [u32; 3]) -> bool {
    if case >= CASES {
        return false;
    }
    if (5..=8).contains(&case) {
        parts(case, out);
        return true;
    }
    if case == 10 {
        let mut graph = FixedRateRenderGraph::new();
        let mut state = [1u32, 7];
        time(&mut graph, control(false), out, |deck| {
            noise(&mut state, deck)
        });
        return true;
    }
    let mut fx = vec![0.0f32; 2 * FX_DECK_STORAGE_SAMPLES];
    let (a, b) = fx.split_at_mut(FX_DECK_STORAGE_SAMPLES);
    let Ok(mut graph) = FixedRateRenderGraph::with_fx([a, b]) else {
        return false;
    };
    let plain = control(false);
    let mut state = [1u32, 7];
    let mut at = [0usize; 2];
    match case {
        0 => time(&mut graph, plain, out, |deck| noise(&mut state, deck)),
        9 => {
            let mut one = plain;
            one.decks[1].playing = false;
            time(&mut graph, one, out, |deck| noise(&mut state, deck));
        }
        1 => time(&mut graph, control(true), out, |deck| {
            noise(&mut state, deck)
        }),
        2 => {
            // Read a slot and copy it to the history, as the reader does.
            for (deck, ring) in INTERNAL[0].iter().enumerate() {
                for slot in ring {
                    slot.store(
                        pack(noise(&mut state, deck).unwrap_or_default()),
                        Ordering::Relaxed,
                    );
                }
            }
            time(&mut graph, plain, out, |deck| {
                let i = at[deck] % INTERNAL_RING;
                at[deck] += 1;
                let word = INTERNAL[0][deck][i].load(Ordering::Relaxed);
                INTERNAL[1][deck][i].store(word, Ordering::Relaxed);
                Some(unpack(word))
            });
        }
        _ => {
            let mut ring = vec![0u32; 2 * PSRAM_RING];
            let mut history = vec![0u32; 2 * HISTORY];
            for (i, slot) in ring.iter_mut().enumerate() {
                *slot = pack(noise(&mut state, i & 1).unwrap_or_default());
            }
            if case == 3 {
                time(&mut graph, plain, out, |deck| {
                    let i = at[deck];
                    at[deck] += 1;
                    let word = ring[deck * PSRAM_RING + i % PSRAM_RING];
                    history[deck * HISTORY + i % HISTORY] = word;
                    Some(unpack(word))
                });
            } else {
                // The reader's acquire loads: the head, the ring and the history.
                let head = AtomicU32::new(u32::MAX);
                let ring_base = AtomicPtr::new(ring.as_mut_ptr());
                let history_base = AtomicPtr::new(history.as_mut_ptr());
                time(&mut graph, plain, out, |deck| {
                    let i = at[deck];
                    at[deck] += 1;
                    if head.load(Ordering::Acquire) == i as u32 {
                        return None;
                    }
                    let ring = ring_base.load(Ordering::Acquire);
                    let history = history_base.load(Ordering::Acquire);
                    // SAFETY: the offset stays inside `ring`, which outlives this case.
                    let word = unsafe { ring.add(deck * PSRAM_RING + i % PSRAM_RING).read() };
                    // SAFETY: the offset stays inside `history`, which outlives this case.
                    unsafe { history.add(deck * HISTORY + i % HISTORY).write(word) };
                    Some(unpack(word))
                });
            }
            core::hint::black_box((&ring, &history));
        }
    }
    true
}

/// Runs case `case` (0 to 4) into the caller's three words; returns 1, or 0 when it
/// could not run.
///
/// # Safety
/// `out` must be valid for three aligned `u32` writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_render_bench(case: u32, out: *mut u32) -> u32 {
    if out.is_null() || !(out as usize).is_multiple_of(4) {
        return 0;
    }
    // SAFETY: the caller's three words.
    let out = unsafe { &mut *out.cast::<[u32; 3]>() };
    u32::from(run(case, out))
}
