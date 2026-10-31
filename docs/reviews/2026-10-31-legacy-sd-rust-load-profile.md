# JC1060 `sd-rust-load` diagnostic profile

Date: 2026-10-31. Branch: `ogranak/jc1060-sd-rust-load`, from main
`dbfecf36d372eb1bf1924643af2b00dc2f69d86b` (after PRs #57, #58 and #59).
No COM3 access, flash or device run. Native minimum 302 and APTA unchanged.

## What it adds

A separate opt-in IDF profile that runs the shared Library media path on the
read-only Rust SD bridge:

1. Mount the exFAT volume.
2. Derive the exFAT volume identity with the native formula.
3. Import `export.pdb` into a PSRAM-backed `VecCatalogStage`.
4. Run the load preflight for every track.
5. Repeat steps 3 and 4 once a second.

Each phase reports scalar telemetry, with free and minimum PSRAM. Profile
details and event codes are in the
[legacy README](../../firmware-idf/legacy-jc1060/README.md#sd-rust-load-profile-build-only).

- Rust: `media_load::volume_identity` and the `ruzina_legacy_load_run` export
  (feature `idf-media-load`). The export reuses the existing bridge, mount,
  `import_catalog` and `preflight`. Its stale or failed outcomes stop the
  client, which hands the card back to the owner as the exFAT profile does.
- C: a load branch in the bridge client task, with a 49152-byte stack and a
  PSRAM initialization check, plus `ruzina_bridge_load_report`. The owner,
  the read-only command filter and the DMA bounce buffer are unchanged.
- Configuration: `CONFIG_RUZINA_SD_RUST_LOAD` depends on the bridge and
  PSRAM, and excludes the fixture run. The CMake guard now allows PSRAM with
  the bridge only for this profile, and applies the memory profile's PSRAM
  rules (80 MHz, caps-only, no XiP). `sdkconfig.sd-rust-load.defaults` copies
  the memory profile's PSRAM settings.
- Build tool and CI: the new variant gets its features, defaults and guards
  (PSRAM config, `ruzina_legacy_load_run` linked, no FatFs, no SD write or
  erase symbols). It is added to the flash-plan variant list and to the
  legacy CI matrix.

## Evidence

Local, in WSL with Rust 1.95.0:
- Legacy core: 39 tests pass with `idf-media-load`, one of them new (volume
  identity).
- Host clippy and target clippy pass.
- The target release build exports `ruzina_legacy_load_run` and imports
  `ruzina_bridge_load_report` and `heap_caps_aligned_alloc`.
- 19 tooling tests pass in WSL, including the new guard test with 7 cases.
  The remaining 2 need esptool, which WSL does not have.

The full IDF image was not built locally. This machine has no ESP-IDF 6.1
Python environment (`idf6.1_py3.11_env`). The legacy CI matrix builds the
variant with the pinned IDF and its guards; read that run for this head.
The C changes were checked against the pinned IDF headers
(`esp_psram_is_initialized`, `esp_psram_get_size`) but are not compiled here.

## Next: first device run (needs the owner)

The device run needs:
- the CI-built package for the exact head;
- a flash plan that replays the existing predecessor chain;
- a card with a real Rekordbox export.

Acceptance:
- catalog count matching the export;
- per-track outcomes, with ANLZ results compared against the earlier
  189-track comparison;
- PSRAM minimum stable across passes;
- removal and reinsertion with app restarts;
- cold return.

Display, audio, USB host and C6 stay outside this profile.
