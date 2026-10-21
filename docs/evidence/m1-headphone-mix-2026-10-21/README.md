# Controller headphone mix evidence

Captured on 2026-10-21 from `ogranak/headphone-cue-master`, based on main
`cb69dbd28a36462b3e9a609478218a5efce0e379` (PR #14).
See [the gate](../../M1_RUST_HEADPHONE_MIX_GATE_2026-10-21.md) for behavior and limits.

- `manifest.json`: LF-normalized SHA-256 of changed source/documents, unchanged
  lockfiles, logs and preservation checks.
- `integration.json`: selected PR #14 head and merge-tree equality, hosted CI
  billing annotations, and the inspected C monitor source/fixture hashes.
- `headphone-focused.log`: focused DSP/deck/service/runtime tests including all
  11 new composed headphone scenarios and existing FX/loop regression tests.
- `host-fmt.log`, `host-clippy.log`, `host-tests.log`, `desktop-check.log`: full
  host gate with 639 passing tests and warnings-denied workspace Clippy.
- `composed-clippy.log`, `composed-failures.log`, `runtime-owners.log`: explicit
  runtime-owner Clippy, 11 composed failure cases and nine owner-health cases.
- `ui-owner-replay.log`: both host UI geometries and 38 unchanged pixel goldens.
- `p4-matrix-summary.log`, `p4-01.log` through `p4-33.log`, `board-guards.log`:
  33 P4 checks/links and 24 expected invalid feature rejections.

Focused tests overlap the workspace count. Physical output, headphone listening,
timing and the donor C suite were not run. Local results do not qualify hosted CI.
The primary checkout's 18 pre-existing KiCad edits, both lockfiles and the separate
APTA branch are preserved.
