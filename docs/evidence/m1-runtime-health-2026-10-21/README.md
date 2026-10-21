# Runtime health publication evidence

See the [scope, publication contract and open gates](../../M1_RUST_RUNTIME_HEALTH_GATE_2026-10-21.md).

- `source-manifest.json`: LF-normalized SHA-256 of changed source/docs relative
  to base `4252d6d48eaaec33eb5b0f03403591cb579387d5`, excluding this evidence directory.
- `host-*.log`, `desktop-check.log`: workspace formatting, warnings-denied clippy,
  577 passing tests and desktop compilation. Adapter coverage includes actual
  render starvation, scheduler overflow, muted/idle/active sink statistics and
  PCM reset identity; publication tests cover deadlines, rejection and exhaustion.
- `p4-01.log` through `p4-29.log`, `p4-matrix-summary.log`, `board-guards.log`:
  29 target checks/links and 20 expected board-feature rejections. These are
  compile/link results; no board was flashed or measured.
- `ui-*.log`: actual compiled Slint software renderer replay on both geometries,
  followed by an independent run matching all 38 reviewed RGB hashes.
- `*.png`: the reviewed captures, including stale Idle and heartbeat recovery.
  All 24 earlier non-Health scene bodies match the preceding package below navigation.
- `*-health-*.txt`: V2 reports from the corresponding replay scenes. A stale
  report retains the original sample timestamp and reports RuntimeStale.
  The recovered Idle report has a new sequence and age zero.
- `service-cli-jc8012.txt`: a fresh separate host session, not a live device query.
- `verification.txt`: lock external-record and primary KiCad hash preservation.

No listening, device watchdog, Guition display/touch, physical output, memory
performance or installed-image acceptance is claimed. Device boot diagnostics
remain an immutable snapshot, not a runtime heartbeat. Hosted CI is separate
from the local evidence above.
