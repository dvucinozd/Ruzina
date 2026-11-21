//! The deck screen on the panel: Slint for everything that changes rarely, and direct
//! drawing for the waveforms, which change many times a second.
//!
//! It shows the decks: what the engine publishes in `deck_view::LIVE`, and each
//! loaded track's title, key and analysis. It logs draw times as load events: 21 the
//! Slint parts (worst, then 31 the average), 22 a waveform frame (32). A Slint frame over
//! `SLOW_FRAME_US` also logs 23 its draw (pass: the rows drawn) and 24 the page updates
//! before it (pass: the open page), then 25 each area drawn (pass: first and last row,
//! value: first and last column, 16 bits each) and 26 the lines (pass) and pixels drawn.
//!
//! The boot screen and the screensaver lie over everything (`screen_idle`); Rust then draws
//! nothing.
use crate::deck_view::{self, DeckSnapshot, FxSnapshot, Library, LibraryRow, LoadedTrack, Marks};
use crate::screen_idle::{Cover, Saver};
use alloc::boxed::Box;
use alloc::rc::Rc;
use ruzina_track_analysis::{AnalysisMemoryCue, Beat, WaveformColumn};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, PhysicalSize};

slint::include_modules!();

const WIDTH: usize = 1024;
const HEIGHT: usize = 600;
const REPORT_MS: u64 = 20_000;
/// A Slint frame longer than this is logged on its own.
const SLOW_FRAME_US: u32 = 30_000;

// A rectangle Rust draws, which Slint must never write over. The numbers below match
// the dark rectangles in `overview.slint`.
use ruzina_screen_lines::Area;

const LANE_WIDTH: usize = 872;
const LANE_HEIGHT: usize = 201;
const LANES: [Area; 2] = [
    Area {
        x: 82,
        y: 46,
        width: LANE_WIDTH,
        height: LANE_HEIGHT,
    },
    Area {
        x: 82,
        y: 248,
        width: LANE_WIDTH,
        height: LANE_HEIGHT,
    },
];
const MINIS: [Area; 2] = [
    Area {
        x: 8,
        y: 552,
        width: 496,
        height: 45,
    },
    Area {
        x: 520,
        y: 552,
        width: 496,
        height: 45,
    },
];
/// The channel meters, left of the lanes: ten segments, 11 pixels high and 14 apart.
const METERS: [Area; 2] = [
    Area {
        x: 68,
        y: 49,
        width: 10,
        height: 140,
    },
    Area {
        x: 68,
        y: 251,
        width: 10,
        height: 140,
    },
];
const DIRECT: [Area; 6] = [LANES[0], LANES[1], MINIS[0], MINIS[1], METERS[0], METERS[1]];
/// The Library, Hot Cues, Settings and FX pages cover the lanes and meters; Rust then draws only
/// the overviews.
const COVERED_DIRECT: [Area; 2] = [MINIS[0], MINIS[1]];
/// The cover's lit letter moves one letter on each step.
const COVER_STEP_MS: u64 = 120;
/// The cover's word has six letters; its lit letter runs from two before the first to two
/// past the last.
const COVER_HEADS: u64 = 6 + 5;
/// The Library page's rows.
const LIBRARY_ROWS: u32 = 12;
// The owner's Rust palette (`docs/reviews/2026-11-21-rust-palette.md`). The main
// background is #100C08, not the spec's #0F0B09, which RGB565 turns neutral grey.
const PANEL: u16 = rgb(0x10, 0x0c, 0x08);
/// The meters: acid green to 0 dB, deep coral red above it, laser red at the top.
const METER_OFF: u16 = rgb(0x2a, 0x1b, 0x14);
const METER_GREEN: u16 = rgb(0x39, 0xff, 0x14);
const METER_AMBER: u16 = rgb(0xe6, 0x19, 0x3c);
const METER_RED: u16 = rgb(0xff, 0x00, 0x33);

const BACKGROUND: u16 = rgb(0x10, 0x0c, 0x08);
const BEAT_LINE: u16 = rgb(0x38, 0x25, 0x1e);
const BAR_LINE: u16 = rgb(0x57, 0x4b, 0x45);
const PLAYHEAD: u16 = rgb(0xf5, 0xf0, 0xed);
const PLAYHEAD_X: usize = LANE_WIDTH / 2;
/// Refreshes per frame: 30 frames a second, each shown exactly two refreshes after the
/// last.
const REFRESHES_PER_FRAME: u32 = 2;
/// Half the mini overview's height, for a full-scale column.
const MINI_HALF: u32 = 21;
const PLAYED_SHADE: u16 = rgb(0x38, 0x25, 0x1e);
const MINI_PLAYHEAD: u16 = rgb(0xf5, 0xf0, 0xed);
/// The waveform by frequency: lows, mids and highs. Rekordbox's detail waveform (PWV3)
/// has no bands, only how white each column is, 0 to 7: the whiter, the brighter the
/// sound, so 0 to 2 draw as lows, 3 and 4 as mids, 5 to 7 as highs.
const BANDS: [[u8; 3]; 3] = [[0xe6, 0x4c, 0x0c], [0x00, 0x88, 0xff], [0x00, 0xe5, 0xff]];
const LOWS: u16 = rgb(BANDS[0][0], BANDS[0][1], BANDS[0][2]);
const MIDS: u16 = rgb(BANDS[1][0], BANDS[1][1], BANDS[1][2]);
const HIGHS: u16 = rgb(BANDS[2][0], BANDS[2][1], BANDS[2][2]);
/// The acid green triangle on each bar line, for beat matching: its base at the lane's
/// edge towards the other deck, pointing into the lane.
const BAR_CAP: u16 = rgb(0x39, 0xff, 0x14);
const BAR_CAP_REACH: i64 = 6;
const BAR_CAP_HEIGHT: usize = 8;
/// Hot Cues A to H: a line through the lane and a flag at the edge away from the bar
/// triangles.
/// A to D as the spec has them; E to H chosen with the owner (2026-11-21).
const CUE_RGB: [[u8; 3]; 8] = [
    [0x00, 0xff, 0x66],
    [0x00, 0xe5, 0xff],
    [0xff, 0x00, 0x33],
    [0xff, 0xd6, 0x00],
    [0xa2, 0x59, 0xff],
    [0xff, 0x2e, 0xc4],
    [0x2f, 0x6b, 0xff],
    [0xb6, 0xff, 0x00],
];
const CUE_COLORS: [u16; 8] = {
    let mut colors = [0; 8];
    let mut pad = 0;
    while pad < 8 {
        let [red, green, blue] = CUE_RGB[pad];
        colors[pad] = rgb(red, green, blue);
        pad += 1;
    }
    colors
};
const CUE_FLAG_WIDTH: i64 = 7;
const CUE_FLAG_HEIGHT: usize = 7;
/// Rekordbox's memory cues, in warm white (red is Hot Cue C's): a line through the lane
/// (and at a memory loop's end), and at its start a triangle at the Hot Cue flags' edge,
/// pointing into the lane. The overview marks them at its top and bottom.
const MEMORY: u16 = rgb(0xf5, 0xf0, 0xed);
const MEMORY_FLAG_REACH: i64 = 5;
const MEMORY_FLAG_HEIGHT: usize = 7;
const MINI_MEMORY_ROWS: usize = 5;
/// The running loop: its ends as lines, and its span shaded behind the waveform.
const LOOP_EDGE: u16 = rgb(0xff, 0x77, 0x00);
const LOOP_SHADE: u16 = rgb(0x30, 0x1c, 0x00);
/// The white edge on loud peaks.
const TIP: u16 = rgb(0xf5, 0xf0, 0xed);
const TIP_LOUDNESS: u8 = 26;

const fn rgb(red: u8, green: u8, blue: u8) -> u16 {
    ((red as u16 >> 3) << 11) | ((green as u16 >> 2) << 5) | (blue as u16 >> 3)
}

unsafe extern "C" {
    fn esp_timer_get_time() -> i64;
    fn ruzina_panel_flush(buffer: *mut u16, y: u32, rows: u32);
    fn ruzina_panel_present(buffer: *mut u16);
    fn ruzina_panel_refreshes() -> u32;
    fn ruzina_panel_wait_refresh();
    fn ruzina_panel_move(
        from: *const u16,
        to: *mut u16,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        dx: u32,
    ) -> i32;
    /// A strip of `pixels` pixels in internal RAM, made on the first call, or null.
    fn ruzina_panel_strip(pixels: u32) -> *mut u16;
    fn ruzina_panel_blit(
        from: *const u16,
        from_width: u32,
        from_x: u32,
        to: *mut u16,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> i32;
    fn ruzina_bridge_load_report(pass: u32, event: u32, value: u32);
    /// The running app's description (IDF `esp_app_desc_t`).
    fn esp_app_get_description() -> *const AppDescription;
    /// GT911 touches now (0 when none or no touch controller), the first one mapped to
    /// the screen in `x` and `y`.
    fn ruzina_touch_poll(
        raw_x: *mut u32,
        raw_y: *mut u32,
        x: *mut u32,
        y: *mut u32,
        errors: *mut u32,
    ) -> u32;
}

/// Reads the touch controller and hands Slint a press, a move or a release; `down`
/// holds the last touched point.
fn touch(window: &slint::Window, down: &mut Option<slint::LogicalPosition>, errors: &mut u32) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let (mut raw_x, mut raw_y, mut x, mut y) = (0, 0, 0, 0);
    // SAFETY: five writable locals; C reads I2C on this task only.
    let touches = unsafe { ruzina_touch_poll(&mut raw_x, &mut raw_y, &mut x, &mut y, errors) };
    let event = match (touches > 0, *down) {
        (true, None) => {
            let position = slint::LogicalPosition::new(x as f32, y as f32);
            *down = Some(position);
            WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            }
        }
        (true, Some(_)) => {
            let position = slint::LogicalPosition::new(x as f32, y as f32);
            *down = Some(position);
            WindowEvent::PointerMoved { position }
        }
        (false, Some(position)) => {
            *down = None;
            let _ = window.dispatch_event_with_result(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
            WindowEvent::PointerExited
        }
        (false, None) => return,
    };
    let _ = window.dispatch_event_with_result(event);
}

