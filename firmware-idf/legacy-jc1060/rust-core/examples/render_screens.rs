//! Draws the deck screen's pages on the PC, with the board's Slint renderer, fonts and
//! images (the same `ui/overview.slint` the build compiles), into `OUT/<page>.ppm`: the
//! overview, the Library, Hot Cues, Settings, FX, the boot screen, the screensaver and
//! UPDATING FIRMWARE. What Rust draws itself on the board (waveforms, overviews, meters,
//! the phase strip) is left as the page's own background. Demonstration texts only.
//! Then it times frames as the board draws them and prints their parts (`frame_parts`).
//!
//! cargo run --example render_screens --features "idf-soak-autoplay idf-slint-ui" -- OUT
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType, Rgb565Pixel};
use slint::platform::{Platform, WindowAdapter};
use slint::{Color, Model, ModelRc, SharedString, VecModel};
use std::rc::Rc;

slint::include_modules!();

const WIDTH: usize = 1024;
const HEIGHT: usize = 600;

struct Pc {
    window: Rc<MinimalSoftwareWindow>,
    start: std::time::Instant,
}

impl Platform for Pc {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
    fn duration_since_start(&self) -> core::time::Duration {
        self.start.elapsed()
    }
}

fn text(value: &str) -> SharedString {
    value.into()
}

fn model<T: Clone + 'static>(rows: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(rows))
}

/// The cue colours A to H, as `slint_ui.rs` has them.
const CUES: [(u8, u8, u8); 8] = [
    (0x00, 0xff, 0x66),
    (0x00, 0xe5, 0xff),
    (0xff, 0x00, 0x33),
    (0xff, 0xd6, 0x00),
    (0xa2, 0x59, 0xff),
    (0xff, 0x2e, 0xc4),
    (0x2f, 0x6b, 0xff),
    (0xb6, 0xff, 0x00),
];

fn cues(set: &[usize]) -> ModelRc<CueTile> {
    model(
        (0..8)
            .map(|pad| {
                let (r, g, b) = CUES[pad];
                CueTile {
                    letter: text(&((b'A' + pad as u8) as char).to_string()),
                    time: text(if set.contains(&pad) { "1:12.4" } else { "" }),
                    set: set.contains(&pad),
                    tint: Color::from_rgb_u8(r, g, b),
                }
            })
            .collect(),
    )
}

fn demo(ui: &Overview) {
    ui.set_cover(0);
    ui.set_title_1(text("Demo Track One"));
    ui.set_title_2(text("Second Demo Track (Extended Mix)"));
    ui.set_elapsed_1(text("01:23"));
    ui.set_elapsed_2(text("03:05"));
    ui.set_remaining_1(text("-04:56"));
    ui.set_remaining_2(text("-02:41"));
    ui.set_bpm_1(text("128.00"));
    ui.set_bpm_2(text("127.49"));
    ui.set_pitch_1(text("+0.0%"));
    ui.set_pitch_2(text("-0.4%"));
    ui.set_key_1(text("8A"));
    ui.set_key_2(text("9A"));
    ui.set_status_1(text("LOOP"));
    ui.set_status_2(text("SYNC"));
    ui.set_master_tempo_1(true);
    ui.set_quantize_2(true);
    ui.set_ending_2(true);
    ui.set_blink(true);
    ui.set_playing_1(true);
    ui.set_loaded_1(true);
    ui.set_loaded_2(true);
    ui.set_phase(text("+12 ms"));
    // A stand-in cover: a gradient where the track's artwork goes.
    let mut art = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(200, 200);
    for (i, pixel) in art.make_mut_slice().iter_mut().enumerate() {
        let (x, y) = ((i % 200) as u8, (i / 200) as u8);
        *pixel = slint::Rgb8Pixel::new(80 + x / 2, 40 + y / 3, 120);
    }
    ui.set_cover_1(slint::Image::from_rgb8(art.clone()));
    ui.set_has_cover_1(true);
    ui.set_cover_2(slint::Image::from_rgb8(art));
    ui.set_has_cover_2(true);
    ui.set_fx_effect(text("ECHO"));
    ui.set_fx_beat(text("1/2"));
    ui.set_fx_depth(60);
    ui.set_fx_target(1);
    ui.set_fx_on(true);
    ui.set_cues_1(cues(&[0, 1, 3]));
    ui.set_cues_2(cues(&[2, 4]));
    ui.set_memory_1(model(vec![
        MemoryLine {
            number: text("1"),
            time: text("0:32.0"),
            looped: false,
        },
        MemoryLine {
            number: text("2"),
            time: text("1:04.0"),
            looped: true,
        },
    ]));
    ui.set_library_heading(text("ALL TRACKS"));
    ui.set_library_count(text("25 TRACKS"));
    ui.set_library_has_playlists(true);
    ui.set_library_sort_column(2);
    ui.set_library_bpm_reference(text("128.0"));
    ui.set_library_rows(model(
        (0..10)
            .map(|i| LibraryLine {
                number: text(&(i + 1).to_string()),
                title: text(&format!("Demo Track {}", i + 1)),
                artist: text("Demo Artist"),
                bpm: text(&format!("{}.0", 124 + i)),
                key: text(["8A", "9A", "3B", "10A"][i % 4]),
                time: text(if i % 3 == 0 { "10:24" } else { "5:12" }),
                decks: text(if i == 0 { "1" } else { "" }),
                selected: i == 2,
                kind: if i == 0 { 2 } else { 0 },
                fit: (i % 5) as i32,
            })
            .collect(),
    ));
    let line = |label: &str, value: &str| SettingLine {
        label: text(label),
        value: text(value),
    };
    ui.set_settings_left(model(vec![
        line("NETWORK", "Ruzina-AB2D"),
        line("PASSWORD", "12345678"),
        line("WEB", "192.168.4.1"),
    ]));
    ui.set_settings_right(model(vec![
        line("FIRMWARE", "s219"),
        line("BUILT", "Nov 26 2026 14:00"),
        line("CONTROLLER", "Pioneer DDJ-FLX4"),
    ]));
}

