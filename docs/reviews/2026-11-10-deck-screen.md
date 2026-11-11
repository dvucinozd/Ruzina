# JC1060: deck screen in Slint, waveforms drawn directly

Date: 2026-11-10. Branch: `ogranak/jc1060-slint-spike`, from `main` at `2e6957c`.

A diagnostic image, not yet the product screen: touch does nothing yet. It answered
whether the deck screen fits next to USB audio (s40-s52, demo data), and since 2026-11-11
it shows the decks themselves (s53-s56).

## What it is

- **Layout:** `rust-core/ui/overview.slint` follows the Overview page of the donor firmware
  (DDJ-FFL4 / Pajoniiir) on 1024 x 600: tabs, deck badges with Play and Cue and a level
  meter, two 872 x 201 waveform lanes, the FX panel, a title bar, times, BPM, pitch, MT, key
  and two track overviews. Font: DejaVu Sans. Positions follow the donor's constants;
  colours partly come from photos.
- **Slint:** the software renderer, no `std`, line by line into internal RAM, so the panel
  never shows a half-drawn line. Only what changed is redrawn.
- **Waveforms:** Rust draws them directly into the frame buffer; Slint skips those
  rectangles. The ESP32-P4 PPA moves the previous frame's lane left in hardware, and the CPU
  draws only the new columns and the playhead.
- **Frames:** two frame buffers, swapped at the end of a panel refresh. One frame every two
  refreshes (about 30 a second); the scroll follows the refresh count, so each frame moves
  by the same step. The UI task runs on core 1 at priority 4, above the SD owner.
- **Audio in IRAM (`--fill-iram`):** the USB audio fill path, 47 functions and 75.5 KB, moves
  to internal RAM by renaming exact function sections; the build checks the instructions
  are unchanged and every moved function links into IRAM.
- **Memory:** `partitions-slint.csv` gives the app 12 MB and moves the Hot Cue journal to
  `0xC10000` for these images. The app is 1.43 MB. The production `deck-engine` image is
  unchanged.

## Live deck data (2026-11-11)

- **From the engine:** after every render block the deck engine stores each deck's
  position, Play, Sync, Master Tempo, loop, pitch and BPM, and the Beat FX, in atomics
  (`deck_view.rs`). The UI reads them per frame.
- **From the load task:** at each load it reads the track's analysis again (beat grid,
  PWV3 detail, preview, cues) and hands it with title, artist, key and length through a
  one-place mailbox. The startup pass keeps none of it.
- **Waveform as the donor draws it:** one solid column per pixel, the loudest of it and
  its neighbours, at least a centre line, so the shape stays joined; the donor's palette
  picked by the PWV3 colour hint and loudness, with white tips on loud peaks. The
  overviews use the same palette.
- **Zoom:** the browse knob moves the lane through 2, 4, 8, 12, 16, 24 and 32 beats of the
  track's tempo (the donor's five plus one at each end), 16 at start, 0.5 s to 30 s.
- **Bar lines:** a red triangle on each bar line, deck 1 at the bottom of its lane and
  deck 2 at the top, so the two meet in the middle for beat matching.
- **Channel meters (s57, s58):** the render graph reports each channel's loudest sample
  before its fader (after trim, EQ, filter and FX). The screen shows ten segments (green
  to -9 dBFS, amber at -6 and -3, red from -0.5). Since s59 Rust draws them, only when
  they change: in Slint they cost 2.3 ms a frame and dropped it below 30 frames a
  second. Each is written back at once, because the PPA driver invalidates the lane's
  rows before its copy and dropped unwritten meter pixels (s59 flickered). The FLX4's channel meters get the donor's
  CC 0x02, linear 0 to 127, every 30 ms, rising at once and falling 12 a step.
- **Paused deck:** the hardware copies the whole lane, so both buffers always match; s53
  skipped it, which made the playhead flicker. Since s59 a lane that each buffer already
  holds is left alone.

## How it got there

