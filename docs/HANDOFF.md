# Rust M2.5 development handoff

**Current state (2026-11-03): [two MP3 decks play clean](reviews/2026-11-03-dual-mp3-opt3.md).**

- **What fixed it:** the decode crates are built at `opt-level = 3` (commit
  `521172a`). On the JC1060 the owner played two MP3 decks together for about
  59 s with 0 underruns and heard it clean.
- **USB audio follow-up (2026-11-04):** the image with the USB audio fix is
  owner-accepted after about 5 minutes with seeks. A lost isochronous packet
  is now only counted, and the recovery budget refills after clean streaming.
- **SD reads (2026-11-04):** [multi-sector reads](reviews/2026-11-04-sd-multisector.md)
  of up to 16 sectors, through a read-ahead in PSRAM. Next to a FLAC deck the
  MP3 frame walk now finishes in 2.2 s instead of 51 s. The worst cold MP3 seek
  is 55 ms instead of 5.4 s. Underruns 0/0, `packet_errors=0`, and the owner
  said the run was fine.
- **Flash QIO 80 MHz (2026-11-04):** [the flash mode change](reviews/2026-11-04-flash-qio80.md)
  from the bring-up DIO 40 MHz cut the longest USB audio fill from about
  5.5 ms to 3.3 ms. Fills over 3 ms went from 967 to 4. Seeks also prime about
  twice as fast. The owner checked by ear: "sve OK".
- **PSRAM 200 MHz (2026-11-04):** [the PSRAM speed change](reviews/2026-11-04-psram-200m.md)
  brought the longest USB audio fill under the 3 ms target: 2.8 ms, with no
  fill over 3 ms in about 5 minutes. The full PSRAM memory test runs at every
  boot. The owner checked by ear: "sve OK".
- **Soak (2026-11-04):** [a soak DJ diagnostic](reviews/2026-11-04-soak-autoplay.md)
  kept both decks loading and playing unattended. The first 40-minute run
  showed underruns only right after an automatic load and Play. A load now
  starts with the seek cushion. The next 35-minute run had 16 automatic
  loads, 0/0 underruns, a longest fill of 2.8 ms and no packet errors.
- **Untagged VBR MP3 seeks (2026-11-05):** [they now resume through the frame index](reviews/2026-11-05-untagged-vbr-seek.md),
  sample-identical to a linear decode on the host. On the board, seeks were
  checked for regressions, and the owner said they were fine ("Sve OK").
- **Loops (2026-11-05):** [seamless loops from a played-audio history](reviews/2026-11-05-loops.md)
  in PSRAM, with a 5.8 ms crossfade at the wrap. On the FLX4 the owner heard the
  section repeat with no click. 4 BEAT/EXIT, IN, OUT, halve, double and Beat Loop
  pads work, and Shift + 4 BEAT brings back the last loop.
- **Scratch (2026-11-05):** [the jog wheel scratches from the same history](reviews/2026-11-05-scratch.md),
  about 24 s back and 1.5 s forward, with 5.8 ms fades. On the FLX4 the owner
  heard no click and the speed follows the hand, also on a paused deck and in a
  loop. 1024 ticks per turn is an estimate.
- **Beat FX and Pad FX (2026-11-05):** [connected to the render graph](reviews/2026-11-05-fx.md),
  with 1.5 MB of PSRAM and short fades so filters do not click. The owner tried
  them on the FLX4 ("Radi"). Delay, turning Beat FX off, Pad FX bank 2 and the
  channel switch are not in the device log.
- **On the board:** app `c4280a75…` (production with loops, reloop, scratch and FX; QIO 80 MHz flash, 200 MHz PSRAM). Live pre-write backups are in
  `D:\BoardEvidence\JC1060-20261030-COM3\s{10..35}-*-flash`.
- **Parked work:** the matched control/opt3 source-compare preparation is on
  `ogranak/jc1060-source-compare-parked`, not for merge.
- **Open:**
  - `fill_max_us` is 3.2 ms in production and 3.4 ms under profiling while
    really mixing, over the 3000 µs headroom target, with no underruns. The
    render graph is most of it. The owner stopped here; IRAM is kept for when
    FX add render time;
  - scratching with both decks playing reached 3.4 ms, the same as mixing, with
    no underruns;
  - the 35 to 40-minute soaks were unattended: no seeks, scratch or EQ
    during them;
- **Older paragraphs** below are historical checkpoints.

Latest offline implementation: [legacy MP3 in-place reset](reviews/2026-11-03-mp3-in-place-reset.md).
Only legacy dependency resolution uses two pinned local crate copies; three
upstream files change. Safe field resets replace core construction in wrapper
reset/seek; native workspace and normal compiler defaults remain unchanged.
Normal/opt3 builds pass old stack guards, reset frame 16 B. Unit/codec tests
and byte-exact PCM comparisons pass. No COM3 access; s07/history **44** remain.
Review/CI, clean package/archive completion and a fresh explicitly registered
matched-control/opt3 transition chain precede any device work. Existing old
writer bindings/plans cannot authorize these new source/image identities.
Source `a245335` has review 0 issues and three clean source/artifact-verified
packages; the 31466623-byte/405-member archive `fe94bb23...` passes CRC/member/
secondary checks. Exact-source host CI #586 passes; legacy #109 is queued.
Verify that remaining CI gate before preparing any new registered transition.

