# Pajoniiir-M1 native Rust firmware

This directory is the new native Rust software workspace for Pajoniiir-M1.

The released Pajoniiir M2.2 product is the functional golden reference. Active hardware targets are Guition **JC1060P470C_I_W_Y** and **JC8012P4A1C_I_W_Y**, with owner-reported ESP32-P4 v3.2 silicon. See the [platform decision](../docs/M1_GUITION_PLATFORM_DECISION_2026-10-20.md). Custom M1 hardware contracts are historical and do not define these boards' wiring.

## Current phase

The workspace retains host-tested product crates and a separate P4 integration target:

- `desktop-simulator` proves the Slint UI development path without ESP32-P4 hardware.
- `pajoniiir-core` contains shared product identifiers/state primitives.
- `pajoniiir-controller-core` establishes semantic controller events.
- `pajoniiir-ui-model` establishes immutable UI snapshot boundaries.
- `pajoniiir-waveform` establishes a no_std RGB565 waveform rasterizer.

`apps/m1-firmware` is a standalone embedded workspace with feature-gated storage, controller and software audio owners. Its current hardware bindings still describe the historical custom M1 board. Guition BSPs, display/touch, audio sinks and exact-board peripheral qualification remain pending. Do not flash custom-board bring-up features onto Guition boards before their pin, memory and power contracts are established.

The native Rust foundation is being integrated into `main`; `codex/native-apta-consumer` remains a separate, unmerged analysis-integration branch.

## Toolchain

Starting baseline:

- Rust 1.95.0
- Slint 1.18.1
- embedded-graphics 0.8.2
- esp-hal 1.2.2 for the ESP32-P4 target integration
- target `riscv32imafc-unknown-none-elf`

The host workspace and standalone ESP32-P4 firmware `Cargo.lock` files are committed. CI resolves both graphs with `--locked`; dependency changes therefore require an explicit reviewed lockfile update.

## Host commands

From this directory:

```bash
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo run -p desktop-simulator
```

The simulator window is fixed at the product's initial 800x480 UI size.

## Verification rule

Host success must not be reported as hardware success.

Use these states in development evidence:

`HOST_VERIFIED -> SIMULATOR_VERIFIED -> COMPILE_VERIFIED -> HARDWARE_PENDING -> HARDWARE_VERIFIED`

See:

- `../docs/M1_RUST_PRODUCT_REQUIREMENTS.md`
- `../docs/M1_RUST_ARCHITECTURE.md`
- `../docs/adr/`
