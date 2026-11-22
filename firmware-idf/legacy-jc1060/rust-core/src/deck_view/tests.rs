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
    assert!(queue.push(0, TouchControl::LibraryOpen));
    assert!(queue.push(0, TouchControl::LibraryBack));
    assert_eq!(queue.pop(), Some((0, TouchControl::LibraryOpen)));
    assert_eq!(queue.pop(), Some((0, TouchControl::LibraryBack)));
    assert!(queue.push(1, TouchControl::Memory(0)));
    assert!(queue.push(0, TouchControl::Memory(15)));
    assert_eq!(queue.pop(), Some((1, TouchControl::Memory(0))));
    assert_eq!(queue.pop(), Some((0, TouchControl::Memory(15))));
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
    assert!(queue.push(0, TouchControl::MasterTempo));
    assert_eq!(queue.pop(), Some((0, TouchControl::MasterTempo)));
    for fx in [
        FxTouch::Effect(3),
        FxTouch::Beat(4),
        FxTouch::Target(2),
        FxTouch::Depth(127),
        FxTouch::Depth(0),
        FxTouch::On,
    ] {
        assert!(queue.push(1, TouchControl::Fx(fx)));
        assert_eq!(queue.pop(), Some((1, TouchControl::Fx(fx))));
    }
    for (pad, delete, down) in [(0, false, true), (7, true, false), (5, true, true)] {
        let control = TouchControl::HotCue { pad, delete, down };
        assert!(queue.push(0, control));
        assert_eq!(queue.pop(), Some((0, control)));
    }
}

#[test]
fn the_settings_page_gets_the_mix_figures() {
    publish_stats([1, 2, 3, 4, 5, 6, 7, 8]);
    let read: [u32; 8] = core::array::from_fn(|i| STATS[i].load(Ordering::Relaxed));
    assert_eq!(read, [1, 2, 3, 4, 5, 6, 7, 8]);
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
fn the_browse_knob_and_load_follow_the_sorted_filtered_view() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    publish_library(std::vec![LibraryRow::default(); 4]);
    // The page shows tracks 3 and 1, in that order.
    publish_view(Level::All, Some(&[3, 1]));
    assert_eq!(level_len(Level::All), 2);
    assert_eq!(base_len(Level::All), 4);
    assert_eq!(row_at(Level::All, 0), Some(Row::Track(3)));
    assert_eq!(row_at(Level::All, 1), Some(Row::Track(1)));
    assert_eq!(row_at(Level::All, 2), None);
    select(0);
    browse(5);
    assert_eq!(selected(), Some(1), "the knob stops at the view's last row");
    assert_eq!(
        selected_row(),
        Some(Row::Track(1)),
        "LOAD takes the row shown"
    );
    // A view of another level leaves this one as it is.
    publish_view(Level::Top, Some(&[0]));
    assert_eq!(level_len(Level::All), 4);
    assert_eq!(row_at(Level::All, 3), Some(Row::Track(3)));
    publish_view(Level::All, None);
    assert_eq!(level_len(Level::All), 4);
    publish_library(std::vec::Vec::new());
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

/// A catalog of three tracks (source ids 10, 20 and 30) and its playlist tree:
/// `Warmup` (a playlist, sorted second) and `Clubs` (a folder) at the top, `Peak` (a
/// playlist) in `Clubs`.
fn shelf_fixture() -> Shelf {
    use ruzina_core::MediaTrackId;
    use ruzina_media_catalog::playlists::{InlinePlaylistStore, PlaylistBuilder};
    use ruzina_media_catalog::{CatalogBuilder, CatalogText, CatalogTrack};
    use ruzina_media_session::{MediaGeneration, MediaLease, MediaSourceId};
    let lease = MediaLease {
        generation: MediaGeneration::initial(),
        source: MediaSourceId::new(1).unwrap(),
    };
    let mut catalog = CatalogBuilder::<4>::new(lease);
    for (id, source) in [(1, 10), (2, 20), (3, 30)] {
        catalog
            .push(CatalogTrack {
                track_id: MediaTrackId([id; 16]),
                source_track_id: source,
                title: CatalogText::try_from_str("t").unwrap(),
                ..CatalogTrack::EMPTY
            })
            .unwrap();
    }
    let catalog = catalog.finish(1);
    let mut tree = PlaylistBuilder::new(InlinePlaylistStore::<8, 8>::new());
    assert!(tree.push_node(5, 0, 2, false, "Warmup"));
    assert!(tree.push_node(7, 0, 1, true, "Clubs"));
    assert!(tree.push_node(9, 7, 0, false, "Peak"));
    for (list, index, track) in [(5, 2, 30), (5, 1, 10), (9, 0, 20), (9, 1, 20)] {
        assert!(tree.push_entry(list, index, track));
    }
    Shelf::from_index(&tree.finish(&catalog))
}

#[test]
fn the_shelf_keeps_rekordboxs_tree_and_order() {
    let shelf = shelf_fixture();
    let name = |node: u32| shelf.nodes[node as usize].name.as_str();
    let top: Vec<_> = shelf.top.iter().map(|&node| name(node)).collect();
    assert_eq!(top, ["Clubs", "Warmup"]);
    let clubs = &shelf.nodes[shelf.top[0] as usize];
    assert!(clubs.folder && clubs.tracks.is_empty() && clubs.parent.is_none());
    let peak = &shelf.nodes[clubs.children[0] as usize];
    assert_eq!(peak.name, "Peak");
    assert_eq!(peak.parent, Some(shelf.top[0]));
    assert_eq!(
        peak.tracks,
        [1, 1],
        "a track twice in a playlist stays twice"
    );
    let warmup = &shelf.nodes[shelf.top[1] as usize];
    assert_eq!(warmup.tracks, [0, 2], "in the playlist's order");
}

#[test]
fn back_returns_to_the_row_it_came_from_however_deep_the_tree() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    // A chain of 20 folders, deeper than the old 16-level history. Each one sits among two
    // empty playlists, folder d's child at row d % 3, so every level's row differs.
    const LEVELS: usize = 20;
    let mut shelf = Shelf::default();
    let node = |shelf: &mut Shelf, folder: bool, parent: Option<u32>| {
        shelf.nodes.push(ShelfNode {
            name: std::format!("n{}", shelf.nodes.len()),
            folder,
            parent,
            children: std::vec::Vec::new(),
            tracks: std::vec::Vec::new(),
        });
        shelf.nodes.len() as u32 - 1
    };
    let mut chain = std::vec![node(&mut shelf, true, None)];
    let pads = [node(&mut shelf, false, None), node(&mut shelf, false, None)];
    shelf.top = std::vec![pads[0], chain[0], pads[1]];
    for d in 0..LEVELS - 1 {
        let parent = Some(chain[d]);
        let child = node(&mut shelf, true, parent);
        let mut rows = std::vec![
            node(&mut shelf, false, parent),
            node(&mut shelf, false, parent)
        ];
        rows.insert(d % 3, child);
        shelf.nodes[chain[d] as usize].children = rows;
        chain.push(child);
    }
    publish_library(std::vec![LibraryRow::default(); 1]);
    publish_shelf(shelf);
    assert!(back());
    assert_eq!(level(), Level::Top);
    select(1);
    assert!(open());
    for d in 0..LEVELS - 1 {
        select(d as u32 % 3);
        assert!(open());
    }
    assert_eq!(level(), Level::Node(chain[LEVELS - 1]));
    assert_eq!(path_names().len(), LEVELS, "the whole path, however deep");
    for d in (0..LEVELS - 1).rev() {
        assert!(back());
        assert_eq!(level(), Level::Node(chain[d]));
        assert_eq!(selected(), Some(d as u32 % 3), "back at level {d}");
    }
    assert!(back());
    assert_eq!((level(), selected()), (Level::Top, Some(1)));
    clear_shelf();
    publish_library(std::vec::Vec::new());
    take_library();
}

