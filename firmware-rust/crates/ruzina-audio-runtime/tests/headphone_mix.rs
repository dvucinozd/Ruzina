use ruzina_audio_dsp::PcmFrame;
use ruzina_audio_runtime::{
    fx::FX_DECK_STORAGE_SAMPLES,
    playback::PlaybackOutput,
    render::{FixedRateRenderGraph, RenderControl, RenderDeckControl, RenderOutputBlock},
    routing::{ChannelPair, OutputAvailability, OutputRouter, ReadyStream},
    sink::{BlockProgress, SinkBlock, SinkError, SinkRole},
};
use ruzina_controller_core::{ControlEvent, ControlValue, DeckId, SemanticControl};
use ruzina_mixer::{HeadphoneGains, MIXER_CONTROL_MAX, MixerState};

const INPUT: [PcmFrame; 2] = [
    PcmFrame {
        left: 2000,
        right: 6000,
    },
    PcmFrame {
        left: 10000,
        right: 10000,
    },
];
fn press(m: &mut MixerState, deck: Option<DeckId>, control: SemanticControl) {
    m.handle_control(ControlEvent {
        deck,
        control,
        value: ControlValue::Pressed(true),
    });
}
fn raw(m: &mut MixerState, deck: Option<DeckId>, control: SemanticControl, value: u16) {
    m.handle_control(ControlEvent {
        deck,
        control,
        value: ControlValue::Absolute {
            value,
            max: MIXER_CONTROL_MAX,
        },
    });
}
fn control(m: &MixerState) -> RenderControl {
    let gains = m.stage_gains();
    RenderControl {
        decks: core::array::from_fn(|i| {
            let deck = m.deck(if i == 0 { DeckId::One } else { DeckId::Two });
            RenderDeckControl {
                source_epoch: 1,
                stream_generation: i as u32 + 1,
                playing: true,
                pre_gain: gains.pre[i],
                post_gain: gains.post[i],
                pfl_enabled: deck.pfl_enabled,
                ..RenderDeckControl::idle()
            }
        }),
        master_gain: gains.master,
        headphones: m.headphone_gains(),
    }
}
fn render(
    g: &mut FixedRateRenderGraph<'_>,
    c: RenderControl,
    input: [PcmFrame; 2],
) -> RenderOutputBlock {
    let mut out = RenderOutputBlock::silence();
    g.render_block(c, &mut out, |i| Some(input[i]));
    out
}
fn last(c: RenderControl, input: [PcmFrame; 2]) -> RenderOutputBlock {
    render(&mut FixedRateRenderGraph::new(), c, input)
}
fn sample(out: &RenderOutputBlock) -> PcmFrame {
    out.headphones[127]
}

#[test]
fn default_monitor_is_stereo_master_and_headphone_controls_cannot_change_main() {
    let mut m = MixerState::new();
    let baseline = last(control(&m), INPUT);
    assert_eq!(baseline.headphones, baseline.main);
    assert_eq!(
        sample(&baseline),
        PcmFrame {
            left: 12000,
            right: 16000
        }
    );
    raw(&mut m, None, SemanticControl::HeadphoneLevel, 0);
    let silent = last(control(&m), INPUT);
    assert_eq!(silent.main, baseline.main);
    assert!(silent.headphones.iter().all(|s| *s == PcmFrame::default()));
    raw(
        &mut m,
        None,
        SemanticControl::HeadphoneLevel,
        MIXER_CONTROL_MAX,
    );
    press(&mut m, None, SemanticControl::MasterCue);
    let off = last(control(&m), INPUT);
    assert_eq!(off.main, baseline.main);
    assert!(off.headphones.iter().all(|s| *s == PcmFrame::default()));
}

