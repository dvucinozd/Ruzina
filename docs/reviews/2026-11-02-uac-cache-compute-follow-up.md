# Next cause gate after calibrated IRQ/scheduler profiling

Status: PLAN_ONLY / NOT_IMPLEMENTED. Read the [device result](2026-11-02-uac-cause-profile-device.md).
S07 is restored and owner-confirmed; all monitors are closed. Native minimum
remains 302. Do not change priorities, gain, DSP or begin other peripherals.

1. Finish an offline memory/cache-counter audit against pinned IDF and the
   exact profile ELF. Code inspection already places pack/closure/write (704
   bytes combined) and graph (2204 bytes) in flash-mapped text. Inspect packet
   scratch/output buffer allocation and actual memory-capability contracts;
   code section addresses do not establish runtime buffer location. Identify
   supported per-core cycle/cache events, counter ownership, overflow rules,
   nested-IRQ effects and an independent liveness calibration. If the P4/IDF
   path cannot supply trustworthy counters, record that limit explicitly.
2. Select one bounded experiment only after the audit. Prefer observational
   cache/cycle counters before moving code/data. Keep an unchanged instrumented
   control with the same logging, calibration and histograms. A narrow pack
   placement variant is an alternative only if all generated callees and
   section/RAM growth can be verified; do not move the full graph or perform
   an algorithm rewrite at the same time. Do not warm the product fill through
   a call that advances deck, cadence, graph or transport state.
3. Extend deterministic tests for counter liveness/failure, wrap, nesting and
   interval coherence. Require byte-identical packet/render output, unchanged
   media/transport/recovery behavior and normal-build exclusion. Verify exact
   generated code, static/queue/stack deltas and symbols. Lower host time,
   helper-call counts or an IRAM label do not establish a target timing win.
4. Review and publish source, clean-build control and the single changed
   variant, verify every source/artifact hash and archive secondary copies.
   Register exact image identities, unchanged >=6000 us rejection and <=3000 us
   headroom target, sample denominators and comparable cold/steady windows.
   Current history has **33** predecessors, full reconstruction
   `4324fee5c109cc9fe28aebbafa8f5cf2116ee4950afce374be5ef6f81aa2ae68`.
   Register each transition/return; later variants must include earlier writes.
5. On this exact rev103 unit, require fresh physical ROM entry, identity,
   security/RDID and full predecessor equality before writing. No controls,
   track load or playback in the idle cause campaign. Repeat comparable cold
   observations if deciding on a mechanism; a single disappearance of an early
   outlier is not enough to claim a fix. Keep both target calibrations, valid
   exclusive partitions and broader-window bounds. Do not add scheduler-away
   and IRQ durations as disjoint intervals or call their residual CPU time.
6. Accept a proposed fix only with comparable whole-fill target timing plus
   unchanged PCM and bounded RAM/stack evidence. Preserve every old failed
   observation. Restore/readback/cold-boot s07 and confirm normal sound/Pause/CUE
   before ending any failed or unqualified experiment. I02–I13, target recovery
   timing and playing-source supply qualification remain separate open gates.
