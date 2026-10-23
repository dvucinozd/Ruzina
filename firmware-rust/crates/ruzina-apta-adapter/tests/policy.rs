#[path = "support/allocation.rs"]
mod allocation;
#[allow(dead_code)]
mod support;
use libapta::{
    WaveformColumn,
    session::{Session, SessionConfig},
    waveform::NormalizedSample,
};
use ruzina_apta_adapter::{
    Error,
    source::{SourceError, SourceRequest},
    storage::LocalTempoStorage,
    streaming::StreamLimits,
    worker::Requests,
};
use support::*;

#[test]
fn opened_object_limit_closes_without_reading_and_close_failure_is_visible() {
    for mode in [0, 5] {
        let m = mounted();
        let mut r = Requests::default();
        let mut fs = Fs::new(&m, wav(300));
        fs.mode = mode;
        let c = catalog(&m);
        let request = SourceRequest::new(&mut r, &m, &c, 0).unwrap();
        let limits = StreamLimits {
            object_bytes: fs.bytes.len() as u64 - 1,
            ..StreamLimits::UNLIMITED
        };
        let error = run(request.open_stream_with_limits(&mut fs, &r, &m, limits))
            .err()
            .unwrap();
        if mode == 0 {
            assert!(matches!(error, SourceError::Limit));
        } else {
            assert!(matches!(error, SourceError::Io(Fault::Io)));
        }
        assert_eq!((fs.reads, fs.closes), (0, 1));
    }
}

#[test]
fn duration_limit_is_exact_and_checked_after_close_before_source_verification() {
    // At 8 kHz, 304 frames is exactly 38 ms; 305 frames must not be rounded down.
    for frames in [303, 304, 305] {
        let m = mounted();
        let mut r = Requests::default();
        let mut fs = Fs::new(&m, wav(frames));
        fs.max = 4096;
        let c = catalog(&m);
        let request = SourceRequest::new(&mut r, &m, &c, 0).unwrap();
        let limits = StreamLimits {
            object_bytes: fs.bytes.len() as u64,
            duration_ms: 38,
            scan_steps: 1,
            processing_steps: 100,
        };
        let mut rd = run(request.open_stream_with_limits(&mut fs, &r, &m, limits)).unwrap();
        let mut bytes = [0; 4096];
        assert!(run(rd.step(&mut fs, &r, &m, &mut bytes)).unwrap());
        // A completed scan is idempotent and does not spend another read/step.
        assert!(run(rd.step(&mut fs, &r, &m, &mut bytes)).unwrap());
        let result = run(rd.finish(&mut fs, &mut r, &m));
        if frames <= 304 {
            assert!(result.is_ok());
        } else {
            assert!(matches!(result, Err(SourceError::Limit)));
        }
        assert_eq!((fs.reads, fs.closes), (1, 1));
    }
}

#[test]
fn scan_exhaustion_is_terminal_and_handle_remains_available_for_abort_or_finish() {
    for allowed in [0, 1] {
        for finish in [false, true] {
            let m = mounted();
            let mut r = Requests::default();
            let mut fs = Fs::new(&m, wav(300));
            fs.max = 1;
            let c = catalog(&m);
            let request = SourceRequest::new(&mut r, &m, &c, 0).unwrap();
            let limits = StreamLimits {
                scan_steps: allowed,
                ..StreamLimits::UNLIMITED
            };
            let mut rd = run(request.open_stream_with_limits(&mut fs, &r, &m, limits)).unwrap();
            let mut bytes = [0; 4096];
            for _ in 0..allowed {
                assert!(!run(rd.step(&mut fs, &r, &m, &mut bytes)).unwrap());
            }
            assert!(matches!(
                run(rd.step(&mut fs, &r, &m, &mut bytes)),
                Err(SourceError::Limit)
            ));
            assert!(matches!(
                run(rd.step(&mut fs, &r, &m, &mut bytes)),
                Err(SourceError::Failed)
            ));
            assert_eq!(fs.reads as u64, allowed);
            if finish {
                assert!(run(rd.finish(&mut fs, &mut r, &m)).is_err());
            } else {
                run(rd.abort(&mut fs)).unwrap();
            }
            assert_eq!(fs.closes, 1);
        }
    }
}

fn process(limit: u64) -> (u64, bool) {
    let m = mounted();
    let mut r = Requests::default();
    let mut fs = Fs::new(&m, wav(300));
    let c = catalog(&m);
    let request = SourceRequest::new(&mut r, &m, &c, 0).unwrap();
    let limits = StreamLimits {
        processing_steps: limit,
        ..StreamLimits::UNLIMITED
    };
    let mut bytes = [0; 4096];
    let mut pcm = [0.; 512];
    let mut queue = [NormalizedSample::default(); 17];
    let mut cols = [WaveformColumn::default(); 2];
    allocation::start();
    let mut rd = run(request.open_stream_with_limits(&mut fs, &r, &m, limits)).unwrap();
    while !run(rd.step(&mut fs, &r, &m, &mut bytes)).unwrap() {}
    let verified = run(rd.finish(&mut fs, &mut r, &m)).unwrap();
    let session = Session::new(
        SessionConfig {
            sample_rate: 8000,
            channel_count: 1,
            total_frames: 300,
            frames_per_column: 256,
        },
        &mut queue,
        &mut cols,
    )
    .unwrap();
    let mut w = verified.worker(session, &mut bytes).unwrap();
    run(w.open(&mut fs, &r, &m)).unwrap();
    let mut steps = 0;
    let complete = loop {
        match run(w.step(&mut fs, &r, &m, &mut pcm)) {
            Ok(done) => {
                steps += 1;
                if done {
                    break true;
                }
            }
            Err(SourceError::Limit) => break false,
            Err(e) => panic!("unexpected {e:?}"),
        }
        assert!(steps < 1000);
    };
    let reads = fs.reads;
    if complete {
        assert!(run(w.step(&mut fs, &r, &m, &mut pcm)).unwrap());
        run(w.finish(&mut fs, &r, &m)).unwrap();
        assert!(w.into_worker().is_ok());
    } else {
        assert_eq!(steps, limit);
        assert!(matches!(
            run(w.step(&mut fs, &r, &m, &mut pcm)),
            Err(SourceError::Failed)
        ));
        assert!(run(w.finish(&mut fs, &r, &m)).is_err());
        assert!(w.into_worker().is_err());
    }
    assert_eq!(fs.reads, reads);
    assert_eq!(fs.closes, 2);
    allocation::finish();
    (steps, complete)
}
#[test]
fn processing_budget_survives_partial_acceptance_and_never_releases_partial_output() {
    let (steps, done) = process(u64::MAX);
    assert!(done && steps > 2);
    assert_eq!(process(steps), (steps, true));
    assert_eq!(process(steps - 1), (steps - 1, false));
    assert_eq!(process(0), (0, false));
}

