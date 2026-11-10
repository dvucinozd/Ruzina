//! The deck screen on the panel: Slint for everything that changes rarely, and direct
//! drawing for the waveforms, which change many times a second.
//!
//! The screen shows demo data for now, not the decks. It logs draw times as load
//! events: 21 the Slint parts (worst, then 31 the average), 22 a waveform frame (32).
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use slint::platform::software_renderer::{
    LineBufferProvider, MinimalSoftwareWindow, RepaintBufferType, Rgb565Pixel,
};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, PhysicalSize};

slint::include_modules!();

const WIDTH: usize = 1024;
const HEIGHT: usize = 600;
const REPORT_MS: u64 = 20_000;

/// A rectangle Rust draws, which Slint must never write over. The numbers match the
/// dark rectangles in `overview.slint`.
#[derive(Clone, Copy)]
struct Area {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

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
const DIRECT: [Area; 4] = [LANES[0], LANES[1], MINIS[0], MINIS[1]];

const BACKGROUND: u16 = rgb(0x02, 0x04, 0x06);
const BEAT_LINE: u16 = rgb(0x30, 0x34, 0x3b);
const BAR_LINE: u16 = rgb(0x6e, 0x72, 0x78);
const PLAYHEAD: u16 = rgb(0x00, 0xff, 0x00);
const PLAYHEAD_X: usize = LANE_WIDTH / 2;
/// Waveform columns per panel refresh: about 122 a second, 7.2 s across the lane. The
/// position follows the refresh count, so every frame moves by whole, equal steps.
const COLUMNS_PER_REFRESH: usize = 2;
/// Refreshes per frame: 30 frames a second, each shown exactly two refreshes after the
/// last, so every frame moves the waveforms by the same step.
const REFRESHES_PER_FRAME: u32 = 2;
/// Columns per beat at 120 BPM with the scroll above.
const COLUMNS_PER_BEAT: usize = 50;

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
    fn ruzina_bridge_load_report(pass: u32, event: u32, value: u32);
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

/// Renders each line in internal RAM, then copies it out whole, around the rectangles
/// Rust draws. Only finished pixels reach the framebuffer the panel is scanning, so a
/// redraw cannot flicker.
struct Lines<'a> {
    frame: &'a mut [u16],
    top: usize,
    bottom: usize,
}

impl LineBufferProvider for &mut Lines<'_> {
    type TargetPixel = Rgb565Pixel;

    fn process_line(
        &mut self,
        line: usize,
        range: core::ops::Range<usize>,
        render_fn: impl FnOnce(&mut [Rgb565Pixel]),
    ) {
        let mut pixels = [Rgb565Pixel(0); WIDTH];
        render_fn(&mut pixels[range.clone()]);
        let row = &mut self.frame[line * WIDTH..(line + 1) * WIDTH];
        let mut x = range.start;
        while x < range.end {
            // The first direct rectangle on this line at or after `x`.
            let next = DIRECT
                .iter()
                .filter(|area| {
                    (area.y..area.y + area.height).contains(&line) && area.x + area.width > x
                })
                .min_by_key(|area| area.x);
            let end = next.map_or(range.end, |area| area.x.clamp(x, range.end));
            for (out, pixel) in row[x..end].iter_mut().zip(&pixels[x..end]) {
                *out = pixel.0;
            }
            x = next.map_or(range.end, |area| (area.x + area.width).max(end));
        }
        self.top = self.top.min(line);
        self.bottom = self.bottom.max(line + 1);
    }
}

/// A demo track: per column its half height and colour, as an analysis would give.
fn demo_track(seed: u32) -> Vec<(u8, u16)> {
    const WAVE: [u16; 4] = [
        rgb(0x20, 0xd8, 0xf0),
        rgb(0xf0, 0x80, 0xc0),
        rgb(0xf0, 0xe0, 0x60),
        rgb(0xe0, 0xf8, 0xff),
    ];
    let mut random = seed | 1;
    (0..16_384u32)
        .map(|column| {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            let beat = column as usize % COLUMNS_PER_BEAT;
            let envelope = 100u32.saturating_sub(beat as u32 * 4);
            let half = (envelope * (40 + random % 60) / 100).max(2) as u8;
            let color = if beat < 3 {
                WAVE[3]
            } else {
                WAVE[(random >> 8) as usize % 3]
            };
            (half, color)
        })
        .collect()
}

