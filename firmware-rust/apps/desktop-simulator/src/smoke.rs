use super::*;
use slint::platform::{
    Platform, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::{fs, io::Write, path::Path};

struct Headless(Rc<MinimalSoftwareWindow>);
impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

/// Headless software-renderer evidence from the actual compiled Slint component.
/// Callback replay covers bindings; pointer hit testing is exercised separately.
pub fn run(output: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(output)?;
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless(window.clone())))?;
    let ui = MainWindow::new()?;
    let state = Rc::new(RefCell::new(Session::new()));
    bind(&ui, state.clone());
    refresh(&ui, &state.borrow());
    ui.show()?;
    for (name, width, height) in [("jc1060", 1024, 600), ("jc8012", 800, 1280)] {
        *state.borrow_mut() = Session::for_board(if name == "jc1060" {
            BoardId::GuitionJc1060P470cIwY
        } else {
            BoardId::GuitionJc8012P4a1cIwY
        });
        refresh(&ui, &state.borrow());
        window.set_size(slint::PhysicalSize::new(width, height));
        ui.set_board_label(format!("{name} · HOST SIMULATOR").into());
        ui.set_page(0);
        capture(&ui, output, &format!("{name}-empty"))?;
        ui.set_health_events(false);
        ui.set_page(6);
        health_smoke::capture_health(&ui, &state, output, &format!("{name}-health-initial"))?;
        health_smoke::freshness(&ui, &state, output, name)?;
        // Keep all existing scenario clocks/counters independent of this replay.
        let board = state.borrow().diagnostics().identity.board;
        *state.borrow_mut() = Session::for_board(board);
        refresh(&ui, &state.borrow());
        ui.set_page(0);
        ui.invoke_select_track(0);
        ui.invoke_load_track(0);
        ui.invoke_select_track(1);
        ui.invoke_load_track(1);
        ui.invoke_deck_action(0, 0);
        mix_fx_smoke::scenes(&ui, &state, output, name)?;
        state.borrow_mut().tick(1770);
        refresh(&ui, &state.borrow());
        assert!(state.borrow().presentation(0).transport.playing);
        ui.invoke_deck_action(1, 2);
        capture(&ui, output, &format!("{name}-decks"))?;
        ui.invoke_deck_action(0, 0);
        ui.set_page(2);
        ui.invoke_cue_action(0, 2, false);
        assert!(state.borrow().presentation(0).cues[2].is_some());
        capture(&ui, output, &format!("{name}-cues"))?;
        // M2.5 MEMORY panel: deck 1's imported list replaces its pads.
        ui.set_memory_deck(0);
        capture(&ui, output, &format!("{name}-memory"))?;
        ui.set_memory_deck(-1);
        ui.invoke_cue_action(0, 2, true);
        assert!(state.borrow().presentation(0).cues[2].is_none());
        ui.set_page(3);
        ui.set_remaining(true);
        ui.invoke_display_changed();
        ui.invoke_deck_action(0, 8);
        capture(&ui, output, &format!("{name}-settings"))?;
        ui.set_page(1);
        ui.invoke_select_track(3);
        ui.invoke_load_track(0);
        assert!(state.borrow().analysis(0).is_none());
        capture(&ui, output, &format!("{name}-library-fallback"))?;
        ui.set_page(0);
        capture(&ui, output, &format!("{name}-deck-fallback"))?;
        ui.set_page(1);
        ui.invoke_media_action();
        assert!(!state.borrow().presentation(0).transport.loaded);
        capture(&ui, output, &format!("{name}-removed"))?;
        ui.set_page(5);
        capture(&ui, output, &format!("{name}-fx-removed"))?;
        ui.set_page(1);
        ui.invoke_media_action();
        ui.set_remaining(false);
        pointer_smoke(&ui, &state, width as f32);
        mix_fx_smoke::pointer(&ui, &state, width as f32, height as f32);
        health_smoke::recovery(&ui, &state, output, name, width as f32)?;
        analysis_smoke::run(&ui, &state, output, name, width as f32)?;
    }
    // Last, so the overlay's keyboard focus cannot alter earlier scenes.
    for (name, width, height) in [("jc1060", 1024, 600), ("jc8012", 800, 1280)] {
        window.set_size(slint::PhysicalSize::new(width, height));
        ui.set_board_label(format!("{name} · HOST SIMULATOR").into());
        *state.borrow_mut() = Session::for_board(if name == "jc1060" {
            BoardId::GuitionJc1060P470cIwY
        } else {
            BoardId::GuitionJc8012P4a1cIwY
        });
        screensaver(&ui, &state, output, name, width as f32, height as f32)?;
        library_playlists(&ui, &state, output, name)?;
        hold_restore(&ui, &state, name);
        load_lock(&ui, &state, output, name)?;
    }
    println!(
        "Slint deck/mixer/FX/health callback + pointer/keyboard smoke in both geometries; 64 software-rendered snapshots: PASS"
    );
    Ok(())
}

