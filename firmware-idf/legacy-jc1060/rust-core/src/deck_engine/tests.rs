use super::*;
use crate::deck_mix::Mix;
use crate::deck_play::attach_ring;
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
    COMMANDS[0].take();
    COMMANDS[0].take_seek();

    // Play on an empty deck is refused.
    press(&mut mix, SemanticControl::Play, true);
    run(&mut mix, 2);
    assert_eq!(mix.engine().playing(), [false, false]);

    // A load commits once its producer has published it.
    serve(0);
    LOADED[0].post(LoadedTrack {
        ready: true,
        index: 3,
        bpm_x100: 12_800,
        track_id: [7; 16],
    });
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
    assert_eq!(COMMANDS[0].take_seek(), Some(0));
    assert_eq!(mix.engine().playing(), [false, false]);
    run(&mut mix, 5);
    assert_eq!(deck(&mut mix).position_ms, 0);
    serve(0);
    assert_eq!(run(&mut mix, 5), 0);
    assert_eq!(deck(&mut mix).position_ms, 0);

    // Stopped at the cue, CUE previews while held and returns on release.
    press(&mut mix, SemanticControl::Cue, true);
    assert_eq!(mix.engine().playing(), [true, false]);
    assert_eq!(run(&mut mix, 50), 2_500);
    press(&mut mix, SemanticControl::Cue, false);
    assert_eq!(mix.engine().playing(), [false, false]);
    assert_eq!(COMMANDS[0].take_seek(), Some(0));
    serve(0);
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
    assert_eq!(COMMANDS[0].take_seek(), None);

    // The jog scrubs while paused (a seek) and bends while playing.
    mix.control(&event(SemanticControl::JogBend, ControlValue::Relative(10)));
    let target = COMMANDS[0].take_seek().expect("scrub seek");
    assert!(target > paused);
    serve(target);
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
    assert_eq!(COMMANDS[0].take_seek(), Some(stored));
    serve(stored);
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
    LOADED[0].post(LoadedTrack {
        ready: false,
        index: 4,
        bpm_x100: 0,
        track_id: [8; 16],
    });
    run(&mut mix, 3); // at least one 128-frame block, where loads commit
    press(&mut mix, SemanticControl::Play, true);
    assert_eq!(mix.engine().playing(), [false, false]);
    assert_eq!(mix.engine().counters()[1], 1);
}

#[test]
fn the_load_mailbox_reads_only_whole_posts() {
    let mailbox = Loaded::new();
    let mut seen = 0;
    assert_eq!(mailbox.take(&mut seen), None);
    let track = LoadedTrack {
        ready: true,
        index: 9,
        bpm_x100: 17_400,
        track_id: core::array::from_fn(|i| i as u8),
    };
    mailbox.post(track);
    assert_eq!(mailbox.take(&mut seen), Some(track));
    assert_eq!(mailbox.take(&mut seen), None);
    mailbox.seq.fetch_add(1, Ordering::Relaxed); // a writer mid-post
    assert_eq!(mailbox.take(&mut seen), None);
}
