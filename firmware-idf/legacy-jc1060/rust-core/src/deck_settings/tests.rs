use super::*;

extern crate std;

/// The settings are process-wide, and the engine's tests use them too; one test at a
/// time.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    let guard = crate::DECK_TESTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset();
    guard
}

#[test]
fn every_setting_survives_the_packing() {
    let _serial = serial();
    let settings = ProductSettings {
        backlight_percent: 45,
        time_display: TimeDisplay::Remaining,
        cue_mode: CueMode::SplitMono,
        master_trim: MasterTrim::Minus6Db,
        wifi_remote: false,
        dj_link: true,
        load_lock: false,
        jog_modes: [JogMode::Cdj, JogMode::Vinyl],
    };
    assert_eq!(unpack(pack(&settings)), settings);
    let other = ProductSettings {
        master_trim: MasterTrim::Minus3Db,
        jog_modes: [JogMode::Vinyl, JogMode::Cdj],
        ..BOARD_DEFAULT
    };
    assert_eq!(unpack(pack(&other)), other);
    assert_eq!(current(), BOARD_DEFAULT);
    // Out of the box: as bright as the screen has always started, with LOAD LOCK on.
    assert_eq!(current().backlight_percent, 25);
    assert!(current().load_lock);
}

#[test]
fn a_change_is_saved_once_it_has_been_quiet() {
    let _serial = serial();
    assert_eq!(take_owed(10_000), None);
    let packed = current_packed();
    let changed = change(1_000, |s| s.load_lock = false);
    assert!(!changed.load_lock);
    assert_ne!(current_packed(), packed);
    // Not before the change has been quiet for two seconds.
    assert_eq!(take_owed(1_000 + SAVE_QUIET_MS - 1), None);
    // A second change restarts the wait, and the save carries both.
    change(2_500, |s| s.jog_modes[1] = JogMode::Cdj);
    assert_eq!(take_owed(1_000 + SAVE_QUIET_MS), None);
    let saved = take_owed(2_500 + SAVE_QUIET_MS).unwrap();
    assert!(!saved.load_lock);
    assert_eq!(saved.jog_modes, [JogMode::Vinyl, JogMode::Cdj]);
    // Taken once.
    assert_eq!(take_owed(60_000), None);
    // A change to what is already in force owes nothing.
    change(70_000, |s| s.load_lock = false);
    assert_eq!(take_owed(80_000), None);
    // The clock may wrap.
    change(u32::MAX - 500, |s| s.load_lock = true);
    assert_eq!(take_owed(100), None);
    assert!(take_owed(SAVE_QUIET_MS).unwrap().load_lock);
}

#[test]
fn the_brightness_stays_within_the_boards_range() {
    let _serial = serial();
    assert_eq!(step_backlight(25, true), 30);
    assert_eq!(step_backlight(25, false), 20);
    assert_eq!(
        step_backlight(BACKLIGHT_MAX_PERCENT, true),
        BACKLIGHT_MAX_PERCENT
    );
    assert_eq!(
        step_backlight(BACKLIGHT_MIN_PERCENT, false),
        BACKLIGHT_MIN_PERCENT
    );
    // A record from a brighter setting comes back within range, and owes no save.
    publish_loaded(&ProductSettings {
        backlight_percent: 80,
        ..BOARD_DEFAULT
    });
    assert_eq!(current().backlight_percent, BACKLIGHT_MAX_PERCENT);
    assert_eq!(take_owed(60_000), None);
    assert_eq!(
        change(0, |s| s.backlight_percent = 100).backlight_percent,
        BACKLIGHT_MAX_PERCENT
    );
}

#[test]
fn the_trim_lowers_main_by_its_decibels() {
    let db = |trim| 20.0 * trim_gain(trim).log10();
    assert!(db(MasterTrim::ZeroDb).abs() < 0.001);
    assert!((db(MasterTrim::Minus3Db) + 3.0).abs() < 0.001);
    assert!((db(MasterTrim::Minus6Db) + 6.0).abs() < 0.001);
}
