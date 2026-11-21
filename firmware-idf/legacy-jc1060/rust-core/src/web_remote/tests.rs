use super::*;

fn handle(request: &str) -> std::string::String {
    let mut out = std::vec![0u8; ruzina_web::assets::LARGEST_BYTES + 1024];
    // SAFETY: both buffers are valid for their lengths.
    let len = unsafe {
        ruzina_web_handle(
            7,
            request.as_ptr(),
            request.len(),
            out.as_mut_ptr(),
            out.len(),
        )
    };
    std::string::String::from_utf8_lossy(&out[..len]).into_owned()
}

fn drain_web() -> std::vec::Vec<(usize, TouchControl)> {
    core::iter::from_fn(|| deck_view::WEB.pop()).collect()
}

#[test]
fn play_and_cue_take_the_touch_path_and_other_controls_go_as_events() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    drain_web();
    let post = |query: &str| {
        handle(&std::format!(
            "POST /api/control?{query} HTTP/1.1\r\nHost: 192.168.4.1\r\nOrigin: http://192.168.4.1\r\nX-DDJ-Control: 1\r\nContent-Length: 0\r\n\r\n"
        ))
    };
    let play = post("action=play_pause&deck=2");
    assert!(play.starts_with("HTTP/1.1 200"), "{play}");
    assert_eq!(drain_web(), [(1, TouchControl::Play)], "the press only");
    let cue = post("action=cue&deck=1");
    assert!(cue.starts_with("HTTP/1.1 200"), "{cue}");
    assert_eq!(
        drain_web(),
        [(0, TouchControl::Cue(true)), (0, TouchControl::Cue(false))]
    );
    while deck_view::WEB_EVENTS.pop().is_some() {}
    let volume = post("action=volume&deck=1&value=4000");
    assert!(volume.starts_with("HTTP/1.1 200"), "{volume}");
    assert_eq!(drain_web(), [], "not the touch path");
    let event = deck_view::WEB_EVENTS.pop().expect("the volume event");
    assert_eq!(event.deck, Some(DeckId::One));
    assert_eq!(event.control, SemanticControl::ChannelVolume);
    assert!(deck_view::WEB_EVENTS.pop().is_none());
}

fn post(query: &str) -> std::string::String {
    handle(&std::format!(
        "POST /api/control?{query} HTTP/1.1\r\nHost: 192.168.4.1\r\nOrigin: http://192.168.4.1\r\nX-DDJ-Control: 1\r\nContent-Length: 0\r\n\r\n"
    ))
}

#[test]
fn the_mixer_and_sync_reach_the_mix_and_come_back_in_the_status() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    drain_web();
    while deck_view::WEB_EVENTS.pop().is_some() {}
    let mut mix = crate::deck_mix::Mix::new();
    for query in [
        "action=volume&deck=2&value=4000",
        "action=crossfader&value=12000",
        "action=eq_low&deck=1&value=1000",
        "action=filter&deck=2&value=3000",
        "action=pfl&deck=1",
    ] {
        let answer = post(query);
        assert!(answer.starts_with("HTTP/1.1 200"), "{query}: {answer}");
    }
    service(&mut mix);
    let view = MixView::load().expect("published");
    assert_eq!(view.volume[1], 4000);
    assert_eq!(view.crossfader, 12000);
    assert_eq!(view.eq[0][0], 1000);
    assert_eq!(view.filter[1], 3000);
    assert!(view.pfl[0]);
    let status = get("/api/status");
    assert!(status.contains("\"crossfader\":12000"), "{status}");
}

