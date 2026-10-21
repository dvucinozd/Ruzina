# PCM FX execution evidence

Captured on 2026-10-21 from `ogranak/pcm-fx-render`, based on main
`3e56319d912f389168534217e697c06a2e7a7c62` (PR #13).
See [the gate](../../M1_RUST_PCM_FX_GATE_2026-10-21.md) for behavior and limits.

- `manifest.json`: LF-normalized SHA-256 for changed sources/documents and both
  unchanged lockfiles; hashes for retained evidence and preservation checks.
- `integration.json`: selected PR #13 head and merge tree equality, merge metadata
  and hosted CI account restriction annotations.
- `fx-focused.log`: DSP/deck/service tests with runtime-owner features and all
  11 new real-PCM FX scenarios.
- `host-fmt.log`, `host-clippy.log`, `host-tests.log`, `desktop-check.log`: full host
  gate, including 628 passing tests and warnings-denied workspace Clippy.
- `composed-clippy.log`, `composed-failures.log`, `runtime-owners.log`: explicit
  feature-enabled Clippy, 11 composed failure cases and nine owner-health cases.
- `ui-owner-replay.log`: both host geometries and 38 unchanged screenshot goldens.
- `p4-matrix-summary.log`, `p4-01.log` through `p4-33.log`, `board-guards.log`:
  all 33 configured P4 checks/links and 24 expected invalid feature rejections.

Focused tests overlap the workspace total. Hosted CI is not qualified by the
local runs. Physical output, listening and timing were not run. The primary
checkout's 18 existing KiCad edits, both Cargo lockfiles and the unmerged APTA
branch are preserved.
