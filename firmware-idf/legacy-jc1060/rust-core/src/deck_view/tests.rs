use super::*;

#[test]
fn texts_read_like_the_deck_panel() {
    assert_eq!(clock(0), "00:00");
    assert_eq!(clock(61_999), "01:01");
    assert_eq!(clock(3_723_000), "1:02:03");
    assert_eq!(remaining(1_000, 182_400), "-03:02");
    assert_eq!(remaining(182_400, 182_400), "-00:00");
    assert_eq!(remaining(200_000, 182_400), "-00:00");
    assert_eq!(pitch(0), "+0.00%");
    assert_eq!(pitch(156), "+1.56%");
    assert_eq!(pitch(-1_000), "-10.00%");
    assert_eq!(bpm(12_800, 0), "128.00");
    assert_eq!(bpm(12_800, 156), "130.00");
    assert_eq!(bpm(12_000, -1_000), "108.00");
    assert_eq!(bpm(0, 0), "--.--");
}

#[test]
fn the_fx_word_round_trips() {
    for effect in 0..4 {
        for beat in 0..5 {
            let fx = FxSnapshot {
                effect,
                beat,
                target: 2,
                depth: 127,
                on: true,
            };
            assert_eq!(FxSnapshot::unpack(fx.pack()), fx);
        }
    }
    let fx = FxSnapshot {
        depth: 64,
        ..FxSnapshot::default()
    };
    assert_eq!(fx.depth_percent(), 50);
    assert_eq!(fx.effect_name(), "FILTER");
    assert_eq!(FxSnapshot { beat: 2, ..fx }.beat_name(), "1");
}

#[test]
fn the_browse_knob_steps_the_zoom_through_the_donors_beats() {
    // The zoom is shared: the engine's Shift + browse test moves it too.
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(window_ms(12_000), 8_000, "16 beats at 120 BPM by default");
    zoom_by(-1);
    assert_eq!(window_ms(12_000), 6_000);
    zoom_by(-10);
    assert_eq!(window_ms(12_000), 1_000, "2 beats at the closest");
    assert_eq!(window_ms(0), 1_000, "500 ms beats without a tempo");
    assert_eq!(window_ms(30_000), 500, "at least 0.5 s");
    zoom_by(10);
    assert_eq!(window_ms(12_000), 16_000, "32 beats at the widest");
    assert_eq!(window_ms(6_000), 30_000, "at most 30 s");
    assert_eq!(window_ms(1), 30_000);
    zoom_by(-1);
    assert_eq!(window_ms(12_000), 12_000);
    zoom_by(-1);
    assert_eq!(window_ms(12_000), 8_000);
}

#[test]
fn the_meter_reads_the_loudest_peak_since_it_last_looked() {
    let peaks = Peaks::new();
    peaks.add([0, 1_000]);
    peaks.add([200, 23_197]);
    peaks.add([0, 500]);
    assert_eq!(peaks.take(1), 23_197);
    assert_eq!(peaks.take(1), 0);
    assert_eq!(peaks.take(0), 200);
    assert_eq!(meter_segments(0), 0);
    assert_eq!(meter_segments(327), 0, "below -40 dBFS");
    assert_eq!(meter_segments(328), 1);
    assert_eq!(meter_segments(11_627), 7, "-9 dBFS: all green");
    assert_eq!(meter_segments(23_197), 9, "-3 dBFS: the amber");
    assert_eq!(meter_segments(30_934), 10, "-0.5 dBFS: red");
    assert_eq!(meter_segments(u32::from(u16::MAX)), 10);
}

#[test]
fn the_controller_meter_rises_at_once_and_falls_by_steps() {
    assert_eq!(meter_led(0, 32_767), 126);
    assert_eq!(meter_led(0, u32::from(u16::MAX)), 127, "clipped at the top");
    assert_eq!(meter_led(0, 16_384), 63, "-6 dBFS: half way");
    assert_eq!(meter_led(100, 0), 88, "falls 12 an update");
    assert_eq!(meter_led(100, 16_384), 88);
    assert_eq!(meter_led(60, 16_384), 63, "a louder peak wins");
    assert_eq!(meter_led(5, 0), 0);
}

