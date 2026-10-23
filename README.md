# Ruzina

Native Rust DJ firmware and desktop simulator for Guition ESP32-P4 boards.
Development targets are **JC1060P470C_I_W_Y** and **JC8012P4A1C_I_W_Y**, with
owner-reported v3.2 silicon. The minimum configured revision is 302.

The boards are in transit. Product logic, simulation and target compilation are
verified separately; physical boot, peripherals and audible output are pending.
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

1. Correct PDB title lookup and complete S3CP v2/v3/v4 tooling/runtime compatibility.
2. Complete CUE preview, VINYL/CDJ, shared load lock and source/memory/loop cue semantics.
3. Add playlist hierarchy, bounded artwork/PWV4 and predictable cold MP3 seeks.
4. Complete durable settings, Web/network and signed OTA state machines, then
   integrate and qualify physical UAC/DAC and per-board product owners.

These are open tasks, including behavior already present in the C M2.5 release.
Later headphone-level, reconnect and MT/cache repairs supply additional regression
contracts. Extra controllers and Pro DJ Link remain separate optional scope.

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
