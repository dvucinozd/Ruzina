use super::*;
use crate::deck_mix::Mix;
use crate::deck_play::attach_ring;
use crate::deck_transport::{LOAD_RESULTS, LoadResult};
use ruzina_audio_dsp::PcmFrame;
use ruzina_controller_core::{ControlValue, PadAction, PadMode, SemanticControl};
use ruzina_deck::DeckState;

fn event(control: SemanticControl, value: ControlValue) -> ControlEvent {
    ControlEvent {
        deck: Some(DeckId::One),
        control,
        value,
    }
}

fn press(mix: &mut Mix, control: SemanticControl, pressed: bool) {
    mix.control(&event(control, ControlValue::Pressed(pressed)));
}

/// What a producer does for a new epoch at `ms`: 20000 frames of a
/// constant 48 kHz source from that position (the ring still holds the old
/// epoch's frames until the reader follows).
fn serve(ms: u32) {
    RINGS[0].restart_at(48_000, ms * 48);
    let frames = std::vec![PcmFrame { left: 10_000, right: 10_000 }; 20_000];
    assert!(RINGS[0].push(&frames));
}

/// Renders `packets` 1 ms packets; returns the last MAIN left sample.
fn run(mix: &mut Mix, packets: usize) -> i16 {
    let (mut main, mut headphones) = ([PcmFrame::default(); 48], [PcmFrame::default(); 48]);
    for _ in 0..packets {
        mix.fill(&mut main, &mut headphones, 48_000, [false, false]);
    }
    main[47].left
}

fn deck(mix: &mut Mix) -> DeckState {
    *mix.engine().deck(DeckId::One)
}

fn near(value: u32, expected: u32, slack: u32) -> bool {
    value.abs_diff(expected) <= slack
}

