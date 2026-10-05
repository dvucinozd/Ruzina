#[path = "support/allocation.rs"]
mod allocation;
mod support;
use libapta::{
    WaveformColumn,
    analysis::{BIN_CAPACITY, OnsetBin},
    session::{Session, SessionConfig},
    waveform::NormalizedSample,
};
use pajoniiir_apta_adapter::{
    source::{SourceError, SourceRequest},
    worker::{Pin, Requests, WorkerError},
};
use pajoniiir_core::DeckId;
use pajoniiir_deck::DeckProductState;
use pajoniiir_media_fs::AsyncFileSystem;
use pajoniiir_track_analysis::Beat;
use support::*;
const EMPTY: Beat = Beat {
    time_ms: 0,
    phase: 0,
    bpm_x100: 0,
};

fn analyze<F: AsyncFileSystem>(
    fs: F,
    media: &pajoniiir_media_session::MediaSession,
    original: &[u8],
) where
    F::Error: core::fmt::Debug,
{
    let control = std::rc::Rc::new(support::broker::Control::default());
    let broker = support::broker::Broker::new(fs, control.clone());
    let mut fs = pajoniiir_media_fs::analysis::Client::new(
        broker,
        media.lease().unwrap(),
        Box::leak(Box::new([0; 4096])),
    );
    let snapshot = catalog(media);
    let mut catalog = pajoniiir_media_catalog::CatalogBankBuilder::new(
        snapshot.lease(),
        Bank(Vec::with_capacity(1)),
    );
    catalog.push(*snapshot.get(0).unwrap()).unwrap();
    let catalog = catalog.finish(1);
    let mut requests = Requests::default();
    let mut bytes = vec![0; original.len()];
    allocation::start();
    let request = SourceRequest::from_bank(&mut requests, media, &catalog, 0).unwrap();
    let mut read = run(request.open(&mut fs, &requests, media, &mut bytes)).unwrap();
    let mut steps = 0;
    while !run(read.step(&mut fs, &requests, media)).unwrap() {
        steps += 1;
        assert!(steps < 20000);
    }
    let source = run(read.finish(&mut fs, &mut requests, media)).unwrap();
    allocation::finish();
    assert_eq!(
        source.source().fingerprint,
        pajoniiir_media_identity::sha256(original).0
    );
    assert_eq!(source.source().fingerprint_kind, 2);
    let s = source.source();
    let plan = pajoniiir_apta_adapter::storage::LocalTempoStorage::plan(
        s.total_frames.unwrap(),
        256,
        17,
        128,
        108772,
    )
    .unwrap();
    assert_eq!(plan.total_bytes, 108772);
    assert_eq!(plan.overview_columns, 1250);
    let mut queue = [NormalizedSample::default(); 17]; // force partial acceptance
    let mut columns = vec![WaveformColumn::default(); 1250];
    let mut bins = vec![OnsetBin::default(); BIN_CAPACITY];
    let mut flux = vec![0.; BIN_CAPACITY];
    let mut session = Session::new(
        SessionConfig {
            sample_rate: s.sample_rate,
            channel_count: s.channel_count,
            total_frames: s.total_frames.unwrap(),
            frames_per_column: 256,
        },
        &mut queue,
        &mut columns,
    )
    .unwrap();
    session.enable_tempo(&mut bins, &mut flux).unwrap();
    session.enable_meter().unwrap();
    let mut bank = [EMPTY; 128];
    let mut render = vec![
        pajoniiir_waveform::WaveformColumn {
            peak: 0,
            color_rgb565: 0
        };
        1250
    ];
    let (completed, view) = {
        allocation::start();
        let mut worker = source.worker(session).unwrap();
        let mut scratch = [0.; 512];
        let mut done = false;
        for _ in 0..40000 {
            if worker.step(&requests, media, &mut scratch).unwrap() {
                done = true;
                break;
            }
        }
        assert!(done);
        let completed = worker.complete(&requests, media, &mut bank).unwrap();
        let view = worker
            .waveform(&requests, media, &mut render, 0x07e0)
            .unwrap();
        allocation::finish();
        (completed, view)
    };
    drop(bytes);

    verify_consumers(
        completed,
        view,
        &requests,
        media,
        if libapta::wav::Wav::parse(original).unwrap().encoding() == libapta::wav::Encoding::S16 {
            4160
        } else {
            4120
        },
    );
    let decks = DeckProductState::new();
    let selected = Pin::load(DeckId::One, &decks, completed, &requests, media).unwrap();
    analyze_stream(
        &mut fs,
        media,
        original,
        selected.track(media).unwrap(),
        view,
        &mut requests,
    );
    assert!(fs.is_idle());
    // Reuse this same actual native filesystem endpoint through the persistent
    // owner, with retained output independent of its reusable working arrays.
    let mut work = support::owner::Work::new();
    let mut a = support::owner::Bank::new();
    let mut b = support::owner::Bank::new();
    let mut owner = pajoniiir_apta_adapter::owner::Owner::new(fs, requests, work.borrow());
    allocation::start();
    owner.select_bank(media, &catalog, 0).unwrap();
    let first = drive(owner.run(
        media,
        pajoniiir_apta_adapter::streaming::StreamLimits::UNLIMITED,
        256,
        a.borrow(),
        0x07e0,
    ))
    .unwrap();
    let pin = Pin::load(
        DeckId::One,
        &decks,
        first.completed,
        owner.requests(),
        media,
    )
    .unwrap();
    owner.select_bank(media, &catalog, 0).unwrap();
    let second = drive(owner.run(
        media,
        pajoniiir_apta_adapter::streaming::StreamLimits::UNLIMITED,
        256,
        b.borrow(),
        0x07e0,
    ))
    .unwrap();
    let next = Pin::load(
        DeckId::One,
        &decks,
        second.completed,
        owner.requests(),
        media,
    )
    .unwrap();
    assert_eq!(pin.track(media).unwrap().generation(), 3);
    assert_eq!(next.track(media).unwrap().generation(), 4);
    assert_eq!(first.waveform.columns, second.waveform.columns);
    assert_eq!(view.columns, second.waveform.columns);
    assert_eq!(
        selected.track(media).unwrap().beat_grid().unwrap().beats(),
        next.track(media).unwrap().beat_grid().unwrap().beats()
    );
    allocation::finish();
    verify_consumers(
        second.completed,
        second.waveform,
        owner.requests(),
        media,
        if libapta::wav::Wav::parse(original).unwrap().encoding() == libapta::wav::Encoding::S16 {
            4160
        } else {
            4120
        },
    );
    assert!(owner.is_idle());
    // Run the actual long-lived handoff loop on every native FAT32/exFAT profile.
    use pajoniiir_apta_adapter::service::{Command, Selection, Service};
    let selection = Selection::from_bank(&catalog, 0).unwrap();
    let mut service = Service::new(owner, [a.borrow(), b.borrow()]);
    let state = std::rc::Rc::new(support::service::State::new(*media));
    let mut port = support::service::HostPort(state.clone());
    assert!(state.send(Command::Analyze(selection, DeckId::One)).is_ok());
    allocation::start();
    {
        let mut task = core::pin::pin!(service.run(
            &mut port,
            pajoniiir_apta_adapter::streaming::StreamLimits::UNLIMITED,
            256,
            0x07e0
        ));
        for expected in 1..=2 {
            for _ in 0..200_000 {
                assert!(
                    core::future::Future::poll(
                        task.as_mut(),
                        &mut core::task::Context::from_waker(core::task::Waker::noop())
                    )
                    .is_pending()
                );
                if state.published.get() == expected {
                    break;
                }
            }
            assert_eq!(state.published.get(), expected);
            let outputs = state.outputs.borrow();
            let output = outputs.back().unwrap();
            let track = output.track(media).unwrap();
            assert_eq!(track.generation(), 4 + expected as u32);
            assert_eq!(
                track.beat_grid().unwrap().beats(),
                selected.track(media).unwrap().beat_grid().unwrap().beats()
            );
            assert_eq!(output.waveform(media).unwrap().columns, view.columns);
            if expected == 1 {
                assert!(state.send(Command::Analyze(selection, DeckId::Two)).is_ok());
            }
        }
    }
    allocation::finish();
    for output in state.outputs.borrow().iter() {
        verify_neutral(
            output.track(media).unwrap(),
            output.waveform(media).unwrap(),
            if libapta::wav::Wav::parse(original).unwrap().encoding() == libapta::wav::Encoding::S16
            {
                4160
            } else {
                4120
            },
        );
    }
    assert!(service.owner_mut().is_idle());
    support::broker::free_buffer(service.owner_mut().retire_buffer().unwrap());
}
fn analyze_stream<F: AsyncFileSystem>(
    fs: &mut F,
    media: &pajoniiir_media_session::MediaSession,
    original: &[u8],
    expected: pajoniiir_track_analysis::TrackAnalysis<'_>,
    expected_waveform: pajoniiir_waveform::WaveformView<'_>,
    requests: &mut Requests,
) where
    F::Error: core::fmt::Debug,
{
    let snapshot = catalog(media);
    let mut catalog = pajoniiir_media_catalog::CatalogBankBuilder::new(
        snapshot.lease(),
        Bank(Vec::with_capacity(1)),
    );
    catalog.push(*snapshot.get(0).unwrap()).unwrap();
    let catalog = catalog.finish(1);
    let mut bytes = [0; 4096];
    allocation::start();
    let request = SourceRequest::from_bank(requests, media, &catalog, 0).unwrap();
    let limits = pajoniiir_apta_adapter::streaming::StreamLimits {
        object_bytes: original.len() as u64,
        duration_ms: 40_000,
        scan_steps: 20_000,
        processing_steps: 40_000,
    };
    let mut scan = None;
    let source = drive(request.scan(&mut scan, fs, requests, media, limits, &mut bytes)).unwrap();
    assert!(scan.is_none());
    allocation::finish();
    assert_eq!(
        source.source().fingerprint,
        pajoniiir_media_identity::sha256(original).0
    );
    assert_eq!(source.source().fingerprint_kind, 2);
    let s = source.source();
    let plan = pajoniiir_apta_adapter::storage::LocalTempoStorage::plan(
        s.total_frames.unwrap(),
        256,
        17,
        128,
        108772,
    )
    .unwrap();
    assert_eq!(plan.total_bytes, 108772);
    assert_eq!(plan.overview_columns, 1250);
    let mut queue = [NormalizedSample::default(); 17]; // force partial acceptance
    let mut columns = vec![WaveformColumn::default(); 1250];
    let mut bins = vec![OnsetBin::default(); BIN_CAPACITY];
    let mut flux = vec![0.; BIN_CAPACITY];
    let mut bank = [EMPTY; 128];
    let mut render = vec![
        pajoniiir_waveform::WaveformColumn {
            peak: 0,
            color_rgb565: 0
        };
        1250
    ];
    let (completed, view) = {
        allocation::start();
        let mut slot = None;
        source
            .prepare_local_tempo(
                &mut slot,
                pajoniiir_apta_adapter::storage::LocalTempoBuffers {
                    queue: &mut queue,
                    columns: &mut columns,
                    bins: &mut bins,
                    flux: &mut flux,
                },
                256,
                &mut bytes,
            )
            .unwrap();
        let worker = slot.as_mut().unwrap();
        let mut scratch = [0.; 512];
        drive(worker.run(fs, requests, media, &mut scratch)).unwrap();
        let worker = worker.verified_worker().unwrap();
        let completed = worker.complete(requests, media, &mut bank).unwrap();
        let view = worker
            .waveform(requests, media, &mut render, 0x07e0)
            .unwrap();
        allocation::finish();
        (completed, view)
    };

    assert_eq!(view.columns, expected_waveform.columns);
    let decks = DeckProductState::new();
    let new = Pin::load(DeckId::One, &decks, completed, requests, media).unwrap();
    let actual = new.track(media).unwrap();
    assert_eq!(expected.provider(), actual.provider());
    assert_eq!(expected.generation() + 1, actual.generation());
    assert_eq!(expected.bpm_x100(), actual.bpm_x100());
    assert_eq!(
        expected.beat_grid().unwrap().beats(),
        actual.beat_grid().unwrap().beats()
    );
    verify_consumers(
        completed,
        view,
        requests,
        media,
        if libapta::wav::Wav::parse(original).unwrap().encoding() == libapta::wav::Encoding::S16 {
            4160
        } else {
            4120
        },
    );
}
fn verify_consumers(
    completed: pajoniiir_apta_adapter::worker::Completed<'_>,
    view: pajoniiir_waveform::WaveformView<'_>,
    requests: &Requests,
    media: &pajoniiir_media_session::MediaSession,
    expected_pixels: usize,
) {
    let decks = DeckProductState::new();
    let pin = Pin::load(DeckId::One, &decks, completed, requests, media).unwrap();
    let track = pin.track(media).unwrap();
    let _second = Pin::load(DeckId::Two, &decks, completed, requests, media).unwrap();
    verify_neutral(track, view, expected_pixels);
}
fn verify_neutral(
    track: pajoniiir_track_analysis::TrackAnalysis<'_>,
    view: pajoniiir_waveform::WaveformView<'_>,
    expected_pixels: usize,
) {
    let mut decks = DeckProductState::new();
    assert_eq!(track.bpm_x100(), 12000);
    assert_eq!(track.beat_grid().unwrap().beats().len(), 80);
    use pajoniiir_controller_core::{ControlEvent, ControlValue, SemanticControl};
    let beats = track.beat_grid().unwrap().beats();
    decks.set_position_ms(DeckId::One, beats[10].time_ms);
    // Reuse the foundation's bounded owner lane. Refusal must not commit a
    // staged Deck mutation or lose the pinned analysis needed to retry it.
    use pajoniiir_deck::{DeckEffect, DeckEffectBroker, DeckEffects};
    let mut effects = DeckEffectBroker::<1, 0, 0>::new();
    let pending = DeckEffect::Pause { deck: DeckId::Two };
    assert!(effects.try_push_all(DeckEffects::one(pending)));
    let event = ControlEvent {
        deck: Some(DeckId::One),
        control: SemanticControl::BeatJumpForward,
        value: ControlValue::Pressed(true),
    };
    let mut staged = decks;
    let refused = staged.handle_control_with_analysis(event, [Some(track), None]);
    assert!(!effects.try_push_all(refused));
    assert_eq!(decks.deck(DeckId::One).position_ms, beats[10].time_ms);
    assert_eq!(effects.transport_front(), Some(pending));
    assert!(effects.complete_transport(false));
    assert!(!effects.try_push_all(refused));
    assert_eq!(effects.transport_front(), Some(pending));
    assert!(effects.complete_transport(true));
    let mut staged = decks;
    let accepted = staged.handle_control_with_analysis(event, [Some(track), None]);
    assert!(effects.try_push_all(accepted));
    decks = staged;
    assert_eq!(
        effects.transport_front(),
        Some(DeckEffect::Seek {
            deck: DeckId::One,
            position_ms: beats[11].time_ms,
        })
    );
    let seek = effects.transport_front();
    assert!(effects.complete_transport(false));
    assert_eq!(effects.transport_front(), seek);
    assert_eq!(effects.transport_len(), 1);
    assert!(effects.complete_transport(true));
    assert!(!effects.complete_transport(true));
    assert!(effects.is_empty());
    assert_eq!(decks.deck(DeckId::One).position_ms, beats[11].time_ms);
    let position = decks.deck(DeckId::One).position_ms;
    decks.set_position_ms(DeckId::Two, position + 17);
    decks.handle_control_with_analysis(
        ControlEvent {
            deck: Some(DeckId::One),
            control: SemanticControl::Sync,
            value: ControlValue::Pressed(true),
        },
        [Some(track), Some(track)],
    );
    assert!(decks.deck(DeckId::One).sync_enabled);
    assert_eq!(decks.deck(DeckId::One).position_ms, position + 17);
    let mut pixels = vec![0; 800 * 128];
    pajoniiir_waveform::Rgb565Surface::new(&mut pixels, 800, 128)
        .unwrap()
        .draw_centered_waveform(0, 0, 800, 128, view);
    assert_eq!(
        pixels.iter().filter(|&&p| p == 0x07e0).count(),
        expected_pixels
    );
}
#[test]
fn short_reads_exact_hash_bounded_decode_and_retained_consumers() {
    let m = mounted();
    let b = wav(320000);
    analyze(Fs::new(&m, b.clone()), &m, &b);
}