/// Idle screensaver from a fresh session: it appears at the M2.5 timeout,
/// stays away while a deck plays, and the touch or key that dismisses it
/// reaches no deck.
fn screensaver(
    ui: &MainWindow,
    state: &Rc<RefCell<Session>>,
    output: &Path,
    name: &str,
    width: f32,
    height: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    use ruzina_ui_model::idle::IDLE_DEFAULT_TIMEOUT_MS;
    use slint::platform::WindowEvent;
    let board = state.borrow().diagnostics().identity.board;
    *state.borrow_mut() = Session::for_board(board);
    ui.set_page(0);
    refresh(ui, &state.borrow());
    state.borrow_mut().tick(IDLE_DEFAULT_TIMEOUT_MS - 1);
    refresh(ui, &state.borrow());
    assert!(!ui.get_screensaver());
    state.borrow_mut().tick(1);
    refresh(ui, &state.borrow());
    assert!(ui.get_screensaver());
    capture(ui, output, &format!("{name}-screensaver"))?;
    // The touch lands where DECK 1 PLAY is, but only wakes the screen.
    click(ui, 70.0, 356.0);
    assert!(!ui.get_screensaver());
    assert!(!state.borrow().presentation(0).transport.playing);
    assert_eq!(ui.get_page(), 0);
    state.borrow_mut().tick(IDLE_DEFAULT_TIMEOUT_MS);
    refresh(ui, &state.borrow());
    ui.window().take_snapshot()?;
    assert!(ui.get_screensaver());
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: " ".into() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text: " ".into() });
    assert!(!ui.get_screensaver(), "a key wakes it too");
    assert!(!state.borrow().presentation(0).transport.playing);
    // Due in one second, but playback holds it off and its stop restarts the
    // countdown instead of blanking at once.
    ui.invoke_select_track(0);
    ui.invoke_load_track(0);
    state.borrow_mut().tick(IDLE_DEFAULT_TIMEOUT_MS - 1_000);
    ui.invoke_deck_action(0, 0);
    for _ in 0..50 {
        state.borrow_mut().tick(100);
    }
    refresh(ui, &state.borrow());
    assert!(state.borrow().presentation(0).transport.playing);
    assert!(!ui.get_screensaver());
    ui.invoke_deck_action(0, 0);
    state.borrow_mut().tick(IDLE_DEFAULT_TIMEOUT_MS - 1);
    refresh(ui, &state.borrow());
    assert!(!ui.get_screensaver());
    state.borrow_mut().tick(1);
    refresh(ui, &state.borrow());
    assert!(ui.get_screensaver());
    state.borrow_mut().wake();
    let _ = (width, height);
    println!("Idle screensaver show/wake/no-deck-input/playback inhibit: PASS at {name}");
    Ok(())
}

