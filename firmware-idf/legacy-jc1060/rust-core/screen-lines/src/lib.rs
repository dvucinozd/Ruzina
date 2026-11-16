//! The deck screen's Slint draw, line by line.
//!
//! Slint's software renderer is generic over the line buffer and the pixel, so its code is
//! made in the crate that calls it, at that crate's optimisation. The firmware is built
//! for size (`opt-level = "s"`); this crate holds the one call, `render`, and is built for
//! speed. The same frame then draws in about 60 % of the time on the host, pixel for
//! pixel the same.
#![no_std]

use slint::platform::software_renderer::{LineBufferProvider, Rgb565Pixel, SoftwareRenderer};

/// The panel's width in pixels.
pub const WIDTH: usize = 1024;

/// The areas a slow frame logs.
pub const RUNS: usize = 6;

/// A rectangle Rust draws, which Slint must never write over.
#[derive(Clone, Copy)]
pub struct Area {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

/// What a draw changed: the rows from `top` to `bottom`, and for the slow-frame log, the
/// lines Slint drew, their pixels, and the first areas: runs of consecutive lines with
/// the same columns (first and last line, first and last column).
pub struct Drawn {
    pub top: usize,
    pub bottom: usize,
    pub calls: u32,
    pub pixels: u32,
    pub runs: [(u16, u16, u16, u16); RUNS],
    pub run_count: usize,
    /// Microseconds from the call to Slint's first line: its preparation of the scene.
    pub prepare_us: u32,
    /// Microseconds Slint spent drawing lines into the line buffer.
    pub lines_us: u32,
    /// Microseconds spent copying finished lines into `frame`.
    pub copy_us: u32,
}

/// Draws what changed into `frame`, `WIDTH` pixels a row, around the `direct` rectangles.
///
/// `frame` is the buffer the panel is not showing. Slint fills every pixel of each range
/// it draws, background first, so a line no `direct` rectangle crosses is drawn straight
/// into `frame`. A line one does cross is drawn in internal RAM, and only the parts
/// outside the rectangles are copied out.
///
/// `clock` gives microseconds; it times the parts of the draw for the slow-frame log.
#[inline(never)]
pub fn render(
    renderer: &SoftwareRenderer,
    frame: &mut [u16],
    direct: &[Area],
    clock: fn() -> u64,
) -> Drawn {
    let started = clock();
    let mut lines = Lines {
        frame,
        direct,
        clock,
        started,
        drawn: Drawn {
            top: usize::MAX,
            bottom: 0,
            calls: 0,
            pixels: 0,
            runs: [(0, 0, 0, 0); RUNS],
            run_count: 0,
            prepare_us: 0,
            lines_us: 0,
            copy_us: 0,
        },
    };
    renderer.render_by_line(&mut lines);
    if lines.drawn.calls == 0 {
        lines.drawn.prepare_us = (clock() - started) as u32;
    }
    lines.drawn
}

struct Lines<'a> {
    frame: &'a mut [u16],
    direct: &'a [Area],
    clock: fn() -> u64,
    started: u64,
    drawn: Drawn,
}

impl Drawn {
    fn note(&mut self, line: usize, range: &core::ops::Range<usize>) {
        self.calls += 1;
        self.pixels += range.len() as u32;
        self.top = self.top.min(line);
        self.bottom = self.bottom.max(line + 1);
        let (line, start, end) = (line as u16, range.start as u16, range.end as u16);
        if let Some(run) = self
            .run_count
            .checked_sub(1)
            .and_then(|i| self.runs.get_mut(i))
            && run.2 == start
            && run.3 == end
            && run.1 + 1 == line
        {
            run.1 = line;
            return;
        }
        if let Some(run) = self.runs.get_mut(self.run_count) {
            *run = (line, line, start, end);
        }
        self.run_count += 1;
    }
}

impl LineBufferProvider for &mut Lines<'_> {
    type TargetPixel = Rgb565Pixel;

    fn process_line(
        &mut self,
        line: usize,
        range: core::ops::Range<usize>,
        render_fn: impl FnOnce(&mut [Rgb565Pixel]),
    ) {
        let before = (self.clock)();
        if self.drawn.calls == 0 {
            self.drawn.prepare_us = (before - self.started) as u32;
        }
        self.drawn.note(line, &range);
        let row = &mut self.frame[line * WIDTH..(line + 1) * WIDTH];
        let crossed = self.direct.iter().any(|area| {
            (area.y..area.y + area.height).contains(&line)
                && area.x < range.end
                && area.x + area.width > range.start
        });
        if !crossed {
            render_fn(pixels_of(&mut row[range]));
            self.drawn.lines_us += ((self.clock)() - before) as u32;
            return;
        }
        let mut pixels = [Rgb565Pixel(0); WIDTH];
        render_fn(&mut pixels[range.clone()]);
        let drawn = (self.clock)();
        self.drawn.lines_us += (drawn - before) as u32;
        let pixels = pixels_of_ref(&pixels);
        outside(self.direct, line, range, |span| {
            row[span.clone()].copy_from_slice(&pixels[span]);
        });
        self.drawn.copy_us += ((self.clock)() - drawn) as u32;
    }
}

/// `pixels` as Slint's pixel type.
fn pixels_of(pixels: &mut [u16]) -> &mut [Rgb565Pixel] {
    // SAFETY: `Rgb565Pixel` is `#[repr(transparent)]` over `u16`, so the slice has the same
    // length, alignment and layout; the borrow is moved, not copied.
    unsafe { core::slice::from_raw_parts_mut(pixels.as_mut_ptr().cast(), pixels.len()) }
}

/// Slint's pixels as `u16`, the frame buffer's type.
fn pixels_of_ref(pixels: &[Rgb565Pixel]) -> &[u16] {
    // SAFETY: as in `pixels_of`, the other way round.
    unsafe { core::slice::from_raw_parts(pixels.as_ptr().cast(), pixels.len()) }
}

/// Calls `f` for each part of `range` on `line` outside the rectangles Rust draws.
fn outside(
    direct: &[Area],
    line: usize,
    range: core::ops::Range<usize>,
    mut f: impl FnMut(core::ops::Range<usize>),
) {
    let mut x = range.start;
    while x < range.end {
        // The first direct rectangle on this line at or after `x`.
        let next = direct
            .iter()
            .filter(|area| {
                (area.y..area.y + area.height).contains(&line) && area.x + area.width > x
            })
            .min_by_key(|area| area.x);
        let end = next.map_or(range.end, |area| area.x.clamp(x, range.end));
        if end > x {
            f(x..end);
        }
        x = next.map_or(range.end, |area| (area.x + area.width).max(end));
    }
}
