# Follow-up after callback substage/scheduler capture

Status: PLAN_ONLY / NOT_IMPLEMENTED. Read the [measured device record](2026-11-01-uac-callback-profiling-device.md).
Do not change priorities, DSP, gain or the >=6000 us criterion before a controlled
cause test. Native minimum remains revision 302; this unit uses legacy rev103.

## Established evidence and limits

The profile image's two retained over-budget callback records are ordinals 709
and 710, around 5.02–5.04 seconds uptime. Whole times are 13087/13039 us. Both
have exact exclusive-stage sums and coherent scheduler windows; recorded away
time and switches are 0/0. The worst stage walls are graph 6523 us and pack
3725 us. A separate steady window from callback 15000 to 24000 added no >3000
or >=6000 us crossings. Source was idle, not playing, and target recovery was
not exercised. These are different image/run observations from earlier maxima.

This does not establish pure compute cost: ISR time, cache/bus stalls and the
instrumentation itself remain in wall time. A zero scheduler delta is a measured
result, but absolute switch totals across a known yield were not logged. Kernel
call integration and host trace tests passed; target hook-liveness calibration
is still useful before declaring preemption excluded. Do not choose a priority
change merely because the old maximum resembled one 10 ms tick.

## Next bounded experiment

1. Add a profiling-only scheduler-liveness calibration before audio transfers
   are submitted: bind the actual USB-client task, record absolute counters,
   deliberately yield once, then record counters again. Verify at least one
   completed tracked switch-out interval, record time and validity, and stop
   causal scheduler inference if it fails. Keep the deliberate yield out of
   the audio fill path and retain its explicit startup measurement. Add absolute
   before/after counts to retained records without discarding delta/window data.
2. Audit the pinned RISC-V interrupt dispatch/trace points for the actual USB,
   timer and other enabled IRQs. Do not assume a FreeRTOS traceISR macro covers
   every ISR. Choose a bounded IRAM-safe IRQ accounting method only after its
   coverage and nested-interrupt rules are established. Report uncovered IRQs
   and invalid samples; do not call residual wall time CPU execution time.
3. Capture baseline distributions cheaply: counts and bounded histograms per
   exclusive stage, plus coherent worst context. Include callbacks that exceed
   3000 but not 6000 us, which the existing worst/latest-over records do not
   necessarily retain. Never format, allocate or stream per-call logs in the
   callback. Preserve total/phase maxima and generation tags.
4. If IRQ accounting leaves substantial unexplained wall time, inspect graph
   and PCM packing generated code, memory placement and cache/bus behavior.
   Use a registered single-variable experiment and keep a no-change control;
   do not move a broad subsystem to IRAM or rewrite DSP speculatively. Normal
   and profiling builds must retain PCM/transport/media/recovery equivalence.
5. Extend deterministic host tests for calibration failure, nested IRQ coverage
   if implemented, histogram saturation, record identity and dropped snapshots.
   Validate normal-build exclusion and RAM/stack growth. Review and publish,
   clean-build, verify hashes, and register exact image/plan/criteria before
   measuring. Current history after this experiment's verified s07 return is
   31 plans, not the previous 29.
6. Repeat exact-image cold and steady idle without controls/playback. Compare
   denominators, validity, absolute/delta scheduler data and covered IRQ time.
   Choose a fix only when the experiment distinguishes a mechanism. Any win
   requires target whole-fill comparison plus unchanged PCM and RAM/stack
   evidence; lower host time or fewer helper calls is insufficient.
7. Restore and verify s07 normal operation before ending an unsuccessful
   experiment. Keep I02–I13, target recovery timings and playing-source supply
   margins NOT_RUN until the callback gate and acceptance decision are closed.

Retain the raw logs and the earlier failed measurements. Do not retrospectively
reclassify an old uninstrumented maximum using this new image's measurements.