/// RP-06 playlist hierarchy through the product `LibraryBrowser`: PLAYLISTS
/// opens the tree, LOAD opens folders and playlists without loading, LOAD on
/// a playlist track loads it, and BACK walks up to the track list.
fn library_playlists(
    ui: &MainWindow,
    state: &Rc<RefCell<Session>>,
    output: &Path,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let board = state.borrow().diagnostics().identity.board;
    *state.borrow_mut() = Session::for_board(board);
    ui.set_page(1);
    refresh(ui, &state.borrow());
    assert!(ui.get_library_top());
    assert_eq!(ui.get_library_path(), "DEMO LIBRARY");
    ui.invoke_library_back();
    assert!(!ui.get_library_top());
    assert_eq!(ui.get_library_path(), "PLAYLISTS");
    let labels = |ui: &MainWindow| -> Vec<String> {
        ui.get_library_rows()
            .iter()
            .map(|row| row.label.to_string())
            .collect()
    };
    assert_eq!(
        labels(ui),
        ["FOLDER · Sets", "PLAYLIST · Recovery checks (2)"]
    );
    capture(ui, output, &format!("{name}-library-playlists"))?;
    ui.invoke_select_track(0);
    ui.invoke_load_track(0);
    assert_eq!(ui.get_library_path(), "PLAYLISTS / Sets");
    assert!(
        !state.borrow().presentation(0).transport.loaded,
        "opening loads nothing"
    );
    assert_eq!(
        labels(ui),
        ["PLAYLIST · Warm-up (2)", "PLAYLIST · Peak time (2)"]
    );
    ui.invoke_select_track(0);
    ui.invoke_load_track(1);
    assert_eq!(ui.get_library_path(), "PLAYLISTS / Sets / Warm-up");
    assert_eq!(labels(ui), [session::TITLES[1], session::TITLES[0]]);
    capture(ui, output, &format!("{name}-library-playlist"))?;
    ui.invoke_select_track(0);
    ui.invoke_load_track(1);
    assert_eq!(
        state.borrow().title(1),
        session::TITLES[1],
        "playlist order, not track order"
    );
    for path in ["PLAYLISTS / Sets", "PLAYLISTS", "DEMO LIBRARY"] {
        ui.invoke_library_back();
        assert_eq!(ui.get_library_path(), path);
    }
    assert!(ui.get_library_top());
    assert_eq!(labels(ui).len(), session::TITLES.len());
    println!("Library playlist tree open/load/back: PASS at {name}");
    Ok(())
}

/// M2.5 HOLD RESTORE by pointer, under deck 1's delete row: a short press
/// keeps a local delete, a 400 ms hold brings the Rekordbox cue back, and a
/// page change while held restores nothing. A fresh session keeps the other
/// scenes' clocks.
fn hold_restore(ui: &MainWindow, state: &Rc<RefCell<Session>>, name: &str) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let board = state.borrow().diagnostics().identity.board;
    *state.borrow_mut() = Session::for_board(board);
    ui.set_page(1);
    ui.invoke_select_track(0);
    ui.invoke_load_track(0);
    ui.set_page(2);
    ui.invoke_cue_action(0, 0, true);
    assert!(state.borrow().presentation(0).cues[0].is_none());
    let position = slint::LogicalPosition::new(100.0, 434.0);
    let button = PointerEventButton::Left;
    for (held_ms, leave_page, restored) in
        [(200, false, false), (400, true, false), (400, false, true)]
    {
        ui.set_page(2);
        ui.window().take_snapshot().unwrap();
        ui.window()
            .dispatch_event(WindowEvent::PointerPressed { position, button });
        if leave_page {
            ui.set_page(0);
            ui.window().take_snapshot().unwrap();
        }
        state.borrow_mut().tick(held_ms);
        refresh(ui, &state.borrow());
        ui.window()
            .dispatch_event(WindowEvent::PointerReleased { position, button });
        state.borrow_mut().tick(400);
        assert_eq!(state.borrow().presentation(0).cues[0].is_some(), restored);
    }
    println!("HOLD RESTORE short press/page change/long press: PASS at {name}");
}

