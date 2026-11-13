use super::*;
use crate::deck_mix::Mix;
use crate::deck_play::attach_ring;
use crate::deck_transport::{LOAD_RESULTS, LoadResult};
use ruzina_audio_dsp::PcmFrame;
use ruzina_controller_core::{ControlValue, PadAction, PadMode, SemanticControl};
use ruzina_deck::DeckState;

/// No cues, for a track with no Rekordbox analysis: not this track's bank.
const NO_CUES: HotCueBank = HotCueBank::empty(MediaTrackId([0; 16]));

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
                    source_cues: NO_CUES,
                    persisted_cues: None,
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
                    source_cues: NO_CUES,
                    persisted_cues: None,
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
    post_load_with_cues(mix, fixture, NO_CUES, None);
}

fn post_load_with_cues(
    mix: &mut Mix,
    fixture: LoadFixture,
    source_cues: HotCueBank,
    persisted_cues: Option<HotCueBank>,
) {
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
            source_cues,
            persisted_cues,
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
                source_cues: NO_CUES,
                persisted_cues: None,
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
                source_cues: NO_CUES,
                persisted_cues: None,
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
                source_cues: NO_CUES,
                persisted_cues: None,
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
                source_cues: NO_CUES,
                persisted_cues: None,
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
            source_cues: NO_CUES,
            persisted_cues: None,
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
    assert!(mix.engine().deck.beat_fx().enabled);
    tone.play(&mut mix, 400);
    press(&mut mix, SemanticControl::Play, true);
    run(&mut mix, 3);
    assert!(peak(&tone_silent(&mut mix, 200)) > 100, "the echo rings on");
    // Clear ends it.
    system_press(&mut mix, SemanticControl::BeatFxClear);
    assert!(!mix.engine().deck.beat_fx().enabled);
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

/// What the flash owner does after each bank.
fn save_done() {
    crate::deck_transport::HOT_CUE_SAVES_DONE.fetch_add(1, core::sync::atomic::Ordering::Release);
}

fn hot_cue_pad(pad: u8, pressed: bool) -> ControlEvent {
    event(
        SemanticControl::PadAction,
        ControlValue::PadAction(PadAction {
            pad,
            mode: PadMode::HotCue,
            shifted: false,
            pressed,
        }),
    )
}

/// A loaded deck 1, with the ring serving, and nothing left from other tests.
fn mix_with_cues(track: u8, source: HotCueBank, persisted: Option<HotCueBank>) -> Mix {
    attach_ring(0);
    attach_ring(1);
    let mut mix = Mix::new();
    SEEK_REQUESTS[0].take();
    SEEK_RESULTS[0].take();
    crate::deck_transport::HOT_CUE_SAVES.take();
    serve(0);
    post_load_with_cues(
        &mut mix,
        LoadFixture {
            ready: true,
            index: u32::from(track),
            bpm_x100: 12_000,
            track_id: [track; 16],
        },
        source,
        persisted,
    );
    run(&mut mix, 3);
    mix
}

#[test]
fn rekordbox_cues_reach_the_pads_and_saved_cues_win_over_them() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let id = MediaTrackId([21; 16]);
    let mut source = HotCueBank::empty(id).as_source();
    assert!(source.set_single(0, 1_500));
    assert!(source.set_single(1, 2_500));
    let mut mix = mix_with_cues(21, source, None);
    assert_eq!(mix.engine().hot_cue_masks(), [0b11, 0]);
    let hot_cue = |mix: &mut Mix| mix.engine().deck.hot_cues(DeckId::One).unwrap();
    assert_eq!(hot_cue(&mut mix).slot(1).unwrap().pos_ms, 2_500);
    // The pad jumps to it, which stores nothing.
    mix.control(&hot_cue_pad(1, true));
    mix.control(&hot_cue_pad(1, false));
    assert_eq!(take_seek(), Some(2_500));
    assert_eq!(mix.engine().hot_cue_saves[0], None);

    // With a saved bank, a track loads with both: the saved slot wins.
    let mut saved = HotCueBank::empty(id);
    assert!(saved.set_single(1, 7_000));
    assert!(saved.set_single(2, 8_000));
    let mut mix = mix_with_cues(21, source, Some(saved));
    assert_eq!(mix.engine().hot_cue_masks(), [0b111, 0]);
    let bank = hot_cue(&mut mix);
    assert_eq!(bank.slot(0).unwrap().pos_ms, 1_500);
    assert_eq!(bank.slot(1).unwrap().pos_ms, 7_000);
}

