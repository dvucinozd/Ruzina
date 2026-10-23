//! RP-17: Master Tempo starvation, ordinary-resampling fallback and refill
//! never rewind the real consumer cursor (donor `2fa22631`).
//!
//! The donor bug: ordinary resampling advanced the consumer while MT lacked
//! look-ahead (loop cut, EOF), then the stale MT clock resumed and moved the
//! playhead backwards. Here MT starves without advancing, and every mode
//! change re-anchors MT at the actual cursor; this pins that behavior across
//! the donor's matrix of rates, tempos, retained runway and loop timelines.

use ruzina_audio_dsp::PcmFrame;
use ruzina_audio_runtime::{
    pcm_stream::{PCM_STREAM_BLOCK_FRAMES, PcmStreamBlock},
    source::{CanonicalSource, IngestResult, SOURCE_HISTORY_FRAMES, SourceControl},
    timeline::{FrameLoop, PcmTimeline},
};

fn pcm(frame: u64) -> PcmFrame {
    let phase = (frame % 96) as i32 - 48;
    PcmFrame {
        left: (phase * 600) as i16,
        right: (-phase * 400) as i16,
    }
}

struct Feed {
    generation: u32,
    rate: u32,
    timeline: PcmTimeline,
    next: u64,
}

impl Feed {
    /// Ingests up to `limit` (exclusive PCM coordinate) as far as backpressure
    /// allows.
    fn fill(&mut self, source: &mut CanonicalSource<'_>, limit: u64) {
        while self.next < limit {
            let len = (limit - self.next).min(PCM_STREAM_BLOCK_FRAMES as u64) as usize;
            let mut block = PcmStreamBlock::empty(self.generation, self.next, self.rate);
            block.timeline = self.timeline;
            assert!(block.set_len(len));
            for (offset, frame) in block.frames_mut()[..len].iter_mut().enumerate() {
                *frame = pcm(self.next + offset as u64);
            }
            match source.ingest(&block) {
                IngestResult::Accepted => self.next += len as u64,
                IngestResult::Backpressure => return,
                other => panic!("unexpected ingest result {other:?}"),
            }
        }
    }
}

struct Case {
    rate: u32,
    tempo: f32,
    runway: u64,
    looped: bool,
    fallback: bool,
}

fn run(case: &Case) {
    let mut storage = [PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut source = CanonicalSource::new(&mut storage).unwrap();
    source.reset(3);
    let origin = 1_000;
    let timeline = PcmTimeline::at(
        origin,
        case.looped
            .then(|| FrameLoop::new(1_000, 4_000, 1_000_000).unwrap()),
    );
    let mut feed = Feed {
        generation: 3,
        rate: case.rate,
        timeline,
        next: origin,
    };
    let mt = SourceControl {
        playing: true,
        master_tempo: true,
        tempo_factor: case.tempo,
        output_sample_rate_hz: 48_000,
        ..SourceControl::default()
    };
    let ordinary = SourceControl {
        master_tempo: false,
        ..mt
    };
    let context = format!(
        "rate={} tempo={} runway={} looped={} fallback={}",
        case.rate, case.tempo, case.runway, case.looped, case.fallback
    );

    // Withhold PCM twice so MT runs dry with only `runway` frames retained,
    // optionally falls back to ordinary resampling, then refills.
    let stalls = [(origin + 3_000, 600u32), (origin + 9_000, 900)];
    let mut last = source.position();
    let mut starved = 0u32;
    let mut stall = 0;
    let mut held = 0u32;
    for step in 0..40_000u32 {
        let limit = match stalls.get(stall) {
            Some((at, _)) => (*at + case.runway).min(source.position() + 2_048),
            None => source.position() + 2_048,
        };
        feed.fill(&mut source, limit);
        let in_stall = stalls
            .get(stall)
            .is_some_and(|(at, _)| feed.next >= *at + case.runway);
        let control = if in_stall && case.fallback && held % 200 < 100 {
            ordinary
        } else {
            mt
        };
        let sample = source.next_sample(control);
        assert!(!sample.deferred, "{context}");
        let position = source.position();
        assert!(
            position >= last,
            "{context}: cursor moved back from {last} to {position} at step {step}"
        );
        last = position;
        if sample.starved {
            starved += 1;
        }
        // The stall lasts this many dry samples, not merely withheld input.
        if in_stall && sample.starved {
            held += 1;
            if held >= stalls[stall].1 {
                stall += 1;
                held = 0;
            }
        }
        let progress = source.take_progress(mt);
        if let Some(progress) = progress {
            assert_eq!(
                Some(progress.end_frame),
                timeline.media_frame(source.audible_position()),
                "{context}"
            );
        }
    }
    assert!(starved > 0, "{context}: the case never starved");
    assert_eq!(stall, stalls.len(), "{context}");
    assert!(
        last > origin + 9_000 + case.runway + 1_000,
        "{context}: playback did not resume after refill ({last})"
    );
}

#[test]
fn master_tempo_starvation_and_fallback_never_rewind_the_consumer() {
    let mut cases = 0;
    for rate in [44_100, 48_000] {
        for tempo in [0.95, 1.0, 1.05] {
            for runway in [0, 1, 64, 300, 700] {
                for looped in [false, true] {
                    for fallback in [false, true] {
                        run(&Case {
                            rate,
                            tempo,
                            runway,
                            looped,
                            fallback,
                        });
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 120);
}
