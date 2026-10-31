# JC1060 `sd-rust-load` diagnostic profile

Date: 2026-10-31. Branch: `ogranak/jc1060-sd-rust-load`, from main
`dbfecf36d372eb1bf1924643af2b00dc2f69d86b` (after PRs #57, #58 and #59).
The build sections below had no device access. The first device run, with
its stack fix, is at the end. Native minimum 302 and APTA unchanged.

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
[legacy README](../../firmware-idf/legacy-jc1060/README.md#sd-rust-load-profile).

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

## First device run (2026-10-31, owner present)

PR #60 merged as `a0da6a6`; its CI passed all 11 jobs after the Actions limit
was raised. The owner was at the board for every physical step. Records are in
[`docs/boards/evidence/jc1060-20261030-com3`](../boards/evidence/jc1060-20261030-com3/)
(`s06-rust-load-*`), indexed by the
[device index](../boards/readiness/jc1060-s06-sd-rust-load-device-20261031.json).

### The first image overflowed its stack

The planned image (`62118365…`) was flashed with the reviewed plan:
- the identity, security and RDID checks passed;
- the predecessor sectors matched;
- readback matched
  ([result](../boards/evidence/jc1060-20261030-com3/s06-rust-load-flash-result.json)).

With no card it booted and quiesced on `ESP_ERR_TIMEOUT`. With the card,
`sd_rust_client` hit a **Stack protection fault** right after
`S6_RUST_LOAD`, before the mount event
([capture](../boards/evidence/jc1060-20261030-com3/s06-rust-load-stack-overflow-run-1.log)).

The ELF showed why. The prologue of `ruzina_legacy_load_run` reserves 85344
bytes on a 49136-byte stack. The poll function of `probe_export_pdb` needs
171968 bytes, and `scan_optional_name_table` needs 46592. The IDF caller passed
the 4 KiB page buffer by value. That buffer moves through every nested
`(file, buffer, result)` return and `await`, and LLVM does not overlap those
temporaries. Host tests cannot see this, because host threads have megabytes of
stack. The native firmware is not affected: its `Usb0LibraryFiles` buffer is a
`&'static mut [u8]`.

Fix (uncommitted at the time of the run):
- `media_load::import_catalog` passes a `Box<[u8]>` page buffer from the PSRAM
  heap.
- `ruzina_legacy_load_run` keeps the import and preflight futures in
  `Box::pin`.

The largest Rust frames are now 16048 bytes (the import poll), 15504 bytes and
10800 bytes (the run). The app shrank from 489232 to 349056 bytes.

`tools/stack_frames.py` reads each function's prologue from the linked ELF.
`build_legacy.py` now rejects every Rust-bridge variant that has a Rust frame
over 24 KiB. On the first ELF it reports all three oversized frames.

### Stack-fixed image

The new image is `893c09ba…`. It was built from `a0da6a6` plus the fix, with
the dirty diff recorded. The flash plan is `191c73aa…`, with the first image as
its predecessor and a chain of 9 plans. Flashing passed the same checks with a
matching readback
([result](../boards/evidence/jc1060-20261030-com3/s06-rust-load-fix-flash-result.json)).

The card is the owner's exFAT card with a new Rekordbox export, mounted at LBA
2048. Results:

| Run | Result |
|---|---|
| [ROM boot](../boards/evidence/jc1060-20261030-com3/s06-rust-load-first-boot-result.json) | 2 passes; catalog 25 tracks, 1 playlist node; all 25 tracks with analysis |
| [3 app restarts](../boards/evidence/jc1060-20261030-com3/s06-rust-load-app-restarts-result.json) | 10 passes each, 25/25 every pass |
| [Removal mid-run](../boards/evidence/jc1060-20261030-com3/s06-rust-load-removal-result.json) | passes 15–18 at 25/25, then pass 19 failed at import (stage 3); owner `QUIESCED`, client `STOPPED -8`, 0 denied commands, no fault |
| [Same-card reinsertion, 3 app restarts](../boards/evidence/jc1060-20261030-com3/s06-rust-load-reinsert-result.json) | fresh mount, 5 passes each at 25/25 |
| [Cold return](../boards/evidence/jc1060-20261030-com3/s06-rust-load-cold-result.json) | USB power removed for 5–10 s, no BOOT/RESET; `reset_reason=1`, uptime restarted, 18 new passes at 25/25 |

Across every run:
- After each pass, free PSRAM is 30933660 bytes and the PSRAM minimum is
  30449200 bytes. Both stayed identical in all passes, including those after
  reinsertion and the cold return.
- At least 13312 bytes of the client stack stayed free.
- No SD command was denied, because none was attempted.
- There was no panic or watchdog event.

As in the exFAT cold run, the ROM and bootloader output was missed during USB
re-enumeration.

Still open:
- The owner should confirm that the export holds 25 tracks and one playlist
  node.
- ANLZ content for this export has not been compared with the M2.5 parser.
  The 189-track comparison was a different export.
- Only one card has been run. Display, audio, USB host and C6 stay outside this
  profile.
