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
    }
    println!(
        "Slint deck/mixer/FX/health callback + pointer/keyboard smoke in both geometries; 38 software-rendered snapshots: PASS"
    );
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
    ui.window().dispatch_event(WindowEvent::KeyPressed {
        text: slint::platform::Key::Return.into(),
    });
    ui.window().dispatch_event(WindowEvent::KeyReleased {
        text: slint::platform::Key::Return.into(),
    });
    assert_eq!(state.borrow().presentation(0).transport.position_ms, 0);
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
    println!(
        "Pointer navigation/load/play/refresh/focus/Space/Return/Tab/cue/delete: PASS at width {width}"
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