#[test]
fn the_library_walks_the_playlists_and_comes_back_where_it_was() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    publish_library(std::vec![LibraryRow::default(); 3]);
    clear_shelf();
    select(2);
    assert!(!back(), "no playlists: the track list stays");
    publish_shelf(shelf_fixture());
    assert_eq!(level(), Level::All);
    select(2);
    assert!(!open(), "a track does not open");
    assert_eq!(selected_row(), Some(Row::Track(2)));

    assert!(back());
    assert_eq!(level(), Level::Top);
    assert_eq!(level_len(Level::Top), 2);
    assert_eq!(selected(), Some(0));
    let clubs = selected_row();
    assert!(open());
    assert_eq!(path_names(), ["Clubs"]);
    assert!(open());
    assert_eq!(path_names(), ["Clubs", "Peak"]);
    assert_eq!(selected_row(), Some(Row::Track(1)));
    assert_eq!(level_len(level()), 2);

    assert!(back());
    assert_eq!(path_names(), ["Clubs"]);
    assert!(back());
    assert_eq!((level(), selected_row()), (Level::Top, clubs));
    select(1);
    assert!(open());
    assert_eq!(path_names(), ["Warmup"]);
    select(1);
    assert_eq!(selected_row(), Some(Row::Track(2)));
    assert!(back());
    assert_eq!(selected(), Some(1), "back on Warmup");
    assert!(back());
    assert_eq!(level(), Level::All);
    assert_eq!(selected(), Some(2), "the track list's selection");

    clear_shelf();
    publish_library(std::vec::Vec::new());
    take_library();
}

#[test]
fn a_playing_deck_is_drawn_where_it_will_be_when_the_frame_shows() {
    let playing = DeckSnapshot {
        position_ms: 10_000,
        flags: LOADED | PLAYING,
        pitch_centipercent: 0,
        bpm_x100: 12_800,
    };
    assert_eq!(ahead(playing, 25_000).position_ms, 10_025);
    // At +8 % pitch the deck moves 8 % faster.
    let pitched = DeckSnapshot {
        pitch_centipercent: 800,
        ..playing
    };
    assert_eq!(ahead(pitched, 25_000).position_ms, 10_027);
    let slowed = DeckSnapshot {
        pitch_centipercent: -800,
        ..playing
    };
    assert_eq!(ahead(slowed, 25_000).position_ms, 10_023);
    // Paused, looping, or a frame too far off: the position as read.
    let paused = DeckSnapshot {
        flags: LOADED,
        ..playing
    };
    assert_eq!(ahead(paused, 25_000), paused);
    let looping = DeckSnapshot {
        flags: LOADED | PLAYING | LOOPING,
        ..playing
    };
    assert_eq!(ahead(looping, 25_000), looping);
    assert_eq!(ahead(playing, 40_001), playing);
    // A time before the snapshot wraps to a large count: also as read.
    assert_eq!(ahead(playing, 0u32.wrapping_sub(5_000)), playing);
}
