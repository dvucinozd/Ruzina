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