// One test: the rings, commands and mailboxes are process-wide statics.
#[test]
fn the_deck_engine_owns_transport() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    attach_ring(0);
    attach_ring(1);
    let mut mix = Mix::new();
    SEEK_REQUESTS[0].take();
    SEEK_RESULTS[0].take();

    // Play on an empty deck is refused.
    press(&mut mix, SemanticControl::Play, true);
    run(&mut mix, 2);
    assert_eq!(mix.engine().playing(), [false, false]);

    // A load commits once its producer has published it.
    serve(0);
    post_load(
        &mut mix,
        LoadFixture {
            ready: true,
            index: 3,
            bpm_x100: 12_800,
            track_id: [7; 16],
        },
    );
    run(&mut mix, 3); // at least one 128-frame block, where loads commit
    assert_eq!(deck(&mut mix).base_bpm_x100, 12_800);
    assert!(mix.engine().load_allowed(DeckId::One));

    // Play: audible, the position follows the source, and Load is refused.
    press(&mut mix, SemanticControl::Play, true);
    assert_eq!(run(&mut mix, 100), 2_500);
    assert!(
        near(deck(&mut mix).position_ms, 100, 3),
        "{:?}",
        deck(&mut mix)
    );
    assert!(!mix.engine().load_allowed(DeckId::One));

    // CUE while playing returns to the cue (0) and pauses. Until the seek's
    // epoch arrives, the stale position is not reported.
    press(&mut mix, SemanticControl::Cue, true);
    press(&mut mix, SemanticControl::Cue, false);
    assert_eq!(take_seek(), Some(0));
    assert_eq!(mix.engine().playing(), [false, false]);
    run(&mut mix, 5);
    assert_eq!(deck(&mut mix).position_ms, 0);
    serve(0);
    acknowledge_seek(&mut mix);
    assert_eq!(run(&mut mix, 5), 0);
    assert_eq!(deck(&mut mix).position_ms, 0);

    // Stopped at the cue, CUE previews while held and returns on release.
    press(&mut mix, SemanticControl::Cue, true);
    assert_eq!(mix.engine().playing(), [true, false]);
    assert_eq!(run(&mut mix, 50), 2_500);
    press(&mut mix, SemanticControl::Cue, false);
    assert_eq!(mix.engine().playing(), [false, false]);
    assert_eq!(take_seek(), Some(0));
    serve(0);
    acknowledge_seek(&mut mix);
    run(&mut mix, 2);

    // Play, then pause away from the cue: CUE stores a new cue point there
    // without moving.
    press(&mut mix, SemanticControl::Play, true);
    run(&mut mix, 200);
    press(&mut mix, SemanticControl::Play, true);
    let paused = deck(&mut mix).position_ms;
    assert!(near(paused, 200, 5), "{paused}");
    press(&mut mix, SemanticControl::Cue, true);
    press(&mut mix, SemanticControl::Cue, false);
    assert_eq!(deck(&mut mix).cue_point_ms, paused);
    assert_eq!(take_seek(), None);

    // The jog scrubs while paused (a seek) and bends while playing.
    mix.control(&event(SemanticControl::JogBend, ControlValue::Relative(10)));
    let target = take_seek().expect("scrub seek");
    assert!(target > paused);
    serve(target);
    acknowledge_seek(&mut mix);
    run(&mut mix, 2);
    assert!(near(deck(&mut mix).position_ms, target, 2));
    press(&mut mix, SemanticControl::Play, true);
    mix.control(&event(SemanticControl::JogBend, ControlValue::Relative(5)));
    assert_eq!(mix.engine().transport().1[0], 1_000);
    run(&mut mix, 250);
    assert_eq!(mix.engine().transport().1[0], 0);

    // The Tempo fader at its top end is +10 % through the engine.
    mix.control(&event(
        SemanticControl::Tempo,
        ControlValue::Absolute {
            value: 0,
            max: 16_383,
        },
    ));
    assert_eq!(mix.engine().transport().1[0], 1_000);
    let before = deck(&mut mix).position_ms;
    run(&mut mix, 100);
    assert!(near(deck(&mut mix).position_ms - before, 110, 3));

    // A Hot Cue pad stores the current position in the local bank, and the
    // next press jumps back there.
    let pad = |pressed| {
        event(
            SemanticControl::PadAction,
            ControlValue::PadAction(PadAction {
                pad: 0,
                mode: PadMode::HotCue,
                shifted: false,
                pressed,
            }),
        )
    };
    mix.control(&pad(true));
    mix.control(&pad(false));
    let stored = deck(&mut mix).position_ms;
    run(&mut mix, 50);
    mix.control(&pad(true));
    mix.control(&pad(false));
    assert_eq!(take_seek(), Some(stored));
    serve(stored);
    acknowledge_seek(&mut mix);
    run(&mut mix, 2);
    assert!(near(deck(&mut mix).position_ms, stored, 3));

    // The end of the track stops the deck at its last position.
    RINGS[0].end();
    run(&mut mix, 1_000);
    assert_eq!(mix.engine().playing(), [false, false]);
    assert!(!deck(&mut mix).playing);

    // The display status carries the position and flags.
    let mut status = [0u32; 4];
    // SAFETY: four writable slots.
    unsafe { ruzina_legacy_engine_status(status.as_mut_ptr()) };
    assert_eq!(status[0], deck(&mut mix).position_ms);
    assert_eq!(status[1], 1);

    // A failed load unloads the deck: Play is refused again.
    post_load(
        &mut mix,
        LoadFixture {
            ready: false,
            index: 4,
            bpm_x100: 0,
            track_id: [8; 16],
        },
    );
    run(&mut mix, 3); // at least one 128-frame block, where loads commit
    press(&mut mix, SemanticControl::Play, true);
    assert_eq!(mix.engine().playing(), [false, false]);
    assert_eq!(mix.engine().counters()[1], 1);
}

struct LoadFixture {
    ready: bool,
    index: u32,
    bpm_x100: u32,
    track_id: [u8; 16],
}

