// Test-only allocator instrumentation; production crates forbid unsafe code.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};
thread_local! { static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) }; }
struct Counter;
// SAFETY: forwards the exact allocation contract to System; the thread-local
// scalar counter allocates nothing and never inspects allocation memory.
unsafe impl GlobalAlloc for Counter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|c| {
            if let Some(n) = c.get() {
                c.set(Some(n + 1));
            }
        });
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static COUNTER: Counter = Counter;
use libapta::{
    WaveformColumn,
    analysis::{BIN_CAPACITY, OnsetBin},
    session::{Session, SessionConfig, SourceIdentity},
    waveform::NormalizedSample,
};
use pajoniiir_apta_adapter::{
    Error,
    worker::{Completed, Job, Pin, Requests, Worker, WorkerError},
};
use pajoniiir_controller_core::{ControlEvent, ControlValue, SemanticControl};
use pajoniiir_core::{DeckId, MediaTrackId};
use pajoniiir_deck::{DeckEffect, DeckProductState};
use pajoniiir_media_session::{MediaHandle, MediaSession, MediaSourceId};
use pajoniiir_track_analysis::Beat;
const EMPTY: Beat = Beat {
    time_ms: 0,
    phase: 0,
    bpm_x100: 0,
};
fn mounted() -> MediaSession {
    let mut media = MediaSession::new();
    media.on_connect(MediaSourceId::new(1).unwrap());
    let lease = media.lease().unwrap();
    assert!(media.bind_handle(lease, MediaHandle::new(1).unwrap()));
    assert!(media.commit_mounted(lease));
    media
}
fn analyze<'a>(
    requests: &Requests,
    media: &MediaSession,
    job: Job,
    bank: &'a mut [Beat],
) -> Completed<'a> {
    let mut queue = [NormalizedSample::default(); 256];
    let mut columns = vec![WaveformColumn::default(); 1250];
    let mut bins = vec![OnsetBin::default(); BIN_CAPACITY];
    let mut flux = vec![0.0; BIN_CAPACITY];
    let mut session = Session::new(
        SessionConfig {
            sample_rate: 8000,
            channel_count: 1,
            total_frames: 320000,
            frames_per_column: 256,
        },
        &mut queue,
        &mut columns,
    )
    .unwrap();
    session.enable_tempo(&mut bins, &mut flux).unwrap();
    session.enable_meter().unwrap();
    ALLOCATIONS.with(|c| c.set(Some(0)));
    let mut worker = Worker::new(session, job).unwrap();
    let mut offset = 0;
    while offset < 320000 {
        let mut pcm = [0.0; 256];
        for (i, value) in pcm.iter_mut().enumerate() {
            let frame = offset + i;
            let phase = frame % 4000;
            if phase < 64 {
                *value =
                    (64 - phase) as f32 / 64.0 * if frame / 4000 % 4 == 0 { 0.75 } else { 0.2 };
            }
        }
        let n = worker
            .push(requests, media, &pcm[..256.min(320000 - offset)])
            .unwrap();
        offset += n;
        worker.step(requests, media).unwrap();
    }
    worker.finish_input(requests, media).unwrap();
    let mut done = false;
    for _ in 0..10000 {
        if worker.step(requests, media).unwrap() {
            done = true;
            break;
        }
    }
    assert!(done);
    let mut short = [EMPTY; 1];
    assert!(matches!(
        worker.complete(requests, media, &mut short),
        Err(WorkerError::Analysis(Error::Capacity))
    ));
    assert_eq!(short, [EMPTY]);
    let completed = worker.complete(requests, media, bank).unwrap();
    let allocations = ALLOCATIONS.with(|c| c.replace(None));
    assert_eq!(
        allocations,
        Some(0),
        "worker PCM/processing/conversion allocated"
    );
    completed
}
#[test]
fn retained_pcm_banks_follow_media_requests_and_actual_deck_state() {
    let mut media = mounted();
    let mut requests = Requests::default();
    let identity = SourceIdentity::new(1, [42; 32]).unwrap();
    let track_id = MediaTrackId([7; 16]);
    let job = requests
        .request(&media, media.lease().unwrap(), track_id, identity)
        .unwrap();
    let mut first = [EMPTY; 128];
    let completed = analyze(&requests, &media, job, &mut first);
    let mut decks = DeckProductState::new();
    let mut pin = Pin::load(DeckId::One, &decks, completed, &requests, &media).unwrap();
    assert_eq!(pin.track_id(), track_id);
    let original = pin.track(&media).unwrap();
    assert_eq!(original.beat_grid().unwrap().beats().len(), 80);
    let control = |control| ControlEvent {
        deck: Some(DeckId::One),
        control,
        value: ControlValue::Pressed(true),
    };
    decks.set_position_ms(
        DeckId::One,
        original.beat_grid().unwrap().beats()[10].time_ms,
    );
    decks.handle_control_with_analysis(
        control(SemanticControl::BeatJumpForward),
        [pin.track(&media), None],
    );
    assert_eq!(
        decks.deck(DeckId::One).position_ms,
        original.beat_grid().unwrap().beats()[11].time_ms
    );
    let effects = decks
        .handle_control_with_analysis(control(SemanticControl::Play), [pin.track(&media), None]);
    let Some(DeckEffect::PlaybackRequest { request_id, .. }) = effects.items[0] else {
        panic!("missing playback request")
    };
    assert!(!decks.deck(DeckId::One).can_replace_analysis());
    assert!(matches!(
        Pin::load(DeckId::One, &decks, completed, &requests, &media),
        Err(WorkerError::Analysis(Error::Playing))
    ));
    assert!(decks.resolve_playback_request(DeckId::One, request_id, true));
    assert!(decks.deck(DeckId::One).playing);
    let job = requests
        .request(&media, media.lease().unwrap(), track_id, identity)
        .unwrap();
    let mut second = [EMPTY; 128];
    let newer = analyze(&requests, &media, job, &mut second);
    assert_eq!(
        pin.upgrade(&decks, newer, &requests, &media),
        Err(WorkerError::Analysis(Error::Playing))
    );
    assert_eq!(pin.track(&media).unwrap().generation(), 1);
    let effects = decks
        .handle_control_with_analysis(control(SemanticControl::Play), [pin.track(&media), None]);
    let Some(DeckEffect::PlaybackRequest { request_id, .. }) = effects.items[0] else {
        panic!("missing playback request")
    };
    assert!(!decks.deck(DeckId::One).can_replace_analysis());
    assert!(decks.resolve_playback_request(DeckId::One, request_id, true));
    pin.upgrade(&decks, newer, &requests, &media).unwrap();
    assert_eq!(pin.track(&media).unwrap().generation(), 2);
    assert_eq!(original.generation(), 1);
    assert_eq!(
        original.beat_grid().unwrap().beats(),
        pin.track(&media).unwrap().beat_grid().unwrap().beats()
    );
    assert_eq!(
        pin.upgrade(&decks, newer, &requests, &media),
        Err(WorkerError::Analysis(Error::Generation))
    );
    let position = pin.track(&media).unwrap().beat_grid().unwrap().beats()[10].time_ms;
    decks.set_position_ms(DeckId::One, position);
    decks.set_position_ms(DeckId::Two, position + 17);
    decks.handle_control_with_analysis(
        control(SemanticControl::Sync),
        [pin.track(&media), pin.track(&media)],
    );
    assert!(decks.deck(DeckId::One).sync_enabled);
    assert_eq!(decks.deck(DeckId::One).position_ms, position + 17);
    // Completion was queued while a replacement request arrived.
    requests
        .request(
            &media,
            media.lease().unwrap(),
            MediaTrackId([8; 16]),
            identity,
        )
        .unwrap();
    assert!(matches!(
        Pin::load(DeckId::Two, &decks, newer, &requests, &media),
        Err(WorkerError::Stale)
    ));
    assert!(pin.track(&media).is_some());
    let old_lease = media.lease().unwrap();
    media.on_disconnect(Some(MediaHandle::new(1).unwrap()));
    assert!(pin.track(&media).is_none());
    media.on_connect(MediaSourceId::new(1).unwrap());
    let lease = media.lease().unwrap();
    assert!(media.bind_handle(lease, MediaHandle::new(2).unwrap()));
    assert!(media.commit_mounted(lease));
    assert_eq!(
        requests.request(&media, old_lease, track_id, identity),
        Err(WorkerError::Stale)
    );
    assert!(pin.track(&media).is_none());
    assert!(matches!(
        Pin::load(DeckId::Two, &decks, newer, &requests, &media),
        Err(WorkerError::Stale)
    ));
}

