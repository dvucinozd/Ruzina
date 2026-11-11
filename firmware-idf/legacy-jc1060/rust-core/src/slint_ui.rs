//! The deck screen on the panel: Slint for everything that changes rarely, and direct
//! drawing for the waveforms, which change many times a second.
//!
//! It shows the decks: what the engine publishes in `deck_view::LIVE`, and each
//! loaded track's title, key and analysis. It logs draw times as load events: 21 the
//! Slint parts (worst, then 31 the average), 22 a waveform frame (32).
use crate::deck_view::{self, DeckSnapshot, FxSnapshot, LoadedTrack, Marks};
use alloc::boxed::Box;
use alloc::rc::Rc;
use ruzina_track_analysis::{Beat, WaveformColumn};
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
const PANEL: u16 = rgb(0x0d, 0x0f, 0x11);
const METER_OFF: u16 = rgb(0x1c, 0x22, 0x28);
const METER_GREEN: u16 = rgb(0x6e, 0xe1, 0x28);
const METER_AMBER: u16 = rgb(0xeb, 0x87, 0x0f);
const METER_RED: u16 = rgb(0xff, 0x17, 0x44);

const BACKGROUND: u16 = rgb(0x02, 0x04, 0x06);
const BEAT_LINE: u16 = rgb(0x30, 0x34, 0x3b);
const BAR_LINE: u16 = rgb(0x6e, 0x72, 0x78);
const PLAYHEAD: u16 = rgb(0x00, 0xff, 0x00);
const PLAYHEAD_X: usize = LANE_WIDTH / 2;
/// Refreshes per frame: 30 frames a second, each shown exactly two refreshes after the
/// last.
const REFRESHES_PER_FRAME: u32 = 2;
/// Half the mini overview's height, for a full-scale column.
const MINI_HALF: u32 = 21;
const PLAYED_SHADE: u16 = rgb(0x30, 0x30, 0x30);
const MINI_PLAYHEAD: u16 = rgb(0xff, 0xff, 0xff);
/// The donor's waveform palette, picked by a column's colour hint and loudness.
const PINK: u16 = rgb(0xff, 0x2e, 0x6e);
const BLUE: u16 = rgb(0x3a, 0x7b, 0xff);
const CYAN: u16 = rgb(0x26, 0xe0, 0xff);
const GREEN: u16 = rgb(0x38, 0xf5, 0x8c);
const AMBER: u16 = rgb(0xff, 0xb7, 0x33);
const PURPLE: u16 = rgb(0xb5, 0x7c, 0xff);
/// The red triangle on each bar line, for beat matching: its base at the lane's edge
/// towards the other deck, pointing into the lane.
const BAR_CAP: u16 = rgb(0xff, 0x17, 0x44);
const BAR_CAP_REACH: i64 = 6;
const BAR_CAP_HEIGHT: usize = 8;
/// Hot Cues A to H: a line through the lane and a flag at the edge away from the bar
/// triangles.
const CUE_COLORS: [u16; 8] = [
    rgb(0x28, 0xe2, 0x14),
    rgb(0x30, 0xd2, 0xe8),
    rgb(0x30, 0x5a, 0xff),
    rgb(0xaa, 0x72, 0xff),
    rgb(0xff, 0x12, 0x7b),
    rgb(0xff, 0x8c, 0x00),
    rgb(0xe0, 0xff, 0x00),
    rgb(0x10, 0xb1, 0x76),
];
const CUE_FLAG_WIDTH: i64 = 7;
const CUE_FLAG_HEIGHT: usize = 7;
/// The running loop: its ends as lines, and its span shaded behind the waveform.
const LOOP_EDGE: u16 = rgb(0xff, 0x8c, 0x00);
const LOOP_SHADE: u16 = rgb(0x30, 0x1c, 0x00);
/// The white edge on loud peaks.
const TIP: u16 = rgb(0xff, 0xff, 0xff);
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
    fn ruzina_bridge_load_report(pass: u32, event: u32, value: u32);
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