#[cfg(feature = "idf-uac-profile")]
#[test]
fn profiled_render_matches_unarmed_pcm_across_blocks_and_partial_blocks() {
    use crate::uac_profile;
    let _lock = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    attach_ring(0);
    attach_ring(1);
    let run_case = |profile: bool| {
        SEEK_REQUESTS[0].take();
        SEEK_RESULTS[0].take();
        let mut mix = Mix::new();
        serve(0);
        post_load(
            &mut mix,
            LoadFixture {
                ready: true,
                index: 3,
                bpm_x100: 12800,
                track_id: [7; 16],
            },
        );
        run(&mut mix, 3);
        press(&mut mix, SemanticControl::Play, true);
        let mut all = std::vec::Vec::new();
        let mut seen_blocks = 0;
        for _ in 0..12 {
            if profile {
                uac_profile::begin();
            }
            {
                let _fill = uac_profile::fill_scope();
                for _ in 0..6 {
                    let (mut main, mut hp) = ([PcmFrame::default(); 48], [PcmFrame::default(); 48]);
                    mix.fill(&mut main, &mut hp, 48000, [false, false]);
                    all.extend(main);
                    all.extend(hp);
                }
            }
            if profile {
                let mut stats = [0; 20];
                assert_eq!(unsafe { uac_profile::end(stats.as_mut_ptr(), 20) }, 20);
                assert_eq!(stats[1], 0);
                seen_blocks += stats[3];
            }
        }
        press(&mut mix, SemanticControl::Play, true);
        (all, seen_blocks)
    };
    let plain = run_case(false);
    let profiled = run_case(true);
    assert_eq!(plain.0, profiled.0);
    assert!(profiled.1 > 12);
}
fn post_load(mix: &mut Mix, fixture: LoadFixture) {
    use ruzina_media_session::{ConnectResult, MediaSession, MediaSourceId};
    if mix.engine().lease.is_none() {
        let mut session = MediaSession::new();
        let ConnectResult::Accepted(lease) = session.on_connect(MediaSourceId::new(7).unwrap())
        else {
            panic!()
        };
        crate::media_signal::activate(lease);
        mix.engine().lease = Some(lease);
    }
    assert!(mix.engine().request_load(DeckId::One, fixture.index));
    let request = LOAD_REQUESTS[0].take().unwrap();
    GATES[0].begin_replacement(request.permit).unwrap();
    LOAD_RESULTS[0]
        .put(LoadResult {
            request,
            ready: fixture.ready,
            bpm_x100: fixture.bpm_x100,
            track_id: fixture.track_id,
        })
        .unwrap();
}

#[test]
fn load_permit_protects_real_producer_and_rejects_late_results() {
    use crate::deck_play::{Event, Producer, tests::Memory};
    use crate::deck_transport::guarded_load;
    use ruzina_media_session::{ConnectResult, MediaSession, MediaSourceId};
    let _lock = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let mut session = MediaSession::new();
    let ConnectResult::Accepted(lease) = session.on_connect(MediaSourceId::new(9).unwrap()) else {
        panic!()
    };
    const WAV: &[u8] = include_bytes!(
        "../../../../../firmware-rust/crates/ruzina-audio-runtime/tests/fixtures/codecs/stereo.wav"
    );
    for (i, deck) in DECKS.into_iter().enumerate() {
        // Retire any completed previous test state; never interleave globals.
        LOAD_REQUESTS[i].take();
        LOAD_RESULTS[i].take();
        let mut engine = Engine::new();
        crate::media_signal::activate(lease);
        engine.lease = Some(lease);
        attach_ring(i);
        let mut producer = Producer::new();
        let mut files = Memory::new(&[WAV]);
        assert!(engine.request_load(deck, 0));
        assert!(!engine.request_load(deck, 1));
        let first = LOAD_REQUESTS[i].take().unwrap();
        assert!(matches!(
            guarded_load(i, first, lease, &mut producer, &mut files, Some("0")),
            Some(Event::Ready { .. })
        ));
        LOAD_RESULTS[i]
            .put(LoadResult {
                request: first,
                ready: true,
                track_id: [1; 16],
                bpm_x100: 12000,
            })
            .unwrap();
        engine.transport();
        assert!(engine.loaded[i]);
        assert!(GATES[i].is_idle());
        assert!(engine.request_load(deck, 1));
        let canceled = LOAD_REQUESTS[i].take().unwrap();
        let epoch = RINGS[i].epoch();
        engine.control(&ControlEvent {
            deck: Some(deck),
            control: SemanticControl::Play,
            value: ControlValue::Pressed(true),
        });
        assert!(engine.playing[i]);
        assert_eq!(
            guarded_load(i, canceled, lease, &mut producer, &mut files, None),
            None
        );
        assert_eq!(RINGS[i].epoch(), epoch); // producer did not destroy the old source
        LOAD_RESULTS[i]
            .put(LoadResult {
                request: canceled,
                ready: false,
                track_id: [2; 16],
                bpm_x100: 0,
            })
            .unwrap();
        engine.transport();
        assert!(engine.loaded[i]);
        assert!(engine.playing[i]);
        engine.control(&ControlEvent {
            deck: Some(deck),
            control: SemanticControl::Play,
            value: ControlValue::Pressed(true),
        });
        assert!(engine.request_load(deck, 1));
        let replacing = LOAD_REQUESTS[i].take().unwrap();
        GATES[i].begin_replacement(replacing.permit).unwrap();
        engine.control(&ControlEvent {
            deck: Some(deck),
            control: SemanticControl::Play,
            value: ControlValue::Pressed(true),
        });
        assert!(!engine.playing[i]);
        LOAD_RESULTS[i]
            .put(LoadResult {
                request: replacing,
                ready: false,
                track_id: [2; 16],
                bpm_x100: 0,
            })
            .unwrap();
        engine.transport();
        assert!(!engine.loaded[i]);
        assert!(GATES[i].is_idle());
        assert!(engine.request_load(deck, 0));
        let last = LOAD_REQUESTS[i].take().unwrap();
        GATES[i].finish(last.permit);
    }
}

