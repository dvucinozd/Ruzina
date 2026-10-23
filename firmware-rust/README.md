# Ruzina Rust workspace

The host workspace contains the reusable DJ product logic and the interactive
Slint simulator. Active hardware targets are Guition JC1060P470C_I_W_Y and
JC8012P4A1C_I_W_Y, ESP32-P4 minimum silicon revision 302.

Released M2.5 is the functional reference. See the [parity plan](../docs/M2_5_PARITY_PLAN.md)
for missing v3/v4 profiles, transport/library features and unfinished services;
existing host-tested foundations do not establish complete release parity.

## Boundaries

- Product crates own semantic controls, controller profiles, decks, media identity,
  Rekordbox analysis, bounded decoding/render/FX and headphone routing.
- `ruzina-board-profile` owns source-audited candidate data, observations,
  memory-test policy and inactive panel/touch adapter contracts.
- `apps/desktop-simulator` uses the shared model and Slint views with simulated
  media/clock and captured diagnostics. It provides no physical audio or USB.
- `apps/m1-firmware` is a separate embedded workspace so HAL dependencies stay
  outside the default host graph. Only its selected Guition BSP may acquire board
  resources. Product-owner modules have a separate compile-only integration gate.
- Optional `ruzina-apta-adapter` converts native analysis behind neutral
  ownership/publication contracts. It is excluded from default product graphs.

## Toolchain and checks

Pinned Rust 1.95.0, Slint 1.18.1, embedded-graphics 0.8.2 and esp-hal 1.2.2 are
recorded in the manifests and committed lockfiles. From this directory:

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
python3 -m unittest discover -s tools/tests -v
python3 tools/check_docs.py
python3 tools/check_apta_core_boundary.py
cargo run --locked -p desktop-simulator -- --board jc1060
cargo run --locked -p desktop-simulator -- --board jc8012
cargo run --locked -p desktop-simulator -- --smoke target/ui-smoke
python3 tools/check_ui_goldens.py target/ui-smoke
```

The host build needs the desktop/window/font dependencies documented in
[host CI](../.github/workflows/rust-host.yml). See [simulator controls](apps/desktop-simulator/README.md).

## Current scope

WAV/MP3/FLAC sources feed the same bounded PCM/history/render path. Loops, scratch,
Master Tempo, FX, PFL and output completion are host tested. The reference sink
uses software timing; physical I2S/UAC cadence and listening remain pending.
Headphones always use controller UAC channels; MAIN prefers a ready external
stereo DAC and otherwise uses compatible controller MAIN audio.

Guition idle/diagnostic/probe images compile independently. The PSRAM diagnostic
uses an exclusive 64 KiB CPU-cached span at 80 MHz, without an allocator. It does
not prove full-array, cache/DMA or production-clock behavior. Panel tables and
touch firmware are not bundled. No product peripheral is activated by a selector.

See [firmware build profiles](apps/m1-firmware/README.md),
[development status](../docs/DEVELOPMENT.md), [validation](../docs/VALIDATION.md)
and the [first-boot procedure](../docs/boards/GUITION_FIRST_BOOT_2026-10-20.md).
