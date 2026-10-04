# Pajoniiir M1 ↔ Libapta: native Rust integration handoff

Date: 2026-10-04

## Purpose and scope

Read this as cross-project context and a proposed integration direction. It is not evidence that integration is complete, nor authorization to merge, enable production firmware, remove compatibility, or change either project's accepted architecture. Verify current repositories and discuss any conflicting requirements with your user.

**SIMPLE IS RULE NR 1. Both projects are being rewritten in Rust. Their native integration should use a direct Rust crate dependency, without routing through the C ABI.**

The collaborator supplying this document can contribute to both repositories, including through PRs. Coordinate ownership so concurrent agents do not overwrite each other's work.

## Repositories and inspected evidence

- Libapta: https://github.com/dvucinozd/libapta-audio/tree/rust-rewrite
  - Inspected checkpoint: `c3042eae91d399ab6a684e15ff83ad881567d2d7`.
  - Native core: `rust/`, package `libapta`.
  - Desktop ownership layer: `rust/runtime/`, package `libapta-runtime`.
  - Canonical status: `docs/rust/MIGRATION.md`, especially “Publication retry, numerical boundary and desktop/platform continuation — 2026-10-04”.
  - Usage: `rust/README.md`.
- Pajoniiir: https://github.com/dvucinozd/Pajoniiir-M1/tree/codex/m1-rust-foundation
  - Inspected checkpoint: `0a036abaa70c5a11e4b5e7ebcebe212a484c8a53`.
  - Workspace: `firmware-rust/`.
  - Read `docs/M1_RUST_ARCHITECTURE.md`, `docs/M1_RUST_PRODUCT_REQUIREMENTS.md`, and ADRs 001, 006 and 008.

These are dated inspection anchors, not a claim that either branch is still at that commit. Inspect live state before changing anything. Paths on the originating collaborator's machine are `/home/shome/p/libapta-audio` and `/home/shome/p/Pajoniiir-M1`; use your own checkout paths.

## What Pajoniiir already establishes

1. ADR-001 selects native Rust `no_std` on ESP32-P4. ESP-IDF, FreeRTOS and `esp-idf-sys` must not become permanent application dependencies. Therefore, ESP-IDF integration is not the prerequisite for this consumer. Other Libapta consumers may still require it.
2. ADR-006 establishes immutable provider-neutral `TrackAnalysis`. Deck/UI/Sync/Beat Jump should consume that model rather than provider-specific structures. A playing deck pins an analysis generation until a safe upgrade boundary.
3. `pajoniiir-track-analysis` already uses `#![no_std]` and `#![forbid(unsafe_code)]`. It represents `AptaNative` and `AptaCache` providers, but those enum variants do not establish an implemented provider.
4. At inspection, `TrackAnalysis` contains provider, `u32` generation, BPM ×100 and an optional borrowed beat grid. Beats contain `u32` milliseconds, `u16` phase and `u32` BPM ×100. It does not yet represent the full Libapta analysis graph.
5. The architecture lists a future `pajoniiir-apta-adapter`; that crate was absent from the inspected workspace.
6. ADR-008 requires explicit internal SRAM/PSRAM placement and bounded memory. Rust-native integration does not remove DMA/cache, stack or hardware qualification requirements.

## Proposed dependency direction

```text
Libapta portable Rust core
            |
            | direct Cargo dependency
            v
pajoniiir-apta-adapter
            |
            | immutable provider-neutral analysis
            v
TrackAnalysis → Deck / Sync / Beat Jump / Waveform / UI
```

Keep Libapta independent of Pajoniiir. Own product-specific conversion in Pajoniiir. Use the existing analysis engine; do not create another DSP engine or a generic integration framework.

Use a pinned, reproducible dependency appropriate to repository policy. Avoid machine-specific absolute dependencies in a PR. The firmware path must depend on the portable core, not accidentally acquire the desktop `std` runtime. Desktop tooling may use `libapta-runtime` where useful.

Idiomatic Rust means typed values, explicit units, `Result`/`Option`, slices and safe ownership. Fixed-width integers remain appropriate for sample coordinates, bounded storage and wire formats. Do not replace useful integers with floats merely to appear more Rust-native.

## Work that can leave the native integration critical path

Provided no actual native consumer requires it, defer:

- C ABI entry points, opaque handles and manual C acquire/release APIs.
- Exact C allocator callback classes and allocation failure ordering.
- Binary-equivalent C workspace layouts.
- C static/shared packaging, CMake/pkg-config integration and frozen C consumer replacement.
- Windows C toolchain acceptance as a prerequisite for P4 integration.

