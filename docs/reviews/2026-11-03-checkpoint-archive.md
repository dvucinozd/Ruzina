# Development checkpoints, 2026-11-01 to 2026-11-03 (archive)

These paragraphs were the top of `README.md` and `docs/DEVELOPMENT_STATUS.md`
until 2026-11-18. They are kept as they were, with their links rewritten for this
folder. They describe the bring-up campaign before two MP3 decks played clean
(s07, history 44), not the current state: see the [handoff](../HANDOFF.md) and the
[current status](../DEVELOPMENT_STATUS.md).

## From README.md: development checkpoint, 2026-11-03

Latest [legacy MP3 reset fix](2026-11-03-mp3-in-place-reset.md)
removes the large runtime reset/seek temporary through safe in-place clearing.
Both normal and opt3 builds now pass unchanged stack guards; the reset frame
is 16 B and opt3's maximum Rust frame is 16048 B. Host PCM remains byte-exact.
This is build/host evidence only: no new COM3 access, s07/history44 unchanged.
Exact-source review/CI, clean packages and fresh registered comparison still
precede any physical campaign. Native minimum 302 and compiler defaults stay.
Review completed with 0 issues; all three clean packages and their dual-copy
archive pass. Exact-source host CI passed; legacy CI remains queued.

Latest [offline source-budget investigation](2026-11-03-source-budget.md)
adds a deterministic shared-time producer test, release-mode PCM benchmark
and control audit. S03 contained 74 extra control messages, so its original
numerical admission is not isolated-control acceptance. A default-OFF MP3
compiler experiment improves host decode time with exact PCM equality, but
the P4 build is **REJECTED**: constructor frame 39008 > 24576 bytes; independently
estimated load chain 90544 > 90112 bytes. No new COM3 access or flash.
Next: bounded-stack decoder reconstruction before another candidate or plan.
The reviewed audit fix has 0 CodeRabbit issues; final evidence is archived on
both drives. New remote CI remains separate from completed local validation.

Latest [continuous device campaign](2026-11-03-continuous-device.md):
cold/idle and each MP3 source separately passed their observation gates. Both
playing failed after 15.88 seconds with source underruns **78/94**, matching
the owner's crackling report. This is not a dual-deck pass. Exact s07 was
restored with full 16 MiB readback; one software boot pulse was issued, with
application execution **NOT_OBSERVED**. No repeated manual recovery checks.
Current executed history is **44**; 42/43 plans are consumed. Evidence has
verified primary/secondary archives. Next: offline decoder-step/read budget
attribution before selecting a single change and registering another campaign.
Native minimum remains **302**; no experimental image is promoted.

Earlier preparation and device checkpoints below are historical; the result
above supersedes their current-state and next-action statements.

New [42/43 continuous campaign preparation](2026-11-03-continuous-registration.md)
has both offline plans/contracts and full ancestry reconstruction passing.
The 53-test combined suite passes per OS. Writer review raised 0 issues;
exact source CI passes 22/22. All three independent write/capture/boot bindings
and both dual-copy archives are verified. No new COM3 access. Next: verify
publication/offline bytes and refresh topology before the candidate experiment.

The preparation checkpoints below retain their earlier counts and status.

The [continuous COM3 adapter and analyzer](2026-11-03-continuous-adapter.md)
now pass offline whole-session and fault tests. They gate each case on validated
continuous evidence and actual owner readiness. No new target access: software
boot action, fresh 42/43 registration/binding and review/CI still precede use.

The separate [s07 software boot action](2026-11-03-software-boot.md)
is also implemented offline; the combined 46-test suite passes per OS.
Review/CI and new policies/bindings remain required. No manual s07 recovery
qualification is reinstated.

CodeRabbit is now connected to Ruzina. Combined tooling review is complete
(one minor documentation issue fixed; final fix review 0 issues), with a
verified dual-copy archive. Latest source CI/new 42/43 registration remain
separate gates before device use.

Offline [continuous capture core](2026-11-03-continuous-capture.md)
now retains the full stream, complete report offsets and owner-ready case
boundaries. Its host tests pass; live adapter/analysis and fresh 42/43
registration still precede another target attempt. No new COM3 access.

