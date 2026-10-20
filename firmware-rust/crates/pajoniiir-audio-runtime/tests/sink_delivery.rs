use pajoniiir_audio_dsp::PcmFrame;
#[path = "../examples/clocked_output.rs"]
mod clocked_output;
use pajoniiir_audio_runtime::{
    render::{RENDER_BLOCK_FRAMES, RenderOutputBlock},
    routing::{ChannelPair, OutputAvailability, OutputRouter, ReadyStream},
    sink::{MAX_SINK_BLOCKS, OutputDelivery, SinkBlock, SinkError, SinkLane, SinkRole},
};

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
fn pcm(value: i16) -> [PcmFrame; RENDER_BLOCK_FRAMES] {
    [PcmFrame {
        left: value,
        right: -value,
    }; RENDER_BLOCK_FRAMES]
}
fn block(value: i16) -> RenderOutputBlock {
    RenderOutputBlock {
        main: pcm(value),
        ..RenderOutputBlock::silence()
    }
}

#[test]
fn partial_usb_packets_and_block_callbacks_consume_the_same_pcm_once() {
    let mut router = OutputRouter::default();
    ready(&mut router, controller());
    let mut storage = [SinkBlock::EMPTY; 2];
    let mut lane = SinkLane::new(SinkRole::Main, &mut storage).unwrap();
    lane.sync_route(&router, 0).unwrap();
    let token = lane.token().unwrap();
    lane.expect_active(token, true).unwrap();
    lane.set_mute(token, false, 0).unwrap();
    lane.submit(token, 0, &pcm(1_000)).unwrap();
    lane.submit(token, 128, &pcm(2_000)).unwrap();
    let mut output = pcm(9_999);
    for first in [0, 48, 96, 144, 192] {
        lane.consume(token, first, 48, &mut output).unwrap();
        for (offset, frame) in output[..48].iter().enumerate() {
            assert_eq!(
                frame.left,
                if first + (offset as u64) < 128 {
                    1_000
                } else {
                    2_000
                }
            );
        }
        assert!(
            output[48..]
                .iter()
                .all(|frame| *frame == PcmFrame::default())
        );
    }
    lane.consume(token, 240, 16, &mut output).unwrap();
    assert_eq!(lane.queued_blocks(), 0);
    assert_eq!(lane.clock_frame(), 256);
    assert_eq!(lane.stats().pcm_frames, 256);
    assert_eq!(lane.stats().underrun_frames, 0);
}

#[test]
fn active_starvation_recovers_without_replaying_pcm_and_idle_is_separate() {
    let mut router = OutputRouter::default();
    ready(&mut router, controller());
    let mut storage = [SinkBlock::EMPTY; 1];
    let mut lane = SinkLane::new(SinkRole::Main, &mut storage).unwrap();
    lane.sync_route(&router, 0).unwrap();
    let token = lane.token().unwrap();
    lane.expect_active(token, true).unwrap();
    lane.set_mute(token, false, 0).unwrap();
    lane.submit(token, 0, &pcm(7_000)).unwrap();
    let mut output = pcm(0);
    lane.consume(token, 0, 128, &mut output).unwrap();
    for first in [128, 256] {
        lane.consume(token, first, 128, &mut output).unwrap();
        assert_eq!(output, pcm(0));
    }
    assert_eq!(lane.stats().underrun_events, 1);
    assert_eq!(lane.submit(token, 128, &pcm(9_000)), Err(SinkError::Late));
    lane.submit(token, 384, &pcm(3_000)).unwrap();
    lane.consume(token, 384, 128, &mut output).unwrap();
    assert_eq!(output, pcm(3_000));
    lane.expect_active(token, false).unwrap();
    lane.consume(token, 512, 128, &mut output).unwrap();
    assert_eq!(output, pcm(0));
    assert_eq!(lane.stats().underrun_frames, 256);
    assert_eq!(lane.stats().idle_frames, 128);
    lane.expect_active(token, true).unwrap();
    lane.consume(token, 640, 128, &mut output).unwrap();
    assert_eq!(lane.stats().underrun_events, 2);
    assert_eq!(lane.stats().late_blocks, 1);
}

