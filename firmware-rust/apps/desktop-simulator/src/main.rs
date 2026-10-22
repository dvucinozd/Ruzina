mod analysis_smoke;
mod analysis_ui;
mod compositor;
mod health_smoke;
mod health_ui;
mod mix_fx_smoke;
mod mix_fx_ui;
mod session;
mod smoke;

use pajoniiir_board_profile::{BoardId, profile};
use pajoniiir_ui_model::{DeckAction, cue_event};
use session::{Session, deck};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

slint::include_modules!();

fn format_bpm(bpm_milli: u32) -> String {
    format!("{:.2} BPM", bpm_milli as f64 / 1000.0)
}
fn format_time(position_ms: u64) -> String {
    let seconds = position_ms / 1000;
    format!(
        "{:02}:{:02}.{:03}",
        seconds / 60,
        seconds % 60,
        position_ms % 1000
    )
}

fn refresh(ui: &MainWindow, session: &Session) {
    let model = ui.get_decks();
    let rows = model
        .as_any()
        .downcast_ref::<VecModel<DeckView>>()
        .expect("bound deck model");
    let views: Vec<DeckView> = (0..2)
        .map(|i| {
            let p = session.presentation(i);
            let t = p.transport;
            let time = if ui.get_remaining() {
                format!(
                    "−{}",
                    format_time(t.duration_ms.saturating_sub(t.position_ms))
                )
            } else {
                format_time(t.position_ms)
            };
            let analysis = session.analysis(i);
            let source = match analysis {
                Some(a) if !a.detail().is_empty() => "GRID + PREVIEW + DETAIL",
                Some(_) => "GRID + PREVIEW · NO DETAIL",
                None if t.loaded => "NO ANALYSIS · BPM FALLBACK",
                None => "LOAD FROM LIBRARY",
            };
            let cues = (0..8)
                .map(|slot| CueView {
                    label: format!(
                        "{}{}",
                        slot + 1,
                        if p.cues[slot].is_some() { " ●" } else { "" }
                    )
                    .into(),
                    occupied: p.cues[slot].is_some(),
                })
                .collect::<Vec<_>>();
            let cue_model = rows.row_data(i).unwrap().cues;
            let cue_rows = cue_model
                .as_any()
                .downcast_ref::<VecModel<CueView>>()
                .expect("bound cue model");
            for (slot, cue) in cues.into_iter().enumerate() {
                cue_rows.set_row_data(slot, cue);
            }
            DeckView {
                title: session.title(i).into(),
                bpm: if t.loaded {
                    format_bpm(t.bpm_milli)
                } else {
                    "— BPM".into()
                }
                .into(),
                time: time.into(),
                status: format!(
                    "{}  {:+.2}%",
                    if t.playing { "PLAY" } else { "PAUSE" },
                    f64::from(t.pitch_centi_percent) / 100.0
                )
                .into(),
                analysis: source.into(),
                loaded: t.loaded,
                playing: t.playing,
                synced: t.sync,
                quantize: p.quantize,
                range: p.tempo_range_percent.to_string().into(),
                overview: compositor::image(&compositor::compose(p, analysis, false)),
                detail: compositor::image(&compositor::compose(p, analysis, true)),
                cues: cue_model,
                loop_text: p
                    .loop_region
                    .map(|(a, b)| {
                        format!(
                            "LOOP {}–{}",
                            format_time(u64::from(a)),
                            format_time(u64::from(b))
                        )
                    })
                    .unwrap_or("LOOP OFF".into())
                    .into(),
            }
        })
        .collect();
    // Keep repeater instances/focus/pressed state stable across clock updates.
    for (i, view) in views.into_iter().enumerate() {
        rows.set_row_data(i, view);
    }
    ui.set_selection(session.selected as i32);
    ui.set_media_ready(session.connected());
    ui.set_message(session.message.clone().into());
    mix_fx_ui::refresh(ui, session);
    health_ui::refresh(ui, session.diagnostics());
    analysis_ui::refresh(ui, session.analysis_observation);
}