#[test]
fn a_new_cue_waits_for_the_flash_owner_and_a_reload_keeps_it() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let saves = &crate::deck_transport::HOT_CUE_SAVES;
    let id = MediaTrackId([22; 16]);
    let mut mix = mix_with_cues(22, HotCueBank::empty(id), None);
    let unsupported = mix.engine().counters()[0];
    assert_eq!(mix.engine().hot_cue_masks(), [0, 0]);

    // A pad with no cue stores one. Nothing is offered before the flush.
    mix.control(&hot_cue_pad(0, true));
    mix.control(&hot_cue_pad(0, false));
    assert_eq!(mix.engine().hot_cue_masks(), [1, 0]);
    assert_eq!(mix.engine().counters()[0], unsupported, "it is executed");
    assert!(saves.take().is_none());

    // A reload before the bank reached flash, whose copy is older, keeps it.
    post_load_with_cues(
        &mut mix,
        LoadFixture {
            ready: true,
            index: 22,
            bpm_x100: 12_000,
            track_id: [22; 16],
        },
        HotCueBank::empty(id),
        None,
    );
    run(&mut mix, 3);
    assert_eq!(mix.engine().hot_cue_masks(), [1, 0]);

    // The flash owner gets the latest bank, one per track.
    mix.control(&hot_cue_pad(3, true));
    mix.control(&hot_cue_pad(3, false));
    mix.engine().flush_hot_cues();
    let bank = saves.take().expect("offered");
    assert_eq!(bank.track_id(), id);
    assert_eq!(bank.exists_mask(), 0b1001);
    mix.engine().flush_hot_cues();
    assert!(saves.take().is_none(), "nothing else waits");

    // Until the owner reports it done, a load of the track still gets that bank,
    // because flash may not hold it yet: the owner waits for a silent moment.
    post_load_with_cues(
        &mut mix,
        LoadFixture {
            ready: true,
            index: 22,
            bpm_x100: 12_000,
            track_id: [22; 16],
        },
        HotCueBank::empty(id),
        None,
    );
    run(&mut mix, 3);
    assert_eq!(mix.engine().hot_cue_masks(), [0b1001, 0]);
    save_done();
    mix.engine().flush_hot_cues();
    assert!(mix.engine().hot_cue_in_flight.is_none());

    // A busy owner keeps the bank waiting, in order, with the oldest dropped
    // only when the queue is full.
    let waiting = |track: u8| {
        let mut bank = HotCueBank::empty(MediaTrackId([track; 16]));
        assert!(bank.set_single(0, u32::from(track)));
        bank
    };
    assert!(saves.put(waiting(200)).is_ok());
    for track in 0..17 {
        mix.engine().queue_hot_cues(waiting(track));
    }
    mix.engine().flush_hot_cues();
    assert_eq!(saves.take().unwrap().track_id(), MediaTrackId([200; 16]));
    mix.engine().flush_hot_cues();
    assert_eq!(saves.take().unwrap().track_id(), MediaTrackId([1; 16]));
    // One at a time: the next waits for the owner's report on this one.
    mix.engine().flush_hot_cues();
    assert!(saves.take().is_none());
    save_done();
    for track in 2..17 {
        mix.engine().flush_hot_cues();
        assert_eq!(saves.take().unwrap().track_id(), MediaTrackId([track; 16]));
        save_done();
    }
    mix.engine().flush_hot_cues();
    assert!(saves.take().is_none());
}

#[test]
fn the_leds_follow_the_deck_and_mixer_and_all_reach_the_flx4() {
    use ruzina_controller_core::{ControllerLedFrame, LedState, LedUpdate, SemanticLedId as Led};
    use ruzina_controller_profile::Profile;
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let mut mix = playing_mix();
    let mut leds = ControllerLedFrame::new();
    let on = |mix: &Mix, leds: &mut ControllerLedFrame, led, deck| {
        mix.refresh_leds(leds);
        leds.state(led, deck) == Some(LedState::On)
    };
    let one = Some(DeckId::One);
    let two = Some(DeckId::Two);

    // Every LED of a new binding reaches the controller, and none is left out:
    // a global state for a per-deck LED would map to nothing.
    mix.refresh_leds(&mut leds);
    let profile = Profile::parse(crate::controller_midi::FLX4_PROFILE).unwrap();
    let mut sent = 0;
    while let Some(update) = leds.take_next_dirty() {
        assert!(profile.map_led_update(update).is_some(), "{update:?}");
        sent += 1;
    }
    assert!(sent > 60, "{sent}");
    mix.refresh_leds(&mut leds);
    assert!(!leds.has_dirty(), "an unchanged state sends nothing");

    // Pad mode buttons: exactly the active mode, per deck.
    assert!(on(&mix, &mut leds, Led::PAD_MODE_HOT_CUE, one));
    assert!(!on(&mix, &mut leds, Led::PAD_MODE_SAMPLER, one));
    press(&mut mix, SemanticControl::PadModePadFx1, true);
    assert!(on(&mix, &mut leds, Led::PAD_MODE_PAD_FX1, one));
    assert!(!on(&mix, &mut leds, Led::PAD_MODE_HOT_CUE, one));
    assert!(on(&mix, &mut leds, Led::PAD_MODE_HOT_CUE, two));
    press(&mut mix, SemanticControl::PadModeBeatJump, true);
    assert!(on(&mix, &mut leds, Led::PAD_MODE_BEAT_JUMP, one));
    assert!(!on(&mix, &mut leds, Led::PAD_MODE_PAD_FX1, one));
    // The deck state has no Sampler mode, so its button stays off.
    press(&mut mix, SemanticControl::PadModeSampler, true);
    assert!(on(&mix, &mut leds, Led::PAD_MODE_BEAT_JUMP, one));
    assert!(!on(&mix, &mut leds, Led::PAD_MODE_SAMPLER, one));

    // PFL per deck.
    assert!(!on(&mix, &mut leds, Led::PFL, one));
    press(&mut mix, SemanticControl::Pfl, true);
    press(&mut mix, SemanticControl::Pfl, false);
    assert!(on(&mix, &mut leds, Led::PFL, one));
    assert!(!on(&mix, &mut leds, Led::PFL, two));

    // The system buttons follow the mixer.
    let (cfx, fader, master) = (
        on(&mix, &mut leds, Led::SMART_CFX, None),
        on(&mix, &mut leds, Led::SMART_FADER, None),
        on(&mix, &mut leds, Led::MASTER_CUE, None),
    );
    system_press(&mut mix, SemanticControl::SmartCfx);
    system_press(&mut mix, SemanticControl::SmartFader);
    system_press(&mut mix, SemanticControl::MasterCue);
    assert_eq!(on(&mix, &mut leds, Led::SMART_CFX, None), !cfx);
    assert_eq!(on(&mix, &mut leds, Led::SMART_FADER, None), !fader);
    assert_eq!(on(&mix, &mut leds, Led::MASTER_CUE, None), !master);

    // Beat FX On is per deck on this controller.
    system_press(&mut mix, SemanticControl::BeatFxOn);
    assert!(on(&mix, &mut leds, Led::BEAT_FX_ON, one));
    assert!(on(&mix, &mut leds, Led::BEAT_FX_ON, two));
    let update = LedUpdate {
        led: Led::BEAT_FX_ON,
        deck: one,
        state: LedState::On,
    };
    assert_eq!(profile.map_led_update(update), Some([0x94, 0x47, 0x7f]));
}