#[test]
fn read_failures_and_cancellation_cannot_finish_and_close_once() {
    for mode in [1, 2, 3, 4, 6, 7] {
        let m = mounted();
        let mut r = Requests::default();
        let c = catalog(&m);
        let mut fs = Fs::new(&m, wav(100));
        let mut bytes = vec![0; fs.bytes.len()];
        let q = SourceRequest::new(&mut r, &m, &c, 0).unwrap();
        let mut read = run(q.open(&mut fs, &r, &m, &mut bytes)).unwrap();
        fs.mode = mode;
        if mode == 4 {
            use core::{
                future::Future,
                pin::pin,
                task::{Context, Poll},
            };
            let mut f = pin!(read.step(&mut fs, &r, &m));
            assert!(matches!(
                f.as_mut()
                    .poll(&mut Context::from_waker(core::task::Waker::noop())),
                Poll::Pending
            ));
        } else {
            assert!(run(read.step(&mut fs, &r, &m)).is_err());
        }
        fs.mode = 0;
        assert!(matches!(
            run(read.step(&mut fs, &r, &m)),
            Err(SourceError::Failed)
        ));
        assert!(run(read.finish(&mut fs, &mut r, &m)).is_err());
        assert_eq!(fs.closes, 1);
        assert_eq!(fs.reads, 1);
    }
}
#[test]
fn capacity_close_failure_invalid_wav_and_incomplete_fail_closed() {
    for case in 0..4 {
        let m = mounted();
        let c = catalog(&m);
        let mut r = Requests::default();
        let mut fs = Fs::new(&m, wav(100));
        if case == 2 {
            fs.bytes[0] = 0;
        }
        let mut bytes = vec![0; if case == 0 { 1 } else { fs.bytes.len() }];
        let q = SourceRequest::new(&mut r, &m, &c, 0).unwrap();
        let read = run(q.open(&mut fs, &r, &m, &mut bytes));
        if case == 0 {
            assert!(matches!(read, Err(SourceError::Capacity)));
            assert_eq!(fs.closes, 1);
            continue;
        }
        let mut read = read.unwrap();
        if case != 3 {
            while !run(read.step(&mut fs, &r, &m)).unwrap() {}
        }
        if case == 1 {
            fs.mode = 5;
        }
        assert!(run(read.finish(&mut fs, &mut r, &m)).is_err());
        assert_eq!(fs.closes, 1);
    }
}
#[test]
fn replacement_cancel_and_original_catalog_lease_are_checked() {
    for cancel in [false, true] {
        let m = mounted();
        let c = catalog(&m);
        let mut r = Requests::default();
        let mut fs = Fs::new(&m, wav(100));
        let mut bytes = vec![0; fs.bytes.len()];
        let q = SourceRequest::new(&mut r, &m, &c, 0).unwrap();
        let mut read = run(q.open(&mut fs, &r, &m, &mut bytes)).unwrap();
        if cancel {
            r.cancel();
        } else {
            let _ = SourceRequest::new(&mut r, &m, &c, 0).unwrap();
        }
        assert!(matches!(
            run(read.step(&mut fs, &r, &m)),
            Err(SourceError::Worker(WorkerError::Stale))
        ));
        run(read.abort(&mut fs)).unwrap();
        assert_eq!(fs.closes, 1);
    }
    let mut m = mounted();
    let c = catalog(&m);
    m.on_disconnect(Some(pajoniiir_media_session::MediaHandle::new(1).unwrap()));
    let mut r = Requests::default();
    assert!(SourceRequest::new(&mut r, &m, &c, 0).is_err());
}
#[test]
fn pending_completion_replacement_and_geometry_mismatch_are_rejected() {
    for replace in [false, true] {
        let m = mounted();
        let c = catalog(&m);
        let mut r = Requests::default();
        let mut fs = Fs::new(&m, wav(100));
        let mut bytes = vec![0; fs.bytes.len()];
        let q = SourceRequest::new(&mut r, &m, &c, 0).unwrap();
        let mut read = run(q.open(&mut fs, &r, &m, &mut bytes)).unwrap();
        while !run(read.step(&mut fs, &r, &m)).unwrap() {}
        if replace {
            let _ = SourceRequest::new(&mut r, &m, &c, 0).unwrap();
            assert!(run(read.finish(&mut fs, &mut r, &m)).is_err());
        } else {
            let source = run(read.finish(&mut fs, &mut r, &m)).unwrap();
            let mut q = [NormalizedSample::default(); 16];
            let mut c = [WaveformColumn::default(); 1];
            let session = Session::new(
                SessionConfig {
                    sample_rate: 16000,
                    channel_count: 1,
                    total_frames: 100,
                    frames_per_column: 256,
                },
                &mut q,
                &mut c,
            )
            .unwrap();
            assert!(source.worker(session).is_err());
        }
    }
}