fn save(window: &MinimalSoftwareWindow, path: &std::path::Path) {
    let mut pixels = vec![Rgb565Pixel(0); WIDTH * HEIGHT];
    slint::platform::update_timers_and_animations();
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, WIDTH);
    });
    let mut ppm = format!("P6\n{WIDTH} {HEIGHT}\n255\n").into_bytes();
    for Rgb565Pixel(p) in pixels {
        let (r, g, b) = ((p >> 11) & 31, (p >> 5) & 63, p & 31);
        ppm.extend([
            (r << 3 | r >> 2) as u8,
            (g << 2 | g >> 4) as u8,
            (b << 3 | b >> 2) as u8,
        ]);
    }
    std::fs::write(path, ppm).expect("writes the page");
}

fn main() {
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "screens".into()));
    std::fs::create_dir_all(&out).expect("output folder");
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(slint::PhysicalSize::new(WIDTH as u32, HEIGHT as u32));
    slint::platform::set_platform(Box::new(Pc {
        window: window.clone(),
        start: std::time::Instant::now(),
    }))
    .expect("platform");
    let ui = Overview::new().expect("the deck screen");
    ui.show().expect("shown");
    demo(&ui);
    for (name, tab) in [
        ("overview", 0),
        ("library", 1),
        ("cues", 2),
        ("settings", 3),
        ("fx", 4),
    ] {
        ui.set_active_tab(tab);
        save(&window, &out.join(format!("{name}.ppm")));
    }
    ui.set_active_tab(0);
    for (name, cover, shine) in [
        ("boot", 1, 300),
        ("screensaver", 2, 300),
        ("updating", 3, -1000),
    ] {
        ui.set_cover(cover);
        ui.set_cover_shine(shine);
        save(&window, &out.join(format!("{name}.ppm")));
    }
    let _ = ui.get_library_rows().row_count();
    println!("pages drawn into {}", out.display());
    frame_parts(&window, &ui);
}

fn clock() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_nanos() as u64
}

/// The parts of a frame `frame_parts` prints.
const PARTS: [&str; 9] = [
    "total", "prepare", "dirty", "items", "text", "scene", "lines", "texts", "glyphs",
];

/// Draws frames as the board does (a reused buffer, line by line) and prints the fastest
/// of each part, in nanoseconds on this PC (steadier than the mean on a busy PC), with the
/// scene's preparation split as the board's slow-frame log splits it (events 27, 50 to 52):
/// 1000 Library page turns, each changing all ten rows, then 200 frames on each page in
/// which only the first deck's time changes.
fn frame_parts(window: &MinimalSoftwareWindow, ui: &Overview) {
    ui.set_cover(0);
    ui.set_active_tab(1);
    let rows = Rc::new(VecModel::from(
        ui.get_library_rows().iter().collect::<Vec<_>>(),
    ));
    ui.set_library_rows(rows.clone().into());
    let mut frame = vec![0u16; WIDTH * HEIGHT];
    let mut draw = || {
        let mut parts = [0u64; 9];
        slint::platform::update_timers_and_animations();
        window.draw_if_needed(|renderer| {
            renderer.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
            renderer.set_probe_clock(clock);
            let started = clock();
            let lines = ruzina_screen_lines::render(renderer, &mut frame, &[], clock, None);
            let total = clock() - started;
            let prep = renderer.scene_prep();
            parts = [
                total,
                u64::from(lines.prepare_us),
                u64::from(prep.dirty_us),
                u64::from(prep.items_us),
                u64::from(prep.text_us),
                u64::from(prep.scene_us),
                u64::from(lines.lines_us),
                u64::from(prep.texts),
                u64::from(prep.glyphs),
            ];
        });
        parts
    };
    let fastest = |label: &str, frames: &mut dyn FnMut() -> [u64; 9], count: usize| {
        let mut best = [u64::MAX; 9];
        for _ in 0..count {
            for (best, part) in best.iter_mut().zip(frames()) {
                *best = (*best).min(part);
            }
        }
        let parts: Vec<_> = PARTS
            .iter()
            .zip(best)
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        println!("{label} {}", parts.join(" "));
    };
    draw();
    let mut turn = 0;
    fastest(
        "library_page_turn",
        &mut || {
            turn += 1;
            for row in 0..rows.row_count() {
                let mut line = rows.row_data(row).expect("a row");
                line.title = text(&format!("Demo Track {} of page {turn}", row + 1));
                line.number = text(&(turn * 10 + row + 1).to_string());
                line.selected = row == turn % 10;
                rows.set_row_data(row, line);
            }
            draw()
        },
        1000,
    );
    for (name, tab) in [
        ("overview", 0),
        ("library", 1),
        ("cues", 2),
        ("settings", 3),
        ("fx", 4),
    ] {
        ui.set_active_tab(tab);
        draw();
        let mut second = 0;
        fastest(
            &format!("{name}_time_only"),
            &mut || {
                second += 1;
                ui.set_elapsed_1(text(&format!("01:{:02}", second % 60)));
                draw()
            },
            200,
        );
    }
}
