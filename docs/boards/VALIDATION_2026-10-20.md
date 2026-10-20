# Guition profile/BSP preparation validation

Date: 2026-10-20

Base: `f715a65c18f4ad9ce374816c507a3db30908b653` (`main`). Work is isolated on `ogranak/guition-board-profiles`. APTA remains at `fd7ebd592b469b27c9f45337dced2f9b2912966f`; none of its 24 unique commits is integrated.

## Implemented scope

Both supplied vendor packages were examined, including schematic visual review and the special P4V3X demo branches. The [matrix](GUITION_HARDWARE_MATRIX_2026-10-20.md) records candidate values, explicit conflicts and missing physical evidence. The [manifest](source-manifest.json) pins 25 selected source/comparison files, including 15 local owner-supplied files.

The `no_std` board-profile crate records both models independently. The embedded platform layer has explicit mutually exclusive selectors and isolated historical M1 ownership. Guition selectors link scheduler-only skeletons. There are no Guition display, touch, SD, USB, codec or persistent-flash drivers activated by these selectors.

## Checks

All checks below ran locally using pinned Rust 1.95.0 and the committed dependency graphs with `--locked`:

| Check | Result |
|---|---|
| Host workspace format | PASS |
| Full host workspace Clippy, all targets, warnings denied | PASS |
| Full host workspace tests | PASS, 472 tests |
| Desktop simulator compile | PASS |
| P4 partition validator | PASS |
| P4 workflow Cargo matrix | PASS, 25 checks/builds |
| JC1060 and JC8012 independent cross-compile/release link | PASS, included in matrix |
| Invalid board/legacy-feature combinations | PASS, 18 expected compile rejections |
| Supplied local source-hash replay | PASS, 15 files |
| External dependency-version preservation | PASS; only the new local board-profile package is added to locks |
| Hardware design edits | None |

Logs are retained in [preparation evidence](../evidence/guition-board-preparation-2026-10-20/). Embedded builds can retain the earlier unused-import/dead-code warnings; no such warning is treated as hardware evidence. Host Clippy is warning-free. The default feature set remains empty and silicon minimum remains 302.

## Remaining physical work

Boards are in transit. No boot, reset/recovery, flash/PSRAM identification, panel/touch, USB host power, SD, codec, stereo MAIN/PFL, GUI runtime, audio listening or integrated load test ran. The profile memory check compares reported values against documented expectations; it does not establish wiring correctness or authorize a flash layout.

Before enabling peripheral adapters, identify the exact panel, reconcile the raw schematic/BSP USB GPIO labels with the v3.2 module/package, establish shared USB host-power behavior, and choose a qualified stereo MAIN/headphone route. GSL3680 firmware is not bundled and its distribution/provisioning path remains open.
