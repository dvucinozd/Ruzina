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

## Reviewed clean-build registration

Source `8a9fe16d48387b0a3cfbd461659696091b2aa89d` is pushed on
`ogranak/jc1060-uac-callback-profile`, [PR #75](https://github.com/dvucinozd/Ruzina/pull/75)
above #74. CodeRabbit reviewed the committed 36-file diff with **0 issues**, exit
0. Local/remote SHA equality was checked. Both source-head workflows pass:
[host #532](https://github.com/dvucinozd/Ruzina/actions/runs/41357949681) and
[legacy #55](https://github.com/dvucinozd/Ruzina/actions/runs/41357949720),
**20/20 jobs** including 17 normal IDF profiles and one explicit profiling build.

Fresh normal and profiling packages record the same source commit and empty
dirty status. All 277 source hashes and 16 artifact hashes passed verification.

| Package | Bytes | App SHA-256 |
|---|---:|---|
| normal deck-engine | 817344 | `f42878fc953ee62f179d48a9a87faa7940e8e7769c3d9cd4defad5944dc30b46` |
| explicit profile | 820432 | `46cb70ef201c4be12415931fc3330a186dadf48d0d435121432a1731dda5a4a2` |

The profile contains verified task-switch hook calls; normal symbol-isolation
checks pass. ELF size comparison adds 2870 text, 216 data and 338 BSS bytes
(554 static RAM bytes). The snapshot grows from 208 to 516 bytes: 308 extra
queue payload bytes plus the same RTOS control block. The logger stack remains
3072 bytes and reports its high-water mark in the experiment. UAC Rust direct-
call estimates are 3120 normal / 3104 profile bytes, load 73616 bytes; these
exclude indirect and C/kernel frames and are not runtime stack qualification.
No speed improvement is inferred from a stack estimate or compiler layout.

All 29 current flash predecessors replay successfully, including restore
hashes and preserved tails; four factory backup copies pass. The expected
complete s07 predecessor is `3130901f09e304949556d582b46b3b8e417713600317bce755bb819b6f876a40`.

- [Diagnostic plan](../boards/readiness/jc1060-uac-callback-profile-write-recovery-plan-20261101.json):
  `370faa84843f2ecd14c686550f6f837238d67dedeb580f757cacf97e3d2f75b6`.
- [Return-to-s07 plan](../boards/readiness/jc1060-uac-callback-profile-return-s07-write-recovery-plan-20261101.json):
  `7dea4155234fd4e1328856f4fcb06ca3c4b7365514f516b1f41a2ac14e8e0441`.
- [Registration](evidence/2026-11-01-uac-profile/diagnostic-registration.json),
  manifests and raw review/CI metadata retain the exact clean-build identity.
- [Archive index](evidence/2026-11-01-uac-profile/registration-archive.json):
  61 files, CRC/content verification and identical secondary SHA-256.

At registration: **NOT_FLASHED / DEVICE_PROFILING_NOT_RUN**. A later device
session must independently verify identity/security/RDID and full predecessor
equality, capture cold boot and steady idle only, retain the >=6000 us aggregate
criterion, and restore/verify s07 before exit if the experiment fails or remains
unqualified. Stage layout: word 0 total Rust wall us, 1 validity faults, 2 packet
attempts, 3 render-block calls, 4–11 exclusive times in enum order, 12–19 segment
counts in that same order. `partition_valid=0` forbids time attribution claims.

## Later device execution and exit

The [device record](2026-11-01-uac-callback-profiling-device.md) supersedes the
registration-only state above. The guarded exact candidate was measured in
cold and separate steady idle windows. Two coherent records took 13087/13039
us, with valid exclusive partitions and zero observed scheduler switch deltas.
The aggregate >=6000 us criterion fails. IRQ/cache/observer costs and runtime
hook-liveness remain open; no priority or DSP fix is justified yet.

Exact accepted s07 was restored through the registered plan, all written spans
passed readback, and cold boot matched the expected ELF. A separate playback
capture records 993312 frames, zero underruns and USB errors; the owner confirms
normal sound in both ears, Pause and CUE. All monitors are closed. Future plans
need 31 predecessors. The [cause follow-up](2026-11-01-uac-callback-cause-follow-up.md)
is PLAN_ONLY and has not been implemented.
