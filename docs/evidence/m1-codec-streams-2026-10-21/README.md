# Shared codec source evidence

Captured on 2026-10-21 from `ogranak/audio-codec-streams`, based on main
`ba5e76c029b82af9c07ca4909b9a384a37d152b7` (PR #15).
See [the gate](../../M1_RUST_CODEC_STREAM_GATE_2026-10-21.md) for limits.

- `manifest.json`: LF-normalized source/document/lock hashes, raw binary fixture
  hashes, retained log hashes, counts and preservation checks.
- `integration.json`: PR #15 reviewed/merged tree equality, review metadata and
  hosted CI account restriction annotations.
- `codec-focused.log`: runtime tests including all 20 codec scenarios.
- `host-fmt.log`, `host-clippy.log`, `host-tests.log`, `desktop-check.log`: full
  host gate with 659 tests and warnings-denied workspace Clippy.
- `composed-clippy.log`, `composed-failures.log`, `runtime-owners.log`: explicit
  runtime-owner Clippy, 11 failure-policy and nine owner-health cases.
- `ui-owner-replay.log`: both host geometries and 38 unchanged pixel goldens.
- `p4-matrix-summary.log`, `p4-01.log` through `p4-33.log`, `board-guards.log`:
  all configured P4 checks/links and 24 expected invalid feature rejections.
- `pr15-coderabbit.jsonl`, `codec-coderabbit-complete.jsonl`, `codec-coderabbit-final.jsonl`: isolated code review transcripts;
  the manifest records review scope/results for the captured source version.
- `codec-layout.log` and `codec-stack-audit.json`: full legacy audio/health link, ELF section sizes and compiler prologue checks for source/provisioning functions. These are not physical stack or timing measurements.

Focused suites overlap the workspace total. Physical board/audio/timing tests
were not run. Both lockfiles intentionally add the pinned codec dependencies;
board defaults, 18 primary KiCad edits and the unmerged APTA branch are preserved.
