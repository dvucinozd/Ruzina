//! Headless deterministic consumer-clock demonstration; no audio device opened.
use ruzina_audio_dsp::PcmFrame;
use ruzina_audio_runtime::{
    pcm_stream::{PcmStreamBlock, PcmTransportCursor},
    playback::PlaybackOutput,
    render::{FixedRateRenderGraph, RenderControl, RenderDeckControl, RenderOutputBlock},
    routing::{ChannelPair, OutputAvailability, OutputRouter, ReadyStream},
    sink::{BlockProgress, SinkBlock, SinkRole},
    source::{CanonicalSource, IngestResult, SOURCE_HISTORY_FRAMES},
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
    let mut playback = PlaybackOutput::new(&mut main_storage, &mut phone_storage).unwrap();
    playback.sync_routes(&router, 0).unwrap();
    playback.sync_sources([1, 0], 0).unwrap();
    let main = playback.lane(SinkRole::Main).token().unwrap();
    let phones = playback.lane(SinkRole::Headphones).token().unwrap();
    for (role, token) in [(SinkRole::Main, main), (SinkRole::Headphones, phones)] {
        playback.lane_mut(role).expect_active(token, true).unwrap();
        playback.lane_mut(role).set_mute(token, false, 64).unwrap();
    }
    let mut history_a = [PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut history_b = [PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut sources = [
        CanonicalSource::new(&mut history_a).unwrap(),
        CanonicalSource::new(&mut history_b).unwrap(),
    ];
    sources[0].reset(1);
    let mut prefetched = 0;
    let mut transport = PcmTransportCursor::new();
    let mut pending_main = None;
    let mut pending_phones = None;

    let mut graph = FixedRateRenderGraph::new();
    let mut render = RenderOutputBlock::silence();
    let mut output = [PcmFrame::default(); 128];
    let control = RenderControl {
        decks: [
            RenderDeckControl {
                playing: true,
                stream_generation: 1,
                pfl_enabled: true,
                post_gain: 0.25,
                ..RenderDeckControl::idle()
            },
            RenderDeckControl::idle(),
        ],
        master_gain: 1.0,
        headphones: ruzina_mixer::HeadphoneGains {
            cue: 1.0,
            master: 0.0,
            level: 1.0,
        },
    };
    let mut main_peak = 0i16;
    let mut phone_peak = 0i16;
    // 16-frame virtual steps: exact 48-frame USB requests and 128-frame DMA
    // requests share a 48 kHz clock. No rounded microsecond sleep accumulates.
    for tick in 0..30_000u64 {
        // A driver completes on the following virtual turn, not at buffer fill.
        for receipt in [pending_main.take(), pending_phones.take()]
            .into_iter()
            .flatten()
        {
            playback.complete(receipt).unwrap();
        }
        if let Some((progress, frames)) = playback.pending_progress(0) {
            transport.accept_active_progress(progress, frames, 1);
            playback.clear_progress(0);
        }
        // MAIN is the explicit clock authority in this scenario. Render only
        // after checking space; bounded lookahead is two 128-frame blocks.
        while playback.lane(SinkRole::Main).queued_blocks() < 2 {
            let first = playback.next_block_frame().unwrap();
            for _ in 0..16 {
                let mut block = PcmStreamBlock::empty(1, prefetched, 48_000);
                block.set_len(256);
                block.frames.fill(PcmFrame {
                    left: 8_000,
                    right: 8_000,
                });
                match sources[0].ingest(&block) {
                    IngestResult::Accepted => prefetched += 256,
                    IngestResult::Backpressure => break,
                    other => panic!("unexpected source result: {other:?}"),
                }
            }
            let permit = playback.render_permit();
            let report = graph.render_canonical_block(control, &mut render, &mut sources);
            let progress = BlockProgress {
                decks: [
                    sources[0].take_progress(control.decks[0].source_control()),
                    None,
                ],
                source_frames: [report.source_frames_consumed[0] as u16, 0],
            };
            // Product monitor bus, with cue-only settings for this simulation.
            let results = playback.submit_render(permit, first, &render, progress);
            assert_eq!(results, [Ok(()); 2]);
        }
        if tick % 8 == 0 {
            pending_main = Some(
                playback
                    .lane_mut(SinkRole::Main)
                    .issue(main, tick * 16, 128, &mut output)
                    .unwrap(),
            );
            main_peak = main_peak.max(output.iter().map(|f| f.left).max().unwrap());
        }
        if tick % 3 == 0 {
            pending_phones = Some(
                playback
                    .lane_mut(SinkRole::Headphones)
                    .issue(phones, tick * 16, 48, &mut output)
                    .unwrap(),
            );
            phone_peak = phone_peak.max(output[..48].iter().map(|f| f.left).max().unwrap());
        }
    }
    for receipt in [pending_main, pending_phones].into_iter().flatten() {
        playback.complete(receipt).unwrap();
    }
    if let Some((progress, frames)) = playback.pending_progress(0) {
        transport.accept_active_progress(progress, frames, 1);
    }
    assert_eq!(playback.lane(SinkRole::Main).clock_frame(), 480_000);
    assert_eq!(playback.lane(SinkRole::Headphones).clock_frame(), 480_000);
    assert_eq!(playback.lane(SinkRole::Main).stats().underrun_frames, 0);
    assert_eq!(
        playback.lane(SinkRole::Headphones).stats().underrun_frames,
        0
    );
    assert_eq!(transport.position_frame(), 480_000);
    assert!(sources[0].position() > transport.position_frame());
    assert_eq!((main_peak, phone_peak), (2_000, 8_000));
    println!(
        "SIMULATION_ONLY frames=480000 duration=10s MAIN_peak={main_peak} PFL_peak={phone_peak}"
    );
    println!(
        "MAIN {:?}\nHEADPHONES {:?}",
        playback.lane(SinkRole::Main).stats(),
        playback.lane(SinkRole::Headphones).stats()
    );
    println!(
        "TRANSPORT confirmed={} rendered={} prefetched={prefetched}",
        transport.position_frame(),
        sources[0].position()
    );
}
