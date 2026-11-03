// With the deck engine, playback is the engine's (see `deck_engine` tests).
#![cfg(not(feature = "deck-engine"))]
use super::*;
use crate::deck_play::attach_ring;
use std::vec::Vec;

fn absolute(control: SemanticControl, deck: Option<DeckId>, value: u16) -> ControlEvent {
    ControlEvent {
        deck,
        control,
        value: ControlValue::Absolute {
            value,
            max: MIXER_CONTROL_MAX,
        },
    }
}

fn press(control: SemanticControl, deck: DeckId) -> ControlEvent {
    ControlEvent {
        deck: Some(deck),
        control,
        value: ControlValue::Pressed(true),
    }
}

/// A new source on `deck`: `frames` constant frames at `rate`.
fn source(deck: usize, rate: u32, value: i16, frames: usize) {
    RINGS[deck].restart(rate);
    let block = std::vec![PcmFrame { left: value, right: value }; frames];
    assert!(RINGS[deck].push(&block));
}

/// Renders `packets` 1 ms packets; returns the last packet's MAIN and
/// headphone left samples.
fn run(mix: &mut Mix, packets: usize, playing: [bool; 2]) -> (Vec<i16>, Vec<i16>) {
    let (mut main, mut headphones) = ([PcmFrame::default(); 48], [PcmFrame::default(); 48]);
    for _ in 0..packets {
        mix.fill(&mut main, &mut headphones, 48_000, playing);
    }
    (
        main.iter().map(|f| f.left).collect(),
        headphones.iter().map(|f| f.left).collect(),
    )
}

fn near(samples: &[i16], expected: i16) -> bool {
    samples
        .iter()
        .all(|s| (i32::from(*s) - i32::from(expected)).abs() <= 2)
}

// One test: the deck rings are process-wide statics, as on the device.
#[test]
fn two_decks_mix_through_the_product_graph() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    attach_ring(0);
    attach_ring(1);
    let mut mix = Mix::new();
    // Nothing loaded: silence even while playing.
    let (main, headphones) = run(&mut mix, 4, [true, true]);
    assert!(main.iter().chain(&headphones).all(|s| *s == 0));

    source(0, 48_000, 10_000, 20_000);
    source(1, 48_000, -8_000, 20_000);
    // Deck 1 alone, at the start levels: Master Volume a quarter (MAIN
    // 2500), Headphone Level a quarter and the -12 dB ceiling on the
    // MAIN monitor (156).
    let (main, headphones) = run(&mut mix, 10, [true, false]);
    assert!(near(&main, 2_500), "{main:?}");
    assert!(near(&headphones, 156), "{headphones:?}");
    assert_eq!(mix.stats()[5..7], [48_000, 48_000]);

    // Both decks at the centred crossfader sum; each crossfader end keeps
    // only its own deck.
    let (main, _) = run(&mut mix, 10, [true, true]);
    assert!(near(&main, 500), "{main:?}");
    mix.control(&absolute(SemanticControl::Crossfader, None, 0));
    let (main, _) = run(&mut mix, 10, [true, true]);
    assert!(near(&main, 2_500), "{main:?}");
    mix.control(&absolute(
        SemanticControl::Crossfader,
        None,
        MIXER_CONTROL_MAX,
    ));
    let (main, _) = run(&mut mix, 10, [true, true]);
    assert!(near(&main, -2_000), "{main:?}");

    // Deck 1's channel fader down silences it on MAIN, but its PFL tap is
    // pre-fader: with Headphone Mix on CUE the headphones hear deck 1 only.
    mix.control(&absolute(SemanticControl::Crossfader, None, 8_192));
    mix.control(&absolute(
        SemanticControl::ChannelVolume,
        Some(DeckId::One),
        0,
    ));
    mix.control(&press(SemanticControl::Pfl, DeckId::One));
    mix.control(&absolute(SemanticControl::HeadphoneMix, None, 0));
    let (main, headphones) = run(&mut mix, 10, [true, true]);
    assert!(near(&main, -2_000), "{main:?}");
    assert!(near(&headphones, 625), "{headphones:?}");

    // Master Volume at full: MAIN carries deck 2 at unity.
    mix.control(&absolute(
        SemanticControl::MasterVolume,
        None,
        MIXER_CONTROL_MAX,
    ));
    let (main, _) = run(&mut mix, 10, [true, true]);
    assert!(near(&main, -8_000), "{main:?}");

    // Tempo fader at the top end of +-10 %: deck 1 takes 10 % more source.
    let before = mix.stats()[3];
    run(&mut mix, 100, [true, false]);
    let normal = before - mix.stats()[3];
    mix.control(&absolute(SemanticControl::Tempo, Some(DeckId::One), 0));
    let before = mix.stats()[3];
    run(&mut mix, 100, [true, false]);
    let faster = before - mix.stats()[3];
    assert!((i64::from(normal) - 4_800).abs() <= 128, "{normal}");
    assert!((i64::from(faster) - 5_280).abs() <= 128, "{faster}");

    // Paused decks consume nothing.
    let buffered = mix.stats()[3..5].to_vec();
    run(&mut mix, 10, [false, false]);
    assert_eq!(mix.stats()[3..5], buffered[..]);

    // A playing deck that runs dry counts underruns; at the end of its
    // track it does not.
    assert_eq!(mix.stats()[1..3], [0, 0]);
    run(&mut mix, 400, [true, false]);
    let dry = mix.stats()[1];
    assert!(dry > 0);
    RINGS[0].end();
    run(&mut mix, 100, [true, false]);
    assert_eq!(mix.stats()[1], dry);

    // A device not at 48 kHz gets silence.
    source(1, 44_100, 9_000, 4_000);
    let (mut main, mut headphones) = (
        [PcmFrame { left: 1, right: 1 }; 44],
        [PcmFrame::default(); 44],
    );
    mix.fill(&mut main, &mut headphones, 44_100, [true, true]);
    assert!(main.iter().all(|f| *f == PcmFrame::default()));
    assert!(mix.stats()[0] > 0);
}
