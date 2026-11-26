//! The deck screen's Slint draw, line by line.
//!
//! Slint's software renderer is generic over the line buffer and the pixel, so its code is
//! made in the crate that calls it, at that crate's optimisation. The firmware is built
//! for size (`opt-level = "s"`); this crate holds the one call, `render`, and is built for
//! speed. The same frame then draws in about 60 % of the time on the host, pixel for
//! pixel the same.
#![no_std]

use core::ops::Range;
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
    /// Microseconds spent getting finished lines into `frame`.
    pub copy_us: u32,
    /// Whether the CPU wrote any of `frame`: those rows need writing back from cache for
    /// the panel. What a `Stage` copied is in memory already.
    pub cpu_wrote: bool,
}

/// A block of a stage copied into the frame: `width` by `height` pixels from column
/// `stage_x` of a stage `stage_width` pixels a row, to (`x`, `y`) of the frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Block {
    pub stage_width: usize,
    pub stage_x: usize,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

/// Internal RAM to draw lines in, and how its blocks get into the frame: the PPA on the
/// board, so the CPU does not write PSRAM while the USB audio fill runs on the other core.
///
/// A stage of two lines or more is used in two halves: while a band's blocks copy out of
/// one, the next band is drawn in the other.
pub struct Stage<'a> {
    pub pixels: &'a mut [u16],
    /// Begins copying a block, once the copy before it has ended; false when it did not
    /// begin. The copy reads the stage and writes the frame until `wait` returns.
    pub start: fn(&[u16], &mut [u16], Block) -> bool,
    /// Waits for the copy begun last; false when it did not end. True when none is under
    /// way.
    pub wait: fn() -> bool,
}

/// Draws what changed into `frame`, `WIDTH` pixels a row, around the `direct` rectangles.
///
/// `frame` is the buffer the panel is not showing. Slint fills every pixel of each range
/// it draws, background first. With a `stage`, lines are drawn there, in bands of
/// consecutive lines with the same columns, and each band's parts outside the
/// rectangles go to `frame` as blocks, the last one copying while the next band is drawn.
/// Without a stage, or once a block fails, the CPU writes `frame`: a line no rectangle
/// crosses straight, one that a rectangle crosses drawn in internal RAM and copied out
/// around it. Every copy has ended when `render` returns.
///
/// `clock` gives microseconds; it times the parts of the draw for the slow-frame log.
#[inline(never)]
pub fn render(
    renderer: &SoftwareRenderer,
    frame: &mut [u16],
    direct: &[Area],
    clock: fn() -> u64,
    stage: Option<Stage<'_>>,
) -> Drawn {
    let started = clock();
    let mut lines = Lines::new(frame, direct, clock, stage);
    renderer.render_by_line(&mut lines);
    lines.finish();
    if lines.drawn.calls == 0 {
        lines.drawn.prepare_us = (clock() - started) as u32;
    }
    lines.drawn
}

struct Lines<'a, 's> {
    frame: &'a mut [u16],
    direct: &'a [Area],
    clock: fn() -> u64,
    started: u64,
    drawn: Drawn,
    stage: Option<Stage<'s>>,
    /// The stage's halves (one or two), and the one the band is drawn in.
    halves: usize,
    half: usize,
    band: Band,
    /// The block copying now, and where its band starts in the stage: what the CPU copies
    /// if it does not end.
    copying: Option<(usize, Block)>,
}

/// Consecutive lines drawn in the stage, one after another, `range.len()` pixels each:
/// the same columns, crossed by the same rectangles (`crossing`).
struct Band {
    first: usize,
    lines: usize,
    range: Range<usize>,
    crossed: u32,
}

