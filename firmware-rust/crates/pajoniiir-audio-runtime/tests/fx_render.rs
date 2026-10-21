use pajoniiir_audio_dsp::{DelayMode, PadFxMode, PcmFrame};
use pajoniiir_audio_runtime::{
    fx::{DeckFxControl, FX_DECK_STORAGE_SAMPLES, FX_TOTAL_STORAGE_SAMPLES},
    render::{FixedRateRenderGraph, RenderControl, RenderDeckControl, RenderOutputBlock},
};

fn memory() -> [Vec<f32>; 2] {
    [
        vec![0.0; FX_DECK_STORAGE_SAMPLES],
        vec![0.0; FX_DECK_STORAGE_SAMPLES],
    ]
}
fn active() -> RenderDeckControl {
    RenderDeckControl {
        source_epoch: 1,
        stream_generation: 1,
        playing: true,
        pfl_enabled: true,
        ..RenderDeckControl::idle()
    }
}
fn config(deck: RenderDeckControl) -> RenderControl {
    RenderControl {
        decks: [deck, RenderDeckControl::idle()],
        master_gain: 1.0,
    }
}
fn delay(deck: &mut RenderDeckControl, mode: DelayMode, milliseconds: u32) {
    deck.fx.beat_selected = true;
    deck.fx.beat.time.enabled = true;
    deck.fx.beat.time.mode = mode;
    deck.fx.beat.time.delay_ms = milliseconds;
    deck.fx.beat.time.wet_q15 = 32767;
    deck.fx.beat.time.feedback_q15 = if mode == DelayMode::Echo { 16000 } else { 0 };
}
fn render(
    graph: &mut FixedRateRenderGraph<'_>,
    control: RenderControl,
    inputs: [PcmFrame; 2],
) -> RenderOutputBlock {
    let mut output = RenderOutputBlock::silence();
    graph.render_block(control, &mut output, |i| Some(inputs[i]));
    output
}
fn impulse(graph: &mut FixedRateRenderGraph<'_>, control: RenderControl) -> RenderOutputBlock {
    let mut offset = [0usize; 2];
    let mut output = RenderOutputBlock::silence();
    graph.render_block(control, &mut output, |deck| {
        let first = offset[deck] == 0;
        offset[deck] += 1;
        Some(if first {
            PcmFrame {
                left: 12000,
                right: -6000,
            }
        } else {
            PcmFrame::default()
        })
    });
    output
}

#[test]
fn disabled_rack_is_bit_identical_to_existing_eq_filter_and_mixer_path() {
    let [mut a, mut b] = memory();
    let mut wet = FixedRateRenderGraph::with_fx([&mut a, &mut b]).unwrap();
    let mut dry = FixedRateRenderGraph::new();
    let mut deck = active();
    deck.eq_low = 6000;
    deck.eq_high = 10000;
    deck.filter_raw = 7000;
    deck.post_gain = 0.4;
    let control = RenderControl {
        decks: [deck; 2],
        master_gain: 0.7,
    };
    for n in 0..20 {
        let inputs = [
            PcmFrame {
                left: 11000 + n * 13,
                right: -4000,
            },
            PcmFrame {
                left: -2000,
                right: 9000 - n * 21,
            },
        ];
        assert_eq!(
            render(&mut wet, control, inputs),
            render(&mut dry, control, inputs)
        );
    }
}

#[test]
fn delay_is_audible_at_exact_output_time_and_pfl_precedes_fader_and_master() {
    let [mut a, mut b] = memory();
    let mut graph = FixedRateRenderGraph::with_fx([&mut a, &mut b]).unwrap();
    let mut deck = active();
    delay(&mut deck, DelayMode::Delay, 1);
    deck.post_gain = 0.0;
    let mut control = config(deck);
    control.master_gain = 0.0;
    let block = impulse(&mut graph, control);
    // The existing pop resampler contributes one frame of startup latency.
    assert_eq!(block.pfl[0][0], PcmFrame::default());
    assert_eq!(block.pfl[0][1].left, 12000);
    assert!(block.pfl[0][49].left >= 11998);
    assert!(block.pfl[0][49].right <= -5998);
    assert!(
        block.pfl[0][2..49]
            .iter()
            .all(|v| *v == PcmFrame::default())
    );
    assert!(block.main.iter().all(|v| *v == PcmFrame::default()));
    assert!(block.pfl[1].iter().all(|v| *v == PcmFrame::default()));
}