/// The start of IDF's `esp_app_desc_t` at v6.1, which the Settings page shows.
#[repr(C)]
struct AppDescription {
    magic_word: u32,
    secure_version: u32,
    reserved: [u32; 2],
    version: [u8; 32],
    project_name: [u8; 32],
    time: [u8; 16],
    date: [u8; 16],
    idf_version: [u8; 32],
    elf_sha256: [u8; 32],
}

/// A C string field as text, up to its first zero.
fn c_text(bytes: &[u8]) -> &str {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    core::str::from_utf8(&bytes[..end]).unwrap_or("?")
}

/// The Settings page's choices, as the settings in force have them.
fn show_setting_values(ui: &Overview) {
    use ruzina_settings::{JogMode, MasterTrim, Screensaver};
    let settings = crate::deck_settings::current();
    ui.set_setting_load_lock(settings.load_lock);
    ui.set_setting_jog_cdj_1(settings.jog_modes[0] == JogMode::Cdj);
    ui.set_setting_jog_cdj_2(settings.jog_modes[1] == JogMode::Cdj);
    ui.set_setting_backlight(i32::from(settings.backlight_percent));
    ui.set_setting_trim(match settings.master_trim {
        MasterTrim::ZeroDb => 0,
        MasterTrim::Minus3Db => 1,
        MasterTrim::Minus6Db => 2,
    });
    ui.set_setting_screensaver(match settings.screensaver {
        Screensaver::Off => 0,
        Screensaver::Minutes2 => 1,
        Screensaver::Minutes5 => 2,
        Screensaver::Minutes10 => 3,
    });
}

/// The Settings page's two columns, as their models: left, then right.
type SettingsModels = (
    Rc<slint::VecModel<SettingLine>>,
    Rc<slint::VecModel<SettingLine>>,
);

/// Brings `model` to `lines`, changing only the rows that differ.
fn sync_rows(model: &slint::VecModel<SettingLine>, lines: alloc::vec::Vec<SettingLine>) {
    use slint::Model;
    let len = lines.len();
    for (row, line) in lines.into_iter().enumerate() {
        if row >= model.row_count() {
            model.push(line);
        } else if model.row_data(row).as_ref() != Some(&line) {
            model.set_row_data(row, line);
        }
    }
    while model.row_count() > len {
        model.remove(model.row_count() - 1);
    }
}

/// The Settings page's two columns of figures: the decks, then the mix and the
/// firmware. The settings themselves sit below the decks.
fn show_settings(models: &SettingsModels, uptime_ms: u64) {
    use core::sync::atomic::Ordering::Relaxed;
    let stats: [u32; 8] = core::array::from_fn(|i| deck_view::STATS[i].load(Relaxed));
    let line = |label: &str, value: alloc::string::String| SettingLine {
        label: label.into(),
        value: value.into(),
    };
    let mut left = alloc::vec![line(
        "Tracks",
        alloc::format!("{}", deck_view::library_len())
    )];
    // Each deck on one line: its source rate, the audio buffered, and its underruns.
    for deck in 0..2 {
        let rate = stats[5 + deck];
        let buffered = stats[3 + deck];
        left.push(line(
            if deck == 0 { "Deck 1" } else { "Deck 2" },
            if rate == 0 {
                "no track".into()
            } else {
                let buffered = match buffered.checked_mul(100).and_then(|b| b.checked_div(rate)) {
                    Some(centiseconds) => {
                        alloc::format!("{}.{:02} s", centiseconds / 100, centiseconds % 100)
                    }
                    None => "-".into(),
                };
                alloc::format!(
                    "{}.{} kHz, {}, {} underruns",
                    rate / 1_000,
                    rate % 1_000 / 100,
                    buffered,
                    stats[1 + deck]
                )
            },
        ));
    }
    // SAFETY: IDF returns its static app description, valid for the program's life.
    let app = unsafe { &*esp_app_get_description() };
    let right = alloc::vec![
        line(
            "Controller",
            match crate::controller_midi::attached() {
                None => "none".into(),
                Some((_, _, Some(name))) => name.into(),
                Some((vid, pid, None)) => alloc::format!("{vid:04X}:{pid:04X}, no profile"),
            }
        ),
        line(
            "Played",
            deck_view::clock((u64::from(stats[0]) * 1_000 / 48_000) as u32),
        ),
        line("Limiter", alloc::format!("{} samples", stats[7])),
        line(
            "Uptime",
            deck_view::clock(uptime_ms.min(u64::from(u32::MAX)) as u32)
        ),
        line("Firmware", c_text(&app.version).into()),
        line(
            "Built",
            alloc::format!("{} {}", c_text(&app.date), c_text(&app.time)),
        ),
        line(
            "ELF",
            alloc::format!(
                "{:02x}{:02x}{:02x}{:02x}",
                app.elf_sha256[0],
                app.elf_sha256[1],
                app.elf_sha256[2],
                app.elf_sha256[3]
            ),
        ),
        line(
            "Soak DJ",
            if cfg!(feature = "soak-autoplay") {
                "on"
            } else {
                "off"
            }
            .into(),
        ),
    ];
    #[cfg(feature = "idf-web-remote")]
    let right = {
        let mut right = right;
        let (network, password) = wifi_credentials();
        right.push(line("Wi-Fi", network));
        right.push(line("Password", password));
        right
    };
    sync_rows(&models.0, left);
    sync_rows(&models.1, right);
}

/// The deck's own Wi-Fi network and its password, from the C6 probe.
#[cfg(feature = "idf-web-remote")]
fn wifi_credentials() -> (alloc::string::String, alloc::string::String) {
    unsafe extern "C" {
        fn ruzina_wifi_credentials(
            ssid: *mut u8,
            ssid_len: usize,
            password: *mut u8,
            password_len: usize,
        ) -> bool;
    }
    let (mut ssid, mut password) = ([0u8; 33], [0u8; 65]);
    // SAFETY: both buffers are writable for their lengths; C writes NUL-terminated text.
    let ready = unsafe {
        ruzina_wifi_credentials(
            ssid.as_mut_ptr(),
            ssid.len(),
            password.as_mut_ptr(),
            password.len(),
        )
    };
    if !ready {
        return ("starting".into(), "-".into());
    }
    (c_text(&ssid).into(), c_text(&password).into())
}

#[cfg(all(feature = "screen-still", feature = "fill-gap-cycle"))]
compile_error!("The still screen and the fill gap cycle each need the diagnostic phase");

/// What the USB audio fill's profile learns ran beside it (`uac_profile_sched.h`).
const SCREEN: u32 = 0;
const MOVE: u32 = 3;
const FLUSH: u32 = 4;

/// Profile images: the USB audio fill learns whether a frame was being drawn, a lane
/// moved or rows written back while it ran.
#[cfg(feature = "idf-uac-profile")]
fn mark(which: u32, running: bool) {
    unsafe extern "C" {
        fn ruzina_uac_activity(which: u32, running: bool);
    }
    // SAFETY: two scalars; only this task marks these activities.
    unsafe { ruzina_uac_activity(which, running) };
}
#[cfg(not(feature = "idf-uac-profile"))]
fn mark(_: u32, _: bool) {}

/// Profile images: the screen counts only while this task is on the CPU, not while it
/// sleeps waiting for a gap between fills. A lane's move goes on in the PPA meanwhile and
/// counts as its own activity.
#[cfg(feature = "idf-uac-profile")]
fn mark_own_task() {
    unsafe extern "C" {
        fn xTaskGetCurrentTaskHandle() -> *mut core::ffi::c_void;
        fn ruzina_uac_activity_task(which: u32, task: *const core::ffi::c_void);
    }
    // SAFETY: the running task's handle, kept by C only to compare in its switch hooks.
    unsafe { ruzina_uac_activity_task(SCREEN, xTaskGetCurrentTaskHandle()) };
}

/// The 30 s diagnostic phases (`--screen-still`, `--fill-gap-cycle`): on every other 30 s.
/// The fill's profile is told which.
#[cfg(any(feature = "screen-still", feature = "fill-gap-cycle"))]
fn diagnostic_phase() -> bool {
    const PHASE_US: u64 = 30_000_000;
    unsafe extern "C" {
        fn ruzina_uac_phase(on: bool);
    }
    let on = (now_us() / PHASE_US) % 2 == 1;
    // SAFETY: a scalar.
    unsafe { ruzina_uac_phase(on) };
    on
}

/// The steps of a frame that move the most memory, each kept out of the USB audio fills.
#[derive(Clone, Copy)]
enum Step {
    /// Slint's lines, their copy into the frame buffer and its write-back.
    Slint = 0,
    /// The PPA moving a lane.
    Move = 1,
    /// A lane's new columns and its write-back.
    Columns = 2,
}

