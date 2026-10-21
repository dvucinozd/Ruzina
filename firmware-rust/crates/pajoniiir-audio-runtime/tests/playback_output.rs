use pajoniiir_audio_dsp::PcmFrame;
use pajoniiir_audio_runtime::{
    pcm_stream::{PcmStreamProgress, PcmTransportCursor},
    playback::PlaybackOutput,
    render::RenderOutputBlock,
    routing::{ChannelPair, OutputAvailability, OutputRouter, ReadyStream},
    sink::{BlockProgress, MAX_IN_FLIGHT_RECEIPTS, SinkBlock, SinkError, SinkRole},
};

#[test]
fn generation_replacement_keeps_other_decks_confirmed_position_and_rejects_bad_marks() {
    let mut a = [SinkBlock::EMPTY; 1];
    let mut b = [SinkBlock::EMPTY; 1];
    let mut owner = PlaybackOutput::new_reference(&mut a, &mut b).unwrap();
    owner.sync_sources([1, 2], 0).unwrap();
    let mut both = mark(1, 128, false);
    both.decks[1] = Some(PcmStreamProgress {
        generation: 2,
        end_frame: 256,
        sample_rate_hz: 96_000,
        eof: false,
        faulted: false,
    });
    both.source_frames[1] = 256;
    submit(&mut owner, 0, both);
    let token = owner.lane(SinkRole::Main).token().unwrap();
    let mut output = pcm();
    let receipt = owner
        .lane_mut(SinkRole::Main)
        .issue(token, 0, 128, &mut output)
        .unwrap();
    owner.complete(receipt).unwrap();
    assert_eq!(owner.sync_sources([3, 2], 0), Err(SinkError::Order));
    owner.sync_sources([3, 2], 128).unwrap();
    assert!(owner.confirmed_progress(0).is_none());
    assert_eq!(owner.confirmed_progress(1).unwrap().end_frame, 256);
    assert_eq!(owner.pending_progress(1).unwrap().1, 256);
    assert_eq!(
        owner.submit_render(owner.render_permit(), 128, &render(), both),
        [Err(SinkError::Stale); 2]
    );
    assert_eq!(owner.lane(SinkRole::Main).queued_blocks(), 0);
}

