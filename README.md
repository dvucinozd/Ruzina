# Ruzina

Native Rust DJ firmware and desktop simulator for Guition ESP32-P4 boards.
Development targets are **JC1060P470C_I_W_Y** and **JC8012P4A1C_I_W_Y**, with
v3.2 silicon required by the native target. The minimum configured revision is 302.

The first JC1060 arrived on 2026-10-30. Readout on COM3 identifies **v1.3**,
which the native Rust HAL does not support. See the
[arrival evidence and separate ESP-IDF assessment](docs/boards/JC1060_ARRIVAL_2026-10-30.md).
An isolated ESP-IDF 6.1 / shared Rust probe built, linked and executed from RAM.
Four scalar Rust calls matched expectations. Warm reset after that RAM mode
failed; a complete power disconnect restored ROM/factory boot, and the owner
confirmed the factory display works. Native firmware has not been flashed.
The separate [S4 IDF diagnostic campaign](docs/reviews/2026-10-30-legacy-s4-runtime.md)
has now programmed and readback-verified a flash-backed scalar image. It boots,
reports monotonic heartbeats and passed three application restarts; reset and
reconnect scenarios are recorded independently in that report.
The separate [S5 memory/task probe](docs/reviews/2026-10-30-legacy-s5-memory-task.md)
observed32MiB PSRAM at80MHz, bounded scratch/allocator checks, preempted C/Rust
FP loops and the existing EQ benchmark. These are diagnostic measurements;
DMA, full product timing and peripherals remain unqualified.
The isolated [S6 display/touch diagnostic](docs/reviews/2026-10-30-legacy-s6-display-touch.md)
now initializes the New_Panel candidate and GT911. The owner confirmed the
static color order/corners; touch coordinates and lifecycle are recorded in
that report. It is not yet the product UI renderer.
The separate [read-only SD/exFAT diagnostic](docs/reviews/2026-10-30-legacy-s6-sd-readonly.md)
has mounted the owner's64GB card and enumerated its root without creating
files. This diagnostic temporarily replaces the static display profile;
file/audio streaming and product storage integration remain unqualified.
The [known-file extension](docs/reviews/2026-10-30-legacy-s6-sd-file.md) now
verifies an8MiB+513 UTF-8-path fixture byte-for-byte and by SHA-256, EOF and
boundary seeks, including active-read removal/reinsertion smoke. It is still
a standalone diagnostic rather than a wired product media pipeline.
The [read-only Rust media bridge protocol](docs/reviews/2026-10-30-legacy-s6-rust-media-bridge.md)
now has host coverage for owned-buffer cancellation and media generations.
Its [concrete IDF transport](docs/reviews/2026-10-30-legacy-s6-idf-media-transport.md)
runs a separate raw-block diagnostic on the board, with first-boot and three
app-restart hash observations. Idle removal and same-card reinsertion were
observed, with matching raw reads after an owner-confirmed cold return (initial
boot prefix missed). A subsequent [shared Rust partition/exFAT profile](docs/reviews/2026-10-30-legacy-s6-rust-exfat.md)
now mounts the same card and verifies the full UTF-8 fixture SHA/EOF/seek cases,
with three first-boot passes and three app-restart observations. This is still
an isolated diagnostic. Open-file removal/reinsertion and a full fixture pass
after owner-confirmed cold return are observed on its exact image (initial boot
prefix missed). Product loading/executor and broader media qualification remain
separate gates.
Product logic, simulation and target
compilation are verified separately; product boot, peripherals and audible
output remain pending. JC8012 has no physical observations yet.
Pajoniiir **M2.5** is now the released functional reference. The rewrite started
from M2.2; the [M2.5 parity audit and plan](docs/M2_5_PARITY_PLAN.md) records the
remaining differences and separately evaluates newer shared-P4 beta/master work.

## Development boards

| Model | Documented display | Controller / touch | Documented memory |
|---|---|---|---|
| JC1060P470C_I_W_Y | 7 inch, 1024x600 | JD9165 / GT911 | 16 MiB flash, 32 MiB PSRAM |
| JC8012P4A1C_I_W_Y | 10.1 inch, 800x1280 | JD9365 / GSL3680 | 16 MiB flash, 32 MiB PSRAM |

These are source-audited candidates. Actual unit, panel, chip and memory
identity must be recorded after arrival. See the [hardware matrix](docs/boards/GUITION_HARDWARE_MATRIX_2026-10-20.md).

## Software available

- Two-deck semantic transport, controller profiles, Hot Cues, loops, Sync and Beat Jump.
- Bounded WAV/MP3/FLAC source decoding, PCM history, Master Tempo, scratch,
  mixer, Beat/Pad FX and separate MAIN/headphone render paths.
