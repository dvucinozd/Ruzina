# Source-supply and USB service diagnostic

Date: 2026-11-03. Branch `ogranak/jc1060-source-supply-profile`, parent
`fd466b38a90af16baab6d614c2bff9302715ba2c`. Follows the
[rejected normal two-MP3 campaign](2026-11-02-normal-engine-device.md).
Status: **REVIEWED_BUILT_ARCHIVED / NOT_REGISTERED / NOT_FLASHED**.

Accepted s07 remains installed, with 40 executed predecessors and independently
verified full flash `7875f650ba7404475e9c26561990851b04c11409b3f309401f6e77a51ddf38a6`.
No COM3 access is part of this source preparation. Rev103 uses pinned IDF 6.1;
native minimum 302 remains unchanged. No DAC, APTA, network or peripheral work.

## Question and boundary

The old normal image passed cold/idle (2397 us fill max), missed <=3000 us
headroom on a single source (3214 us), and failed with both sources: buffers
drained, underruns 4765/5003 and one USB packet error, fill max 4982 us. This
does not prove SD, CPU, cache or scheduler causation. The cumulative 270015 us
producer gap includes earlier loading and cannot be assigned to dual playback.
Raw load receipts identify prior catalog indices 3/4 as codec 2 (MP3), both 44100 Hz;
owner descriptions were Disturbed and Ben Folds Five. Indices belong only to
that captured catalog, not a permanent track identity.

Both producers run on `sd_rust_client` (core 1/priority 3). Its per-deck poll
budget of 1000 us is soft, checked between up to 16 decoder actions. A decoder
step or file read can exceed it; neither can be safely preempted by that check.
SD block owner is core 0/priority 4; USB client is core 0/priority 4, USB library
core 0/priority 5. These placements are observations from source, not proof of
the bottleneck. Do not alter affinity, priority, buffer capacity, stack size,
gain, deadline or underrun counting to make this diagnostic pass.

## Explicit build isolation

`build_legacy.py --variant deck-engine --producer-profile` selects Rust
`idf-producer-profile` and `CONFIG_RUZINA_PRODUCER_PROFILE=y`. It is mutually
exclusive with UAC substage/scheduler/IRQ profiling and IRAM packet placement;
CLI, CMake and Rust feature guards reject combinations. Defaults remain OFF.
Manifest records `producer_profile` independently of `uac_profile` and
`uac_pack_iram`. Fresh builds verify the producer snapshot symbol exists only
in the diagnostic. No producer counters/extra clocks compile into normal mode.
The ordinary decode implementation is shared and keeps its existing budget,
media/epoch/load/seek ownership and PCM behavior. New normal builds still need
independent runtime qualification; compile-time isolation is not equivalence.

## Measurement definitions

All durations below are **wall time including preemption/wait**, not CPU time.
Counters/totals are boot-cumulative; invalid-clock/partition and saturation
flags invalidate quantitative conclusions. Never subtract unrelated maxima.

- Per-deck `poll`: one owner's service of that deck, including admission,
  load/seek/decoder work and receipt handling. `busy` counts worked turns.
  `frames` counts newly published source frames using the producer-owned head
  delta; it is not output UAC frames, and frame totals may include priming.
  `max_service_gap_us` is the largest interval from the previous poll's end to
  the next poll's start. Idle/peer work is included, and maxima stay cumulative.
- `decode`: a nested decode invocation. `step`, `read`, `push` and
  `decode_other` partition this same invocation; their totals sum to decode.
  Step is `SourceDecoder::step`; read is synchronous logical `read_at` wall
  time (including block-owner wait/cache/filesystem work); push is PCM ring
  delivery. Other includes control, provide/copy, clock and observation overhead.
  Read bytes are logical bytes returned, not physical SD traffic. Header/open
  reads outside decode remain in the poll remainder, not decoder read totals.
- Poll includes decode, so adding poll+decode double counts. Likewise all
  measured maxima belong to separate samples and cannot be subtracted to
  estimate a worst-case residual. The decoder partition uses child deltas
  inside the same invocation with checked arithmetic.
