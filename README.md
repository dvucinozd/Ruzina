# Ruzina

Native Rust DJ firmware and desktop simulator for Guition ESP32-P4 boards.
Development targets are **JC1060P470C_I_W_Y** and **JC8012P4A1C_I_W_Y**, with
v3.2 silicon required by the native target. The minimum configured revision is 302.

## Development checkpoint — 2026-11-01

The observed JC1060 has ESP32-P4 **v1.3/103**, 16 MiB flash and 32 MiB PSRAM.
It uses the separate pinned **ESP-IDF 6.1 + shared Rust** backend. The native
Rust backend still requires revision **302** and must not be flashed to this unit.
JC8012 has no transferable hardware acceptance from JC1060.

The latest accepted playback record in this repository is
[deck-play](docs/reviews/2026-11-01-legacy-deck-play.md): one deck reads the SD
Rekordbox export and plays FLAC/MP3/WAV through the DDJ-FLX4 headphones, alongside
the library display/touch and MIDI/LED control. This is bounded device evidence,
not full product qualification. The [s08/s09 recovery reconciliation](docs/reviews/2026-11-01-review-fix-history-reconciliation.md)
now verifies the complete history and live 16 MiB predecessor image.

`deck-mix` and `deck-engine` add two-deck MAIN/PFL, mixer processing and product
transport. Their PRs #71/#72 were open at the reviewed base. The local
`ogranak/jc1060-review-fixes` branch applies the
[review repair plan](docs/reviews/2026-11-01-review-fix-implementation-plan.md):
transactional LOAD, identified seek receipts, coherent ring descriptors, media
revocation, USB recovery and incremental producer scheduling. Read the
[execution log](docs/reviews/2026-11-01-review-fix-execution.md) for actual test
and build results. Offline steps A–H are complete: all 17 IDF packages and the
host/target checks pass; final CodeRabbit reports 0 issues. Published [PR #73](https://github.com/dvucinozd/Ruzina/pull/73)
passed 19/19 CI jobs. The [first device campaign](docs/reviews/2026-11-01-review-fix-device-results.md)
booted the candidate but failed its registered timing gate (`fill_max_us=12260`,
limit 6000 us), despite idle underruns 0/0. The exact accepted deck-play image
was restored; cold boot, normal sound, Pause and CUE are confirmed again.

The [phase diagnostic](docs/reviews/2026-11-01-uac-timing-device-results.md) has
now measured the outlier: initial prefill maximum 1844 us, callback maximum
10373 us (3 of 6000 callbacks >=6000 us). The aggregate criterion still fails;
substage/preemption cause is not established. The s07 image was restored and
cold-booted; normal sound, Pause and CUE were confirmed again, with zero captured
underruns/USB errors. All monitors are closed.
The later [callback substage run](docs/reviews/2026-11-01-uac-callback-profiling-device.md)
captured valid exclusive records at 13087/13039 us. The worst record includes
6523 us graph and 3725 us PCM packing wall time, with zero observed scheduler
switch deltas. IRQ/cache/observer effects and runtime hook-liveness remain open;
these numbers do not establish pure CPU cost or a fix. A separate 9000-callback
steady window adds no threshold crossings. S07 is restored and owner-confirmed
again, with zero captured underruns/USB errors; all monitors are closed.
Next: follow the [cause experiment plan](docs/reviews/2026-11-01-uac-callback-cause-follow-up.md)
before resuming two-deck MAIN/PFL, control-race, media-loss and USB-reconnect qualification.
Source cues/analysis, loops, scratch, keylock and FX wiring, product
UI/settings, C6 networking and OTA remain separate subsequent development work.
See the [current status](docs/DEVELOPMENT_STATUS.md) and
[handoff](docs/HANDOFF.md). Historical bring-up results retain their own
image hashes and scenario limits in `docs/reviews`.

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
headphones. External DAC detection and I2S output remain planned. FLX4 UAC headphone
playback has bounded device evidence; two-deck MAIN/PFL requires its own campaign.
The [external audio plan](docs/boards/JC1060_EXTERNAL_AUDIO_PLAN.md) also records
a later optional headphone DAC and independent Settings; that extension is not
the current implemented headphone policy.
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