/// Keeps the screen's memory work out of the USB audio fills: on s163 a screen frame
/// during a fill made 88 % of the Master Tempo fills over 3 ms. Each step waits, asleep,
/// for a gap between fills that it fits, by how long it took before (`fill_gap.h`).
struct Gate {
    /// Each step's recent mean, in microseconds.
    means: [u32; 3],
    /// The diagnostic phase in which the screen does not wait (`--fill-gap-cycle`).
    #[cfg_attr(not(feature = "fill-gap-cycle"), allow(dead_code))]
    off: bool,
    /// This frame's waits, in microseconds.
    waited: u32,
    /// Profile images: each step's time and each frame's waits.
    #[cfg(feature = "idf-uac-profile")]
    times: [Times; 4],
}

impl Gate {
    fn new() -> Self {
        Self {
            means: [0; 3],
            off: false,
            waited: 0,
            #[cfg(feature = "idf-uac-profile")]
            times: Default::default(),
        }
    }

    fn run<R>(&mut self, step: Step, work: impl FnOnce() -> R) -> R {
        let mean = &mut self.means[step as usize];
        // A quarter over the mean, for the steps that run long.
        let budget = *mean + *mean / 4;
        #[cfg(feature = "fill-gap-cycle")]
        let gated = !self.off;
        #[cfg(not(feature = "fill-gap-cycle"))]
        let gated = true;
        if gated {
            unsafe extern "C" {
                fn ruzina_uac_wait_gap(budget_us: u32) -> u32;
            }
            // SAFETY: a scalar; this is the screen's task, the only one that waits.
            self.waited += unsafe { ruzina_uac_wait_gap(budget) };
        }
        let started = now_us();
        let result = work();
        let took = (now_us() - started) as u32;
        *mean = if *mean == 0 {
            took
        } else {
            (*mean * 7 + took) / 8
        };
        #[cfg(feature = "idf-uac-profile")]
        self.times[step as usize].add(took);
        result
    }

    /// The end of a frame: its waits.
    fn frame_done(&mut self) {
        #[cfg(feature = "idf-uac-profile")]
        self.times[3].add(self.waited);
        self.waited = 0;
    }

    /// Profile images: each step's longest and mean time, and the frames' waits.
    #[cfg(feature = "idf-uac-profile")]
    fn report(&mut self) {
        for (i, times) in self.times.iter_mut().enumerate() {
            times.report(33 + i as u32);
        }
    }
}

fn now_us() -> u64 {
    // SAFETY: IDF monotonic task-safe clock, no retained pointer.
    unsafe { esp_timer_get_time() as u64 }
}

struct Board {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for Board {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
    fn duration_since_start(&self) -> core::time::Duration {
        core::time::Duration::from_micros(now_us())
    }
}

/// A loaded track's waveform as the lane needs it.
#[derive(Clone, Copy)]
struct Wave<'a> {
    detail: &'a [WaveformColumn],
    beats: &'a [Beat],
    rate_hz: u16,
    /// The time the lane shows.
    window_ms: i64,
    marks: Marks,
    /// The playhead, where the shading from a Loop In ends.
    position_ms: u32,
    memory: &'a [AnalysisMemoryCue],
}

impl Wave<'_> {
    /// The track time at the start of track pixel `at`.
    const fn pixel_ms(&self, at: i64) -> i64 {
        at * self.window_ms / LANE_WIDTH as i64
    }

    /// The track pixel at `position_ms`.
    const fn track_pixel(&self, position_ms: u32) -> i64 {
        position_ms as i64 * LANE_WIDTH as i64 / self.window_ms
    }

    /// Loudness and colour of the loudest detail column under track pixel `at`;
    /// `None` outside the track.
    fn sample(&self, at: i64) -> Option<(u8, u16)> {
        if self.rate_hz == 0 || at < 0 {
            return None;
        }
        let rate = i64::from(self.rate_hz);
        let first = usize::try_from(self.pixel_ms(at) * rate / 1_000).ok()?;
        let len = self.detail.len();
        if first >= len {
            return None;
        }
        let last = usize::try_from((self.pixel_ms(at + 1) * rate + 999) / 1_000)
            .unwrap_or(0)
            .clamp(first + 1, len);
        self.detail[first..last]
            .iter()
            .map(|&column| ink(column))
            .reduce(|best, next| if next.0 > best.0 { next } else { best })
    }

    /// Half height, colour and white tip of track pixel `at`, drawn as the donor does:
    /// the loudest of the pixel and its two neighbours, at least a centre line, so the
    /// shape stays joined. `None` outside the track.
    fn column(&self, at: i64) -> Option<(u8, u16, bool)> {
        let own = self.sample(at)?;
        let (loudness, color) = [self.sample(at - 1), self.sample(at + 1)]
            .into_iter()
            .flatten()
            .fold(own, |best, next| if next.0 > best.0 { next } else { best });
        let half = 1 + u32::from(loudness) * (LANE_HEIGHT as u32 - 8) / 62;
        Some((half as u8, color, loudness >= TIP_LOUDNESS && half >= 3))
    }

    /// The grid line at track pixel `at`: a bar line on a downbeat, a beat line on other
    /// beats.
    fn grid(&self, at: i64) -> u16 {
        if at < 0 {
            return BACKGROUND;
        }
        let start = self.pixel_ms(at) as u32;
        let end = self.pixel_ms(at + 1) as u32;
        let next = self.beats.partition_point(|beat| beat.time_ms < start);
        match self.beats.get(next) {
            Some(beat) if beat.time_ms < end && beat.phase == 0 => BAR_LINE,
            Some(beat) if beat.time_ms < end => BEAT_LINE,
            _ if self.shaded(start) => LOOP_SHADE,
            _ => BACKGROUND,
        }
    }

    /// Whether the lane shades `ms`: inside the running loop, or from a Loop In waiting
    /// for its Out up to the playhead. Columns the playhead passes are drawn again each
    /// frame, so the shading follows it.
    fn shaded(&self, ms: u32) -> bool {
        self.marks
            .looped
            .is_some_and(|(from, to)| (from..to).contains(&ms))
            || self
                .marks
                .pending_in
                .is_some_and(|from| (from..self.position_ms).contains(&ms))
    }

    /// Whether `ms` falls in track pixel `at`.
    fn holds(&self, at: i64, ms: u32) -> bool {
        at >= 0 && (self.pixel_ms(at)..self.pixel_ms(at + 1)).contains(&i64::from(ms))
    }

    /// The Hot Cue at track pixel `at`, by pad.
    fn cue(&self, at: i64) -> Option<usize> {
        (0..8).find(|&pad| self.marks.cues[pad].is_some_and(|ms| self.holds(at, ms)))
    }

    /// Whether a memory cue starts in track pixel `at`, or with `ends` a memory loop
    /// ends there.
    fn memory_at(&self, at: i64, ends: bool) -> bool {
        self.memory.iter().any(|cue| {
            self.holds(at, cue.start_ms)
                || ends && cue.end_ms.is_some_and(|end| self.holds(at, end))
        })
    }

    /// The colour of a mark through the whole of track pixel `at`: a Hot Cue, else an
    /// end of the running loop, else a memory cue.
    fn mark(&self, at: i64) -> Option<u16> {
        if let Some(pad) = self.cue(at) {
            return Some(CUE_COLORS[pad]);
        }
        let loop_edge = match (self.marks.pending_in, self.marks.looped) {
            (Some(from), _) => self.holds(at, from),
            (None, Some((start, end))) => self.holds(at, start) || self.holds(at, end),
            (None, None) => false,
        };
        if loop_edge {
            return Some(LOOP_EDGE);
        }
        self.memory_at(at, true).then_some(MEMORY)
    }
}

/// A waveform column's loudness (0 to 31) and its colour in the donor's palette. A
/// Rekordbox column carries a 3-bit colour hint, which the import keeps as a blue to
/// white shade; other colours stay as they are.
fn ink(column: WaveformColumn) -> (u8, u16) {
    let loudness = ((u32::from(column.peak) * 31 + 32_767) / 65_535) as u8;
    let hint = ((column.color_rgb565 >> 11) * 7 + 15) / 31;
    let shade = ((hint * 31 / 7) << 11) | ((hint * 63 / 7) << 5) | 31;
    if shade != column.color_rgb565 {
        return (loudness, band_mix(column.color_rgb565));
    }
    let color = match hint {
        0..=2 => LOWS,
        3 | 4 => MIDS,
        _ => HIGHS,
    };
    (loudness, color)
}

/// A Rekordbox colour column (PWV4), whose red, green and blue are its bass, mid and
/// treble: the three band colours mixed in that measure.
fn band_mix(rgb565: u16) -> u16 {
    let weights = [
        u32::from(rgb565 >> 11) * 2,
        u32::from(rgb565 >> 5 & 0x3f),
        u32::from(rgb565 & 0x1f) * 2,
    ];
    let total: u32 = weights.iter().sum();
    if total == 0 {
        return LOWS;
    }
    let channel = |c: usize| {
        let sum: u32 = (0..3)
            .map(|band| weights[band] * u32::from(BANDS[band][c]))
            .sum();
        (sum / total) as u8
    };
    rgb(channel(0), channel(1), channel(2))
}

/// The lane's view of `track` with `marks` and the playhead at `position_ms`, showing
/// `window_ms` of it.
fn wave(track: Option<&LoadedTrack>, window_ms: u32, marks: Marks, position_ms: u32) -> Wave<'_> {
    let window_ms = i64::from(window_ms);
    let Some(view) = track
        .and_then(|track| track.analysis.as_ref())
        .map(|a| a.view())
    else {
        return Wave {
            detail: &[],
            beats: &[],
            rate_hz: 0,
            window_ms,
            marks,
            position_ms,
            memory: &[],
        };
    };
    Wave {
        detail: view.detail(),
        beats: view.beat_grid().map_or(&[], |grid| grid.beats()),
        rate_hz: view.detail_rate_hz(),
        window_ms,
        marks,
        position_ms,
        memory: view.memory_cues(),
    }
}