Latest 2026-11-03 [producer diagnostic result](2026-11-03-source-supply-device.md):
cold/idle pass at **1880 us**, playback **NOT_CAPTURED**. The functional window
ended without an active source and its initial joined telemetry was rejected.
Dual-MP3 root cause remains open. Exact **s07 is restored**, tools are closed;
current executed history is **42**, whole flash
`28830827d3e3426bbcd58b9a25d4d2129e28f150bade33fe3c551cd577f93921`.
Next: continuous capture/framing and operator-ready coordination before a new
42/43-plan campaign. The [new recovery policy](../RECOVERY_POLICY.md) removes
repeated manual s07 cold/listening/Pause/CUE checks; automatic readback remains.

The earlier development checkpoints below retain their original results.

Continuation 2026-11-03: [source-supply/USB diagnostic preparation](2026-11-03-source-supply-profile.md)
adds default-OFF wall measurements to identify the dual-MP3 bottleneck.
Host/target checks, final fix review (0 issues), **22/22 CI** and clean normal/
diagnostic builds pass. Exact packages/sources are archived on both drives.
The separate [40/41-plan target registration](2026-11-03-source-supply-execution.md)
passes offline checks; execution/analysis tools have final review 0 issues,
22/22 CI and a frozen dual-copy binding.
At that preparation checkpoint no device access/promotion was claimed; the
later physical result above supersedes its current-state/next-step status.

The observed JC1060 has ESP32-P4 **v1.3/103**, 16 MiB flash and 32 MiB PSRAM.
It uses the separate pinned **ESP-IDF 6.1 + shared Rust** backend. The native
Rust backend still requires revision **302** and must not be flashed to this unit.
JC8012 has no transferable hardware acceptance from JC1060.

