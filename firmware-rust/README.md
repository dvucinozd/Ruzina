# Pajoniiir-M1 native Rust firmware

This directory is the new native Rust software workspace for Pajoniiir-M1.

The released Pajoniiir M2.2 product is the functional golden reference. Active hardware targets are Guition **JC1060P470C_I_W_Y** and **JC8012P4A1C_I_W_Y**, with owner-reported ESP32-P4 v3.2 silicon. See the [platform decision](../docs/M1_GUITION_PLATFORM_DECISION_2026-10-20.md). Custom M1 hardware contracts are historical and do not define these boards' wiring.

## Current phase

Latest audio work: [streamed PCM loop execution](../docs/M1_RUST_PCM_LOOP_GATE_2026-10-21.md) repeats WAV regions without a steady-wrap generation reset, maps confirmed media position through pitch/Master Tempo/scratch/Censor, and wires SetLoop/ClearLoop into both PCM target variants. Physical rewind/output timing and listening remain separate gates.

Latest reliability work: [composed Library/source/transport/cue-write failure policy](../docs/M1_RUST_COMPOSED_FAILURE_GATE_2026-10-21.md) contains ordinary seek failures to one deck, keeps a faulted persistence owner responsive, fences stale completions and preserves unconfirmed cue banks. Physical driver recovery remains a separate gate.

Integration review: [development stack corrections and validation](../docs/M1_RUST_MAIN_REVIEW_GATE_2026-10-21.md) closes the confirmed shared-cue, EOF-counter and early media cleanup regressions before main consolidation. APTA remains separate.

Latest service: [composed runtime owner publication and recovery](../docs/M1_RUST_OWNER_HEALTH_GATE_2026-10-21.md). Existing product/controller/media loops have opt-in legacy health wiring and composed host failure tests. Guition peripheral ownership remains inactive; its boot diagnostics still repeat an immutable snapshot, not a heartbeat.

Latest mixer/FX: [mixer/PFL, headphone and Beat/Pad FX host controls](../docs/M1_RUST_MIXER_FX_UI_GATE_2026-10-21.md) use authoritative shared state and 24 reviewed pixel goldens. Actual audible DSP/output and device UI qualification remain open.

Latest UI: [interactive decks, Library, Hot Cues and session Settings](../docs/M1_RUST_INTERACTIVE_UI_GATE_2026-10-21.md), with the shared waveform rasterizer and reviewed headless goldens for both documented board sizes. See [simulator controls and scope](apps/desktop-simulator/README.md).

Latest analysis work: [generation-bound Rekordbox DAT/EXT publication](../docs/M1_RUST_REKORDBOX_ANALYSIS_GATE_2026-10-21.md) supplies Sync, Beat Jump, quantized loops and neutral waveform/cue views. Host inspection: `cargo run --locked -p pajoniiir-rekordbox --example analysis_inspect -- FILE.DAT FILE.EXT`.

Latest: [completed-output transport progress](../docs/M1_RUST_PLAYBACK_PROGRESS_GATE_2026-10-21.md) connects canonical PCM through delayed output confirmations to position/EOF. The reference target uses the same completion bookkeeping; its timer and physical driver gaps remain explicit.

The [consumer-clock delivery gate](../docs/M1_RUST_SINK_DELIVERY_GATE_2026-10-20.md) adds bounded MAIN/controller-headphone queues and a headless render-to-consumer example: `cargo run --locked -p pajoniiir-audio-runtime --example clocked_output`. Actual embedded I2S/UAC pacing and driver integration remain open.

The workspace retains host-tested product crates and a separate P4 integration target:

- `desktop-simulator` proves the Slint UI development path without ESP32-P4 hardware.
- `pajoniiir-core` contains shared product identifiers/state primitives.
- `pajoniiir-controller-core` establishes semantic controller events.
- `pajoniiir-ui-model` establishes immutable UI snapshot boundaries.
- `pajoniiir-waveform` establishes a no_std RGB565 waveform rasterizer.

`apps/m1-firmware` is a standalone embedded workspace with feature-gated storage, controller and software audio owners. `pajoniiir-board-profile` records audited candidate data separately for JC1060 and JC8012. Their explicit selectors start scheduler-only BSP skeletons; display/touch, audio sinks and peripheral qualification remain pending. Historical M1 tasks and wiring are isolated in `src/platform/legacy_m1.rs` and require `board-legacy-m1`. They cannot be activated through either Guition selector. See the [hardware matrix and source audit](../docs/boards/GUITION_HARDWARE_MATRIX_2026-10-20.md).

The native Rust foundation is integrated into `main`; `codex/native-apta-consumer` remains a separate, unmerged analysis-integration branch.

The [PCM history/routing gate](../docs/M1_RUST_AUDIO_HISTORY_ROUTING_GATE_2026-10-20.md) implements bounded live source arbitration in the legacy render integration and a host-tested modular output router. Headphones always use controller UAC. Guition `board-diagnostics` is an opt-in service-only first-boot report, not peripheral or audio acceptance.

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
cargo test --locked -p pajoniiir-audio-runtime --test loop_playback
cargo test --locked -p pajoniiir-service --features runtime-owners --test composed_failures
cargo run --locked -p desktop-simulator -- --board jc1060
cargo run --locked -p desktop-simulator -- --board jc8012
cargo run --locked -p desktop-simulator -- --smoke target/ui-smoke
python3 tools/check_ui_goldens.py target/ui-smoke
```

The simulator selects documented 1024×600 or 800×1280 geometry; decks stack in portrait. It uses synthetic demo media and a host transport clock, with no physical audio or peripherals.

## Verification rule

Host success must not be reported as hardware success.

Use these states in development evidence:

`HOST_VERIFIED -> SIMULATOR_VERIFIED -> COMPILE_VERIFIED -> HARDWARE_PENDING -> HARDWARE_VERIFIED`

See:

- `../docs/M1_RUST_PRODUCT_REQUIREMENTS.md`
- `../docs/M1_RUST_ARCHITECTURE.md`
- `../docs/adr/`