#[test]
fn echo_release_rings_while_paused_then_expires_and_clear_kills_tail() {
    let [mut a, mut b] = memory();
    let mut graph = FixedRateRenderGraph::with_fx([&mut a, &mut b]).unwrap();
    let mut deck = active();
    delay(&mut deck, DelayMode::Echo, 3);
    impulse(&mut graph, config(deck));
    deck.playing = false;
    deck.fx.beat.time.enabled = false;
    let tail = render(&mut graph, config(deck), [PcmFrame::default(); 2]);
    assert!(tail.pfl[0].iter().any(|v| v.left != 0));
    for _ in 0..751 {
        render(&mut graph, config(deck), [PcmFrame::default(); 2]);
    }
    assert!(
        render(&mut graph, config(deck), [PcmFrame::default(); 2])
            .main
            .iter()
            .all(|v| *v == PcmFrame::default())
    );
    deck.playing = true;
    deck.fx.beat.time.enabled = true;
    impulse(&mut graph, config(deck));
    deck.fx.beat.time.enabled = false;
    deck.fx.beat_reset_epoch += 1;
    assert!(
        render(&mut graph, config(deck), [PcmFrame::default(); 2])
            .main
            .iter()
            .all(|v| *v == PcmFrame::default())
    );
}

#[test]
fn pad_echo_snapshot_release_uses_held_pad_identity_and_never_restarts_tail() {
    let [mut a, mut b] = memory();
    let mut graph = FixedRateRenderGraph::with_fx([&mut a, &mut b]).unwrap();
    let mut deck = active();
    deck.fx.pad.mode = PadFxMode::PadFx1;
    deck.fx.pad.pad = 2; // 250 ms Echo
    deck.fx.pad.active = true;
    impulse(&mut graph, config(deck));
    deck.playing = false;
    deck.fx.pad = DeckFxControl::bypass().pad; // source state no longer stores pad 2
    let mut heard_tail = false;
    for i in 0..755 {
        deck.fx.beat_reset_epoch = i / 11; // unrelated Beat Clear must not renew the Pad tail
        let output = render(&mut graph, config(deck), [PcmFrame::default(); 2]);
        heard_tail |= output.pfl[0].iter().any(|v| v.left != 0);
        if i > 751 {
            assert!(output.main.iter().all(|v| *v == PcmFrame::default()));
        }
    }
    assert!(heard_tail);
}

#[test]
fn source_epoch_generation_or_unloaded_binding_cannot_leak_echo_to_next_track() {
    for replacement in 0..3 {
        let [mut a, mut b] = memory();
        let mut graph = FixedRateRenderGraph::with_fx([&mut a, &mut b]).unwrap();
        let mut deck = active();
        delay(&mut deck, DelayMode::Echo, 3);
        impulse(&mut graph, config(deck));
        match replacement {
            0 => deck.source_epoch += 1,
            1 => deck.stream_generation += 1,
            _ => deck.source_epoch = 0,
        }
        deck.fx = DeckFxControl::bypass();
        for _ in 0..20 {
            assert!(
                render(&mut graph, config(deck), [PcmFrame::default(); 2])
                    .main
                    .iter()
                    .all(|v| *v == PcmFrame::default())
            );
        }
    }
}

#[test]
fn target_removal_stops_old_deck_effect_and_other_decks_pfl_is_independent() {
    let [mut a, mut b] = memory();
    let mut graph = FixedRateRenderGraph::with_fx([&mut a, &mut b]).unwrap();
    let mut a_control = active();
    delay(&mut a_control, DelayMode::Echo, 3);
    let b_control = active();
    let mut independent = FixedRateRenderGraph::new();
    impulse(
        &mut independent,
        RenderControl {
            decks: [RenderDeckControl::idle(), b_control],
            master_gain: 1.0,
        },
    );
    impulse(
        &mut graph,
        RenderControl {
            decks: [a_control, b_control],
            master_gain: 1.0,
        },
    );
    a_control.fx.beat_selected = false;
    let control = RenderControl {
        decks: [a_control, b_control],
        master_gain: 1.0,
    };
    let mut output = RenderOutputBlock::silence();
    let report = graph.render_block(control, &mut output, |i| {
        if i == 0 {
            None
        } else {
            Some(PcmFrame {
                left: 3000,
                right: -3000,
            })
        }
    });
    assert!(report.underflow_frames[0] > 0);
    assert_eq!(report.underflow_frames[1], 0);
    assert!(output.pfl[0].iter().all(|v| *v == PcmFrame::default()));
    let baseline = render(
        &mut independent,
        RenderControl {
            decks: [RenderDeckControl::idle(), b_control],
            master_gain: 1.0,
        },
        [
            PcmFrame::default(),
            PcmFrame {
                left: 3000,
                right: -3000,
            },
        ],
    );
    assert_eq!(output.pfl[1], baseline.pfl[1]);
    assert!(
        output.pfl[1][1..]
            .iter()
            .all(|v| v.left == 3000 && v.right == -3000)
    );
}

