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
    // A new source drops what the ring held, whatever an earlier test left.
    crate::deck_play::Reader::new().sync(&RINGS[0]);
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

#[test]
fn scratch_messages_with_no_track_loaded_are_not_errors() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let mut engine = Engine::new();
    let message = |control, value| ControlEvent {
        deck: Some(DeckId::Two),
        control,
        value,
    };
    engine.control(&message(
        SemanticControl::JogTouch,
        ControlValue::Pressed(true),
    ));
    let before = engine.counters();
    for _ in 0..8 {
        engine.control(&message(
            SemanticControl::JogScratch,
            ControlValue::Relative(1),
        ));
    }
    engine.control(&message(
        SemanticControl::JogTouch,
        ControlValue::Pressed(false),
    ));
    assert_eq!(engine.counters(), before);
    assert_eq!(engine.playing(), [false; 2]);
    assert_eq!(engine.take_scratch(1), None);
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

#[test]
fn peer_source_changes_preserve_playing_pcm_at_partial_boundaries() {
    use crate::deck_transport::{SeekOutcome, SeekResult};
    use ruzina_media_session::{ConnectResult, MediaSession, MediaSourceId};
    let _lock = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let case = |active: usize, rate, offset: usize, action: u8| {
        let mut mix = Mix::new();
        let mut session = MediaSession::new();
        let ConnectResult::Accepted(lease) = session.on_connect(MediaSourceId::new(7).unwrap())
        else {
            panic!()
        };
        crate::media_signal::activate(lease);
        mix.engine().lease = Some(lease);
        for (i, deck) in DECKS.into_iter().enumerate() {
            attach_ring(i);
            LOAD_REQUESTS[i].take();
            LOAD_RESULTS[i].take();
            SEEK_REQUESTS[i].take();
            SEEK_RESULTS[i].take();
            RINGS[i].restart_at(rate, 0);
            let frames: std::vec::Vec<_> = (0..12_000)
                .map(|n| PcmFrame {
                    left: (n % 5000) as i16,
                    right: (n % 3000) as i16,
                })
                .collect();
            assert!(RINGS[i].push(&frames));
            assert!(mix.engine().request_load(deck, i as u32));
            let request = LOAD_REQUESTS[i].take().unwrap();
            GATES[i].begin_replacement(request.permit).unwrap();
            LOAD_RESULTS[i]
                .put(LoadResult {
                    request,
                    ready: true,
                    track_id: [i as u8 + 1; 16],
                    bpm_x100: 12000,
                })
                .unwrap();
            mix.tick();
        }
        for (control, value) in [
            (SemanticControl::Tempo, 0),
            (SemanticControl::EqLow, 11000),
            (SemanticControl::Filter, 10000),
        ] {
            mix.control(&ControlEvent {
                deck: Some(DECKS[active]),
                control,
                value: ControlValue::Absolute { value, max: 16383 },
            });
        }
        mix.control(&ControlEvent {
            deck: Some(DECKS[active]),
            control: SemanticControl::Play,
            value: ControlValue::Pressed(true),
        });
        let (mut warm, mut hp) = (
            std::vec![PcmFrame::default();256+offset],
            std::vec![PcmFrame::default();256+offset],
        );
        mix.fill(&mut warm, &mut hp, 48000, [false; 2]);
        let peer = 1 - active;
        if (1..=4).contains(&action) {
            mix.engine().apply(DeckEffects::one(DeckEffect::Seek {
                deck: DECKS[peer],
                position_ms: 100,
            }));
            let request = SEEK_REQUESTS[peer].take().unwrap();
            if action > 1 {
                SEEK_RESULTS[peer]
                    .put(SeekResult {
                        request,
                        outcome: match action {
                            2 => SeekOutcome::Ready,
                            3 => SeekOutcome::Failed,
                            _ => SeekOutcome::Cancelled,
                        },
                        epoch: RINGS[peer].epoch(),
                        position_ms: 100,
                    })
                    .unwrap();
            }
        } else if action >= 5 {
            assert!(mix.engine().request_load(DECKS[peer], 9));
            let request = LOAD_REQUESTS[peer].take().unwrap();
            GATES[peer].begin_replacement(request.permit).unwrap();
            LOAD_RESULTS[peer]
                .put(LoadResult {
                    request,
                    ready: action == 5,
                    track_id: [9; 16],
                    bpm_x100: 12000,
                })
                .unwrap();
        }
        let (mut main, mut headphones) = ([PcmFrame::default(); 384], [PcmFrame::default(); 384]);
        mix.fill(&mut main, &mut headphones, 48000, [false; 2]);
        (main, headphones)
    };
    for active in 0..2 {
        for rate in [44100, 48000] {
            for offset in [1, 48, 127] {
                let reference = case(active, rate, offset, 0);
                for action in 1..=6 {
                    assert_eq!(
                        reference,
                        case(active, rate, offset, action),
                        "active={active} rate={rate} offset={offset} action={action}"
                    );
                }
            }
        }
    }
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

/// A seek while playing resumes only once the new epoch has buffered a
/// cushion: resuming on the seek's first block underran on the device. A
/// producer that never catches up holds the deck for at most a second.
#[test]
fn a_seek_resumes_playback_only_after_a_buffered_cushion() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    attach_ring(0);
    attach_ring(1);
    let mut mix = Mix::new();
    SEEK_REQUESTS[0].take();
    SEEK_RESULTS[0].take();
    serve(0);
    post_load(
        &mut mix,
        LoadFixture {
            ready: true,
            index: 5,
            bpm_x100: 12_000,
            track_id: [5; 16],
        },
    );
    run(&mut mix, 3);
    press(&mut mix, SemanticControl::Play, true);
    run(&mut mix, 100);
    let before = mix.stats()[1];
    // A jog scrub would pause; a Hot Cue jump keeps playing.
    let pad = |pressed| {
        event(
            SemanticControl::PadAction,
            ControlValue::PadAction(PadAction {
                pad: 1,
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
    // The seek completes with only its first block in the ring.
    let first = std::vec![PcmFrame { left: 10_000, right: 10_000 }; 256];
    RINGS[0].restart_at(48_000, stored * 48);
    assert!(RINGS[0].push(&first));
    acknowledge_seek(&mut mix);
    assert_eq!(mix.engine().playing(), [true, false]);
    run(&mut mix, 20);
    // Held: nothing rendered from the thin epoch, no underrun, no movement.
    assert_eq!(mix.stats()[1], before);
    assert_eq!(mix.stats()[3], 256);
    assert_eq!(deck(&mut mix).position_ms, stored);
    // The producer catches up: playback resumes from the target.
    let rest = std::vec![PcmFrame { left: 10_000, right: 10_000 }; 8_000];
    assert!(RINGS[0].push(&rest));
    run(&mut mix, 50);
    assert!(near(deck(&mut mix).position_ms, stored + 50, 3));
    assert_eq!(mix.stats()[1], before);

    // A producer that never catches up holds the deck for at most a second.
    mix.control(&pad(true));
    mix.control(&pad(false));
    take_seek().expect("hot cue seek");
    RINGS[0].restart_at(48_000, stored * 48);
    assert!(RINGS[0].push(&first));
    acknowledge_seek(&mut mix);
    run(&mut mix, 900);
    assert_eq!(deck(&mut mix).position_ms, stored);
    run(&mut mix, 200);
    assert!(deck(&mut mix).position_ms > stored);
}

#[test]
fn a_play_right_after_a_load_waits_for_a_buffered_cushion() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    attach_ring(0);
    attach_ring(1);
    let mut mix = Mix::new();
    SEEK_REQUESTS[0].take();
    SEEK_RESULTS[0].take();
    // The producer publishes Ready with only the new track's first block.
    RINGS[0].restart_at(48_000, 0);
    let first = std::vec![PcmFrame { left: 10_000, right: 10_000 }; 256];
    assert!(RINGS[0].push(&first));
    post_load(
        &mut mix,
        LoadFixture {
            ready: true,
            index: 6,
            bpm_x100: 12_000,
            track_id: [6; 16],
        },
    );
    run(&mut mix, 1); // the load commits in the first block
    // Play at once, as the soak DJ does: accepted, but held.
    press(&mut mix, SemanticControl::Play, true);
    assert_eq!(mix.engine().playing(), [true, false]);
    let before = mix.stats()[1];
    run(&mut mix, 20);
    assert_eq!(mix.stats()[1], before, "no underrun while held");
    assert_eq!(mix.stats()[3], 256, "nothing consumed while held");
    assert_eq!(deck(&mut mix).position_ms, 0);
    // The producer catches up: the track plays from its start.
    let rest = std::vec![PcmFrame { left: 10_000, right: 10_000 }; 8_000];
    assert!(RINGS[0].push(&rest));
    run(&mut mix, 50);
    assert!(near(deck(&mut mix).position_ms, 50, 3));
    assert_eq!(mix.stats()[1], before);
}

#[test]
fn loop_in_and_out_keep_the_deck_inside_the_loop_until_exit() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    attach_ring(0);
    attach_ring(1);
    let mut mix = Mix::new();
    SEEK_REQUESTS[0].take();
    SEEK_RESULTS[0].take();
    serve(0);
    // About 1 s more source, so the exit can play on past the loop end.
    let more = std::vec![PcmFrame { left: 10_000, right: 10_000 }; 30_000];
    assert!(RINGS[0].push(&more));
    post_load(
        &mut mix,
        LoadFixture {
            ready: true,
            index: 7,
            bpm_x100: 12_000,
            track_id: [7; 16],
        },
    );
    run(&mut mix, 3);
    press(&mut mix, SemanticControl::Play, true);
    run(&mut mix, 100);
    assert_eq!(mix.engine().loop_leds()[0], (false, false));
    press(&mut mix, SemanticControl::LoopIn, true);
    assert_eq!(
        mix.engine().loop_leds()[0],
        (true, false),
        "In marks the start"
    );
    let start = deck(&mut mix).position_ms;
    run(&mut mix, 150);
    press(&mut mix, SemanticControl::LoopOut, true);
    assert_eq!(mix.engine().loop_leds()[0], (true, true), "the loop runs");
    let end = deck(&mut mix).position_ms;
    assert!(end > start + 100, "{start}..{end}");
    // Several passes: the position stays in the loop, with no seek and no
    // underrun.
    let underruns = mix.stats()[1];
    for _ in 0..10 {
        run(&mut mix, 37);
        let at = deck(&mut mix).position_ms;
        assert!((start..end + 3).contains(&at), "{at} not in {start}..{end}");
    }
    assert_eq!(take_seek(), None);
    assert_eq!(mix.stats()[1], underruns);
    assert_eq!(mix.engine().counters()[0], 0, "loops are executed");
    // Reloop/Exit leaves the loop: playback runs past its end.
    press(&mut mix, SemanticControl::ReloopExit, true);
    assert_eq!(mix.engine().loop_leds()[0], (false, false));
    run(&mut mix, 300);
    assert!(deck(&mut mix).position_ms > end + 100);
    assert_eq!(mix.stats()[1], underruns);
}

#[test]
fn the_four_beat_button_starts_a_loop_and_exits_it() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    attach_ring(0);
    attach_ring(1);
    let mut mix = Mix::new();
    SEEK_REQUESTS[0].take();
    SEEK_RESULTS[0].take();
    serve(0);
    let more = std::vec![PcmFrame { left: 10_000, right: 10_000 }; 30_000];
    assert!(RINGS[0].push(&more));
    // 480 BPM: 4 beats are 500 ms, so the test source covers several passes.
    post_load(
        &mut mix,
        LoadFixture {
            ready: true,
            index: 8,
            bpm_x100: 48_000,
            track_id: [8; 16],
        },
    );
    run(&mut mix, 3);
    press(&mut mix, SemanticControl::Play, true);
    run(&mut mix, 100);
    let start = deck(&mut mix).position_ms;
    press(&mut mix, SemanticControl::ReloopExit, true);
    press(&mut mix, SemanticControl::ReloopExit, false);
    let region = deck(&mut mix).loop_state.active.expect("a 4-beat loop");
    assert_eq!(mix.engine().loop_leds()[0], (true, true));
    assert_eq!((region.start_ms, region.end_ms), (start, start + 500));
    let underruns = mix.stats()[1];
    for _ in 0..20 {
        run(&mut mix, 90);
        let at = deck(&mut mix).position_ms;
        assert!((start..start + 503).contains(&at), "{at} from {start}");
    }
    assert_eq!(take_seek(), None);
    // Pressed again it exits: the track plays on past the loop end.
    press(&mut mix, SemanticControl::ReloopExit, true);
    press(&mut mix, SemanticControl::ReloopExit, false);
    assert_eq!(deck(&mut mix).loop_state.active, None);
    assert_eq!(mix.engine().loop_leds()[0], (false, false));
    run(&mut mix, 600);
    let past = deck(&mut mix).position_ms;
    assert!(past > start + 500, "{past} from {start}");
    // And once more it starts a new 4-beat loop where the deck is now.
    press(&mut mix, SemanticControl::ReloopExit, true);
    let again = deck(&mut mix).loop_state.active.expect("a new 4-beat loop");
    assert_eq!((again.start_ms, again.end_ms), (past, past + 500));
    assert_eq!(mix.stats()[1], underruns);
}

#[test]
fn shift_four_beat_brings_back_the_last_loop_then_stops_and_forgets_it() {
    use ruzina_controller_core::{DeckExtAction, DeckExtActionValue};
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    attach_ring(0);
    attach_ring(1);
    let mut mix = Mix::new();
    SEEK_REQUESTS[0].take();
    SEEK_RESULTS[0].take();
    serve(0);
    let more = std::vec![PcmFrame { left: 10_000, right: 10_000 }; 30_000];
    assert!(RINGS[0].push(&more));
    post_load(
        &mut mix,
        LoadFixture {
            ready: true,
            index: 9,
            bpm_x100: 48_000,
            track_id: [9; 16],
        },
    );
    run(&mut mix, 3);
    let shift_four_beat = |pressed| {
        event(
            SemanticControl::DeckExtAction,
            ControlValue::DeckExtAction(DeckExtActionValue {
                action: DeckExtAction::ReloopStop,
                pressed,
            }),
        )
    };
    // With nothing to bring back it changes nothing.
    mix.control(&shift_four_beat(true));
    mix.control(&shift_four_beat(false));
    assert_eq!(deck(&mut mix).loop_state.active, None);
    press(&mut mix, SemanticControl::Play, true);
    run(&mut mix, 100);
    // A 4-beat loop, then exit: the deck remembers it.
    press(&mut mix, SemanticControl::ReloopExit, true);
    let region = deck(&mut mix).loop_state.active.expect("a 4-beat loop");
    run(&mut mix, 100);
    press(&mut mix, SemanticControl::ReloopExit, true);
    assert_eq!(deck(&mut mix).loop_state.active, None);
    run(&mut mix, 50);
    // Shift + 4 BEAT brings that same loop back, and the deck stays inside it.
    mix.control(&shift_four_beat(true));
    mix.control(&shift_four_beat(false));
    assert_eq!(deck(&mut mix).loop_state.active, Some(region));
    assert_eq!(mix.engine().loop_leds()[0], (true, true));
    for _ in 0..12 {
        run(&mut mix, 90);
        let at = deck(&mut mix).position_ms;
        assert!(
            (region.start_ms..region.end_ms + 3).contains(&at),
            "{at} outside {region:?}"
        );
    }
    assert_eq!(take_seek(), None);
    // With the loop running, it stops the loop and forgets it.
    mix.control(&shift_four_beat(true));
    assert_eq!(deck(&mut mix).loop_state.active, None);
    assert_eq!(deck(&mut mix).loop_state.last, None);
    assert_eq!(mix.engine().loop_leds()[0], (false, false));
    mix.control(&shift_four_beat(false));
    mix.control(&shift_four_beat(true));
    assert_eq!(deck(&mut mix).loop_state.active, None);
}

#[test]
fn the_jog_scratches_a_playing_and_a_paused_deck_and_release_plays_on() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    attach_ring(0);
    attach_ring(1);
    let mut mix = Mix::new();
    SEEK_REQUESTS[0].take();
    SEEK_RESULTS[0].take();
    serve(0);
    let more = std::vec![PcmFrame { left: 10_000, right: 10_000 }; 30_000];
    assert!(RINGS[0].push(&more));
    post_load(
        &mut mix,
        LoadFixture {
            ready: true,
            index: 10,
            bpm_x100: 12_000,
            track_id: [10; 16],
        },
    );
    run(&mut mix, 3);
    press(&mut mix, SemanticControl::Play, true);
    run(&mut mix, 300);
    let before = mix.stats()[1];
    let unsupported = mix.engine().counters()[0];
    // Touching the platter holds a playing deck, in silence.
    press(&mut mix, SemanticControl::JogTouch, true);
    assert_eq!(mix.engine().playing(), [true, false]);
    run(&mut mix, 10);
    assert_eq!(run(&mut mix, 3), 0);
    let held = deck(&mut mix).position_ms;
    assert_eq!(run(&mut mix, 50), 0);
    assert!(near(deck(&mut mix).position_ms, held, 2));
    // Backward: the position follows the platter, with audio and no underrun.
    let jog = |ticks| event(SemanticControl::JogScratch, ControlValue::Relative(ticks));
    let mut audible = 0;
    for _ in 0..40 {
        mix.control(&jog(-3));
        if run(&mut mix, 5) != 0 {
            audible += 1;
        }
    }
    assert!(audible > 20, "{audible}");
    let back = deck(&mut mix).position_ms;
    assert!(back + 100 < held, "{back} from {held}");
    assert_eq!(mix.stats()[1], before);
    assert_eq!(
        mix.engine().counters()[0],
        unsupported,
        "scratch is executed"
    );
    // Release: the deck plays on from the head.
    press(&mut mix, SemanticControl::JogTouch, false);
    run(&mut mix, 100);
    let after = deck(&mut mix).position_ms;
    assert!(
        after > back + 50 && after < back + 200,
        "{after} from {back}"
    );
    assert_eq!(mix.stats()[1], before);
    // Paused, the platter still moves the audio, and release stays paused.
    press(&mut mix, SemanticControl::Play, true);
    assert_eq!(mix.engine().playing(), [false, false]);
    run(&mut mix, 5);
    press(&mut mix, SemanticControl::JogTouch, true);
    run(&mut mix, 3);
    let paused = deck(&mut mix).position_ms;
    for _ in 0..30 {
        mix.control(&jog(-3));
        run(&mut mix, 5);
    }
    let moved = deck(&mut mix).position_ms;
    assert!(moved + 50 < paused, "{moved} from {paused}");
    press(&mut mix, SemanticControl::JogTouch, false);
    run(&mut mix, 50);
    assert!(near(deck(&mut mix).position_ms, moved, 80));
    assert_eq!(mix.engine().playing(), [false, false]);
    assert_eq!(run(&mut mix, 3), 0);
}

