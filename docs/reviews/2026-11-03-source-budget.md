# Offline source budget and isolated MP3 compiler candidate

This follows the [continuous campaign](2026-11-03-continuous-device.md). No
device was opened, reset or flashed in this investigation. Accepted s07 full
readback/history **44** remain the latest device state; application execution
after the recorded boot pulse remains **NOT_OBSERVED**. Native minimum is
still **302**, and the rev103 experiment stays on its separate IDF path.

## Reassessment of control isolation

The read-only [control audit](evidence/2026-11-03-source-budget/control-audit.json)
checks the original stream/case digests and leaves frozen admissions unchanged.
It found **74 additional messages in S03**: MASTER volume 51, JogTouch 2,
JogScratch 8, JogBend 9, MasterCue 4. It found no extra control pairs in the
other four case slices. Its allowlist describes requested setup pairs, not
their timing/order or full physical acceptance.

The single counter transition 0->8 immediately follows eight deck-2 ScratchJog
messages. A regression through the actual engine reproduces eight unexecuted
scratch effects without refused loads or playback. `unsupported=8` is not
eight media/SD errors. This does not establish who generated the messages or
explain every inactive position update.

The pinned analyzer's four admitted checkpoints remain historical numerical
observations. **S03 did not follow the requested isolated-control protocol**;
do not promote that entire window as an isolated-source acceptance pass.
MASTER/headphone routing isolation was not established. Its complete stable
source intervals remain diagnostic data, including the effects of received
controls. The dual-source underrun and owner's crackling report remain a real
failure. No retrospective slice repair, new PASS, repeated device campaign or
s07 manual return qualification was performed.

## Reproduction and host measurement

The new test-only service model uses actual Producer/SourceDecoder/Ring and
the target scheduling function, shared virtual time, exact 44.1-kHz fractional
consumption during blocking reads, and the existing CC0 eight-second VBR MP3.
Both sources are loaded/primed first. Compute/preemption is zero-cost;
synthetic owner overhead is 100 us/turn. The 16-action/1000-us poll budgets
remain real; an indivisible read may overrun the time budget before yielding.

With injected 40-ms reads, one source survives the five-second bound; two
sources first miss PCM after approximately 4.03 seconds. The first fault ends
the run at the end of the first indivisible read interval that reports missing
frames. Zero-delay cases consume exactly 220500 frames per playing deck
without missing frames. This tests pressure/fairness and read-time budgeting;
**40 ms is not a measured target latency**, and decoder compute is not modeled.
It is not a faithful replay of the original recordings or a target fix proof.
An additional four-second regression caps consumption at the requested bound
even if a blocking read finishes afterward; no post-bound samples may turn a
bounded case into failure. Earlier model output before this correction is
retained as a superseded attempt, not the final simulation result.

The manual release benchmark decodes the same 352800-frame fixture with memory
and sleeping reads. The final matrix runs sequentially, five repetitions per
reader/profile, using Rust 1.95.0. All four exported PCM files match byte for
byte, SHA-256 `2fb60aa3d5dc7243d38208d215fa40c13083d062b51c5eeebca8a88c6fd3ddac`.
Each full decode performs 138 reads. Requested 200-us sleep accumulates about
38-39 ms of actual host read wall time, not the nominal 27.6 ms.

| Host compiler policy | Median decoder-step wall, memory | Median full elapsed, memory | Median full elapsed, delayed reads |
|---|---:|---:|---:|
| Current release `s` | 107378 us | 110537 us | 154312 us |
| Only `nanomp3-core` at 3 | 53331 us | 56851 us | 97278 us |
| Core + MP3 wrapper + audio runtime at 3 | 11331 us | 14634 us | 55950 us |
| Entire release at 3 | 11237 us | 14316 us | 53973 us |

The matrix and exact Cargo commands are retained separately. Host microseconds
are not P4 times or CPU utilization. Current source partitions include waits
and preemption; the simulation deliberately excludes decoder compute. Neither
result alone diagnoses SD hardware, core affinity or CPU saturation.

## Selected single change and implementation

The first candidate changes only the compile optimization level of
**`nanomp3-core 0.2.0` from `s` to `3`**, leaving its source/version and other
packages' defaults intact. It is the smallest measured compiler change, with
roughly 2x host decoder-step improvement. The faster broader host policies
remain comparisons, not bundled candidate changes.

The builder's explicit **`--mp3-core-opt3`** option requires the producer-profile
deck-engine diagnostic. Only Cargo build/Clippy receive
`profile.release.package.nanomp3-core.opt-level=3`; fmt/version queries and
normal/default builds retain their original command policy. The manifest
records `mp3_core_opt3` and `rust_release_overrides`; the candidate keeps verbose
compiler evidence. No Cargo.toml default, decoder algorithm, ring, read buffer,
task, affinity, gain or fault threshold was changed.