/// Columns of a lane's strip in internal RAM (`draw_strips`): 25 KiB.
const STRIP_COLUMNS: usize = 64;
/// The strip in internal RAM: 16 of Slint's lines (`ruzina_screen_lines::Stage`), 32 KiB,
/// or a lane's columns in its start.
const STRIP_PIXELS: usize = 16 * WIDTH;
const _: () = assert!(STRIP_COLUMNS * LANE_HEIGHT <= STRIP_PIXELS);

/// Copies a block of Slint's lines from the strip into a frame buffer, by the PPA.
fn copy_block(strip: &[u16], frame: &mut [u16], block: ruzina_screen_lines::Block) -> bool {
    mark(MOVE, true);
    // SAFETY: the block lies inside `strip`, `stage_width` to a row (screen-lines makes
    // it so); `frame` is one of the panel's frame buffers and C bounds the block to it.
    let copied = unsafe {
        ruzina_panel_blit(
            strip.as_ptr(),
            block.stage_width as u32,
            block.stage_x as u32,
            frame.as_mut_ptr(),
            block.x as u32,
            block.y as u32,
            block.width as u32,
            block.height as u32,
        )
    } == 0;
    mark(MOVE, false);
    copied
}

/// Where a lane's pixels go: the frame buffer, or a strip that holds columns `first` and
/// up of all the lane's rows, `stride` to a row.
struct Canvas<'a> {
    pixels: &'a mut [u16],
    stride: usize,
    /// The index of the lane's top left pixel, or of the strip's.
    base: usize,
    first: usize,
}

impl<'a> Canvas<'a> {
    fn frame(frame: &'a mut [u16], area: Area) -> Self {
        Self {
            pixels: frame,
            stride: WIDTH,
            base: area.y * WIDTH + area.x,
            first: 0,
        }
    }

    /// The index of the lane's column `x`, at or after `first`, in row `y`.
    fn index(&self, x: usize, y: usize) -> usize {
        self.base + y * self.stride + x - self.first
    }
}

/// Draws columns `columns` of the lane whose left edge shows detail column `left`. Each
/// row is built in internal RAM and written in one copy; rows mirror below the middle.
fn draw_columns(
    canvas: &mut Canvas<'_>,
    area: Area,
    wave: Wave<'_>,
    left: i64,
    columns: core::ops::Range<usize>,
) {
    let width = columns.len();
    if width == 0 {
        return;
    }
    let mut half = [0u8; LANE_WIDTH];
    let mut color = [BACKGROUND; LANE_WIDTH];
    let mut back = [BACKGROUND; LANE_WIDTH];
    // The distance from the middle where the white tip starts; past the column if none.
    let mut tip = [u8::MAX; LANE_WIDTH];
    for (i, x) in columns.clone().enumerate() {
        let at = left + x as i64;
        if (PLAYHEAD_X - 1..=PLAYHEAD_X + 1).contains(&x) {
            (color[i], back[i]) = (PLAYHEAD, PLAYHEAD);
            continue;
        }
        back[i] = wave.grid(at);
        if let Some(mark) = wave.mark(at) {
            back[i] = mark;
            continue;
        }
        if let Some((height, ink, tipped)) = wave.column(at) {
            (half[i], color[i]) = (height, ink);
            if tipped {
                tip[i] = height - 1;
            }
        }
    }
    let middle = LANE_HEIGHT / 2;
    let mut line = [BACKGROUND; LANE_WIDTH];
    for distance in 0..=middle {
        let reach = distance as u8;
        for (pixel, (((&height, &ink), &ground), &tip)) in line[..width]
            .iter_mut()
            .zip(half.iter().zip(&color).zip(&back).zip(&tip))
        {
            *pixel = if height == 0 || reach > height {
                ground
            } else if reach >= tip {
                TIP
            } else {
                ink
            };
        }
        for row in [middle - distance, middle + distance] {
            if row < LANE_HEIGHT {
                let start = canvas.index(columns.start, row);
                canvas.pixels[start..start + width].copy_from_slice(&line[..width]);
            }
        }
    }
    draw_bar_caps(canvas, area, wave, left, columns.clone());
    draw_memory_flags(canvas, area, wave, left, columns.clone());
    draw_cue_flags(canvas, area, wave, left, columns);
}

/// Draws the triangles of the memory cues near columns `columns`, clipped to them and
/// clear of the playhead: at the Hot Cue flags' edge, pointing into the lane. A Hot Cue's
/// flag is drawn over one.
fn draw_memory_flags(
    canvas: &mut Canvas<'_>,
    area: Area,
    wave: Wave<'_>,
    left: i64,
    columns: core::ops::Range<usize>,
) {
    if wave.memory.is_empty() {
        return;
    }
    let at_top = area.y == LANES[0].y;
    let (first, end) = (columns.start as i64, columns.end as i64);
    for centre in first - MEMORY_FLAG_REACH..end + MEMORY_FLAG_REACH {
        if !wave.memory_at(left + centre, false) {
            continue;
        }
        for row in 0..MEMORY_FLAG_HEIGHT {
            // Widest at the edge, a point towards the middle.
            let reach =
                MEMORY_FLAG_REACH * (MEMORY_FLAG_HEIGHT - row) as i64 / MEMORY_FLAG_HEIGHT as i64;
            let y = if at_top { row } else { LANE_HEIGHT - 1 - row };
            for x in (centre - reach).max(first)..=(centre + reach).min(end - 1) {
                let x = x as usize;
                if !(PLAYHEAD_X - 1..=PLAYHEAD_X + 1).contains(&x) {
                    let at = canvas.index(x, y);
                    canvas.pixels[at] = MEMORY;
                }
            }
        }
    }
}

/// Draws the flags of the Hot Cues near columns `columns`, clipped to them and clear of
/// the playhead: a square right of the cue's line, at the edge away from the bar
/// triangles (the top of deck 1's lane, the bottom of deck 2's).
fn draw_cue_flags(
    canvas: &mut Canvas<'_>,
    area: Area,
    wave: Wave<'_>,
    left: i64,
    columns: core::ops::Range<usize>,
) {
    let at_top = area.y == LANES[0].y;
    let (first, end) = (columns.start as i64, columns.end as i64);
    for line in first - CUE_FLAG_WIDTH..end {
        let Some(pad) = wave.cue(left + line) else {
            continue;
        };
        for row in 0..CUE_FLAG_HEIGHT {
            let y = if at_top { row } else { LANE_HEIGHT - 1 - row };
            for x in line.max(first)..(line + CUE_FLAG_WIDTH).min(end) {
                let x = x as usize;
                if !(PLAYHEAD_X - 1..=PLAYHEAD_X + 1).contains(&x) {
                    let at = canvas.index(x, y);
                    canvas.pixels[at] = CUE_COLORS[pad];
                }
            }
        }
    }
}

/// Draws the red triangles of the bar lines near columns `columns`, clipped to them and
/// clear of the playhead. Deck 1's lane has them at the bottom, deck 2's at the top.
fn draw_bar_caps(
    canvas: &mut Canvas<'_>,
    area: Area,
    wave: Wave<'_>,
    left: i64,
    columns: core::ops::Range<usize>,
) {
    let at_bottom = area.y == LANES[0].y;
    let (first, end) = (columns.start as i64, columns.end as i64);
    for centre in first - BAR_CAP_REACH..end + BAR_CAP_REACH {
        if wave.grid(left + centre) != BAR_LINE {
            continue;
        }
        for row in 0..BAR_CAP_HEIGHT {
            // Widest at the edge, a point towards the middle.
            let reach = BAR_CAP_REACH * (BAR_CAP_HEIGHT - row) as i64 / BAR_CAP_HEIGHT as i64;
            let y = if at_bottom {
                LANE_HEIGHT - 1 - row
            } else {
                row
            };
            for x in (centre - reach).max(first)..=(centre + reach).min(end - 1) {
                let x = x as usize;
                if !(PLAYHEAD_X - 1..=PLAYHEAD_X + 1).contains(&x) {
                    let at = canvas.index(x, y);
                    canvas.pixels[at] = BAR_CAP;
                }
            }
        }
    }
}

/// Draws columns `columns` of the lane in `frame`: in strips in internal RAM that the PPA
/// copies into `frame`, so the CPU does not write PSRAM. Without a strip, or once a copy
/// fails, it draws straight into `frame` and drops the strip, and the caller then writes
/// the lane's rows back: a later copy would discard what the CPU wrote there.
fn draw_strips(
    frame: &mut [u16],
    strip: &mut Option<&mut [u16]>,
    area: Area,
    wave: Wave<'_>,
    left: i64,
    columns: core::ops::Range<usize>,
) {
    let mut start = columns.start;
    while start < columns.end {
        let Some(pixels) = strip.as_deref_mut() else {
            draw_columns(
                &mut Canvas::frame(frame, area),
                area,
                wave,
                left,
                start..columns.end,
            );
            return;
        };
        let end = (start + STRIP_COLUMNS).min(columns.end);
        let width = end - start;
        let pixels = &mut pixels[..width * LANE_HEIGHT];
        let mut canvas = Canvas {
            pixels,
            stride: width,
            base: 0,
            first: start,
        };
        draw_columns(&mut canvas, area, wave, left, start..end);
        mark(MOVE, true);
        // SAFETY: the strip holds `width` by LANE_HEIGHT pixels; `frame` is one of the
        // panel's frame buffers and C bounds the block to it.
        let copied = unsafe {
            ruzina_panel_blit(
                canvas.pixels.as_ptr(),
                width as u32,
                0,
                frame.as_mut_ptr(),
                (area.x + start) as u32,
                area.y as u32,
                width as u32,
                LANE_HEIGHT as u32,
            )
        } == 0;
        mark(MOVE, false);
        if copied {
            start = end;
        } else {
            *strip = None;
        }
    }
}

