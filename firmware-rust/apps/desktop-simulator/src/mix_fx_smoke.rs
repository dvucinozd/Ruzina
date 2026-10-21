use super::smoke::{capture, click, nav};
use super::*;
use pajoniiir_controller_core::DeckId;
use slint::platform::{PointerEventButton, WindowEvent};
use std::path::Path;

pub fn scenes(
    ui: &MainWindow,
    s: &Rc<RefCell<Session>>,
    output: &Path,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    ui.set_page(4);
    capture(ui, output, &format!("{name}-mixer-default"))?;
    for (i, code, value) in [
        (0, 0, 12000),
        (0, 1, 12000),
        (0, 2, 4000),
        (0, 4, 0),
        (0, 5, 9000),
        (1, 3, 10000),
        (-1, 6, 0),
        (-1, 7, 14000),
        (-1, 8, 0),
        (-1, 9, 9000),
    ] {
        ui.invoke_mixer_level(i, code, value);
    }
    ui.invoke_mixer_toggle(1, 0);
    for code in [1, 2, 3] {
        ui.invoke_mixer_toggle(-1, code);
    }
    assert!(s.borrow().mixer.deck(DeckId::Two).pfl_enabled);
    assert_eq!(s.borrow().mixer.crossfader(), 0);
    capture(ui, output, &format!("{name}-mixer-active"))?;
    let before = s.borrow().mixer;
    ui.invoke_mixer_level(0, 6, 16383); // wrong target for a global parameter
    ui.invoke_mixer_level(-1, 7, -1);
    ui.invoke_mixer_toggle(-2, 0);
    assert_eq!(s.borrow().mixer, before);
    ui.set_page(5);
    capture(ui, output, &format!("{name}-fx-default"))?;
    ui.invoke_fx_action(1);
    ui.invoke_fx_action(2);
    ui.invoke_fx_target(0);
    ui.invoke_fx_depth(96);
    ui.invoke_fx_action(4);
    ui.invoke_pad_bank(0, 1);
    ui.invoke_pad_fx(0, 1, 3, true);
    assert!(s.borrow().decks.deck(DeckId::One).active_pad_fx.is_some());
    capture(ui, output, &format!("{name}-fx-active"))?;
    ui.invoke_pad_fx(0, 1, 3, false);
    ui.invoke_pad_fx(0, 1, 2, true);
    ui.invoke_pad_bank(0, 0);
    assert!(s.borrow().decks.deck(DeckId::One).active_pad_fx.is_none());
    ui.invoke_pad_fx(0, 0, 2, true);
    ui.set_page(0);
    ui.window().take_snapshot()?;
    assert!(s.borrow().decks.deck(DeckId::One).active_pad_fx.is_none());
    Ok(())
}