fn take_seek() -> Option<u32> {
    // Inspect the engine's outbound request, keeping it for the simulated producer.
    let request = SEEK_REQUESTS[0].take()?;
    SEEK_REQUESTS[0].put(request).unwrap();
    Some(request.target_ms)
}
fn acknowledge_seek(mix: &mut Mix) {
    use crate::deck_transport::{SEEK_RESULTS, SeekResult};
    let request = SEEK_REQUESTS[0].take().unwrap();
    SEEK_RESULTS[0]
        .put(SeekResult {
            request,
            outcome: SeekOutcome::Ready,
            epoch: RINGS[0].epoch(),
            position_ms: request.target_ms,
        })
        .unwrap();
    mix.tick();
}

#[test]
fn seek_receipts_finish_eof_failure_and_ignore_superseded_work() {
    use crate::deck_play::{Producer, tests::Memory};
    use crate::deck_transport::{SEEK_RESULTS, execute_seek, guarded_load};
    use ruzina_media_session::{ConnectResult, MediaSession, MediaSourceId};
    let _lock = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let mut session = MediaSession::new();
    let ConnectResult::Accepted(lease) = session.on_connect(MediaSourceId::new(10).unwrap()) else {
        panic!()
    };
    const WAV: &[u8] = include_bytes!(
        "../../../../../firmware-rust/crates/ruzina-audio-runtime/tests/fixtures/codecs/stereo.wav"
    );
    for (i, deck) in DECKS.into_iter().enumerate() {
        LOAD_REQUESTS[i].take();
        LOAD_RESULTS[i].take();
        SEEK_REQUESTS[i].take();
        SEEK_RESULTS[i].take();
        let mut engine = Engine::new();
        crate::media_signal::activate(lease);
        engine.lease = Some(lease);
        attach_ring(i);
        let mut producer = Producer::new();
        let mut files = Memory::new(&[WAV]);
        assert!(engine.request_load(deck, 0));
        let source = LOAD_REQUESTS[i].take().unwrap();
        guarded_load(i, source, lease, &mut producer, &mut files, Some("0")).unwrap();
        LOAD_RESULTS[i]
            .put(LoadResult {
                request: source,
                ready: true,
                track_id: [3; 16],
                bpm_x100: 12000,
            })
            .unwrap();
        engine.transport();
        for (target, outcome, actual) in [
            (100, SeekOutcome::Ready, 100),
            (250, SeekOutcome::Eof, 250),
            (999, SeekOutcome::Eof, 250),
            (0, SeekOutcome::Ready, 0),
        ] {
            engine.apply(DeckEffects::one(DeckEffect::Seek {
                deck,
                position_ms: target,
            }));
            let request = SEEK_REQUESTS[i].take().unwrap();
            let result = execute_seek(i, request, Some(source), &mut producer, &mut files);
            assert_eq!(result.outcome, outcome);
            assert_eq!(result.position_ms, actual);
            // Producer can finish before the next control tick. No render needed.
            SEEK_RESULTS[i].put(result).unwrap();
            engine.transport();
            assert!(engine.seeking[i].is_none());
            assert_eq!(engine.deck.deck(deck).position_ms, actual);
            if outcome == SeekOutcome::Eof {
                assert!(!engine.playing[i]);
            }
        }
        engine.apply(DeckEffects::one(DeckEffect::Seek {
            deck,
            position_ms: 50,
        }));
        let old = SEEK_REQUESTS[i].take().unwrap();
        engine.apply(DeckEffects::one(DeckEffect::Seek {
            deck,
            position_ms: 125,
        }));
        let new = SEEK_REQUESTS[i].take().unwrap();
        SEEK_RESULTS[i]
            .put(execute_seek(
                i,
                old,
                Some(source),
                &mut producer,
                &mut files,
            ))
            .unwrap();
        engine.transport();
        assert_eq!(engine.seeking[i], Some(new));
        SEEK_RESULTS[i]
            .put(execute_seek(i, new, None, &mut producer, &mut files))
            .unwrap();
        engine.transport();
        assert!(engine.seeking[i].is_none());
        assert!(!engine.playing[i]);
        engine.apply(DeckEffects::one(DeckEffect::Seek {
            deck,
            position_ms: 0,
        }));
        let request = SEEK_REQUESTS[i].take().unwrap();
        files.fail_reads_after = Some(0);
        let failed = execute_seek(i, request, Some(source), &mut producer, &mut files);
        assert_eq!(failed.outcome, SeekOutcome::Failed);
        SEEK_RESULTS[i].put(failed).unwrap();
        engine.transport();
        assert!(engine.seeking[i].is_none());
        assert!(!engine.playing[i]);
        engine.control(&ControlEvent {
            deck: Some(deck),
            control: SemanticControl::Play,
            value: ControlValue::Pressed(true),
        });
        assert!(
            !engine.playing[i],
            "faulted source must not accept silent PLAY"
        );
        assert!(engine.load_allowed(deck));
    }
}

