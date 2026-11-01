# Review repair execution — JC1060

Date: 2026-11-01. Branch: `ogranak/jc1060-review-fixes`.
Base HEAD: `b35dcd39f4c1ddb9f4b253d6af89110ee68a2335`.
Fetched main: `bd52402160af27a3e234f7647d703a847644817b`.

Implements the [ordered plan](2026-11-01-review-fix-implementation-plan.md).
Final status: **OFFLINE_A_H_COMPLETE / BUILD_ONLY / NOT_FLASHED**.
I is a prepared later operator-assisted campaign. No COM3 access, flash, RAM
probe or changes to native revision 302, APTA, DAC/pin plans or audio gains.

## Preparation and baseline

Preserved existing local planning/review documentation. Created the repair branch from the reviewed head without resetting or staging other changes. Read the plan, review, handoff, deck-mix/deck-engine records and the shared load admission contract.

A persistent isolated baseline copy was made under `D:/BoardEvidence/JC1060-20261030-COM3/review-fixes-20261101/baseline`. It uses copies of legacy Rust source and the shared crate dependencies. Applied the recorded test-only patch there. The four `review_` tests failed as expected, including the threaded mixed-epoch/origin test. No baseline regression patch was applied to the working checkout.

## A / R3 implementation

Files: `firmware-idf/legacy-jc1060/rust-core/src/deck_play.rs` and `src/deck_play/tests.rs`.

- Added a separate descriptor sequence while preserving the existing single-producer/single-consumer PCM head/tail ownership.
- Writer marks sequence odd before changing start/rate/origin/epoch/terminal state, then marks it even. Every descriptor/sequence access uses SeqCst. Reader loads fields into a temporary, validates the same even sequence, and only then updates its local state and publishes tail.
- One bounded attempt per snapshot; a preempted writer never causes audio-task spinning. Busy snapshots retain the prior reader view and do not change shared tail.
- EOF publication uses the same descriptor protocol. A new epoch clears the previous terminal marker; zero is reserved and skipped at epoch wrap.
- Added deterministic busy/partial/interleaved publication checks, epoch/sequence/head wrap and full-ring ownership checks, and concurrent rate/origin/epoch stress.

The sequence protocol assumes fewer than 2^31 complete descriptor publications occur within one snapshot attempt; sequence equality alone cannot detect a full counter cycle during a suspended read. Runtime media/source lifetime fencing remains a separate step D, and this change does not claim to solve LOAD/PLAY or seek acknowledgement.

## Validation log

Persistent logs are under `D:/BoardEvidence/JC1060-20261030-COM3/review-fixes-20261101`.

- `baseline-regressions.log`: four expected failures on unchanged baseline.
- `step-a-tests.log`: engine feature, 55 tests PASS (52 existing + 3 new).
- Initial matrix invocation suffered Windows/WSL shell argument expansion and ran with empty features; its 0-test result is NOT a feature-matrix PASS. Replaced it with a saved bash script, `step-a-checks.sh`.
- Initial Clippy/build runs exposed the `manual_is_multiple_of` lint, first in production and then test code. These are failed intermediate checks, not final validation.
- Final [feature matrix](evidence/2026-11-01-review-fixes/step-a-matrix-pass.log): deck-play **53**, deck-mix **54**, deck-engine **55** tests PASS; host test Clippy and release RISC-V target Clippy PASS for each. [Saved commands](evidence/2026-11-01-review-fixes/step-a-checks.sh).
- The [original review snapshot regression](evidence/2026-11-01-review-fixes/step-a-original-regression.log) also passes after copying only repaired `deck_play.rs` into the isolated reproduction copy. That copy is now an after-fix reproduction, not an untouched baseline; [the original four-failure log](evidence/2026-11-01-review-fixes/baseline-regressions.log) remains preserved.
- Second build stopped because IDF expected a nonexistent default Python environment. Third build explicitly selected the existing `python-idf61` environment and returned **BUILD_PACKAGE_PASS**, no hardware access.
- [Package result and exact source hashes](evidence/2026-11-01-review-fixes/step-a-build-result.json): app **808256 bytes**, SHA256 `663e99902f8e5415e797af19acb3072dce121658458271aae789cdb6a93988b7`. Source is the recorded base commit plus the two hashed modified Rust files; it is not a clean committed-source build. Full manifest and build artifacts remain in the external evidence directory, `step-a-deck-engine-build-3`.
- [UAC fill stack estimate](evidence/2026-11-01-review-fixes/uac-fill-stack.log): **3104 bytes**, below 4 KiB. [Load-run estimate](evidence/2026-11-01-review-fixes/load-run-stack.log): **73008 bytes**, below 88 KiB. Existing partition/frame guards pass; indirect calls and target timing still require physical qualification.

