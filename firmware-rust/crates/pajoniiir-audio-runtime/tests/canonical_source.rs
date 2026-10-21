use pajoniiir_audio_dsp::PcmFrame;
use pajoniiir_audio_runtime::{
    pcm_stream::PcmStreamBlock,
    render::{FixedRateRenderGraph, RenderControl, RenderDeckControl, RenderOutputBlock},
    source::{CanonicalSource, IngestResult, SOURCE_HISTORY_FRAMES, SourceControl},
};
use pajoniiir_audio_scratch::ScratchConfig;

fn control() -> SourceControl {
    SourceControl {
        playing: true,
        tempo_factor: 1.0,
        output_sample_rate_hz: 48_000,
        ..SourceControl::default()
    }
}

fn block(generation: u32, first: u64, len: usize, eof: bool, signal: i16) -> PcmStreamBlock {
    let mut block = PcmStreamBlock::empty(generation, first, 48_000);
    assert!(block.set_len(len));
    block.frames_mut()[..len].fill(PcmFrame {
        left: signal,
        right: -signal,
    });
    block.eof = eof;
    block
}

fn prefill(source: &mut CanonicalSource<'_>, generation: u32, first: u64, signal: i16) {
    for index in 0..8 {
        assert_eq!(
            source.ingest(&block(generation, first + index * 256, 256, false, signal)),
            IngestResult::Accepted
        );
    }
}