#[test]
fn in_loop_adjust_the_jog_edits_the_loop_and_does_not_scratch() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let mut mix = mix_with_cues(23, HotCueBank::empty(MediaTrackId([23; 16])), None);
    press(&mut mix, SemanticControl::Play, true);
    run(&mut mix, 100);
    press(&mut mix, SemanticControl::JogTouch, true);
    assert_eq!(mix.engine().take_scratch(0), Some(0), "a touch scratches");
    press(&mut mix, SemanticControl::JogTouch, false);
    assert_eq!(mix.engine().take_scratch(0), None);
    // With a loop in adjust mode, the same touch leaves the deck playing.
    press(&mut mix, SemanticControl::ReloopExit, true);
    press(&mut mix, SemanticControl::ReloopExit, false);
    mix.control(&event(
        SemanticControl::DeckExtAction,
        ControlValue::DeckExtAction(ruzina_controller_core::DeckExtActionValue {
            action: ruzina_controller_core::DeckExtAction::LoopAdjustIn,
            pressed: true,
        }),
    ));
    let adjust = deck(&mut mix).loop_adjust_mode;
    assert_ne!(
        adjust,
        ruzina_deck::LoopAdjustMode::None,
        "needs loop adjust"
    );
    press(&mut mix, SemanticControl::JogTouch, true);
    assert_eq!(mix.engine().take_scratch(0), None);
    press(&mut mix, SemanticControl::JogTouch, false);
}

#[cfg(feature = "soak-autoplay")]
#[test]
fn the_soak_dj_does_everything_and_leaves_nothing_held() {
    use crate::soak_autoplay::{Exerciser, GESTURES};
    use std::collections::BTreeSet;
    let mut dj = Exerciser::new();
    let mut seen = BTreeSet::new();
    let mut held: BTreeSet<std::string::String> = BTreeSet::new();
    let mut idle_with_nothing_held = 0;
    // An hour at one call per 10 ms, as the USB client loop does at the least.
    for ms in (0..3_600_000u32).step_by(10) {
        let event = dj.next(ms, [true, true]);
        if let Some(started) = dj.take_started() {
            seen.insert(started >> 8);
            assert!(held.is_empty(), "a gesture ended with {held:?} held");
            idle_with_nothing_held += 1;
        }
        let Some(event) = event else { continue };
        let key = match event.value {
            ControlValue::Pressed(pressed) => Some((
                std::format!("{:?} {:?}", event.deck, event.control),
                pressed,
            )),
            ControlValue::PadAction(action) => Some((
                std::format!("{:?} {:?} {}", event.deck, action.mode, action.pad),
                action.pressed,
            )),
            _ => None,
        };
        if let Some((key, pressed)) = key {
            if pressed {
                held.insert(key);
            } else {
                held.remove(&key);
            }
        }
    }
    assert_eq!(seen.len(), GESTURES.len(), "{seen:?}");
    assert!(idle_with_nothing_held > 500, "{idle_with_nothing_held}");
}

#[cfg(feature = "soak-autoplay")]
#[test]
fn the_soak_dj_runs_through_the_mix_without_a_stuck_state() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let mut mix = playing_mix();
    let unsupported = mix.engine().counters()[0];
    for ms in 0..120_000u32 {
        if ms % 10 == 0 {
            mix.exercise(ms);
            mix.engine().flush_hot_cues();
            crate::deck_transport::HOT_CUE_SAVES.take();
            save_done();
        }
        // Serve what the DJ's seeks ask for, as the producer would.
        if take_seek().is_some() {
            acknowledge_seek(&mut mix);
        }
        if ms % 1_000 == 0 {
            run(&mut mix, 1);
        }
    }
    assert_eq!(mix.engine().counters()[0], unsupported);
}

/// A grid of `count` beats `length` ms apart from `first`, numbered 0 to 3 in the bar
/// from `bar_from`.
fn grid(first: u32, length: u32, count: u32, bar_from: u16) -> std::vec::Vec<Beat> {
    (0..count)
        .map(|n| Beat {
            time_ms: first + n * length,
            phase: (bar_from + n as u16) % 4,
            bpm_x100: 6_000_000 / length,
        })
        .collect()
}

