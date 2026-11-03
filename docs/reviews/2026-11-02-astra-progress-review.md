# Astra review of legacy development and cause profiling

Date: 2026-11-02. Reviewed HEAD: `e4aa31d83d73f9d3193e828374d09c4d6acbfc56`.
Scope: current legacy deck-mix/engine, repair batch and successive phase,
callback and calibrated-cause diagnostics; current remote PR stack and evidence.
This is a review, not firmware qualification or implementation of fixes.
No COM3 access, flash, gain change or native minimum revision change occurred.

## Assessment

The overall direction is sound: retain shared Rust product logic, use the
pinned IDF backend for the observed rev103 unit, fix lifetime/ownership bugs
before adding features, and measure on the target before optimizing. The
measurement sequence has progressively narrowed the problem, rather than
demonstrated a timing improvement. Two newly reproduced code defects below
must be added to the next implementation plan. In particular, the current
two-deck implementation is not ready for acceptance merely because CI passes.

## R1 — P1: one deck's seek disrupts the other deck's output

Location: [Mix::tick](../../firmware-idf/legacy-jc1060/rust-core/src/deck_mix.rs),
lines 200–208, and the global engine revision updates in
[Engine](../../firmware-idf/legacy-jc1060/rust-core/src/deck_engine.rs).
Introduced by the repair batch `b454092`, still present at reviewed HEAD.

Every accepted seek/load-result changes one shared revision. The next tick
sets the shared output position to the end of the 128-frame block and resets
both render graphs. The readers already consumed the source frames needed for
that block. Dropping its unconsumed suffix therefore skips the unaffected
deck's output, while resetting its resampler/EQ/filter state adds a transient.
This also occurs when the changed deck is paused and contributes no sound.

Reproduction uses an isolated copy of the actual legacy Rust crate and the
unchanged shared crates. Both decks are loaded with ramp PCM; only deck 1
plays. After ten 48-frame packets (480 output frames), a seek is requested on
paused deck 2. Compare the next 192 MAIN frames against an otherwise identical
run without that seek. All 192 differ. The reference starts at sample 120;
the peer-seek case starts with 0 and then 128. The shared block had 32 frames
remaining; the affected code discards them. The regression assertion fails
on the reviewed source. This is host PCM evidence, not a new listening test.
Repeating the reference case after the seek case produces identical reference
PCM, excluding leftover shared ring state as the cause of the difference.

Fix: track affected decks separately and preserve the other deck's resampler,
filter and output continuity. Merely changing `reset_deck(0/1)` to reset one
deck is insufficient while the shared block suffix is still discarded. Choose
an explicit contract: apply ordinary source changes at a render-block boundary
with bounded latency, or retain per-deck cached contributions and recompose the
unconsumed output. Media revocation must retain its documented silence/fencing
requirements. Do not rewind the other deck's already committed ring tail.

Acceptance: reproduce first, then require identical unaffected MAIN/PFL PCM
for A-playing/B-seeking and the reverse, LOAD completion, seek failure and
cancelled/stale receipts; exercise partial block offsets, 44.1/48 kHz, pitch,
and enabled EQ/filter. Preserve existing revocation and stale-source tests.

## R2 — P2: the fail-closed analyzer accepts impossible record values

Location: [analyze_uac_cause.py](../../firmware-idf/legacy-jc1060/tools/analyze_uac_cause.py),
lines 62–92. `words()` validates calibration rows, but coherent record/stage
and cause rows mostly use unrestricted Python `int()` and upper-bound checks.

Three direct reproductions using the existing valid test fixture:

- `irq_dispatch_us=-1`: `irq_dispatch_usable=true`, with impossible bounds
  `[0, -1]`.
- `graph=101`, `pack=-1`, Rust total 100: `partition_verified=true` and both
  attribution flags remain true because the signed values still sum to 100.
- Missing `irq_entries`: IRQ attribution remains usable.

Fix: require complete schemas and unsigned u32 bounds for every required
record/cause/stage field before any validity flag becomes true. Validate
durations, identities, entry/nesting counts and consistent interval bounds;
on missing/invalid fields leave attribution false. Add negative, oversized,
missing and malformed-field regression cases for these rows, not just the
calibration rows. Retain raw bytes and hashes.

This does not invalidate the recorded device observations: both real logs were
reanalyzed and match the checked-in analyses exactly; their retained values
are nonnegative and their partitions/histogram denominators are valid.

## What the earlier work actually achieved

- The accepted s07 image demonstrated deck-1 FLAC/MP3/WAV playback through
  FLX4 headphones. The latest recorded restore includes cold boot, readback,
  owner confirmation of sound/Pause/CUE and 580032 output frames (12.084 s).