/// Gives the mix's render graph its effect memory.
fn attach_fx(mix: &mut Mix) {
    use ruzina_audio_runtime::fx::FX_TOTAL_STORAGE_SAMPLES;
    let storage = std::vec![0.0_f32; FX_TOTAL_STORAGE_SAMPLES].leak();
    crate::deck_resources::FX_STORAGE
        .store(storage.as_mut_ptr(), core::sync::atomic::Ordering::Release);
    mix.attach_fx();
}

fn system_press(mix: &mut Mix, control: SemanticControl) {
    for pressed in [true, false] {
        mix.control(&ControlEvent {
            deck: None,
            control,
            value: ControlValue::Pressed(pressed),
        });
    }
}

/// Deck 1 loaded and playing a source that `Tone::feed` keeps going.
fn playing_mix() -> Mix {
    attach_ring(0);
    attach_ring(1);
    let mut mix = Mix::new();
    attach_fx(&mut mix);
    SEEK_REQUESTS[0].take();
    SEEK_RESULTS[0].take();
    serve(0);
    post_load(
        &mut mix,
        LoadFixture {
            ready: true,
            index: 11,
            bpm_x100: 24_000,
            track_id: [11; 16],
        },
    );
    run(&mut mix, 3);
    press(&mut mix, SemanticControl::Play, true);
    mix
}