#[test]
fn the_controllers_fader_takes_over_only_once_it_reaches_the_pages_value() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    drain_web();
    while deck_view::WEB_EVENTS.pop().is_some() {}
    let mut mix = crate::deck_mix::Mix::new();
    let physical = |mix: &mut crate::deck_mix::Mix, value: u16| {
        mix.controller_control(&ControlEvent {
            deck: Some(DeckId::Two),
            control: SemanticControl::ChannelVolume,
            value: ControlValue::Absolute { value, max: 16_383 },
        });
        mix.mixer().deck(DeckId::Two).channel_volume
    };
    assert_eq!(
        physical(&mut mix, 12_000),
        12_000,
        "nothing held: the fader counts"
    );
    let answer = post("action=volume&deck=2&value=4000");
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    service(&mut mix);
    assert_eq!(mix.mixer().deck(DeckId::Two).channel_volume, 4000);
    // The fader still sits high: its moves there do not count.
    assert_eq!(physical(&mut mix, 11_800), 4000);
    assert_eq!(physical(&mut mix, 9_000), 4000);
    // Within 2 % of the page's value it takes over.
    assert_eq!(physical(&mut mix, 4_200), 4_200);
    assert_eq!(physical(&mut mix, 6_000), 6_000, "and stays in charge");
    // Passing the page's value in one jump also takes over.
    let answer = post("action=volume&deck=2&value=8000");
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    service(&mut mix);
    assert_eq!(physical(&mut mix, 2_000), 8_000);
    assert_eq!(physical(&mut mix, 12_000), 12_000, "jumped past 8000");
    // A fader that sat below the page's value and moves past it in its first move after
    // the page set it takes over: where it sat is kept across the page's update.
    assert_eq!(physical(&mut mix, 6_000), 6_000);
    let answer = post("action=volume&deck=2&value=8000");
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    service(&mut mix);
    assert_eq!(
        physical(&mut mix, 12_000),
        12_000,
        "6000 to 12000 passes 8000"
    );
    // Another channel's fader and other controls are never held.
    let other = ControlEvent {
        deck: Some(DeckId::One),
        control: SemanticControl::ChannelVolume,
        value: ControlValue::Absolute {
            value: 1_000,
            max: 16_383,
        },
    };
    mix.controller_control(&other);
    assert_eq!(mix.mixer().deck(DeckId::One).channel_volume, 1_000);
}

/// What a fake USB task took while the web task waited: a load (`'l'`) or a seek
/// (`'s'`), the deck and the arguments.
type Taken = std::rc::Rc<core::cell::RefCell<std::vec::Vec<(char, usize, [u32; 3])>>>;

/// While the web task waits, a USB task that takes every load and seek and gives the
/// answer `answer` holds then.
fn usb_task(answer: std::rc::Rc<core::cell::Cell<u8>>) -> Taken {
    let taken = Taken::default();
    let seen = taken.clone();
    USB_TURN.with(|turn| {
        *turn.borrow_mut() = Some(Box::new(move || {
            for (kind, exchanges) in [('l', &WEB_LOAD), ('s', &WEB_SEEK)] {
                for (deck, exchange) in exchanges.iter().enumerate() {
                    if let Some((ticket, args)) = exchange.take() {
                        seen.borrow_mut().push((kind, deck, args));
                        exchange.answer(ticket, answer.get());
                    }
                }
            }
        }));
    });
    taken
}

/// No USB task: nothing takes the web task's requests.
fn no_usb_task() {
    USB_TURN.with(|turn| *turn.borrow_mut() = None);
}