- Rekordbox PDB and DAT/EXT analysis with media/load generation fencing.
- Interactive Slint simulator, RGB565 waveform rendering and UI screenshot goldens.
- Bounded service/health observations and optional analysis diagnostics.
- Per-board diagnostic packages, read-only flash RDID, an opt-in PSRAM scratch
  test, and inactive display/touch adapter contracts.

The default firmware runs the scheduler. Guition diagnostics do not start
product USB, storage or audio owners. `integration-check` compiles reusable owner
modules without acquiring their board resources or starting them. It cannot be
combined with a Guition selector.

Experimental APTA remains excluded from simulator/default firmware dependencies.
The original `codex/native-apta-consumer` branch remains unmerged.

## Next software work

PDB titles, S3CP v2/v3/v4, CUE/VINYL/CDJ, load lock, playlists, artwork/PWV4
and indexed MP3 seeking have shared implementations and host coverage.
The simulator serves the Web Remote page and status/control/library/load APIs.

HTTP profile/network configuration and diagnostics are connected to host owners;
incremental signed OTA has HTTP/A-B composition tests with explicit synthetic
trust. The demo simulator leaves OTA upload unavailable until a product
trust/project/slot adapter is selected. Next: extend permitted real-media fixtures,
then wire and qualify the per-board product runtime. The [current status matrix](docs/DEVELOPMENT_STATUS.md)
distinguishes host, simulator, target compilation and physical evidence.
Extra controllers and Pro DJ Link remain separate optional scope.

## Audio policy

MAIN automatically prefers a present, initialized external stereo DAC, then a
compatible controller's MAIN output; otherwise MAIN is muted.
**Headphones/CUE/PFL always use the controller.** MAIN switching does not reroute
headphones. Physical DAC detection, channel mapping and I2S/UAC output are pending.
The boards' onboard mono speaker codec is not the stereo MAIN/headphone path.

## Build and simulate

Use the committed Rust 1.95.0 toolchain and lockfiles. From `firmware-rust`:

```sh
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo run --locked -p desktop-simulator -- --board jc1060
cargo run --locked -p desktop-simulator -- --board jc8012
python3 tools/check_docs.py
```

Linux simulator builds need the platform font/window libraries installed by
[host CI](.github/workflows/rust-host.yml). For P4, from
`firmware-rust/apps/m1-firmware`:

```sh
rustup target add --toolchain 1.95.0 riscv32imafc-unknown-none-elf
cargo +1.95.0 build --locked --release --features board-jc1060,board-diagnostics
cargo +1.95.0 build --locked --release --features board-jc8012,board-diagnostics
```

Build each model separately. The [first-boot package tool](firmware-rust/tools/guition_first_boot.py)
retains per-model ELFs, source/build identity and hashes before another build can
replace the target output. These are compile instructions; no programming layout
or flash command is selected.

## Documentation

- [Start here: agent handoff and exact continuation branch](docs/HANDOFF.md)
- [Development status and next steps](docs/DEVELOPMENT.md)
- [M2.5 parity audit, priorities and acceptance](docs/M2_5_PARITY_PLAN.md)
- [Architecture](docs/M1_RUST_ARCHITECTURE.md)
- [Product requirements](docs/M1_RUST_PRODUCT_REQUIREMENTS.md)
- [First boot and diagnostic packages](docs/boards/GUITION_FIRST_BOOT_2026-10-20.md)
- [Board source fingerprints](docs/boards/source-manifest.json)
- [Panel/touch source fingerprints](docs/boards/guition-display-sources-2026-10-21.json)
- [Validation scope](docs/VALIDATION.md)
- [Architecture decisions](docs/adr/)
- [Rust workspace](firmware-rust/README.md)

## Repository layout

```text
firmware-rust/apps/       desktop simulator and P4 firmware
firmware-rust/crates/     reusable product, media, audio, controller and board modules
firmware-rust/tools/      validation, stress and diagnostic packaging tools
docs/                    current product, board and development documentation
.github/workflows/       host and P4 checks
```

After the boards arrive: identify each unit, preserve factory firmware, establish
ROM recovery, then qualify boot, memory, display/touch, storage, USB and audio
independently for each exact image. Host tests and builds do not establish those
physical results.
- [C01/C02 operator preparation and recovery gates](docs/boards/GUITION_C01_C02_READINESS.md)
- [Per-model peripheral ownership cards](docs/boards/GUITION_PERIPHERAL_OWNERSHIP.md)