#[test]
fn each_pfl_and_both_pfl_sum_before_fader_crossfader_and_master() {
    let mut m = MixerState::new();
    raw(&mut m, None, SemanticControl::HeadphoneMix, 0);
    raw(&mut m, None, SemanticControl::MasterVolume, 0);
    raw(&mut m, Some(DeckId::One), SemanticControl::ChannelVolume, 0);
    raw(&mut m, Some(DeckId::Two), SemanticControl::ChannelVolume, 0);
    assert_eq!(sample(&last(control(&m), INPUT)), PcmFrame::default());
    press(&mut m, Some(DeckId::One), SemanticControl::Pfl);
    assert_eq!(
        sample(&last(control(&m), INPUT)),
        PcmFrame {
            left: 4000,
            right: 4000
        }
    );
    press(&mut m, Some(DeckId::Two), SemanticControl::Pfl);
    let both = last(control(&m), INPUT);
    assert_eq!(
        sample(&both),
        PcmFrame {
            left: 14000,
            right: 14000
        }
    );
    assert!(both.main.iter().all(|s| *s == PcmFrame::default()));
    press(&mut m, Some(DeckId::One), SemanticControl::Pfl);
    for xf in [0, 8192, MIXER_CONTROL_MAX] {
        raw(&mut m, None, SemanticControl::Crossfader, xf);
        assert_eq!(
            sample(&last(control(&m), INPUT)),
            PcmFrame {
                left: 10000,
                right: 10000
            }
        );
    }
}

#[test]
fn midpoint_matches_c_monitor_fixture_and_level_scales_the_complete_mix() {
    let mut m = MixerState::new();
    raw(&mut m, Some(DeckId::Two), SemanticControl::ChannelVolume, 0);
    press(&mut m, Some(DeckId::Two), SemanticControl::Pfl);
    raw(&mut m, None, SemanticControl::HeadphoneMix, 8192);
    let mix = last(control(&m), INPUT);
    assert_eq!(mix.main[127], INPUT[0]);
    assert_eq!(
        sample(&mix),
        PcmFrame {
            left: 6000,
            right: 8000
        }
    );
    raw(&mut m, None, SemanticControl::HeadphoneLevel, 8192);
    let half = last(control(&m), INPUT);
    assert_eq!(half.main, mix.main);
    assert_eq!(
        sample(&half),
        PcmFrame {
            left: 3000,
            right: 4000
        }
    );
    press(&mut m, None, SemanticControl::MasterCue);
    let cue = last(control(&m), INPUT);
    assert_eq!(
        sample(&cue),
        PcmFrame {
            left: 2500,
            right: 2500
        }
    );
}

#[test]
fn cue_is_summed_with_float_headroom_then_attenuated_before_pcm_conversion() {
    let mut c = control(&MixerState::new());
    c.headphones = HeadphoneGains {
        cue: 1.0,
        master: 0.0,
        level: 0.25,
    };
    for deck in &mut c.decks {
        deck.pfl_enabled = true;
        deck.pre_gain = 2.0;
        deck.post_gain = 0.0;
    }
    let input = [PcmFrame {
        left: 30000,
        right: 30000,
    }; 2];
    let out = last(c, input);
    assert_eq!(
        sample(&out),
        PcmFrame {
            left: 30000,
            right: 30000
        }
    );
    assert_eq!(out.pfl[0][127].left, i16::MAX); // diagnostic tap may saturate
    c.headphones.level = 1.0;
    assert_eq!(
        sample(&last(c, input)),
        PcmFrame {
            left: i16::MAX,
            right: i16::MAX
        }
    );
    assert!(out.main.iter().all(|s| *s == PcmFrame::default()));
    let cancelling = [
        input[0],
        PcmFrame {
            left: -30000,
            right: -30000,
        },
    ];
    assert_eq!(sample(&last(c, cancelling)), PcmFrame::default());
}