Step A status: **HOST_TESTED / BUILD_PACKAGE_PASS / NOT_FLASHED**. Other review findings remain open. No new CI run, commit, push or merge was performed for this batch.

## B / R1: transactional source replacement

Added `deck_transport.rs` with bounded ownership-transfer slots, typed
MediaLease/LoadPermit requests and terminal results. The USB owner reserves a
real shared DeckLoadGate; the SD producer calls begin_replacement before
closing the old source. PLAY/jog claims cancel a reservation or reject active
replacement. The permit survives producer completion until engine commit.
Late results cannot unload another source. Display track identity comes from
the confirmed engine publication, not a speculative LOAD button assignment.

Tests drive both decks through the real producer, canceled reservation,
replacement-time PLAY rejection, late failure and permit reuse. Initial B
test run: 57 PASS. These totals are historical intermediate counts.

## C / R2: identified seek completion

Seek requests carry the full loaded-source request and their own request ID.
The engine installs pending state before publishing. Ready/EOF/failure/
cancellation receipts are consumed on the USB control tick, even without
render activity. Stale receipts cannot finish a newer seek. Correct zero-length
EOF blocks publish a terminal epoch; failed seeks clear old ring data, stop
transport and refuse PLAY until a successful seek or a new load repairs the
source. Cached render blocks and resamplers are invalidated on source revision.

Tests cover exact/beyond WAV EOF, return from EOF, fast completion, two requests,
foreign-source cancellation and read failure on both decks, alongside existing
MP3/FLAC and CUE scenarios. C intermediate run: 58 PASS.

## D / R4: media and controller lifecycle

Added a revocation fence for this backend's one non-remountable SD owner.
The existing bridge lease activates it; invalidation and every exit from the
load task revoke it. Engine cleanup releases pending permits and clears loaded,
playing, seek, cue and published track state. Cached render output and the
single-deck output are silenced at the next fill. Already queued USB transfers
may drain; no end-to-end mute latency is claimed. Reinsert still requires an
app restart. Connection deliveries now reach the deck owner and release CUE
preview on disconnect without resuming it on reconnect.

Tests include live cached PCM, media loss, late load/seek receipts and controller
CUE-preview disconnect. The fence deliberately does not implement hot remount;
that would require a fresh lifetime/publication protocol.

## E / R5: USB recovery and retirement

Extracted the actual UAC driver into `main/uac_driver.inc` and its allocation-free
policy into `uac_recovery.h`. A complete transfer pool is allocated before
startup. Fill/submit/completion/packet faults request recovery from the client
loop: halt, flush, drain callbacks, clear, refill. At most three recoveries per
connection are allowed. Partial startup, EP0 errors and recovery failures expose
an explicit state. A drain timeout holds resources/device instead of allowing
new contexts to replace live ones. Callback contexts carry connection generation.
MIDI teardown likewise refuses device close while its transfers remain busy.

The host C harness compiles the actual included driver, with fault injection for
completion/fill/submit, partial allocation, retry exhaustion, drain timeout,
EP0 loss and stale-generation callback. A second test verifies the shared policy.
Both C tests passed on WSL. Windows tooling reports these as skipped because
its host C compiler is unavailable; that skip is not their PASS evidence.

## F / R6: incremental production and scheduling

Production loads/seeks now begin a job and continue through bounded polls.
Header/RIFF parsing performs one metadata read per turn, including arbitrarily
long valid chunk chains. Decoder polls use 16 actions and a soft 1000 us clock
budget; one blocking filesystem/driver call can exceed that soft budget.
Budget exhaustion preserves the decoder as Pending, not a failure.