#[test]
fn beat_filter_flanger_and_both_pad_filter_banks_change_real_pcm() {
    for effect in 0..4 {
        let [mut a, mut b] = memory();
        let mut wet = FixedRateRenderGraph::with_fx([&mut a, &mut b]).unwrap();
        let mut dry = FixedRateRenderGraph::new();
        let mut deck = active();
        match effect {
            0 => {
                deck.fx.beat_selected = true;
                deck.fx.beat.filter.enabled = true;
                deck.fx.beat.filter.raw = 1500;
            }
            1 => {
                deck.fx.beat_selected = true;
                deck.fx.beat.flanger.enabled = true;
                deck.fx.beat.flanger.depth_q15 = 32767;
            }
            _ => {
                deck.fx.pad.active = true;
                deck.fx.pad.mode = if effect == 2 {
                    PadFxMode::PadFx1
                } else {
                    PadFxMode::PadFx2
                };
            }
        }
        let mut changed = false;
        for _ in 0..20 {
            let input = [
                PcmFrame {
                    left: 15000,
                    right: -12000,
                },
                PcmFrame::default(),
            ];
            changed |=
                render(&mut wet, config(deck), input) != render(&mut dry, config(deck), input);
        }
        assert!(changed, "effect={effect}");
    }
}

#[test]
fn pcm_loop_wraps_retain_delay_memory_and_do_not_turn_into_eof() {
    use pajoniiir_audio_runtime::{
        source::{CanonicalSource, IngestResult, SOURCE_HISTORY_FRAMES},
        timeline::FrameLoop,
        wav::{WavPcm16Format, WavPcm16SourceDescriptor},
        wav_stream::WavStreamDecoder,
    };
    let descriptor = WavPcm16SourceDescriptor::new(
        WavPcm16Format {
            channels: 1,
            sample_rate_hz: 48000,
            byte_rate: 96000,
            block_align: 2,
        },
        44,
        128,
    )
    .unwrap();
    let mut stream = WavStreamDecoder::new(descriptor, 7, 0, FrameLoop::new(0, 64, 64)).unwrap();
    let mut data = [0u8; 128];
    data[..2].copy_from_slice(&12000i16.to_le_bytes());
    let [mut fa, mut fb] = memory();
    let mut graph = FixedRateRenderGraph::with_fx([&mut fa, &mut fb]).unwrap();
    let mut ha = vec![PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut hb = vec![PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut sources = [
        CanonicalSource::new(&mut ha).unwrap(),
        CanonicalSource::new(&mut hb).unwrap(),
    ];
    sources[0].reset(7);
    let first = stream.decode(&data).unwrap();
    assert_eq!(sources[0].ingest(&first), IngestResult::Accepted);
    for _ in 0..4 {
        assert_eq!(
            sources[0].ingest(&stream.cached_block().unwrap()),
            IngestResult::Accepted
        );
    }
    let mut deck = active();
    deck.stream_generation = 7;
    delay(&mut deck, DelayMode::Delay, 2); // 96 output frames, crosses a 64-frame wrap
    let mut block = RenderOutputBlock::silence();
    let report = graph.render_canonical_block(config(deck), &mut block, &mut sources);
    assert_eq!(report.underflow_frames[0], 0);
    assert!(block.pfl[0][96].left > 11000);
    let progress = sources[0].take_progress(deck.source_control()).unwrap();
    assert_eq!(progress.end_frame, 0);
    assert!(!progress.eof);
}

#[test]
fn canonical_source_fault_hard_mutes_effect_history() {
    use pajoniiir_audio_runtime::{
        pcm_stream::PcmStreamBlock,
        source::{CanonicalSource, SOURCE_HISTORY_FRAMES},
    };
    let [mut fa, mut fb] = memory();
    let mut graph = FixedRateRenderGraph::with_fx([&mut fa, &mut fb]).unwrap();
    let mut ha = vec![PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut hb = vec![PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut sources = [
        CanonicalSource::new(&mut ha).unwrap(),
        CanonicalSource::new(&mut hb).unwrap(),
    ];
    sources[0].reset(1);
    let mut pcm = PcmStreamBlock::empty(1, 0, 48000);
    pcm.set_len(256);
    pcm.frames[0].left = 12000;
    sources[0].ingest(&pcm);
    let mut deck = active();
    delay(&mut deck, DelayMode::Echo, 3);
    let mut block = RenderOutputBlock::silence();
    graph.render_canonical_block(config(deck), &mut block, &mut sources);
    let mut fault = PcmStreamBlock::empty(1, 256, 48000);
    fault.mark_faulted();
    sources[0].ingest(&fault);
    graph.render_canonical_block(config(deck), &mut block, &mut sources);
    assert!(block.main.iter().all(|v| *v == PcmFrame::default()));
}

#[test]
fn fx_storage_is_explicit_and_limited_without_render_time_allocation() {
    let mut short = vec![0.0; FX_DECK_STORAGE_SAMPLES - 1];
    let mut full = vec![0.0; FX_DECK_STORAGE_SAMPLES];
    assert!(FixedRateRenderGraph::with_fx([&mut short, &mut full]).is_err());
    assert_eq!(FX_TOTAL_STORAGE_SAMPLES * 4, 1_540_704);
    let [mut a, mut b] = memory();
    let graph = FixedRateRenderGraph::with_fx([&mut a, &mut b]).unwrap();
    assert!(graph.has_fx_storage());
}

#[test]
fn semantic_product_snapshot_drives_wet_pcm_and_controller_loss_releases_pad() {
    use pajoniiir_audio_runtime::fx::product_fx_controls;
    use pajoniiir_controller_core::{
        BeatFxTarget, ControlEvent, ControlValue, DeckId, PadAction, PadMode, SemanticControl,
    };
    use pajoniiir_deck::DeckProductState;
    let mut product = DeckProductState::new();
    product.set_controller_connected(true);
    let pressed = |control| ControlEvent {
        deck: None,
        control,
        value: ControlValue::Pressed(true),
    };
    product.handle_control(pressed(SemanticControl::BeatFxSelectNext)); // Filter -> Echo
    product.handle_control(pressed(SemanticControl::BeatFxOn));
    product.handle_control(ControlEvent {
        deck: None,
        control: SemanticControl::BeatFxTarget,
        value: ControlValue::BeatFxTarget(BeatFxTarget::ChannelOne),
    });
    product.handle_control(ControlEvent {
        deck: Some(DeckId::Two),
        control: SemanticControl::PadAction,
        value: ControlValue::PadAction(PadAction {
            mode: PadMode::PadFx2,
            pad: 0,
            shifted: false,
            pressed: true,
        }),
    });
    let fx = product_fx_controls(&product);
    assert!(fx[0].beat_selected && fx[0].beat.time.enabled);
    assert_eq!(fx[0].beat.time.delay_ms, 500); // authoritative 120 BPM timing
    assert!(!fx[1].beat_selected && !fx[1].beat.time.enabled);
    assert!(fx[1].pad.active);
    assert_eq!(fx[1].pad.mode, PadFxMode::PadFx2);
    let [mut a, mut b] = memory();
    let mut graph = FixedRateRenderGraph::with_fx([&mut a, &mut b]).unwrap();
    let mut decks = [active(); 2];
    for i in 0..2 {
        decks[i].fx = fx[i];
    }
    let input = [PcmFrame {
        left: 5000,
        right: -4000,
    }; 2];
    let output = render(
        &mut graph,
        RenderControl {
            decks,
            master_gain: 1.0,
        },
        input,
    );
    assert_ne!(output.pfl[0], output.pfl[1]);
    product.set_controller_connected(false);
    let released = product_fx_controls(&product);
    assert!(!released[1].pad.active);
    assert_eq!(released[0].beat, fx[0].beat);
    product.handle_control(pressed(SemanticControl::BeatFxClear));
    let cleared = product_fx_controls(&product);
    assert_ne!(cleared[0].beat_reset_epoch, released[0].beat_reset_epoch);
    assert!(!cleared[0].beat.time.enabled && !cleared[1].beat.time.enabled);
}