#[test]
fn media_revocation_discards_cached_audio_and_late_receipts() {
    let _lock = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    attach_ring(0);
    attach_ring(1);
    let mut mix = Mix::new();
    post_load(
        &mut mix,
        LoadFixture {
            ready: true,
            index: 0,
            bpm_x100: 12000,
            track_id: [5; 16],
        },
    );
    serve(0);
    run(&mut mix, 3);
    press(&mut mix, SemanticControl::Play, true);
    assert_ne!(run(&mut mix, 5), 0); // block has unconsumed cached output
    let old = mix.engine().source[0].unwrap();
    mix.engine().apply(DeckEffects::one(DeckEffect::Seek {
        deck: DeckId::One,
        position_ms: 100,
    }));
    let seek = SEEK_REQUESTS[0].take().unwrap();
    crate::media_signal::revoke();
    assert_eq!(run(&mut mix, 1), 0);
    assert_eq!(mix.engine().playing(), [false, false]);
    assert!(mix.engine().source[0].is_none());
    LOAD_RESULTS[0]
        .put(LoadResult {
            request: old,
            ready: true,
            track_id: [5; 16],
            bpm_x100: 12000,
        })
        .unwrap();
    crate::deck_transport::SEEK_RESULTS[0]
        .put(crate::deck_transport::SeekResult {
            request: seek,
            outcome: SeekOutcome::Ready,
            epoch: RINGS[0].epoch(),
            position_ms: 100,
        })
        .unwrap();
    mix.tick();
    assert!(!mix.engine().loaded[0]);
    assert!(!mix.engine().request_load(DeckId::One, 0));
    assert_eq!(ruzina_legacy_engine_track(1), -1);
    crate::media_signal::reset();
}

#[test]
fn controller_disconnect_releases_a_cue_preview_without_resuming_it() {
    let _lock = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let mut mix = Mix::new();
    post_load(
        &mut mix,
        LoadFixture {
            ready: true,
            index: 0,
            bpm_x100: 12000,
            track_id: [6; 16],
        },
    );
    mix.tick();
    press(&mut mix, SemanticControl::Cue, true);
    assert!(mix.engine().playing()[0]);
    mix.engine().connection(false);
    assert!(!mix.engine().playing()[0]);
    assert_eq!(SEEK_REQUESTS[0].take().unwrap().target_ms, 0);
    mix.engine().connection(true);
    assert!(!mix.engine().playing()[0]);
}