#[test]
fn cancellation_replacement_unmount_and_source_failure_never_publish() {
    for mode in 0..4 {
        let mut media = mounted();
        let mut requests = Requests::default();
        let identity = SourceIdentity::new(1, [42; 32]).unwrap();
        let job = requests
            .request(
                &media,
                media.lease().unwrap(),
                MediaTrackId([7; 16]),
                identity,
            )
            .unwrap();
        let mut queue = [NormalizedSample::default(); 2];
        let mut columns = [WaveformColumn::default(); 1];
        let session = Session::new(
            SessionConfig {
                sample_rate: 8000,
                channel_count: 1,
                total_frames: 64,
                frames_per_column: 64,
            },
            &mut queue,
            &mut columns,
        )
        .unwrap();
        let mut worker = Worker::new(session, job).unwrap();
        assert_eq!(worker.push(&requests, &media, &[0.25; 4]), Ok(2));
        assert_eq!(worker.push(&requests, &media, &[0.25]), Ok(0));
        let mut bank = [EMPTY; 1];
        assert!(matches!(
            worker.complete(&requests, &media, &mut bank),
            Err(WorkerError::Incomplete)
        ));
        match mode {
            0 => requests.cancel(),
            1 => {
                requests
                    .request(
                        &media,
                        media.lease().unwrap(),
                        MediaTrackId([8; 16]),
                        identity,
                    )
                    .unwrap();
            }
            2 => media.mark_unmounted(),
            _ => worker.source_failed(),
        }
        assert!(worker.step(&requests, &media).is_err());
        assert_eq!(
            worker.push(&requests, &media, &[0.25]),
            Err(WorkerError::Failed)
        );
        assert!(matches!(
            worker.complete(&requests, &media, &mut bank),
            Err(WorkerError::Failed)
        ));
        assert_eq!(bank, [EMPTY]);
    }
}