#[test]
fn mono_cue_downmix_handles_opposite_stereo_polarity() {
    let mut c = control(&MixerState::new());
    c.decks[0].pfl_enabled = true;
    c.headphones = HeadphoneGains {
        cue: 1.0,
        master: 0.0,
        level: 1.0,
    };
    let out = last(
        c,
        [
            PcmFrame {
                left: 10000,
                right: -10000,
            },
            PcmFrame::default(),
        ],
    );
    assert_eq!(sample(&out), PcmFrame::default());
    assert_eq!(
        out.main[127],
        PcmFrame {
            left: 10000,
            right: -10000
        }
    );
}

#[test]
fn monitor_master_uses_limited_main_and_cue_overload_does_not_pollute_main_stats() {
    let mut c = control(&MixerState::new());
    let mut out = RenderOutputBlock::silence();
    let mut graph = FixedRateRenderGraph::new();
    let loud = PcmFrame {
        left: 30000,
        right: -30000,
    };
    let report = graph.render_block(c, &mut out, |_| Some(loud));
    assert!(report.limiter.limited_samples > 0);
    assert_eq!(out.headphones, out.main);
    c.headphones = HeadphoneGains {
        cue: 1.0,
        master: 0.0,
        level: 1.0,
    };
    for deck in &mut c.decks {
        deck.pfl_enabled = true;
        deck.post_gain = 0.0;
    }
    let report = graph.render_block(c, &mut out, |_| {
        Some(PcmFrame {
            left: 30000,
            right: 30000,
        })
    });
    assert_eq!(report.limiter.limited_samples, 0);
    assert_eq!(
        sample(&out),
        PcmFrame {
            left: i16::MAX,
            right: i16::MAX
        }
    );
}

#[test]
fn invalid_monitor_gains_fail_closed_and_finite_out_of_range_values_are_bounded() {
    let mut c = control(&MixerState::new());
    c.decks[0].pfl_enabled = true;
    c.headphones = HeadphoneGains {
        cue: f32::NAN,
        master: f32::INFINITY,
        level: 1.0,
    };
    assert_eq!(sample(&last(c, INPUT)), PcmFrame::default());
    c.headphones = HeadphoneGains {
        cue: 1.0,
        master: 1.0,
        level: f32::INFINITY,
    };
    assert_eq!(sample(&last(c, INPUT)), PcmFrame::default());
    c.headphones = HeadphoneGains {
        cue: 2.0,
        master: -2.0,
        level: 2.0,
    };
    assert_eq!(
        sample(&last(c, INPUT)),
        PcmFrame {
            left: 4000,
            right: 4000
        }
    );
}

#[test]
fn starved_or_paused_deck_does_not_suppress_other_deck_cue() {
    let mut c = control(&MixerState::new());
    c.headphones = HeadphoneGains {
        cue: 1.0,
        master: 0.0,
        level: 1.0,
    };
    c.decks.iter_mut().for_each(|d| d.pfl_enabled = true);
    let mut graph = FixedRateRenderGraph::new();
    render(&mut graph, c, INPUT);
    let mut out = RenderOutputBlock::silence();
    let report = graph.render_block(c, &mut out, |i| if i == 0 { None } else { Some(INPUT[1]) });
    assert_eq!(report.underflow_frames[0], 128);
    assert_eq!(report.underflow_frames[1], 0);
    assert!(out.headphones.iter().all(|s| *s == INPUT[1]));
    c.decks[0].playing = false;
    let out = render(&mut graph, c, INPUT);
    assert!(out.headphones.iter().all(|s| *s == INPUT[1]));
}