#[test]
fn prefetch_and_terminal_block_do_not_publish_audible_progress_or_early_eof() {
    let mut storage = [PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut source = CanonicalSource::new(&mut storage).unwrap();
    source.reset(7);
    assert_eq!(
        source.ingest(&block(7, 48_000, 128, true, 1_000)),
        IngestResult::Accepted
    );
    assert_eq!(source.position(), 48_000);
    assert!(source.take_progress(control()).is_none());
    for _ in 0..127 {
        assert!(!source.next_sample(control()).starved);
    }
    let progress = source.take_progress(control()).unwrap();
    assert_eq!(progress.end_frame, 48_127);
    assert!(!progress.eof);
    assert_eq!(source.next_sample(control()).frame.left, 1_000);
    assert!(source.take_progress(control()).unwrap().eof);
}

#[test]
fn paused_prefetch_is_bounded_and_preserves_unconsumed_frames() {
    let mut storage = [PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut source = CanonicalSource::new(&mut storage).unwrap();
    source.reset(1);
    prefill(&mut source, 1, 0, 111);
    let pending = block(1, 2048, 256, false, 222);
    assert_eq!(source.ingest(&pending), IngestResult::Backpressure);
    for _ in 0..256 {
        assert_eq!(source.next_sample(control()).frame.left, 111);
    }
    assert_eq!(source.ingest(&pending), IngestResult::Accepted);
    assert_eq!(source.position(), 256);
}

#[test]
fn reset_and_old_blocks_cannot_leak_old_keylock_or_scratch_audio() {
    let mut storage = [PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut source = CanonicalSource::new(&mut storage).unwrap();
    source.reset(1);
    prefill(&mut source, 1, 0, 3_000);
    let mt = SourceControl {
        master_tempo: true,
        ..control()
    };
    for _ in 0..400 {
        assert!(!source.next_sample(mt).starved);
    }
    source.jog(1, 0, -20);
    source.reset(2);
    assert_eq!(
        source.ingest(&block(1, 2048, 256, false, 3_000)),
        IngestResult::Stale
    );
    assert!(source.next_sample(mt).silent);
    prefill(&mut source, 2, 90_000, -2_000);
    source.jog(1, 0, 100);
    for _ in 0..300 {
        assert_eq!(source.next_sample(mt).frame.left, -2_000);
    }
    assert_eq!(source.position(), 90_300);
    assert_eq!(source.stale_blocks(), 1);
}

#[test]
fn starvation_is_silent_and_recovers_without_advancing_an_unread_source() {
    for sample_rate in [24_000, 44_100, 48_000] {
        for master_tempo in [false, true] {
            let mut storage = [PcmFrame::default(); SOURCE_HISTORY_FRAMES];
            let mut source = CanonicalSource::new(&mut storage).unwrap();
            source.reset(1);
            let mut first = block(1, 0, 4, false, 1_000);
            first.sample_rate_hz = sample_rate;
            assert_eq!(source.ingest(&first), IngestResult::Accepted);
            let mode = SourceControl {
                master_tempo,
                ..control()
            };
            for _ in 0..32 {
                source.next_sample(mode);
            }
            let before = source.position();
            for _ in 0..128 {
                let sample = source.next_sample(mode);
                assert!(sample.silent && sample.starved);
                assert_eq!(source.position(), before);
            }
            for index in 0..4 {
                let mut next = block(1, 4 + index * 256, 256, false, -1_000);
                next.sample_rate_hz = sample_rate;
                assert_eq!(source.ingest(&next), IngestResult::Accepted);
            }
            let mut recovered = false;
            for _ in 0..128 {
                recovered |= source.next_sample(mode).frame.left < 0;
            }
            assert!(recovered);
            assert!(source.position() > before);
        }
    }
}

#[test]
fn scratch_has_priority_over_keylock_while_canonical_timeline_advances_independently() {
    let mut storage = [PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut source = CanonicalSource::new(&mut storage).unwrap();
    source.reset(1);
    for index in 0..8 {
        let mut next = block(1, index * 256, 256, false, 0);
        for (offset, frame) in next.frames_mut().iter_mut().enumerate() {
            frame.left = (100 + index * 256 + offset as u64) as i16;
        }
        assert_eq!(source.ingest(&next), IngestResult::Accepted);
    }
    for _ in 0..256 {
        source.next_sample(control());
    }
    source.configure_scratch(ScratchConfig::sanitized(1.0, 1, 1.0, 6.0, 100));
    source.jog(1, 0, -4);
    let scratch = SourceControl {
        scratch: true,
        master_tempo: true,
        ..control()
    };
    let first = source.next_sample(scratch);
    let second = source.next_sample(scratch);
    assert_eq!(first.frame.left, 356);
    assert_eq!(second.frame.left, 352);
    assert_eq!(source.position(), 258);
    assert!(!second.starved && !second.deferred);
    let resumed = source.next_sample(SourceControl {
        master_tempo: true,
        ..control()
    });
    assert_eq!(resumed.frame.left, 348);
    assert_eq!(source.position(), 249);
}

#[test]
fn paused_scratch_does_not_advance_slip_time_and_release_clears_motion() {
    let mut storage = [PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut source = CanonicalSource::new(&mut storage).unwrap();
    source.reset(1);
    prefill(&mut source, 1, 0, 900);
    for _ in 0..200 {
        source.next_sample(control());
    }
    source.configure_scratch(ScratchConfig::sanitized(1.0, 1, 1.0, 6.0, 100));
    source.jog(1, 0, -2);
    let scratch = SourceControl {
        playing: false,
        scratch: true,
        ..control()
    };
    assert_eq!(source.next_sample(scratch).frame.left, 900);
    assert_eq!(source.position(), 200);
    source.next_sample(SourceControl {
        playing: false,
        ..control()
    });
    assert_eq!(source.next_sample(scratch).frame, PcmFrame::default());
    assert_eq!(source.position(), 198);
}

#[test]
fn scratch_prefetch_reference_shift_preserves_the_absolute_head() {
    let mut storage = [PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut source = CanonicalSource::new(&mut storage).unwrap();
    source.reset(1);
    prefill(&mut source, 1, 0, 1_234);
    for _ in 0..256 {
        source.next_sample(control());
    }
    source.configure_scratch(ScratchConfig::sanitized(1.0, 1, 1.0, 6.0, 100));
    source.jog(1, 0, -1);
    let scratch = SourceControl {
        scratch: true,
        ..control()
    };
    assert_eq!(source.next_sample(scratch).frame.left, 1_234);
    assert_eq!(
        source.ingest(&block(1, 2048, 256, false, -9_000)),
        IngestResult::Accepted
    );
    assert_eq!(source.next_sample(scratch).frame.left, 1_234);
}

#[test]
fn censor_reverse_stops_at_history_edge_and_resumes_the_canonical_cursor() {
    let mut storage = [PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut source = CanonicalSource::new(&mut storage).unwrap();
    source.reset(1);
    prefill(&mut source, 1, 0, 700);
    for _ in 0..4 {
        source.next_sample(control());
    }
    let censor = SourceControl {
        censor: true,
        ..control()
    };
    for _ in 0..4 {
        assert_eq!(source.next_sample(censor).frame.left, 700);
    }
    assert!(source.next_sample(censor).silent);
    assert_eq!(source.position(), 9);
    assert_eq!(source.next_sample(control()).frame.left, 700);
    assert_eq!(source.position(), 10);
}

#[test]
fn scratch_at_eof_defers_completion_until_release() {
    let mut storage = [PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut source = CanonicalSource::new(&mut storage).unwrap();
    source.reset(1);
    assert_eq!(
        source.ingest(&block(1, 0, 4, true, 700)),
        IngestResult::Accepted
    );
    for _ in 0..2 {
        source.next_sample(control());
    }
    let scratch = SourceControl {
        scratch: true,
        ..control()
    };
    for _ in 0..2 {
        source.next_sample(scratch);
    }
    assert!(!source.take_progress(scratch).unwrap().eof);
    source.next_sample(control());
    assert!(!source.take_progress(control()).unwrap().eof);
    source.next_sample(control());
    assert!(source.take_progress(control()).unwrap().eof);
}

#[test]
fn malformed_current_generation_faults_and_stale_input_cannot_clear_it() {
    let mut storage = [PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut source = CanonicalSource::new(&mut storage).unwrap();
    source.reset(2);
    prefill(&mut source, 2, 0, 800);
    let mut invalid = block(2, 2048, 1, false, 900);
    invalid.len = 257;
    assert_eq!(source.ingest(&invalid), IngestResult::Fault);
    assert!(source.next_sample(control()).silent);
    assert!(source.take_progress(control()).unwrap().faulted);
    assert_eq!(
        source.ingest(&block(1, 2048, 10, false, 500)),
        IngestResult::Stale
    );
    assert!(source.next_sample(control()).silent);
}

#[test]
fn live_keylock_survives_history_wrap_and_coordinate_rebase() {
    for sample_rate in [24_000, 44_100, 48_000, 96_000] {
        for tempo_factor in [0.5, 1.0, 1.5, 2.0] {
            let mut storage = [PcmFrame::default(); SOURCE_HISTORY_FRAMES];
            let mut source = CanonicalSource::new(&mut storage).unwrap();
            source.reset(1);
            let mut writer = 1_000_000_000_000u64;
            let mode = SourceControl {
                master_tempo: true,
                tempo_factor,
                ..control()
            };
            for _ in 0..400 {
                for _ in 0..8 {
                    let mut next = block(1, writer, 256, false, 2_000);
                    next.sample_rate_hz = sample_rate;
                    if source.ingest(&next) == IngestResult::Backpressure {
                        break;
                    }
                    writer += 256;
                }
                for _ in 0..128 {
                    let sample = source.next_sample(mode);
                    assert!(
                        !sample.starved && !sample.deferred,
                        "rate={sample_rate} tempo={tempo_factor}"
                    );
                    assert!((sample.frame.left - 2_000).abs() <= 1);
                }
            }
            let expected = 1_000_000_000_000
                + (51_200.0 * sample_rate as f64 / 48_000.0 * tempo_factor as f64) as u64;
            assert!(source.position().abs_diff(expected) <= 1);
        }
    }
}

#[test]
fn live_render_keeps_other_deck_and_pfl_independent_during_starvation_and_mode_changes() {
    let mut a = [PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut b = a;
    let mut sources = [
        CanonicalSource::new(&mut a).unwrap(),
        CanonicalSource::new(&mut b).unwrap(),
    ];
    for source in &mut sources {
        source.reset(1);
    }
    assert_eq!(
        sources[0].ingest(&block(1, 0, 4, false, 1_000)),
        IngestResult::Accepted
    );
    prefill(&mut sources[1], 1, 0, 200);
    let mut deck = RenderDeckControl::idle();
    deck.playing = true;
    deck.stream_generation = 1;
    deck.master_tempo = true;
    deck.pfl_enabled = true;
    let mut graph = FixedRateRenderGraph::new();
    let mut output = RenderOutputBlock::default();
    let report = graph.render_canonical_block(
        RenderControl {
            decks: [deck, deck],
            master_gain: 1.0,
            ..RenderControl::idle()
        },
        &mut output,
        &mut sources,
    );
    assert!(report.underflow_frames[0] > 0);
    assert_eq!(report.underflow_frames[1], 0);
    assert_eq!(report.deferred_mode_frames, [0, 0]);
    assert!(
        output.pfl[0][16..]
            .iter()
            .all(|frame| *frame == PcmFrame::default())
    );
    assert!(output.pfl[1].iter().all(|frame| frame.left == 200));
    assert!(output.main[16..].iter().all(|frame| frame.left == 200));
}

#[test]
fn unsupported_keylock_rates_fail_closed_instead_of_silently_clamping_tempo() {
    let mut storage = [PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut source = CanonicalSource::new(&mut storage).unwrap();
    source.reset(1);
    prefill(&mut source, 1, 0, 900);
    let sample = source.next_sample(SourceControl {
        master_tempo: true,
        tempo_factor: 3.0,
        ..control()
    });
    assert!(sample.deferred && sample.silent);
    assert_eq!(source.position(), 0);
}

#[test]
fn expired_scratch_history_mutes_until_release_without_retargeting_a_ring_slot() {
    let mut storage = [PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut source = CanonicalSource::new(&mut storage).unwrap();
    source.reset(1);
    prefill(&mut source, 1, 0, 1_234);
    let scratch = SourceControl {
        scratch: true,
        ..control()
    };
    source.next_sample(scratch);
    let mut writer = 2048;
    for _ in 0..150 {
        for _ in 0..128 {
            source.next_sample(scratch);
        }
        for _ in 0..2 {
            let next = block(1, writer, 256, false, -8_000);
            if source.ingest(&next) == IngestResult::Backpressure {
                break;
            }
            writer += 256;
        }
    }
    source.jog(1, 0, 10);
    let expired = source.next_sample(scratch);
    assert!(expired.silent && expired.starved);
    let canonical = source.position();
    let resumed = source.next_sample(control());
    assert_eq!(resumed.frame.left, -8_000);
    assert_eq!(source.position(), canonical + 1);
}

#[test]
fn live_master_tempo_preserves_tone_pitch_while_speed_changes_canonical_progress() {
    for master_tempo in [false, true] {
        let mut storage = [PcmFrame::default(); SOURCE_HISTORY_FRAMES];
        let mut source = CanonicalSource::new(&mut storage).unwrap();
        source.reset(1);
        let mut writer = 0;
        let mode = SourceControl {
            master_tempo,
            tempo_factor: 1.5,
            ..control()
        };
        let mut previous = 0;
        let mut crossings = 0;
        for _ in 0..64 {
            for _ in 0..8 {
                let mut next = block(1, writer, 256, false, 0);
                for (index, frame) in next.frames_mut().iter_mut().enumerate() {
                    let phase =
                        (writer + index as u64) as f64 * 440.0 * core::f64::consts::TAU / 48_000.0;
                    frame.left = (libm::sin(phase) * 10_000.0) as i16;
                }
                if source.ingest(&next) == IngestResult::Backpressure {
                    break;
                }
                writer += 256;
            }
            for _ in 0..128 {
                let sample = source.next_sample(mode);
                assert!(!sample.starved && !sample.deferred);
                if previous < 0 && sample.frame.left >= 0 {
                    crossings += 1;
                }
                previous = sample.frame.left;
            }
        }
        let measured_hz = crossings as f64 * 48_000.0 / 8192.0;
        let expected = if master_tempo { 440.0 } else { 660.0 };
        assert!(
            (measured_hz - expected).abs() < 20.0,
            "master_tempo={master_tempo}: {measured_hz} Hz"
        );
        assert_eq!(source.position(), 12_288);
    }
}

#[test]
fn uncommitted_stream_and_stale_product_control_cannot_render_new_pcm() {
    let mut a = [PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut b = a;
    let mut sources = [
        CanonicalSource::new(&mut a).unwrap(),
        CanonicalSource::new(&mut b).unwrap(),
    ];
    sources[0].reset(2);
    prefill(&mut sources[0], 2, 0, -3_000);
    let mut deck = RenderDeckControl::idle();
    deck.playing = true;
    let mut graph = FixedRateRenderGraph::new();
    let mut output = RenderOutputBlock::default();
    for generation in [0, 1] {
        deck.stream_generation = generation;
        graph.render_canonical_block(
            RenderControl {
                decks: [deck, RenderDeckControl::idle()],
                master_gain: 1.0,
                ..RenderControl::idle()
            },
            &mut output,
            &mut sources,
        );
        assert!(
            output
                .main
                .iter()
                .all(|frame| *frame == PcmFrame::default())
        );
        assert_eq!(sources[0].position(), 0);
    }
    deck.stream_generation = 2;
    graph.render_canonical_block(
        RenderControl {
            decks: [deck, RenderDeckControl::idle()],
            master_gain: 1.0,
            ..RenderControl::idle()
        },
        &mut output,
        &mut sources,
    );
    assert!(output.main.iter().all(|frame| frame.left == -3_000));
    assert_eq!(sources[0].position(), 128);
}

#[test]
fn a_new_gesture_resets_scratch_velocity_and_rejects_a_delayed_old_gesture() {
    let mut storage = [PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut source = CanonicalSource::new(&mut storage).unwrap();
    source.reset(1);
    prefill(&mut source, 1, 0, 1_000);
    for _ in 0..256 {
        source.next_sample(control());
    }
    source.configure_scratch(ScratchConfig::sanitized(1.0, 1, 1.0, 6.0, 100));
    source.jog(1, 10, -4);
    let first = SourceControl {
        scratch: true,
        scratch_epoch: 10,
        ..control()
    };
    assert!(!source.next_sample(first).silent);
    let second = SourceControl {
        scratch_epoch: 11,
        ..first
    };
    assert!(source.next_sample(second).silent);
    source.jog(1, 10, -4);
    assert!(source.next_sample(second).silent);
    source.jog(1, 11, -4);
    assert!(!source.next_sample(second).silent);
}