/// Draws columns `from..to` of the lane at `position`. Each row is built in internal RAM
/// and written in one copy; rows mirror below the middle.
fn draw_columns(
    frame: &mut [u16],
    area: Area,
    track: &[(u8, u16)],
    position: usize,
    columns: core::ops::Range<usize>,
) {
    let width = columns.len();
    if width == 0 {
        return;
    }
    let mut half = [0u8; LANE_WIDTH];
    let mut color = [BACKGROUND; LANE_WIDTH];
    let mut back = [BACKGROUND; LANE_WIDTH];
    for (i, x) in columns.clone().enumerate() {
        let at = position + x;
        let (height, ink) = track[at % track.len()];
        (half[i], color[i], back[i]) = if (PLAYHEAD_X - 1..=PLAYHEAD_X + 1).contains(&x) {
            (0, PLAYHEAD, PLAYHEAD)
        } else if at.is_multiple_of(COLUMNS_PER_BEAT * 4) {
            (height, ink, BAR_LINE)
        } else if at.is_multiple_of(COLUMNS_PER_BEAT) {
            (height, ink, BEAT_LINE)
        } else {
            (height, ink, BACKGROUND)
        };
    }
    let middle = LANE_HEIGHT / 2;
    let mut line = [BACKGROUND; LANE_WIDTH];
    for distance in 0..=middle {
        let reach = distance as u8;
        for (pixel, ((&height, &ink), &ground)) in line[..width]
            .iter_mut()
            .zip(half.iter().zip(&color).zip(&back))
        {
            *pixel = if reach <= height { ink } else { ground };
        }
        for row in [middle - distance, middle + distance] {
            if row < LANE_HEIGHT {
                let start = (area.y + row) * WIDTH + area.x + columns.start;
                frame[start..start + width].copy_from_slice(&line[..width]);
            }
        }
    }
}

/// Brings the lane in `back` to `position`. When `shown` holds it at an earlier position,
/// the hardware moves those pixels left and the CPU draws only the new columns on the
/// right and the playhead; otherwise the CPU draws the whole lane.
fn draw_lane(
    back: &mut [u16],
    shown: &[u16],
    shown_at: Option<usize>,
    area: Area,
    track: &[(u8, u16)],
    position: usize,
) {
    let step = shown_at.and_then(|at| position.checked_sub(at));
    let moved = step.is_some_and(|dx| {
        dx > 0
            && dx < LANE_WIDTH / 2
            // SAFETY: both are the panel's frame buffers; C bounds the block to them.
            && unsafe {
                ruzina_panel_move(
                    shown.as_ptr(),
                    back.as_mut_ptr(),
                    area.x as u32,
                    area.y as u32,
                    LANE_WIDTH as u32,
                    LANE_HEIGHT as u32,
                    dx as u32,
                )
            } == 0
    });
    match (moved, step) {
        (true, Some(dx)) => {
            draw_columns(back, area, track, position, LANE_WIDTH - dx..LANE_WIDTH);
            // The old playhead moved left with the rest.
            draw_columns(
                back,
                area,
                track,
                position,
                PLAYHEAD_X - 1 - dx..PLAYHEAD_X + 2,
            );
        }
        (false, Some(0)) => {
            draw_columns(back, area, track, position, PLAYHEAD_X - 1..PLAYHEAD_X + 2)
        }
        _ => draw_columns(back, area, track, position, 0..LANE_WIDTH),
    }
    flush(back, area.y, LANE_HEIGHT);
}

/// Draws the whole-track overview once.
fn draw_mini(frame: &mut [u16], area: Area, track: &[(u8, u16)]) {
    let middle = area.height / 2;
    let step = track.len() / area.width;
    for row in 0..area.height {
        let distance = row.abs_diff(middle) as u32;
        let start = (area.y + row) * WIDTH + area.x;
        for x in 0..area.width {
            let (half, color) = track[x * step];
            let height = u32::from(half) * middle as u32 / 100;
            frame[start + x] = if distance <= height {
                color
            } else {
                BACKGROUND
            };
        }
    }
    flush(frame, area.y, area.height);
}

