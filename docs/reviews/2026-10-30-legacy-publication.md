# Legacy JC1060 bring-up publication checkpoint

Date:2026-10-30. User-authorized scope: review, persist and push the current
bring-up batch, check branch CI and prepare a merge-ready PR. Product media/load
implementation is a later task. This publication does not access COM3, reset or
flash the board, modify native minimum302, merge APTA or publish a release.

Base: `4fe18e986e0a97d7f5eec0e68303f96481928af4`, freshly fetched origin/main.
Branch: `ogranak/jc1060-c01-c02`. No newer main commits or existing open PR for
this branch were found at the start. The primary checkout stays on main and
its unrelated files are preserved.

## Included scope and evidence

- Isolated pinned IDF6.1 / no_std Rust static-library shell for this P4v1.3
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

## Publication gates

CodeRabbit review, final local checks, selective commit/push and branch CI are
in progress. Outcomes and immutable source/commit identities are added after
execution. A pending or infrastructure-blocked remote job is not a test PASS.
The PR will remain unmerged in this step; merge readiness is conditional on
review and actual required checks, not a successful push alone.
