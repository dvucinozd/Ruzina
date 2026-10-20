//! Headless deterministic consumer-clock demonstration; no audio device opened.
use pajoniiir_audio_dsp::PcmFrame;
use pajoniiir_audio_runtime::{
    render::{FixedRateRenderGraph, RenderControl, RenderDeckControl, RenderOutputBlock},
    routing::{ChannelPair, OutputAvailability, OutputRouter, ReadyStream},
    sink::{OutputDelivery, SinkBlock},
};

#[cfg(not(test))]
fn main() {
    run_simulation();
}

pub fn run_simulation() {
    let stream = |left, right| ReadyStream {
        connection_epoch: 1,
        stream_epoch: 1,
        interface: 1,
        channels: 4,
        pair: ChannelPair { left, right },
        sample_rate_hz: 48_000,
        sample_bits: 16,
    };
    let mut router = OutputRouter::default();
    let pending = router
        .update(OutputAvailability {
            controller_main: Some(stream(0, 1)),
            controller_headphones: Some(stream(2, 3)),
            external_dac: None,
        })
        .unwrap();
    // Only the simulated drivers acknowledge readiness in this executable.
    assert!(router.commit_prepared(pending));
    let mut main_storage = [SinkBlock::EMPTY; 4];
    let mut phone_storage = [SinkBlock::EMPTY; 4];
    let mut delivery = OutputDelivery::new(&mut main_storage, &mut phone_storage).unwrap();
    delivery.sync_routes(&router, 0).unwrap();
    let main = delivery.main.token().unwrap();
    let phones = delivery.headphones.token().unwrap();
    delivery.main.expect_active(main, true).unwrap();
    delivery.headphones.expect_active(phones, true).unwrap();
    delivery.main.set_mute(main, false, 64).unwrap();
    delivery.headphones.set_mute(phones, false, 64).unwrap();

    let mut graph = FixedRateRenderGraph::new();
    let mut render = RenderOutputBlock::silence();
    let mut output = [PcmFrame::default(); 128];
    let control = RenderControl {
        decks: [
            RenderDeckControl {
                playing: true,
                pfl_enabled: true,
                post_gain: 0.25,
                ..RenderDeckControl::idle()
            },
            RenderDeckControl::idle(),
        ],
        master_gain: 1.0,
    };
    let mut main_peak = 0i16;
    let mut phone_peak = 0i16;
    // 16-frame virtual steps: exact 48-frame USB requests and 128-frame DMA
    // requests share a 48 kHz clock. No rounded microsecond sleep accumulates.
    for tick in 0..30_000u64 {
        // MAIN is the explicit clock authority in this scenario. Render only
        // after checking space; bounded lookahead is two 128-frame blocks.
        while delivery.main.queued_blocks() < 2 {
            let first = delivery.main.next_block_frame().unwrap();
            graph.render_block(control, &mut render, |_| {
                Some(PcmFrame {
                    left: 8_000,
                    right: -8_000,
                })
            });
            // Explicit demonstration mix: deck-one PFL only. Product cue/master
            // mixing and headphone gain are still a separate integration step.
            let results =
                delivery.submit_render(Some(main), Some(phones), first, &render, &render.pfl[0]);
            assert_eq!(results, [Ok(()); 2]);
        }
        if tick % 8 == 0 {
            delivery
                .main
                .consume(main, tick * 16, 128, &mut output)
                .unwrap();
            main_peak = main_peak.max(output.iter().map(|f| f.left).max().unwrap());
        }
        if tick % 3 == 0 {
            delivery
                .headphones
                .consume(phones, tick * 16, 48, &mut output)
                .unwrap();
            phone_peak = phone_peak.max(output[..48].iter().map(|f| f.left).max().unwrap());
        }
    }
    assert_eq!(delivery.main.clock_frame(), 480_000);
    assert_eq!(delivery.headphones.clock_frame(), 480_000);
    assert_eq!(delivery.main.stats().underrun_frames, 0);
    assert_eq!(delivery.headphones.stats().underrun_frames, 0);
    assert_eq!((main_peak, phone_peak), (2_000, 8_000));
    println!(
        "SIMULATION_ONLY frames=480000 duration=10s MAIN_peak={main_peak} PFL_peak={phone_peak}"
    );
    println!(
        "MAIN {:?}\nHEADPHONES {:?}",
        delivery.main.stats(),
        delivery.headphones.stats()
    );
}
