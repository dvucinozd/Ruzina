use super::*;
use ruzina_controller_core::{DeckId, PadAction, PadMode, SemanticControl};

extern crate std;

/// The flags are process-wide; one test at a time.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    let guard = crate::DECK_TESTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    ACTIVITY.store(false, Ordering::Release);
    guard
}

const WAIT: u32 = 120_000;

fn event(control: SemanticControl, value: ControlValue) -> ControlEvent {
    ControlEvent {
        deck: Some(DeckId::One),
        control,
        value,
    }
}

fn play(pressed: bool) -> ControlEvent {
    event(SemanticControl::Play, ControlValue::Pressed(pressed))
}

fn fader() -> ControlEvent {
    event(
        SemanticControl::ChannelVolume,
        ControlValue::Absolute {
            value: 100,
            max: 16_383,
        },
    )
}

/// A saver past its boot screen at `BOOT_MS`, then idle until the screensaver shows.
fn sleeping() -> (Saver, u32) {
    let mut saver = Saver::new(0, WAIT);
    assert_eq!(saver.tick(BOOT_MS, false, false, WAIT), Cover::None);
    let at = BOOT_MS + WAIT;
    assert_eq!(saver.tick(at, false, false, WAIT), Cover::Screensaver);
    (saver, at)
}

#[test]
fn the_boot_screen_shows_first_and_a_touch_does_not_cut_it_short() {
    let _serial = serial();
    let mut saver = Saver::new(5, WAIT);
    assert_eq!(saver.tick(5, false, false, WAIT), Cover::Boot);
    assert_eq!(saver.tick(1_000, true, false, WAIT), Cover::Boot);
    assert!(
        controller(&play(true)),
        "the deck is not asleep while it boots"
    );
    assert_eq!(saver.tick(BOOT_MS + 4, false, true, WAIT), Cover::Boot);
    assert_eq!(saver.tick(BOOT_MS + 5, false, false, WAIT), Cover::None);
    // The wait counts from the deck's first frame, not from what came while it booted.
    assert_eq!(
        saver.tick(BOOT_MS + 5 + WAIT - 1, false, false, WAIT),
        Cover::None
    );
    assert_eq!(
        saver.tick(BOOT_MS + 5 + WAIT, false, false, WAIT),
        Cover::Screensaver
    );
}

#[test]
fn a_touch_wakes_it_and_restarts_the_wait() {
    let _serial = serial();
    let (mut saver, at) = sleeping();
    assert_eq!(saver.tick(at + 10, true, false, WAIT), Cover::None);
    assert_eq!(
        saver.tick(at + 10 + WAIT - 1, false, false, WAIT),
        Cover::None
    );
    assert_eq!(
        saver.tick(at + 10 + WAIT, false, false, WAIT),
        Cover::Screensaver
    );
}

#[test]
fn a_press_only_wakes_it_while_faders_and_releases_go_on() {
    let _serial = serial();
    let (mut saver, at) = sleeping();
    assert!(controller(&fader()), "a fader always acts");
    assert!(controller(&play(false)), "a release always goes on");
    let pad = ControlValue::PadAction(PadAction {
        pad: 0,
        mode: PadMode::HotCue,
        shifted: false,
        pressed: true,
    });
    for value in [ControlValue::Pressed(true), pad, ControlValue::Relative(1)] {
        assert!(
            !controller(&event(SemanticControl::Play, value)),
            "{value:?}"
        );
    }
    // Woken: the next press acts.
    assert_eq!(saver.tick(at + 30, false, false, WAIT), Cover::None);
    assert!(controller(&play(true)));
    // Every control is activity, a fader too.
    assert_eq!(saver.tick(at + 60, false, false, WAIT), Cover::None);
    let later = at + 60 + WAIT - 1;
    assert!(controller(&fader()));
    assert_eq!(saver.tick(later, false, false, WAIT), Cover::None);
    assert_eq!(
        saver.tick(later + WAIT, false, false, WAIT),
        Cover::Screensaver
    );
}

#[test]
fn the_web_remote_wakes_it() {
    let _serial = serial();
    let (mut saver, at) = sleeping();
    remote();
    assert_eq!(saver.tick(at + 30, false, false, WAIT), Cover::None);
}

#[test]
fn a_playing_deck_holds_it_off_and_off_turns_it_off() {
    let _serial = serial();
    let (mut saver, at) = sleeping();
    // A deck the Web Remote or the soak started: the screensaver goes.
    assert_eq!(saver.tick(at + 1, false, true, WAIT), Cover::None);
    assert_eq!(saver.tick(at + 10 * WAIT, false, true, WAIT), Cover::None);
    // Stopping counts as activity.
    let stopped = at + 10 * WAIT + 1;
    assert_eq!(saver.tick(stopped, false, false, WAIT), Cover::None);
    assert_eq!(
        saver.tick(stopped + WAIT - 2, false, false, WAIT),
        Cover::None
    );
    // A longer wait set meanwhile counts from when it was set.
    let longer = 5 * 60 * 1000;
    assert_eq!(
        saver.tick(stopped + WAIT - 1, false, false, longer),
        Cover::None
    );
    assert_eq!(
        saver.tick(stopped + WAIT - 1 + longer - 1, false, false, longer),
        Cover::None
    );
    assert_eq!(
        saver.tick(stopped + WAIT - 1 + longer, false, false, longer),
        Cover::Screensaver
    );
    // OFF takes it down and keeps it down.
    assert_eq!(
        saver.tick(stopped + 100 * WAIT, false, false, 0),
        Cover::None
    );
    assert_eq!(
        saver.tick(stopped + 200 * WAIT, false, false, 0),
        Cover::None
    );
}