#[test]
fn wet_echo_reaches_headphone_mix_with_main_fader_closed_and_clears_on_source_change() {
    let mut a = vec![0.0; FX_DECK_STORAGE_SAMPLES];
    let mut b = vec![0.0; FX_DECK_STORAGE_SAMPLES];
    let mut graph = FixedRateRenderGraph::with_fx([&mut a, &mut b]).unwrap();
    let mut c = control(&MixerState::new());
    c.decks[1].playing = false;
    c.decks[0].post_gain = 0.0;
    c.decks[0].pfl_enabled = true;
    c.headphones = HeadphoneGains {
        cue: 1.0,
        master: 0.0,
        level: 1.0,
    };
    let fx = &mut c.decks[0].fx;
    fx.beat_selected = true;
    fx.beat.time.enabled = true;
    fx.beat.time.delay_ms = 3;
    fx.beat.time.wet_q15 = 32767;
    let mut first = true;
    let mut out = RenderOutputBlock::silence();
    graph.render_block(c, &mut out, |_| {
        let v = first;
        first = false;
        Some(if v { INPUT[1] } else { PcmFrame::default() })
    });
    c.decks[0].playing = false;
    c.decks[0].fx.beat.time.enabled = false;
    let tail = render(&mut graph, c, [PcmFrame::default(); 2]);
    assert!(tail.headphones.iter().any(|s| s.left > 9000));
    assert!(tail.main.iter().all(|s| *s == PcmFrame::default()));
    c.decks[0].source_epoch += 1;
    let reset = render(&mut graph, c, [PcmFrame::default(); 2]);
    assert!(reset.headphones.iter().all(|s| *s == PcmFrame::default()));
}

fn stream(epoch: u32, pair: ChannelPair) -> ReadyStream {
    ReadyStream {
        connection_epoch: epoch,
        stream_epoch: 1,
        interface: 1,
        channels: 4,
        pair,
        sample_rate_hz: 48000,
        sample_bits: 16,
    }
}
fn available(epoch: u32, dac: bool) -> OutputAvailability {
    OutputAvailability {
        external_dac: dac.then(|| stream(99, ChannelPair { left: 0, right: 1 })),
        controller_main: Some(stream(epoch, ChannelPair { left: 0, right: 1 })),
        controller_headphones: Some(stream(epoch, ChannelPair { left: 2, right: 3 })),
    }
}
fn ready(r: &mut OutputRouter, a: OutputAvailability) {
    let pending = r.update(a).unwrap();
    assert!(r.commit_prepared(pending));
}

#[test]
fn composed_monitor_pcm_survives_main_dac_switch_and_controller_detach_revokes_it() {
    let mut router = OutputRouter::default();
    ready(&mut router, available(1, false));
    let mut main = [SinkBlock::EMPTY; 2];
    let mut phones = [SinkBlock::EMPTY; 2];
    let mut owner = PlaybackOutput::new(&mut main, &mut phones).unwrap();
    owner.sync_routes(&router, 0).unwrap();
    let token = owner.lane(SinkRole::Headphones).token().unwrap();
    owner
        .lane_mut(SinkRole::Headphones)
        .set_mute(token, false, 0)
        .unwrap();
    let mut m = MixerState::new();
    press(&mut m, Some(DeckId::Two), SemanticControl::Pfl);
    raw(&mut m, None, SemanticControl::HeadphoneMix, 8192);
    let block = last(control(&m), INPUT);
    assert_eq!(
        owner.submit_render(owner.render_permit(), 0, &block, BlockProgress::default()),
        [Ok(()); 2]
    );
    let mut received = [PcmFrame::default(); 128];
    let receipt = owner
        .lane_mut(SinkRole::Headphones)
        .issue(token, 0, 48, &mut received)
        .unwrap();
    assert_eq!(received[..48], block.headphones[..48]);
    owner.complete(receipt).unwrap();
    ready(&mut router, available(1, true));
    owner.sync_routes(&router, 0).unwrap();
    assert_eq!(owner.lane(SinkRole::Headphones).token(), Some(token));
    let receipt = owner
        .lane_mut(SinkRole::Headphones)
        .issue(token, 48, 80, &mut received)
        .unwrap();
    assert_eq!(received[..80], block.headphones[48..]);
    let mut detached = available(1, true);
    detached.controller_main = None;
    detached.controller_headphones = None;
    ready(&mut router, detached);
    owner.sync_routes(&router, 128).unwrap();
    assert!(owner.lane(SinkRole::Headphones).token().is_none());
    assert!(owner.lane(SinkRole::Main).token().is_some());
    assert_eq!(owner.complete(receipt), Err(SinkError::Stale));
    ready(&mut router, available(2, true));
    owner.sync_routes(&router, 128).unwrap();
    let fresh = owner.lane(SinkRole::Headphones).token().unwrap();
    assert_ne!(fresh, token);
    owner
        .lane_mut(SinkRole::Headphones)
        .set_mute(fresh, false, 0)
        .unwrap();
    assert_eq!(owner.lane(SinkRole::Headphones).queued_blocks(), 0);
    assert_eq!(
        owner.submit_render(owner.render_permit(), 128, &block, BlockProgress::default()),
        [Ok(()); 2]
    );
    let receipt = owner
        .lane_mut(SinkRole::Headphones)
        .issue(fresh, 128, 128, &mut received)
        .unwrap();
    assert_eq!(received, block.headphones);
    owner.complete(receipt).unwrap();
}