impl<'a, 's> Lines<'a, 's> {
    fn new(
        frame: &'a mut [u16],
        direct: &'a [Area],
        clock: fn() -> u64,
        stage: Option<Stage<'s>>,
    ) -> Self {
        // A band tells its lines apart by the rectangles crossing them, one bit each.
        let stage = stage.filter(|stage| direct.len() <= 32 && stage.pixels.len() >= WIDTH);
        let halves = match &stage {
            Some(stage) if stage.pixels.len() >= 2 * WIDTH => 2,
            _ => 1,
        };
        Self {
            frame,
            direct,
            clock,
            started: clock(),
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
                cpu_wrote: false,
            },
            stage,
            halves,
            half: 0,
            band: Band {
                first: 0,
                lines: 0,
                range: 0..0,
                crossed: 0,
            },
            copying: None,
        }
    }

    /// The pixels of one half of the stage.
    fn half_len(&self) -> usize {
        self.stage
            .as_ref()
            .map_or(0, |stage| stage.pixels.len() / self.halves)
    }

    /// Sends the band's blocks to the frame and moves to the other half; the last block is
    /// still copying when this returns.
    fn finish_band(&mut self) {
        if self.band.lines == 0 || self.stage.is_none() {
            return;
        }
        let started = (self.clock)();
        let base = self.half * self.half_len();
        let band = &self.band;
        let width = band.range.len();
        let mut blocks = [Block {
            stage_width: width,
            stage_x: 0,
            x: 0,
            y: band.first,
            width: 0,
            height: band.lines,
        }; 33];
        let mut count = 0;
        let mut add = |span: Range<usize>| {
            blocks[count].stage_x = span.start - band.range.start;
            blocks[count].x = span.start;
            blocks[count].width = span.len();
            count += 1;
        };
        if band.crossed == 0 {
            add(band.range.clone());
        } else {
            outside(self.direct, band.first, band.range.clone(), add);
        }
        for &block in &blocks[..count] {
            self.send(base, block);
        }
        self.band.lines = 0;
        self.half = (self.half + 1) % self.halves;
        // With one half, the next band is drawn where this one copies from.
        if self.halves == 1 {
            self.settle();
        }
        self.leave_if_failed();
        self.drawn.copy_us += ((self.clock)() - started) as u32;
    }

    /// Copies a block of the band that starts at `base` in the stage, once the block before
    /// it has ended. A block that does not begin or end goes by the CPU instead, and so
    /// does every block after it: a later copy would discard from the cache rows the CPU
    /// wrote.
    fn send(&mut self, base: usize, block: Block) {
        self.settle();
        let Some(stage) = self.stage.as_ref() else {
            return;
        };
        let pixels = &stage.pixels[base..base + block.height * block.stage_width];
        if !self.drawn.cpu_wrote && (stage.start)(pixels, self.frame, block) {
            self.copying = Some((base, block));
        } else {
            self.cpu_copy(base, block);
        }
    }

    /// Waits for the block copying, if any; one that does not end goes by the CPU.
    fn settle(&mut self) {
        let Some((base, block)) = self.copying.take() else {
            return;
        };
        if let Some(stage) = self.stage.as_ref()
            && !(stage.wait)()
        {
            self.cpu_copy(base, block);
        }
    }

    /// Copies a block from the stage by the CPU.
    fn cpu_copy(&mut self, base: usize, block: Block) {
        let Some(stage) = self.stage.as_ref() else {
            return;
        };
        for row in 0..block.height {
            let from = base + row * block.stage_width + block.stage_x;
            let to = (block.y + row) * WIDTH + block.x;
            self.frame[to..to + block.width]
                .copy_from_slice(&stage.pixels[from..from + block.width]);
        }
        self.drawn.cpu_wrote = true;
    }

    /// Once the CPU has copied a block, the lines after it are drawn in the frame too.
    fn leave_if_failed(&mut self) {
        if self.drawn.cpu_wrote && self.copying.is_none() {
            self.stage = None;
        }
    }

    /// Sends the last band and waits for its copy.
    fn finish(&mut self) {
        self.finish_band();
        let started = (self.clock)();
        self.settle();
        self.leave_if_failed();
        self.drawn.copy_us += ((self.clock)() - started) as u32;
    }
}