/// Packets until the deck has played what `serve` put in front of the tone.
const TONE_AFTER_PACKETS: usize = 450;

/// A 1 kHz tone, pushed as fast as a 48 kHz deck plays it.
struct Tone(u32);
impl Tone {
    fn feed(&mut self) {
        let frames: std::vec::Vec<PcmFrame> = (0..48)
            .map(|_| {
                let angle = self.0 as f32 * 2.0 * core::f32::consts::PI / 48.0;
                self.0 = (self.0 + 1) % 48;
                let value = (8_000.0 * angle.sin()) as i16;
                PcmFrame {
                    left: value,
                    right: value,
                }
            })
            .collect();
        assert!(RINGS[0].push(&frames));
    }

    /// Plays `packets` 1 ms packets; returns MAIN's left channel.
    fn play(&mut self, mix: &mut Mix, packets: usize) -> std::vec::Vec<i32> {
        let (mut main, mut headphones) = ([PcmFrame::default(); 48], [PcmFrame::default(); 48]);
        let mut out = std::vec::Vec::new();
        for _ in 0..packets {
            self.feed();
            mix.fill(&mut main, &mut headphones, 48_000, [false, false]);
            out.extend(main.iter().map(|frame| i32::from(frame.left)));
        }
        out
    }
}

fn largest_step(samples: &[i32]) -> i32 {
    samples
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .max()
        .unwrap_or(0)
}

