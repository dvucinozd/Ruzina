# Next experiment: locate UAC callback refill latency

Status: PLAN_ONLY / NOT_IMPLEMENTED. Follows the measured [phase diagnostic](2026-11-01-uac-timing-device-results.md).
The subsequent [implementation record](2026-11-01-uac-callback-profiling-execution.md)
tracks progress and later gates separately from this original plan.
Do not change priorities, gains, DSP behavior or native minimum 302 to make a
timer pass. Keep the aggregate >=6000 us campaign bound and the original failures.

## What the phase run establishes

The instrumented image's initial prefill maximum was 1844 us, callback maximum
10373 us, and all three >=6000 us samples occurred within the first 1000
callbacks. No more crossings appeared through callback 6000. Recovery had no
samples. This is evidence about this exact image and short cold-boot/idle run.
It does not classify the old candidate's 12260 us maximum or qualify playback.

Lazy graph initialization cannot explain this run's callback maximum by itself.
Equal-priority USB client/SD owner on core 0, the 10 ms tick, early library scan,
cache/interrupt effects and render/control work are possible contributors.
The numeric proximity of 10373 us to one tick is not proof of a task switch.

## Ordered work

1. Preserve whole-fill and phase timers. Add an explicit OFF-by-default profiling
   option and a bounded context record for the worst/over-budget callback:
   connection generation, callback ordinal, uptime, pool/transfer state, catalog
   phase, deck load/playing flags and snapshot drop count. Never allocate,
   format or write a log from the fill path. Keep recording failures.
2. Divide the measured Rust call into non-overlapping intervals. Start with
   initialization, descriptor/source synchronization plus transport, render
   control preparation, graph rendering, progress/commit work, and PCM packing.
   Calls occur per packet/block, so accumulate durations across the full six-
   packet fill. Treat source pulls inside graph rendering as part of that graph
   interval until a separate nested study is needed. Do not add nested totals
   to their parent. Retain unattributed overhead and the outer whole-fill time.
3. Measure elapsed wall time and, if supported by the pinned platform, task
   switch/runtime accounting for the USB client. A CPU cycle counter spanning
   preemption is not task-only execution time. Check the actual IDF/FreeRTOS
   configuration and trace facility before choosing an implementation. Do not
   disable interrupts or mask scheduling to obtain an attractive number.
4. Extend host tests with a deterministic clock and staged delays, including
   multiple blocks/packets, failure exit, no new block, callback recovery and
   dropped snapshots. Verify attribution excludes double counting and all
   reported fields belong to the same generation/sample. Keep production PCM
   comparison and existing recovery/transport tests. Validate fixed memory and
   stack growth explicitly; source-text assertions are not sufficient.
5. Review and publish the exact diagnostic source before fresh clean builds.
   Keep source/app/ELF/plan identities, target stack estimates and RAM usage.
   Construct a fresh plan from the current restored state: the 27 earlier
   plans plus this phase-diagnostic plan and its s07 return (29 plans). Verify
   live predecessor equality and prepare an ancestry-aware return package.
6. Register the next measurement as diagnosis. Capture cold boot/idle and a
   steady idle window with exact phase/substage denominators and context.
   Preserve failures; a different threshold needs a separate prior rationale,
   not a retroactive reinterpretation. Playback and I02–I13 remain closed until
   the callback issue and acceptance decision are documented.
7. Choose a fix only from the measured cause. For preemption, review task work
   placement and ownership; for computation, optimize the measured interval.
   Require a target whole-fill comparison with phase/context, stack/RAM and
   unchanged PCM/behavior. Neither host timing nor fewer helper calls proves a
   target win. Restore and verify s07 before ending an unsuccessful experiment.

Report raw evidence, operator acceptance, unresolved mechanisms and NOT_RUN
cases separately. Do not describe missing recovery/playing measurements as zero
cost, and do not describe the diagnostic as accepted firmware.