struct Device {
    bytes: Vec<u8>,
}
impl pajoniiir_media_io::OwnedBlockDevice for Device {
    type Error = ();
    type Buffer = [u8; 512];
    fn geometry(&self) -> pajoniiir_media_block::BlockGeometry {
        pajoniiir_media_block::BlockGeometry {
            block_size: 512,
            block_count: (self.bytes.len() / 512) as u64,
        }
    }
    async fn read_blocks_owned(
        &mut self,
        first: u64,
        count: u32,
        mut b: [u8; 512],
    ) -> Result<[u8; 512], ((), [u8; 512])> {
        assert_eq!(count, 1);
        let start = first as usize * 512;
        b.copy_from_slice(&self.bytes[start..start + 512]);
        Ok(b)
    }
}
#[test]
fn native_async_fat32_fragmented_file_reaches_actual_consumers() {
    for b in [
        wav(320000),
        wav_profile(320000, 16, 1, false, false),
        wav_profile(320000, 24, 2, false, true),
        wav_profile(320000, 32, 2, false, false),
        wav_profile(320000, 32, 2, true, true),
    ] {
        fat32_profile(b);
    }
}
fn fat32_profile(b: Vec<u8>) {
    use pajoniiir_media_fat32::{Fat32AsyncFileSystem, Fat32BootInfo};
    let m = mounted();
    let clusters = b.len().div_ceil(512);
    let fat_sectors = (clusters + 3).div_ceil(128);
    let data_start = 1 + fat_sectors;
    let sectors = data_start + clusters + 1;
    let mut image = vec![0; sectors * 512];
    let root = data_start * 512;
    image[root..root + 11].copy_from_slice(b"TRACK   WAV");
    image[root + 11] = 0x20;
    image[root + 26..root + 28].copy_from_slice(&3u16.to_le_bytes());
    image[root + 28..root + 32].copy_from_slice(&(b.len() as u32).to_le_bytes());
    image[512 + 8..512 + 12].copy_from_slice(&0x0fffffffu32.to_le_bytes());
    // Reverse all clusters after the first: reads must follow FAT, not contiguous assumptions.
    let chain: Vec<usize> = core::iter::once(3).chain((4..clusters + 3).rev()).collect();
    for (i, &cluster) in chain.iter().enumerate() {
        let next = chain.get(i + 1).copied().unwrap_or(0x0fffffff);
        let p = 512 + cluster * 4;
        image[p..p + 4].copy_from_slice(&(next as u32).to_le_bytes());
        let p = (data_start + cluster - 2) * 512;
        let chunk = &b[i * 512..b.len().min((i + 1) * 512)];
        image[p..p + chunk.len()].copy_from_slice(chunk);
    }
    let boot = Fat32BootInfo {
        bytes_per_sector: 512,
        sectors_per_cluster: 1,
        reserved_sectors: 1,
        fat_count: 1,
        sectors_per_fat: fat_sectors as u32,
        total_sectors: sectors as u32,
        root_cluster: 2,
        fs_info_sector: 1,
        backup_boot_sector: 0,
        fat_start_sector: 1,
        data_start_sector: data_start as u64,
        cluster_count: (clusters + 1) as u32,
        max_cluster: (clusters + 2) as u32,
        volume_id: 7,
    };
    let fs = match Fat32AsyncFileSystem::new(
        Device { bytes: image },
        [0; 512],
        boot,
        m.lease().unwrap(),
    ) {
        Ok(fs) => fs,
        Err(_) => panic!("fixture mount"),
    };
    analyze(fs, &m, &b);
}

