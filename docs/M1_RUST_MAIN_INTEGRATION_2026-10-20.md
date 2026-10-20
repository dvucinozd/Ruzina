# M1 Rust foundation integration

Date: 2026-10-20

## Scope

The owner approved correcting the reviewed underflow defect, carrying forward the Guition platform decision, enabling Rust CI on `main`, validating the result and merging foundation through [PR #1](https://github.com/dvucinozd/Pajoniiir-M1/pull/1). The owner explicitly deferred the APTA branch. Its tip `fd7ebd592b469b27c9f45337dced2f9b2912966f` is retained without merging its 24 unique commits.

The starting foundation commit is `396b6fd34b9f273281ffece1ceae58fe392af916`. The [original review](M1_RUST_FOUNDATION_REVIEW_2026-10-20.md) records the failing behavior and historical CI results.

## Underflow repair

Render now retains a per-deck source-starvation state across output frames and block boundaries. Any failed source read clears cached resampler samples but preserves the fractional phase. Interpolation-only frames remain silent until a fresh source read succeeds. A deck reset clears both audio history and starvation state.

This avoids both replaying old PCM and repeatedly resetting the fractional phase, which could prevent a slow source from reaching its next read. Starvation emits silence to MAIN and PFL and increments underflow counters for every affected output frame. The other deck continues independently.

Three new host tests cover sub-unity rate/pitch starvation across blocks, recovery with fresh PCM while the other deck remains active, and source reset. The starvation tests were run against the original implementation and failed before the repair. The independent 24 kHz review probe also passes after the repair.

## Platform and CI changes

- The repository, Rust READMEs, architecture, runtime ADR and product requirements now point to Guition JC1060P470C_I_W_Y and JC8012P4A1C_I_W_Y with owner-reported v3.2 silicon.
- Custom-M1 pin maps, component assumptions, flash partition layout and framebuffer sizes remain historical implementation details requiring explicit mapping to each selected board. No Guition BSP is claimed implemented.
- Both Rust workflows now run on path-filtered `push` to `main` in addition to PRs and manual dispatch.
- The P4 matrix additionally links render + USB0 + USB1 + SDMMC + PSRAM together.
- Rust versions, minimum silicon revision, dependency locks, production feature defaults, firmware images and hardware design files are unchanged.

## Validation

Validation used the pinned Rust 1.95.0 toolchain in Ubuntu/WSL with isolated user-cache toolchain/native dependencies. No system package installation or dependency-lock update was needed.

| Check | Result |
|---|---|
| Host workspace format | PASS |
| Full workspace Clippy, all targets, warnings denied | PASS |
| Full workspace unit tests, including simulator | PASS, 471 tests |
| Desktop simulator compile gate | PASS |
| P4 workflow partition validator | PASS |
| All P4 workflow Cargo commands, including added combined render link | PASS, 20 checks/builds |
| Independent original underflow probe | PASS; no nonzero cached frames after exhaustion |

Validation results, command summary and SHA-256 hashes of the source/workflow/lock files are recorded in `docs/evidence/m1-rust-main-integration-2026-10-20/`. Embedded builds retain pre-existing unused-import/dead-code warnings; they pass compilation/linking, while host Clippy is warning-free. Simulator compilation and its unit tests do not establish GUI runtime or hardware acceptance.

The review found that the previous GitHub checks failed before starting because of account billing/spending limits. The complete local workflow checks above provide current software evidence; final PR and main CI status must still be inspected after push/merge and reported independently.

Hardware bring-up, display/touch, physical USB/audio, exact flash/PSRAM and operator acceptance remain pending for both boards. No device flashing or release publication is part of this integration.