/// M2.5 LOAD LOCK: while deck 1 plays its LOAD key reads "D1 LOCKED" at
/// 40 %, a tap is refused by the owner, and the lock lifts on folder rows.
fn load_lock(
    ui: &MainWindow,
    state: &Rc<RefCell<Session>>,
    output: &Path,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let board = state.borrow().diagnostics().identity.board;
    *state.borrow_mut() = Session::for_board(board);
    ui.set_page(1);
    ui.invoke_select_track(0);
    ui.invoke_load_track(0);
    ui.invoke_deck_action(0, 0);
    ui.invoke_select_track(1);
    let keys = |ui: &MainWindow| -> Vec<String> {
        ui.get_load_keys()
            .iter()
            .map(|key| key.to_string())
            .collect()
    };
    assert_eq!(keys(ui), ["D1 LOCKED", "LOAD DECK 2"]);
    capture(ui, output, &format!("{name}-library-locked"))?;
    ui.invoke_load_track(0);
    assert_eq!(state.borrow().title(0), session::TITLES[0], "refused");
    // The Ruzina switch: off unlocks the key and replaces the playing deck.
    ui.invoke_load_lock_toggle();
    assert!(!ui.get_load_lock_on());
    assert_eq!(ui.get_load_lock_text(), "LOAD LOCK: OFF");
    // Read back from the settings record, as after a restart.
    state.borrow_mut().reload_settings();
    refresh(ui, &state.borrow());
    assert!(!ui.get_load_lock_on());
    assert_eq!(keys(ui), ["LOAD DECK 1", "LOAD DECK 2"]);
    ui.invoke_load_track(0);
    assert_eq!(state.borrow().title(0), session::TITLES[1], "replaced");
    ui.invoke_load_lock_toggle();
    assert!(ui.get_load_lock_on());
    ui.invoke_deck_action(0, 0);
    assert_eq!(keys(ui), ["D1 LOCKED", "LOAD DECK 2"]);
    ui.invoke_library_back();
    assert_eq!(keys(ui), ["LOAD DECK 1", "LOAD DECK 2"]);
    ui.invoke_library_back();
    ui.invoke_deck_action(0, 0);
    assert_eq!(keys(ui), ["LOAD DECK 1", "LOAD DECK 2"]);
    println!("LOAD LOCK key/refusal/folder rows: PASS at {name}");
    Ok(())
}

pub(super) fn click(ui: &MainWindow, x: f32, y: f32) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let position = slint::LogicalPosition::new(x, y);
    // Headless replay has no native event loop to lay out newly created pages.
    ui.window().take_snapshot().unwrap();
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    slint::platform::update_timers_and_animations();
}