| Image | Change | Frames/s | Wave frame | Longest fill | Fills over 3 ms | 6 ms+ |
|---|---|---:|---:|---:|---:|---:|
| s40 | no screen (soak only) | | | 4466 µs | 0.6 % | 0 |
| s43 | simulator UI, direct to scanout | | 68 ms | 7420 µs | 26 % | 41 |
| s44 | line by line, only changes | | 29-39 ms | 5105 µs | 8.7 % | 0 |
| s48 | donor layout, CPU waveforms, IRAM fill | 11 | 43 ms | 4191 µs | 10 % | 0 |
| s50 | two buffers, mirrored rows | 19 | 24 ms | | | |
| s51 | PPA scroll | 25-29 | 20 ms | 4111 µs | 5.0 % | 0 |
| s52 | fixed cadence, priority 4 | 30 | 18 ms | **4505 µs** | 6.3 % | **0** |
| s53 | live deck data | 30 | 9-22 ms | | | |
| s54 | bars with gaps, paused copy | 30 | 21-36 ms | | | |
| s55 | donor waveform, browse zoom | 30 | 25-36 ms | | | |
| s56 | seven zoom steps | 30 | | 5789 µs | 5.0 % | 0 |
| s57 | screen meters, bar triangles | 30 | 22-27 ms | 6234 µs | 8.8 % | 2 |
| s58 | controller meters | 30 | 22-27 ms | 5866 µs | 9.2 % | 0 |
| s59 | meters drawn by Rust | 30 | 17 ms | 6149 µs | 8.3 % | 1 |
| s60 | (deck engine in IRAM, reverted) | 30 | 18 ms | 5737 µs | 8.2 % | 0 |
| s61 | (SD owner below the fill, reverted) | 30 | | 6137 µs | 8.2 % | 1 |

All images ran the soak DJ (both decks loading and playing, with its gestures).

- **s52 over about 5 minutes:** underruns 0/0, `packet_errors=0`, no faults. Longest wave
  frame 37 ms early, then about 19 ms.
- **Owner:** s43 flickered on deck 1, s48 to s51 jerked, and at s52: "sad je glatko, ne trza".
  s53: the playhead flickered and the waveform had the wrong shape; s54: it lacked colour
  and broke apart; s55: one zoom step short; s56: "Radi, sve ok".
- **s53-s56:** underruns 0/0 and no faults; the longest wave frame (about 36 ms) comes
  with a track load.
  s49 and s50 also showed a wavy "through water" look, from drawing into the buffer on
  the panel.

## Open

- **Fills since the live decks:** the longest went over the 5 ms limit again (s55 6449 µs,
  s57 6234 µs with two of 6 ms or more, s58 5866 µs) and 5 to 9 % of fills take over 3 ms.
  Still no underrun. Neither the deck engine in IRAM (s60) nor the SD owner below the
  fill (s61) changed it.
- **Accepted for now (owner, 2026-11-11):** fills of up to about 6.3 ms during a fast
  forward scratch, with no underrun; without a scratch the worst is about 4.2 ms. To be
  looked at again with the USB stick media path, which changes how the decks are fed.
- **Fill profile next to the screen (s63, `--uac-profile --soak-autoplay --slint-ui`):**
  the worst fill, 5814 µs at 58.7 s, came just after the soak DJ started a scratch. It
  was not preempted (2 µs away, 34 µs of interrupts): 4761 µs of it was the render
  graph, against 2158 µs at worst without a screen (s19) and 2559 µs in a typical fill
  now. The graph waits on memory, most likely PSRAM shared with the screen's lane copies.
- **Without the lanes (s64, diagnostic only):** worst fill 4693 µs, 7.3 % of fills over
  3 ms (11.3 % in s63), worst graph 3923 µs. The lane copies cost about 1.1 ms of the worst
  fill; the rest of the screen and the scratch cost the remainder.
- **What the deck was doing (s66, s67):** the profile's deck flags now also carry scratch
  (4), Beat FX (8), Pad FX (16), loop (32) and history replay (64). In ten minutes the
  worst fills (6337 and 6348 µs) both came during a scratch; without one the worst was
  4197 µs. A forward scratch takes the ring up to six times as fast, each frame read from
  the ring and written to the history in PSRAM. Keeping the newest 16384 history frames
  in internal RAM for the scratch's reads (s67) did not help (6495 µs) and was reverted.
- **Lane copies in strips that wait for the fill (s68, reverted):** typical fills
  improved (6.6 % over 3 ms instead of 11.3 %), but the worst stayed during a scratch
  (6280 µs) and the screen fell to 11 frames a second. The worst case is the fast
  scratch itself, which takes up to six source frames for each one played.
- **32-byte PPA bursts (s65, reverted):** worse, 6165 µs worst with three fills of 6 ms or
  more, and the lanes took 21.6 ms a frame instead of 17.3.
- **Fills over 3 ms (s52):** 6.3 % against 0.6 % without a screen. The longest stays under the
  5 ms limit, but the screen costs margin; the PPA's PSRAM copies are likely part of it.
- **Lost USB packets:** two in s49, one each in s44 and s54 (after about 12 minutes); none
  in s51, s52 and s58. Not explained.
- **Producer low water:** both decks' rings ran low for 100-130 ms early in s51 and s52,
  near track loads. No underrun. SD will be replaced by a USB stick later, which changes
  this load.
- **Not on the lane yet:** hot cue and loop markers. Shift + browse does not zoom faster.
- **Next:** touch, then Master Tempo. The 7 MB of images and non-ASCII titles are not in
  this screen yet.