#[test]
fn the_phase_jump_lands_on_the_reference_beat_of_the_bar_at_its_place_in_the_beat() {
    // Reference: 500 ms beats; at 2_250 ms it is a quarter into beat 4, the bar's first.
    let reference = grid(125, 500, 40, 0);
    assert_eq!(beat_at(&reference, 2_250), Some((4, 0.25)));
    // Target: 480 ms beats from 50 ms. At 3_000 ms it is in beat 6 (number 2); the
    // bar starts around it are beats 4 (1_970 ms) and 8 (3_890 ms), the first nearer.
    let target = grid(50, 480, 40, 0);
    let jump = phase_jump_ms(&target, 3_000, &reference, 2_250);
    assert_eq!(jump, Some((1_970 + 120 - 3_000, 3_890 + 120 - 3_000)));
    // Already in phase: no jump, and the other way is a bar.
    assert_eq!(
        phase_jump_ms(&target, 4_010, &reference, 2_250),
        Some((0, 1_970 + 120 - 4_010))
    );
    // Beats numbered from another place in the bar shift the target's bars.
    let shifted = grid(50, 480, 40, 1);
    assert_eq!(
        phase_jump_ms(&shifted, 3_000, &reference, 2_250),
        Some((3_410 + 120 - 3_000, 1_490 + 120 - 3_000))
    );
    // Outside either grid there is no phase.
    assert_eq!(phase_jump_ms(&target, 10, &reference, 2_250), None);
    assert_eq!(phase_jump_ms(&target, 3_000, &reference, 100), None);
    assert_eq!(phase_jump_ms(&target, 3_000, &reference, 60_000), None);
}

fn pad(mode: PadMode, pad: u8, pressed: bool) -> ControlEvent {
    event(
        SemanticControl::PadAction,
        ControlValue::PadAction(PadAction {
            pad,
            mode,
            shifted: false,
            pressed,
        }),
    )
}

fn pad_leds(mix: &mut Mix) -> (u8, u8) {
    use ruzina_controller_core::{LedState, SemanticLedId};
    let mut leds = ControllerLedFrame::new();
    mix.refresh_leds(&mut leds);
    let lit = |led: fn(u8) -> Option<SemanticLedId>| {
        (0..8u8).fold(0, |mask, pad| {
            let on = leds.state(led(pad).unwrap(), Some(DeckId::One)) == Some(LedState::On);
            mask | u8::from(on) << pad
        })
    };
    (
        lit(SemanticLedId::beat_loop_pad),
        lit(SemanticLedId::beat_jump_pad),
    )
}

#[test]
fn beat_loop_pads_light_their_running_loop_and_beat_jump_pads_while_held() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let mut mix = playing_mix();
    run(&mut mix, 100);
    assert_eq!(pad_leds(&mut mix), (0, 0));
    // A Beat Loop pad starts its loop and stays lit while the loop runs.
    mix.control(&pad(PadMode::BeatLoop, 3, true));
    assert_eq!(pad_leds(&mut mix), (1 << 3, 0));
    mix.control(&pad(PadMode::BeatLoop, 3, false));
    assert_eq!(pad_leds(&mut mix), (1 << 3, 0));
    // The 4 BEAT button's loop lights its pad, and exit puts it out.
    press(&mut mix, SemanticControl::ReloopExit, true);
    press(&mut mix, SemanticControl::ReloopExit, false);
    assert_eq!(pad_leds(&mut mix), (0, 0));
    press(&mut mix, SemanticControl::ReloopExit, true);
    press(&mut mix, SemanticControl::ReloopExit, false);
    assert_eq!(pad_leds(&mut mix), (1 << FOUR_BEAT_LOOP_PAD, 0));
    press(&mut mix, SemanticControl::ReloopExit, true);
    press(&mut mix, SemanticControl::ReloopExit, false);
    assert_eq!(pad_leds(&mut mix), (0, 0));
    // A Beat Jump pad lights while it is held.
    mix.control(&pad(PadMode::BeatJump, 5, true));
    assert_eq!(pad_leds(&mut mix), (0, 1 << 5));
    mix.control(&pad(PadMode::BeatJump, 5, false));
    assert_eq!(pad_leds(&mut mix), (0, 0));
    // It jumped, through a seek no producer serves here: drop it for the next test.
    assert!(SEEK_REQUESTS[0].take().is_some());
}

/// Deck `deck` loaded with catalog track `index` at 120 BPM, its ring serving a 48 kHz
/// source from the start, and a beat grid of 500 ms beats numbered from the bar's first.
fn load_with_grid(mix: &mut Mix, deck: DeckId, index: u32) {
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
    let i = usize::from(deck == DeckId::Two);
    RINGS[i].restart_at(48_000, 0);
    crate::deck_play::Reader::new().sync(&RINGS[i]);
    let frames = std::vec![PcmFrame { left: 10_000, right: 10_000 }; 20_000];
    assert!(RINGS[i].push(&frames));
    assert!(mix.engine().request_load(deck, index));
    let request = LOAD_REQUESTS[i].take().unwrap();
    GATES[i].begin_replacement(request.permit).unwrap();
    LOAD_RESULTS[i]
        .put(LoadResult {
            source_cues: NO_CUES,
            persisted_cues: None,
            request,
            ready: true,
            bpm_x100: 12_000,
            track_id: [index as u8; 16],
        })
        .unwrap();
    mix.engine().grids[i] = Some(Box::new(Grid {
        index,
        beats: grid(0, 500, 60, 0).into(),
    }));
}