fn peak(samples: &[i32]) -> i32 {
    samples.iter().map(|v| v.abs()).max().unwrap_or(0)
}

/// A clean 1 kHz tone steps by 13 % of its peak per sample; a click steps by far
/// more, whatever the effect does to the level.
fn assert_smooth(samples: &[i32], what: &str) {
    let (step, level) = (largest_step(samples), peak(samples));
    assert!(step * 10 < level * 3, "{what}: step {step} of peak {level}");
}

#[test]
fn beat_fx_echo_rings_on_after_the_deck_stops_and_counts_nothing_as_unsupported() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let mut mix = playing_mix();
    let unsupported = mix.engine().counters()[0];
    let mut tone = Tone(0);
    tone.play(&mut mix, 200);
    // Without an effect, a paused deck is silent at once.
    press(&mut mix, SemanticControl::Play, true);
    run(&mut mix, 3);
    assert_eq!(peak(&tone_silent(&mut mix, 20)), 0);
    press(&mut mix, SemanticControl::Play, true);
    tone.play(&mut mix, 50);
    system_press(&mut mix, SemanticControl::BeatFxSelectNext);
    system_press(&mut mix, SemanticControl::BeatFxOn);
    assert!(mix.engine().beat_fx_on());
    tone.play(&mut mix, 400);
    press(&mut mix, SemanticControl::Play, true);
    run(&mut mix, 3);
    assert!(peak(&tone_silent(&mut mix, 200)) > 100, "the echo rings on");
    // Clear ends it, and the LED follows.
    system_press(&mut mix, SemanticControl::BeatFxClear);
    assert!(!mix.engine().beat_fx_on());
    run(&mut mix, 3);
    assert_eq!(peak(&tone_silent(&mut mix, 20)), 0);
    assert_eq!(mix.engine().counters()[0], unsupported);
}

