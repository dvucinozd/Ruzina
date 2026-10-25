mod analysis_smoke;
mod analysis_ui;
mod compositor;
mod health_smoke;
mod health_ui;
mod mix_fx_smoke;
mod mix_fx_ui;
mod session;
mod settings_store;
mod smoke;

use ruzina_board_profile::{BoardId, profile};
use ruzina_hot_cues::HotCueKind;
use ruzina_media_catalog::{
    artwork::{ARTWORK_DECK_EDGE, ARTWORK_ROW_EDGE, ArtworkView},
    default_artwork::DEFAULT_ARTWORK_DECK,
    playlists::LibraryLevel,
};
use ruzina_track_analysis::TrackAnalysis;
use ruzina_ui_model::{
    DeckAction, cue_event,
    labels::{HotCueName, LoadKey, TrackTime, deck_key, load_lock_field},
    memory_cues::{MEMORY_CUE_EMPTY, MemoryCueTitle, memory_cue_lines},
};
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

/// The loaded track's cover, or the Ruzina emblem while it is missing,
/// loading or refused; nothing for an empty deck (RP-07, M2.5).
fn artwork_image(view: ArtworkView<'_>) -> slint::Image {
    let pixels: &[u16] = match view {
        ArtworkView::Empty => return slint::Image::default(),
        ArtworkView::Cover { pixels, .. } => pixels,
        ArtworkView::Loading | ArtworkView::Fallback(_) => &DEFAULT_ARTWORK_DECK,
    };
    let edge = u32::from(ARTWORK_DECK_EDGE);
    compositor::rgb565_image(pixels, edge, edge)
}

/// The M2.5 MEMORY panel for one deck: its title and one line per imported
/// memory point or loop, or "No memory cues".
fn memory_cue_text(i: usize, analysis: Option<TrackAnalysis<'_>>) -> (String, String) {
    let cues = analysis.map_or(&[][..], |analysis| analysis.memory_cues());
    let title = MemoryCueTitle {
        deck: deck(i),
        count: cues.len(),
        truncated: analysis.is_some_and(|analysis| analysis.memory_cues_truncated()),
    };
    let lines: Vec<String> = memory_cue_lines(cues.iter().map(|cue| (cue.start_ms, cue.end_ms)))
        .map(|line| line.to_string())
        .collect();
    let text = if lines.is_empty() {
        MEMORY_CUE_EMPTY.into()
    } else {
        lines.join("\n")
    };
    (title.to_string(), text)
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
            let cues = (0..8u8)
                .map(|slot| {
                    let cue = p.cues[usize::from(slot)];
                    CueView {
                        label: HotCueName {
                            slot,
                            is_loop: cue.is_some_and(|cue| cue.kind == HotCueKind::Loop),
                        }
                        .to_string()
                        .into(),
                        occupied: cue.is_some(),
                        letter: HotCueName::letter(slot).to_string().into(),
                    }
                })
                .collect::<Vec<_>>();
            let (memory_title, memory_text) = memory_cue_text(i, analysis);
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
                artwork: artwork_image(session.artwork(i)),
                key: deck_key(session.key(i)).into(),
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
                memory_title: memory_title.into(),
                memory_text: memory_text.into(),
            }
        })
        .collect();
    // Keep repeater instances/focus/pressed state stable across clock updates.
    for (i, view) in views.into_iter().enumerate() {
        rows.set_row_data(i, view);
    }
    library_refresh(ui, session);
    ui.set_media_ready(session.connected());
    ui.set_message(session.message.clone().into());
    ui.set_screensaver(session.screensaver());
    mix_fx_ui::refresh(ui, session);
    health_ui::refresh(ui, session.diagnostics());
    analysis_ui::refresh(ui, session.analysis_observation);
}