This changes the order of work, not the published C compatibility promise. Keep the existing C product, headers and ABI intact. A future C compatibility boundary can be a separate workstream for consumers that need it.

Continue using unchanged C as a behavioral/DSP reference. Preserve `.apta` file compatibility where caches and interchange need it; a stable file format does not require C-shaped in-memory Rust APIs.

## Libapta capabilities and boundaries

The native implementation already integrates waveform/bands/detail, sparse scheduling, overview seeding, source/pull contracts, musical stages and revisions, immutable results, native ownership, explicit sparse capacity reservation, and source identity transport. Desktop CLI supports opt-in SHA-256 source-object hashing/verification, metadata at export and JSON inspection.

Important remaining boundaries include unknown-duration sparse ownership/scheduling, fuller consumer interfaces, embedded memory/stack/timing acceptance and original DSP accuracy gates. See the canonical migration document for exact supported profiles; do not infer completion from this summary.

One numerical parity boundary is understood: for uniform 120 BPM impulses at 8 kHz, 320000 frames and amplitude 0.75, portable selected key score is 55734 versus host C 55735. The audit traced cosine rounding and later log/chroma differences; the portable cosine was closer to the mathematical value at the first divergent coefficient. Do not hard-code a correction, silently swap math backends or weaken fixtures. Determine an explicit numerical acceptance policy separately. C parity alone is not musical accuracy qualification.

Dated Libapta verification at the inspected checkpoint includes 123 host C tests, 271 ordinary Rust tests and 55 external-C groups per debug/release profile. An explicit i686 SSE2 profile executes the same matrix; default x87 has numerical differences. RV32 compilation and a limited stack probe are not P4 firmware acceptance. These results were not Pajoniiir integration tests.

## Suggested first integrated PR

1. Inspect live branches, dirty/untracked files, local instructions and existing issues/PRs. Preserve all ongoing work. Agree which repository and files each collaborator owns.
2. Audit the current consumer model and actual Libapta APIs. Define only the fields needed for the first useful path.
3. Add a small native adapter in Pajoniiir with a reproducible Libapta dependency. Begin with a deterministic host fixture through Libapta into `TrackAnalysis` and existing beat-grid consumers.
4. Specify coordinate and unit conversion: sample frames versus milliseconds, sample rate, BPM precision, rounding, overflow, absent/partial analysis, phase meaning and generation handling. Never silently truncate incompatible generations or coordinates.
5. Establish storage ownership: borrowed views must not outlive their backing result; a pinned generation must remain immutable during ongoing analysis. If conversion needs storage, make its capacity and allocation policy explicit.
6. Test the actual consumer path, including safe generation replacement, missing grid/tempo, boundary coordinates, and relevant failure/backpressure behavior. Extend waveform/key/meter interfaces incrementally where consumers need them.
7. Keep analysis outside the real-time audio callback. Verify bounded work and memory for the chosen scheduling model.
8. Then verify the portable dependency on the real P4 target/toolchain and qualify software integration, memory placement and stack use. Compilation alone is not execution or physical qualification.

Pajoniiir ADR-006 and product requirement M1R-P1-001 currently defer Libapta integration until upstream embedded/release gates pass. If proposing host-only experimental integration before those gates, explicitly scope and document it in the PR. Do not silently enable a production provider or mark those gates passed. Production activation remains subject to the existing acceptance policy unless maintainers explicitly revise it.

## Contracts that still matter without C ABI compatibility

- Invalid floating PCM is rejected before mutation.
- Source buffers have clear lifetimes and are released exactly once where a release protocol is used.
- Processing/publication failures do not expose invalid or mutable retained results.
- Accepted changes, cancellation, retry and backpressure have explicit semantics.
- Frame positions, durations, sample rates and source identity remain consistent.
- Existing Libapta stage order and accepted replay/revision/seed contracts remain intact unless deliberately redesigned and verified.
- Portable core remains `no_std`, allocator-free and unsafe-free. Platform unsafe belongs in a narrowly reviewed boundary.
- Firmware memory budgets, stack use and scheduling need evidence. Physical P4 and musical accuracy qualification remain separate gates.

## Collaboration and completion

Do not expand this into an unsolicited rewrite of Pajoniiir's model or Libapta's engine. Prefer the smallest end-to-end native path, then extend it through demonstrated requirements. Run relevant tests in both repositories and distinguish host acceptance, cross-compilation, target execution and hardware qualification.

When reporting, state what works end-to-end, what remains experimental, what C-only work was deferred, and what production gates remain. Open a focused PR if requested by your user; do not treat this context document alone as permission to publish or merge changes.
