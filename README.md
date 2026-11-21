# Ruzina

Native Rust DJ firmware and desktop simulator for Guition ESP32-P4 boards.
Development targets are **JC1060P470C_I_W_Y** and **JC8012P4A1C_I_W_Y**, with
v3.2 silicon required by the native target. The minimum configured revision is 302.

## Current state — 2026-11-21

**On the board:** the owner's JC1060 (ESP32-P4 v1.3, rev103) runs the legacy path:
- ESP-IDF 6.1, with the shared `no_std` Rust core, in
  [`firmware-idf/legacy-jc1060`](firmware-idf/legacy-jc1060);
- a DDJ-FLX4 for control, MAIN and headphones;
- a Rekordbox export on an exFAT USB stick, behind a hub on the board's HS port.

Which image the board keeps for play, and the last one measured, is on the
[status page](docs/DEVELOPMENT_STATUS.md#the-boards-images). Every point below has a dated
review, collected in the [handoff](docs/HANDOFF.md), with its device evidence: a log with
counters, the owner's observation on the board, or both. The long soaks were scripted or
unattended, logs and counters only; the owner's listening checks are separate, shorter
runs.

- **Playback:**
  - two decks of WAV, MP3 and FLAC;
  - media reads through a read-ahead: 8 KiB per stick read, up to 16 sectors from an SD
    card;
  - QIO 80 MHz flash and 200 MHz PSRAM;
  - indexed MP3 seeks;
  - no underruns in the 35 to 42-minute soaks, the last one from the stick (counters;
    nobody listened).
- **Performance:**
  - CUE, loops and scratch from a played-audio history;
  - SYNC, also continuous;
  - Beat Jump and Quantize;
  - Master Tempo, with its own keylock;
  - Beat FX and Pad FX;
  - Rekordbox and own Hot Cues, in a flash journal;
  - the FLX4's LEDs;
  - soft takeover.
- **Deck screen (Slint):**
  - waveforms with Hot Cue, loop and beat marks;
  - a Library with Rekordbox playlists, and each deck's cover;
  - Hot Cues, FX and Settings pages;
  - Rekordbox memory cues, called from the HOT CUES page or the FLX4's SAMPLER pads;
  - settings kept on flash: LOAD LOCK, VINYL/CDJ, brightness, the MAIN trim and the
    screensaver's wait;
  - a boot screen and a screensaver with the RUZINA wordmark.
- **Media:** the USB stick shares a hub with the FLX4. Pulled out, the Library shows
  MEDIA REMOVED; put back, the app restarts and reads it again. No SD card is needed.
- **Network:** the deck's own Wi-Fi (ESP32-C6 via ESP-Hosted) and the Web Remote, with
  play, cue, library, mixer and SYNC.

**Still to come:** the final layout, with the board's own power on CN2 and the stick on the
FS port. The console is already on UART0 ([service link](docs/boards/JC1060_UART0_SERVICE.md)).

**Open:**
- USB audio fills over the 3 ms target, with no underruns. On the s163 profile image
  (2026-11-21, Master Tempo on both decks), 18.3 % of Master Tempo fills took over 3 ms
  while the screen drew, against 1.7 % with it still, and 88 % of those over 3 ms
  overlapped a screen frame ([fill contention](docs/reviews/2026-11-21-fill-contention.md)).
  Earlier figures, each with its image and workload, are on the
  [status page](docs/DEVELOPMENT_STATUS.md#open). Keeping the screen's frames from slowing
  the fill is in progress.
- Slint's scene preparation is still 16 to 26 ms of a big frame.
- In the SD build only: an SD clock failure at the first boot after a flash.

**Evidence layers:** each claim names its layer.
- **Physical:** this JC1060 on the legacy path.
- **Host and simulator:** [`firmware-rust`](firmware-rust/README.md).
- **Native `esp-hal` firmware** (`apps/m1-firmware`, silicon rev 302 and up): compiles
  only.
- **The JC8012:** not qualified.

The [status page](docs/DEVELOPMENT_STATUS.md) lists each feature with its evidence. The
bring-up campaign of 2026-11-01 to 2026-11-03 is
[archived](docs/reviews/2026-11-03-checkpoint-archive.md).

Pajoniiir **M2.5** is now the released functional reference. The rewrite started
from M2.2; the [M2.5 parity audit and plan](docs/M2_5_PARITY_PLAN.md) records the
remaining differences and separately evaluates newer shared-P4 beta/master work.

## Development boards

| Model | Documented display | Controller / touch | Documented memory |
|---|---|---|---|
| JC1060P470C_I_W_Y | 7 inch, 1024x600 | JD9165 / GT911 | 16 MiB flash, 32 MiB PSRAM |
| JC8012P4A1C_I_W_Y | 10.1 inch, 800x1280 | JD9365 / GSL3680 | 16 MiB flash, 32 MiB PSRAM |

The JC1060 has arrived: its unit is an ESP32-P4 v1.3 (rev103), recorded with the
[COM3 evidence](docs/boards/evidence/jc1060-20261030-com3). The JC8012 is still a
source-audited candidate. See the [hardware matrix](docs/boards/GUITION_HARDWARE_MATRIX_2026-10-20.md).

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
- [Native firmware plan and next steps](docs/DEVELOPMENT.md)
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

For each new board: identify the unit, preserve factory firmware, establish ROM
recovery, then qualify boot, memory, display/touch, storage, USB and audio
independently for each exact image. Host tests and builds do not establish those
physical results.
- [C01/C02 operator preparation and recovery gates](docs/boards/GUITION_C01_C02_READINESS.md)
- [Per-model peripheral ownership cards](docs/boards/GUITION_PERIPHERAL_OWNERSHIP.md)