fn pointer_smoke(ui: &MainWindow, state: &Rc<RefCell<Session>>, width: f32) {
    use slint::platform::{PointerEventButton, WindowEvent};
    // Actual hit testing, rather than invoking generated callbacks directly.
    ui.set_page(0);
    nav(ui, width, 1);
    assert_eq!(ui.get_page(), 1);
    assert!(ui.get_media_ready());
    click(ui, width / 2.0, 238.0);
    assert_eq!(state.borrow().selected, 0);
    click(ui, width / 6.0, 478.0);
    assert!(state.borrow().presentation(0).transport.loaded);
    nav(ui, width, 0);
    assert_eq!(ui.get_page(), 0);
    click(ui, 70.0, 356.0);
    assert!(state.borrow().presentation(0).transport.playing);
    // A timer/model refresh during a held click must not destroy the button.
    let position = slint::LogicalPosition::new(70.0, 356.0);
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    state.borrow_mut().tick(50);
    refresh(ui, &state.borrow());
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    assert!(!state.borrow().presentation(0).transport.playing);
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: " ".into() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text: " ".into() });
    assert!(state.borrow().presentation(0).transport.playing);
    click(ui, 70.0, 356.0);
    assert!(!state.borrow().presentation(0).transport.playing);
    for playing in [true, false] {
        ui.window().dispatch_event(WindowEvent::KeyPressed {
            text: slint::platform::Key::Return.into(),
        });
        ui.window().dispatch_event(WindowEvent::KeyReleased {
            text: slint::platform::Key::Return.into(),
        });
        assert_eq!(state.borrow().presentation(0).transport.playing, playing);
    }
    ui.window().dispatch_event(WindowEvent::KeyPressed {
        text: slint::platform::Key::Tab.into(),
    });
    ui.window().dispatch_event(WindowEvent::KeyReleased {
        text: slint::platform::Key::Tab.into(),
    });
    // Keyboard CUE on a paused deck away from its cue sets the (quantized) cue
    // there (CDJ); holding CUE again previews, and releasing returns to it.
    assert_ne!(state.borrow().presentation(0).transport.position_ms, 0);
    ui.window().dispatch_event(WindowEvent::KeyPressed {
        text: slint::platform::Key::Return.into(),
    });
    ui.window().dispatch_event(WindowEvent::KeyReleased {
        text: slint::platform::Key::Return.into(),
    });
    let cue_point = u64::from(
        state
            .borrow()
            .decks
            .deck(crate::session::deck(0))
            .cue_point_ms,
    );
    assert_ne!(cue_point, 0);
    assert_eq!(
        state.borrow().presentation(0).transport.position_ms,
        cue_point
    );
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: " ".into() });
    assert!(state.borrow().presentation(0).transport.playing);
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text: " ".into() });
    assert!(!state.borrow().presentation(0).transport.playing);
    assert_eq!(
        state.borrow().presentation(0).transport.position_ms,
        cue_point
    );
    nav(ui, width, 2);
    assert_eq!(ui.get_page(), 2);
    click(ui, 50.0, 342.0);
    assert_eq!(state.borrow().presentation(0).transport.position_ms, 1000);
    let position = slint::LogicalPosition::new(50.0, 388.0);
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    refresh(ui, &state.borrow());
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    assert!(state.borrow().presentation(0).cues[0].is_none());
    // MEMORY toggles deck 1's list (the fifth button of its transport row);
    // leaving the page closes it.
    let memory_x = if width > 900.0 { 450.0 } else { 700.0 };
    click(ui, memory_x, 296.0);
    assert_eq!(ui.get_memory_deck(), 0);
    click(ui, memory_x, 296.0);
    assert_eq!(ui.get_memory_deck(), -1);
    click(ui, memory_x, 296.0);
    assert_eq!(ui.get_memory_deck(), 0);
    nav(ui, width, 0);
    ui.window().take_snapshot().unwrap();
    assert_eq!(ui.get_memory_deck(), -1);
    println!(
        "Pointer navigation/load/play/refresh/focus/Space/Return/Tab/cue/delete/memory: PASS at width {width}"
    );
}

pub(super) fn nav(ui: &MainWindow, width: f32, page: usize) {
    click(
        ui,
        16.0 + (page as f32 + 0.5) * (width - 80.0) / 7.0 + page as f32 * 8.0,
        80.0,
    );
}

pub(super) fn capture(
    ui: &MainWindow,
    output: &Path,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    slint::platform::update_timers_and_animations();
    let snapshot = ui.window().take_snapshot()?;
    let mut file = std::io::BufWriter::new(fs::File::create(output.join(format!("{name}.ppm")))?);
    write!(
        file,
        "P6\n{} {}\n255\n",
        snapshot.width(),
        snapshot.height()
    )?;
    for p in snapshot.as_slice() {
        file.write_all(&[p.r, p.g, p.b])?;
    }
    file.flush()?;
    Ok(())
}