#[test]
fn local_tempo_array_budget_is_checked_including_two_retained_banks() {
    allocation::start();
    let p = LocalTempoStorage::plan(320000, 256, 17, 128, 113772).unwrap();
    assert_eq!(
        (p.overview_columns, p.onset_bytes, p.queue_bytes),
        (1250, 81920, 136)
    );
    assert_eq!(
        (p.overview_bytes, p.rendered_bytes, p.beat_banks_bytes),
        (12500, 5000, 3072)
    );
    assert_eq!((p.source_scratch_bytes, p.total_bytes), (6144, 113772));
    assert_eq!(p.rendered_banks_bytes, 10000);
    assert_eq!(
        p.total_bytes,
        p.onset_bytes
            + p.queue_bytes
            + p.overview_bytes
            + p.beat_banks_bytes
            + p.source_scratch_bytes
            + 2 * p.rendered_bytes
    );
    assert_eq!(
        LocalTempoStorage::plan(320000, 256, 17, 128, 108772),
        Err(Error::Capacity)
    );
    assert_eq!(
        LocalTempoStorage::plan(320000, 256, 17, 128, 113771),
        Err(Error::Capacity)
    );
    assert_eq!(
        LocalTempoStorage::plan(301, 256, 17, 128, usize::MAX)
            .unwrap()
            .overview_columns,
        2
    );
    assert_eq!(
        LocalTempoStorage::plan(301, 256, usize::MAX, 128, usize::MAX),
        Err(Error::Capacity)
    );
    assert_eq!(
        LocalTempoStorage::plan(301, 256, 17, usize::MAX, usize::MAX),
        Err(Error::Capacity)
    );
    // Each summand fits but their sum does not.
    assert_eq!(
        LocalTempoStorage::plan(301, 256, usize::MAX / 8, 128, usize::MAX),
        Err(Error::Capacity)
    );
    for (frames, fpc, queue) in [
        (0, 256, 17),
        (u64::MAX, 256, 17),
        (301, 0, 17),
        (301, 255, 17),
        (301, 256, 0),
    ] {
        assert_eq!(
            LocalTempoStorage::plan(frames, fpc, queue, 128, usize::MAX),
            Err(Error::Invalid)
        );
    }
    allocation::finish();
}

#[test]
fn service_plan_matches_actual_two_bank_payload_and_broker_scratch() {
    use core::mem::size_of_val;
    use libapta::{
        WaveformColumn,
        analysis::{BIN_CAPACITY, OnsetBin},
        waveform::NormalizedSample,
    };
    use ruzina_apta_adapter::storage::ServiceStorage;
    use ruzina_track_analysis::Beat;
    use ruzina_waveform::WaveformColumn as RenderColumn;
    let plan = ServiceStorage::plan(320000, 256, 17, 128, 117868).unwrap();
    let queue = vec![NormalizedSample::default(); 17];
    let columns = vec![WaveformColumn::default(); 1250];
    let bins = vec![OnsetBin::default(); BIN_CAPACITY];
    let flux = vec![0f32; BIN_CAPACITY];
    let beats = [
        vec![
            Beat {
                time_ms: 0,
                phase: 0,
                bpm_x100: 0
            };
            128
        ],
        vec![
            Beat {
                time_ms: 0,
                phase: 0,
                bpm_x100: 0
            };
            128
        ],
    ];
    let wave = [
        vec![
            RenderColumn {
                peak: 0,
                color_rgb565: 0
            };
            1250
        ],
        vec![
            RenderColumn {
                peak: 0,
                color_rgb565: 0
            };
            1250
        ],
    ];
    let bytes = [0u8; 4096];
    let pcm = [0f32; 512];
    let broker = [0u8; 4096];
    let actual = size_of_val(queue.as_slice())
        + size_of_val(columns.as_slice())
        + size_of_val(bins.as_slice())
        + size_of_val(flux.as_slice())
        + beats
            .iter()
            .map(|v| size_of_val(v.as_slice()))
            .sum::<usize>()
        + wave
            .iter()
            .map(|v| size_of_val(v.as_slice()))
            .sum::<usize>()
        + size_of_val(&bytes)
        + size_of_val(&pcm)
        + size_of_val(&broker);
    assert_eq!(plan.total_bytes, actual);
    assert_eq!(plan.total_bytes, 117868);
    assert_eq!(
        ServiceStorage::plan(320000, 256, 17, 128, 117867),
        Err(Error::Capacity)
    );
    assert_eq!(
        ServiceStorage::plan(320000, 256, 17, 128, 4095),
        Err(Error::Capacity)
    );
}