/// Brings the lane in `back` to show track pixel `left` at its left edge. When `shown`,
/// the other frame buffer, holds it at most half a lane earlier (its lane starts at the
/// pixel given with it), the hardware copies those pixels, moved left, and only the new
/// columns and the playhead are drawn; otherwise the whole lane is. The columns are drawn
/// through `strip` (`draw_strips`). `back` holds the frame before `shown`, so it is never
/// left as it was. Each of the two steps starts between USB audio fills (`Gate`).
fn draw_lane(
    gate: &mut Gate,
    strip: Option<&mut [u16]>,
    back: &mut [u16],
    shown: Option<(&[u16], i64)>,
    area: Area,
    wave: Wave<'_>,
    left: i64,
) {
    let step = shown.map(|(_, shown_left)| left - shown_left);
    let moved = shown.zip(step).is_some_and(|((shown, _), dx)| {
        dx >= 0
            && dx < LANE_WIDTH as i64 / 2
            && gate.run(Step::Move, || {
                mark(MOVE, true);
                // SAFETY: both are the panel's frame buffers; C bounds the block to them.
                let moved = unsafe {
                    ruzina_panel_move(
                        shown.as_ptr(),
                        back.as_mut_ptr(),
                        area.x as u32,
                        area.y as u32,
                        LANE_WIDTH as u32,
                        LANE_HEIGHT as u32,
                        dx as u32,
                    )
                };
                mark(MOVE, false);
                moved
            }) == 0
    });
    gate.run(Step::Columns, || {
        let mut strip = strip;
        match (moved, step) {
            // A paused deck: the copy is the whole frame.
            (true, Some(0)) => {}
            (true, Some(dx)) => {
                let dx = dx as usize;
                draw_strips(
                    back,
                    &mut strip,
                    area,
                    wave,
                    left,
                    LANE_WIDTH - dx..LANE_WIDTH,
                );
                // The old playhead moved left with the rest.
                let playhead = PLAYHEAD_X - 1 - dx..PLAYHEAD_X + 2;
                draw_strips(back, &mut strip, area, wave, left, playhead);
            }
            _ => draw_strips(back, &mut strip, area, wave, left, 0..LANE_WIDTH),
        }
        // The PPA wrote memory itself; what the CPU wrote is still in cache.
        if strip.is_none() {
            flush(back, area.y, LANE_HEIGHT);
        }
    });
}

/// Draws a channel meter with `lit` of its ten segments lit from the bottom: green, then
/// amber from the eighth, red at the top. Flushed at once: its cache lines reach into
/// the lane, which the PPA driver invalidates before it copies, and would drop the
/// meter's unwritten pixels.
fn draw_meter(frame: &mut [u16], area: Area, lit: i32) {
    for row in 0..area.height {
        // Segment 9 is at the top; each is 11 rows with a 3-row gap below.
        let segment = 9 - (row / 14) as i32;
        let color = if row % 14 >= 11 {
            PANEL
        } else if segment >= lit {
            METER_OFF
        } else if segment >= 9 {
            METER_RED
        } else if segment >= 7 {
            METER_AMBER
        } else {
            METER_GREEN
        };
        let start = (area.y + row) * WIDTH + area.x;
        frame[start..start + area.width].fill(color);
    }
    flush(frame, area.y, area.height);
}

/// Draws the whole-track overview, shaded up to the playhead at `x`, with each memory
/// cue of a track `duration_ms` long marked at its top and bottom.
fn draw_mini(
    frame: &mut [u16],
    area: Area,
    preview: &[WaveformColumn],
    x: Option<usize>,
    memory: &[AnalysisMemoryCue],
    duration_ms: u32,
) {
    let middle = area.height / 2;
    let mut heights = [0u32; 512];
    let mut colors = [BACKGROUND; 512];
    let mut marked = [false; 512];
    if duration_ms != 0 {
        for cue in memory {
            let column = (u64::from(cue.start_ms.min(duration_ms)) * (area.width as u64 - 1)
                / u64::from(duration_ms)) as usize;
            if let Some(mark) = marked.get_mut(column) {
                *mark = true;
            }
        }
    }
    for column in 0..area.width {
        if let Some(entry) = preview.get(column * preview.len() / area.width.max(1)) {
            heights[column] = u32::from(entry.peak) * MINI_HALF / u32::from(u16::MAX);
            colors[column] = if x.is_some_and(|x| column < x) {
                PLAYED_SHADE
            } else {
                ink(*entry).1
            };
        }
    }
    for row in 0..area.height {
        let distance = row.abs_diff(middle) as u32;
        let edge = row < MINI_MEMORY_ROWS || row >= area.height - MINI_MEMORY_ROWS;
        let start = (area.y + row) * WIDTH + area.x;
        for column in 0..area.width {
            frame[start + column] = if x == Some(column) {
                MINI_PLAYHEAD
            } else if edge && marked[column] {
                MEMORY
            } else if heights[column] != 0 && distance <= heights[column] {
                colors[column]
            } else {
                BACKGROUND
            };
        }
    }
    flush(frame, area.y, area.height);
}

/// Writes `rows` rows of the buffer `frame` back from cache, for the panel's DMA.
fn flush(frame: &mut [u16], y: usize, rows: usize) {
    mark(FLUSH, true);
    // SAFETY: `frame` is one of the two frame buffers; C bounds the rows to it.
    unsafe { ruzina_panel_flush(frame.as_mut_ptr(), y as u32, rows as u32) };
    mark(FLUSH, false);
}

#[derive(Clone, Copy, Default)]
struct Times {
    count: u32,
    worst: u32,
    total: u64,
}

impl Times {
    fn add(&mut self, us: u32) {
        self.count += 1;
        self.worst = self.worst.max(us);
        self.total += u64::from(us);
    }
    fn report(&mut self, event: u32) {
        if self.count != 0 {
            let average = (self.total / u64::from(self.count)) as u32;
            // SAFETY: diagnostic telemetry passes three u32 values only.
            unsafe {
                ruzina_bridge_load_report(self.count, event, self.worst);
                ruzina_bridge_load_report(self.count, event + 10, average);
            }
        }
        *self = Self::default();
    }
}

/// An overview as drawn: the track's catalog index and the playhead column.
type MiniState = (u32, Option<usize>);

/// One deck's texts and flags, as last handed to Slint.
#[derive(Clone, Default, PartialEq)]
struct DeckTexts {
    title: alloc::string::String,
    elapsed: alloc::string::String,
    remaining: alloc::string::String,
    bpm: alloc::string::String,
    pitch: alloc::string::String,
    key: alloc::string::String,
    status: &'static str,
    playing: bool,
    master_tempo: bool,
    quantize: bool,
}

fn texts(track: Option<&LoadedTrack>, deck: DeckSnapshot) -> DeckTexts {
    let loaded = deck.has(deck_view::LOADED);
    let duration = track.map_or(0, |track| track.duration_ms);
    let title = track.map_or_else(alloc::string::String::new, |track| {
        if track.title.is_empty() {
            track.artist.clone()
        } else {
            track.title.clone()
        }
    });
    DeckTexts {
        title,
        elapsed: if loaded {
            deck_view::clock(deck.position_ms)
        } else {
            "--:--".into()
        },
        remaining: if loaded && duration != 0 {
            deck_view::remaining(deck.position_ms, duration)
        } else {
            "--:--".into()
        },
        bpm: deck_view::bpm(deck.bpm_x100, deck.pitch_centipercent),
        pitch: deck_view::pitch(deck.pitch_centipercent),
        key: track.map_or_else(alloc::string::String::new, |track| track.key.clone()),
        status: if !loaded {
            "EMPTY"
        } else if deck.has(deck_view::SYNC) {
            "SYNC"
        } else if deck.has(deck_view::LOOPING) {
            "LOOP"
        } else {
            ""
        },
        playing: deck.has(deck_view::PLAYING),
        master_tempo: deck.has(deck_view::MASTER_TEMPO),
        quantize: deck.has(deck_view::QUANTIZE),
    }
}

fn show_texts(ui: &Overview, deck: usize, texts: &DeckTexts) {
    let title = texts.title.as_str().into();
    let elapsed = texts.elapsed.as_str().into();
    let remaining = texts.remaining.as_str().into();
    let bpm = texts.bpm.as_str().into();
    let pitch = texts.pitch.as_str().into();
    let key = texts.key.as_str().into();
    let status = texts.status.into();
    if deck == 0 {
        ui.set_title_1(title);
        ui.set_elapsed_1(elapsed);
        ui.set_remaining_1(remaining);
        ui.set_bpm_1(bpm);
        ui.set_pitch_1(pitch);
        ui.set_key_1(key);
        ui.set_status_1(status);
        ui.set_playing_1(texts.playing);
        ui.set_master_tempo_1(texts.master_tempo);
        ui.set_quantize_1(texts.quantize);
    } else {
        ui.set_title_2(title);
        ui.set_elapsed_2(elapsed);
        ui.set_remaining_2(remaining);
        ui.set_bpm_2(bpm);
        ui.set_pitch_2(pitch);
        ui.set_key_2(key);
        ui.set_status_2(status);
        ui.set_playing_2(texts.playing);
        ui.set_master_tempo_2(texts.master_tempo);
        ui.set_quantize_2(texts.quantize);
    }
}