#[test]
fn native_async_exfat_contiguous_and_fragmented_files_reach_consumers() {
    for contiguous in [false, true] {
        for b in [
            wav(320000),
            wav_profile(320000, 16, 1, false, false),
            wav_profile(320000, 24, 2, false, true),
            wav_profile(320000, 32, 2, false, false),
            wav_profile(320000, 32, 2, true, true),
        ] {
            exfat_profile(b, contiguous);
        }
    }
}
fn exfat_profile(b: Vec<u8>, contiguous: bool) {
    use pajoniiir_media_exfat::{ExFatAsyncBootInfo, ExFatAsyncVolume};
    let m = mounted();
    let clusters = b.len().div_ceil(512);
    let fat_len = (clusters + 3).div_ceil(128);
    let heap = 24 + fat_len;
    let sectors = heap + clusters + 1;
    let mut image = vec![0; sectors * 512];
    let mut entries = [[0u8; 32]; 3];
    entries[0][0] = 0x85;
    entries[0][1] = 2;
    entries[1][0] = 0xc0;
    entries[1][1] = if contiguous { 3 } else { 1 };
    entries[1][3] = 9;
    entries[1][8..16].copy_from_slice(&(b.len() as u64).to_le_bytes());
    entries[1][20..24].copy_from_slice(&3u32.to_le_bytes());
    entries[1][24..32].copy_from_slice(&(b.len() as u64).to_le_bytes());
    entries[2][0] = 0xc1;
    for (i, u) in "TRACK.WAV".encode_utf16().enumerate() {
        entries[2][2 + i * 2..4 + i * 2].copy_from_slice(&u.to_le_bytes());
    }
    let mut checksum = 0u16;
    for (i, e) in entries.iter().enumerate() {
        for (j, &v) in e.iter().enumerate() {
            if i == 0 && (j == 2 || j == 3) {
                continue;
            }
            checksum = checksum.rotate_right(1).wrapping_add(v as u16);
        }
    }
    entries[0][2..4].copy_from_slice(&checksum.to_le_bytes());
    for (i, e) in entries.iter().enumerate() {
        image[heap * 512 + i * 32..heap * 512 + (i + 1) * 32].copy_from_slice(e);
    }
    image[24 * 512 + 8..24 * 512 + 12].copy_from_slice(&u32::MAX.to_le_bytes());
    let chain: Vec<_> = if contiguous {
        (3..clusters + 3).collect()
    } else {
        core::iter::once(3).chain((4..clusters + 3).rev()).collect()
    };
    for (i, &c) in chain.iter().enumerate() {
        let next = chain.get(i + 1).map_or(u32::MAX, |&n| n as u32);
        let p = 24 * 512 + c * 4;
        image[p..p + 4].copy_from_slice(&next.to_le_bytes());
        let p = (heap + c - 2) * 512;
        let chunk = &b[i * 512..b.len().min((i + 1) * 512)];
        image[p..p + chunk.len()].copy_from_slice(chunk);
    }
    let boot = ExFatAsyncBootInfo {
        partition_offset: 0,
        volume_length: sectors as u64,
        fat_offset: 24,
        fat_length: fat_len as u32,
        cluster_heap_offset: heap as u32,
        cluster_count: (clusters + 1) as u32,
        root_directory_cluster: 2,
        volume_serial: 7,
        file_system_revision: 0x100,
        volume_flags: 0,
        bytes_per_sector: 512,
        sectors_per_cluster: 1,
        number_of_fats: 1,
        percent_in_use: 0,
    };
    let fs =
        match ExFatAsyncVolume::new(Device { bytes: image }, [0; 512], boot, m.lease().unwrap()) {
            Ok(fs) => fs,
            Err(_) => panic!("fixture mount"),
        };
    analyze(fs, &m, &b);
}