/// Empties deck `i`'s ring of whatever an earlier test left in it.
fn empty_ring(i: usize) {
    RINGS[i].restart_at(48_000, 0);
    crate::deck_play::Reader::new().sync(&RINGS[i]);
}

/// Renders `packets` 1 ms packets while both decks' producers keep 48 frames a packet
/// coming.
fn run_fed(mix: &mut Mix, packets: usize) {
    let (mut main, mut headphones) = ([PcmFrame::default(); 48], [PcmFrame::default(); 48]);
    let frames = [PcmFrame {
        left: 10_000,
        right: 10_000,
    }; 48];
    for _ in 0..packets {
        for ring in &RINGS {
            if ring.free() >= 48 {
                assert!(ring.push(&frames));
            }
        }
        mix.fill(&mut main, &mut headphones, 48_000, [false, false]);
    }
}

fn press_on(mix: &mut Mix, deck: DeckId, control: SemanticControl) {
    for pressed in [true, false] {
        mix.control(&ControlEvent {
            deck: Some(deck),
            control,
            value: ControlValue::Pressed(pressed),
        });
    }
}

#[test]
fn sync_puts_the_deck_in_the_reference_bar_at_once_without_a_seek() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    attach_ring(0);
    attach_ring(1);
    let mut mix = Mix::new();
    SEEK_REQUESTS[0].take();
    SEEK_REQUESTS[1].take();
    load_with_grid(&mut mix, DeckId::One, 31);
    load_with_grid(&mut mix, DeckId::Two, 32);
    run_fed(&mut mix, 3);
    press_on(&mut mix, DeckId::One, SemanticControl::Play);
    run_fed(&mut mix, 330);
    press_on(&mut mix, DeckId::Two, SemanticControl::Play);
    run_fed(&mut mix, 200);
    let positions = |mix: &mut Mix| {
        let engine = mix.engine();
        [DeckId::One, DeckId::Two].map(|deck| engine.deck(deck).position_ms)
    };
    let [one, two] = positions(&mut mix);
    assert!(near(one - two, 330, 30), "{one} {two}");
    press_on(&mut mix, DeckId::Two, SemanticControl::Sync);
    assert!(mix.engine().deck(DeckId::Two).sync_enabled);
    run_fed(&mut mix, 10);
    let (deck, event, value) = mix.engine().take_phase_report().expect("a jump");
    assert_eq!((deck, event), (1, PHASE_JUMPED));
    // Deck 2 has no earlier beat with deck 1's number, so it skips forward through its
    // ring to deck 1's place: by the distance between them.
    let jumped = value as i32 - PHASE_REPORT_ZERO;
    assert!(
        near(jumped as u32, one - two, 3),
        "{jumped} for {one} {two}"
    );
    let [one, two] = positions(&mut mix);
    assert!(two.abs_diff(one) <= 2, "{one} {two}");
    assert_eq!(SEEK_REQUESTS[1].take(), None, "no seek");
    // A second later the phase error is reported, and it is none.
    run_fed(&mut mix, 1_000);
    let (deck, event, value) = mix.engine().take_phase_report().expect("a check");
    assert_eq!((deck, event), (1, PHASE_ERROR));
    assert!((value as i32 - PHASE_REPORT_ZERO).abs() <= 2, "{value}");
    // A second SYNC press turns it off, and nothing more happens.
    press_on(&mut mix, DeckId::Two, SemanticControl::Sync);
    assert!(!mix.engine().deck(DeckId::Two).sync_enabled);
    run_fed(&mut mix, 1_000);
    assert_eq!(mix.engine().take_phase_report(), None);
}

#[test]
fn a_synced_deck_follows_the_other_decks_tempo_and_stays_in_phase() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    attach_ring(0);
    attach_ring(1);
    let mut mix = Mix::new();
    SEEK_REQUESTS[0].take();
    SEEK_REQUESTS[1].take();
    load_with_grid(&mut mix, DeckId::One, 33);
    load_with_grid(&mut mix, DeckId::Two, 34);
    run_fed(&mut mix, 3);
    press_on(&mut mix, DeckId::One, SemanticControl::Play);
    run_fed(&mut mix, 200);
    press_on(&mut mix, DeckId::Two, SemanticControl::Play);
    run_fed(&mut mix, 100);
    press_on(&mut mix, DeckId::Two, SemanticControl::Sync);
    run_fed(&mut mix, 10);
    mix.engine().take_phase_report();
    // Deck 1's Tempo fader to its top end, +10 %: deck 2 follows at once, in SYNC.
    mix.control(&ControlEvent {
        deck: Some(DeckId::One),
        control: SemanticControl::Tempo,
        value: ControlValue::Absolute {
            value: 0,
            max: 16_383,
        },
    });
    assert_eq!(mix.engine().transport().1, [1_000, 1_000]);
    assert!(mix.engine().deck(DeckId::Two).sync_enabled);
    let positions = |mix: &mut Mix| {
        let engine = mix.engine();
        [DeckId::One, DeckId::Two].map(|deck| engine.deck(deck).position_ms)
    };
    let [one, two] = positions(&mut mix);
    run_fed(&mut mix, 500);
    let [one_later, two_later] = positions(&mut mix);
    assert!(near(one_later - one, 550, 10), "{one} {one_later}");
    assert!(
        (two_later - two).abs_diff(one_later - one) <= 2,
        "deck 2 keeps pace: {two} {two_later}"
    );
    // Nothing left for the next test: requests taken, both rings emptied.
    SEEK_REQUESTS[0].take();
    SEEK_REQUESTS[1].take();
    empty_ring(0);
    empty_ring(1);
}

