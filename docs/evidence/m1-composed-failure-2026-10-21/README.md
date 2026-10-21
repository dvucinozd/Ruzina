# Local composed-failure evidence

See [the failure policy and limits](../../M1_RUST_COMPOSED_FAILURE_GATE_2026-10-21.md).
`manifest.json` pins the baseline commit, current changed source/document hashes,
lockfile hashes, retained log hashes and preservation checks. Text hashes normalize
CRLF to LF so checkout line endings do not change the result.

The source is based on main `d1013291cf189eb550ccf902689b791eef78287a`.
These are local results; no hosted run, board flash, listening acceptance,
physical fault injection or power-cut qualification is claimed.

| Evidence | Result |
| --- | --- |
| `host-fmt.log`, `host-clippy.log`, `host-tests.log`, `desktop-check.log` | Workspace formatting, warnings-denied Clippy, 601 host tests, desktop check pass. An empty formatting log is normal successful output. |
| `composed-clippy.log`, `composed-failures.log` | Explicit `runtime-owners` test Clippy and all ten new composed scenarios pass. |
| `runtime-owners.log` | Nine existing owner-health integration scenarios pass. These are included in the workspace total. |
| `ui-owner-replay.log` | Both geometries pass replay and all 38 existing UI pixel goldens remain unchanged. |
| `p4-matrix-summary.log`, `p4-01.log` through `p4-33.log` | Every current workflow check/link configuration passes. Existing feature-dependent target warnings are retained in full. |
| `board-guards.log` | All 24 expected invalid board/feature combinations are rejected. |

The ten new scenarios cover: failed seek with the other deck playing and reload;
read fault and obsolete terminal result; rendered fault after sink receipt;
terminal result from a pre-seek stream; Library failure before/after source
revocation; media detach with an unconfirmed cue write and remount; a full
transport queue released by source invalidation; failed write with a pending
Library read and subsequent real controller FIFO deliveries; absent/corrupt cue
storage with a latched failure; and same-track reload while cue writes complete.

Fault outcomes are injected at operation boundaries. ProductRuntime, transport
transitions, cue-store access policy, controller scheduling and receipt bookkeeping
are actual project code. Embassy task interleavings and hardware drivers are not
executed by these host tests. Library-specific feature wiring is covered by the
target builds rather than a simulated filesystem.

The capture also verifies that both lockfiles and managed hardware are unchanged,
all 18 pre-existing primary-checkout KiCad file hashes still match the retained
snapshot, and APTA remains unmerged at
`fd7ebd592b469b27c9f45337dced2f9b2912966f`.
