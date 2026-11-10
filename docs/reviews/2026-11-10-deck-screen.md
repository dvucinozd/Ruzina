# JC1060: deck screen in Slint, waveforms drawn directly

Date: 2026-11-10. Branch: `ogranak/jc1060-slint-spike`, from `main` at `2e6957c`.

A diagnostic image, not yet the product screen: it shows **demo data**, not the decks, and
touch does nothing yet. It answers whether the deck screen fits next to USB audio.

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

All images ran the soak DJ (both decks loading and playing, with its gestures).

- **s52 over about 5 minutes:** underruns 0/0, `packet_errors=0`, no faults. Longest wave
  frame 37 ms early, then about 19 ms.
- **Owner:** s43 flickered on deck 1, s48 to s51 jerked, and at s52: "sad je glatko, ne trza".
  s49 and s50 also showed a wavy "through water" look, from drawing into the buffer on
  the panel.

## Open

- **Fills over 3 ms:** 6.3 % against 0.6 % without a screen. The longest stays under the
  5 ms limit, but the screen costs margin; the PPA's PSRAM copies are likely part of it.
- **Two lost USB packets** in s49 and one in s44; none in s51 and s52. Not explained.
- **Producer low water:** both decks' rings ran low for 100-130 ms early in s51 and s52,
  near track loads. No underrun. SD will be replaced by a USB stick later, which changes
  this load.
- **Next:** live deck data (titles, times, BPM, pitch, playhead, analysis waveforms and
  cues), then touch, then Master Tempo. The 7 MB of images and non-ASCII titles are not in
  this screen yet.