/// SYNC with Master Tempo on `mt_deck`, the deck at +8 %: the phase error a second later.
fn sync_phase_with_master_tempo(mt_deck: DeckId, synced: DeckId) -> i32 {
    use crate::deck_view::TouchControl;
    attach_ring(0);
    attach_ring(1);
    let mut mix = Mix::new();
    mix.attach_keylocks();
    SEEK_REQUESTS[0].take();
    SEEK_REQUESTS[1].take();
    load_with_grid(&mut mix, DeckId::One, 35);
    load_with_grid(&mut mix, DeckId::Two, 36);
    run_fed(&mut mix, 3);
    let mt = usize::from(mt_deck == DeckId::Two);
    let fx = mix.engine().fx_view();
    for event in crate::controller_midi::touch_events(mt, TouchControl::MasterTempo, fx)
        .into_iter()
        .flatten()
    {
        mix.control(&event);
    }
    // The fader at +8 % of its 10 % range.
    mix.control(&ControlEvent {
        deck: Some(mt_deck),
        control: SemanticControl::Tempo,
        value: ControlValue::Absolute {
            value: 1_638,
            max: 16_383,
        },
    });
    press_on(&mut mix, DeckId::One, SemanticControl::Play);
    run_fed(&mut mix, 300);
    press_on(&mut mix, DeckId::Two, SemanticControl::Play);
    run_fed(&mut mix, 200);
    mix.engine().take_phase_report();
    press_on(&mut mix, synced, SemanticControl::Sync);
    let mut error = None;
    for _ in 0..1_500 {
        run_fed(&mut mix, 1);
        if let Some((_, event, value)) = mix.engine().take_phase_report()
            && event == PHASE_ERROR
        {
            error = Some(value as i32 - PHASE_REPORT_ZERO);
            break;
        }
    }
    SEEK_REQUESTS[0].take();
    SEEK_REQUESTS[1].take();
    empty_ring(0);
    empty_ring(1);
    error.expect("a phase check")
}

#[test]
fn master_tempo_going_off_loses_no_audio() {
    use crate::deck_view::TouchControl;
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    attach_ring(0);
    attach_ring(1);
    let mut mix = Mix::new();
    mix.attach_keylocks();
    SEEK_REQUESTS[0].take();
    empty_ring(1);
    load_with_grid(&mut mix, DeckId::One, 37);
    run_fed(&mut mix, 3);
    let toggle = |mix: &mut Mix| {
        let fx = mix.engine().fx_view();
        for event in crate::controller_midi::touch_events(0, TouchControl::MasterTempo, fx)
            .into_iter()
            .flatten()
        {
            mix.control(&event);
        }
    };
    toggle(&mut mix);
    mix.control(&ControlEvent {
        deck: Some(DeckId::One),
        control: SemanticControl::Tempo,
        value: ControlValue::Absolute {
            value: 1_638,
            max: 16_383,
        },
    });
    press_on(&mut mix, DeckId::One, SemanticControl::Play);
    run_fed(&mut mix, 500);
    let before = mix.engine().deck(DeckId::One).position_ms;
    // Master Tempo off: the frames the stretch took but did not play are played now.
    toggle(&mut mix);
    run_fed(&mut mix, 100);
    let after = mix.engine().deck(DeckId::One).position_ms;
    assert!(
        near(after - before, 108, 4),
        "100 ms at +8 % moved {} ms",
        after - before
    );
    SEEK_REQUESTS[0].take();
    empty_ring(0);
    empty_ring(1);
}

#[test]
fn sync_against_or_from_a_master_tempo_deck_lands_in_phase() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let against = sync_phase_with_master_tempo(DeckId::One, DeckId::Two);
    let from = sync_phase_with_master_tempo(DeckId::Two, DeckId::Two);
    std::println!("phase error: against MT {against} ms, from MT {from} ms");
    assert!(
        against.abs() <= 3,
        "against a Master Tempo deck: {against} ms"
    );
    assert!(from.abs() <= 3, "from a Master Tempo deck: {from} ms");
}

#[test]
fn sync_on_a_paused_deck_waits_for_its_grid_and_then_aligns() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    attach_ring(0);
    attach_ring(1);
    let mut mix = Mix::new();
    SEEK_REQUESTS[0].take();
    SEEK_REQUESTS[1].take();
    load_with_grid(&mut mix, DeckId::One, 34);
    load_with_grid(&mut mix, DeckId::Two, 35);
    // Deck 2's first beat comes 300 ms into its track.
    mix.engine().grids[1].as_mut().unwrap().beats = grid(300, 500, 60, 0).into();
    run_fed(&mut mix, 3);
    press_on(&mut mix, DeckId::One, SemanticControl::Play);
    run_fed(&mut mix, 400);
    press_on(&mut mix, DeckId::Two, SemanticControl::Sync);
    assert!(mix.engine().deck(DeckId::Two).sync_enabled);
    press_on(&mut mix, DeckId::Two, SemanticControl::Play);
    run_fed(&mut mix, 200);
    assert_eq!(
        mix.engine().take_phase_report(),
        None,
        "no phase before the grid"
    );
    run_fed(&mut mix, 200);
    let (deck, event, _) = mix.engine().take_phase_report().expect("a jump");
    assert_eq!((deck, event), (1, PHASE_JUMPED));
    run_fed(&mut mix, 1_000);
    let (deck, event, value) = mix.engine().take_phase_report().expect("a check");
    assert_eq!((deck, event), (1, PHASE_ERROR));
    assert!((value as i32 - PHASE_REPORT_ZERO).abs() <= 2, "{value}");
}