/// Plays `packets` with no source frames added; returns MAIN's left channel.
fn tone_silent(mix: &mut Mix, packets: usize) -> std::vec::Vec<i32> {
    let (mut main, mut headphones) = ([PcmFrame::default(); 48], [PcmFrame::default(); 48]);
    let mut out = std::vec::Vec::new();
    for _ in 0..packets {
        mix.fill(&mut main, &mut headphones, 48_000, [false, false]);
        out.extend(main.iter().map(|frame| i32::from(frame.left)));
    }
    out
}

#[test]
fn pad_fx_reach_the_audio_while_held() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let mut mix = playing_mix();
    let unsupported = mix.engine().counters()[0];
    let mut tone = Tone(0);
    tone.play(&mut mix, TONE_AFTER_PACKETS);
    let dry = peak(&tone.play(&mut mix, 100)[2_400..]);
    assert!(dry > 0);
    press(&mut mix, SemanticControl::PadModePadFx1, true);
    let pad = |pad, pressed| {
        event(
            SemanticControl::PadAction,
            ControlValue::PadAction(PadAction {
                pad,
                mode: PadMode::PadFx1,
                shifted: false,
                pressed,
            }),
        )
    };
    for number in 0..8 {
        mix.control(&pad(number, true));
        assert!(deck(&mut mix).active_pad_fx.is_some(), "pad {number}");
        let held = tone.play(&mut mix, 700);
        let level = peak(&held[held.len() - 2_400..]);
        mix.control(&pad(number, false));
        assert!(deck(&mut mix).active_pad_fx.is_none(), "pad {number}");
        let released = tone.play(&mut mix, 100);
        tone.play(&mut mix, 2_100);
        // Pads 0 to 3 have effects; the filter pads may lower or raise a 1 kHz
        // tone, and the echoes raise it once they repeat.
        if number < 4 {
            assert!(
                level.abs_diff(dry) > dry as u32 / 20,
                "pad {number}: {level}"
            );
        } else {
            assert_eq!(level, dry, "pad {number}");
        }
        assert_smooth(&held, "pad press");
        assert_smooth(&released, "pad release");
    }
    assert_eq!(mix.engine().counters()[0], unsupported);
}