#[test]
fn blocked_main_does_not_block_headphones_and_invalid_packets_change_neither_queue() {
    let mut a = [SinkBlock::EMPTY; 1];
    let mut b = [SinkBlock::EMPTY; 1];
    let mut delivery = OutputDelivery::new(&mut a, &mut b).unwrap();
    let mut router = OutputRouter::default();
    ready(&mut router, controller());
    delivery.sync_routes(&router, 0).unwrap();
    let main = delivery.main.token();
    let phones = delivery.headphones.token();
    delivery
        .headphones
        .set_mute(phones.unwrap(), false, 0)
        .unwrap();
    assert_eq!(
        delivery.submit_render(main, phones, 0, &block(1_000), &pcm(5_000)),
        [Ok(()); 2]
    );
    let mut output = pcm(0);
    delivery
        .headphones
        .consume(phones.unwrap(), 0, 128, &mut output)
        .unwrap();
    assert_eq!(output, pcm(5_000));
    assert_eq!(
        delivery.submit_render(main, phones, 128, &block(2_000), &pcm(6_000)),
        [Err(SinkError::Full), Ok(())]
    );
    delivery
        .headphones
        .consume(phones.unwrap(), 128, 128, &mut output)
        .unwrap();
    assert_eq!(output, pcm(6_000));
    assert_eq!(delivery.main.stats().full_blocks, 1);
    let mut invalid = block(9_000);
    invalid.len = 127;
    assert_eq!(
        delivery.submit_render(main, phones, 256, &invalid, &pcm(9_000)),
        [Err(SinkError::Invalid); 2]
    );
    assert_eq!(delivery.headphones.queued_blocks(), 0);
    assert_eq!(delivery.main.queued_blocks(), 1);
}

#[test]
fn dac_change_preserves_phones_but_discards_main_pcm_and_old_callbacks() {
    let mut a = [SinkBlock::EMPTY; 2];
    let mut b = [SinkBlock::EMPTY; 2];
    let mut delivery = OutputDelivery::new(&mut a, &mut b).unwrap();
    let mut router = OutputRouter::default();
    ready(&mut router, controller());
    delivery.sync_routes(&router, 0).unwrap();
    let main = delivery.main.token().unwrap();
    let phones = delivery.headphones.token().unwrap();
    delivery.headphones.set_mute(phones, false, 0).unwrap();
    assert_eq!(
        delivery.submit_render(Some(main), Some(phones), 0, &block(1_000), &pcm(5_000)),
        [Ok(()); 2]
    );
    let upgrade = router
        .update(OutputAvailability {
            external_dac: Some(stream(10, 0, 1)),
            ..controller()
        })
        .unwrap();
    delivery.sync_routes(&router, 0).unwrap();
    assert!(delivery.main.token().is_none());
    assert_eq!(delivery.headphones.token(), Some(phones));
    assert!(router.commit_prepared(upgrade));
    delivery.sync_routes(&router, 0).unwrap();
    assert_ne!(delivery.main.token(), Some(main));
    assert_eq!(delivery.main.queued_blocks(), 0);
    let mut output = pcm(9_000);
    assert_eq!(
        delivery.main.consume(main, 0, 128, &mut output),
        Err(SinkError::Stale)
    );
    assert_eq!(output, pcm(0));
    assert_eq!(delivery.main.clock_frame(), 0);
    delivery
        .headphones
        .consume(phones, 0, 128, &mut output)
        .unwrap();
    assert_eq!(output, pcm(5_000));
    assert_eq!(delivery.main.stats().discarded_frames, 128);
}

#[test]
fn route_changes_returning_to_same_binding_still_revoke_old_pcm() {
    let mut storage = [SinkBlock::EMPTY; 1];
    let mut lane = SinkLane::new(SinkRole::Main, &mut storage).unwrap();
    let mut router = OutputRouter::default();
    ready(&mut router, controller());
    lane.sync_route(&router, 0).unwrap();
    let old = lane.token().unwrap();
    lane.submit(old, 0, &pcm(4_000)).unwrap();
    ready(&mut router, OutputAvailability::default());
    ready(&mut router, controller());
    lane.sync_route(&router, 0).unwrap();
    assert_ne!(lane.token(), Some(old));
    assert_eq!(lane.queued_blocks(), 0);
    assert_eq!(lane.submit(old, 0, &pcm(4_000)), Err(SinkError::Stale));
}

#[test]
fn source_reset_flushes_partial_pcm_and_keeps_new_stream_muted_until_primed() {
    let mut storage = [SinkBlock::EMPTY; 2];
    let mut lane = SinkLane::new(SinkRole::Main, &mut storage).unwrap();
    let mut router = OutputRouter::default();
    ready(&mut router, controller());
    lane.sync_route(&router, 0).unwrap();
    let old = lane.token().unwrap();
    lane.set_mute(old, false, 0).unwrap();
    lane.submit(old, 0, &pcm(8_000)).unwrap();
    let mut output = pcm(0);
    lane.consume(old, 0, 48, &mut output).unwrap();
    lane.reset_pcm(48).unwrap();
    let new = lane.token().unwrap();
    assert_ne!(new, old);
    assert_eq!(lane.stats().discarded_frames, 80);
    assert_eq!(lane.next_block_frame(), Some(128));
    assert!(lane.is_muted());
    lane.expect_active(new, true).unwrap();
    lane.consume(new, 48, 48, &mut output).unwrap();
    assert_eq!(output, pcm(0));
    assert_eq!(lane.stats().underrun_frames, 0);
    assert_eq!(lane.stats().muted_frames, 48);
}