- `producer_owner`: completed owner-turn wall time = summed per-deck service
  wall + remainder. Remainder includes selection/details, event reporting,
  bookkeeping, publication overhead and idle yield. It is not CPU idle time.
  Its publication includes completed preceding turns only; its timestamp is
  their last end, separately from each deck's snapshot timestamp.
- Snapshot metadata records current track index/ring epoch, source rate,
  playing flag, pitch and buffered supply. Each producer and owner snapshot
  is independently coherent; they are not a simultaneous cross-core sample.
- `producer_usb_service`: stream-started client-loop handle-events wall and
  client-tail wall, plus callback-entry spacing. Handle-events includes waiting
  and callback/render execution. Tail includes report/MIDI/housekeeping. These
  intervals partition a selected client-loop turn; callback spacing overlaps
  them and must not be added. Samples whose starting generation changed are
  excluded, and service counters reset on a new UAC generation. Callback
  spacing is not a physical IRQ-to-dispatch latency or pool-occupancy proof.

One SD-client writer publishes per-deck and owner snapshots at most once per
second through fixed AtomicU32 slots. SeqCst sequence validation gives one
bounded copy attempt per slot; a preempted writer produces unavailable status,
never a reader spin/wait. Only the existing USB report cadence copies snapshots
to the low-priority logger's single latest-snapshot queue. The queue's drop
counter remains visible. Missing/unavailable, stale/non-advancing timestamps
or sequences, invalid partitions, saturation, or missing required stage rows
are NOT_CAPTURED/INVALID, never measurement PASS. Account for intentional
one-second publication and asynchronous logger timing when checking freshness.

No formatting, allocation, heap walk or extra render clock is added to refill.
The optional callback-entry timestamp is outside refill and still has nonzero
observer cost. Producer stats, atomic copies and the larger logger snapshot
also have cost. Current task stacks are unchanged; build/frame checks and
runtime watermarks remain mandatory. A profiled image cannot qualify normal
headroom, two-deck sound, reconnect or soak.

## Offline evidence and sequence

Host checks include real WAV PCM through a controlled 5000 us logical read,
read-failure propagation, same-window partition arithmetic, saturation and
clock rejection, owner remainder, bounded unavailable snapshots and concurrent
publication coherence. Actual C tests cover production-driver snapshot copy,
generation-filtered service intervals, callback gaps, invalid clocks and
saturation. Test clocks model read delay; they do not benchmark host decoding.
Run normal and diagnostic Rust tests/Clippy, all actual C tests, Python config
guards, target Clippy, fresh normal/diagnostic packages, static stack/RAM limits
and exact artifact/source/symbol checks. CI gains a separate diagnostic build.

Review/publish exact source and CI, then register the diagnostic and exact s07
return using 40/41 executed predecessors. Do not reuse normal or older profile
registration/tool bindings: source/package/flags and full state must be bound
again. Dual-copy archives and exact-hash execution tools precede any COM3 work.

Future target cases: cold/idle admission first; separately captured one MP3 per
deck, then both same known sources, with source metadata and stable playback
states. Compare frame-production deltas per source elapsed time/pitch against
consumption and observed supply; compare nested poll/decode/read/step wall
partitions and USB service rows. Keep case baselines/deltas and observer drops.
Do not infer steady throughput from load/prime time or a mixed control window.
Stop on registered >=6000 us fill, reserve failure, unexpected USB/source error
or steady underrun; preserve the first bounded failure and restore/readback/
cold/listen exact s07. No PFL/transport/codec/reconnect/soak extension until the
normal failure is resolved. Select one measured fix only after this separation.

Local preparation verification: 75 diagnostic Rust tests and host/target
Clippy PASS; 70 normal engine Rust tests and target Clippy PASS; eight actual
C cases PASS. Pinned Windows Python runs 49 tests: 41 PASS/eight C skips, with
those eight C cases executed separately in WSL. A broad system-WSL Python
attempt lacked esptool in two unrelated tool tests; that unavailable environment
is not reported as a full WSL-suite PASS. Config CLI/CMake isolation, existing
PCM/fault/fairness tests and 496 documentation links pass. Raw logs are in
`docs/reviews/evidence/2026-11-03-source-supply/`. Source review, exact CI and
fresh packages remain pending at this checkpoint.

