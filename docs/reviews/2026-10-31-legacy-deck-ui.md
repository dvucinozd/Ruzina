# JC1060 `deck-ui`: the controller drives the library screen

Date: 2026-10-31. Branch: from main `697dacc` (after PR #67). Build only: no
device run yet. Native minimum 302 and APTA unchanged.

## What it adds

This is the first profile that runs several owners together:
- the read-only Rust SD load from [`sd-rust-load`](2026-10-31-legacy-sd-rust-load-profile.md);
- the status screen with touch selection from [`sd-rust-load-display`](2026-10-31-legacy-load-display.md);
- the USB host with FLX4 MIDI and LEDs from [`usb-midi`](2026-10-31-legacy-usb-midi.md).

- **Rust, `controller_midi`:** `ui_action` turns semantic events into library
  actions:
  - `BrowseDelta` becomes browse by delta;
  - a `BrowsePress` press becomes choose;
  - a `Load` press on deck 1 or 2 becomes load.

  These reach C through `ruzina_bridge_midi_ui(action, deck, value)`, which
  carries only scalars, so C does not depend on the semantic enum's order.
  The host test drives the real profile: browse encoder +1 and −1
  (`0xB6 0x40`, two's complement), browse press (`0x96 0x41`) and load
  deck 1 and 2 (`0x96 0x46/0x47`).
- **C, `usb_host_enum`:** logs each action as `midi_ui` and, with the status
  screen, forwards it.
- **C, `load_display`:**
  - Browse moves the selection, clamped to the catalog, and asks the load
    task for that track's detail, exactly as a tap does.
  - Choose re-requests the detail.
  - Load assigns the selected track to deck 1 or 2. The header then shows
    `D1 <artist - title>` and `D2 <artist - title>` instead of the banner.
- **Start-up order:** the display starts first, then the SD bridge, then the
  USB host, so controller actions find the screen ready.
- **Configuration:**
  - Defaults are `sdkconfig.sd-rust-load`, `load-display` and the new
    `deck-ui`, which adds the USB host, MIDI and the 4 KiB control buffer.
  - The CMake guard allows the USB host beside SD and the display only in
    this full combination. That combination still excludes the touch
    diagnostic, memory/task and pure RAM; the guard test gains 6 cases.
- **Build tool:**
  - The `deck-ui` variant builds Rust with `idf-media-load` and
    `idf-controller-midi` together.
  - It requires `ruzina_legacy_load_run`, `ruzina_legacy_midi_ingest` and
    `usb_host_install` to be linked, plus the USB host, display and SD-bridge
    sources.
  - The existing 24 KiB guard applies to every Rust frame, and the MIDI
    exports must stay within 2 KiB.
  - CI target-builds the combined features and adds the variant to the
    matrix.

## Evidence (local)

- Rust: the controller test (now also covering UI actions) and the 39 core
  tests pass. Host clippy and target clippy pass for `idf-exfat-fixture`,
  `idf-media-load`, `idf-controller-midi` and both combined.
- Tooling: 25 tests pass.
- `build_legacy.py --variant deck-ui` returns `BUILD_PACKAGE_PASS` with no
  warnings. The app is 501376 bytes, SHA-256
  `fe91eade13fd3b12e72d94849594c2582aeb0897d7ce92339c4d5f72879a1d1f`, and
  fits the 1 MiB factory partition.
- **Offline flash plan:** `07381298…`. The predecessor is the flashed
  `usb-midi` LED image (`8ef6755c…`), with a chain of 19 plans. It writes
  `0x2000`, `0x8000` and `0x10000..0x8b000`.

## Device acceptance (needs the owner)

Setup:
- PC power on the FS port only, with the battery disconnected.
- The SD card with the 25-track export inserted.
- The FLX4 on bus power on the HS port.

Accept the run when:
- The screen shows the catalog and the passes reach 25/25, as before.
- The FLX4 attaches, and the lamp test runs.
- Turning the browse encoder moves the highlight and shows each track's
  detail and waveform.
- Load deck 1 and Load deck 2 fill the `D1` and `D2` header.
- The touch selection still works.
- The PSRAM and stack telemetry stay comparable to the separate profiles.
- There is no fault and no USB or SD error.