Latest offline result: [source-budget investigation](reviews/2026-11-03-source-budget.md).
Real producer/decoder shared-time tests and a release benchmark isolate read
pressure and compiler sensitivity; all compared host PCM files match exactly.
Control audit finds 74 extra messages in S03 and reproduces eight unsupported
scratch effects, not media failures. Frozen numerical admissions are retained,
but S03 is not isolated-control acceptance. The opt3 core candidate is rejected
before packaging: constructor frame **39008 > 24576**; independent load-chain
estimate **90544 > 90112**. Normal build passes. Default compiler policy and
native minimum 302 stay unchanged. No device access; s07/history **44** persist.
Next: inspect runtime reset/seek construction and design bounded-stack decoder
initialization; do not register/flash this rejected image or enlarge limits.
Full review raised 1 minor audit-boundary issue; fix `59baed2` has final review
0 issues and four audit tests per OS pass. The 25209794-byte/684-member archive
`b08095f6...` passes CRC/member/secondary checks. New exact-source CI was queued;
do not substitute previous-head CI or local checks for it.

Latest device result: [continuous 42/43 campaign](reviews/2026-11-03-continuous-device.md).
Cold/idle and individual Disturbed/Ben Folds Five MP3 windows were admitted;
both-playing failed after 15.88 seconds, underruns **78/94**, matching owner
crackling. Four admitted checkpoints do not make the failed whole session PASS.
S07 recovery passed all spans and independent 16 MiB equality; the separately
bound single software boot pulse completed, application **NOT_OBSERVED**.
No manual recovery qualification was requested. Tools closed. History **44**,
full `28830827d3e3426bbcd58b9a25d4d2129e28f150bade33fe3c551cd577f93921`.
42/43 plans are consumed; a future transition needs new **44/45** ancestry.
Completed archive `b188a18b...`, 39722727 bytes/119 members, CRC/member/secondary
verification passed. Follow the report's ordered offline source-budget work;
no new flash, native-minimum reduction, APTA/DAC/PFL scope or promotion.
All older current-state/next-step paragraphs below are historical checkpoints.

Latest preparation: [new 42/43 continuous-source registration](reviews/2026-11-03-continuous-registration.md)
and frozen [runbook](reviews/2026-11-03-continuous-campaign.md). Both real-file
offline preflights reconstruct exact current/candidate/return states; 53 tests
pass per OS. Writer review 0 issues, exact source `b92fb23` CI 22/22; all three
separate bindings and dual-copy registration/tools archives pass. No target
access. Next: publish/verify remote and offline bytes, refresh actual topology,
then the registered candidate experiment with automatic s07 exit. Earlier checkpoint details
below retain their historical counts/status; do not reuse old plans.

Current device result: [producer cold/idle and incomplete playback](reviews/2026-11-03-source-supply-device.md).
S01/S02 pass at 1880 us all-fill maximum; S03 ends without load/play and its
initial producer/heartbeat prefix fails frozen analysis. No throughput or
dual-MP3 root cause is established; S04/S05 NOT_RUN. Exact s07 was restored
with full readback, cold boot and owner `sva je ok`; captured playback 9.276 s
has zero errors. All tools closed. Current history **42**, full flash
`28830827d3e3426bbcd58b9a25d4d2129e28f150bade33fe3c551cd577f93921`.
Completed archive 37678594 bytes, `4576676b...`, CRC/members/secondary pass.

Owner decision after recovery: **no more repeated s07 physical cold/listening/
Pause/CUE recovery checks**. Use automatic readback and the
[recovery policy](RECOVERY_POLICY.md); prepare a separately reviewed software
boot action if needed. Preserve old frozen runbooks/archives as historical.
The [continuous adapter/analyzer](reviews/2026-11-03-continuous-adapter.md)
now extends the offline core with immutable checkpoints, whole-journal replay,
owner commands and 39 capture/adapter tests per OS, including one synthetic five-case passive
connection. The separate software boot action is also
[implemented and host-tested](reviews/2026-11-03-software-boot.md)
(46 source-supply tests per OS after the async-admission regressions). Target access is still unregistered. Next:
verify latest CI, then new 42/43 plans, contracts and exact bindings. Combined
CodeRabbit review completed (one minor docs issue fixed, final fix review 0);
exact evidence and dual-copy 64731-byte archive `0a59b8b2...` are retained. No
decoder optimization, extra flash or manual recovery qualification is implied.

The preparation checkpoints below are historical.

Historical registration: [producer campaign registration](reviews/2026-11-03-source-supply-execution.md)
and [frozen runbook](reviews/2026-11-03-source-supply-campaign.md). Candidate/s07
plans are registered against 40/41 predecessors; both offline preflights and
326-member dual-copy archive pass. Current actual s07/history 40/full 7875f650
is unchanged. New producer adapter/analyzer and automatic report-fault stop
have 12 tests passing per OS, with 13 shared-guard WSL tests. Final fix review
has 0 issues and exact `bb96132` CI passes 22/22. Four-file binding `a63241b1...`
and the 48748-byte dual-copy tools archive `949e4831...` are frozen; publication
and both offline preflights precede COM3.
Old normal/profile bindings do not authorize this image. Follow S01-S05 only,
then execute its originally registered exit. That manual s07 exit is superseded
for future campaigns by the owner policy above. Do not edit the frozen runbook.