#[test]
fn fades_are_frame_driven_monotonic_and_detach_is_immediately_silent() {
    let mut storage = [SinkBlock::EMPTY; 2];
    let mut lane = SinkLane::new(SinkRole::Main, &mut storage).unwrap();
    let mut router = OutputRouter::default();
    ready(&mut router, controller());
    lane.sync_route(&router, 0).unwrap();
    let token = lane.token().unwrap();
    lane.submit(token, 0, &pcm(16_000)).unwrap();
    lane.set_mute(token, false, 4).unwrap();
    let mut output = pcm(0);
    lane.consume(token, 0, 4, &mut output).unwrap();
    assert_eq!(
        output[..4].iter().map(|p| p.left).collect::<Vec<_>>(),
        [4_000, 8_000, 12_000, 16_000]
    );
    lane.set_mute(token, true, 4).unwrap();
    lane.consume(token, 4, 4, &mut output).unwrap();
    assert_eq!(
        output[..4].iter().map(|p| p.left).collect::<Vec<_>>(),
        [12_000, 8_000, 4_000, 0]
    );
    assert!(lane.is_muted());
    ready(&mut router, OutputAvailability::default());
    lane.sync_route(&router, 8).unwrap();
    assert_eq!(
        lane.consume(token, 8, 4, &mut output),
        Err(SinkError::Stale)
    );
    assert_eq!(output, pcm(0));
}

#[test]
fn duplicate_callbacks_overlap_overflow_and_invalid_capacity_are_fail_closed() {
    assert!(matches!(
        SinkLane::new(SinkRole::Main, &mut []),
        Err(SinkError::Capacity)
    ));
    assert!(matches!(
        SinkLane::new(SinkRole::Main, &mut [SinkBlock::EMPTY; MAX_SINK_BLOCKS + 1]),
        Err(SinkError::Capacity)
    ));
    let mut storage = [SinkBlock::EMPTY; 2];
    let mut lane = SinkLane::new(SinkRole::Main, &mut storage).unwrap();
    let mut router = OutputRouter::default();
    ready(&mut router, controller());
    lane.sync_route(&router, 0).unwrap();
    let token = lane.token().unwrap();
    lane.submit(token, 0, &pcm(1_000)).unwrap();
    assert_eq!(lane.submit(token, 64, &pcm(2_000)), Err(SinkError::Order));
    let mut output = pcm(9_000);
    lane.consume(token, 0, 48, &mut output).unwrap();
    assert_eq!(
        lane.consume(token, 0, 48, &mut output),
        Err(SinkError::Order)
    );
    assert_eq!(lane.clock_frame(), 48);
    assert_eq!(output, pcm(0));
    assert_eq!(
        lane.consume(token, 48, 129, &mut output),
        Err(SinkError::Invalid)
    );
    assert_eq!(
        lane.submit(token, u64::MAX, &pcm(2_000)),
        Err(SinkError::Overflow)
    );
    lane.reset_pcm(u64::MAX - 8).unwrap();
    let token = lane.token().unwrap();
    assert_eq!(lane.next_block_frame(), None);
    assert_eq!(
        lane.consume(token, u64::MAX - 8, 16, &mut output),
        Err(SinkError::Overflow)
    );
    assert_eq!(lane.clock_frame(), u64::MAX - 8);
}

#[test]
fn inactive_headphones_never_receive_dac_pcm_and_tokens_cannot_cross_lanes() {
    let mut a = [SinkBlock::EMPTY; 1];
    let mut b = [SinkBlock::EMPTY; 1];
    let mut delivery = OutputDelivery::new(&mut a, &mut b).unwrap();
    let mut router = OutputRouter::default();
    ready(
        &mut router,
        OutputAvailability {
            external_dac: Some(stream(10, 0, 1)),
            ..OutputAvailability::default()
        },
    );
    delivery.sync_routes(&router, 0).unwrap();
    let dac = delivery.main.token().unwrap();
    assert!(delivery.headphones.token().is_none());
    assert_eq!(
        delivery.submit_render(Some(dac), Some(dac), 0, &block(9_000), &pcm(9_000)),
        [Ok(()), Err(SinkError::Stale)]
    );
    let mut output = pcm(9_000);
    assert_eq!(
        delivery.headphones.consume(dac, 0, 128, &mut output),
        Err(SinkError::Stale)
    );
    assert_eq!(output, pcm(0));
    assert_eq!(delivery.headphones.queued_blocks(), 0);
}