fn key(ui: &MainWindow, key: slint::SharedString) {
    slint::platform::update_timers_and_animations();
    ui.window().take_snapshot().unwrap();
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: key.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text: key });
}
fn pointer_event(ui: &MainWindow, position: slint::LogicalPosition, pressed: bool) {
    ui.window().dispatch_event(if pressed {
        WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        }
    } else {
        WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        }
    });
}
pub fn pointer(ui: &MainWindow, s: &Rc<RefCell<Session>>, width: f32, height: f32) {
    let portrait = height > width;
    nav(ui, width, 4);
    assert_eq!(ui.get_page(), 4);
    let old = s.borrow().mixer.deck(DeckId::One).pfl_enabled;
    click(ui, 70.0, 476.0);
    assert_eq!(s.borrow().mixer.deck(DeckId::One).pfl_enabled, !old);
    let global_x = if portrait {
        16.0
    } else {
        16.0 + (width - 20.0) / 3.0
    };
    let global_width = if portrait {
        width - 32.0
    } else {
        (width - 56.0) / 3.0
    };
    let y = if portrait { 639.0 } else { 187.0 };
    click(ui, global_x + global_width / 3.0, y);
    key(ui, slint::platform::Key::Home.into());
    assert_eq!(s.borrow().mixer.crossfader(), 0);
    ui.invoke_mixer_level(-1, 6, 8192);
    key(ui, slint::platform::Key::RightArrow.into());
    assert_eq!(s.borrow().mixer.crossfader(), 8356);
    key(ui, slint::platform::Key::End.into());
    assert_eq!(s.borrow().mixer.crossfader(), 16383);
    // A drag remains live across an unrelated refresh.
    let start = slint::LogicalPosition::new(global_x + global_width - 20.0, y);
    pointer_event(ui, start, true);
    refresh(ui, &s.borrow());
    let end = slint::LogicalPosition::new(global_x + 16.0, y);
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position: end });
    pointer_event(ui, end, false);
    assert!(s.borrow().mixer.crossfader() < 2000);
    nav(ui, width, 5);
    assert_eq!(ui.get_page(), 5);
    let beat_width = if portrait {
        width - 32.0
    } else {
        (width - 44.0) * 0.4
    };
    let effect = s.borrow().decks.beat_fx().effect;
    click(ui, 16.0 + beat_width - 30.0, 180.0);
    assert_ne!(s.borrow().decks.beat_fx().effect, effect);
    click(ui, 16.0 + beat_width / 2.0, 280.0);
    assert_eq!(
        s.borrow().decks.beat_fx().target,
        pajoniiir_controller_core::BeatFxTarget::ChannelTwo
    );
    click(ui, 16.0 + beat_width / 3.0, 341.0);
    key(ui, slint::platform::Key::Home.into());
    assert_eq!(s.borrow().decks.beat_fx().depth, 0);
    key(ui, slint::platform::Key::End.into());
    assert_eq!(s.borrow().decks.beat_fx().depth, 127);
    let enabled = s.borrow().decks.beat_fx().enabled;
    click(ui, 70.0, 384.0);
    assert_eq!(s.borrow().decks.beat_fx().enabled, !enabled);
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    ui.window().take_snapshot().unwrap();
    let strip_x = if portrait {
        0.0
    } else {
        (width - 44.0) * 0.4 + 12.0
    };
    let strip_width = if portrait {
        width - 32.0
    } else {
        (width - 44.0) * 0.6
    };
    let cell = (strip_width - 66.0) / 8.0;
    let position = slint::LogicalPosition::new(
        16.0 + strip_x + 12.0 + cell / 2.0,
        if portrait { 619.0 } else { 227.0 },
    );
    pointer_event(ui, position, true);
    assert!(s.borrow().decks.deck(DeckId::One).active_pad_fx.is_some());
    refresh(ui, &s.borrow());
    ui.invoke_pad_bank(0, 1);
    assert!(s.borrow().decks.deck(DeckId::One).active_pad_fx.is_none());
    pointer_event(ui, position, false);
    assert!(s.borrow().decks.deck(DeckId::One).active_pad_fx.is_none());
    pointer_event(ui, position, true);
    assert_eq!(
        s.borrow()
            .decks
            .deck(DeckId::One)
            .active_pad_fx
            .unwrap()
            .bank,
        pajoniiir_deck::PadFxBank::Two
    );
    ui.window().dispatch_event(WindowEvent::PointerExited);
    assert!(s.borrow().decks.deck(DeckId::One).active_pad_fx.is_none());
    click(ui, position.x, position.y);
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: " ".into() });
    assert!(s.borrow().decks.deck(DeckId::One).active_pad_fx.is_some());
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(false));
    assert!(s.borrow().decks.deck(DeckId::One).active_pad_fx.is_none());
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    pointer_event(ui, position, true);
    ui.invoke_media_action();
    ui.window().take_snapshot().unwrap();
    assert!(s.borrow().decks.deck(DeckId::One).active_pad_fx.is_none());
    pointer_event(ui, position, false);
    assert!(s.borrow().message.contains("Media removed"));
    println!(
        "Mixer PFL/slider external update/Home/End/drag+refresh; Beat FX effect/target/depth/toggle; Pad FX bank/cancel/hold/release/focus loss/detach: PASS at {width}x{height}"
    );
}