#[test]
fn quantize_puts_loops_and_new_cues_on_the_grid_and_beat_jump_keeps_the_offset() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    attach_ring(0);
    attach_ring(1);
    let mut mix = Mix::new();
    SEEK_REQUESTS[0].take();
    empty_ring(1);
    load_with_grid(&mut mix, DeckId::One, 33);
    run_fed(&mut mix, 3);
    press(&mut mix, SemanticControl::Play, true);
    run_fed(&mut mix, 330);
    assert!(deck(&mut mix).quantize_enabled, "on from the first event");
    let at = deck(&mut mix).position_ms;
    assert!(near(at, 330, 30), "{at}");
    // Loop In marks the nearest beat, 500 ms.
    press(&mut mix, SemanticControl::LoopIn, true);
    assert_eq!(deck(&mut mix).loop_state.pending_in_ms, Some(500));
    // A new Hot Cue lands on it too.
    mix.control(&hot_cue_pad(0, true));
    mix.control(&hot_cue_pad(0, false));
    let bank = mix.engine().hot_cue_saves[0].expect("a new cue");
    assert_eq!(bank.slot(0).unwrap().pos_ms, 500);
    // Beat Jump moves by a beat from where the deck is, not to a beat.
    let at = deck(&mut mix).position_ms;
    press(&mut mix, SemanticControl::BeatJumpForward, true);
    assert_eq!(
        SEEK_REQUESTS[0].take().map(|seek| seek.target_ms),
        Some(at + 500)
    );
}

#[test]
fn the_screen_gets_hot_cues_and_the_loop_and_shift_browse_zooms_faster() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    attach_ring(0);
    attach_ring(1);
    let mut mix = Mix::new();
    SEEK_REQUESTS[0].take();
    empty_ring(1);
    load_with_grid(&mut mix, DeckId::One, 36);
    run_fed(&mut mix, 3);
    press(&mut mix, SemanticControl::Play, true);
    run_fed(&mut mix, 330);
    mix.control(&hot_cue_pad(3, true));
    mix.control(&hot_cue_pad(3, false));
    // Loop In shades from its beat while the loop waits for its Out.
    press(&mut mix, SemanticControl::LoopIn, true);
    mix.engine().publish_marks();
    assert_eq!(crate::deck_view::MARKS[0].load().1.pending_in, Some(500));
    mix.control(&pad(PadMode::BeatLoop, 7, true));
    mix.engine().publish_marks();
    let (_, marks) = crate::deck_view::MARKS[0].load();
    assert_eq!(marks.cues[3], Some(500), "on the beat");
    assert_eq!(marks.cues.iter().flatten().count(), 1);
    assert_eq!(marks.looped, Some((500, 2_500)), "4 beats from the beat");
    assert_eq!(marks.pending_in, None, "a running loop has no pending In");
    assert!(
        crate::deck_view::LIVE[0]
            .load()
            .has(crate::deck_view::QUANTIZE)
    );
    press(&mut mix, SemanticControl::ReloopExit, true);
    mix.engine().publish_marks();
    assert_eq!(crate::deck_view::MARKS[0].load().1.looped, None);

    // Shift + browse moves the zoom three steps a detent, from 16 beats to 4.
    crate::deck_view::zoom_by(-10);
    crate::deck_view::zoom_by(4);
    assert_eq!(crate::deck_view::window_ms(12_000), 8_000);
    mix.control(&ControlEvent {
        deck: None,
        control: SemanticControl::ShiftBrowseDelta,
        value: ControlValue::Relative(-1),
    });
    assert_eq!(crate::deck_view::window_ms(12_000), 2_000);
    crate::deck_view::zoom_by(3);
}

#[test]
fn the_hot_cues_page_sets_and_deletes_cues_as_the_pads_do() {
    use crate::controller_midi::touch_events;
    use crate::deck_view::TouchControl;
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    attach_ring(0);
    attach_ring(1);
    let mut mix = Mix::new();
    SEEK_REQUESTS[0].take();
    empty_ring(1);
    load_with_grid(&mut mix, DeckId::One, 36);
    run_fed(&mut mix, 3);
    press(&mut mix, SemanticControl::Play, true);
    run_fed(&mut mix, 330);
    // A tap is the pad going down and then up.
    let tap = |mix: &mut Mix, pad, delete| {
        for down in [true, false] {
            let control = TouchControl::HotCue { pad, delete, down };
            let fx = mix.engine().fx_view();
            for event in touch_events(0, control, fx).into_iter().flatten() {
                mix.control(&event);
            }
        }
    };
    tap(&mut mix, 5, false);
    mix.engine().publish_marks();
    assert_eq!(
        crate::deck_view::MARKS[0].load().1.cues[5],
        Some(500),
        "set on the beat, with Quantize"
    );
    tap(&mut mix, 2, true);
    mix.engine().publish_marks();
    assert_eq!(
        crate::deck_view::MARKS[0]
            .load()
            .1
            .cues
            .iter()
            .flatten()
            .count(),
        1,
        "deleting an empty pad changes nothing"
    );
    tap(&mut mix, 5, true);
    mix.engine().publish_marks();
    assert_eq!(crate::deck_view::MARKS[0].load().1.cues, [None; 8]);
    SEEK_REQUESTS[0].take();
}