#[test]
fn canonical_eof_and_seek_replacement_follow_completed_pcm_not_prefetch() {
    use pajoniiir_audio_runtime::{
        pcm_stream::PcmStreamBlock,
        render::{FixedRateRenderGraph, RenderControl, RenderDeckControl},
        source::{CanonicalSource, IngestResult, SOURCE_HISTORY_FRAMES},
    };
    let mut history_a = [PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut history_b = [PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut sources = [
        CanonicalSource::new(&mut history_a).unwrap(),
        CanonicalSource::new(&mut history_b).unwrap(),
    ];
    sources[0].reset(1);
    let mut data = PcmStreamBlock::empty(1, 0, 48_000);
    data.set_len(200);
    data.eof = true;
    data.frames.fill(PcmFrame {
        left: 1_000,
        right: -1_000,
    });
    assert_eq!(sources[0].ingest(&data), IngestResult::Accepted);
    let mut control = RenderControl {
        decks: [
            RenderDeckControl {
                playing: true,
                stream_generation: 1,
                ..RenderDeckControl::idle()
            },
            RenderDeckControl::idle(),
        ],
        master_gain: 1.0,
        ..RenderControl::idle()
    };
    let mut a = [SinkBlock::EMPTY; 2];
    let mut b = [SinkBlock::EMPTY; 1];
    let mut owner = PlaybackOutput::new_reference(&mut a, &mut b).unwrap();
    owner.sync_sources([1, 0], 0).unwrap();
    let mut graph = FixedRateRenderGraph::new();
    let mut block = RenderOutputBlock::silence();
    let mut output = pcm();
    for first in [0, 128] {
        let permit = owner.render_permit();
        let report = graph.render_canonical_block(control, &mut block, &mut sources);
        let progress = BlockProgress {
            decks: [
                sources[0].take_progress(control.decks[0].source_control()),
                None,
            ],
            source_frames: [report.source_frames_consumed[0] as u16, 0],
        };
        assert_eq!(
            owner.submit_render(permit, first, &block, progress)[0],
            Ok(())
        );
    }
    assert_eq!(sources[0].position(), 200);
    assert!(owner.confirmed_progress(0).is_none());
    let old = owner.lane(SinkRole::Main).token().unwrap();
    let first = owner
        .lane_mut(SinkRole::Main)
        .issue(old, 0, 128, &mut output)
        .unwrap();
    owner.complete(first).unwrap();
    assert_eq!(owner.confirmed_progress(0).unwrap().end_frame, 128);
    assert!(!owner.confirmed_progress(0).unwrap().eof);
    let old_eof = owner
        .lane_mut(SinkRole::Main)
        .issue(old, 128, 128, &mut output)
        .unwrap();
    // Seek starts a fresh source at frame 1000. The old EOF is in flight.
    sources[0].reset(2);
    data.generation = 2;
    data.first_frame = 1_000;
    data.set_len(128);
    assert_eq!(sources[0].ingest(&data), IngestResult::Accepted);
    owner.sync_sources([2, 0], 256).unwrap();
    control.decks[0].stream_generation = 2;
    graph.reset_deck(0);
    let permit = owner.render_permit();
    let report = graph.render_canonical_block(control, &mut block, &mut sources);
    let progress = BlockProgress {
        decks: [
            sources[0].take_progress(control.decks[0].source_control()),
            None,
        ],
        source_frames: [report.source_frames_consumed[0] as u16, 0],
    };
    assert_eq!(
        owner.submit_render(permit, 256, &block, progress)[0],
        Ok(())
    );
    assert_eq!(owner.complete(old_eof), Err(SinkError::Stale));
    assert!(owner.pending_progress(0).is_none());
    let new = owner.lane(SinkRole::Main).token().unwrap();
    let new_eof = owner
        .lane_mut(SinkRole::Main)
        .issue(new, 256, 128, &mut output)
        .unwrap();
    owner.complete(new_eof).unwrap();
    let (progress, frames) = owner.pending_progress(0).unwrap();
    assert_eq!(
        (progress.generation, progress.end_frame, progress.eof),
        (2, 1_128, true)
    );
    let mut transport = PcmTransportCursor::new();
    transport.bind_seek(2, 1_000, 48_000);
    transport.accept_active_progress(progress, frames, 2);
    assert_eq!(transport.position_frame(), 1_128);
    assert!(transport.ended());
}

fn stream(epoch: u32, left: u8, right: u8) -> ReadyStream {
    ReadyStream {
        connection_epoch: epoch,
        stream_epoch: 1,
        interface: 1,
        channels: 4,
        pair: ChannelPair { left, right },
        sample_rate_hz: 48_000,
        sample_bits: 16,
    }
}
fn controller() -> OutputAvailability {
    OutputAvailability {
        controller_main: Some(stream(1, 0, 1)),
        controller_headphones: Some(stream(1, 2, 3)),
        external_dac: None,
    }
}
fn ready(router: &mut OutputRouter, available: OutputAvailability) {
    let transition = router.update(available).unwrap();
    assert!(router.commit_prepared(transition));
}
fn mark(generation: u32, position: u64, eof: bool) -> BlockProgress {
    BlockProgress {
        decks: [
            Some(PcmStreamProgress {
                generation,
                end_frame: position,
                sample_rate_hz: 48_000,
                eof,
                faulted: false,
            }),
            None,
        ],
        source_frames: [128, 0],
    }
}
fn pcm() -> [PcmFrame; 128] {
    [PcmFrame {
        left: 3_000,
        right: -3_000,
    }; 128]
}
fn render() -> RenderOutputBlock {
    RenderOutputBlock {
        main: pcm(),
        headphones: pcm(),
        ..RenderOutputBlock::silence()
    }
}
fn submit(owner: &mut PlaybackOutput<'_>, first: u64, progress: BlockProgress) {
    let result = owner.submit_render(owner.render_permit(), first, &render(), progress);
    assert_eq!(result[0], Ok(()));
}

#[test]
fn render_prefetch_and_driver_submission_do_not_publish_position_or_eof() {
    let mut a = [SinkBlock::EMPTY; 2];
    let mut b = [SinkBlock::EMPTY; 1];
    let mut owner = PlaybackOutput::new_reference(&mut a, &mut b).unwrap();
    owner.sync_sources([7, 0], 0).unwrap();
    submit(&mut owner, 0, mark(7, 128, false));
    submit(&mut owner, 128, mark(7, 256, true));
    assert!(owner.pending_progress(0).is_none());
    let token = owner.lane(SinkRole::Main).token().unwrap();
    let mut output = pcm();
    let first = owner
        .lane_mut(SinkRole::Main)
        .issue(token, 0, 128, &mut output)
        .unwrap();
    let last = owner
        .lane_mut(SinkRole::Main)
        .issue(token, 128, 128, &mut output)
        .unwrap();
    assert!(owner.confirmed_progress(0).is_none());
    owner.complete(first).unwrap();
    assert_eq!(owner.pending_progress(0).unwrap().0.end_frame, 128);
    assert!(!owner.pending_progress(0).unwrap().0.eof);
    owner.complete(last).unwrap();
    let (progress, frames) = owner.pending_progress(0).unwrap();
    assert_eq!(progress.end_frame, 256);
    assert!(progress.eof);
    assert_eq!(frames, 256);
}

#[test]
fn partial_packets_wait_for_the_last_output_frame_before_publishing_eof() {
    let mut a = [SinkBlock::EMPTY; 1];
    let mut b = [SinkBlock::EMPTY; 1];
    let mut owner = PlaybackOutput::new_reference(&mut a, &mut b).unwrap();
    owner.sync_sources([1, 0], 0).unwrap();
    submit(&mut owner, 0, mark(1, 91, true));
    let token = owner.lane(SinkRole::Main).token().unwrap();
    let mut output = pcm();
    for (first, len) in [(0, 48), (48, 48)] {
        let receipt = owner
            .lane_mut(SinkRole::Main)
            .issue(token, first, len, &mut output)
            .unwrap();
        owner.complete(receipt).unwrap();
        assert!(owner.pending_progress(0).is_none());
    }
    let receipt = owner
        .lane_mut(SinkRole::Main)
        .issue(token, 96, 32, &mut output)
        .unwrap();
    assert!(owner.pending_progress(0).is_none());
    owner.complete(receipt).unwrap();
    assert!(owner.pending_progress(0).unwrap().0.eof);
}

#[test]
fn out_of_order_duplicate_and_pre_reset_receipts_cannot_update_transport() {
    let mut a = [SinkBlock::EMPTY; 2];
    let mut b = [SinkBlock::EMPTY; 1];
    let mut owner = PlaybackOutput::new_reference(&mut a, &mut b).unwrap();
    owner.sync_sources([1, 0], 0).unwrap();
    submit(&mut owner, 0, mark(1, 128, false));
    let token = owner.lane(SinkRole::Main).token().unwrap();
    let mut output = pcm();
    let first = owner
        .lane_mut(SinkRole::Main)
        .issue(token, 0, 64, &mut output)
        .unwrap();
    let last = owner
        .lane_mut(SinkRole::Main)
        .issue(token, 64, 64, &mut output)
        .unwrap();
    assert_eq!(owner.complete(last), Err(SinkError::Order));
    assert!(owner.pending_progress(0).is_none());
    owner.complete(first).unwrap();
    assert_eq!(owner.complete(first), Err(SinkError::Order));
    owner.sync_sources([2, 0], 128).unwrap();
    assert_eq!(owner.complete(last), Err(SinkError::Stale));
    assert!(owner.pending_progress(0).is_none());
}

#[test]
fn source_replacement_revokes_both_mix_queues_permits_and_old_pending_eof() {
    let mut a = [SinkBlock::EMPTY; 2];
    let mut b = [SinkBlock::EMPTY; 2];
    let mut owner = PlaybackOutput::new(&mut a, &mut b).unwrap();
    let mut router = OutputRouter::default();
    ready(&mut router, controller());
    owner.sync_routes(&router, 0).unwrap();
    owner.sync_sources([1, 2], 0).unwrap();
    let permit = owner.render_permit();
    submit(&mut owner, 0, mark(1, 128, true));
    let main = owner.lane(SinkRole::Main).token().unwrap();
    let phones = owner.lane(SinkRole::Headphones).token().unwrap();
    let mut output = pcm();
    let receipt = owner
        .lane_mut(SinkRole::Main)
        .issue(main, 0, 128, &mut output)
        .unwrap();
    owner.complete(receipt).unwrap();
    assert!(owner.pending_progress(0).unwrap().0.eof);
    owner.sync_sources([3, 2], 128).unwrap();
    assert!(owner.pending_progress(0).is_none());
    assert!(owner.confirmed_progress(0).is_none());
    assert_eq!(owner.lane(SinkRole::Headphones).queued_blocks(), 0);
    assert_ne!(owner.lane(SinkRole::Headphones).token(), Some(phones));
    assert_eq!(
        owner.submit_render(permit, 128, &render(), mark(1, 256, true)),
        [Err(SinkError::Stale); 2]
    );
    assert_eq!(
        owner
            .lane_mut(SinkRole::Main)
            .issue(main, 128, 128, &mut output),
        Err(SinkError::Stale)
    );
}

#[test]
fn secondary_headphones_never_double_count_main_progress() {
    let mut a = [SinkBlock::EMPTY; 1];
    let mut b = [SinkBlock::EMPTY; 1];
    let mut owner = PlaybackOutput::new(&mut a, &mut b).unwrap();
    let mut router = OutputRouter::default();
    ready(&mut router, controller());
    owner.sync_routes(&router, 0).unwrap();
    owner.sync_sources([1, 0], 0).unwrap();
    submit(&mut owner, 0, mark(1, 128, false));
    let mut output = pcm();
    for role in [SinkRole::Headphones, SinkRole::Main] {
        let token = owner.lane(role).token().unwrap();
        let receipt = owner
            .lane_mut(role)
            .issue(token, 0, 128, &mut output)
            .unwrap();
        owner.complete(receipt).unwrap();
        if role == SinkRole::Headphones {
            assert!(owner.pending_progress(0).is_none());
        }
    }
    assert_eq!(owner.pending_progress(0).unwrap().1, 128);
}

#[test]
fn main_loss_uses_controller_headphones_but_no_output_freezes_transport() {
    let mut a = [SinkBlock::EMPTY; 2];
    let mut b = [SinkBlock::EMPTY; 2];
    let mut owner = PlaybackOutput::new(&mut a, &mut b).unwrap();
    let mut router = OutputRouter::default();
    ready(&mut router, controller());
    owner.sync_routes(&router, 0).unwrap();
    owner.sync_sources([1, 0], 0).unwrap();
    submit(&mut owner, 0, mark(1, 128, false));
    let mut output = pcm();
    let token = owner.lane(SinkRole::Headphones).token().unwrap();
    let receipt = owner
        .lane_mut(SinkRole::Headphones)
        .issue(token, 0, 128, &mut output)
        .unwrap();
    ready(
        &mut router,
        OutputAvailability {
            controller_headphones: controller().controller_headphones,
            ..OutputAvailability::default()
        },
    );
    owner.sync_routes(&router, 0).unwrap();
    assert_eq!(owner.authority(), Some(SinkRole::Headphones));
    owner.complete(receipt).unwrap();
    assert_eq!(owner.pending_progress(0).unwrap().0.end_frame, 128);
    ready(&mut router, OutputAvailability::default());
    owner.sync_routes(&router, 128).unwrap();
    assert!(owner.next_block_frame().is_none());
    assert_eq!(owner.authority(), None);
    assert_eq!(owner.confirmed_progress(0).unwrap().end_frame, 128);
}

#[test]
fn delayed_headphone_handover_does_not_rewind_past_the_last_main_checkpoint() {
    let mut a = [SinkBlock::EMPTY; 2];
    let mut b = [SinkBlock::EMPTY; 2];
    let mut owner = PlaybackOutput::new(&mut a, &mut b).unwrap();
    let mut router = OutputRouter::default();
    ready(&mut router, controller());
    owner.sync_routes(&router, 0).unwrap();
    owner.sync_sources([1, 0], 0).unwrap();
    submit(&mut owner, 0, mark(1, 128, false));
    submit(&mut owner, 128, mark(1, 256, false));
    let mut output = pcm();
    let main = owner.lane(SinkRole::Main).token().unwrap();
    for first in [0, 128] {
        let receipt = owner
            .lane_mut(SinkRole::Main)
            .issue(main, first, 128, &mut output)
            .unwrap();
        owner.complete(receipt).unwrap();
    }
    let phones = owner.lane(SinkRole::Headphones).token().unwrap();
    let delayed = owner
        .lane_mut(SinkRole::Headphones)
        .issue(phones, 0, 128, &mut output)
        .unwrap();
    ready(
        &mut router,
        OutputAvailability {
            controller_headphones: controller().controller_headphones,
            ..OutputAvailability::default()
        },
    );
    owner.sync_routes(&router, 256).unwrap();
    owner.complete(delayed).unwrap();
    assert_eq!(owner.confirmed_progress(0).unwrap().end_frame, 256);
    assert_eq!(owner.pending_progress(0).unwrap().1, 256);
}

#[test]
fn sink_starvation_idle_and_pause_do_not_invent_source_progress() {
    let mut a = [SinkBlock::EMPTY; 1];
    let mut b = [SinkBlock::EMPTY; 1];
    let mut owner = PlaybackOutput::new_reference(&mut a, &mut b).unwrap();
    owner.sync_sources([1, 0], 0).unwrap();
    let token = owner.lane(SinkRole::Main).token().unwrap();
    owner
        .lane_mut(SinkRole::Main)
        .set_mute(token, false, 0)
        .unwrap();
    owner
        .lane_mut(SinkRole::Main)
        .expect_active(token, true)
        .unwrap();
    let mut output = pcm();
    let gap = owner
        .lane_mut(SinkRole::Main)
        .issue(token, 0, 128, &mut output)
        .unwrap();
    owner.complete(gap).unwrap();
    assert!(owner.pending_progress(0).is_none());
    submit(&mut owner, 128, mark(1, 128, false));
    let pre_pause = owner
        .lane_mut(SinkRole::Main)
        .issue(token, 128, 128, &mut output)
        .unwrap();
    // Pause applies at the next rendered block; already issued PCM completes.
    owner
        .lane_mut(SinkRole::Main)
        .expect_active(token, false)
        .unwrap();
    owner.complete(pre_pause).unwrap();
    owner.clear_progress(0);
    submit(&mut owner, 256, BlockProgress::default());
    let paused = owner
        .lane_mut(SinkRole::Main)
        .issue(token, 256, 128, &mut output)
        .unwrap();
    owner.complete(paused).unwrap();
    assert!(owner.pending_progress(0).is_none());
    assert_eq!(owner.confirmed_progress(0).unwrap().end_frame, 128);
}

#[test]
fn congestion_coalesces_only_completed_snapshots_and_preserves_vinyl_rewind() {
    let mut a = [SinkBlock::EMPTY; 1];
    let mut b = [SinkBlock::EMPTY; 1];
    let mut owner = PlaybackOutput::new_reference(&mut a, &mut b).unwrap();
    owner.sync_sources([1, 0], 0).unwrap();
    let token = owner.lane(SinkRole::Main).token().unwrap();
    let mut output = pcm();
    for (first, position) in [(0, 1_000), (128, 800)] {
        submit(&mut owner, first, mark(1, position, false));
        let receipt = owner
            .lane_mut(SinkRole::Main)
            .issue(token, first, 128, &mut output)
            .unwrap();
        owner.complete(receipt).unwrap();
    }
    let (progress, frames) = owner.pending_progress(0).unwrap();
    assert_eq!(progress.end_frame, 800);
    assert_eq!(frames, 256);
    let mut transport = PcmTransportCursor::new();
    transport.accept_active_progress(progress, frames, 1);
    assert_eq!(transport.position_frame(), 800);
}

#[test]
fn outstanding_driver_buffers_are_bounded_and_reset_revokes_all_tickets() {
    let mut a = [SinkBlock::EMPTY; 1];
    let mut b = [SinkBlock::EMPTY; 1];
    let mut owner = PlaybackOutput::new_reference(&mut a, &mut b).unwrap();
    owner.sync_sources([1, 0], 0).unwrap();
    let token = owner.lane(SinkRole::Main).token().unwrap();
    let mut output = pcm();
    let mut receipts = Vec::new();
    for first in 0..MAX_IN_FLIGHT_RECEIPTS {
        receipts.push(
            owner
                .lane_mut(SinkRole::Main)
                .issue(token, first as u64, 1, &mut output)
                .unwrap(),
        );
    }
    assert_eq!(
        owner
            .lane_mut(SinkRole::Main)
            .issue(token, 8, 1, &mut output),
        Err(SinkError::InFlightFull)
    );
    assert_eq!(owner.lane(SinkRole::Main).clock_frame(), 8);
    owner.sync_sources([2, 0], 8).unwrap();
    assert_eq!(
        owner.lane(SinkRole::Main).stats().revoked_in_flight_frames,
        8
    );
    for receipt in receipts {
        assert_eq!(owner.complete(receipt), Err(SinkError::Stale));
    }
    assert!(owner.pending_progress(0).is_none());
}

#[test]
fn malformed_source_fault_is_delivered_after_completion_even_without_a_rate() {
    let mut a = [SinkBlock::EMPTY; 1];
    let mut b = [SinkBlock::EMPTY; 1];
    let mut owner = PlaybackOutput::new_reference(&mut a, &mut b).unwrap();
    owner.sync_sources([1, 0], 0).unwrap();
    let mut progress = mark(1, 0, true);
    progress.decks[0].as_mut().unwrap().sample_rate_hz = 0;
    progress.decks[0].as_mut().unwrap().faulted = true;
    progress.source_frames[0] = 0;
    submit(&mut owner, 0, progress);
    assert!(owner.pending_progress(0).is_none());
    let token = owner.lane(SinkRole::Main).token().unwrap();
    let mut output = pcm();
    let receipt = owner
        .lane_mut(SinkRole::Main)
        .issue(token, 0, 128, &mut output)
        .unwrap();
    owner.complete(receipt).unwrap();
    let (confirmed, frames) = owner.pending_progress(0).unwrap();
    assert!(confirmed.faulted && confirmed.eof);
    let mut transport = PcmTransportCursor::new();
    assert!(matches!(
        transport.accept_active_progress(confirmed, frames, 1),
        pajoniiir_audio_runtime::pcm_stream::PcmBlockDisposition::Fault { .. }
    ));
}

#[test]
fn old_completion_cannot_cross_an_owner_restart_with_identical_source_and_clock() {
    let mut a = [SinkBlock::EMPTY; 1];
    let mut b = [SinkBlock::EMPTY; 1];
    let old_receipt = {
        let mut owner = PlaybackOutput::new_reference(&mut a, &mut b).unwrap();
        owner.sync_sources([1, 0], 0).unwrap();
        submit(&mut owner, 0, mark(1, 999, true));
        let token = owner.lane(SinkRole::Main).token().unwrap();
        let mut output = pcm();
        owner
            .lane_mut(SinkRole::Main)
            .issue(token, 0, 128, &mut output)
            .unwrap()
    };
    let mut owner = PlaybackOutput::new_reference(&mut a, &mut b).unwrap();
    owner.sync_sources([1, 0], 0).unwrap();
    submit(&mut owner, 0, mark(1, 128, false));
    let token = owner.lane(SinkRole::Main).token().unwrap();
    let mut output = pcm();
    let current = owner
        .lane_mut(SinkRole::Main)
        .issue(token, 0, 128, &mut output)
        .unwrap();
    assert_eq!(owner.complete(old_receipt), Err(SinkError::Stale));
    assert!(owner.pending_progress(0).is_none());
    owner.complete(current).unwrap();
    assert_eq!(owner.confirmed_progress(0).unwrap().end_frame, 128);
}
