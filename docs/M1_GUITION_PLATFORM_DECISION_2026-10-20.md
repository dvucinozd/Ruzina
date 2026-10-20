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

Completed software preparation: foundation is integrated into main; both owner-supplied vendor bundles have a [source-pinned hardware matrix](boards/GUITION_HARDWARE_MATRIX_2026-10-20.md), separate Rust board profiles and scheduler-only BSP selectors. Legacy GPIO/peripheral ownership is isolated behind an explicit legacy-board selector. Peripheral runtime acceptance remains open.

1. On arrival, capture exact unit/PCB/panel identity and chip, flash and PSRAM observations. Select the actual panel variant and reconcile USB-FS pad mapping, shared USB power and the required stereo audio output path.
2. Build and boot a minimal Rust image on each board independently. Capture boot log and exact firmware commit/image identity; establish reset and recovery behavior before activating other peripherals.
3. Implement and test memory, display/backlight and touch adapters per board, followed by SD, USB and agreed audio paths. Confirm actual partition bounds before any persistent writes. No legacy M1 bring-up feature is a Guition adapter.
4. Reconnect product functionality, then run relevant host, embedded and physical operator checks. Record acceptance separately for each board and image. Native APTA integration remains deferred by the owner's explicit instruction.

Primary bring-up order between the two models remains unselected. Both are selected development targets; neither is marked hardware-qualified by this decision.
