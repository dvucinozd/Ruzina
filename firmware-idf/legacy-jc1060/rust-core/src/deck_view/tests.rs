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
    // The only test that moves the zoom.
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