## First source review and package checkpoint

Source `e9ce242` built clean normal and diagnostic packages. Each verifies all
289 source hashes and eight artifacts, exact app/ELF identity and mode isolation.
Normal: 819744 bytes, app `4eb5161dcfc27714f66c445cb73425b224e0e873cb0de46bc39f64b0d68a7ef7`,
static load/fill 74640/3120 bytes. Diagnostic: 824720 bytes, app
`173933e100e2de34257beb89013556b0937e8b0e94bd36bf797eb755d505d1cd`,
static load/fill 75712/3120 bytes. All unchanged limits pass. These packages
remain BUILD_ONLY, not registered/flashed or runtime qualified.

Full committed CodeRabbit review found 3 issues: two missing spaces in this
documentation and one CI matrix issue. The producer include matched/modified
the existing normal deck-engine combination, removing its separate CI build.
The fix adds `producer_profile: [false]` to the base matrix, so the true include
creates an additional job. Exact subsequent CI must contain both plain
deck-engine and producer-profile jobs (22 total jobs across the two workflows).
The docs and matrix are fixed; final fix review/CI remains pending. The source
files recorded in both package manifests are byte-identical across this fix;
the changed CI/docs files are outside those source manifests. Preserve the
original review and fixing commit beside the exact final CI evidence.

## Verified offline completion

The full source review's matrix issue is fixed in `d6a5906`; its follow-up
reported only a documentation spacing issue, fixed in `0c3d86d`. Final committed
review against `d6a5906` completed with **0 issues**. The raw full/fix/final
reviews and quota attempt are retained; the final result does not erase the
earlier findings. Exact `0c3d86d131eaee0fdd0380d2feba2f3364850253` CI is
**22/22 PASS**: [host #567](https://github.com/dvucinozd/Ruzina/actions/runs/41544300271)
and [legacy #90](https://github.com/dvucinozd/Ruzina/actions/runs/41544300145).
Both normal and producer-profile engine jobs are present and successful.
See [CI evidence](evidence/2026-11-03-source-supply/ci-verified.json) and
[final review](evidence/2026-11-03-source-supply/coderabbit-final.raw.log).

Both clean package manifests pin firmware source `e9ce242`. All 289 source
inputs remain byte-identical through the CI/docs-only fixes to `0c3d86d`;
this equivalence and all eight artifacts per package are checked, rather than
silently changing a manifest source commit. [Package checks](evidence/2026-11-03-source-supply/package-checks.json)
retain full app/ELF/manifests/sections. Diagnostic static DRAM BSS increases by
576 bytes; task stack sizes are unchanged, while its larger logger snapshot
and producer observations still need actual heap/stack/timing measurements.
Static call-chain limits pass, not runtime reserves or normal qualification.

The [dual-copy archive index](evidence/2026-11-03-source-supply/artifact-archive.json)
records `reviewed-source-supply-builds.zip`: **15118202 bytes**, SHA-256
`9dce7560842bfe759aab2cb3aff196a904786bdd3a61bd02f62094b248bef822`.
ZIP CRC, every member and every secondary member/archive match. It contains
both packages, exact source bytes, tests, review chain, CI and the reviewed
measurement contract. Primary:
`D:/BoardEvidence/JC1060-20261030-COM3/source-supply-20261103/`;
secondary: `C:/Users/Daniel/Documents/BoardBackups/JC1060-20261030-COM3/source-supply-20261103/`.

Next is a separately frozen target campaign/runbook, **40/41** candidate/s07
recovery plans from full `7875f650...`, and exact reviewed execution/analysis
tools that validate the new profile flag and complete/fresh stage snapshots.
No old normal/profile binding authorizes this image. Record baseline/delta
semantics and the unchanged stop/return conditions before target work. Until
then COM3 stays unopened and accepted s07/history 40 remains installed. The
dual-MP3 problem is **not fixed or target-diagnosed** by this offline milestone.