/// The Library page as shown: the list's version, the playlist tree, the level, the
/// first row, the selection and each deck's track.
/// The catalog version, the shelf, the level, the first row, the selection, each deck's
/// track, and whether the media are gone (their tracks can no longer load).
type LibraryState = (
    u32,
    usize,
    deck_view::Level,
    u32,
    Option<u32>,
    [u32; 2],
    bool,
);

/// Shows the Library page's level from row `first`: the selected row, and each track's
/// decks.
///
/// The page's model lasts: only the lines that differ are replaced, so moving the
/// selection redraws two rows, not the page.
fn show_library(
    ui: &Overview,
    model: &slint::VecModel<LibraryLine>,
    rows: &[LibraryRow],
    state: LibraryState,
) {
    use deck_view::{Level, Row};
    use slint::Model;
    let (_, _, level, first, selected, decks, gone) = state;
    let len = deck_view::level_len(level);
    let shelf = deck_view::shelf();
    let lines = (first..len.min(first + LIBRARY_ROWS)).filter_map(|index| {
        let line = match deck_view::row_at(level, index)? {
            Row::Track(track) => {
                let row = rows.get(track as usize)?;
                let on = |deck: usize| decks[deck] == track;
                LibraryLine {
                    number: alloc::format!("{}", index + 1).into(),
                    title: if row.title.is_empty() {
                        row.artist.as_str().into()
                    } else {
                        row.title.as_str().into()
                    },
                    artist: row.artist.as_str().into(),
                    bpm: deck_view::bpm(row.bpm_x100, 0).into(),
                    key: row.key.as_str().into(),
                    time: deck_view::clock(row.duration_ms).into(),
                    decks: match (on(0), on(1)) {
                        (true, true) => "D1 D2",
                        (true, false) => "D1",
                        (false, true) => "D2",
                        (false, false) => "",
                    }
                    .into(),
                    selected: false,
                    kind: 0,
                }
            }
            Row::Node(node) => {
                let node = shelf?.nodes.get(node as usize)?;
                let (count, what) = if node.folder {
                    (node.children.len(), "FOLDER")
                } else {
                    (node.tracks.len(), "PLAYLIST")
                };
                LibraryLine {
                    number: alloc::format!("{}", index + 1).into(),
                    title: node.name.as_str().into(),
                    artist: alloc::format!("{what}  {count}").into(),
                    kind: if node.folder { 1 } else { 2 },
                    ..LibraryLine::default()
                }
            }
        };
        Some(LibraryLine {
            selected: selected == Some(index),
            ..line
        })
    });
    let mut count = 0;
    for (row, line) in lines.enumerate() {
        count = row + 1;
        if row >= model.row_count() {
            model.push(line);
        } else if model.row_data(row).as_ref() != Some(&line) {
            model.set_row_data(row, line);
        }
    }
    while model.row_count() > count {
        model.remove(model.row_count() - 1);
    }
    // The media went away: the rows stay as they were read, but none of them loads until
    // the app restarts (a USB stick put back restarts it by itself).
    if gone {
        ui.set_library_count("RESTART NEEDED".into());
        ui.set_library_heading("MEDIA REMOVED".into());
        ui.set_library_can_back(level != Level::All || deck_view::level_len(Level::Top) != 0);
        return;
    }
    ui.set_library_count(match selected {
        Some(index) => alloc::format!("{} / {}", index + 1, len).into(),
        None if level == Level::All => "NO TRACKS".into(),
        None => "EMPTY".into(),
    });
    let mut heading = alloc::string::String::from(match level {
        Level::All => "ALL TRACKS",
        Level::Top | Level::Node(_) => "PLAYLISTS",
    });
    for name in deck_view::path_names() {
        heading.push_str(" / ");
        heading.push_str(name);
    }
    ui.set_library_heading(heading.into());
    ui.set_library_can_back(level != Level::All || deck_view::level_len(Level::Top) != 0);
}

/// The Library page's first row: when the selection leaves the page, the list turns a
/// whole page, so scrolling redraws every row once a page rather than at every step (a
/// page of text takes about 100 ms to draw).
fn library_first(first: u32, selected: u32, len: u32) -> u32 {
    let first = if selected < first || selected >= first + LIBRARY_ROWS {
        selected - selected % LIBRARY_ROWS
    } else {
        first
    };
    first.min(len.saturating_sub(LIBRARY_ROWS))
}

/// Shows deck `deck`'s cover, `COVER_EDGE` square in RGB565, or none.
fn show_cover(ui: &Overview, deck: usize, cover: Option<&[u16]>) {
    let edge = u32::from(deck_view::COVER_EDGE);
    let image = cover
        .filter(|pixels| pixels.len() == (edge * edge) as usize)
        .map(|pixels| {
            let mut buffer = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(edge, edge);
            for (out, &pixel) in buffer.make_mut_slice().iter_mut().zip(pixels) {
                let (r, g, b) = (pixel >> 11, pixel >> 5 & 0x3f, pixel & 0x1f);
                *out = slint::Rgb8Pixel {
                    r: (r << 3 | r >> 2) as u8,
                    g: (g << 2 | g >> 4) as u8,
                    b: (b << 3 | b >> 2) as u8,
                };
            }
            slint::Image::from_rgb8(buffer)
        });
    let shown = image.is_some();
    let image = image.unwrap_or_default();
    if deck == 0 {
        ui.set_cover_1(image);
        ui.set_has_cover_1(shown);
    } else {
        ui.set_cover_2(image);
        ui.set_has_cover_2(shown);
    }
}

/// The Hot Cues page as shown: each deck's cues by pad, and whether it has a track.
type CueState = [([Option<u32>; 8], bool); 2];

/// Shows each deck's eight pads: the cue's time, or how to set one.
fn show_cues(ui: &Overview, state: CueState) {
    for (deck, (cues, loaded)) in state.into_iter().enumerate() {
        let tiles: alloc::vec::Vec<CueTile> = (0..8)
            .map(|pad| {
                let [red, green, blue] = CUE_RGB[pad];
                CueTile {
                    letter: alloc::format!("{}", char::from(b'A' + pad as u8)).into(),
                    time: match cues[pad] {
                        Some(ms) => {
                            alloc::format!("{}.{}", deck_view::clock(ms), ms / 100 % 10).into()
                        }
                        None if loaded => "TAP TO SET".into(),
                        None => "".into(),
                    },
                    set: cues[pad].is_some(),
                    tint: slint::Color::from_rgb_u8(red, green, blue),
                }
            })
            .collect();
        let tiles = slint::ModelRc::new(slint::VecModel::from(tiles));
        if deck == 0 {
            ui.set_cues_1(tiles);
            ui.set_loaded_1(loaded);
        } else {
            ui.set_cues_2(tiles);
            ui.set_loaded_2(loaded);
        }
    }
}

/// Lists a deck's memory cues on the Hot Cues page: each number, its time, and whether it
/// is a loop.
fn show_memory(ui: &Overview, deck: usize, track: Option<&LoadedTrack>) {
    let lines: alloc::vec::Vec<MemoryLine> = track
        .and_then(|track| track.analysis.as_ref())
        .map(|analysis| {
            analysis
                .view()
                .memory_cues()
                .iter()
                .enumerate()
                .map(|(n, cue)| MemoryLine {
                    number: alloc::format!("{}", n + 1).into(),
                    time: alloc::format!(
                        "{}.{}",
                        deck_view::clock(cue.start_ms),
                        cue.start_ms / 100 % 10
                    )
                    .into(),
                    looped: cue.end_ms.is_some(),
                })
                .collect()
        })
        .unwrap_or_default();
    let lines = slint::ModelRc::new(slint::VecModel::from(lines));
    if deck == 0 {
        ui.set_memory_1(lines);
    } else {
        ui.set_memory_2(lines);
    }
}

fn show_fx(ui: &Overview, fx: FxSnapshot) {
    ui.set_fx_effect(fx.effect_name().into());
    ui.set_fx_beat(fx.beat_name().into());
    ui.set_fx_depth(fx.depth_percent());
    ui.set_fx_target(i32::from(fx.target));
    ui.set_fx_on(fx.on);
}