#[test]
fn turning_each_beat_fx_on_and_off_does_not_click() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let mut mix = playing_mix();
    let mut tone = Tone(0);
    tone.play(&mut mix, TONE_AFTER_PACKETS);
    let dry = peak(&tone.play(&mut mix, 100)[2_400..]);
    assert_smooth(&tone.play(&mut mix, 100), "plain");
    for (effect, changes_level) in [
        ("filter", true),
        ("echo", true),
        ("flanger", true),
        ("delay", true),
    ] {
        system_press(&mut mix, SemanticControl::BeatFxOn);
        let on = tone.play(&mut mix, 400);
        // Before the first repeat of the time effects, which arrives whole.
        assert_smooth(&on[..9_600], effect);
        // One beat at 240 BPM is 250 ms, so the time effects have repeated.
        let level = peak(&on[on.len() - 4_800..]);
        assert_eq!(
            level.abs_diff(dry) > dry as u32 / 20,
            changes_level,
            "{effect}: {level}"
        );
        system_press(&mut mix, SemanticControl::BeatFxOn);
        // The fade and the tail end inside one second.
        assert_smooth(&tone.play(&mut mix, 100), effect);
        tone.play(&mut mix, 2_000);
        system_press(&mut mix, SemanticControl::BeatFxSelectNext);
    }
}