- The newer source adds two-deck mixer/transport integration and substantial
  offline repairs: coherent ring descriptors; load permits; identified seek
  results; media revocation; USB fault recovery; incremental producer work;
  lower-priority logging; and targeted host/actual-C tests.
- The physical repair campaign stopped at I01 timing. Successive diagnostics
  separated initial prefill from callbacks, then stages and scheduler activity,
  and finally calibrated scheduler/IRQ counters. I02–I13 remain unexecuted.
- The latest callback outlier is 14812 us, with 1593–1653 us of covered IRQ
  dispatch and zero observed scheduler switch delta. Two callbacks exceeded
  the registered 6000 us rejection bound. The later steady interval adds
  9000 callbacks without new crossings. This is idle evidence only.

The 6000 us bound remains a failed campaign criterion. With a transfer pool,
exceeding that bound alone is not proof of an audible underrun, and the logs
report zero USB errors/idle underruns. It nevertheless prevents accepting the
candidate under the agreed criterion. The 10373/13087/14812 us maxima from
different instrumentation/boots are not a performance trend.

## Recommended order

1. Fix R1 offline with the real mixer/engine regression and fix R2's parser
   validation. Keep these changes reviewable and distinguish them from the
   unchanged historical timing evidence.
   A renderer fix creates a new baseline: both later comparison variants must
   contain the same functional fixes, and need fresh exact-image timing.
2. Continue the existing [bounded cache/compute audit](2026-11-02-uac-cache-compute-follow-up.md).
   Identify actual code/data/stack/DMA placement and supported counters first.
   Cache is a hypothesis, not an established root cause. Quantify observer
   overhead; compare an unchanged instrumented control and one changed variant
   over repeated equivalent cold boots. Add no priorities/DSP/gain changes to
   that experiment. Bound the number of diagnostic rounds with an explicit
   decision at the end of each one.
3. After a candidate meets its timing gate, run the two-deck MAIN/PFL,
   LOAD/PLAY/CUE/seek/EOF and source-rate cases, then SD loss, USB reconnect,
   recovery and extended exact-image playback. A timing pass does not cover R1.
4. Only then expand product integration: source cues/analysis, missing
   performance controls, product UI/settings, C6/networking and OTA. External
   DAC/headphone modules and JC8012 retain their separate hardware gates;
   APTA remains excluded.

The PR stack is six open unmerged PRs (#71–#76). Preserve dependency order and
review the final combined code when consolidating it; do not merge the stack
solely because the top PR is green. Handoff/status documents should eventually
keep one current next-action section, linking older checkpoints as history.

## Verification in this review

- Remote fetch and `ls-remote`: branch HEAD equals remote `e4aa31d`; main remains
  `bd52402160af27a3e234f7647d703a847644817b`. PRs #71–#76 are open/unmerged.
- GitHub Actions on reviewed HEAD: host #539 and legacy #62 completed
  successfully; 20/20 jobs across both runs.
- Fresh Rust engine/profile suite: 68/68 PASS. Fresh pinned Windows Python:
  33 PASS plus six explicit C skips. Those six actual C tests PASS in WSL.
  The initial full WSL Python run also had two environment errors because its
  Python lacks esptool; these are not source failures or a full-suite PASS.
- Documentation checker: 394 links PASS before this review, 398 with it.
- Recomputed cold/steady analysis equals both saved JSON files. Cold has
  three retained records/seven complete distributions; steady has three
  repeated retained records/ten complete distributions. Both restoration log
  hashes match their recorded values.
- Both clean normal/profile packages: all 282 source hashes per package and
  all eight artifact hashes per package match actual local files.
- New peer-seek regression: FAIL as described in R1; three malformed-analyzer
  inputs accepted as described in R2. Production source is unchanged.

External reproduction/evidence root:
`D:/BoardEvidence/JC1060-20261030-COM3/astra-review-20261102/`.
It contains `reproduce.py`, isolated `rust-core`, `peer-seek.log`,
`peer-seek-repeat-control.log`,
`analyzer-reproduction.json`, `verify_evidence.py` and
`evidence-verification.json`. No personal memory was changed.

## Independent CodeRabbit result

The first two attempts were rejected with `rate_limit`. After the allowance
reset, the final review completed on committed `62717ce..e4aa31d` (68 files).
CodeRabbit raised 0 issues. Raw output is `coderabbit-final.raw.log` in the
external evidence root. This is the last diagnostic PR's scope, not a full
review of the preceding repair stack. R1 and R2 above are this audit's own
source/test findings: R1 is outside that diff; CodeRabbit did not identify R2.
No claim of a defect-free product follows from that result.