/// Runs the UI on this task. It draws each frame into the buffer the panel does not
/// show, then swaps the two at the end of a refresh.
///
/// # Safety
/// `shown` and `back` must be the panel's two `1024 * 600` RGB565 frame buffers, `shown`
/// the one on the panel, written by no other code.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_slint_run(shown: *mut u16, back: *mut u16) {
    // This task's small allocations come from a pool, and the slow-frame log counts
    // them (`idf_alloc`).
    crate::idf_alloc::serve_task();
    // Slint redraws in each buffer what changed since that buffer was last shown. Copying
    // the other buffer instead (s97) took a PSRAM-to-PSRAM burst that delayed the USB
    // audio fill on the other core by up to 4 ms.
    let window = MinimalSoftwareWindow::new(RepaintBufferType::SwappedBuffers);
    window.set_size(PhysicalSize::new(WIDTH as u32, HEIGHT as u32));
    if slint::platform::set_platform(Box::new(Board {
        window: window.clone(),
    }))
    .is_err()
    {
        return;
    }
    let Ok(ui) = Overview::new() else { return };
    if ui.show().is_err() {
        return;
    }
    ui.on_play(|deck| {
        deck_view::TOUCH.push(deck as usize & 1, deck_view::TouchControl::Play);
    });
    ui.on_cue(|deck, down| {
        deck_view::TOUCH.push(deck as usize & 1, deck_view::TouchControl::Cue(down));
    });
    // The Library page's first row, which a tap counts from.
    let library_top = Rc::new(core::cell::Cell::new(0u32));
    let top = library_top.clone();
    ui.on_pick(move |row| {
        let index = top.get() + row.max(0) as u32;
        // A second tap on a selected folder or playlist opens it.
        let node = matches!(
            deck_view::row_at(deck_view::level(), index),
            Some(deck_view::Row::Node(_))
        );
        if node && deck_view::selected() == Some(index) {
            deck_view::TOUCH.push(0, deck_view::TouchControl::LibraryOpen);
        } else {
            deck_view::select(index);
        }
    });
    ui.on_library_back(|| {
        deck_view::TOUCH.push(0, deck_view::TouchControl::LibraryBack);
    });
    ui.on_load(|deck| {
        deck_view::TOUCH.push(deck as usize & 1, deck_view::TouchControl::Load);
    });
    ui.on_fx(|kind, value| {
        use deck_view::FxTouch;
        let value = value.clamp(0.0, 127.0) as u8;
        let fx = match kind {
            0 => FxTouch::Effect(value),
            1 => FxTouch::Beat(value),
            2 => FxTouch::Target(value),
            // A drag along the depth bar sends only depths the engine does not have.
            3 if FxSnapshot::unpack(deck_view::FX.load(core::sync::atomic::Ordering::Relaxed))
                .depth
                == value =>
            {
                return;
            }
            3 => FxTouch::Depth(value),
            _ => FxTouch::On,
        };
        deck_view::TOUCH.push(0, deck_view::TouchControl::Fx(fx));
    });
    ui.on_master_tempo(|deck| {
        deck_view::TOUCH.push(deck as usize & 1, deck_view::TouchControl::MasterTempo);
    });
    ui.on_hot_cue(|deck, pad, down, delete| {
        let pad = (pad & 7) as u8;
        let control = deck_view::TouchControl::HotCue { pad, delete, down };
        deck_view::TOUCH.push(deck as usize & 1, control);
    });
    ui.on_memory_cue(|deck, n| {
        let control = deck_view::TouchControl::Memory((n & 15) as u8);
        deck_view::TOUCH.push(deck as usize & 1, control);
    });
    // A setting tapped: in force at once (the USB client task applies it on its next
    // turn, the brightness here), saved by the load task once it has been quiet.
    let weak = ui.as_weak();
    ui.on_setting(move |kind, value| {
        use ruzina_settings::{JogMode, MasterTrim, Screensaver};
        let jog = if value == 1 {
            JogMode::Cdj
        } else {
            JogMode::Vinyl
        };
        let before = crate::deck_settings::current();
        let settings = crate::deck_settings::change((now_us() / 1_000) as u32, |s| match kind {
            0 => s.load_lock = value == 1,
            1 => s.jog_modes[0] = jog,
            2 => s.jog_modes[1] = jog,
            3 => {
                s.backlight_percent =
                    crate::deck_settings::step_backlight(s.backlight_percent, value == 1)
            }
            4 => {
                s.master_trim = match value {
                    1 => MasterTrim::Minus3Db,
                    2 => MasterTrim::Minus6Db,
                    _ => MasterTrim::ZeroDb,
                }
            }
            _ => {
                s.screensaver = match value {
                    0 => Screensaver::Off,
                    2 => Screensaver::Minutes5,
                    3 => Screensaver::Minutes10,
                    _ => Screensaver::Minutes2,
                }
            }
        });
        if settings.backlight_percent != before.backlight_percent {
            unsafe extern "C" {
                fn ruzina_display_backlight(percent: u32);
            }
            // SAFETY: a scalar; the C side clamps it and drives the backlight PWM.
            unsafe { ruzina_display_backlight(u32::from(settings.backlight_percent)) };
        }
        if let Some(ui) = weak.upgrade() {
            show_setting_values(&ui);
        }
    });
    show_setting_values(&ui);
    let mut library = Box::new(Library {
        rows: alloc::vec::Vec::new(),
    });
    let mut library_version = 0u32;
    let mut shown_library: Option<LibraryState> = None;
    let library_model = Rc::new(slint::VecModel::<LibraryLine>::default());
    ui.set_library_rows(slint::ModelRc::from(library_model.clone()));
    // The Settings page's two columns keep their models: a refresh changes only the
    // rows whose text changed, so Slint keeps the others' items.
    let settings_models: SettingsModels = (Rc::default(), Rc::default());
    ui.set_settings_left(slint::ModelRc::from(settings_models.0.clone()));
    ui.set_settings_right(slint::ModelRc::from(settings_models.1.clone()));
    let mut shown_cues: Option<CueState> = None;
    // When the Settings page next refreshes its figures.
    let mut settings_at = 0u64;
    let mut was_covered = false;
    let mut shown_cover = Cover::Boot;
    let mut shown_head = None;
    let (mut touched, mut touch_errors) = (None, 0);
    // SAFETY: the caller hands over both frame buffers.
    let mut buffers = unsafe {
        [
            core::slice::from_raw_parts_mut(shown, WIDTH * HEIGHT),
            core::slice::from_raw_parts_mut(back, WIDTH * HEIGHT),
        ]
    };
    let mut tracks: [Option<Box<LoadedTrack>>; 2] = [None, None];
    let mut shown_texts: [Option<DeckTexts>; 2] = [None, None];
    let mut shown_fx = None;
    // The channel meters' lit segments; they fall a segment a frame.
    let mut meters = [0i32; 2];
    // Per buffer and deck: the meter's lit segments.
    let mut meter_at: [[Option<i32>; 2]; 2] = [[None; 2]; 2];
    // Per buffer and deck: the lane's left column and time span, and the overview's track
    // and playhead.
    let mut lane_at: [[Option<(i64, i64)>; 2]; 2] = [[None; 2]; 2];
    let mut mini_at: [[Option<MiniState>; 2]; 2] = [[None; 2]; 2];
    // Per buffer and deck: the revision of the marks its lane shows.
    let mut marks_at: [[Option<u32>; 2]; 2] = [[None; 2]; 2];
    let (mut slint_times, mut wave_times) = (Times::default(), Times::default());
    let start = now_us() / 1_000;
    let mut next_report = REPORT_MS;
    let mut drawing = 1;
    let mut gate = Gate::new();
    // SAFETY: C makes the strip once and hands it only to this task.
    let mut strip = unsafe {
        let pixels = ruzina_panel_strip(STRIP_PIXELS as u32);
        (!pixels.is_null()).then(|| core::slice::from_raw_parts_mut(pixels, STRIP_PIXELS))
    };
    #[cfg(feature = "idf-uac-profile")]
    mark_own_task();
    // SAFETY: reads a counter.
    let mut target = unsafe { ruzina_panel_refreshes() } + REFRESHES_PER_FRAME;
    let timeout_ms = || crate::deck_settings::current().screensaver.timeout_ms();
    let mut saver = Saver::new(0, timeout_ms());
    loop {
        let ms = now_us() / 1_000 - start;
        if ms >= next_report {
            slint_times.report(21);
            wave_times.report(22);
            #[cfg(feature = "idf-uac-profile")]
            gate.report();
            next_report = ms + REPORT_MS;
        }
        touch(ui.window(), &mut touched, &mut touch_errors);
        #[cfg(feature = "fill-gap-cycle")]
        {
            gate.off = diagnostic_phase();
        }
        #[cfg(feature = "screen-still")]
        if diagnostic_phase() {
            // SAFETY: waits on the panel's refresh.
            unsafe { ruzina_panel_wait_refresh() };
            continue;
        }
        let updates_started = now_us();
        let decks = [deck_view::LIVE[0].load(), deck_view::LIVE[1].load()];
        // The boot screen, then the screensaver once the deck has stood idle.
        let playing = decks.iter().any(|deck| deck.has(deck_view::PLAYING));
        let cover = saver.tick(ms as u32, touched.is_some(), playing, timeout_ms());
        if cover != shown_cover {
            ui.set_cover(match cover {
                Cover::None => 0,
                Cover::Boot => 1,
                Cover::Screensaver => 2,
            });
            // Off again: the overviews come back, drawn anew in each buffer.
            mini_at = [[None; 2]; 2];
            shown_cover = cover;
        }
        let hidden = cover != Cover::None;
        let head = hidden.then(|| ((ms / COVER_STEP_MS) % COVER_HEADS) as i32 - 2);
        if head != shown_head {
            ui.set_cover_head(head.unwrap_or(0));
            shown_head = head;
        }
        for deck in 0..2 {
            if let Some(track) = deck_view::take_track(deck) {
                show_cover(&ui, deck, track.cover.as_deref());
                show_memory(&ui, deck, Some(&track));
                tracks[deck] = Some(track);
                lane_at[0][deck] = None;
                lane_at[1][deck] = None;
            }
            if !decks[deck].has(deck_view::LOADED) && tracks[deck].is_some() {
                show_cover(&ui, deck, None);
                show_memory(&ui, deck, None);
                tracks[deck] = None;
                lane_at[0][deck] = None;
                lane_at[1][deck] = None;
            }
            let texts = texts(tracks[deck].as_deref(), decks[deck]);
            if shown_texts[deck].as_ref() != Some(&texts) {
                show_texts(&ui, deck, &texts);
                shown_texts[deck] = Some(texts);
            }
        }
        for (deck, meter) in meters.iter_mut().enumerate() {
            *meter = deck_view::meter_segments(deck_view::PEAKS.take(deck)).max(*meter - 1);
        }
        // The browse knob pressed on another page shows the Library.
        if deck_view::SHOW_LIBRARY.swap(false, core::sync::atomic::Ordering::Relaxed) {
            ui.set_active_tab(1);
        }
        // Pressed on a track in the Library, it goes back to the Overview.
        if deck_view::SHOW_OVERVIEW.swap(false, core::sync::atomic::Ordering::Relaxed)
            && ui.get_active_tab() == 1
        {
            ui.set_active_tab(0);
        }
        // The Library page: the browse knob's target, and the list around the selection.
        let tab = ui.get_active_tab();
        let library_open = tab == 1;
        deck_view::LIBRARY_OPEN.store(library_open, core::sync::atomic::Ordering::Relaxed);
        // The Library, Hot Cues, Settings and FX pages cover the lanes and meters, and the
        // boot screen and screensaver cover everything.
        let covered = hidden || matches!(tab, 1..=4);
        if covered != was_covered {
            // A page covered the lanes and meters, or uncovered them: Rust draws them
            // again in each buffer.
            lane_at = [[None; 2]; 2];
            meter_at = [[None; 2]; 2];
            was_covered = covered;
        }
        // The Settings page's figures, twice a second while it is open.
        if tab == 3 {
            if ms >= settings_at {
                show_settings(&settings_models, ms);
                settings_at = ms + 500;
            }
        } else {
            settings_at = 0;
        }
        if tab == 2 {
            let state = core::array::from_fn(|deck| {
                let loaded = decks[deck].has(deck_view::LOADED);
                (deck_view::MARKS[deck].load().1.cues, loaded)
            });
            if shown_cues != Some(state) {
                show_cues(&ui, state);
                shown_cues = Some(state);
            }
        }
        if let Some(rows) = deck_view::take_library() {
            library = rows;
            library_version = library_version.wrapping_add(1);
        }
        if library_open {
            let level = deck_view::level();
            let selected = deck_view::selected();
            let first = library_first(
                library_top.get(),
                selected.unwrap_or(0),
                deck_view::level_len(level),
            );
            library_top.set(first);
            let decks = core::array::from_fn(|deck| {
                tracks[deck].as_ref().map_or(u32::MAX, |track| track.index)
            });
            let shelf = deck_view::shelf().map_or(0, |shelf| core::ptr::from_ref(shelf) as usize);
            let gone = crate::media_signal::muted();
            let state = (library_version, shelf, level, first, selected, decks, gone);
            if shown_library != Some(state) {
                show_library(&ui, &library_model, &library.rows, state);
                shown_library = Some(state);
            }
        }
        let fx = FxSnapshot::unpack(deck_view::FX.load(core::sync::atomic::Ordering::Relaxed));
        if shown_fx != Some(fx) {
            show_fx(&ui, fx);
            shown_fx = Some(fx);
        }
        slint::platform::update_timers_and_animations();
        let [first, second] = &mut buffers;
        let (buffer, shown) = if drawing == 0 {
            (&mut **first, &**second)
        } else {
            (&mut **second, &**first)
        };
        mark(SCREEN, true);
        let allocs_before = crate::idf_alloc::counted();
        let started = now_us();
        let mut rows_drawn = 0;
        let mut drawn_areas = None;
        let mut split = None;
        let direct: &[Area] = if hidden {
            &[]
        } else if covered {
            &COVERED_DIRECT
        } else {
            &DIRECT
        };
        // Every frame goes through Slint, which then also brings this buffer up to date
        // with what changed while the other one was shown.
        window.request_redraw();
        gate.run(Step::Slint, || {
            window.draw_if_needed(|renderer| {
                // Made in its own crate, built for speed (`ruzina-screen-lines`). Its lines
                // go through the strip and the PPA, off the CPU's writes to PSRAM.
                let stage = strip
                    .as_deref_mut()
                    .map(|pixels| ruzina_screen_lines::Stage {
                        pixels,
                        copy: copy_block,
                    });
                let lines =
                    ruzina_screen_lines::render(renderer, &mut *buffer, direct, now_us, stage);
                drawn_areas = Some((lines.calls, lines.pixels, lines.runs, lines.run_count));
                let mut flush_us = 0;
                if lines.bottom > lines.top {
                    let (top, bottom) = (lines.top, lines.bottom);
                    rows_drawn = bottom - top;
                    // What the PPA copied is in memory; what the CPU wrote is in cache.
                    if lines.cpu_wrote {
                        let flushing = now_us();
                        flush(buffer, top, bottom - top);
                        flush_us = (now_us() - flushing) as u32;
                    }
                }
                split = Some((lines.prepare_us, lines.lines_us, lines.copy_us, flush_us));
            })
        });
        let drawn = now_us();
        let slint_us = (drawn - started) as u32;
        slint_times.add(slint_us);
        if slint_us > SLOW_FRAME_US {
            // SAFETY: diagnostic telemetry passes three u32 values only.
            unsafe {
                ruzina_bridge_load_report(rows_drawn as u32, 23, slint_us);
                ruzina_bridge_load_report(tab as u32, 24, (started - updates_started) as u32);
                if let Some((calls, pixels, runs, count)) = drawn_areas {
                    for &(first, last, start, end) in runs.iter().take(count) {
                        ruzina_bridge_load_report(
                            u32::from(first) << 16 | u32::from(last),
                            25,
                            u32::from(start) << 16 | u32::from(end),
                        );
                    }
                    ruzina_bridge_load_report(calls, 26, pixels);
                }
                // Where the frame's time went: the scene's preparation and Slint's lines,
                // then the copy into the frame buffer and its write-back for the panel.
                if let Some((prepare, lines, copy, flush)) = split {
                    ruzina_bridge_load_report(prepare, 27, lines);
                    ruzina_bridge_load_report(copy, 28, flush);
                }
                // The draw's allocations and frees, and the microseconds they took.
                let (calls, cycles_8) = crate::idf_alloc::counted();
                let calls = calls.wrapping_sub(allocs_before.0);
                // 8-cycle units at the 360 MHz CPU clock.
                let us = u64::from(cycles_8.wrapping_sub(allocs_before.1)) * 8 / 360;
                ruzina_bridge_load_report(calls, 29, us as u32);
            }
        }
        for deck in 0..2 {
            let track = tracks[deck].as_deref();
            // New marks redraw the whole lane, once in each buffer.
            let (revision, marks) = deck_view::MARKS[deck].load();
            if marks_at[drawing][deck] != Some(revision) {
                lane_at[drawing][deck] = None;
                marks_at[drawing][deck] = Some(revision);
            }
            let wave = wave(
                track,
                deck_view::window_ms(decks[deck].bpm_x100),
                marks,
                decks[deck].position_ms,
            );
            let left = wave.track_pixel(decks[deck].position_ms) - PLAYHEAD_X as i64;
            // Under the pages only the overviews are Rust's.
            if !covered && meter_at[drawing][deck] != Some(meters[deck]) {
                draw_meter(buffer, METERS[deck], meters[deck]);
                meter_at[drawing][deck] = Some(meters[deck]);
            }
            // After a zoom the shown lane is at another scale: draw it all.
            let at_scale = |at: Option<(i64, i64)>| {
                at.filter(|&(_, window_ms)| window_ms == wave.window_ms)
                    .map(|(left, _)| left)
            };
            // A paused or empty deck's lane is drawn once into each buffer, then left.
            if !covered && at_scale(lane_at[drawing][deck]) != Some(left) {
                // The shown lane is copied only when it has the same marks.
                let shown_left = at_scale(lane_at[1 - drawing][deck])
                    .filter(|_| marks_at[1 - drawing][deck] == Some(revision));
                draw_lane(
                    &mut gate,
                    strip.as_deref_mut(),
                    buffer,
                    shown_left.map(|shown_left| (shown, shown_left)),
                    LANES[deck],
                    wave,
                    left,
                );
                lane_at[drawing][deck] = Some((left, wave.window_ms));
            }
            let index = track.map_or(u32::MAX, |track| track.index);
            let duration = track.map_or(0, |track| track.duration_ms);
            let playhead = (duration != 0).then(|| {
                (u64::from(decks[deck].position_ms.min(duration)) * (MINIS[deck].width as u64 - 1)
                    / u64::from(duration)) as usize
            });
            let mini = Some((index, playhead));
            if !hidden && mini_at[drawing][deck] != mini {
                let view = track
                    .and_then(|track| track.analysis.as_ref())
                    .map(|analysis| analysis.view());
                let preview = view.as_ref().map_or(&[][..], |view| view.preview());
                let memory = view.as_ref().map_or(&[][..], |view| view.memory_cues());
                draw_mini(buffer, MINIS[deck], preview, playhead, memory, duration);
                mini_at[drawing][deck] = mini;
            }
        }
        wave_times.add((now_us() - drawn) as u32);
        gate.frame_done();
        mark(SCREEN, false);
        // The screen runs above the decoding client on its core. After a long frame (a
        // page full of text) it steps aside for two refreshes, so a run of them cannot
        // hold the decoders off: on s97 such a run kept them out for 250 ms, and a deck
        // that had just loaded ran dry.
        if slint_us > SLOW_FRAME_US {
            for _ in 0..2 {
                // SAFETY: waits on the panel's refresh.
                unsafe { ruzina_panel_wait_refresh() };
            }
        }
        // The refresh this frame goes on the panel at; a frame that ran late shows at the
        // next one.
        // SAFETY: reads a counter and waits on the panel's refresh.
        let now = unsafe { ruzina_panel_refreshes() };
        if now >= target {
            target = now + 1;
        }
        while unsafe { ruzina_panel_refreshes() } + 1 < target {
            unsafe { ruzina_panel_wait_refresh() };
        }
        // SAFETY: the buffer is one of the panel's two; C waits for the swap.
        unsafe { ruzina_panel_present(buffer.as_mut_ptr()) };
        target += REFRESHES_PER_FRAME;
        drawing = 1 - drawing;
    }
}