/// A loaded track's waveform as the lane needs it.
#[derive(Clone, Copy)]
struct Wave<'a> {
    detail: &'a [WaveformColumn],
    beats: &'a [Beat],
    rate_hz: u16,
    /// The time the lane shows.
    window_ms: i64,
    marks: Marks,
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
            _ if self
                .marks
                .looped
                .is_some_and(|(from, to)| (from..to).contains(&start)) =>
            {
                LOOP_SHADE
            }
            _ => BACKGROUND,
        }
    }

    /// Whether `ms` falls in track pixel `at`.
    fn holds(&self, at: i64, ms: u32) -> bool {
        at >= 0 && (self.pixel_ms(at)..self.pixel_ms(at + 1)).contains(&i64::from(ms))
    }

    /// The Hot Cue at track pixel `at`, by pad.
    fn cue(&self, at: i64) -> Option<usize> {
        (0..8).find(|&pad| self.marks.cues[pad].is_some_and(|ms| self.holds(at, ms)))
    }

    /// The colour of a mark through the whole of track pixel `at`: a Hot Cue, else an
    /// end of the running loop.
    fn mark(&self, at: i64) -> Option<u16> {
        if let Some(pad) = self.cue(at) {
            return Some(CUE_COLORS[pad]);
        }
        let (start, end) = self.marks.looped?;
        (self.holds(at, start) || self.holds(at, end)).then_some(LOOP_EDGE)
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
        return (loudness, column.color_rgb565);
    }
    let color = match hint {
        1 => PINK,
        2 => BLUE,
        3 | 7 => CYAN,
        4 => GREEN,
        5 => AMBER,
        6 => PURPLE,
        _ => match loudness {
            28.. => CYAN,
            22.. => PINK,
            14.. => BLUE,
            _ => PURPLE,
        },
    };
    (loudness, color)
}

/// The lane's view of `track` with `marks`, showing `window_ms` of it.
fn wave(track: Option<&LoadedTrack>, window_ms: u32, marks: Marks) -> Wave<'_> {
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
        };
    };
    Wave {
        detail: view.detail(),
        beats: view.beat_grid().map_or(&[], |grid| grid.beats()),
        rate_hz: view.detail_rate_hz(),
        window_ms,
        marks,
    }
}

/// Draws columns `columns` of the lane whose left edge shows detail column `left`. Each
/// row is built in internal RAM and written in one copy; rows mirror below the middle.
fn draw_columns(
    frame: &mut [u16],
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
                let start = (area.y + row) * WIDTH + area.x + columns.start;
                frame[start..start + width].copy_from_slice(&line[..width]);
            }
        }
    }
    draw_bar_caps(frame, area, wave, left, columns.clone());
    draw_cue_flags(frame, area, wave, left, columns);
}

/// Draws the flags of the Hot Cues near columns `columns`, clipped to them and clear of
/// the playhead: a square right of the cue's line, at the edge away from the bar
/// triangles (the top of deck 1's lane, the bottom of deck 2's).
fn draw_cue_flags(
    frame: &mut [u16],
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
            let start = (area.y + y) * WIDTH + area.x;
            for x in line.max(first)..(line + CUE_FLAG_WIDTH).min(end) {
                let x = x as usize;
                if !(PLAYHEAD_X - 1..=PLAYHEAD_X + 1).contains(&x) {
                    frame[start + x] = CUE_COLORS[pad];
                }
            }
        }
    }
}

/// Draws the red triangles of the bar lines near columns `columns`, clipped to them and
/// clear of the playhead. Deck 1's lane has them at the bottom, deck 2's at the top.
fn draw_bar_caps(
    frame: &mut [u16],
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
            let line = (area.y + y) * WIDTH + area.x;
            for x in (centre - reach).max(first)..=(centre + reach).min(end - 1) {
                let x = x as usize;
                if !(PLAYHEAD_X - 1..=PLAYHEAD_X + 1).contains(&x) {
                    frame[line + x] = BAR_CAP;
                }
            }
        }
    }
}