fn bind(ui: &MainWindow, state: Rc<RefCell<Session>>) {
    mix_fx_ui::bind(ui, state.clone());
    ui.set_decks(ModelRc::new(VecModel::from(
        (0..2)
            .map(|_| DeckView {
                cues: ModelRc::new(VecModel::from(vec![CueView::default(); 8])),
                ..DeckView::default()
            })
            .collect::<Vec<_>>(),
    )));
    ui.set_tracks(ModelRc::new(VecModel::from(
        session::TITLES
            .iter()
            .map(|s| (*s).into())
            .collect::<Vec<_>>(),
    )));
    let weak = ui.as_weak();
    let session = state.clone();
    ui.on_deck_action(move |i, code| {
        if !(0..2).contains(&i) {
            return;
        }
        if let Some(action) = DeckAction::from_code(code) {
            session
                .borrow_mut()
                .control(action.event(deck(i as usize), true));
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &session.borrow());
            }
        }
    });
    let weak = ui.as_weak();
    let session = state.clone();
    ui.on_deck_hold(move |i, code, pressed| {
        if !(0..2).contains(&i) {
            return;
        }
        if let Some(action) = DeckAction::from_code(code) {
            session
                .borrow_mut()
                .control(action.event(deck(i as usize), pressed));
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &session.borrow());
            }
        }
    });
    let weak = ui.as_weak();
    let session = state.clone();
    ui.on_cue_action(move |i, slot, clear| {
        if !(0..2).contains(&i) || !(0..8).contains(&slot) {
            return;
        }
        if let Some(event) = cue_event(deck(i as usize), slot as u8, clear, true) {
            session.borrow_mut().control(event);
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &session.borrow());
            }
        }
    });
    let weak = ui.as_weak();
    let session = state.clone();
    ui.on_select_track(move |selection| {
        if (0..session::TITLES.len() as i32).contains(&selection) && session.borrow().connected() {
            session.borrow_mut().selected = selection as usize;
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &session.borrow());
            }
        }
    });
    let weak = ui.as_weak();
    let session = state.clone();
    ui.on_load_track(move |i| {
        if !(0..2).contains(&i) {
            return;
        }
        session.borrow_mut().load(deck(i as usize));
        if let Some(ui) = weak.upgrade() {
            refresh(&ui, &session.borrow());
        }
    });
    let weak = ui.as_weak();
    let session = state.clone();
    ui.on_media_action(move || {
        let connected = session.borrow().connected();
        if connected {
            session.borrow_mut().detach();
        } else {
            session.borrow_mut().reconnect();
        }
        if let Some(ui) = weak.upgrade() {
            refresh(&ui, &session.borrow());
        }
    });
    let weak = ui.as_weak();
    ui.on_display_changed(move || {
        if let Some(ui) = weak.upgrade() {
            refresh(&ui, &state.borrow());
        }
    });
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let analysis = if let Some(index) = args.iter().position(|s| s == "--analysis-observation") {
        let path = args
            .get(index + 1)
            .ok_or("--analysis-observation requires a record file")?
            .clone();
        let value = analysis_ui::read(std::path::Path::new(&path))?;
        args.drain(index..=index + 1);
        Some(value)
    } else {
        None
    };
    if analysis.is_some()
        && args
            .first()
            .is_some_and(|a| a == "--smoke" || a == "--diagnostic-report")
    {
        return Err("--analysis-observation is an interactive captured-record option".into());
    }
    if args.first().is_some_and(|a| a == "--diagnostic-report") {
        let (output, board) = match args.as_slice() {
            [_, output] => (output, BoardId::GuitionJc1060P470cIwY),
            [_, output, id] if id == "jc1060" => (output, BoardId::GuitionJc1060P470cIwY),
            [_, output, id] if id == "jc8012" => (output, BoardId::GuitionJc8012P4a1cIwY),
            _ => {
                return Err(
                    "Usage: --diagnostic-report OUTPUT [jc1060|jc8012] (new host session)".into(),
                );
            }
        };
        let mut report = pajoniiir_board_profile::diagnostics::ReportBuffer::<4096>::default();
        Session::for_board(board)
            .diagnostics()
            .write_to(&mut report)
            .map_err(|_| "report buffer exhausted")?;
        std::fs::write(output, report.bytes())?;
        return Ok(());
    }
    if args.first().is_some_and(|a| a == "--smoke") {
        let output = args.get(1).ok_or("--smoke requires an output directory")?;
        return smoke::run(std::path::Path::new(output));
    }
    let board = match args.as_slice() {
        [] => profile(BoardId::GuitionJc1060P470cIwY),
        [option, id] if option == "--board" && id == "jc1060" => {
            profile(BoardId::GuitionJc1060P470cIwY)
        }
        [option, id] if option == "--board" && id == "jc8012" => {
            profile(BoardId::GuitionJc8012P4a1cIwY)
        }
        _ => {
            return Err("Usage: desktop-simulator [--board jc1060|jc8012] [--analysis-observation FILE] | --smoke OUTPUT".into());
        }
    };
    let ui = MainWindow::new()?;
    ui.window().set_size(slint::PhysicalSize::new(
        u32::from(board.display.width),
        u32::from(board.display.height),
    ));
    ui.set_board_label(format!("{} · HOST SIMULATOR", board.model).into());
    let state = Rc::new(RefCell::new(Session::for_board(board.id)));
    if let Some(value) = analysis {
        state.borrow_mut().analysis_observation = value;
    }
    bind(&ui, state.clone());
    refresh(&ui, &state.borrow());
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    let mut last = Instant::now();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(50),
        move || {
            let now = Instant::now();
            let elapsed = now
                .duration_since(last)
                .as_millis()
                .min(u128::from(u32::MAX)) as u32;
            last = now;
            state.borrow_mut().tick(elapsed);
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &state.borrow());
            }
        },
    );
    ui.run()?;
    Ok(())
}

#[cfg(test)]
mod tests;
