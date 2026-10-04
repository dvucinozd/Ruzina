# Experimental native APTA provider

Direct pinned Cargo dependency on the portable `libapta` core. This crate is
`no_std`, allocator-free and unsafe-free. No C ABI, desktop runtime, IDF or
second DSP engine is involved. ADR-006 / M1R-P1-001 production gates remain in
force: neither the firmware nor the desktop simulator enables this provider.

Run from `firmware-rust`, with an external `CARGO_TARGET_DIR` and two build jobs:

```sh
cargo test --locked -p pajoniiir-apta-adapter
cargo run --locked -p pajoniiir-apta-adapter --example analyze_wav -- input.wav output.ppm
cargo check --locked -p pajoniiir-apta-adapter --lib --target riscv32imafc-unknown-none-elf
```

The host example decodes WAV, runs the existing core outside any audio callback,
converts analysis and renders an 800x128 whole-track waveform through the existing
RGB565 rasterizer. It creates a new PPM only. Its 256 MiB input and 65,536-beat
caps are host example policy, not firmware budgets or streaming file support.

## Contract

- `convert` takes a trusted session snapshot/owned-result view, not unvalidated
  externally assembled graphs. It validates the consumer's representable subset;
  it is not a replacement for Libapta's complete external graph validation.
- Fractional Q32 sample positions use the source rate and round once to nearest
  millisecond, ties upward. Results exceeding `u32` and coincident rounded beats
  are errors. Millibpm rounds to BPM x100, ties upward, with wide arithmetic.
- Final continuous single-segment or explicit grids are accepted with one final
  full-source quarter-note meter. Phase is zero-based relative to the actual
  meter downbeat ordinal, modulo its numerator. The selected grid must contain
  that exact downbeat whole-frame/ordinal pair; local meter cannot silently supply
  phase to an unrelated global grid. No implicit four-beat meter.
  Segment beats retain nominal tempo; explicit beats use the selected track tempo
  (zero when absent), without claiming an instantaneous tempo estimate.
  Explicit ordinal gaps, sparse coverage, hybrid authority and multiple segments
  are unsupported. A global grid takes precedence; unsupported global data does
  not silently fall back to a different local interpretation.
- Missing/non-final grid or meter, unknown duration, or provisional phase yields
  tempo only. Missing tempo is represented by neutral BPM zero; existing Deck
  fallback policy applies. State/confidence are not promoted into accuracy claims.
- `u64` native generations must fit a nonzero `u32`. Converted `Analysis` retains
  source fingerprint/geometry and lineage separately from `TrackAnalysis`.
  `upgrade` requires a stopped deck, matching provider and present identity/geometry/lineage,
  and a strictly newer generation. The caller supplies actual deck play state.
  New track loads or provider changes establish a new pin. Absent fingerprints cannot authorize an
  upgrade; supplied fingerprints are transport, not proof of hashing.
- Conversion preflights before writing; short capacity or later coordinate errors
  preserve the destination. Keep two caller storage banks for staging and pinned
  analysis. Borrowing prevents mutation while a view is retained. The converted
  view survives destruction of the original session/cache; no pointers into the
  original result are retained. Existing neutral Deck APIs do not themselves
  enforce pinning: the worker/product owner must retain the selected `Analysis`.
- `convert_cache` validates bytes with caller-specified Libapta limits, requires
  an expected present source identity, and supports the local-grid cache profile.
  Global/hybrid caches return an explicit error. Cache generations are assigned by
  the host; `.apta` does not persist native session generations/lineage. Cache and
  live grids are compared exactly by the integration test.
- `waveform` converts a final continuous overview for the existing whole-track
  renderer. Q15 magnitude maps to full-range u16, nearest rounding; -32768
  saturates. The caller chooses RGB565 color. Sparse/detail viewport geometry and
  band-color interpretation remain separate work.

## Storage and scheduling

Caller beat storage is `12 * capacity` bytes on the tested host/RV32 layout;
converted waveform storage is `4 * columns`. Two live beat banks double that
storage. The core's 4096 onset bins and 4096 f32 flux entries use 80 KiB before
queue, overview, session object, retained results and stack. The host example
prints its concrete sizes. No large inline beat arrays exist in the library.

PCM tests use a 256-frame queue and `maximum_steps=1`, preserving retry at the
unaccepted suffix. They cover full-queue backpressure, invalid PCM before
mutation, cancellation, conversion rollback, immutable retention, Deck Sync /
Beat Jump, cache identity and actual raster output. Conversion takes two bounded
passes over output beats; cache validation has its separate configured limits.
A step budget bounds cooperative work, not elapsed real-time latency.

P4 cross-compilation and dependency inspection establish a portable software
boundary only. The actual worker, PSRAM banks, internal SRAM scratch placement,
DMA/cache discipline, whole-call stack and timing, and physical target execution
are still required. No firmware deployment or hardware operation is part of this
experiment. Libapta's portable key rounding boundary and original musical
accuracy gates remain open independently of this adapter.

## Dated software verification — 2026-10-04

Evidence: `/home/shome/.local/share/Pajoniiir-M1/apta-evidence/`, final stamp
`20261004T172847Z`; `verify-consumer.py` reproduces serial checks with two jobs.
Rust 1.95.0, committed lockfile and the pinned remote Libapta dependency were used.

- Full workspace: **364 tests**, including the desktop simulator's existing tests.
- Focused adapter/Deck/TrackAnalysis/waveform release run: **74 tests**.
- Adapter ASan/leak run: **4 integration tests + 1 lifetime compile-fail doctest**.
- Workspace formatting and Clippy with warnings denied pass.
- Actual P4 `riscv32imafc-unknown-none-elf` library check and optimized object
  generation pass. The normal dependency graph excludes std/Libapta runtime,
  alloc, IDF and RTOS application dependencies. This is not a firmware link/run.
- WAV example: 12000 BPM x100, 80 beats; actual 800x128 RGB565-to-PPM output has
  4,120 green pixels. A separate silence WAV yields BPM zero/no grid and only the
  renderer's 800-pixel centerline. `wav-example-*` and `silence-example-*` retain
  output; `fixture-20261004T172847Z.png` is the inspected raster artifact.
- RV32 layout: Beat 12 B, rendered column 4 B, onset bin 16 B, Session 3536 B,
  SessionSnapshot 3768 B (`p4-layout-*`). Host Session is 3648 B. These are object
  sizes, not simultaneous live-memory/stack maxima.
- RV32 LLVM individual stack frames (`p4-stack2-*`): conversion 336 B, its beat
  closure 304 B, milliseconds 112 B, cache conversion 2480 B, waveform 0 B,
  upgrade 16 B. These exclude nested callees, caller result placement, interrupts
  and task/runtime costs. Full task-stack acceptance remains open. The first
  stack-tool lookup failed because the version-named toolchain lacked LLVM tools;
  the corrected report uses the installed stable LLVM reader on the same object.

The inherited catalog formatting change is already on the foundation branch;
this PR does not change collaborator firmware/catalog implementation. The Slint
build's missing development metadata was supplied in an external user-local
package extraction for host verification; no host service/system installation.
