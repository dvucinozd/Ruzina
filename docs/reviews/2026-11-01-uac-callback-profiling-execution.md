# Callback substage and scheduler profiling: execution

Date: 2026-11-01. Implements the [callback profiling plan](2026-11-01-uac-callback-profiling-plan.md).
Current implementation is diagnostic instrumentation, not a timing fix. Board
state entering this work is the owner-confirmed s07 deck-play image. Native
minimum 302, accepted gain, task priorities and DSP behavior remain unchanged.

## Diagnostic isolation

`CONFIG_RUZINA_UAC_PROFILE` defaults off and requires the deck engine, the pinned
non-SMP kernel and no competing ESP trace provider. Rust feature
`idf-uac-profile` selects the matching instrumentation. The build tool accepts
`--uac-profile` only with `--variant deck-engine` and verifies matching config,
profile symbol presence/absence and actual hook calls in compiled `tasks.c`.
Normal profiles compile with no Rust substage clocks and no scheduler hooks.
CI adds the explicitly enabled profile alongside the existing 17 variants.

## Measurement model

The existing whole-fill wall timer and phase counters remain. A Rust fill guard
records exclusive intervals: init, transport/descriptor handling, render-control
preparation, graph rendering, progress/commit, output copy, PCM packing and other
overhead. Nested scopes restore the previous stage, so graph/source pulls are
not added again to their parent. Intervals accumulate across all packets and
128-frame blocks of the fill. A failed/early-returning fill still finalizes its
record. Reported word layout is scalar u32 values, not Rust structs across FFI.

The C record carries generation, callback ordinal, uptime, phase, busy-slot mask,
health, last catalog event, deck loaded/playing flags, drops and the same call's
whole/Rust durations. Worst and latest >=6000 us callback records are retained
in fixed storage; no unbounded event buffer is used. Their own generation and
ordinal remain attached even after a reconnect. Rendering/packing is unchanged.

Profiling snapshots use the existing one-slot queue and priority-1 logger.
The logger prints phase data, the two coherent profile records, segment counts
and its stack high-water mark. Startup reports exact snapshot and record bytes.
Queue overwrites are counted in the snapshot that replaces the old value.
No allocation, formatting or serial output is added to the fill callback.

## Pinned scheduler audit and limitations

The actual IDF 6.1 non-SMP kernel updates its per-task runtime counter at task
switch-out; a direct before/after query would omit the currently running slice.
Instead, a private forced-include header supplies the existing tasks.c
`traceTASK_SWITCHED_OUT/IN` hooks. The IDF checkout is not modified. A compile
guard rejects SMP or another trace provider; the build verifies both calls in
the compiled kernel object. Hook functions and the systimer read are in IRAM.

Only the bound USB-client task on core 0 is tracked. Lock-free 32-bit atomic
words record completed switch-out-to-switch-in intervals and switch counts.
A single-attempt sequence check rejects incoherent samples without spinning,
masking interrupts or suspending scheduling. The scheduler measurement window
slightly brackets the original whole-fill window; both durations are reported.
Differences use modulo-u32 arithmetic and are intended for short captures, not
intervals spanning the approximately 71-minute microsecond wrap period.

`away_us` is a scheduler-hook interval, including scheduler overhead. It is not
task-only CPU time, does not separately count ISR time without a task switch,
and is not blindly subtracted from the narrower whole-fill value. The capture
must preserve `sched_valid`, window duration and context. A mere 10 ms-shaped
outlier still does not establish its cause.

## Tests and current gates

- Enabled Rust tests include exclusive nested attribution, failure exit/reset,
  clock reversal and saturation, actual six-packet PCM packing, and equality
  of profiled versus unarmed render PCM across multiple/partial blocks.
- The actual C driver runs fault/recovery tests with profiling both disabled
  and enabled, checking phase/record generations, paired begin/end, snapshot
  contents and dropped snapshots.
- The scheduler C test covers wrong task/core, switch-out/in delay, no matching
  return, counter wrap, a switch injected during snapshot and catalog filtering.
- All five C cases pass with `cc -Werror`; Windows tooling runs those as explicit
  skips and exercises the other 27 cases with pinned Python/esptool/CMake.
- Six Rust feature profiles pass: 39 / 2 / 56 / 57 / 63 / 68 tests. Format,
  Clippy with denied warnings and normal/profile RISC-V release builds pass.
  The initial guard-message assertion was adjusted for CMake line wrapping;
  the final pinned Windows run passes 27 tests with five explicit C skips,
  and all five C cases pass separately under WSL. No device measurement is
  part of these host results.
- A dirty-source pilot profile package passes IDF build, symbol isolation and
  actual compiled-kernel-hook checks. Its direct Rust call estimates are
  UAC fill 3104 bytes and load 73616 bytes; this excludes indirect calls and
  C/kernel frames. It is a validation pilot, not the eventual flash candidate.

Primary evidence: `D:/BoardEvidence/JC1060-20261030-COM3/uac-profile-20261101/`.
Pilot build, review, clean published builds and physical registration must be
recorded below before any COM3 operation. A future plan must replay all 29
current predecessors, and must include an ancestry-aware s07 fallback.
