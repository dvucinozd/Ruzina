#[path = "support/allocation.rs"]
mod allocation;
#[allow(dead_code)]
mod support;
use libapta::{
    WaveformColumn,
    session::{Session, SessionConfig},
    waveform::NormalizedSample,
};
use pajoniiir_apta_adapter::{
    source::{SourceError, SourceRequest},
    streaming::VerifiedStream,
    worker::{Requests, WorkerError},
};
use support::*;

fn scan(
    fs: &mut Fs,
    r: &mut Requests,
    m: &pajoniiir_media_session::MediaSession,
) -> VerifiedStream {
    let c = catalog(m);
    let q = SourceRequest::new(r, m, &c, 0).unwrap();
    let mut read = run(q.open_stream(fs, r, m)).unwrap();
    let mut bytes = [0; 4096];
    while !run(read.step(fs, r, m, &mut bytes)).unwrap() {}
    run(read.finish(fs, r, m)).unwrap()
}
#[test]
fn both_passes_reject_bad_io_and_dropped_pending_reads_and_close_once() {
    for second in [false, true] {
        for mode in [1, 2, 3, 4, 6, 7, 9] {
            let m = mounted();
            let mut r = Requests::default();
            let mut fs = Fs::new(&m, wav(300));
            let mut bytes = [0; 4096];
            if second {
                let source = scan(&mut fs, &mut r, &m);
                let mut q = [NormalizedSample::default(); 17];
                let mut cols = [WaveformColumn::default(); 2];
                let session = Session::new(
                    SessionConfig {
                        sample_rate: 8000,
                        channel_count: 1,
                        total_frames: 300,
                        frames_per_column: 256,
                    },
                    &mut q,
                    &mut cols,
                )
                .unwrap();
                let mut w = source.worker(session, &mut bytes).unwrap();
                run(w.open(&mut fs, &r, &m)).unwrap();
                fs.mode = mode;
                let mut pcm = [0.; 512];
                let f = w.step(&mut fs, &r, &m, &mut pcm);
                if mode == 4 || mode == 9 {
                    assert_pending(f);
                } else {
                    assert!(run(f).is_err());
                }
                fs.mode = 0;
                assert!(matches!(
                    run(w.step(&mut fs, &r, &m, &mut pcm)),
                    Err(SourceError::Failed)
                ));
                assert!(run(w.finish(&mut fs, &r, &m)).is_err());
                assert!(w.verified_worker().is_err());
                assert!(w.into_worker().is_err());
                assert_eq!(fs.closes, 2);
            } else {
                let c = catalog(&m);
                let req = SourceRequest::new(&mut r, &m, &c, 0).unwrap();
                let mut rd = run(req.open_stream(&mut fs, &r, &m)).unwrap();
                fs.mode = mode;
                let f = rd.step(&mut fs, &r, &m, &mut bytes);
                if mode == 4 || mode == 9 {
                    assert_pending(f);
                } else {
                    assert!(run(f).is_err());
                }
                fs.mode = 0;
                assert!(matches!(
                    run(rd.step(&mut fs, &r, &m, &mut bytes)),
                    Err(SourceError::Failed)
                ));
                assert!(run(rd.finish(&mut fs, &mut r, &m)).is_err());
                assert_eq!(fs.closes, 1);
            }
        }
    }
}
fn assert_pending(f: impl core::future::Future) {
    use core::{
        pin::pin,
        task::{Context, Poll, Waker},
    };
    assert!(matches!(
        pin!(f)
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
}
#[test]
fn changed_object_bytes_never_release_analysis_even_with_identical_geometry() {
    for offset in [0, 44, 44 + 200 * 4, 44 + 300 * 4] {
        let m = mounted();
        let mut r = Requests::default();
        let mut fs = Fs::new(&m, wav(300));
        let source = scan(&mut fs, &mut r, &m);
        fs.bytes[offset] ^= 1;
        let mut q = [NormalizedSample::default(); 17];
        let mut cols = [WaveformColumn::default(); 2];
        let mut bytes = [0; 4096];
        let mut pcm = [0.; 512];
        let session = Session::new(
            SessionConfig {
                sample_rate: 8000,
                channel_count: 1,
                total_frames: 300,
                frames_per_column: 256,
            },
            &mut q,
            &mut cols,
        )
        .unwrap();
        let mut w = source.worker(session, &mut bytes).unwrap();
        run(w.open(&mut fs, &r, &m)).unwrap();
        while !run(w.step(&mut fs, &r, &m, &mut pcm)).unwrap() {}
        assert!(matches!(
            run(w.finish(&mut fs, &r, &m)),
            Err(SourceError::Failed)
        ));
        assert_eq!(fs.closes, 2);
        assert!(w.verified_worker().is_err());
        assert!(w.into_worker().is_err());
    }
}
#[test]
fn close_failure_incomplete_pcm_and_nonfinite_input_cannot_release_worker() {
    for case in 0..4 {
        let m = mounted();
        let mut r = Requests::default();
        let mut fs = Fs::new(&m, wav(300));
        if case == 2 {
            fs.bytes[44 + 260 * 4..48 + 260 * 4].copy_from_slice(&f32::NAN.to_le_bytes());
        }
        let source = scan(&mut fs, &mut r, &m);
        let mut q = [NormalizedSample::default(); 17];
        let mut cols = [WaveformColumn::default(); 2];
        let mut bytes = [0; 4096];
        let mut pcm = [0.; 512];
        let session = Session::new(
            SessionConfig {
                sample_rate: 8000,
                channel_count: 1,
                total_frames: 300,
                frames_per_column: 256,
            },
            &mut q,
            &mut cols,
        )
        .unwrap();
        let mut w = source.worker(session, &mut bytes).unwrap();
        run(w.open(&mut fs, &r, &m)).unwrap();
        if case != 1 {
            loop {
                match run(w.step(&mut fs, &r, &m, &mut pcm)) {
                    Ok(true) => break,
                    Ok(false) => (),
                    Err(_) => {
                        assert_eq!(case, 2);
                        break;
                    }
                }
            }
        }
        if case == 0 {
            fs.mode = 5;
        }
        if case == 3 {
            r.cancel();
        }
        assert!(run(w.finish(&mut fs, &r, &m)).is_err());
        assert!(w.verified_worker().is_err());
        assert!(w.into_worker().is_err());
        assert_eq!(fs.closes, 2);
    }
}
#[test]
fn reopen_length_session_geometry_stale_request_and_scan_validation_fail_closed() {
    for case in 0..4 {
        let m = mounted();
        let mut r = Requests::default();
        let mut fs = Fs::new(&m, wav(300));
        let source = scan(&mut fs, &mut r, &m);
        let mut q = [NormalizedSample::default(); 17];
        let mut cols = [WaveformColumn::default(); 2];
        let mut bytes = [0; 4096];
        let session = Session::new(
            SessionConfig {
                sample_rate: if case == 1 { 16000 } else { 8000 },
                channel_count: 1,
                total_frames: 300,
                frames_per_column: 256,
            },
            &mut q,
            &mut cols,
        )
        .unwrap();
        if case == 0 {
            fs.bytes.push(0);
        }
        if case == 2 {
            r.cancel();
        }
        if case == 3 {
            let c = catalog(&m);
            let _ = SourceRequest::new(&mut r, &m, &c, 0).unwrap();
        }
        let prepared = source.worker::<Handle>(session, &mut bytes);
        if case == 1 {
            assert!(prepared.is_err());
        } else {
            let mut w = prepared.unwrap();
            assert!(run(w.open(&mut fs, &r, &m)).is_err());
        }
        assert_eq!(fs.closes, if case == 0 { 2 } else { 1 });
    }
    for case in 0..4 {
        let mut m = mounted();
        let mut r = Requests::default();
        let mut fs = Fs::new(&m, wav(if case == 3 { 0 } else { 300 }));
        let c = catalog(&m);
        let req = SourceRequest::new(&mut r, &m, &c, 0).unwrap();
        let mut rd = run(req.open_stream(&mut fs, &r, &m)).unwrap();
        let mut bytes = [0; 4096];
        if case == 0 {
            fs.bytes[0] = 0;
        }
        if case != 1 {
            loop {
                match run(rd.step(&mut fs, &r, &m, &mut bytes)) {
                    Ok(true) | Err(_) => break,
                    Ok(false) => (),
                }
            }
        }
        if case == 2 {
            m.mark_unmounted();
        }
        assert!(run(rd.finish(&mut fs, &mut r, &m)).is_err());
        assert_eq!(fs.closes, 1);
    }
}
#[test]
fn second_pass_pending_resume_once_and_stale_abort() {
    let m = mounted();
    let mut r = Requests::default();
    let mut fs = Fs::new(&m, wav(300));
    let source = scan(&mut fs, &mut r, &m);
    let mut q = [NormalizedSample::default(); 17];
    let mut cols = [WaveformColumn::default(); 2];
    let mut bytes = [0; 4096];
    let mut pcm = [0.; 512];
    let session = Session::new(
        SessionConfig {
            sample_rate: 8000,
            channel_count: 1,
            total_frames: 300,
            frames_per_column: 256,
        },
        &mut q,
        &mut cols,
    )
    .unwrap();
    let mut w = source.worker(session, &mut bytes).unwrap();
    run(w.open(&mut fs, &r, &m)).unwrap();
    fs.mode = 8;
    let reads = fs.reads;
    {
        use core::{
            pin::pin,
            task::{Context, Poll, Waker},
        };
        let mut f = pin!(w.step(&mut fs, &r, &m, &mut pcm));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(matches!(f.as_mut().poll(&mut cx), Poll::Pending));
        assert!(matches!(f.as_mut().poll(&mut cx), Poll::Ready(Ok(false))));
    }
    assert_eq!(fs.reads, reads + 1);
    r.cancel();
    assert!(matches!(
        run(w.step(&mut fs, &r, &m, &mut pcm)),
        Err(SourceError::Worker(WorkerError::Stale))
    ));
    run(w.abort(&mut fs)).unwrap();
    assert_eq!(fs.closes, 2);
}
#[test]
fn single_byte_stereo_packed24_and_data_before_format_preserve_every_frame() {
    // Reordered chunks, odd ancillary payload, stereo packed24, short final column.
    let mut b = b"RIFF\0\0\0\0WAVEdata".to_vec();
    b.extend(1806u32.to_le_bytes());
    for _ in 0..301 {
        b.extend([0, 0, 64, 0, 0, 64]);
    }
    b.extend(b"JUNK\x01\0\0\0x\0fmt \x10\0\0\0");
    b.extend(1u16.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(8000u32.to_le_bytes());
    b.extend(48000u32.to_le_bytes());
    b.extend(6u16.to_le_bytes());
    b.extend(24u16.to_le_bytes());
    let n = b.len() as u32 - 8;
    b[4..8].copy_from_slice(&n.to_le_bytes());
    let m = mounted();
    let mut r = Requests::default();
    let mut fs = Fs::new(&m, b);
    fs.max = 1;
    let source = scan(&mut fs, &mut r, &m);
    let mut q = [NormalizedSample::default(); 17];
    let mut cols = [WaveformColumn::default(); 2];
    let mut bytes = [0; 4096];
    let mut pcm = [0.; 512];
    let session = Session::new(
        SessionConfig {
            sample_rate: 8000,
            channel_count: 2,
            total_frames: 301,
            frames_per_column: 256,
        },
        &mut q,
        &mut cols,
    )
    .unwrap();
    let mut w = source.worker(session, &mut bytes).unwrap();
    run(w.open(&mut fs, &r, &m)).unwrap();
    let mut steps = 0;
    while !run(w.step(&mut fs, &r, &m, &mut pcm)).unwrap() {
        steps += 1;
        assert!(steps < 10000);
    }
    run(w.finish(&mut fs, &r, &m)).unwrap();
    let mut w = w
        .into_worker()
        .unwrap_or_else(|_| panic!("verified worker"));
    let mut out = [pajoniiir_waveform::WaveformColumn {
        peak: 0,
        color_rgb565: 0,
    }; 2];
    let view = w.waveform(&r, &m, &mut out, 7).unwrap();
    assert_eq!(view.columns.len(), 2);
    // Q15 16384 maps by nearest rounding through the existing 32767 denominator.
    assert_eq!(view.columns[0].peak, 32769);
    assert_eq!(view.columns[1].peak, 32769);
    assert_eq!(fs.closes, 2);
}

#[test]
fn premature_extraction_preserves_handle_and_cancelled_close_never_authorizes_output() {
    for case in 0..3 {
        let m = mounted();
        let mut r = Requests::default();
        let mut fs = Fs::new(&m, wav(300));
        let source = scan(&mut fs, &mut r, &m);
        let mut q = [NormalizedSample::default(); 17];
        let mut cols = [WaveformColumn::default(); 2];
        let mut bytes = [0; 4096];
        let mut pcm = [0.; 512];
        let session = Session::new(
            SessionConfig {
                sample_rate: 8000,
                channel_count: 1,
                total_frames: 300,
                frames_per_column: 256,
            },
            &mut q,
            &mut cols,
        )
        .unwrap();
        let mut w = source.worker(session, &mut bytes).unwrap();
        run(w.open(&mut fs, &r, &m)).unwrap();
        assert!(run(w.open(&mut fs, &r, &m)).is_err());
        let mut w = match w.into_worker() {
            Err(w) => w,
            Ok(_) => panic!("premature output"),
        };
        if case == 0 {
            run(w.abort(&mut fs)).unwrap();
            assert!(run(w.abort(&mut fs)).is_err());
            assert_eq!(fs.closes, 2);
            assert!(w.verified_worker().is_err());
            assert!(w.into_worker().is_err());
            continue;
        }
        while !run(w.step(&mut fs, &r, &m, &mut pcm)).unwrap() {}
        fs.mode = 10;
        {
            use core::{
                pin::pin,
                task::{Context, Poll, Waker},
            };
            let mut future = pin!(w.finish(&mut fs, &r, &m));
            let mut cx = Context::from_waker(Waker::noop());
            assert!(matches!(future.as_mut().poll(&mut cx), Poll::Pending));
            if case == 2 {
                assert!(matches!(future.as_mut().poll(&mut cx), Poll::Ready(Ok(()))));
            }
        }
        assert_eq!(fs.closes, 2);
        if case == 1 {
            assert!(run(w.finish(&mut fs, &r, &m)).is_err());
            assert!(w.verified_worker().is_err());
            assert!(w.into_worker().is_err());
        } else {
            assert!(w.verified_worker().is_ok());
            assert!(w.into_worker().is_ok());
        }
        assert_eq!(fs.closes, 2);
    }
}

#[test]
fn original_lease_survives_backend_replacement_during_both_passes() {
    for second in [false, true] {
        for mode in [11, 12, 13] {
            let m = mounted();
            let mut r = Requests::default();
            let mut fs = Fs::new(&m, wav(300));
            let mut bytes = [0; 4096];
            if second {
                let source = scan(&mut fs, &mut r, &m);
                let mut queue = [NormalizedSample::default(); 17];
                let mut columns = [WaveformColumn::default(); 2];
                let session = Session::new(
                    SessionConfig {
                        sample_rate: 8000,
                        channel_count: 1,
                        total_frames: 300,
                        frames_per_column: 256,
                    },
                    &mut queue,
                    &mut columns,
                )
                .unwrap();
                let mut w = source.worker(session, &mut bytes).unwrap();
                if mode == 11 {
                    fs.mode = mode;
                }
                let opened = run(w.open(&mut fs, &r, &m));
                if mode == 11 {
                    assert!(opened.is_err(), "replacement during second open");
                } else {
                    opened.unwrap();
                    let mut pcm = [0.; 512];
                    if mode == 12 {
                        fs.mode = mode;
                        assert!(run(w.step(&mut fs, &r, &m, &mut pcm)).is_err());
                        run(w.abort(&mut fs)).unwrap();
                    } else {
                        while !run(w.step(&mut fs, &r, &m, &mut pcm)).unwrap() {}
                        fs.mode = mode;
                        assert!(run(w.finish(&mut fs, &r, &m)).is_err());
                    }
                }
                assert!(w.verified_worker().is_err());
                assert_eq!(fs.closes, 2);
            } else {
                let c = catalog(&m);
                let request = SourceRequest::new(&mut r, &m, &c, 0).unwrap();
                if mode == 11 {
                    fs.mode = mode;
                }
                let opened = run(request.open_stream(&mut fs, &r, &m));
                if mode == 11 {
                    assert!(opened.is_err(), "replacement during first open");
                } else {
                    let mut scan = opened.unwrap();
                    if mode == 12 {
                        fs.mode = mode;
                        assert!(run(scan.step(&mut fs, &r, &m, &mut bytes)).is_err());
                        run(scan.abort(&mut fs)).unwrap();
                    } else {
                        while !run(scan.step(&mut fs, &r, &m, &mut bytes)).unwrap() {}
                        fs.mode = mode;
                        assert!(run(scan.finish(&mut fs, &mut r, &m)).is_err());
                    }
                }
                assert_eq!(fs.closes, 1);
            }
        }
    }
}

#[test]
fn local_tempo_preparation_preserves_occupied_handle_and_empty_slot_on_error() {
    use libapta::analysis::{BIN_CAPACITY, OnsetBin};
    use pajoniiir_apta_adapter::storage::LocalTempoBuffers;
    for occupied in [false, true] {
        let m = mounted();
        let mut r = Requests::default();
        let mut fs = Fs::new(&m, wav(300));
        let source = scan(&mut fs, &mut r, &m);
        let mut queue = [NormalizedSample::default(); 17];
        let mut columns = [WaveformColumn::default(); 2];
        let mut bytes = [0; 4096];
        let session = Session::new(
            SessionConfig {
                sample_rate: 8000,
                channel_count: 1,
                total_frames: 300,
                frames_per_column: 256,
            },
            &mut queue,
            &mut columns,
        )
        .unwrap();
        let mut slot = if occupied {
            Some(source.worker(session, &mut bytes).unwrap())
        } else {
            None
        };
        if let Some(worker) = &mut slot {
            run(worker.open(&mut fs, &r, &m)).unwrap();
        }
        let source = scan(&mut fs, &mut r, &m);
        let mut queue2 = [NormalizedSample::default(); 17];
        let mut columns2 = [WaveformColumn::default(); 2];
        let mut bytes2 = [0; 4096];
        let mut bins = vec![OnsetBin::default(); BIN_CAPACITY];
        let mut flux = vec![0.; BIN_CAPACITY];
        let result = source.prepare_local_tempo(
            &mut slot,
            LocalTempoBuffers {
                queue: &mut queue2,
                columns: &mut columns2,
                bins: &mut bins,
                flux: &mut flux,
            },
            3,
            &mut bytes2,
        ); // invalid geometry; occupied slot must be rejected first
        assert_eq!(
            result,
            Err(if occupied {
                WorkerError::Failed
            } else {
                WorkerError::Core(libapta::Error::InvalidArgument)
            })
        );
        if occupied {
            let worker = slot.as_mut().unwrap();
            assert!(worker.verified_worker().is_err());
            run(worker.abort(&mut fs)).unwrap();
            assert!(run(worker.abort(&mut fs)).is_err());
            assert_eq!(fs.closes, 3);
        } else {
            assert!(slot.is_none());
            assert_eq!(fs.closes, 2);
        }
    }
}

#[test]
fn cooperative_run_yields_wakes_and_retains_control_after_cancellation() {
    use core::{
        pin::pin,
        task::{Context, Poll},
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct WakeCount(AtomicUsize);
    impl std::task::Wake for WakeCount {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    // Pending reads include a backend that advances its cursor before Pending.
    for mode in [0, 8, 9] {
        for cancel in [false, true] {
            if mode == 9 && !cancel {
                continue;
            }
            let m = mounted();
            let mut r = Requests::default();
            let mut fs = Fs::new(&m, wav(300));
            let source = scan(&mut fs, &mut r, &m);
            let mut q = [NormalizedSample::default(); 17];
            let mut cols = [WaveformColumn::default(); 2];
            let mut bytes = [0; 4096];
            let mut pcm = [0.; 512];
            let session = Session::new(
                SessionConfig {
                    sample_rate: 8000,
                    channel_count: 1,
                    total_frames: 300,
                    frames_per_column: 256,
                },
                &mut q,
                &mut cols,
            )
            .unwrap();
            let mut w = source.worker(session, &mut bytes).unwrap();
            let wake = Arc::new(WakeCount(AtomicUsize::new(0)));
            let waker = std::task::Waker::from(wake.clone());
            let mut cx = Context::from_waker(&waker);
            fs.mode = mode;
            let reads = fs.reads;
            allocation::start();
            {
                let mut f = pin!(w.run(&mut fs, &r, &m, &mut pcm));
                assert!(matches!(f.as_mut().poll(&mut cx), Poll::Pending));
                if mode == 0 {
                    assert_eq!(wake.0.load(Ordering::Relaxed), 1);
                }
                if !cancel {
                    let mut finished = false;
                    for _ in 0..1000 {
                        if let Poll::Ready(value) = f.as_mut().poll(&mut cx) {
                            value.unwrap();
                            finished = true;
                            break;
                        }
                    }
                    assert!(finished);
                }
            }
            if cancel {
                assert_eq!(fs.reads, reads + 1);
                fs.mode = 0;
                assert!(run(w.step(&mut fs, &r, &m, &mut pcm)).is_err());
                assert!(w.verified_worker().is_err());
                run(w.abort(&mut fs)).unwrap();
                assert!(run(w.abort(&mut fs)).is_err());
            } else {
                assert!(w.verified_worker().is_ok());
                assert!(wake.0.load(Ordering::Relaxed) > 1);
            }
            assert_eq!(fs.closes, 2);
            allocation::finish();
        }
    }
}

#[test]
fn cooperative_run_closes_on_step_error_and_reports_cleanup_failure() {
    for close_failure in [false, true] {
        let m = mounted();
        let mut r = Requests::default();
        let mut fs = Fs::new(&m, wav(300));
        let c = catalog(&m);
        let request = SourceRequest::new(&mut r, &m, &c, 0).unwrap();
        let mut bytes = [0; 4096];
        let mut read = run(request.open_stream_with_limits(
            &mut fs,
            &r,
            &m,
            pajoniiir_apta_adapter::streaming::StreamLimits {
                processing_steps: 0,
                ..pajoniiir_apta_adapter::streaming::StreamLimits::UNLIMITED
            },
        ))
        .unwrap();
        while !run(read.step(&mut fs, &r, &m, &mut bytes)).unwrap() {}
        let source = run(read.finish(&mut fs, &mut r, &m)).unwrap();
        let mut q = [NormalizedSample::default(); 17];
        let mut cols = [WaveformColumn::default(); 2];
        let session = Session::new(
            SessionConfig {
                sample_rate: 8000,
                channel_count: 1,
                total_frames: 300,
                frames_per_column: 256,
            },
            &mut q,
            &mut cols,
        )
        .unwrap();
        let mut w = source.worker(session, &mut bytes).unwrap();
        if close_failure {
            fs.mode = 5;
        }
        let mut pcm = [0.; 512];
        let error = drive(w.run(&mut fs, &r, &m, &mut pcm)).unwrap_err();
        if close_failure {
            assert!(matches!(error, SourceError::Io(Fault::Io)));
        } else {
            assert!(matches!(error, SourceError::Limit));
        }
        assert_eq!(fs.closes, 2);
        assert!(w.verified_worker().is_err());
        assert!(run(w.abort(&mut fs)).is_err());
        assert_eq!(fs.closes, 2);
    }
}

#[test]
fn cooperative_scan_retains_handle_and_poison_on_cancelled_yield_or_read() {
    for mode in [0, 8, 9] {
        for finish in [false, true] {
            let m = mounted();
            let c = catalog(&m);
            let mut r = Requests::default();
            let mut fs = Fs::new(&m, wav(3000));
            let request = SourceRequest::new(&mut r, &m, &c, 0).unwrap();
            let mut scan = run(request.open_stream(&mut fs, &r, &m)).unwrap();
            let mut bytes = [0; 4096];
            fs.mode = mode;
            assert_pending(scan.run(&mut fs, &r, &m, &mut bytes));
            assert_eq!(fs.reads, 1);
            assert_eq!(fs.closes, 0);
            fs.mode = 0;
            assert!(matches!(
                run(scan.run(&mut fs, &r, &m, &mut bytes)),
                Err(SourceError::Failed)
            ));
            assert_eq!(fs.reads, 1);
            if finish {
                assert!(run(scan.finish(&mut fs, &mut r, &m)).is_err());
            } else {
                run(scan.abort(&mut fs)).unwrap();
            }
            assert_eq!(fs.closes, 1);
        }
    }
}

#[test]
fn cooperative_scan_wakes_completes_exact_budget_and_still_requires_close() {
    use core::{
        pin::pin,
        task::{Context, Poll},
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct WakeCount(AtomicUsize);
    impl std::task::Wake for WakeCount {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    for mode in [0, 8] {
        let m = mounted();
        let c = catalog(&m);
        let mut r = Requests::default();
        let mut fs = Fs::new(&m, wav(3000));
        fs.mode = mode;
        let expected = pajoniiir_media_identity::sha256(&fs.bytes).0;
        let steps = fs.bytes.len().div_ceil(fs.max);
        let request = SourceRequest::new(&mut r, &m, &c, 0).unwrap();
        let mut scan = run(request.open_stream_with_limits(
            &mut fs,
            &r,
            &m,
            pajoniiir_apta_adapter::streaming::StreamLimits {
                scan_steps: steps as u64,
                ..pajoniiir_apta_adapter::streaming::StreamLimits::UNLIMITED
            },
        ))
        .unwrap();
        let mut bytes = [0; 4096];
        let wakes = Arc::new(WakeCount(AtomicUsize::new(0)));
        let waker = wakes.clone().into();
        let mut cx = Context::from_waker(&waker);
        allocation::start();
        let mut pending = 0;
        {
            let mut future = pin!(scan.run(&mut fs, &r, &m, &mut bytes));
            loop {
                match future.as_mut().poll(&mut cx) {
                    Poll::Ready(result) => {
                        result.unwrap();
                        break;
                    }
                    Poll::Pending => {
                        pending += 1;
                        assert!(pending < 1000);
                    }
                }
            }
        }
        assert_eq!(pending, steps - 1 + if mode == 8 { steps } else { 0 });
        assert_eq!(wakes.0.load(Ordering::Relaxed), pending);
        assert_eq!(fs.reads, steps);
        assert_eq!(fs.closes, 0);
        run(scan.run(&mut fs, &r, &m, &mut bytes)).unwrap();
        assert_eq!(fs.reads, steps);
        let source = run(scan.finish(&mut fs, &mut r, &m)).unwrap();
        assert_eq!(source.source().fingerprint, expected);
        assert_eq!(fs.closes, 1);
        allocation::finish();
    }
}

#[test]
fn cooperative_scan_errors_leave_cleanup_to_owner_without_spending_more_io() {
    for case in 0..4 {
        let m = mounted();
        let c = catalog(&m);
        let mut r = Requests::default();
        let mut fs = Fs::new(&m, wav(3000));
        let request = SourceRequest::new(&mut r, &m, &c, 0).unwrap();
        let mut scan = run(request.open_stream_with_limits(
            &mut fs,
            &r,
            &m,
            pajoniiir_apta_adapter::streaming::StreamLimits {
                scan_steps: if case == 0 { 1 } else { 100 },
                ..pajoniiir_apta_adapter::streaming::StreamLimits::UNLIMITED
            },
        ))
        .unwrap();
        match case {
            1 => fs.mode = 1,
            2 => fs.bytes[0] = 0,
            3 => {
                r.cancel();
            }
            _ => (),
        }
        let mut bytes = [0; 4096];
        assert!(drive(scan.run(&mut fs, &r, &m, &mut bytes)).is_err());
        let reads = fs.reads;
        assert_eq!(fs.closes, 0);
        assert!(matches!(
            run(scan.run(&mut fs, &r, &m, &mut bytes)),
            Err(SourceError::Failed)
        ));
        assert_eq!(fs.reads, reads);
        fs.mode = 0;
        run(scan.abort(&mut fs)).unwrap();
        assert_eq!(fs.closes, 1);
    }
}

#[test]
fn pcm_cursor_preserves_all_formats_across_partial_frames_and_short_queues() {
    use libapta::{
        session::{CancellationToken, WorkBudget},
        wav::Wav,
    };
    for (bits, float) in [(16, false), (24, false), (32, false), (32, true)] {
        for channels in [1, 2] {
            for extensible in [false, true] {
                let original = wav_profile(513, bits, channels, float, extensible);
                let wav = Wav::parse(&original).unwrap();
                let mut samples = vec![0.; 513 * channels as usize];
                assert_eq!(wav.read_frames(0, &mut samples), Ok(513));
                let config = SessionConfig {
                    sample_rate: 8000,
                    channel_count: channels,
                    total_frames: 513,
                    frames_per_column: 64,
                };
                let mut expected = [WaveformColumn::default(); 9];
                let mut reference_queue = [NormalizedSample::default(); 513];
                {
                    let mut s = Session::new(config, &mut reference_queue, &mut expected).unwrap();
                    assert_eq!(s.push_interleaved(&samples), Ok(513));
                    s.finish_input().unwrap();
                    s.process(WorkBudget::default(), &CancellationToken::new())
                        .unwrap();
                }
                for read_size in [1, 5, 4093, 4096] {
                    for queue_size in [1, 17] {
                        let media = mounted();
                        let mut requests = Requests::default();
                        let mut fs = Fs::new(&media, original.clone());
                        fs.max = read_size;
                        let mut bytes = [0; 4096];
                        let mut pcm = [0.; 512];
                        let mut queue = [NormalizedSample::default(); 17];
                        let mut actual = [WaveformColumn::default(); 9];
                        let catalog = catalog(&media);
                        allocation::start();
                        {
                            let request =
                                SourceRequest::new(&mut requests, &media, &catalog, 0).unwrap();
                            let mut scanner = None;
                            let source = drive(request.scan(
                                &mut scanner,
                                &mut fs,
                                &mut requests,
                                &media,
                                pajoniiir_apta_adapter::streaming::StreamLimits::UNLIMITED,
                                &mut bytes,
                            ))
                            .unwrap();
                            let session =
                                Session::new(config, &mut queue[..queue_size], &mut actual)
                                    .unwrap();
                            let mut worker = source.worker(session, &mut bytes).unwrap();
                            drive(worker.run(&mut fs, &requests, &media, &mut pcm)).unwrap();
                            assert!(worker.verified_worker().is_ok());
                        }
                        assert_eq!(fs.closes, 2);
                        assert_eq!(
                            actual, expected,
                            "{bits} {channels} {extensible} {read_size} {queue_size}"
                        );
                        allocation::finish();
                    }
                }
            }
        }
    }
}