#[test]
fn a_tap_on_mt_turns_master_tempo_on_and_off_for_its_deck() {
    use crate::controller_midi::touch_events;
    use crate::deck_view::TouchControl;
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let mut mix = Mix::new();
    let tap = |mix: &mut Mix, deck| {
        let fx = mix.engine().fx_view();
        for event in touch_events(deck, TouchControl::MasterTempo, fx)
            .into_iter()
            .flatten()
        {
            mix.control(&event);
        }
        mix.engine().master_tempo()
    };
    assert_eq!(mix.engine().master_tempo(), [false; 2]);
    assert_eq!(tap(&mut mix, 1), [false, true]);
    assert_eq!(tap(&mut mix, 0), [true, true]);
    assert_eq!(tap(&mut mix, 1), [true, false]);
}

#[test]
fn the_fx_page_picks_the_effect_beat_channel_and_depth_and_turns_fx_on() {
    use crate::controller_midi::touch_events;
    use crate::deck_view::{FxSnapshot, FxTouch, TouchControl};
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let mut mix = Mix::new();
    let touch = |mix: &mut Mix, fx: FxTouch| {
        let now = mix.engine().fx_view();
        for event in touch_events(0, TouchControl::Fx(fx), now)
            .into_iter()
            .flatten()
        {
            mix.control(&event);
        }
        mix.engine().fx_view()
    };
    let start = mix.engine().fx_view();
    assert!(!start.on);
    for effect in [3, 1, 0, 2] {
        assert_eq!(touch(&mut mix, FxTouch::Effect(effect)).effect, effect);
    }
    for beat in [4, 0, 3, 1] {
        assert_eq!(touch(&mut mix, FxTouch::Beat(beat)).beat, beat);
    }
    for target in [1, 2, 0] {
        assert_eq!(touch(&mut mix, FxTouch::Target(target)).target, target);
    }
    assert_eq!(touch(&mut mix, FxTouch::Depth(127)).depth, 127);
    assert_eq!(touch(&mut mix, FxTouch::Depth(40)).depth, 40);
    assert!(touch(&mut mix, FxTouch::On).on);
    let fx = touch(&mut mix, FxTouch::On);
    assert_eq!(
        fx,
        FxSnapshot {
            effect: 2,
            beat: 1,
            target: 0,
            depth: 40,
            on: false,
        }
    );
    // Back to how the other tests expect it.
    mix.control(&ControlEvent {
        deck: None,
        control: SemanticControl::BeatFxClear,
        value: ControlValue::Pressed(true),
    });
}

#[test]
fn on_the_library_page_browse_moves_the_selection_and_load_loads_it() {
    use crate::deck_view::{LIBRARY_OPEN, LibraryRow, publish_library, select, selected};
    use core::sync::atomic::Ordering;
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let mut mix = Mix::new();
    let browse = |mix: &mut Mix, control, delta| {
        mix.control(&ControlEvent {
            deck: None,
            control,
            value: ControlValue::Relative(delta),
        });
    };
    publish_library(std::vec![LibraryRow::default(); 40]);
    select(3);
    let window = crate::deck_view::window_ms(12_000);
    LIBRARY_OPEN.store(true, Ordering::Relaxed);
    browse(&mut mix, SemanticControl::BrowseDelta, 2);
    assert_eq!(selected(), Some(5));
    browse(&mut mix, SemanticControl::ShiftBrowseDelta, 1);
    assert_eq!(selected(), Some(15));
    assert_eq!(crate::deck_view::window_ms(12_000), window, "no zoom");
    LIBRARY_OPEN.store(false, Ordering::Relaxed);
    browse(&mut mix, SemanticControl::BrowseDelta, 1);
    assert_eq!(selected(), Some(15), "the Overview zooms instead");
    browse(&mut mix, SemanticControl::BrowseDelta, -1);
    assert_eq!(crate::deck_view::window_ms(12_000), window);
    // LOAD asks for the selected track on its deck, from any page.
    use ruzina_media_session::{ConnectResult, MediaSession, MediaSourceId};
    let ConnectResult::Accepted(lease) =
        MediaSession::new().on_connect(MediaSourceId::new(7).unwrap())
    else {
        panic!()
    };
    crate::media_signal::activate(lease);
    mix.engine().lease = Some(lease);
    LOAD_REQUESTS[1].take();
    mix.control(&ControlEvent {
        deck: Some(DeckId::Two),
        control: SemanticControl::Load,
        value: ControlValue::Pressed(true),
    });
    let request = LOAD_REQUESTS[1].take().expect("a load");
    assert_eq!(request.index, 15);
    GATES[1].finish(request.permit);
    publish_library(std::vec::Vec::new());
    crate::deck_view::take_library();
}
