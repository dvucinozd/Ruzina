# JC1060: smooth lanes, drawn the way the donor draws them

Date: 2026-11-25. Branch: `ogranak/jc1060-faster-screen`, from main `fd4cbb7`.

After [the redesign (#166)](2026-11-22-redesign-on-fill-gap.md), the lanes still jerked
at the largest zoom and the next one down. This branch goes through seven profile images
(s189p–s197p). The last one moves both lanes and the phase strip at every refresh, and
the owner sees them run smooth.

## What was tried, and what each image showed

All runs had 0/0 underruns. "Late" means a frame that missed its refresh: it was held
for one refresh, then the lanes jumped twice as far.

| Image | Change | Frames / MT fills | Owner |
|---|---|---|---|
| s189p | main, measured | ~28 fps; MT 15.3 % > 3 ms, max 4.24 ms | — |
| s190p | Slint draws only when something changed | ~27 fps; the PPA lane move takes 8.1 ms | jerks |
| s191p | lanes moved by a plain 2D-DMA copy (2.5 ms) | ~28 fps; MT 7.6 %, max 4.28 ms | lanes correct |
| s192p | a frame every refresh (61 fps) | ~51 fps; MT 16.2 %, max 4.5 ms | jerks |
| s193p | lanes drawn from a steady clock | corrections ~1 ms on average; MT 16.0 % | jerks |
| s194p | lane copies run while Slint draws | 15.4 % late; MT 16.4 %, max 5.29 ms | jerks |
| s195p | late frames broken down; gate on/off every 30 s | 16.7 % late with the gate, 11.4 % without | better, still jerks |
| s196p | donor-style lanes (below) | lanes copied at every refresh | **smooth**; the phase strip jerks |
| s197p | the phase strip's halves as lanes too | see below | **smooth** |

**What s195p showed:**

- A late frame took about 20 ms against a 16.4 ms refresh.
- Slint took 8–12 ms of it, redrawing changed text (times, the offset label).
- The lanes took 4–8 ms, and the gate's waits 1–4 ms.

Even without the gate, one frame in about sixteen was late. With two buffers and a swap,
every frame must be whole before it can show, so a frame that runs long holds the lanes.

## How the donor draws them

The donor (Pajoniiir, `firmware/p4-core/components/ui`) does three things differently:

- **One frame buffer** on the JC1060 (`BSP_LCD_FRAMEBUFFER_COUNT 1`), with no swap.
- **A ring cache per deck** (`ui_overview_wave_cache`): a strip wider than the lane,
  copied into the frame in one or two blocks by the PPA after each refresh.
- **A free-running position clock** (`ui_position_interpolator`): it re-anchors only past
  120 ms of error.

## What this branch does now

**One buffer.**

- The deck screen draws into the buffer the panel shows. Slint redraws only what changed
  (`ReusedBuffer`).
- The second frame buffer is still allocated but no longer used.

**A ring cache for each lane** (`LaneCache`, `slint_ui.rs`).

- Each cache is a picture 1024 columns wide, with as many rows as its lane. Column
  `p mod 1024` holds track pixel `p`.
- The deck screen draws each cache ahead of the playhead, through the internal-RAM strip
  and the PPA, in batches:
  - it keeps 32 pixels behind the lane;
  - it keeps up to 178 pixels past the lane's right edge, about 25 refreshes at the
    largest zoom.
- It redraws a cache whole after a new track, a zoom, new marks or a jump.
- It redraws the columns the playhead passes while a Loop In waits for its Out.

**The lanes' task** (`lane_task`, `display_touch.c`).

- The task runs on the deck screen's core at priority 6, above the screen (4).
- It is woken at each refresh.
- **Position:** for each lane, it carries the deck's position on from what the screen
  last published (`ruzina_lanes_publish`), to when the beam reaches the lane's middle.
  - It wraps a running loop.
- **Copy:** it copies the lane's window from the cache by the 2D-DMA.
  - The window goes in two blocks, left and right of the playhead's three columns, so the
    playhead is never touched.
  - A ring wrap adds a third block.
- **Lanes:** both waveforms (814×220), and the phase strip's two halves (814×9 each).
  - Each half is one deck's beats as two-column ticks, at the strip's scale: its real
    span at the deck's speed.
- **Memory:** nothing writes the lanes' rows of the frame from the CPU.
  - The meters now also go through the strip and the PPA, because their cache lines reach
    into the lanes.
  - So neither the cache nor the frame needs a cache sync for these copies.

**Position.** The deck screen still publishes from the steady clock of s193p. Its
corrections average under 1 ms, so the lanes' task sees an almost straight line.

## Measured

- **s196p** (app `03603a8b…`), 15 minutes: the lanes' task copied at every refresh.
  - The copies started at most 0.49 ms after the refresh.
  - They were done in 5.0 ms on average (5.6 ms at most), before the beam reaches the
    lower lane (about 7.4 ms).
  - The cache missed about once in 20 s.
  - Fills: 0.1 % over 3 ms, max 5.09 ms, 0/0 underruns. MT was not on in this run.
- **s197p** (app `8c88f7ac…`), 8 minutes, 4 without MT and 4 with it:
  - Lane copies: done in 5.1–5.2 ms on average, 5.8 ms at most, started at most 0.62 ms
    after the refresh. 25 cache misses in the run, about one in 18 s.
  - MT fills: 13.5 % over 3 ms, max 4.81 ms.
  - Plain fills: 0.3 % over 3 ms, max 5.17 ms.
  - 0/0 underruns.
  - For comparison, s189p (main): MT 15.3 % over 3 ms, max 4.24 ms.

## After #168: one buffer and spare caches

Branch `ogranak/jc1060-lane-polish`, from main `b4a479d`.

**One frame buffer.** The second one went unused once the lanes moved in place. The
panel now has only one, which frees 1.2 MB of PSRAM. The Slint entry takes the shown buffer
alone.

**A spare cache for each lane.** Each lane has two ring caches.

- A window drawn anew goes into the spare cache, which then becomes the front one. That
  covers a new track, zoom, marks or pitch (for the phase strip), and a jump. The lanes'
  task keeps the old front cache meanwhile, so it no longer misses a refresh for it.
- While a loop runs that the front cache cannot hold whole, the spare cache holds the
  window from the loop's start. That is any loop of more than about half a beat at the
  largest zoom, where a beat is 407 pixels. When the loop goes round, the lanes' task takes
  that window, and the screen makes it the front.
- The model goes to C as one struct (`LaneModel` and `lane_model_t`), with its size checked
  on both sides.

**s199p** (app `08c53e6f…`), 8 minutes, 4 without Master Tempo and 4 with it:

- The owner tried a short loop at the largest zoom, the pitch fader and the zoom: "Sve
  glatko".
- Lane copies: 6 misses in the run (25 on s197p); 38 refreshes copied from the spare
  cache; done in 5.1 ms on average, 5.9 ms at most.
- MT fills: 14.2 % over 3 ms, max 4.80 ms.
- Plain fills: 0.3 % over 3 ms, max 4.26 ms.
- 0/0 underruns.

## Known limits

- **Text:** Slint draws into the shown buffer, so a changing text can tear for one
  refresh. The donor has the same.
- **Pitch fader and loops:** fixed after #168 by the spare caches (above).
- **Misses:** about one in 80 s remains on s199p, unexplained.
- **Spare cache cost:** while a loop that does not fit runs, the screen draws the spare
  cache whole (1024 columns) once per loop.