#[test]
fn touches_reach_the_usb_task_in_order_and_a_full_queue_drops_the_newest() {
    let queue = TouchQueue::new();
    assert!(queue.push(0, TouchControl::Play));
    assert!(queue.push(1, TouchControl::Cue(true)));
    assert!(queue.push(1, TouchControl::Cue(false)));
    assert_eq!(queue.pop(), Some((0, TouchControl::Play)));
    assert_eq!(queue.pop(), Some((1, TouchControl::Cue(true))));
    assert_eq!(queue.pop(), Some((1, TouchControl::Cue(false))));
    assert_eq!(queue.pop(), None);
    for _ in 0..TOUCH_SLOTS {
        assert!(queue.push(0, TouchControl::Play));
    }
    assert!(!queue.push(1, TouchControl::Play), "full");
    assert_eq!(queue.pop(), Some((0, TouchControl::Play)));
    assert!(queue.push(1, TouchControl::Cue(true)));
    while queue.pop().is_some() {}
    assert!(queue.push(1, TouchControl::Load));
    assert_eq!(queue.pop(), Some((1, TouchControl::Load)));
    for (pad, delete, down) in [(0, false, true), (7, true, false), (5, true, true)] {
        let control = TouchControl::HotCue { pad, delete, down };
        assert!(queue.push(0, control));
        assert_eq!(queue.pop(), Some((0, control)));
    }
}

#[test]
fn the_library_selection_stays_within_the_list() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let row = |title: &str| LibraryRow {
        title: title.into(),
        ..LibraryRow::default()
    };
    publish_library(std::vec![row("a"), row("b"), row("c")]);
    assert_eq!(take_library().map(|library| library.rows.len()), Some(3));
    assert!(take_library().is_none());
    select(0);
    browse(1);
    assert_eq!(selected(), Some(1));
    browse(SHIFT_BROWSE_ROWS);
    assert_eq!(selected(), Some(2), "stops at the last");
    browse(-SHIFT_BROWSE_ROWS);
    assert_eq!(selected(), Some(0), "stops at the first");
    select(9);
    assert_eq!(selected(), Some(2));
    publish_library(std::vec![row("a")]);
    assert_eq!(selected(), Some(0), "a shorter list keeps it inside");
    publish_library(std::vec::Vec::new());
    assert_eq!(selected(), None);
}

#[test]
fn the_mailbox_hands_over_the_latest_track_once() {
    // Deck 2 only: deck 1's slot belongs to the engine tests that load tracks.
    while take_track(1).is_some() {}
    let mut first = LoadedTrack::none();
    first.index = 1;
    let mut second = LoadedTrack::none();
    second.index = 2;
    publish_track(1, Box::new(first));
    publish_track(1, Box::new(second));
    assert_eq!(take_track(1).map(|track| track.index), Some(2));
    assert!(take_track(1).is_none());
}

#[test]
fn marks_change_their_revision_only_when_they_change() {
    let slot = MarksSlot::new();
    let (first, marks) = slot.load();
    assert_eq!(marks, Marks::NONE);
    let mut cued = Marks::NONE;
    cued.cues[2] = Some(1_500);
    cued.looped = Some((2_000, 4_000));
    cued.pending_in = Some(7_000);
    slot.store(cued);
    let (second, marks) = slot.load();
    assert_eq!(marks, cued);
    assert_ne!(second, first);
    assert_eq!(second % 2, 0, "even once written");
    slot.store(cued);
    assert_eq!(slot.load().0, second, "the same marks keep the revision");
    slot.store(Marks::NONE);
    assert_eq!(slot.load().1, Marks::NONE);
}
