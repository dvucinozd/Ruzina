# Legacy JC1060 bring-up publication checkpoint

Date: 2026-10-30. User-authorized scope: review, persist and push the current
bring-up batch, check branch CI and prepare a merge-ready PR. Product media/load
implementation is a later task. This publication does not access COM3, reset or
flash the board, modify native minimum 302, merge APTA or publish a release.

Base: `4fe18e986e0a97d7f5eec0e68303f96481928af4`, freshly fetched origin/main.
Branch: `ogranak/jc1060-c01-c02`. No newer main commits or existing open PR for
this branch were found at the start. The primary checkout stays on main and
its unrelated files are preserved.

## Included scope and evidence

- Isolated pinned IDF 6.1 / no_std Rust static-library shell for this P4 v1.3
  unit, with separate scalar/RAM, memory/FPU/EQ, display/touch, C/FatFs SD,
  Rust block bridge and shared Rust partition/exFAT fixture profiles.
- Fresh build/config/hash guards, recovery planning with complete predecessor
  replay, negative/offline tooling tests, opt-in runtime profiles and dedicated
  CI. The native HAL/entrypoint and shared product crate sources are unchanged.
- Unit-scoped JSON/text evidence, accepted package/archive identities and
  operator records. Binary firmware/dumps/private recovery payloads remain
  outside Git. Secret-pattern inventory found no credential assignments,
  access tokens or private-key blocks in the newly added text files.
- Raw evidence and immutable recovery-plan snapshots have scoped Git attributes
  preserving their exact bytes, including captured CRLF/trailing spaces. Their
  content hashes and JSON syntax are validated independently; source/docs retain
  normal whitespace checks. Publication must not trim or normalize raw evidence.
- Four primary/secondary factory backup files reverified, plus eight evidence
  archives with matching primary/secondary SHA-256 and valid ZIP CRCs. Earlier
  raw captures and uncommitted-source snapshot fields remain historical; do
  not rewrite them to claim they were built from a later commit.

The current board image remains app SHA-256
`ec6b7f0f159c4f6d77cafcb692df84da046bfd2caa0151854c1df693a3f59267`.
The [exFAT report](2026-10-30-legacy-s6-rust-exfat.md) records full-file
SHA/EOF/10-seek, restarts, open-file removal/reinsert and owner-confirmed cold
return. Initial cold init/reset-reason prefix was missed. This is a bounded
diagnostic, not the product media/load, audio/UI/USB/C6 pipeline.

## Published implementation and review

Implementation commit: `7518d83a47a68009919fe58a036ef22195d0bae3`, pushed to the
continuation branch. Its Git tree is `06187503fbabdaaffac4d21d7579548698a28a14`.
[PR #55](https://github.com/dvucinozd/Ruzina/pull/55) targets main and remains a
draft pending actual CI. The primary checkout/main and APTA refs were untouched.

CodeRabbit CLI 0.9.1 completed the scoped review in 426.993 seconds: **2 issues**,
both minor, 0 major/critical. The full-diff attempt first hit its 150-file limit.
The successful review covered 89 source/CI/docs/readiness paths in an isolated
native Git checkout with the identical full staged tree; 157 raw capture paths
were excluded from the AI diff and independently checked byte-for-byte/hash/
JSON instead. A review-only evidence baseline was never published as history.
[Scope/result](../boards/evidence/jc1060-20261030-com3/publication-coderabbit-result.json),
[raw review log](../boards/evidence/jc1060-20261030-com3/publication-coderabbit.log),
[resolution record](../boards/evidence/jc1060-20261030-com3/publication-review-resolution.json).

1. The exFAT fixture mount rejected `PartitionOffset=0` on a valid partitioned
   volume. [Microsoft exFAT section 3.1.4](https://learn.microsoft.com/en-us/windows/win32/fileio/exfat-specification#314-partitionoffset-field)
   assigns 0 the meaning that the field is ignored. A new regression failed on
   the original code; the fix accepts 0 while retaining rejection of a nonzero
   mismatch and all actual partition bounds/checksum checks. A second regression
   confirms the mismatch remains rejected. [Baseline failure](../boards/evidence/jc1060-20261030-com3/publication-partition-offset-baseline.log).
2. Missing spaces joined numeric values to prose. Human-facing Markdown was
   corrected while numeric tokens, inline code, code fences and link destinations
   were preserved. Raw evidence/plan snapshots were not reformatted.

After the fixes, 29 Rust tests, 76 shared media regressions, fmt and host/target
Clippy passed. [Check record](../boards/evidence/jc1060-20261030-com3/publication-rust-checks-result.json),
[Rust test log](../boards/evidence/jc1060-20261030-com3/publication-host-tests.log).
There was no second AI review; fixes are verified by regressions/builds and
documentation checks, not described as a new zero-issue CodeRabbit pass.
The fresh post-review IDF build passed, including no-write symbols, matching
legacy revision config/header and single-float ELF. The
[publication build index](../boards/readiness/jc1060-legacy-publication-build-20261030.json)
records its 244784-byte app SHA-256
`a08a3e45e8bbff54af969e18adf359fc5a31071154ed0fd14c56c8b1b431cb93` and
the verified primary/secondary source/artifact ZIP. This is BUILD_ONLY.
No COM3 access/reset/flash occurred. The PartitionOffset fix has NOT
been physically loaded; current hardware evidence belongs to the earlier
accepted exact image. Its archived source bytes remain immutable.

## CI infrastructure block and next action

All three workflows on implementation commit 7518d83 failed before executing a
step: host, native P4 and legacy IDF. The actual check annotation is:
"The job was not started because an Actions budget is preventing further use."
[Annotations/empty step lists](../boards/evidence/jc1060-20261030-com3/publication-ci-budget-block.json).
This is an Actions-budget infrastructure block, not code test failure or PASS.
Billing/budgets were not changed, no failed-run retry loop was started, and the
PR was not merged or marked merge-ready.

At this checkpoint the next external action was restoring Actions budget, then
running the current PR head through host, native P4 and all eight legacy IDF
profiles plus the isolated Rust host job. Only after actual success should PR #55 become ready and merge.
Local tests/builds do not substitute for a claimed successful remote CI result.
The following publication follow-up commit contains the two fixes and this
durable record; its exact final local/tracking/remote identity is verified after
push. No source-only build is a new device-qualification image.

## Actions-budget continuation

The owner subsequently confirmed an increased Actions limit. The reviewed
source/fix head is `93e401e5e901f121e0857872ae82f8aacf668deb`. Its original
three runs remained queued with zero steps and some budget-rejected jobs;
GitHub rejected cancellation with HTTP 409 (not in progress) and retry with
HTTP 403 (already running). No runner/test failure is inferred from those
responses. This documentation-only continuation starts fresh PR CI.

The required gate remains actual host, native P4 and eight-profile legacy IDF
success on the current head, including the Rust host job. Results and final
head identity will be recorded in PR #55. No hardware access or new physical
qualification is part of this continuation.

## Merge

The continuation head `00294d18cd59ba5afcd6aaa4b9dcba8465c895b2` executed all
required jobs: Rust host foundation (run 41122323906), Rust ESP32-P4 compile
gate (run 41122323864) and legacy IDF build (run 41122323595: the eight
profiles plus media-bridge-host). All passed. On the owner's instruction PR #55
was marked ready and merged with that head pinned, as merge commit
`46728b2a3779d125b50aadae908b293adaafa25a`. The merge adds no hardware access
or physical qualification; the board image is unchanged.