#[test]
fn headphone_only_canonical_playback_confirms_progress_after_the_last_packet() {
    use ruzina_audio_runtime::{
        pcm_stream::PcmStreamBlock,
        source::{CanonicalSource, IngestResult, SOURCE_HISTORY_FRAMES},
    };
    let mut router = OutputRouter::default();
    let mut availability = available(1, false);
    availability.controller_main = None;
    ready(&mut router, availability);
    let mut main = [SinkBlock::EMPTY; 1];
    let mut phones = [SinkBlock::EMPTY; 1];
    let mut owner = PlaybackOutput::new(&mut main, &mut phones).unwrap();
    owner.sync_routes(&router, 0).unwrap();
    owner.sync_sources([1, 2], 0).unwrap();
    assert_eq!(owner.authority(), Some(SinkRole::Headphones));
    let token = owner.lane(SinkRole::Headphones).token().unwrap();
    owner
        .lane_mut(SinkRole::Headphones)
        .set_mute(token, false, 0)
        .unwrap();
    let mut a = vec![PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut b = vec![PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut sources = [
        CanonicalSource::new(&mut a).unwrap(),
        CanonicalSource::new(&mut b).unwrap(),
    ];
    let c = control(&MixerState::new());
    for (i, source) in sources.iter_mut().enumerate() {
        source.reset(i as u32 + 1);
        let mut data = PcmStreamBlock::empty(i as u32 + 1, 0, 48000);
        data.set_len(128);
        data.frames.fill(INPUT[i]);
        data.eof = true;
        assert_eq!(source.ingest(&data), IngestResult::Accepted);
    }
    let mut graph = FixedRateRenderGraph::new();
    let mut block = RenderOutputBlock::silence();
    let report = graph.render_canonical_block(c, &mut block, &mut sources);
    let progress = BlockProgress {
        decks: core::array::from_fn(|i| sources[i].take_progress(c.decks[i].source_control())),
        source_frames: report.source_frames_consumed.map(|v| v as u16),
    };
    assert_eq!(
        owner.submit_render(owner.render_permit(), 0, &block, progress),
        [Err(SinkError::Stale), Ok(())]
    );
    assert!(owner.confirmed_progress(0).is_none());
    let mut received = [PcmFrame::default(); 128];
    for (start, len) in [(0, 48), (48, 48), (96, 32)] {
        let receipt = owner
            .lane_mut(SinkRole::Headphones)
            .issue(token, start, len, &mut received)
            .unwrap();
        assert_eq!(
            received[..len],
            block.headphones[start as usize..start as usize + len]
        );
        assert!(owner.confirmed_progress(0).is_none());
        owner.complete(receipt).unwrap();
    }
    for i in 0..2 {
        let confirmed = owner.confirmed_progress(i).unwrap();
        assert_eq!(confirmed.end_frame, 128);
        assert!(confirmed.eof);
    }
}