#[test]
fn invalid_float_decode_is_terminal_even_with_a_valid_file_hash() {
    let m = mounted();
    let c = catalog(&m);
    let mut r = Requests::default();
    let mut b = wav(300);
    b[44 + 260 * 4..48 + 260 * 4].copy_from_slice(&f32::NAN.to_le_bytes());
    let mut fs = Fs::new(&m, b);
    let mut bytes = vec![0; fs.bytes.len()];
    let q = SourceRequest::new(&mut r, &m, &c, 0).unwrap();
    let mut read = run(q.open(&mut fs, &r, &m, &mut bytes)).unwrap();
    while !run(read.step(&mut fs, &r, &m)).unwrap() {}
    let source = run(read.finish(&mut fs, &mut r, &m)).unwrap();
    let mut queue = [NormalizedSample::default(); 256];
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
    let mut worker = source.worker(session).unwrap();
    let mut scratch = [0.; 512];
    assert!(!worker.step(&r, &m, &mut scratch).unwrap());
    assert!(matches!(
        worker.step(&r, &m, &mut scratch),
        Err(WorkerError::Core(_))
    ));
    assert_eq!(worker.step(&r, &m, &mut scratch), Err(WorkerError::Failed));
    assert!(worker.complete(&r, &m, &mut []).is_err());
}