#[test]
fn a_seek_needs_a_loaded_track_and_answers_as_the_engine_did() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let answer = std::rc::Rc::new(core::cell::Cell::new(DONE));
    let taken = usb_task(answer.clone());
    deck_view::LIVE[0].store(deck_view::DeckSnapshot::default());
    let refused = post("action=seek&deck=1&value=5000");
    assert!(refused.starts_with("HTTP/1.1 400"), "no track: {refused}");
    assert!(taken.borrow().is_empty(), "never asked");
    let mut track = deck_view::LoadedTrack::none();
    track.index = 0;
    track.duration_ms = 60_000;
    deck_view::publish_track(0, Box::new(track));
    deck_view::LIVE[0].store(deck_view::DeckSnapshot {
        flags: deck_view::LOADED,
        ..deck_view::DeckSnapshot::default()
    });
    let seek = post("action=seek&deck=1&value=90000");
    assert!(seek.starts_with("HTTP/1.1 200"), "{seek}");
    let with_generation = post("action=seek&deck=1&value=100&generation=5");
    assert!(
        with_generation.starts_with("HTTP/1.1 200"),
        "{with_generation}"
    );
    assert_eq!(
        *taken.borrow(),
        [('s', 0, [59_999, 0, 0]), ('s', 0, [100, 1, 5])],
        "the last ms; the generation the page sent"
    );
    for (code, line, message) in [
        (NO_TRACK, "HTTP/1.1 400", "Deck not seekable"),
        (HELD, "HTTP/1.1 409", "Deck is scratching or looping"),
        (CHANGED, "HTTP/1.1 409", "Deck track changed"),
    ] {
        answer.set(code);
        let refused = post("action=seek&deck=1&value=100");
        assert!(refused.starts_with(line), "{refused}");
        assert!(refused.ends_with(message), "{refused}");
    }
    // Without a USB task the seek is withdrawn, so it never runs later.
    no_usb_task();
    let busy = post("action=seek&deck=1&value=100");
    assert!(busy.starts_with("HTTP/1.1 503"), "{busy}");
    assert!(WEB_SEEK[0].take().is_none(), "withdrawn");
    deck_view::LIVE[0].store(deck_view::DeckSnapshot::default());
    while deck_view::take_track(0).is_some() {}
    while deck_view::take_grid(0).is_some() {}
}

#[test]
fn the_status_names_the_deck() {
    let name = "Ruzina-AB2D";
    // SAFETY: a valid string for its length.
    unsafe { ruzina_web_set_name(name.as_ptr(), name.len()) };
    let status = get("/api/status");
    assert!(status.contains("\"name\":\"Ruzina-AB2D\""), "{status}");
}

#[test]
fn the_page_and_the_status_answer_for_the_access_point_only() {
    let page = handle("GET / HTTP/1.1\r\nHost: 192.168.4.1\r\n\r\n");
    assert!(
        page.starts_with("HTTP/1.1 200"),
        "{}",
        &page[..page.len().min(80)]
    );
    let status = handle("GET /api/status HTTP/1.1\r\nHost: 192.168.4.1\r\n\r\n");
    assert!(
        status.starts_with("HTTP/1.1 200"),
        "{}",
        &status[..status.len().min(80)]
    );
    assert!(
        status.contains("\"deck1\"") && status.contains("\"deck2\""),
        "{status}"
    );
    let foreign = handle("GET /api/status HTTP/1.1\r\nHost: example.com\r\n\r\n");
    assert!(!foreign.starts_with("HTTP/1.1 200"), "{foreign}");
}

#[test]
fn a_cue_tap_is_queued_whole_or_not_at_all() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    drain_web();
    // One free slot: the press would fit, its release would not.
    while deck_view::WEB.free() > 1 {
        assert!(deck_view::WEB.push(0, TouchControl::Play));
    }
    let cue = handle(
        "POST /api/control?action=cue&deck=1 HTTP/1.1\r\nHost: 192.168.4.1\r\nOrigin: http://192.168.4.1\r\nX-DDJ-Control: 1\r\nContent-Length: 0\r\n\r\n",
    );
    assert!(cue.starts_with("HTTP/1.1 503"), "{cue}");
    let queued = drain_web();
    assert!(
        queued
            .iter()
            .all(|&(_, control)| control == TouchControl::Play),
        "no CUE edge: {queued:?}"
    );
}

fn get(path: &str) -> std::string::String {
    handle(&std::format!(
        "GET {path} HTTP/1.1\r\nHost: 192.168.4.1\r\n\r\n"
    ))
}

fn load(track: u32, generation: u32, deck: u32) -> std::string::String {
    handle(&std::format!(
        "POST /api/load?track_key={track}&generation={generation}&deck={deck} HTTP/1.1\r\nHost: 192.168.4.1\r\nOrigin: http://192.168.4.1\r\nX-DDJ-Control: 1\r\nContent-Length: 0\r\n\r\n"
    ))
}

