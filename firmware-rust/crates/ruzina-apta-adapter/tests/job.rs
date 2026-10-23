#[path = "support/allocation.rs"]
mod allocation;
#[allow(dead_code)]
mod support;
use core::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};
use libapta::{
    WaveformColumn,
    analysis::{BIN_CAPACITY, OnsetBin},
    waveform::NormalizedSample,
};
use ruzina_apta_adapter::worker::Requests;
use ruzina_apta_adapter::{
    source::{SourceError, SourceRequest},
    storage::LocalTempoBuffers,
    streaming::StreamLimits,
};
use support::*;

#[test]
fn scan_composition_closes_errors_and_preserves_occupied_or_cancelled_control() {
    for mode in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13] {
        let media = mounted();
        let catalog = catalog(&media);
        let mut requests = Requests::default();
        let mut fs = Fs::new(&media, wav(300));
        fs.mode = mode;
        let mut bytes = [0; 4096];
        let mut slot = None;
        let request = SourceRequest::new(&mut requests, &media, &catalog, 0).unwrap();
        allocation::start();
        if [0, 4, 8, 9].contains(&mode) {
            {
                let mut f = pin!(request.scan(
                    &mut slot,
                    &mut fs,
                    &mut requests,
                    &media,
                    StreamLimits::UNLIMITED,
                    &mut bytes
                ));
                assert!(matches!(
                    f.as_mut().poll(&mut Context::from_waker(Waker::noop())),
                    Poll::Pending
                ));
            }
            let reads = fs.reads;
            // A new request cannot overwrite the still-owned poisoned handle.
            let request = SourceRequest::new(&mut requests, &media, &catalog, 0).unwrap();
            assert!(matches!(
                drive(request.scan(
                    &mut slot,
                    &mut fs,
                    &mut requests,
                    &media,
                    StreamLimits::UNLIMITED,
                    &mut bytes
                )),
                Err(SourceError::Failed)
            ));
            assert_eq!((fs.reads, fs.closes), (reads, 0));
            fs.mode = 0; // mock backend has no pending ticket/DMA to drain
            assert!(matches!(
                run(slot
                    .as_mut()
                    .unwrap()
                    .step(&mut fs, &requests, &media, &mut bytes)),
                Err(SourceError::Failed)
            ));
            run(slot.take().unwrap().abort(&mut fs)).unwrap();
        } else {
            let result = drive(request.scan(
                &mut slot,
                &mut fs,
                &mut requests,
                &media,
                StreamLimits::UNLIMITED,
                &mut bytes,
            ));
            assert_eq!(result.is_ok(), mode == 10); // close really suspends and resumes
            assert!(slot.is_none());
        }
        assert_eq!(fs.closes, 1);
        allocation::finish();
    }
}

#[test]
fn scan_composition_preserves_limits_and_cleanup_error_precedence() {
    for mode in [0, 5] {
        for kind in 0..4 {
            let media = mounted();
            let catalog = catalog(&media);
            let mut requests = Requests::default();
            let mut fs = Fs::new(&media, wav(300));
            fs.mode = mode;
            let mut bytes = [0; 4096];
            let mut slot = None;
            let mut limits = StreamLimits::UNLIMITED;
            match kind {
                0 => limits.object_bytes = 1,
                1 => limits.scan_steps = 0,
                2 => limits.duration_ms = 0,
                _ => fs.bytes[0] = 0,
            }
            let request = SourceRequest::new(&mut requests, &media, &catalog, 0).unwrap();
            allocation::start();
            let error = drive(request.scan(
                &mut slot,
                &mut fs,
                &mut requests,
                &media,
                limits,
                &mut bytes,
            ))
            .err()
            .unwrap();
            if mode == 5 {
                assert!(matches!(error, SourceError::Io(Fault::Io)));
            } else if kind == 3 {
                assert!(matches!(error, SourceError::Decode(_)));
            } else {
                assert!(matches!(error, SourceError::Limit));
            }
            assert!(slot.is_none());
            assert_eq!(fs.closes, 1);
            if kind < 2 {
                assert_eq!(fs.reads, 0);
            }
            allocation::finish();
        }
    }
}

