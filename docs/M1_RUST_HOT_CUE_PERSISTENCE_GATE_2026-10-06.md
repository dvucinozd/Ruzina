# M1 Rust Hot Cue Persistence Gate Closure

**Date:** 2026-10-06  
**Branch:** `codex/m1-rust-foundation`  
**Requirement:** `M1R-P0-082`  
**Evidence state:** `HOST_VERIFIED / COMPILE_VERIFIED / HARDWARE_PENDING`

## Gate decision

The pre-hardware Hot Cue persistence gate is **CLOSED**.

The native Rust M1 implementation now has a durable persistence owner backed by
the ESP32-P4 internal QSPI flash, a versioned Hot Cue storage ABI, explicit
Rekordbox-source/local-override semantics, power-loss-safe journal commits and
fail-closed partition validation.

This closure is a software/compile qualification only. Physical flash endurance,
brownout behavior, real reset timing and reboot persistence on EVT hardware
remain `HARDWARE_PENDING`.

## Product semantics closed by this gate

- Hot Cue state is keyed by the provider-neutral 16-byte `MediaTrackId`.
- Imported/Rekordbox cues and local user changes are kept as separate banks.
- Local overrides take precedence over imported cues.
- A local deletion is represented by an override bit with no slot payload, so a
  deleted imported cue cannot reappear after reload.
- V4 records persist both cue presence and the local override/tombstone mask.
- V3 and legacy V2 decode paths remain explicit migration inputs.
- The bounded 15-byte backend key is never trusted as identity: every V4 record
  carries the full track ID, and a truncated-key collision fails closed on load,
  save and delete.

## Durable-write invariant

The Deck persistence lane obeys this invariant:

> A `PersistHotCues` effect is not dequeued from authoritative product state
> until the persistence owner reports a successful durable journal operation.

A storage initialization or write failure permanently faults the persistence
owner for that boot. The pending effect remains unacknowledged rather than being
reported as successfully persisted.

## Journal design

The journal reserves exactly 1 MiB and uses:

- 128 hash buckets;
- two 4 KiB sectors per bucket;
- 168-byte bounded entries;
- entry CRC plus a commit word written last;
- sector-generation CRC plus a sector commit word written last;
- copy-forward compaction into the inactive sector;
- generation selection that is wrap-safe.

Host fault-injection tests cover interrupted entry writes and interrupted sector
rotation. An uncommitted/torn replacement never supersedes the previous committed
generation. CRC/format corruption of a committed entry fails closed.

## M1 flash layout

The hardware schematic specifies `W25Q128JVPIQ`, 128 Mbit / 16 MiB QSPI flash.

The firmware partition authority is
`firmware-rust/apps/m1-firmware/partitions.csv`:

- partition table: `0x00008000`;
- factory app: `0x00010000 .. 0x00F00000`;
- `hotcue`: data subtype `0x40`, `0x00F00000 .. 0x01000000` (1 MiB).

`espflash.toml` references the CSV directly. CI validates that the factory
partition ends exactly where the Hot Cue partition begins and that the Hot Cue
partition ends exactly at the 16 MiB flash boundary.

The target adapter also reads the actual flashed partition table at runtime
before enabling persistence. It requires exactly one non-encrypted `hotcue`
entry at the expected offset/size/subtype and rejects any overlapping partition.
A missing, duplicate, malformed, overlapping or mismatched layout faults
persistence before erase/write.

## ESP32-P4 integration

The target uses `esp-storage = 0.10.0` over the ESP32-P4 `FLASH` peripheral
with multicore auto-parking enabled for flash operations.

Feature `hotcue-flash-storage` owns this path. Feature
`usb1-controller-bringup` implies it, ensuring controller-originated Hot Cue
changes cannot compile into a configuration that has no durable persistence
owner.

## Verification evidence

The functional implementation HEAD `094e4ccb835d0b257a3158df154f372331a5b21a`
passed both required workflows:

- **Rust host foundation**, run **298**: format, Clippy with `-D warnings`,
  complete workspace unit tests and desktop-simulator compile gate all passed.
- **Rust ESP32-P4 compile gate**, run **264**: partition-layout validation,
  default cross-compile/link, explicit `hotcue-flash-storage` cross-compile,
  USB0, USB1, SDMMC, PSRAM and all combined ownership/link gates all passed.

The immediately preceding functional matrix also demonstrated successful
USB1 + Hot Cue persistence ownership and the full
USB0 + USB1 + SDMMC + PSRAM link configuration.

## Remaining hardware evidence

The following claims are intentionally **not** made by this gate:

- actual W25Q128 erase/program timing on the custom M1 board;
- brownout/power-cut behavior during real flash program/erase;
- endurance under production write patterns;
- persistence across real watchdog/power/reset cycles;
- multicore latency impact during real audio/controller load.

These are part of EVT hardware qualification. Until that evidence exists,
`M1R-P0-082` remains `HARDWARE_PENDING` in addition to its host and compile
verification.
