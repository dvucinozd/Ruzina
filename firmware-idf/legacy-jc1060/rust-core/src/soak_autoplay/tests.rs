use super::*;

const MEDIA: View = View {
    media: true,
    loaded: false,
    playing: false,
    pending_load: false,
};

fn view(loaded: bool, playing: bool, pending_load: bool) -> View {
    View {
        loaded,
        playing,
        pending_load,
        ..MEDIA
    }
}

/// Drives deck `i` through one successful load and Play; returns the index.
fn load_and_play(auto: &mut AutoPlay, i: usize, tracks: u32) -> u32 {
    let Action::Load(index) = auto.step(i, view(false, false, false), tracks) else {
        panic!("an idle deck asks for a load");
    };
    assert_eq!(auto.step(i, view(false, false, true), tracks), Action::None);
    assert_eq!(auto.step(i, view(true, false, false), tracks), Action::None);
    assert_eq!(auto.step(i, view(true, false, false), tracks), Action::Play);
    assert_eq!(auto.step(i, view(true, true, false), tracks), Action::None);
    index
}

#[test]
fn nothing_happens_before_the_catalog_and_the_media_are_ready() {
    let mut auto = AutoPlay::new();
    assert_eq!(auto.step(0, MEDIA, 0), Action::None);
    let no_media = View {
        media: false,
        ..MEDIA
    };
    assert_eq!(auto.step(0, no_media, 25), Action::None);
    assert_eq!(auto.step(0, MEDIA, 25), Action::Load(0));
}

#[test]
fn each_deck_walks_its_half_of_the_catalog_and_wraps() {
    let mut auto = AutoPlay::new();
    let mut played = [std::vec::Vec::new(), std::vec::Vec::new()];
    for _ in 0..4 {
        for (i, list) in played.iter_mut().enumerate() {
            list.push(load_and_play(&mut auto, i, 5));
            auto.track_ended(i);
            assert_eq!(auto.step(i, view(true, false, false), 5), Action::None);
        }
    }
    assert_eq!(played[0], [0, 2, 4, 1]);
    assert_eq!(played[1], [1, 3, 0, 2]);
    assert_eq!(auto.loads(), [4, 4]);
}

#[test]
fn a_track_that_plays_out_loads_the_next_one_while_the_other_deck_plays() {
    let mut auto = AutoPlay::new();
    assert_eq!(load_and_play(&mut auto, 0, 25), 0);
    assert_eq!(load_and_play(&mut auto, 1, 25), 1);
    // Deck 1 plays on; deck 2 stays playing too.
    for _ in 0..1000 {
        assert_eq!(auto.step(0, view(true, true, false), 25), Action::None);
        assert_eq!(auto.step(1, view(true, true, false), 25), Action::None);
    }
    // Deck 1's track ends: it asks for track 2 while deck 2 keeps playing.
    auto.track_ended(0);
    assert_eq!(auto.step(0, view(true, false, false), 25), Action::None);
    assert_eq!(auto.step(0, view(true, false, false), 25), Action::Load(2));
    assert_eq!(auto.step(1, view(true, true, false), 25), Action::None);
}

#[test]
fn an_operator_pause_is_left_alone() {
    let mut auto = AutoPlay::new();
    load_and_play(&mut auto, 0, 25);
    for _ in 0..2 * PLAY_WAIT_STEPS {
        assert_eq!(auto.step(0, view(true, false, false), 25), Action::None);
    }
    // The operator presses Play again; the walk continues at the end.
    assert_eq!(auto.step(0, view(true, true, false), 25), Action::None);
    auto.track_ended(0);
    assert_eq!(auto.step(0, view(true, false, false), 25), Action::None);
    assert_eq!(auto.step(0, view(true, false, false), 25), Action::Load(2));
}

#[test]
fn failed_loads_move_on_and_three_in_a_row_stop_the_deck() {
    let mut auto = AutoPlay::new();
    let mut asked = std::vec::Vec::new();
    for _ in 0..MAX_FAILURES {
        let Action::Load(index) = auto.step(0, MEDIA, 25) else {
            panic!("a load after a failure");
        };
        asked.push(index);
        assert_eq!(auto.step(0, view(false, false, true), 25), Action::None);
        // The producer could not load it: the deck stays unloaded.
        assert_eq!(auto.step(0, view(false, false, false), 25), Action::None);
    }
    assert_eq!(asked, [0, 2, 4]);
    for _ in 0..10 {
        assert_eq!(auto.step(0, MEDIA, 25), Action::None);
    }
}

#[test]
fn a_success_resets_the_failure_count() {
    let mut auto = AutoPlay::new();
    for _ in 0..MAX_FAILURES - 1 {
        let Action::Load(_) = auto.step(0, MEDIA, 25) else {
            panic!("load");
        };
        auto.step(0, view(false, false, false), 25);
    }
    assert_eq!(load_and_play(&mut auto, 0, 25), 4);
    auto.track_ended(0);
    auto.step(0, view(true, false, false), 25);
    for _ in 0..MAX_FAILURES - 1 {
        let Action::Load(_) = auto.step(0, view(true, false, false), 25) else {
            panic!("load");
        };
        auto.step(0, view(false, false, false), 25);
    }
    assert!(matches!(auto.step(0, MEDIA, 25), Action::Load(_)));
}

#[test]
fn an_unconfirmed_play_is_pressed_again_then_given_up() {
    let mut auto = AutoPlay::new();
    let Action::Load(_) = auto.step(0, MEDIA, 25) else {
        panic!("load");
    };
    auto.step(0, view(true, false, false), 25);
    for _ in 0..MAX_FAILURES {
        assert_eq!(auto.step(0, view(true, false, false), 25), Action::Play);
        for _ in 0..=PLAY_WAIT_STEPS {
            assert_eq!(auto.step(0, view(true, false, false), 25), Action::None);
        }
    }
    for _ in 0..10 {
        assert_eq!(auto.step(0, view(true, false, false), 25), Action::None);
    }
}

#[test]
fn a_refused_load_is_asked_again_and_a_playing_idle_deck_is_left_alone() {
    let mut auto = AutoPlay::new();
    assert_eq!(auto.step(0, MEDIA, 25), Action::Load(0));
    auto.load_refused(0);
    assert_eq!(auto.step(0, MEDIA, 25), Action::Load(0));
    auto.load_refused(0);
    assert_eq!(auto.step(0, view(true, true, false), 25), Action::None);
}

#[test]
fn losing_the_media_returns_the_deck_to_idle() {
    let mut auto = AutoPlay::new();
    load_and_play(&mut auto, 0, 25);
    let gone = View {
        media: false,
        ..MEDIA
    };
    assert_eq!(auto.step(0, gone, 25), Action::None);
    assert_eq!(auto.step(0, gone, 25), Action::None);
    assert_eq!(auto.step(0, MEDIA, 25), Action::Load(0));
}
