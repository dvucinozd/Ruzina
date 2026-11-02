# Next decision after the failed narrow IRAM experiment

Status: PLAN_ONLY / NOT_IMPLEMENTED. Start from the
[completed comparison](2026-11-02-pack-placement-device.md), not the earlier
build-only report. S07 is restored and confirmed; no monitor is open.

1. Audit the actual USB service contract offline against the pinned IDF/USB
   driver: four six-packet transfers, completion batching, queued audio depth,
   refill/submission ownership and maximum delay before the endpoint lacks data.
   A 6 ms fill criterion and a 24 ms nominal pool are different quantities.
   Keep the failed criterion; do not infer audible underruns from it or silently
   replace it with the pool depth. Write down a justified budget and test model.
2. Audit work coincident with the early outliers (retained ordinals 470–480,
   catalog phase 5), code/data/cache sharing and the actual render path. Phase
   association is context, not proof of causality. Do not add a broad profiler
   or move the graph to IRAM. Assess whether an independently owned render-ahead
   MAIN/PFL PCM queue could bound refill work while preserving epochs, CUE,
   media revocation and teardown. Account for added latency, RAM, rate/pitch,
   cancellation and CPU/bus capacity; buffering cannot fix insufficient throughput.
3. Produce one reviewable design decision: a bounded render/refill separation
   with explicit ownership and tests, or a documented reason to reject it.
   Include normal/profile PCM identity, affected/peer deck continuity, generation
   fencing, bounded silence on media loss, callback-only copy/pack behavior and
   deadline fault handling. No implementation or new target campaign is implied
   by this plan alone. Do not advance product state through a warmup fill.
4. Only after that decision: implement one agreed change, run meaningful host
   and target-build checks, review/publish/CI, and prepare new clean exact-image
   packages and 36-predecessor recovery plans. Repeat target timing and RAM/
   stack evidence; then close I02–I13 on a candidate that passes its gates.

Native minimum 302, legacy rev103 identity, accepted levels and APTA exclusion
remain unchanged. DAC, networking/OTA and extra peripherals stay separate.