Latest [normal-engine device campaign](2026-11-02-normal-engine-device.md)
on [draft PR #81](https://github.com/dvucinozd/Ruzina/pull/81) is **REJECTED**
for two-deck playback. Cold/idle pass at **2397 us** maximum; each MP3 source
alone is owner-confirmed, but misses the <=3000 us headroom target (3214 us).
Both together drain their buffers: **4765/5003 underruns**, one USB packet
error, fill max **4982 us**. PFL and N03-N08 are not accepted/run after the stop.
Source/tool review, 21/21 CI and dual-copy evidence are retained separately.

Exact accepted **s07 is restored** with full readback, physical cold boot and
owner-confirmed sound/Pause/CUE. All monitors are closed. Current executed
history is **40**, whole flash
`7875f650ba7404475e9c26561990851b04c11409b3f309401f6e77a51ddf38a6`.
Next: offline per-deck producer/SD/USB service analysis and a separately reviewed,
registered diagnostic; no normal promotion or automatic merge.

Earlier result: [resource readiness before UAC](2026-11-02-resource-readiness-device.md),
[draft PR #80](https://github.com/dvucinozd/Ruzina/pull/80). Preparing both
permanent deck bundles before audio activation passes three cold idle windows:
callback maxima **2008 / 1847 / 1757 us**, zero >3000/>=6000 crossings across
21000 callbacks. Separate steady idle adds 9000 callbacks with no new crossing.
Calibrations pass, with zero USB/idle errors. Host/target checks, both exact-parent
CodeRabbit reviews (0 issues), clean packages and source/tool CI pass separately.

At that earlier checkpoint, exact accepted s07 was restored with whole-flash
readback, cold boot and owner sound/Pause/CUE confirmation. Recovery then needed
**38** predecessors; the independent full hash then was
`1a45683632f9951f1121ad0866fd7b3050d146022bfc6c719be18346848ad4fe`.
That passed the registered idle timing gate. The later normal-image result
above supersedes its current-state/next-step status.

Historical device checkpoint: the [packetizer placement comparison](2026-11-02-pack-placement-device.md)
completed three cold boots per image plus separate steady idle. Control maxima
were 15827/17386/15849 us; IRAM maxima 14157/13599/15548 us. Both fail the
unchanged >=6000 us gate. Narrow IRAM placement is insufficient and is not
enabled for the product. Exact s07 was restored/readback/cold-boot verified;
the owner confirms sound/Pause/CUE, with 20.112 s captured playback and zero
underruns/USB errors. That checkpoint had **36** executed predecessors; current
ancestry is **40** above. The [service-budget decision](2026-11-02-usb-service-and-resource-decision.md)
led to the completed resource-readiness campaign above, which supersedes its
earlier next-step instructions.
The paragraphs below retain earlier per-image checkpoints and their limits.

The latest accepted playback record in this repository is
[deck-play](2026-11-01-legacy-deck-play.md): one deck reads the SD
Rekordbox export and plays FLAC/MP3/WAV through the DDJ-FLX4 headphones, alongside
the library display/touch and MIDI/LED control. This is bounded device evidence,
not full product qualification. The [s08/s09 recovery reconciliation](2026-11-01-review-fix-history-reconciliation.md)
now verifies the complete history and live 16 MiB predecessor image.

`deck-mix` and `deck-engine` add two-deck MAIN/PFL, mixer processing and product
transport. Their PRs #71/#72 were open at the reviewed base. The local
`ogranak/jc1060-review-fixes` branch applies the
[review repair plan](2026-11-01-review-fix-implementation-plan.md):
transactional LOAD, identified seek receipts, coherent ring descriptors, media
revocation, USB recovery and incremental producer scheduling. Read the
[execution log](2026-11-01-review-fix-execution.md) for actual test
and build results. Offline steps A–H are complete: all 17 IDF packages and the
host/target checks pass; final CodeRabbit reports 0 issues. Published [PR #73](https://github.com/dvucinozd/Ruzina/pull/73)
passed 19/19 CI jobs. The [first device campaign](2026-11-01-review-fix-device-results.md)
booted the candidate but failed its registered timing gate (`fill_max_us=12260`,
limit 6000 us), despite idle underruns 0/0. The exact accepted deck-play image
was restored; cold boot, normal sound, Pause and CUE are confirmed again.

The [phase diagnostic](2026-11-01-uac-timing-device-results.md) has
now measured the outlier: initial prefill maximum 1844 us, callback maximum
10373 us (3 of 6000 callbacks >=6000 us). The aggregate criterion still fails;
substage/preemption cause is not established. The s07 image was restored and
cold-booted; normal sound, Pause and CUE were confirmed again, with zero captured
underruns/USB errors. All monitors are closed.
The later [callback substage run](2026-11-01-uac-callback-profiling-device.md)
captured valid exclusive records at 13087/13039 us. The worst record includes
6523 us graph and 3725 us PCM packing wall time, with zero observed scheduler
switch deltas. IRQ/cache/observer effects and runtime hook-liveness remain open;
these numbers do not establish pure CPU cost or a fix. A separate 9000-callback
steady window adds no threshold crossings. S07 is restored and owner-confirmed
again, with zero captured underruns/USB errors; all monitors are closed.
That checkpoint led to the [cache/compute experiment plan](2026-11-02-uac-cache-compute-follow-up.md);
the latest completed placement result and next action are above.
The [calibrated cause experiment](2026-11-02-uac-cause-profile-execution.md)
has now [run on the exact rev103 unit](2026-11-02-uac-cause-profile-device.md)
in [PR #76](https://github.com/dvucinozd/Ruzina/pull/76). Both liveness
calibrations pass; the 14812 us worst callback has zero observed task-switch
deltas and 1593–1653 us of covered IRQ dispatch inside the call. Timing still
fails; uncovered wall time is not pure CPU cost. Separate steady idle adds no
crossings. S07 is restored/readback/cold-boot verified and owner-confirmed for
normal sound/Pause/CUE; all monitors were closed. That checkpoint required 33 predecessors;
the later completed placement campaign raises the current count to 36.
Source cues/analysis, loops, scratch, keylock and FX wiring, product
UI/settings, C6 networking and OTA remain separate subsequent development work.
See the [current status](../DEVELOPMENT_STATUS.md) and
[handoff](../HANDOFF.md). Historical bring-up results retain their own
image hashes and scenario limits in `docs/reviews`.

## From DEVELOPMENT_STATUS.md: current development status, 2026-11-03

Latest [legacy reset implementation](2026-11-03-mp3-in-place-reset.md)
eliminates runtime MP3 reset/seek temporaries with safe in-place clearing.
Normal and opt3 target builds pass unchanged static stack gates; host PCM and
codec semantics pass. BUILD_ONLY_NOT_QUALIFIED; no new COM3 access. S07/history44
and native minimum302 persist. Next are exact review/CI, clean artifacts and
registration of a matched-control experiment; no hardware improvement claimed.

Latest [offline source-budget result](2026-11-03-source-budget.md):
host decoder compiler sensitivity is reproduced with exact PCM equality.
The isolated opt3 core target build is rejected by frame limit (39008/24576)
and independent load-chain audit (90544/90112). No new target access or timing
qualification. S03 has 74 additional controls; its numerical admission does
not establish isolated-control acceptance. Next: bounded-stack reconstruction,
then fresh review/build gates before any 44/45 registration. S07/history44 remain.

Latest [continuous hardware observation](2026-11-03-continuous-device.md)
admits cold/idle and individual MP3 sources, but rejects both-playing with
78/94 source underruns after 15.88 seconds. S07 is restored by full readback;
software boot pulse completed, application NOT_OBSERVED. No manual recovery
checks repeated. Current history **44**; 42/43 plans consumed. Next is offline
decoder-step/synchronous-read budget attribution, then one independently
reviewed change and fresh registration. Native minimum **302** is unchanged.
The following status entries describe older checkpoints and are superseded
where they name current device state, history or next actions.

New [42/43 continuous-source plans](2026-11-03-continuous-registration.md)
and contracts pass actual-file offline preflight; full factory/history and
source checks pass. Combined suite: 53 tests per OS. No target access or new
device result. Writer review raised 0 issues; `b92fb23` source CI passes 22/22.
Dual-copy archives and all separate bindings pass. Next: publication/remote and
offline byte verification, actual topology refresh, then the registered candidate
experiment. Older checkpoint counts/status follow.

Latest [producer-profile campaign](2026-11-03-source-supply-device.md):
cold/idle pass at 1880 us; functional playback NOT_CAPTURED, initial joined
telemetry rejected. S04/S05 NOT_RUN; root cause open. S07 is restored and
owner-confirmed; tools closed. Current history 42/full
`28830827d3e3426bbcd58b9a25d4d2129e28f150bade33fe3c551cd577f93921`.
The [owner recovery policy](../RECOVERY_POLICY.md) removes future repeated s07
physical cold/listening/Pause/CUE checks; keep automatic full readback.
The [continuous adapter/analyzer](2026-11-03-continuous-adapter.md) now has
39 offline framing/readiness/whole-session tests per OS. No COM3 access. Next:
verify latest CI for the reviewed adapter and [software boot action](2026-11-03-software-boot.md),
new 42/43 plans and exact bindings. Combined suite now has 46 tests per OS. Older checkpoints
below retain historical state.

CodeRabbit connection and combined review are complete: one minor count issue
fixed, final fix review 0 issues. Exact tooling/tests/review attempts have a
verified dual-copy archive; no execution binding or new device result follows.

The [producer campaign](2026-11-03-source-supply-execution.md) is now
offline registered against 40/41 predecessors. Both plans and dual-copy archive
pass; final tool review has 0 issues, exact CI passes 22/22, and the four-file
binding/dual-copy tools archive is frozen. Publication/preflight precedes access.
No COM3 access or new write at this checkpoint;
actual installed s07/history 40/full 7875f650 remains unchanged.

Active [source-supply diagnostic preparation](2026-11-03-source-supply-profile.md)
adds explicit OFF-by-default producer/USB wall measurements, independently of
render/IRQ profiling. Final fix review has 0 issues, exact CI passes 22/22,
and clean normal/diagnostic builds pass source/artifact/symbol/stack checks.
The 15118202-byte archive and all secondary hashes match. No new flash or
registration; installed s07/history 40 and its accepted recovery are unchanged.
Next: separate frozen target runbook, 40/41 recovery plans and exact reviewed
execution/analysis tools; the dual-MP3 root cause remains open.

Latest [normal-engine physical result](2026-11-02-normal-engine-device.md):
**two-deck REJECTED**, s07 restored/full-readback/cold/listening confirmed.
Cold/idle passes at 2397 us; individual MP3 listening passes with max 3214 us
headroom target failure. Both playing: underruns 4765/5003, packet_errors 1,
source supply 0/0, max 4982 us. PFL and N03-N08 stay open/NOT_RUN. Current
executed ancestry **40**, full flash
`7875f650ba7404475e9c26561990851b04c11409b3f309401f6e77a51ddf38a6`.
All monitors closed; dual-copy completed archive verified. Next is offline
source-supply/USB service root-cause measurement design, then new reviewed
40/41-plan registration; no threshold increase, normal promotion or merge.
The paragraphs below describe historical preparation checkpoints.

Latest tool checkpoint: `08a537b` has CodeRabbit **0 issues**, 13 normal monitor
tests per OS (four real-loop synthetic serial cases), and 13 shared-guard WSL
tests. Exact CI passed 21/21 and binding `a35aa0c4...` is frozen/dual-copy
archived; publication precedes execution. No device has been
opened by the normal campaign; the earlier rate-limit entries below are history.

Normal-engine preparation is on [PR #81](https://github.com/dvucinozd/Ruzina/pull/81).
[Source/package and 38/39-plan registration](2026-11-02-normal-engine-execution.md)
pass offline checks, source and registration each have 21/21 CI, and accepted
s07 remains installed. A major capture-duration finding is fixed/tested, but
the 22:10 review found another major connection-coverage issue. The monitor
now excludes initial port waiting and invalidates post-start disconnects;
seven normal coverage tests pass per OS. Prior tool/publication CI passed
21/21 each; this follow-up's own review/CI is pending. No normal binding,
flash or functional acceptance is claimed before those gates close.

The [resource-readiness device campaign](2026-11-02-resource-readiness-device.md)
passes three cold idle windows: 7000 callbacks each, maxima 2008/1847/1757 us,
zero >3000/>=6000 crossings, valid calibrations and zero USB/idle errors. A
separate 60-second steady window adds 9000 callbacks without new crossings/max.
This is idle timing evidence, not two-deck playback qualification. Exact s07
is restored with independent whole-flash readback, cold boot and owner sound/
Pause/CUE confirmation (13.566 captured playback seconds, no errors).
Current ancestry is **38**, full readback `1a45683632f9951f1121ad0866fd7b3050d146022bfc6c719be18346848ad4fe`.
All monitors are closed; device archive and secondary hashes pass. Next is a
separately registered normal-image I02-I13/resource campaign. No normal engine
write, automatic merge or peripheral expansion is implied. Earlier checkpoints
below retain their original limits.

Historical offline checkpoint: the [resource-readiness implementation](2026-11-02-resource-readiness.md)
on `ogranak/jc1060-resource-readiness` ([draft PR #80](https://github.com/dvucinozd/Ruzina/pull/80))
prepares both decks' permanent stores before engine UAC activation. Six host
profiles, nine target Clippy configurations, real C driver tests, exact-parent
review (0 issues), and clean normal/profile builds pass. BUILD_ONLY / NOT_FLASHED:
heap/stack runtime evidence and cold/steady timing remain NOT_RUN. Source
`d28abd0` and publication `a2997bd` each passed 21/21 CI jobs. Candidate/s07
return plans (36/37 predecessors), both offline preflights and a 311-file
dual-copy archive are ready; no plan is executed. Registration `79e251a` and
tool source `51c6967` each pass 21/21 CI; exact-parent tool review has 0 issues
and 13 fake-device tests pass per OS. Reviewed programmer/monitor and their
frozen binding are archived. Fresh operator ROM entry is the next physical gate.
S07, recovery ancestry 36 and native minimum 302 are unchanged. The paragraphs
below retain historical decision/device evidence.

The [service-budget and resource audit](2026-11-02-usb-service-and-resource-decision.md)
closes the post-placement plan's first three steps. DESIGN_ONLY: defer separate
render ownership and first prepare both decks' permanent resources before
product audio activation. Six arithmetic-model checks pass; these do not
qualify USB timing or implement the resource gate. S07/36-plan recovery is unchanged.

The [packetizer placement comparison](2026-11-02-pack-placement-device.md)
on `ogranak/jc1060-pack-placement` (PR #78 above #77) is MEASURED_NOT_QUALIFIED.
Three control boots reached 15827/17386/15849 us; three IRAM boots reached
14157/13599/15548 us. Both fail the registered timing gate. S07 was restored,
readback/cold boot verified and owner-confirmed with 20.112 s captured playback,
zero underruns/USB errors. All monitors closed; future ancestry count **36**.
Next is the [offline USB service-budget/design decision](2026-11-02-post-placement-next-step.md).

Offline continuation after the calibrated campaign: source `eead8c6` on
`ogranak/jc1060-peer-continuity-fixes` fixes peer-deck cached-output continuity
and fail-closed analysis schemas. Host/C tests, exact-parent CodeRabbit review
and clean normal/profile builds pass. See the
[repair record](2026-11-02-peer-continuity-fixes.md). It has no physical
timing acceptance; s07 remains the last accepted image and I02–I13 stay open.
The dated campaign results below remain historical evidence for their images.

Updated 2026-11-02. Development has two distinct backends. Native requires
ESP32-P4 >=302. The arrived JC1060 is v1.3/103 and uses pinned IDF 6.1 with
shared Rust. Active continuation is `ogranak/jc1060-uac-cause-profile`, clean
diagnostic source `3e20a8e64ef7f2ff855bf18706712e289c6cc747`, stacked on the
repair branch through PR #76 -> #75 -> #74 -> #73 -> #72 -> #71. These PRs remain unmerged.

| Component | Current evidence | Next gate |
|---|---|---|
| Shared product logic | Rust host implementations for transport, media import, mixer/DSP and management contracts | Target adapter wiring and physical qualification per feature |
| Legacy SD/library/display | 25-track real export, shared PDB/ANLZ preflight, NewPanel/touch and combined controller diagnostic observed | Broader fixtures and product UI |
| Legacy deck-play | Accepted record: app `95d45203...`, FLX4 headphones, deck 1 FLAC/MP3/WAV; 90 s rendered audio in 15 min USB stream capture | No transfer of this acceptance to newer images |
| Legacy deck-mix/deck-engine | Host/build implementation; s08/s09 history reconciled; no inherited two-deck acceptance | [Controlled cache/compute cause experiment](2026-11-02-uac-cache-compute-follow-up.md) before resuming qualification |
| Review repairs A–H / campaign I | Offline validation complete; published PR #73 passed 19/19 CI jobs; exact candidate booted but I01 timing criterion failed at 12260 us | Candidate rejected for this campaign; s07 restored/readback and cold boot verified, normal playback owner-confirmed with zero captured underruns; I02–I13 NOT_RUN |
| UAC phase diagnostic | [Measured](2026-11-01-uac-timing-device-results.md): prefill max 1844 us, callback max 10373 us, 3/6000 callbacks >=6000 us; zero idle underruns/USB errors | Timing criterion FAIL; s07 restored/cold boot and normal playback confirmed, monitors closed; next is bounded callback substage/preemption profiling |
| UAC callback substage diagnostic | [Measured](2026-11-01-uac-callback-profiling-device.md): coherent 13087/13039 us records, exclusive partitions valid, zero observed switch deltas; 9000 steady callbacks add no threshold crossings | Aggregate FAIL; s07 restored/readback/cold boot and normal sound/Pause/CUE confirmed, monitors closed. [Cause plan](2026-11-01-uac-callback-cause-follow-up.md) awaits hook-liveness and IRQ/cache/compute attribution; future plans require 31 predecessors |
| Native >=302 firmware | Separate compile-only backend on this unit | Compatible silicon and independent hardware gates |
| Calibrated UAC cause experiment | [Measured](2026-11-02-uac-cause-profile-device.md): both target calibrations pass; worst callback 14812 us, switch delta 0, covered IRQ bound 1593–1653 us; no new crossings in 9000 steady callbacks | Aggregate FAIL; s07 restored/readback/cold boot and normal sound/Pause/CUE confirmed, monitors closed. [Cache/compute plan](2026-11-02-uac-cache-compute-follow-up.md) is PLAN_ONLY; future plans require 33 predecessors |
| JC8012 | Separate BSP/source work | Independent arrival/recovery/peripheral/audio qualification |
| External audio | [Future PCM5122/TLV320 plan](../boards/JC1060_EXTERNAL_AUDIO_PLAN.md), including separate I2C and candidate JP1 map | Physical pin/power/module confirmation, drivers and listening/timing campaign |
| C6/Web/settings/OTA | Host/simulator contracts | Real network, durable settings and OTA trust/layout/rollback adapters |
| APTA/recorder/Pro DJ Link | Separately scoped experimental/later work | No automatic product enablement or original APTA branch merge |

The offline repair batch did not contact the device. A subsequent owner-assisted
[physical session](2026-11-01-review-fix-device-results.md) verified the live
identity and full flash ancestry, tested the candidate and returned to s07 after
the timing gate failed. Future plans must include both the candidate and return
plan in the predecessor history; do not reuse the historical s09-only chain.
Current tests/builds and open gates are in the
[execution log](2026-11-01-review-fix-execution.md) and the
[review](2026-11-01-code-review-and-next-steps.md).