The scheduler prioritizes playing decks by buffered time adjusted for sample
rate and pitch. Nonplaying work alternates. Detail scans are explicitly deferred
during playback, with a pending message, then serve the latest selection after
pause. This is the plan's conservative interim policy, not concurrent incremental
ANLZ detail import. Producer maximum-turn-gap telemetry is reported from the SD
task. Final revision moves UAC periodic report formatting to a dedicated priority-1
task with a single overwritten queue snapshot, so neither the callback nor the
USB client loop formats that periodic telemetry. Queue replacement/unavailable
logging increments a drop counter. Queue/task creation failure disables logging
instead of blocking audio. Added per-deck buffered-time low-water and maximum
prime-duration counters (`u32::MAX` low-water means no active sample yet).
Timing, low-water margins and logging effects still need the exact-image target
campaign. The real C driver harness checks the queue overwrite/drop behavior;
virtual-time Rust tests check the new timing counters.

Virtual 500 us/read tests use real codec fixtures with a large ID3 header and a
playing peer, plus a 300-JUNK-chunk WAV. Tests check continuation/read counts,
no playing-source starvation and rate/pitch-aware ordering. The decoder may skip
ID3 padding by offset; the test does not require inefficient linear padding reads.
The generic unsafe byte-copy helper now has an explicit caller safety contract
and only audited concrete inline-storage call sites.

## G / R7 and I preparation

README, handoff and status now have one dated snapshot with separate native,
legacy and shared-host evidence. Historical reports and raw logs were retained.
Native minimum, original APTA exclusion, output gains and future DAC plan stay
intact. The [physical campaign](2026-11-01-review-fix-device-campaign.md) is
prepared with entry gates, scenario matrix, provisional budgets, telemetry,
recovery and evidence requirements. Every physical case is NOT_RUN for the
repair candidate.

## H final gates — PASS

Final source changes are frozen while fresh packages are built. Intermediate
packages which failed a lint/stack limit or detected source changes are not
qualified results. In particular, clearing a whole RenderOutputBlock by value
made the UAC fill stack estimate exceed 4 KiB; clearing arrays in place repairs
that without changing the limit. New packages must independently pass it.

The first final CodeRabbit attempt was stopped by SIGHUP after 201 s and discarded
its results. Its retry completed with one minor issue: pre-existing README
typography and a comment missing whitespace. Those have been corrected in the
final source revision, which also completes the low-priority logger and timing
counters. Packages from the earlier frozen snapshot are superseded, even when
their build passed. A new complete build/test/review pass uses the
`releasecheck-*` evidence paths. Final outcomes are recorded below only after
completion.

Final CodeRabbit outcome: **0 issues**, successful completion and exit code 0,
79 reviewed files including all new Rust/C recovery and queue modules. See the
[structured review result](evidence/2026-11-01-review-fixes/releasecheck-review-result.json)
and [raw review log](evidence/2026-11-01-review-fixes/releasecheck-coderabbit.ndjson).
The reviewed source map is
`708d379fb01e46d9ec0ffd53c852e21b55d427f8cea9998a6f1559498837ddbc`.
The [frozen source archive record](evidence/2026-11-01-review-fixes/final-source-result.json)
preserves 269 files, including untracked additions. It is source evidence for
the dirty checkout, not a published commit or physical acceptance.

Final tests: [structured summary](evidence/2026-11-01-review-fixes/final-test-result.json)
and [complete commands/output](evidence/2026-11-01-review-fixes/releasecheck-tests.log)
record exit code 0. Host features: media-load **39**, controller-audio **2**,
deck-play **56**, deck-mix **57**, deck-engine **63** tests PASS; host Clippy PASS
for all five. All eight target feature combinations passed release Clippy and
build. The six selected shared crates passed **338** tests. Windows tooling
passed 26 tests and skipped the two host-C cases; those two cases passed on WSL,
including the actual driver fault paths and new logging queue behavior. No
whole-workspace/native-board/device PASS is inferred from these checks.

All **17/17** fresh IDF packages passed. The
[verified package inventory](evidence/2026-11-01-review-fixes/final-packages.json)
records binary/manifest/source hashes, stack estimates and artifact verification;
the full per-variant manifests are retained alongside it. Every package uses
the same 269-file source map as the final reviewed and tested snapshot. All
manifest artifact hashes and all current source hashes were rechecked.
In total, **134 artifact hashes** were verified. Final documentation validation
passed **309 links**, and `git diff --check` passed. No files under
`firmware-rust` or `.github` were changed by this implementation batch.

