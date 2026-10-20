# M1 development platform decision

**Date:** 2026-10-20
**Status:** Accepted project direction; board bring-up pending

## Decision

Discontinue development of the custom Pajoniiir-M1 PCB. Continue native Rust development of the standalone dual-deck DJ system on these Guition boards:

| Exact model | Selected silicon | Evidence available in this session |
|---|---|---|
| JC1060P470C_I_W_Y | ESP32-P4 v3.2 | Project-owner report; exact unit not interrogated |
| JC8012P4A1C_I_W_Y | ESP32-P4 v3.2 | Project-owner report; exact unit not interrogated |

The owner corrected an initial v3.1 statement to v3.2. This document records the corrected decision. Board model alone must not be used to infer the silicon revision of other units or batches.

Keep the custom PCB sources, local CAD changes and engineering evidence for reference. Custom-PCB placement, routing, Gerber generation and EVT fabrication are no longer active milestones. Historical manufacturing blockers remain records of the unfinished PCB; they do not block development on purchased boards.

## Verified repository baseline

The following branch tips were inspected before the approved foundation integration on 2026-10-20:

| Branch | Commit | Relevant contents |
|---|---|---|
| `main` | `56175db33f204afd886869df6bd632f3e9f41996` | Custom-PCB design and historical handoff |
| `codex/m1-rust-foundation` | `396b6fd34b9f273281ffece1ceae58fe392af916` | Native Rust product crates, simulator and P4 firmware target |
| `codex/native-apta-consumer` | `fd7ebd592b469b27c9f45337dced2f9b2912966f` | Separate native analysis integration work |

Both Rust branches set `ESP_HAL_CONFIG_MIN_CHIP_REVISION = "302"` in `firmware-rust/apps/m1-firmware/.cargo/config.toml`. The foundation target pins `esp-hal = 1.2.2` and builds for `riscv32imafc-unknown-none-elf`. The owner-reported v3.2 silicon matches the existing minimum-revision policy; no reduction to 301 is needed.

This revision-policy match establishes neither a working Guition BSP nor display, touch, USB, storage or audio support. No build, flash, hardware test or branch merge was performed as part of this documentation change.

## Software boundary

Retain native Rust and the existing host-testable product/domain architecture. Place board-specific pins, clocks, memory configuration, reset sequencing and peripheral ownership behind explicit board profiles/BSPs for the two exact models.

The custom M1 schematic and GPIO contract remain authoritative only for the historical custom board. In particular, the existing USB0 HS / USB1 FS topology, microSD GPIO39..46 wiring, PCM5102A output, display/touch wiring and 16 MiB flash partition layout must each be checked against the selected Guition board before use. Physical availability of the required USB ports, VBUS control and audio connections must be established from that board's schematic and hardware.

Reuse product crates independently of board wiring. Evaluate panel/touch initialization sources by exact board revision and preserve source provenance and any required notices. Firmware payload distribution rights must be established before importing payloads. Prior work in another project is a source candidate, not qualification of this Rust target.

## Next development sequence

1. Collect the exact board-revision schematics, vendor examples and component identities for both models. Record flash/PSRAM size and mode, display controller and timing, touch controller, SD wiring, wireless companion, accessible USB controllers, VBUS circuitry and audio path. Leave unknown fields open rather than inheriting the custom-board values.
2. Integrate `codex/m1-rust-foundation` into `main` after the reviewed underflow repair and validation. The project owner approved this sequence on 2026-10-20 and explicitly deferred merging `codex/native-apta-consumer`. Keep its unique work and all local KiCad changes intact.
3. Add explicit profiles/BSP boundaries for both exact models. Keep dependency locks and silicon minimum unchanged unless a documented compatibility issue requires a change. Confirm partition bounds against the actual flash before enabling persistent writes.
4. Build and boot a minimal Rust image on each board independently. Capture detected chip revision, flash/PSRAM identity, boot log and exact firmware commit/image identity. Establish reset and recovery behavior.
5. Bring up display/backlight and touch per board, followed by required storage, USB and audio paths. Record unavailable interfaces or any required adapters against the product requirements before enabling the existing custom-M1 bring-up features.
6. Reconnect product functionality and native analysis integration, then run the relevant host checks, embedded builds and physical operator tests. Record acceptance separately for each board and each image.

Primary bring-up order between the two models remains unselected. Both are selected development targets; neither is marked hardware-qualified by this decision.
