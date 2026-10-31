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

Full IDF build, local: the pinned IDF v6.1 checkout and its recorded Python
environment under `D:\BoardEvidence\JC1060-20261030-COM3	oolchain` were used
(esptool 5.6.0, CMake 4.0.3, GCC esp-15.2.0, Rust 1.95.0 in WSL). With them:
- all 21 tooling tests pass;
- `build_legacy.py --variant sd-rust-load` returns `BUILD_PACKAGE_PASS` from
  clean commit `2487e70b6dad33a550f7a96aaf5b5ead44d213f2`. This covers the
  PSRAM config checks, the linked `ruzina_legacy_load_run`, no FatFs and no
  SD write or erase symbols.

The app is 489232 bytes, SHA-256
`62118365262a5d7e668e1025bb03f6fa574cd7adeb996fc8f16346b48f59c60e`, and fits
the 1 MiB factory partition. The image includes the earlier exFAT
`PartitionOffset=0` fix.

Remote CI on PR #60 did not run. GitHub refused every job with "an Actions
budget is preventing further use", so no step executed. That is
NOT_RUN, not a failure or a pass.

## Offline flash plan (no device access)

`prepare_flash_plan.py` produced `OFFLINE_REVIEWED_OPERATOR_GATES_OPEN`:
- **Plan:** `D:\BoardEvidence\JC1060-20261030-COM3\s06-rust-load-write-plan`,
  plan SHA-256 `fce1261c55a5a2e268fd4059e9cb5a032a79c24f3362e1b20ac055a382f6b81a`.
  It is recorded byte-for-byte in the
  [plan snapshot](../boards/readiness/jc1060-s06-sd-rust-load-write-recovery-plan-20261031.json).
- **Predecessor:** the flashed exFAT diagnostic, plan `e2d25655…`, app
  `ec6b7f0f…`, which had a matching readback. The full chain of 8 plans is
  replayed. Changed-sector restore targets that diagnostic; all 4 factory
  backup copies were reverified for a full restore.
- **Writes:** bootloader `0x2000`, partition table `0x8000` (identical to the
  current one) and app `0x10000..0x88000`.
- **Archive:** the package and plan are archived with a secondary copy, as
  listed in the [build index](../boards/readiness/jc1060-s06-sd-rust-load-build-20261031.json)
  (`add4c8fe…`, 19 members, CRC, member hashes and secondary copy verified).

Operator gates that remain open:
- physical connector and power;
- manual ROM entry;
- a live identity and security recheck;
- reset telemetry.

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