| Candidate | App bytes | App SHA256 | UAC fill / load stack estimates |
|---|---:|---|---|
| deck-play | 774208 | `4dc44aed9633e980fe4976cd433fb9b813932d6aa87da68dcd05038c67a0b4c6` | N/A / 73088 B |
| deck-mix | 795312 | `6ca97d5c6b557dffe84286489aee9b15fc11c4487718f83deaa4570a5cb96237` | 3120 / 73104 B |
| deck-engine | 816640 | `0d4d07fa79d2eef299575304ed11258d1d071d8bdace84f35d793b176d551d85` | 3120 / 73616 B |

Existing 4 KiB UAC, 88 KiB load-chain, individual-frame and partition limits
pass without being increased. These are static estimates, not target timing
or stack-watermark qualification. The source archive is
`D:/BoardEvidence/JC1060-20261030-COM3/review-fixes-20261101/final-repair-sources.zip`,
SHA256 `44d823aa573ffdfd6cd66bc450163a826c092cf843d9bcacecc2513a2d2ee6d4`.
Full packages remain in the `releasecheck-build-*` subdirectories there.

All review repairs R1–R7 are implemented at the offline layer. The remaining
limits are explicit: details defer while playing, SD reinsertion needs app
restart, synchronous filesystem/driver latency is not bounded by the soft
decode budget, and retry/queue/timing policies need physical acceptance.

At the offline checkpoint, no commit/push/merge had been made. HEAD was the recorded base on local repair
branch `ogranak/jc1060-review-fixes`, with source and documentation changes
reviewable in the working tree. The manifests therefore pin base commit plus
dirty-source hashes; do not present them as clean release builds. At that checkpoint the branch had no upstream. These immutable build manifests
remain records of the dirty build-time snapshot; the corresponding local
host/target/build checks passed. Subsequent publication is tracked below.

Remote state was refreshed read-only: [PR snapshot](evidence/2026-11-01-review-fixes/remote-pr-state.json).
PRs #71/#72 remain open on their original heads, with #72 stacked on #71. No
new remote commit, check run, comment, PR update or merge was performed.

## Continuation

Offline H is complete. Next is publication if requested, or the prepared
device campaign after reconciling current unit/evidence and recovery ancestry.
Any source repair invalidates packages from the preceding snapshot. No physical
case has been executed for this candidate; every case in campaign I remains
NOT_RUN until an owner-assisted session supplies the corresponding evidence.

Physical state was not read. Additional `s08`/`s09` device directories exist in the external evidence location beyond the repository's reviewed checkpoint; they were not created by this implementation. Reconcile them before any later device operation instead of assuming the historical deck-play image is still running.


## Publication continuation

The owner subsequently authorized commit, push and GitHub CI. Publish the
reviewed source and its documentation on `ogranak/jc1060-review-fixes`, as a
separate PR based on `ogranak/jc1060-deck-engine` (#72). The dependency chain is
#71 -> #72 -> the repair PR. No merge or device operation is part of this
publication step.

The [remote repair branch](https://github.com/dvucinozd/Ruzina/tree/ogranak/jc1060-review-fixes)
and [repository Actions](https://github.com/dvucinozd/Ruzina/actions) identify the
published revision and current CI results. The PR description and final
publication handoff record its exact SHA/run outcomes. Historical offline
manifests and archive hashes above are retained without rewriting build-time
commit/dirty metadata. Git normalizes text line endings; CI validates the actual
committed tree independently of Windows working-tree byte hashes.

## Subsequent owner-assisted device campaign

After publication and 19/19 successful CI jobs on `b454092`, the owner continued
with the separate physical session. Read the [device results](2026-11-01-review-fix-device-results.md)
for exact identity, failed I01 timing criterion, recovery and verified normal
s07 playback. The earlier NOT_FLASHED/NOT_RUN entries above describe the offline
checkpoint only. The candidate was later flashed and removed; it remains
unqualified. All current production source hashes still match the reviewed
269-file source set. The [timing follow-up](2026-11-01-uac-timing-follow-up.md)
is the next development action.