#[test]
fn pending_read_resumes_once_and_unmounted_completion_is_closed() {
    use core::{
        future::Future,
        pin::pin,
        task::{Context, Poll},
    };
    let mut m = mounted();
    let c = catalog(&m);
    let mut r = Requests::default();
    let mut fs = Fs::new(&m, wav(1));
    let mut bytes = vec![0; fs.bytes.len()];
    let request = SourceRequest::new(&mut r, &m, &c, 0).unwrap();
    let mut reader = run(request.open(&mut fs, &r, &m, &mut bytes)).unwrap();
    fs.mode = 8;
    {
        let mut f = pin!(reader.step(&mut fs, &r, &m));
        let mut cx = Context::from_waker(core::task::Waker::noop());
        assert!(matches!(f.as_mut().poll(&mut cx), Poll::Pending));
        assert!(matches!(f.as_mut().poll(&mut cx), Poll::Ready(Ok(true))));
    }
    assert_eq!(fs.reads, 1);
    m.mark_unmounted();
    assert!(matches!(
        run(reader.finish(&mut fs, &mut r, &m)),
        Err(SourceError::Worker(WorkerError::Stale))
    ));
    assert_eq!(fs.closes, 1);
}

#[test]
fn original_lease_is_rechecked_after_whole_object_io() {
    for mode in [11, 12, 13] {
        let m = mounted();
        let mut r = Requests::default();
        let c = catalog(&m);
        let mut fs = Fs::new(&m, wav(300));
        let mut bytes = vec![0; fs.bytes.len()];
        let request = SourceRequest::new(&mut r, &m, &c, 0).unwrap();
        if mode == 11 {
            fs.mode = mode;
        }
        let opened = run(request.open(&mut fs, &r, &m, &mut bytes));
        if mode == 11 {
            assert!(opened.is_err());
        } else {
            let mut read = opened.unwrap();
            if mode == 12 {
                fs.mode = mode;
                assert!(run(read.step(&mut fs, &r, &m)).is_err());
                run(read.abort(&mut fs)).unwrap();
            } else {
                while !run(read.step(&mut fs, &r, &m)).unwrap() {}
                fs.mode = mode;
                assert!(run(read.finish(&mut fs, &mut r, &m)).is_err());
            }
        }
        assert_eq!(fs.closes, 1);
    }
}