#[test]
fn future_packets_do_not_hide_gaps_and_equal_end_callbacks_release_capacity() {
    let mut storage = [SinkBlock::EMPTY; 1];
    let mut lane = SinkLane::new(SinkRole::Main, &mut storage).unwrap();
    let mut router = OutputRouter::default();
    ready(&mut router, controller());
    lane.sync_route(&router, 0).unwrap();
    let token = lane.token().unwrap();
    lane.expect_active(token, true).unwrap();
    lane.set_mute(token, false, 0).unwrap();
    lane.submit(token, 128, &pcm(2_000)).unwrap();
    assert_eq!(lane.next_block_frame(), None);
    let mut output = pcm(0);
    lane.consume(token, 0, 128, &mut output).unwrap();
    assert_eq!(output, pcm(0));
    assert_eq!(lane.queued_blocks(), 1);
    lane.consume(token, 128, 128, &mut output).unwrap();
    assert_eq!(output, pcm(2_000));
    assert_eq!(lane.next_block_frame(), Some(256));
    assert_eq!(lane.stats().underrun_frames, 128);
}

#[test]
fn ten_seconds_of_48_and_128_frame_consumers_preserve_cadence_and_pre_fader_pfl() {
    clocked_output::run_simulation();
}

#[test]
fn faster_headphone_clock_reports_drift_symptoms_without_corrupting_main() {
    let mut a = [SinkBlock::EMPTY; 4];
    let mut b = [SinkBlock::EMPTY; 4];
    let mut delivery = OutputDelivery::new(&mut a, &mut b).unwrap();
    let mut router = OutputRouter::default();
    ready(&mut router, controller());
    delivery.sync_routes(&router, 0).unwrap();
    let main = delivery.main.token().unwrap();
    let phones = delivery.headphones.token().unwrap();
    delivery.main.expect_active(main, true).unwrap();
    delivery.headphones.expect_active(phones, true).unwrap();
    delivery.main.set_mute(main, false, 0).unwrap();
    delivery.headphones.set_mute(phones, false, 0).unwrap();
    let mut output = pcm(0);
    for _ in 0..2_000 {
        while delivery.main.queued_blocks() < 2 {
            let first = delivery.main.next_block_frame().unwrap();
            let result =
                delivery.submit_render(Some(main), Some(phones), first, &block(1_000), &pcm(2_000));
            assert_eq!(result[0], Ok(()));
            assert!(matches!(result[1], Ok(()) | Err(SinkError::Late)));
        }
        delivery
            .main
            .consume(main, delivery.main.clock_frame(), 120, &mut output)
            .unwrap();
        assert!(output[..120].iter().all(|frame| frame.left == 1_000));
        delivery
            .headphones
            .consume(phones, delivery.headphones.clock_frame(), 128, &mut output)
            .unwrap();
        assert!(delivery.main.queued_blocks() <= 4 && delivery.headphones.queued_blocks() <= 4);
    }
    assert_eq!(delivery.main.stats().underrun_frames, 0);
    assert!(delivery.headphones.stats().underrun_frames > 0);
    assert!(delivery.headphones.stats().late_blocks > 0);
}

#[test]
fn a_full_headphone_queue_still_accepts_new_main_pcm() {
    let mut a = [SinkBlock::EMPTY; 1];
    let mut b = [SinkBlock::EMPTY; 1];
    let mut delivery = OutputDelivery::new(&mut a, &mut b).unwrap();
    let mut router = OutputRouter::default();
    ready(&mut router, controller());
    delivery.sync_routes(&router, 0).unwrap();
    let main = delivery.main.token().unwrap();
    let phones = delivery.headphones.token().unwrap();
    assert_eq!(
        delivery.submit_render(Some(main), Some(phones), 0, &block(1_000), &pcm(5_000)),
        [Ok(()); 2]
    );
    let mut output = pcm(0);
    delivery.main.consume(main, 0, 128, &mut output).unwrap();
    assert_eq!(
        delivery.submit_render(Some(main), Some(phones), 128, &block(2_000), &pcm(6_000)),
        [Ok(()), Err(SinkError::Full)]
    );
    assert_eq!(delivery.main.queued_blocks(), 1);
    assert_eq!(delivery.headphones.stats().full_blocks, 1);
}