Active reviewed source: `ogranak/jc1060-source-supply-profile`, draft
[PR #82](https://github.com/dvucinozd/Ruzina/pull/82); read the
[producer/USB measurement contract](reviews/2026-11-03-source-supply-profile.md).
Default-OFF producer profiling separates per-deck service/decode/read/step/push,
owner remainder and USB client/service gaps. 75 diagnostic Rust tests plus
Clippy, 70 normal Rust tests and eight actual C cases pass locally; Windows
Python has 41 PASS/eight explicit C skips, executed separately in WSL. Final
fix review has 0 issues; exact `0c3d86d` CI passed 22/22 with both normal and
diagnostic jobs. Clean normal/diagnostic packages pin `e9ce242`; all 289 source
inputs are byte-identical through the CI/docs-only fixes. Stack estimates are
74640/3120 normal and 75712/3120 diagnostic, with unchanged limits. The
15118202-byte archive `9dce7560...` and every secondary member/hash pass.
Next: freeze a target runbook, register new 40/41 recovery plans, and review/
bind tools that validate producer-profile snapshots before COM3. No new device
access, registration or runtime acceptance. S07/history 40/full 7875f650 remain
the installed baseline. Prior normal-result publication `fd466b3` CI is now
success in both workflows (#564/#87). Device evidence below remains immutable.

Current device state: read the [normal-engine rejection and next ordered work](reviews/2026-11-02-normal-engine-device.md).
Cold/idle max 2397 us passes; each MP3 alone is owner-confirmed with no errors,
but max 3214 us misses the headroom target. Two simultaneous MP3 sources drain
both rings, underruns 4765/5003 and one USB packet error (fill max 4982 us):
registered stop, no PFL acceptance or N03-N08 extension. Exact s07 was restored,
full-readback/cold/owner sound-Pause-CUE confirmed; 14.052 captured playback
seconds have zero errors. Tools closed. Current ancestry **40**, full hash
`7875f650ba7404475e9c26561990851b04c11409b3f309401f6e77a51ddf38a6`.
The 37668270-byte completed archive `500b8398...` has matching CRC/members/secondary.
Next is offline producer/decode/SD/USB-service instrumentation design, then
review/CI and a new 40/41-predecessor diagnostic registration. No new device
work, limit increase, normal promotion or merge follows from this rejection.

Historical preparation (23:21 Zagreb): exclusive-port fix plus real-loop synthetic
tests are published as `08a537b20310347a842a0efdef500c826e4baa59`.
CodeRabbit against `3841b7f` completed with **0 issues**. All 13 normal-monitor
tests pass on Windows/WSL, including four real-loop fake-serial cases; 13 shared
destructive-guard WSL tests pass. Exact CI passed 21/21; binding `a35aa0c4...`
and the 48447-byte dual-copy tools archive `670cdc3d...` are frozen. Publish
this checkpoint before normal device access. Read the latest record rather than the
historical rate-limit paragraphs below. Registered firmware/runbook/plans are
unchanged; native minimum remains 302 and the unit uses rev103 IDF only.

Historical next-campaign preparation: `ogranak/jc1060-normal-engine-campaign`, draft
[PR #81](https://github.com/dvucinozd/Ruzina/pull/81) above #80. Read
[normal execution checkpoint](reviews/2026-11-02-normal-engine-execution.md)
and the byte-frozen [runbook](reviews/2026-11-02-normal-engine-campaign.md).
Source `886127e` has clean normal app `0ba716a4...`, source review 0 issues,
six actual C cases and 21/21 CI. Registration `ec43409` has 21/21 CI; 38/39
candidate/s07-return plans, two offline preflights and dual-copy 312-member
archive pass. Owner confirms RCA MASTER speakers plus controller headphones.
Normal adapter's two minor CLI issues were fixed in `dd245b6`. The next full
review found one major capture-duration issue; `05cff46` fixes it, with four
normal-policy/window tests and 13 shared-guard tests passing. Final fix review
is blocked by CodeRabbit free quota (last error says 34 minutes at 21:34 Zagreb;
retry conservatively after **22:10 on 2026-11-02**). Raw failed reviews/limits
are retained; there is no final 0-issue tool review or execution binding yet.
Latest code CI #557/#80 and publication #558/#81 passed 21/21 each. The 22:10
review found another major issue: connection gaps counted toward capture.
The follow-up records connected seconds and invalidates post-start disconnects,
with seven normal coverage tests passing per OS and 13 shared-guard WSL tests.
Its own review/CI and binding are pending; see the execution record's latest
section. The older rate-limit paragraph above is historical. Superseded
`dd245b6` workflows were cancelled, not passed. Do not modify the frozen runbook.
Next: verify exact latest CI, rerun committed fix review against `f89bb36`,
freeze/archive the new normal binding, publish the handoff, then software ROM
entry and the registered normal-image cases. No normal-image COM3 access or
write has occurred. Accepted s07/current 38/full `1a456836...` remain installed,
all device monitors closed. Do not reuse the old profile binding or treat a
captured-but-short/early-stopped cold/idle/soak window as complete.

Completed physical campaign: read the
[resource-readiness device result](reviews/2026-11-02-resource-readiness-device.md)
and current
[device progress](reviews/evidence/2026-11-02-resource-readiness/device-progress.json).
Candidate and s07-return writes/independent whole-flash readbacks match; current
executed ancestry is 38 and full state is `1a456836...`. Three cold windows each have
7000 callbacks and maxima 2008/1847/1757 us, with zero >3000/>=6000 crossings,
valid calibrations and zero USB/idle errors. Steady adds 9000 callbacks without
new crossings/max. Registered s07 return has independent full readback, cold
boot and owner-confirmed sound/Pause/CUE; 13.566 seconds of played frames have
zero USB/underrun errors. All monitors are closed. Exact full current state:
`1a45683632f9951f1121ad0866fd7b3050d146022bfc6c719be18346848ad4fe`.
Future plans require all 38 executed predecessors. The 37697960-byte completed
archive and every secondary copy/hash pass. Next: independently register the
clean normal engine for bounded MAIN/PFL/two-deck LOAD/seek/CUE/EOF and I02-I13,
including playing-source resource budgets, USB-client runtime stack and
minimum internal/largest heap observations. No normal engine write or product
qualification is implied by idle timing. Keep DAC/APTA/networking out of this
next campaign. Earlier offline-only paragraphs retain their historical gates.

Historical implementation/offline checkpoint: `ogranak/jc1060-resource-readiness`, draft
[PR #80](https://github.com/dvucinozd/Ruzina/pull/80) above #79. Read the
[resource readiness record](reviews/2026-11-02-resource-readiness.md).
Source `d28abd0` prepares both decks' permanent stores fallibly before UAC
activation, preserves SD/USB ownership and leaves media admission separate.
All six host profiles, nine target Clippy configurations, six real C cases,
exact-parent CodeRabbit (0 issues) and clean normal/profile packages pass.
Source/artifact hashes and static RAM/stack estimates are recorded; actual
readiness time, heap/stack high-water and USB timing are NOT_RUN. Both source
`d28abd0` and publication `a2997bd` passed 21/21 CI jobs. Candidate and s07 return
plans are now offline registered against 36/37 predecessors, and both offline
preflights plus a 311-member dual-copy archive pass. Neither plan was executed;
accepted s07/36 predecessors remain unchanged. Registration `79e251a` and
execution-tool source `51c6967` now each pass 21/21 CI. Reviewed guarded
programmer/passive monitor, 13 host tests per OS and the exact-hash binding are
complete; see the record's latest section and tools README. Device access is
explicit and remains unexecuted at this checkpoint. The owner confirms the
song list is visible. The owner subsequently requested agent-performed ROM
entry; one supported native USB software entry succeeded with fresh rev103/MAC/
security identity and no flash write. See the latest record. This substitutes
only the entry method: the chip is currently held in ROM, s07 still installed,
36 executed predecessors. Physical cold boots remain required. The tools
default to offline; raw plan templates are not the
execution path. Next is
the registered three-cold/steady idle resource/timing comparison, followed by
exact s07 return. I02-I13 follow only after
the timing/resource gates permit them. No merge or broader architecture change.

Historical decision checkpoint follows; the implementation above supersedes
its NOT_IMPLEMENTED state while retaining its ownership and acceptance rules.

Current offline decision: `ogranak/jc1060-usb-service-decision`. First read the
[USB service-budget/resource design](reviews/2026-11-02-usb-service-and-resource-decision.md).
The first three next-step audit tasks are complete. USB callbacks serialize;
nominal 24 ms pool depth is not a per-call budget. A 581272-byte per-deck
permanent resource initialization follows catalog PASS while USB already runs;
deck 2 repeats it on first LOAD. Contention is plausible, not proven.
Decision: defer render-ahead ownership/latency migration; implement a fallible,
idempotent both-deck resource preparation gate before UAC prefill/submission
as the next single candidate. Exact states, tests and unchanged gates are in
the design. No firmware implementation or COM3 access in this audit. S07 remains
accepted; recovery ancestry 36. Do not silently combine this with a new task,
queue-size change or broader IRAM placement.

Latest work: `ogranak/jc1060-pack-placement`, PR #78 above #77. Read the
[completed device comparison](reviews/2026-11-02-pack-placement-device.md) first.
Control maxima: 15827/17386/15849 us; IRAM: 14157/13599/15548 us. Both fail;
narrow pack placement is not a qualified fix. Each separate steady window adds
9000 callbacks without new crossings. Exact s07 is restored/readback and cold
boot verified, with owner-confirmed sound/Pause/CUE and 20.112 s captured sound,
zero underruns/USB errors. All monitors are closed. Future plans require **36**
predecessors and reconstructed full state
`7df030bf565c63503a04bfaf2a9cd114632c60e5f357335da9f49d81c2e544e6`.
This is reconstructed from verified predecessor/writes, not an independent
whole post-restore read. The unused control-return is not in the executed chain.
Next is the [service-budget and render/refill design decision](reviews/2026-11-02-post-placement-next-step.md),
PLAN_ONLY. Do not run another placement variant or assume a cache mechanism.
Source/build publication `f0ba8ca` passed 21/21 CI; later evidence CI is separate.
The paragraphs below are historical preparation/results, not current actions.

Historical offline continuation: `ogranak/jc1060-peer-continuity-fixes`. Read the
[peer continuity repair record](reviews/2026-11-02-peer-continuity-fixes.md).
Per-deck revisions preserve the cached mixed suffix and reset only the changed
deck at the next render boundary; media revocation still clears immediately.
The analyzer rejects malformed unsigned record/cause/stage data. These fixes
need fresh exact-image timing; old failed timing evidence is retained below.
No COM3 access; s07 remains the last physically accepted image. Cache-counter
ownership/mapping remains the next offline gate before a bounded experiment.

Latest work: `ogranak/jc1060-uac-cause-profile`, continuing from `62717ce`.
Read the [calibrated device result](reviews/2026-11-02-uac-cause-profile-device.md)
first. [PR #76](https://github.com/dvucinozd/Ruzina/pull/76) contains source
`3e20a8e` (20/20 source CI jobs), clean normal/profile packages and the executed
diagnostic/return plans. Both target calibrations pass. Worst callback 14812 us
has no observed switch delta and 1593–1653 us of covered C-dispatch IRQ time.
The aggregate gate fails; the remainder is not established CPU/cache cost.
Separate steady idle adds 9000 callbacks without new threshold crossings.
Exact s07 was restored/readback/cold-boot verified; the owner confirms normal
sound, Pause and CUE, with zero captured underruns/USB errors. All monitors are
closed. Future plans require **33 predecessors**, reconstructed full SHA-256
`4324fee5c109cc9fe28aebbafa8f5cf2116ee4950afce374be5ef6f81aa2ae68`.
Next is the [bounded cache/compute plan](reviews/2026-11-02-uac-cache-compute-follow-up.md),
PLAN_ONLY; no priority, DSP, gain or native-minimum change.
The paragraphs below retain prior measured checkpoints; their predecessor
counts and not-run states do not supersede the current result above.

Latest continuation: `ogranak/jc1060-uac-callback-profile`, [PR #75](https://github.com/dvucinozd/Ruzina/pull/75).
Read the [callback device record](reviews/2026-11-01-uac-callback-profiling-device.md)
first: source `8a9fe16` has CodeRabbit 0 issues, 20/20 CI jobs and clean normal/
profile packages. Two retained cold callbacks took 13087/13039 us; their valid
exclusive partitions report zero observed scheduler deltas. Wall time still
includes IRQ/cache/observer effects, and runtime hook-liveness calibration is
open. The aggregate >=6000 us gate fails; there is no timing fix yet. A separate
steady window adds 9000 callbacks without new threshold crossings. Exact s07
was restored/readback and cold-boot verified; the owner confirmed normal sound,
Pause and CUE, with zero captured underruns/USB errors. All monitors are closed.
Future plans require **31 predecessors**, reconstructed full SHA-256
`817c75491f1c6f3b8e482b576ce44cec43693458d540685ede6adae392123e8d`.
Next is the [cause experiment plan](reviews/2026-11-01-uac-callback-cause-follow-up.md)
(PLAN_ONLY): calibrate switch counters, then distinguish IRQ/cache/compute
effects before choosing a fix. No priority, gain or native-minimum change.
The older sections below describe prior checkpoints, not new device evidence.

Updated 2026-11-01. Continue from `D:/AI/M1_PCB`, branch
`ogranak/jc1060-review-fixes`. Base is `b35dcd39f4c1ddb9f4b253d6af89110ee68a2335`;
fetched main was `bd52402160af27a3e234f7647d703a847644817b`. Existing planning
and review documents were already uncommitted when repairs began. Inspect Git
status before staging; do not reset or sweep another agent's files.

Current continuation is `ogranak/jc1060-uac-fill-timing`, source `5c579e6`,
[PR #74](https://github.com/dvucinozd/Ruzina/pull/74) above #73. Read the
[phase diagnostic execution](reviews/2026-11-01-uac-timing-execution.md) first:
CodeRabbit 0 issues, 19/19 CI, four clean packages and registered plans are
verified. The later [phase measurement](reviews/2026-11-01-uac-timing-device-results.md)
found 10373 us in callbacks (3/6000 >=6000 us), versus 1844 us initial prefill.
There is no timing fix yet. S07 has been restored/readback and cold-boot verified;
normal sound, Pause and CUE were owner-confirmed with zero captured underruns
and USB errors. All monitors are closed. Future flash plans need all 29
predecessors, including this diagnostic and its return-to-s07 plan.

Read in order:

1. [Repair execution log](reviews/2026-11-01-review-fix-execution.md): actual implementation, commands, logs, package hashes and remaining gates.
2. [Detailed implementation plan](reviews/2026-11-01-review-fix-implementation-plan.md): steps A–H offline; I is later physical acceptance.
3. [Review checkpoint](reviews/2026-11-01-code-review-and-next-steps.md): original defects and failure reproductions.
4. [Current development status](DEVELOPMENT_STATUS.md): separate legacy/native completion levels.

Offline steps A–H are complete: 17/17 IDF packages, host/target tests and final
CodeRabbit (0 issues) pass. PR #73 published the repair source as `b454092` and
passed 19/19 CI jobs. The later [device campaign](reviews/2026-11-01-review-fix-device-results.md)
booted the exact candidate, failed the registered fill timing criterion and
restored s07. Cold boot is verified; the owner confirmed normal sound, Pause
and CUE again. The short capture has zero underruns and USB packet errors;
all serial monitors are closed.

The offline repairs cover descriptor publication, transactional LOAD/PLAY,
identified seek outcomes including EOF/failure, media revocation, USB recovery
and bounded producer work. The execution log is the authority for which gates
passed. Host/build success does not qualify the candidate on hardware.

## Device and publication boundary

Observed hardware: JC1060 NewPanel, ESP32-P4 v1.3/103. Native minimum remains
302. Use only the separate IDF 6.1 backend for this unit. No COM3 access, flash,
RAM execution or gain changes occurred in the repair batch.

Latest accepted playback evidence in the reviewed repository:
[deck-play](reviews/2026-11-01-legacy-deck-play.md), app `95d45203...`, one-deck
44.1 kHz FLAC/MP3/WAV through FLX4 headphones. The [s08/s09 reconciliation](reviews/2026-11-01-review-fix-history-reconciliation.md)
verified all 25 prior plans and a byte-exact 16 MiB live s09 predecessor. The
campaign then wrote the repair candidate and returned to s07 through a new
ancestry-aware plan. Future upgrades must include those two additional plans.

[PR #71](https://github.com/dvucinozd/Ruzina/pull/71) was deck-mix against main;
[PR #72](https://github.com/dvucinozd/Ruzina/pull/72) was deck-engine stacked on
#71. Refresh their state before any publication and merge in dependency order.
The owner subsequently authorized a separate repair PR above #72 and CI on its
published head. See the publication continuation in the execution log and the
[remote repair branch](https://github.com/dvucinozd/Ruzina/tree/ogranak/jc1060-review-fixes).
PR #73 remains unmerged. The subsequent device campaign result is separate from
the published offline validation; passing CI does not close its timing gate.

## Next physical campaign and later development

Start with the [callback profiling plan](reviews/2026-11-01-uac-callback-profiling-plan.md):
the original candidate's aggregate maximum was 12260 us; the instrumented image
has a callback maximum of 10373 us, above the same registered 6000 us bound.
The substage/preemption mechanism is unproven. Initial prefill was below 3000 us
in the instrumented run; do not assume eager initialization will fix it.
After the timing gate passes on a new exact image, validate two-deck MAIN/PFL, CUE/seek/LOAD races,
44.1/48 kHz sources, SD loss and USB reconnect with deadline, transfer, ring,
heap and stack telemetry. Current SD reinsertion requires app restart.

Then wire source cues/analysis and missing performance features in bounded
batches, followed by product UI/settings, C6/networking and OTA. JC8012 needs
its own qualification. APTA remains excluded from the product integration.

[External audio plan](boards/JC1060_EXTERNAL_AUDIO_PLAN.md) records PCM5122 MAIN,
optional TLV320DAC3100 headphones and independent Settings. Separate external
I2C avoids the onboard ES8311/TLV320 address collision at `0x18`. Candidate
pin mapping, wiring, drivers and qualification are still future work.

For earlier legacy campaigns, use their dated `docs/reviews/2026-10-30-*` and
`2026-10-31-*` reports and immutable evidence indexes. Their milestones are
historical observations for named images, not current-image instructions.

## Historical handoff — 2026-10-24 and earlier

The following branch identities and test totals are historical snapshots.
Use the current status and execution log above for continuation.
Host-implemented and merged to `main` on 2026-10-23 (PRs #23-#36, each with
host and ESP32-P4 CI passing on `main`): RP-01/02 (PR #23, `ogranak/m25-rust-parity-plan`),
RP-04 load admission (PR #24, `ogranak/m25-rp04-load-admission`), RP-03 CUE
hold/VINYL-CDJ (PR #25, `ogranak/m25-rp03-cue-jog`), RP-05 memory/loop cues
and source restore (PR #26, `ogranak/m25-rp05-cues`), RP-15/18 reconnect
fencing plus PDB row/EXT cue/loop-range edges (PR #27,
`ogranak/m25-rp15-18-edges`), RP-06 playlist hierarchy (PR #28,
`ogranak/m25-rp06-playlists`), RP-07 part 1: PWV4, memory-cue markers,
artwork references and owner (PR #29, `ogranak/m25-rp07-artwork-pwv4`),
RP-08 MP3 frame index/cold seeks/duration (PR #30, `ogranak/m25-rp08-mp3-seek`),
RP-07 part 2: baseline JPEG decoder and cover/logo display (PR #31,
`ogranak/m25-rp07-cover-decoder`), RP-11 durable settings (PR #32,
`ogranak/m25-rp11-settings`), RP-12 Web/C6 contracts (PR #33,
`ogranak/m25-rp12-web-contracts`), RP-13 signed OTA/readiness contracts
(PR #34, `ogranak/m25-rp13-ota`), the RP-09/10/14/16/17 host contracts
(PR #35, `ogranak/m25-rp09-17-audio-platform`) and the UAC 48 to 44.1 kHz
converter (PR #36, `ogranak/m25-uac-resampler`). See the parity
plan rows for evidence and open items (HTTP adapter for uploads, Web Remote admission/tap
semantics, physical controller/
import/listening acceptance). The sections below still describe the original
baseline.

## Read first and locate the checkout

1. [M2.5 parity audit and acceptance plan](M2_5_PARITY_PLAN.md): 20 open work
   items, pinned sources, implementation evidence, priorities and source links.
2. [Development status](DEVELOPMENT.md) and [product requirements](M1_RUST_PRODUCT_REQUIREMENTS.md).
3. [Architecture](M1_RUST_ARCHITECTURE.md), [validation](VALIDATION.md),
   [controller ADR](adr/ADR-004-controller-profile-platform.md) and
   [OTA ADR](adr/ADR-007-ota-boot-recovery.md).
4. [Board matrix](boards/GUITION_HARDWARE_MATRIX_2026-10-20.md) and
   [first-boot procedure](boards/GUITION_FIRST_BOOT_2026-10-20.md) before hardware work.

Repository: `https://github.com/dvucinozd/Ruzina.git`.
Continuation branch: **`ogranak/m25-rust-parity-plan`**. Audit/planning commit:
`9e1c4344833b808bfaf5305399f09156f59b56a7`; this handoff is a subsequent
documentation-only commit on that branch. Read the branch's current HEAD rather
than treating the earlier audit commit as the complete handoff.

On this Windows machine the active continuation checkout is:

```text
C:\Users\Daniel\.codex\worktrees\m1-guition-bsp\M1_PCB
```

Reuse that worktree after checking its current status. The primary checkout
`D:\AI\M1_PCB` was clean on `main` at
`0f2fb1425ef76f2541a8c76841cfa52178dca589` when this handoff was prepared.
This branch is already checked out in the worktree; do not force it into the
primary checkout or reset either tree. Main was not merged/updated by this task.
Fetch and inspect current refs before relying on these snapshots. In a different
clone, fetch origin and select the continuation branch (create a tracking branch
only if it does not exist locally). Preserve any intervening user changes.

## Frozen donor and target boundaries

- Donor: `https://github.com/dvucinozd/Pajoniiir.git`.
- Original M2.2 foundation: `2c2ec32c253d368765123d7bbf8d37389b790b55`.
- New stable functional reference M2.5:
  `20f1c3f04a615209ae25e9bbdae649d0f5b44e8d`.
- Audited newer donor master: `5f41df1bfec994e325ec1ccdd4b3ebf4ad88061e`.
- Local donor checkout: `D:\Documents\DDJ-FFL4`; use pinned `git show`/diffs
  and preserve its independent changes/branch. Its contents were not modified.
- Stable M2.5, shared-P4 beta images and current master have different scopes.
  The parity plan names which later fixes supply useful regression contracts.
  Do not claim beta features or controller acceptance belong to stable M2.5.
- Targets are Guition **JC1060P470C_I_W_Y** and **JC8012P4A1C_I_W_Y**,
  owner-reported silicon v3.2; configured minimum revision 302. Boards remain
  in transit at handoff. All physical acceptance remains pending.

Native Rust/no_std and Slint remain the architecture. Existing shared host code
already covers two-deck control, Hot Cue journaling, media identity/generations,
sample-exact decoding/seek, loops, scratch, Master Tempo, mixer/FX and independent
MAIN/PFL routing. The audit identifies specific gaps; do not restart these
foundations or copy the C donor tree wholesale.

## Implementation order and acceptance

1. **RP-01/02: parser and profile compatibility.** First correct
   `firmware-rust/crates/ruzina-rekordbox/src/lib.rs` title lookup from index
   18 to 17. Prove it with independent 17/18/19 strings, UTF-16 and fallback/
   malformed fixtures. Preserve existing PQTZ 1..4 -> 0..3 conversion. Then
   complete S3CP v2/v3/v4 through parser, fixture compiler, registry and storage:
   CC7 scaling, NOTE_SELECT, bounded initial output SysEx, LED scaling and filter
   capability. Preserve old bytes and replay behavior, reject malformed/version-
   incompatible records and fence initialization to the current binding. See
   RP-02 for complete bounds; future upload validators must share the contract.
2. **RP-03/04/05 plus RP-15/18: transport/cues.** Implement CUE hold preview,
   release/return and PLAY commit; independent VINYL/CDJ actions; one load-lock
   admission/commit path, including PLAY/LOAD races; full loop cue recall,
   separate memory cues and source restore/local/tombstone precedence. Add
   pending input/LED reconnect fencing and relevant EXT/remote-tap/loop bounds
   regressions. Simulator-only load checks are not shared async admission.
3. **RP-06/07/08: Library/media.** Add ordered nested playlists, bounded artwork
   and fallback, optional PWV4 and predictable cold MP3 seek/duration discovery.
   Preserve sample-exact gapless/reservoir behavior and accepted-output progress;
   decoded duration must remain independent of catalog length/analysis span.
4. **RP-11/12/13: services.** Versioned durable settings, transactional write/
   migration policy, C6 fake-transport/Web contracts and signed A/B OTA with
   strict manifest/identity/TLS/retry/readiness/rollback behavior. Host fault
   injection comes before real networking/flash activation.
5. **RP-09/10/14/16/17: audio/platform.** Host descriptor/packet/clock/teardown
   work can proceed now. Actual UAC/DAC output and product-owner activation
   require exact BSP boot/memory/peripheral/power qualification. Retain later
   donor headphone gain, MT fallback and bounded cache/diagnostics regressions.
6. **RP-19/20 and other P1:** explicitly scoped extra controllers, absolute
   latching PFL, Pro DJ Link, extended UAC and ordinary folder/tag/sidecar work.
   These do not replace unfinished P0 acceptance.

Use separate reviewable commits/PRs for implementation batches. Run targeted
tests and the relevant shared/Slint/target checks before merge. The handoff push
does not merge this branch or authorize a firmware release. The audit defines
acceptance criteria; RP-01..20 remain open until their behavior is implemented
and verified. No test result should be inferred from a requirement table.

## Rename to Ruzina (2026-10-24)

The Rust product is **Ruzina**: crates `ruzina-*`, identifiers `ruzina_*`, the
firmware binary `ruzina-m1-firmware` and the repository `dvucinozd/Ruzina`.
**Pajoniiir** remains the name of the C donor (`dvucinozd/Pajoniiir`, M2.5)
wherever the text means the donor.

Owner decisions on persisted and wire names:

- The media identity domains Rust owns are now `ruzina.track.v1`,
  `ruzina.volume.fat32.v1` and `ruzina.volume.exfat.v1`. No Rust unit had
  stored identities yet, so nothing needs a migration.
- `pajoniiir.hotcue.v2` stays: it is the donor's domain
  (`media_identity.c`), which Rust uses to compute donor Hot Cue identities.
- The Web Remote hostname is `ruzina.local` (only), and the planned media
  sidecar directory is `/RUZINA`. Any donor `/PAJONIIIR` data is never
  overwritten.
- The OTA test bundles are signed for project `ruzina-m1-test`. Production
  projects are board-specific `ruzina-*` names.

Deliberately unchanged: donor URLs, commits and M2.5 quotes, `.ddjota`,
fixture provenance and fixture data (for example the `Pajoniiir test fixture`
vendor strings inside S3CP JSON).

Branding: the owner-supplied artwork is in `firmware-rust/assets/branding`
(emblem, wordmark and a splash composition, with SHA-256 records).

- The emblem is the fallback cover (34x34/40x40 RGB565, generated by a
  standard-library script).
- The M2.5 idle screensaver is ported: `ruzina-ui-model::idle` has the
  timing, wake and input rules (2 min default, playback and recording inhibit
  and count as activity, a wrap-safe clock, a physical wake press consumed,
  a remote mutation delivered).
- The simulator shows the splash on the landscape JC1060 and the emblem over
  the wordmark on the portrait JC8012, with two new reviewed goldens.
- The target display screensaver still needs a display owner and a derived
  RGB565 image.

A case-insensitive search for the old name may show only the donor
references listed above.

## Real Rekordbox export comparison (2026-10-24)

One owner export (Rekordbox 6, 189 tracks, no playlists) was copied off the
stick read-only and compared outside the repository. The Rust side is the
`export_inspect` and `analysis_inspect` examples in `ruzina-rekordbox`. The
M2.5 side is `rekordbox_pdb.c`/`rekordbox_anlz.c` from donor `20f1c3f0`,
built with gcc.

- **Fixed:** every Rust PDB page walk (tracks, names, playlists) followed a
  table past its `last_page`. Rekordbox links the last page to the table's
  empty candidate, then to page 0 and past the end, so the firmware import
  failed with `PageOutOfRange` on this export. `PdbPageHeader::next_in_table`
  now ends the chain at `last_page`, and index pages (`page_flags & 0x40`)
  yield no rows.
- **PDB:** all 189 tracks match M2.5 in title, key, BPM, duration, paths and
  cover. Rust additionally resolves 9 non-ASCII artists and the album table,
  which M2.5 drops.
- **ANLZ:** beat grids, waveforms, color previews and memory cues match for
  all 189 tracks, and hot cues for 188. In the last track, cue D exists only in
  the EXT PCOB; Rust imports it and M2.5 drops it.
- **Still open:** a real export with playlists, and physical USB import on the
  board.

The remaining RP-18 import edges followed the same day:

- A rejected EXT is discarded and the DAT analysis is kept.
- A cut-off trailing PWV4 degrades to PWV3.
- Names are bounded to their own row.

The real export is unchanged by them: no name row out of bounds, and the same
PDB and ANLZ results. A real EXT cut inside PWV4 or PWV3 matches the donor.

## Policies that must survive continuation

- **Headphones/CUE/PFL always use controller UAC.** A ready external stereo DAC
  takes MAIN priority, then compatible controller MAIN, then mute. MAIN switching
  must not reroute headphones. No onboard-codec/external-DAC headphone fallback.
- Keep APTA OFF in Guition/default product graphs. Preserve the original
  `codex/native-apta-consumer` branch; do not merge it. Already selected portable
  adapter/lifecycle work does not close native analysis or accuracy gates.
- `integration-check` only compiles owner combinations. It acquires no board
  resources, registers no heap regions and starts no product tasks. No Guition
  selector currently runs a complete DJ product runtime.
- Board-specific USB instances/roles, SD power/presence, journal partition layout,
  display/touch, DMA/cache and C6 transport need explicit qualified ownership.
  Do not reintroduce retired custom PCB/KiCad artifacts or hard-coded donor pins.
- JC8012 touch-payload distribution rights and exact panel/unit qualification
  remain open. Preserve licence/provenance for imported code, fixtures and assets.
- Recorder stays LATER/OFF; donor `ui_next` is a disabled demo prototype, not
  accepted Slint product scope. No other-board hardware result transfers here.

## Verification at handoff

The audit ran these existing Rust tests successfully at the unchanged firmware
source (`0f2fb1425`); only documentation has changed since:

```sh
cd firmware-rust
cargo test --locked -p ruzina-controller-profile -p ruzina-controller-session \
  -p ruzina-deck -p ruzina-rekordbox -p ruzina-audio-runtime -p ruzina-mixer
```

**315 passed, 0 failed, 0 ignored.** This is not an independent M2.5 differential
suite and does not close the discovered title/profile/transport gaps. Test log
is retained locally outside the repository at
`C:\Users\Daniel\.codex\worktrees\m1-guition-bsp\research\m25-audit-tests.log`.
The audit also verified all eight pinned donor source links against Git objects,
20 RP items, unique requirement IDs, local Markdown targets and whitespace.
For this handoff, the local documentation checker passed all 70 links. No CI
result is claimed for the branch-only documentation push.

Recheck documentation before commit:

```sh
python3 firmware-rust/tools/check_docs.py
git diff --check
git diff --cached --check
```

Use committed Rust 1.95.0 and lockfiles. Full host/Clippy/UI, runtime-owner and
P4 checks are listed in [validation](VALIDATION.md) and the two CI workflows.
They were not rerun for this documentation-only handoff; neither were donor
builds or physical procedures. Hosted CI must be inspected per actual run;
unstarted billing-limited jobs are not PASS.

Physical first boot/recovery, factory backup/readback, flash/PSRAM, display/touch,
USB/SD power and reconnect, UAC/DAC timing, latency, listening, OTA and soak on
both boards remain NOT RUN. M2.5's donor soak waiver never transfers to Rust.