/// The current Library level: rows, path and selection (RP-06).
fn library_refresh(ui: &MainWindow, session: &Session) {
    let rows: Vec<LibraryRowView> = session
        .library_rows()
        .into_iter()
        .zip(session.library_row_tracks())
        .map(|((label, kind), track)| {
            let mut row = LibraryRowView {
                label: label.into(),
                kind: match kind {
                    session::RowKind::Track => 0,
                    session::RowKind::Folder => 1,
                    session::RowKind::Playlist => 2,
                },
                ..LibraryRowView::default()
            };
            if let Some(track) = track {
                let (artist, key, bpm, duration_ms) = session.track_columns(track);
                row.artist = artist.into();
                row.key = key.into();
                row.bpm = bpm.to_string().into();
                row.time = TrackTime(duration_ms).to_string().into();
                let edge = u32::from(ARTWORK_ROW_EDGE);
                row.artwork = compositor::rgb565_image(session.row_artwork(track), edge, edge);
            }
            row
        })
        .collect();
    let model = ui.get_library_rows();
    let model = model
        .as_any()
        .downcast_ref::<VecModel<LibraryRowView>>()
        .expect("bound library model");
    // Keep row instances (and their focus) while the level is unchanged.
    if model.row_count() == rows.len() {
        for (i, row) in rows.into_iter().enumerate() {
            if model.row_data(i).as_ref() != Some(&row) {
                model.set_row_data(i, row);
            }
        }
    } else {
        model.set_vec(rows);
    }
    let locked = [session.load_locked(0), session.load_locked(1)];
    let keys: Vec<slint::SharedString> = (0..2)
        .map(|i| {
            LoadKey {
                deck: deck(i),
                locked: locked[i],
            }
            .to_string()
            .into()
        })
        .collect();
    if ui.get_load_keys().iter().ne(keys.iter().cloned()) {
        ui.set_load_keys(ModelRc::new(VecModel::from(keys)));
    }
    if ui.get_load_locked().iter().ne(locked.iter().copied()) {
        ui.set_load_locked(ModelRc::new(VecModel::from(locked.to_vec())));
    }
    ui.set_load_lock_text(load_lock_field(session.load_lock()).into());
    ui.set_load_lock_on(session.load_lock());
    let top = session.library_level() == LibraryLevel::AllTracks;
    ui.set_library_top(top);
    ui.set_library_path(session.library_path().into());
    ui.set_selection(if top || !session.connected() {
        session.selected as i32
    } else {
        session.library_selected().map_or(-1, |i| i as i32)
    });
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
    ui.set_library_rows(ModelRc::new(VecModel::<LibraryRowView>::default()));
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
    ui.on_restore_hold(move |i, pressed| {
        let deck = usize::try_from(i).ok().filter(|i| *i < 2);
        session.borrow_mut().restore_hold(deck, pressed);
        if let Some(ui) = weak.upgrade() {
            refresh(&ui, &session.borrow());
        }
    });
    let weak = ui.as_weak();
    let session = state.clone();
    ui.on_select_track(move |selection| {
        session.borrow_mut().note_activity();
        if selection >= 0 && session.borrow().connected() {
            session.borrow_mut().library_select(selection as usize);
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
        session.borrow_mut().library_load(deck(i as usize));
        if let Some(ui) = weak.upgrade() {
            refresh(&ui, &session.borrow());
        }
    });
    let weak = ui.as_weak();
    let session = state.clone();
    ui.on_load_lock_toggle(move || {
        let on = session.borrow().load_lock();
        session.borrow_mut().set_load_lock(!on);
        if let Some(ui) = weak.upgrade() {
            refresh(&ui, &session.borrow());
        }
    });
    let weak = ui.as_weak();
    let session = state.clone();
    ui.on_library_back(move || {
        session.borrow_mut().library_back();
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
    let session = state.clone();
    ui.on_wake(move || {
        session.borrow_mut().wake();
        if let Some(ui) = weak.upgrade() {
            refresh(&ui, &session.borrow());
        }
    });
    let session = state.clone();
    ui.on_activity(move || session.borrow_mut().note_activity());
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
        let mut report = ruzina_board_profile::diagnostics::ReportBuffer::<4096>::default();
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