#[test]
fn second_pass_composition_cancellation_retains_control_and_closes_failures() {
    for mode in [0, 1, 4, 5, 8, 9, 10, 11, 12, 13] {
        let media = mounted();
        let catalog = catalog(&media);
        let mut requests = Requests::default();
        let mut fs = Fs::new(&media, wav(300));
        let request = SourceRequest::new(&mut requests, &media, &catalog, 0).unwrap();
        let mut bytes = [0; 4096];
        let mut pcm = [0.; 512];
        let mut scan = None;
        let mut slot = None;
        let source = drive(request.scan(
            &mut scan,
            &mut fs,
            &mut requests,
            &media,
            StreamLimits::UNLIMITED,
            &mut bytes,
        ))
        .unwrap();
        let mut queue = [NormalizedSample::default(); 17];
        let mut columns = [WaveformColumn::default(); 2];
        let mut bins = vec![OnsetBin::default(); BIN_CAPACITY];
        let mut flux = vec![0.; BIN_CAPACITY];
        fs.mode = mode;
        allocation::start();
        {
            source
                .prepare_local_tempo(
                    &mut slot,
                    LocalTempoBuffers {
                        queue: &mut queue,
                        columns: &mut columns,
                        bins: &mut bins,
                        flux: &mut flux,
                    },
                    256,
                    &mut bytes,
                )
                .unwrap();
            let f = slot
                .as_mut()
                .unwrap()
                .run(&mut fs, &requests, &media, &mut pcm);
            if [0, 4, 8, 9].contains(&mode) {
                let mut f = pin!(f);
                assert!(matches!(
                    f.as_mut().poll(&mut Context::from_waker(Waker::noop())),
                    Poll::Pending
                ));
            } else {
                assert_eq!(drive(f).is_ok(), mode == 10);
            }
        }
        let worker = slot.as_mut().unwrap();
        if [0, 4, 8, 9].contains(&mode) {
            assert_eq!(fs.closes, 1);
            assert!(worker.verified_worker().is_err());
            fs.mode = 0;
            assert!(run(worker.step(&mut fs, &requests, &media, &mut pcm)).is_err());
            run(worker.abort(&mut fs)).unwrap();
        } else {
            assert_eq!(worker.verified_worker().is_ok(), mode == 10);
        }
        assert_eq!(fs.closes, 2);
        assert!(run(worker.abort(&mut fs)).is_err()); // no second close
        assert_eq!(fs.closes, 2);
        allocation::finish();
    }
}

#[test]
fn composed_jobs_reuse_arrays_and_persistent_requests_after_closed_control_scope() {
    let media = mounted();
    let catalog = catalog(&media);
    let mut requests = Requests::default();
    let control = std::rc::Rc::new(support::broker::Control::default());
    control.yield_receive.set(true);
    let broker = support::broker::Broker::new(Fs::new(&media, wav(300)), control.clone());
    let mut fs = ruzina_media_fs::analysis::Client::new(
        broker,
        media.lease().unwrap(),
        Box::leak(Box::new([0; 4096])),
    );
    let mut bytes = [0; 4096];
    let mut pcm = [0.; 512];
    let mut queue = [NormalizedSample::default(); 17];
    let mut columns = [WaveformColumn::default(); 2];
    let mut bins = vec![OnsetBin::default(); BIN_CAPACITY];
    let mut flux = vec![0.; BIN_CAPACITY];
    let mut bank = [];
    allocation::start();
    for generation in 1..=3 {
        let request = SourceRequest::new(&mut requests, &media, &catalog, 0).unwrap();
        let mut scan = None;
        let mut slot = None;
        let source = drive(request.scan(
            &mut scan,
            &mut fs,
            &mut requests,
            &media,
            StreamLimits::UNLIMITED,
            &mut bytes,
        ))
        .unwrap();
        assert!(scan.is_none());
        source
            .prepare_local_tempo(
                &mut slot,
                LocalTempoBuffers {
                    queue: &mut queue,
                    columns: &mut columns,
                    bins: &mut bins,
                    flux: &mut flux,
                },
                256,
                &mut bytes,
            )
            .unwrap();
        drive(
            slot.as_mut()
                .unwrap()
                .run(&mut fs, &requests, &media, &mut pcm),
        )
        .unwrap();
        let done = slot
            .as_mut()
            .unwrap()
            .verified_worker()
            .unwrap()
            .complete(&requests, &media, &mut bank)
            .unwrap();
        let pin = ruzina_apta_adapter::worker::Pin::load(
            ruzina_core::DeckId::One,
            &ruzina_deck::DeckProductState::new(),
            done,
            &requests,
            &media,
        )
        .unwrap();
        assert_eq!(pin.track(&media).unwrap().generation(), generation);
        // Leaving this iteration releases closed control and its array borrows.
        assert_eq!(control.closes.get(), generation as usize * 2);
        assert!(fs.is_idle());
    }
    allocation::finish();
    support::broker::free_buffer(fs.retire_buffer().unwrap());
}