The actual P4 opt3 build linked successfully but was **REJECTED** by the
unchanged single-frame guard: `nanomp3_core::Decoder::new` needs **39008 bytes**,
above **24576 bytes**. No manifest/package PASS was produced. Keep that ELF and
build log only as a rejected diagnostic artifact, never a registered candidate.
The USB-fill chain was 3104 bytes, but that does not waive the independent
per-frame failure. Normal default build passes with a 74640-byte load chain
(below 90112) and unchanged default optimization. The final normal package is
`BUILD_ONLY_NOT_QUALIFIED`, not installed or promoted.
An independent post-failure disassembly audit also estimates a **90544-byte**
load chain for opt3, above **90112**. The package builder had already stopped
at the per-frame gate; this second result is a separate offline audit, not a
later builder PASS/FAIL stage. The call path runs through SourceDecoder restart
and MP3 seek into `Decoder::new`, which makes runtime reconstruction relevant,
not only the initial heap allocation. The default constructor's own frame is
16 bytes; opt3 materializes the 39008-byte constructor frame.

CI adds the four read-only audit tests and the test-only Rust reproductions.
It deliberately does not add a success-required build of the already rejected
opt3 option. The explicit opt3 builder remains available for offline stack
investigation, clearly marked rejected; its flag is not target authorization.

## Validation checkpoint

[Validation summary](evidence/2026-11-03-source-budget/validation-summary.json),
[host matrix](evidence/2026-11-03-source-budget/host-matrix.json),
[stack audit](evidence/2026-11-03-source-budget/stack-audit.json),
[rejected build log](evidence/2026-11-03-source-budget/rejected-target-build.log)
and [normal manifest](evidence/2026-11-03-source-budget/normal-build-manifest.json)
retain exact scope. Diagnostic tests: **78 passed**, one manual benchmark
ignored in the ordinary suite; its 40 release measurements ran separately.
Normal engine tests: **71 passed**; Linux IDF tools: **50 passed**; Windows
config guards: **14 passed**; control audit: **4 passed per OS**. Host/target
Clippy and formatting pass. The final normal manifest's 290 source inputs and
eight artifacts were rehashed. Normal app SHA-256 is
`e3c5c8709a0a9cf67166ff6f5d623d8ffc76fec903d9b88b3e831d57fc651c96`.

The matrix uses its retained source checkpoint; the subsequent model boundary
correction did not alter the benchmark function (its bytes were compared).
The normal package and final tests use the corrected model source. Earlier
missing-cmake/esptool environment failures, interrupted slow PCM export and
pre-correction model attempts remain separate from successful final checks.
Review/CI/publication and archive completion are recorded separately after
this initial source checkpoint; this section does not infer their success.

## Continuation contract

1. Inspect the rejected constructor's emitted prologue and its callers,
   comparing default/opt3 disassembly with the pinned `nanomp3-core 0.2.0`
   source. Determine whether reset/construction materializes a large temporary.
   The existing static initial-storage copy only covers initial allocation;
   do not assume it protects later decoder reset/seek construction. Design an
   audited bounded-stack initialization path as a separate change. No enlarged
   stack, weakened frame guard, unsafe zero-initialization assumption or target
   flash is permitted to bypass this failure. Keep dependency version/source
   provenance explicit if a maintained patch is required.
2. Re-run host PCM/seek/EOF/removal/cancellation checks and normal/producer
   target builds after the constructor change. Reassess optimized and default
   frame/call-chain bounds and hashes; review and exact-source CI precede
   registration. Only a passing new build can become a candidate.
3. Then prepare new 44/45 plans from the actual executed 44-plan prefix/full s07
   hash. Pin the new manifest/app/ELF, exact allowed compiler override, source,
   runbook and reviewed execution tools. Existing 42/43 plans are consumed;
   the old frozen source/writer contract does not authorize this new image.
4. Bind and verify separate write/capture/return-boot authorities, dual-copy
   archives and publication. Preserve full-before/span/full-after equality and
   the no-reset writer. New source/image identities need an explicit new
   capture contract; do not edit old frozen contracts to make it fit.
5. Only then run physical cold/idle, individual known MP3s and a bounded dual
   case. Explain required controls before starting; retain and independently
   audit received control events. An extra-control run cannot become isolated
   acceptance merely because its numerical analyzer admitted a slice. Stop on
   the unchanged first fault, preserve failed evidence, and automatically
   restore/readback s07 with separately recorded boot and no repeated manual
   return qualification.

PFL isolation, normal dual-deck/transport/reconnect and soak remain open. No
new target timing, listening improvement or root-cause fix is claimed here.