/// The library revision the status reports.
fn revision() -> u32 {
    let status = get("/api/status");
    let at = status
        .find("\"library_revision\":")
        .expect("revision field")
        + 19;
    status[at..]
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|digits| digits.parse().ok())
        .expect("a revision")
}

#[test]
fn the_page_lists_the_library_and_loads_a_track_without_the_screens_selection() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let row = |title: &str, artist: &str| deck_view::LibraryRow {
        title: title.into(),
        artist: artist.into(),
        bpm_x100: 12_450,
        duration_ms: 241_000,
        ..deck_view::LibraryRow::default()
    };
    deck_view::publish_library(std::vec![
        row("Alpha", "A"),
        row("Beta", "B"),
        row("", "Only")
    ]);
    deck_view::select(0);
    for deck in &deck_view::LIVE {
        deck.store(deck_view::DeckSnapshot::default());
    }
    let answer = std::rc::Rc::new(core::cell::Cell::new(DONE));
    let taken = usb_task(answer.clone());
    let library = get("/api/library");
    assert!(library.starts_with("HTTP/1.1 200"), "{library}");
    for expected in [
        "\"Alpha\"",
        "\"Beta\"",
        "\"Only\"",
        "\"track_key\":2",
        "\"bpm\":125",
    ] {
        assert!(library.contains(expected), "{expected} in {library}");
    }
    let generation = revision();
    let loaded = load(2, generation, 2);
    assert!(loaded.starts_with("HTTP/1.1 200"), "{loaded}");
    assert_eq!(
        *taken.borrow(),
        [('l', 1, [2, generation, 0])],
        "catalog index 1, from the listed revision"
    );
    assert_eq!(
        deck_view::selected(),
        Some(0),
        "the screen's selection stays"
    );
    // The engine's refusals reach the page as they are.
    for (code, message) in [
        (BUSY, "Library changed or load busy"),
        (LOCKED, "Deck is playing (load lock)"),
        (CHANGED, "Library generation changed"),
    ] {
        answer.set(code);
        let refused = load(3, generation, 2);
        assert!(refused.starts_with("HTTP/1.1 409"), "{refused}");
        assert!(refused.ends_with(message), "{refused}");
    }
    taken.borrow_mut().clear();
    let stale = load(2, generation.wrapping_add(1), 2);
    assert!(stale.starts_with("HTTP/1.1 409"), "{stale}");
    let missing = load(4, generation, 2);
    assert!(missing.starts_with("HTTP/1.1 404"), "{missing}");
    assert!(taken.borrow().is_empty(), "neither reached the engine");
    // Without a USB task the load is withdrawn, so it never runs later.
    no_usb_task();
    let busy = load(1, generation, 1);
    assert!(busy.starts_with("HTTP/1.1 409"), "{busy}");
    assert!(WEB_LOAD[0].take().is_none(), "withdrawn");
}

#[test]
fn the_status_shows_the_loaded_tracks_title_and_duration() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let mut track = deck_view::LoadedTrack::none();
    track.index = 0;
    track.title = "Gamma".into();
    track.artist = "Artist".into();
    track.duration_ms = 200_000;
    deck_view::publish_track(0, Box::new(track));
    deck_view::LIVE[0].store(deck_view::DeckSnapshot {
        flags: deck_view::LOADED,
        position_ms: 1_000,
        ..deck_view::DeckSnapshot::default()
    });
    let status = get("/api/status");
    assert!(status.contains("\"title\":\"Gamma\""), "{status}");
    assert!(status.contains("\"duration_ms\":200000"), "{status}");
    // Unloaded, the deck shows nothing again.
    deck_view::LIVE[0].store(deck_view::DeckSnapshot::default());
    let status = get("/api/status");
    assert!(!status.contains("Gamma"), "{status}");
    while deck_view::take_track(0).is_some() {}
    while deck_view::take_grid(0).is_some() {}
}
