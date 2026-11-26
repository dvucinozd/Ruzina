# JC1060: Slint's big frames copy while they draw

Date: 2026-11-26. Branch: `ogranak/big-frames`, from `main` at `90efcb4`.

## Problem

A whole Library page (a page turn, opening the Library) took 60 to 96 ms (s231p). Of a
430-row page turn: the scene's preparation about 29 ms, Slint's lines 16 ms, and 11 ms
waiting for the PPA to copy each band of lines from internal RAM into the frame.

## Change

- **Copies overlap the drawing.** `ruzina-screen-lines` uses its strip (16 lines,
  internal RAM) in two halves. A band's blocks start copying (`ruzina_panel_blit_start`),
  and the next band is drawn in the other half meanwhile; a half is drawn again only once
  its copies have ended (`ruzina_panel_blit_wait`). Every copy has ended when the draw
  returns.
  - One copy at a time, as before; the other PPA copies (lane caches) wait for one under
    way first.
  - A copy that does not begin or end goes by the CPU, from the half still holding its
    pixels, and so does every line after it, as before.
  - The host test's PPA copies only when it is waited for, so a band drawn over a half
    still copying would show; with the halves not switching, the test fails.
- **The preparation is timed inside Slint.** The vendored renderer's `lib.rs` gets a clock
  (`set_probe_clock`) and reports the last preparation's parts (`scene_prep`); see
  [the renderer's notice](../../firmware-idf/legacy-jc1060/third_party/slint-renderer-NOTICE.md).
  Slow frames log them as events 50 (dirty tracking, items), 51 (texts, sorting) and 52
  (texts, glyphs).
- **The PC renderer times frames** as the board draws them
  (`examples/render_screens.rs`, `frame_parts`).

## Evidence

- **Host:** fmt; tests and clippy for all ten feature sets; target clippy for the four
  deck screen sets; `ruzina-screen-lines` tests and clippy; `verify_vendor.py` with the
  new pin.
- **Device:** s233p (app `1d6420bf…`, profile image), from this branch, by OTA. The owner
  opened the Library, turned its pages and opened Settings, 5 minutes.

  | Frame (rows drawn) | s231p | s233p |
  |---|---|---|
  | Library page turn (430) | 62.7 ms, copy 11.3 ms (n=8) | 55.2 ms, copy 5.2 ms (n=24) |
  | Library open (600) | 88.5 ms, copy 13.2 ms (n=3) | 80.4 ms, copy 6.5 ms (n=6) |

  - No `ppa stuck`; underruns 0/0, `packet_errors=4` at boot as on s231p.
- **Where a page turn's preparation goes** (s233p, 29.3 ms): the dirty tracking over the
  whole item tree 14.2 ms; the items 12.1 ms, 7.6 ms of it 70 texts with 448 glyphs; the
  sorting 1.3 ms. The same turn on the PC: 268, 197 (141) and 11 µs, about 55 times
  faster.

## Open

- The copy still shows for about half its time: the PPA copies a band more slowly than the
  CPU draws the next.
- The dirty tracking is Slint core's (`i-slint-core`, not vendored) and walks every item;
  on the PC, a change to any field of a row costs as much as a change to all of them, and
  the rows' nested `if`s cost nothing measurable.

## Test plan

1. Open the Library several times and turn its pages with the browse knob.
2. Open Settings for ten seconds, go back to the Overview: it looks as before.
3. The decks play without underruns.
