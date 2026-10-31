# JC1060 `sd-rust-load-display` profile

Date: 2026-10-31. Branch: from main `f8f10bb` (after PR #62). Build only: no
COM3 access, flash or device run yet. Native minimum 302 and APTA unchanged.

## Why

The `display-touch` diagnostic proved the New_Panel on its own. The
`sd-rust-load` image never initializes the panel, so the screen stayed dark
during the load runs, and the bridge guard forbade any display. This profile
shows the load status on the panel, with no touch, audio or USB host.

## What it adds

- **Panel bring-up:** `ruzina_panel_start(fill, &frame)` is factored out of
  `display_touch.c` with the same sequence: LEDC backlight held off, LDO3
  2.5 V, 2-lane DSI at 750 Mb/s, reset, the 60-command init table, 52 MHz DPI
  with one RGB565 framebuffer, a fill before scanout, cache write-back, then
  panel init and 25 % backlight. The touch diagnostic calls it with its stripe
  fill and is otherwise unchanged.
- **Status screen:** `load_display.c`, with the 12x24 subset of Terminus Font
  (SIL OFL 1.1, U+0020..U+017F, so č ć š ž đ render). It shows:
  - the pass, tracks, tracks with analysis and playlists;
  - free and minimum PSRAM, the client stack and the SD owner state;
  - up to 42 "artist - title" rows in two columns, each with a pass-1 outcome
    mark.
- **Data path:**
  - `ruzina_bridge_load_report` forwards each scalar event.
  - A new C callback, `ruzina_bridge_load_title`, receives each track's artist
    and title once, on pass 1. Rust lends the bytes only for the duration of
    the call, and C copies a bounded prefix.
  - All of this goes into one status block under a short critical section,
    with no formatting while locked.
  - A priority-1 task on core 0 redraws changed 24-pixel row bands and writes
    each band back with `esp_cache_msync`.
- **Configuration:** `CONFIG_RUZINA_LOAD_DISPLAY` requires `SD_RUST_LOAD` and
  excludes `DISPLAY_TOUCH`. The CMake guard enforces this, with a new test of
  4 cases. The build tool gets the `sd-rust-load-display` variant: features,
  defaults, PSRAM checks, the `ruzina_legacy_load_run` symbol, the
  `load_display.c.obj` map check, the touch exclusion and the stack-frame
  guard. The variant is added to the flash-plan variant list and to the CI
  matrix.

## Evidence (local)

- Legacy core: 39 tests with `idf-media-load`, host clippy, and target clippy
  for both features. 23 tooling tests pass.
- `build_legacy.py --variant sd-rust-load-display` returns `BUILD_PACKAGE_PASS`.
  The app is 393232 bytes, SHA-256
  `2f1df6215d6a908226452f14491331d7d020d677abd1e0d8533e934942a16c04`, built on
  dirty `f8f10bb` with the diff recorded. The C code compiles with no warnings
  under `-Werror`.
- The refactored `display-touch` variant still returns `BUILD_PACKAGE_PASS`.
- **Offline flash plan:** `64e21688…`, status
  `OFFLINE_REVIEWED_OPERATOR_GATES_OPEN`. The predecessor is the flashed
  stack-fixed load image (`191c73aa…`), with a chain of 10 plans. It writes
  `0x2000`, `0x8000` and `0x10000..0x71000`.

## Device run acceptance (needs the owner)

- The screen lights after reset, shows the mount, the 25-track list with marks
  and an updating pass line. Croatian letters render.
- Load results match the `sd-rust-load` run: 25/25 analysis every pass, a
  stable PSRAM minimum (now lower by the 1.2 MB framebuffer), stack headroom
  and comparable pass time.
- No flicker beyond row redraws; no panic, watchdog or DPI underrun messages.
- Removal shows the stopped state on screen. Reinsertion with app restarts and
  a cold return each bring the screen and the load back.