/// Writes `rows` rows of the buffer `frame` back from cache, for the panel's DMA.
fn flush(frame: &mut [u16], y: usize, rows: usize) {
    // SAFETY: `frame` is one of the two frame buffers; C bounds the rows to it.
    unsafe { ruzina_panel_flush(frame.as_mut_ptr(), y as u32, rows as u32) };
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

fn clock(ms: u64) -> slint::SharedString {
    let seconds = ms / 1_000;
    slint::format!("{:02}:{:02}", seconds / 60 % 60, seconds % 60)
}

/// Runs the UI on this task. It draws each frame into the buffer the panel does not
/// show, then swaps the two at the end of a refresh.
///
/// # Safety
/// `shown` and `back` must be the panel's two `1024 * 600` RGB565 frame buffers, `shown`
/// the one on the panel, written by no other code.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_slint_run(shown: *mut u16, back: *mut u16) {
    // Slint redraws in each buffer what changed since that buffer was last shown.
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
    ui.set_title_1("Deck 1 demo track".into());
    ui.set_title_2("Deck 2 demo track".into());
    ui.set_bpm_1("128.00".into());
    ui.set_bpm_2("126.00".into());
    ui.set_pitch_1("+0.00%".into());
    ui.set_pitch_2("-1.56%".into());
    ui.set_key_1("8A".into());
    ui.set_key_2("9B".into());
    ui.set_status_1("MASTER".into());
    ui.set_status_2("SYNC".into());
    ui.set_master_tempo_2(true);
    ui.set_playing_1(true);
    ui.set_playing_2(true);
    ui.set_level_1(8);
    ui.set_level_2(6);
    ui.set_fx_effect("FILTER".into());
    ui.set_fx_beat("1".into());
    ui.set_fx_depth(50);
    if ui.show().is_err() {
        return;
    }
    // SAFETY: the caller hands over both frame buffers.
    let mut buffers = unsafe {
        [
            core::slice::from_raw_parts_mut(shown, WIDTH * HEIGHT),
            core::slice::from_raw_parts_mut(back, WIDTH * HEIGHT),
        ]
    };
    let tracks = [demo_track(0x2f6e_2b1d), demo_track(0x9c3a_11f7)];
    for buffer in buffers.iter_mut() {
        for (area, track) in MINIS.iter().zip(&tracks) {
            draw_mini(buffer, *area, track);
        }
    }
    let (mut slint_times, mut wave_times) = (Times::default(), Times::default());
    let start = now_us() / 1_000;
    let mut next_report = REPORT_MS;
    let mut drawing = 1;
    // SAFETY: reads a counter.
    let mut target = unsafe { ruzina_panel_refreshes() } + REFRESHES_PER_FRAME;
    // Where each buffer's lanes were last drawn.
    let mut drawn_at: [[Option<usize>; 2]; 2] = [[None; 2]; 2];
    loop {
        let ms = now_us() / 1_000 - start;
        if ms >= next_report {
            slint_times.report(21);
            wave_times.report(22);
            next_report = ms + REPORT_MS;
        }
        ui.set_elapsed_1(clock(ms));
        ui.set_elapsed_2(clock(ms + 37_000));
        ui.set_remaining_1(clock(300_000u64.saturating_sub(ms % 300_000)));
        ui.set_remaining_2(clock(240_000u64.saturating_sub(ms % 240_000)));
        slint::platform::update_timers_and_animations();
        let [first, second] = &mut buffers;
        let (buffer, shown) = if drawing == 0 {
            (&mut **first, &**second)
        } else {
            (&mut **second, &**first)
        };
        let started = now_us();
        // Every frame goes through Slint, which then also brings this buffer up to date
        // with what changed while the other one was shown.
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            let mut lines = Lines {
                frame: &mut *buffer,
                top: HEIGHT,
                bottom: 0,
            };
            renderer.render_by_line(&mut lines);
            if lines.bottom > lines.top {
                let (top, bottom) = (lines.top, lines.bottom);
                flush(buffer, top, bottom - top);
            }
        });
        let drawn = now_us();
        slint_times.add((drawn - started) as u32);
        // The refresh this frame is drawn for. A frame that ran late shows at the next
        // one in the cadence instead.
        // SAFETY: reads a counter.
        let now = unsafe { ruzina_panel_refreshes() };
        if now >= target {
            target = now + 1;
        }
        let position = target as usize * COLUMNS_PER_REFRESH;
        for (index, area) in LANES.iter().enumerate() {
            let at = position + index * 37;
            let shown_at = drawn_at[1 - drawing][index];
            draw_lane(buffer, shown, shown_at, *area, &tracks[index], at);
            drawn_at[drawing][index] = Some(at);
        }
        wave_times.add((now_us() - drawn) as u32);
        // Hand the frame over during the refresh before its own, so it goes on the panel
        // exactly at `target`.
        // SAFETY: reads a counter and waits on the panel's refresh.
        while unsafe { ruzina_panel_refreshes() } + 1 < target {
            unsafe { ruzina_panel_wait_refresh() };
        }
        // SAFETY: the buffer is one of the panel's two; C waits for the swap.
        unsafe { ruzina_panel_present(buffer.as_mut_ptr()) };
        target += REFRESHES_PER_FRAME;
        drawing = 1 - drawing;
    }
}
