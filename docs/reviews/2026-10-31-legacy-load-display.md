# JC1060 `sd-rust-load-display` profile

Date: 2026-10-31. Branch: from main `f8f10bb` (after PR #62). The device run,
with the owner present, is at the end. Native minimum 302 and APTA unchanged.

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

## Device run (2026-10-31, owner present)

CI on PR #63 passed all 12 jobs, including `sd-rust-load-display` and the
refactored `display-touch`. The image `2f1df621…` was flashed with plan
`64e21688…`:
- the identity, security and RDID checks passed;
- the predecessor sectors matched;
- readback matched.

Records are in `docs/boards/evidence/jc1060-20261030-com3/s06-load-display-*`,
indexed by the
[device index](../boards/readiness/jc1060-s06-sd-rust-load-display-device-20261031.json).
The card and export are the same as in the `sd-rust-load` run.

| Run | Result |
|---|---|
| ROM boot | `S6_DISPLAY_INIT` and `S6_LOAD_DISPLAY rows=21 columns=2`; 2 passes, 25/25 analysis. The owner confirmed the header, counts, memory line and the track list are all visible. |
| 3 app restarts | The display re-initialized each time with no IDF errors; 5 passes each at 25/25 |
| Removal mid-run | Passes 9–14 at 25/25. Pass 15 marked track 18 stale (stage 4); the owner quiesced on `ESP_ERR_TIMEOUT`, client `STOPPED -6`, 0 denied commands, no fault. The owner confirmed the red failure line and card message on screen. |
| Same-card reinsertion, 3 app restarts | Fresh mount at LBA 2048, 5 passes each at 25/25 |
| Cold return | USB power removed for 5–10 s, no BOOT/RESET; `reset_reason=1`. The owner confirmed the screen lit again; 3 new passes at 25/25. |

Across every run:
- After each pass, free PSRAM is 29704748 bytes and the PSRAM minimum is
  29220288 bytes. Both stayed identical in all passes. They are 1228912 bytes
  below the `sd-rust-load` run, the 1228800-byte framebuffer plus driver state.
- At least 13296 bytes of the client stack stayed free.
- Five passes took about 25.1 s, against 24.5 s without the display, about 2 %
  slower.

As in the earlier cold runs, the ROM and bootloader output was missed during
USB re-enumeration.

Still open:
- Tearing and flicker were not measured beyond the owner's visual check.
- Touch, interaction and track selection are not part of this profile.

## Touch selection and track detail

This follow-up adds the first interaction to the same `sd-rust-load-display`
profile.

- **Touch:** the GT911 code in `display_touch.c` is split into
  `ruzina_touch_start()` and `ruzina_touch_poll()`. The touch diagnostic uses
  them with unchanged output. The load profile polls every 40 ms on core 0 and
  logs each press as `load_touch press= mapped= index= errors=`.
- **Selection:**
  - A press on a track row highlights it and calls the Rust
    `ruzina_legacy_load_select`. That export only stores an atomic index and
    an "owed" flag.
  - At its next track boundary, the load task preflights the selected track
    out of turn. It does not count that run as a pass track. A stale outcome
    still stops the run.
  - The task then calls `ruzina_bridge_load_detail` with the catalog BPM,
    duration and key, the beat, hot-cue and memory-cue counts, and up to 400
    PWAV preview peaks. All bytes are lent for the call only.
- **Screen:** the track list shrinks to 17 rows per column (34 tracks), making
  room for a 4-band detail panel:
  - the selected title;
  - a facts line;
  - the preview, drawn as a 1000-pixel mirrored waveform.

  C logs `load_detail ... peak_fnv=` so a capture can compare the preview
  without the full array.

Local evidence:
- 39 core tests, host and target clippy, and 23 tooling tests pass.
- `sd-rust-load-display` returns `BUILD_PACKAGE_PASS` with no C warnings. The
  app is 415696 bytes, SHA-256
  `2e866f61e48cfd28d75ac9ea91d6ce5bbaeb90c97a7206b71b7b6fcdb08cdd2c`.
- `display-touch` still returns `BUILD_PACKAGE_PASS`.
- The flash plan is `8881509f…`, with the flashed display image (`64e21688…`)
  as its predecessor and a chain of 11 plans. It writes `0x2000`, `0x8000` and
  `0x10000..0x76000`.

Device acceptance:
- A tap selects and highlights a row.
- Within about a track's preflight, the facts and waveform appear.
- The `load_detail` counts and peak digest agree with the host
  `analysis_inspect` of that track.
- Passes stay 25/25 with stable PSRAM.
- Repeated taps never stall the load.

### Device run (2026-10-31, owner present)

The image `2e866f61…` was flashed with plan `8881509f…`:
- the identity, security and RDID checks passed;
- the predecessor sectors matched;
- readback matched.

Records are in `docs/boards/evidence/jc1060-20261030-com3/s06-load-touch-*`,
indexed by the
[device index](../boards/readiness/jc1060-s06-sd-rust-load-touch-device-20261031.json).

Results:
- **Boot:** `S6_TOUCH_ID address=0x5d config_size=1024x600` was reported next
  to `S6_LOAD_DISPLAY rows=17 columns=2 max_tracks=34`.
- **Taps:** the owner tapped single tracks and then several in quick
  succession, and confirmed on screen the highlight, the facts line and the
  waveform for each selected track.
  - 9 presses were logged. Each mapped to the expected row index, including
    the right column (`mapped=726,150 index=19`), with 0 I2C errors.
  - 6 `load_detail` reports followed. Taps that came before the next track
    boundary collapsed into the latest selection, as designed.
- **Host comparison:** all 6 details match the host import of the PC copy of
  the export:
  - catalog BPM, duration and key;
  - the `analysis_inspect` beat, hot-cue and memory-cue counts;
  - 400 preview columns, with the FNV digest of the device peaks equal to the
    digest of the host-parsed PWAV bytes.

  The comparison script and its output are kept outside the repository in
  `rb-export-compare-20261031`, because the output lists track titles.
- **Passes:** 6 passes during the taps, then 3 after an app restart, all at
  25/25 with no failure or fault. Touch re-initialized after the restart.
- **PSRAM:** the minimum is 29219144 bytes, 1144 bytes below the display-only
  image.
- **Stack:** the client stack minimum dropped from 13312 to 10588 free bytes.
  The out-of-turn preflight closure and the 400-byte peak buffer account for
  this, and it stays within the 24 KiB frame guard.

This run did not repeat removal, reinsertion or the cold return. The display
run covered those, and this change does not touch the SD or load lifecycle.