impl Drawn {
    fn note(&mut self, line: usize, range: &Range<usize>) {
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

impl LineBufferProvider for &mut Lines<'_, '_> {
    type TargetPixel = Rgb565Pixel;

    fn process_line(
        &mut self,
        line: usize,
        range: Range<usize>,
        render_fn: impl FnOnce(&mut [Rgb565Pixel]),
    ) {
        let mut before = (self.clock)();
        if self.drawn.calls == 0 {
            self.drawn.prepare_us = (before - self.started) as u32;
        }
        self.drawn.note(line, &range);
        let crossed = crossing(self.direct, line, &range);
        if self.stage.is_some() {
            let room = self.half_len();
            let width = range.len();
            let band = &self.band;
            let joins = band.lines > 0
                && band.first + band.lines == line
                && band.range == range
                && band.crossed == crossed
                && (band.lines + 1) * width <= room;
            if !joins {
                self.finish_band();
                // The band's blocks count in `copy_us`; this line's drawing starts now.
                before = (self.clock)();
                self.band = Band {
                    first: line,
                    lines: 0,
                    range: range.clone(),
                    crossed,
                };
            }
            // A failed block above dropped the stage.
            let base = self.half * self.half_len();
            if let Some(stage) = self.stage.as_mut() {
                let at = base + self.band.lines * width;
                render_fn(pixels_of(&mut stage.pixels[at..at + width]));
                self.band.lines += 1;
                self.drawn.lines_us += ((self.clock)() - before) as u32;
                return;
            }
        }
        self.drawn.cpu_wrote = true;
        let row = &mut self.frame[line * WIDTH..(line + 1) * WIDTH];
        if crossed == 0 {
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

/// The rectangles that cross `range` on `line`, one bit each in `direct`'s order (the
/// first 32).
fn crossing(direct: &[Area], line: usize, range: &Range<usize>) -> u32 {
    direct
        .iter()
        .take(32)
        .enumerate()
        .filter(|(_, area)| {
            (area.y..area.y + area.height).contains(&line)
                && area.x < range.end
                && area.x + area.width > range.start
        })
        .fold(0, |crossed, (i, _)| crossed | 1 << i)
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
fn outside(direct: &[Area], line: usize, range: Range<usize>, mut f: impl FnMut(Range<usize>)) {
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

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use std::vec;
    use std::vec::Vec;

    const LANE: Area = Area {
        x: 100,
        y: 10,
        width: 200,
        height: 20,
    };
    const HEIGHT: usize = 40;

    fn clock() -> u64 {
        0
    }

    /// What the PPA does: the block, row by row, but only when it is waited for, so a band
    /// drawn over a half still copying would show. Counted; the start of block `NO_START`
    /// fails, and so does the wait for block `NO_END`.
    static BLOCKS: AtomicUsize = AtomicUsize::new(0);
    static NO_START: AtomicUsize = AtomicUsize::new(usize::MAX);
    static NO_END: AtomicUsize = AtomicUsize::new(usize::MAX);
    /// The copy begun and not yet waited for: the stage's and frame's addresses, its block
    /// and its number.
    static COPYING: std::sync::Mutex<Option<(usize, usize, Block, usize)>> =
        std::sync::Mutex::new(None);
    fn start(stage: &[u16], frame: &mut [u16], block: Block) -> bool {
        let mut copying = COPYING.lock().unwrap();
        assert!(
            copying.is_none(),
            "a copy began before the one before it ended"
        );
        let number = BLOCKS.fetch_add(1, Ordering::Relaxed);
        if number == NO_START.load(Ordering::Relaxed) {
            return false;
        }
        assert!(block.height * block.stage_width <= stage.len());
        assert!((block.y + block.height - 1) * WIDTH + block.x + block.width <= frame.len());
        *copying = Some((
            stage.as_ptr() as usize,
            frame.as_mut_ptr() as usize,
            block,
            number,
        ));
        true
    }
    fn wait() -> bool {
        let Some((stage, frame, block, number)) = COPYING.lock().unwrap().take() else {
            return true;
        };
        if number == NO_END.load(Ordering::Relaxed) {
            return false;
        }
        for row in 0..block.height {
            let from = row * block.stage_width + block.stage_x;
            let to = (block.y + row) * WIDTH + block.x;
            // SAFETY: `start` checked the block lies in both, and `render` keeps them
            // alive and the stage's half untouched until this wait.
            unsafe {
                let from = (stage as *const u16).add(from);
                let to = (frame as *mut u16).add(to);
                core::ptr::copy_nonoverlapping(from, to, block.width);
            }
        }
        true
    }

    /// Draws `lines`, each its whole width in a colour of its own, as Slint would, and
    /// returns the frame, whether the CPU wrote it, and the blocks sent.
    fn draw(
        stage_pixels: usize,
        no_start: usize,
        no_end: usize,
        lines: &[(usize, Range<usize>)],
    ) -> (Vec<u16>, bool, usize) {
        BLOCKS.store(0, Ordering::Relaxed);
        NO_START.store(no_start, Ordering::Relaxed);
        NO_END.store(no_end, Ordering::Relaxed);
        let mut frame = vec![0xffff; WIDTH * HEIGHT];
        let mut stage = vec![0; stage_pixels];
        let stage = (stage_pixels > 0).then(|| Stage {
            pixels: &mut stage,
            start,
            wait,
        });
        let direct = [LANE];
        let mut drawn = Lines::new(&mut frame, &direct, clock, stage);
        for (line, range) in lines {
            let colour = *line as u16 + 1;
            (&mut drawn).process_line(*line, range.clone(), |pixels| {
                pixels.fill(Rgb565Pixel(colour));
            });
        }
        drawn.finish();
        assert!(
            COPYING.lock().unwrap().is_none(),
            "a copy was left under way"
        );
        let cpu_wrote = drawn.drawn.cpu_wrote;
        (frame, cpu_wrote, BLOCKS.load(Ordering::Relaxed))
    }

    /// The frame `draw` should leave: each line drawn, except inside the lane.
    fn expected(lines: &[(usize, Range<usize>)]) -> Vec<u16> {
        let mut frame = vec![0xffff; WIDTH * HEIGHT];
        for (line, range) in lines {
            for x in range.clone() {
                let inside = (LANE.y..LANE.y + LANE.height).contains(line)
                    && (LANE.x..LANE.x + LANE.width).contains(&x);
                if !inside {
                    frame[line * WIDTH + x] = *line as u16 + 1;
                }
            }
        }
        frame
    }

    /// Lines 0 to 39, the whole width: above, across and below the lane.
    fn screen() -> Vec<(usize, Range<usize>)> {
        (0..HEIGHT).map(|line| (line, 0..WIDTH)).collect()
    }

    // The tests share the copy counters: one test runs them all in turn.
    #[test]
    fn stage_draws_what_the_cpu_does() {
        const ALL: usize = usize::MAX;
        let lines = screen();
        let want = expected(&lines);

        // Without a stage: the CPU, as before.
        let (frame, cpu_wrote, blocks) = draw(0, ALL, ALL, &lines);
        assert!(frame == want && cpu_wrote && blocks == 0);

        // A stage of 32 lines, two halves of 16: bands of 10 above the lane, 16 and 4
        // across it (two blocks each, either side) and 10 below.
        let (frame, cpu_wrote, blocks) = draw(32 * WIDTH, ALL, ALL, &lines);
        assert!(frame == want);
        assert!(!cpu_wrote);
        assert_eq!(blocks, 1 + 2 + 2 + 1);

        // Halves of 8 lines: 8 and 2 above, 8, 8 and 4 across, 8 and 2 below.
        let (frame, cpu_wrote, blocks) = draw(16 * WIDTH, ALL, ALL, &lines);
        assert!(frame == want && !cpu_wrote);
        assert_eq!(blocks, 2 + 2 * 3 + 2);

        // A narrower range than a whole line, and lines that skip: new bands.
        let partial: Vec<_> = [(2, 10..50), (3, 10..50), (5, 10..50), (6, 20..50)]
            .into_iter()
            .collect();
        let (frame, cpu_wrote, blocks) = draw(16 * WIDTH, ALL, ALL, &partial);
        assert!(frame == expected(&partial) && !cpu_wrote);
        assert_eq!(blocks, 3);

        // The second block does not begin: the CPU writes it and every line after it.
        let (frame, cpu_wrote, blocks) = draw(32 * WIDTH, 1, ALL, &lines);
        assert!(frame == want);
        assert!(cpu_wrote);
        assert_eq!(blocks, 2);

        // The first block does not end, found out while the next band was drawn in the
        // other half: the CPU copies it from its half, and every line after it.
        let (frame, cpu_wrote, blocks) = draw(16 * WIDTH, ALL, 0, &lines);
        assert!(frame == want);
        assert!(cpu_wrote);
        assert_eq!(blocks, 1);

        // A stage of one line: one half, each band waited for before the next.
        let (frame, cpu_wrote, blocks) = draw(WIDTH, ALL, ALL, &lines);
        assert!(frame == want && !cpu_wrote);
        assert_eq!(blocks, 10 + 20 * 2 + 10);

        // A stage smaller than a line is not used.
        let (frame, cpu_wrote, blocks) = draw(WIDTH - 1, ALL, ALL, &lines);
        assert!(frame == want && cpu_wrote && blocks == 0);
    }

    /// A clock that moves only when a line is drawn (1 µs) or a block copied (1000 µs).
    static TICKS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
    fn ticks() -> u64 {
        TICKS.load(Ordering::Relaxed)
    }
    fn slow_copy(_: &[u16], _: &mut [u16], _: Block) -> bool {
        TICKS.fetch_add(1000, Ordering::Relaxed);
        true
    }
    fn done() -> bool {
        true
    }

    #[test]
    fn a_band_finished_by_the_next_line_counts_only_as_copying() {
        let mut frame = vec![0xffff; WIDTH * HEIGHT];
        let mut stage = vec![0; 32 * WIDTH];
        let direct = [LANE];
        let stage = Some(Stage {
            pixels: &mut stage,
            start: slow_copy,
            wait: done,
        });
        let mut drawn = Lines::new(&mut frame, &direct, ticks, stage);
        for (line, range) in screen() {
            (&mut drawn).process_line(line, range, |_| {
                TICKS.fetch_add(1, Ordering::Relaxed);
            });
        }
        drawn.finish();
        // 40 lines, and six blocks: three bands finished by the next line, one at the end.
        assert_eq!(drawn.drawn.lines_us, 40);
        assert_eq!(drawn.drawn.copy_us, 6 * 1000);
    }
}