/// Brings the lane in `back` to show track pixel `left` at its left edge. When `shown`
/// holds it at most half a lane earlier, the hardware copies those pixels, moved left,
/// and the CPU draws only the new columns and the playhead; otherwise the CPU draws the
/// whole lane. `back` holds the frame before `shown`, so it is never left as it was.
fn draw_lane(
    back: &mut [u16],
    shown: &[u16],
    shown_left: Option<i64>,
    area: Area,
    wave: Wave<'_>,
    left: i64,
) {
    let step = shown_left.map(|shown_left| left - shown_left);
    let moved = step.is_some_and(|dx| {
        dx >= 0
            && dx < LANE_WIDTH as i64 / 2
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
        // A paused deck: the copy is the whole frame.
        (true, Some(0)) => {}
        (true, Some(dx)) => {
            let dx = dx as usize;
            draw_columns(back, area, wave, left, LANE_WIDTH - dx..LANE_WIDTH);
            // The old playhead moved left with the rest.
            draw_columns(back, area, wave, left, PLAYHEAD_X - 1 - dx..PLAYHEAD_X + 2);
        }
        _ => draw_columns(back, area, wave, left, 0..LANE_WIDTH),
    }
    flush(back, area.y, LANE_HEIGHT);
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

/// Draws the whole-track overview, shaded up to the playhead at `x`.
fn draw_mini(frame: &mut [u16], area: Area, preview: &[WaveformColumn], x: Option<usize>) {
    let middle = area.height / 2;
    let mut heights = [0u32; 512];
    let mut colors = [BACKGROUND; 512];
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
        let start = (area.y + row) * WIDTH + area.x;
        for column in 0..area.width {
            frame[start + column] = if x == Some(column) {
                MINI_PLAYHEAD
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
    if ui.show().is_err() {
        return;
    }
    ui.on_play(|deck| {
        deck_view::TOUCH.push(deck as usize & 1, deck_view::TouchControl::Play);
    });
    ui.on_cue(|deck, down| {
        deck_view::TOUCH.push(deck as usize & 1, deck_view::TouchControl::Cue(down));
    });
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
    // SAFETY: reads a counter.
    let mut target = unsafe { ruzina_panel_refreshes() } + REFRESHES_PER_FRAME;
    loop {
        let ms = now_us() / 1_000 - start;
        if ms >= next_report {
            slint_times.report(21);
            wave_times.report(22);
            next_report = ms + REPORT_MS;
        }
        touch(ui.window(), &mut touched, &mut touch_errors);
        let decks = [deck_view::LIVE[0].load(), deck_view::LIVE[1].load()];
        for deck in 0..2 {
            if let Some(track) = deck_view::take_track(deck) {
                tracks[deck] = Some(track);
                lane_at[0][deck] = None;
                lane_at[1][deck] = None;
            }
            if !decks[deck].has(deck_view::LOADED) && tracks[deck].is_some() {
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
        for deck in 0..2 {
            let track = tracks[deck].as_deref();
            // New marks redraw the whole lane, once in each buffer.
            let (revision, marks) = deck_view::MARKS[deck].load();
            if marks_at[drawing][deck] != Some(revision) {
                lane_at[drawing][deck] = None;
                marks_at[drawing][deck] = Some(revision);
            }
            let wave = wave(track, deck_view::window_ms(decks[deck].bpm_x100), marks);
            let left = wave.track_pixel(decks[deck].position_ms) - PLAYHEAD_X as i64;
            if meter_at[drawing][deck] != Some(meters[deck]) {
                draw_meter(buffer, METERS[deck], meters[deck]);
                meter_at[drawing][deck] = Some(meters[deck]);
            }
            // After a zoom the shown lane is at another scale: draw it all.
            let at_scale = |at: Option<(i64, i64)>| {
                at.filter(|&(_, window_ms)| window_ms == wave.window_ms)
                    .map(|(left, _)| left)
            };
            // A paused or empty deck's lane is drawn once into each buffer, then left.
            if at_scale(lane_at[drawing][deck]) != Some(left) {
                // The shown lane is copied only when it has the same marks.
                let shown_left = at_scale(lane_at[1 - drawing][deck])
                    .filter(|_| marks_at[1 - drawing][deck] == Some(revision));
                draw_lane(buffer, shown, shown_left, LANES[deck], wave, left);
                lane_at[drawing][deck] = Some((left, wave.window_ms));
            }
            let index = track.map_or(u32::MAX, |track| track.index);
            let duration = track.map_or(0, |track| track.duration_ms);
            let playhead = (duration != 0).then(|| {
                (u64::from(decks[deck].position_ms.min(duration)) * (MINIS[deck].width as u64 - 1)
                    / u64::from(duration)) as usize
            });
            let mini = Some((index, playhead));
            if mini_at[drawing][deck] != mini {
                let preview = track
                    .and_then(|track| track.analysis.as_ref())
                    .map_or(&[][..], |analysis| analysis.view().preview());
                draw_mini(buffer, MINIS[deck], preview, playhead);
                mini_at[drawing][deck] = mini;
            }
        }
        wave_times.add((now_us() - drawn) as u32);
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
