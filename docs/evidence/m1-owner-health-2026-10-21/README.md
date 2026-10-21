# Composed owner health evidence

Scope and open gates: [owner integration gate](../../M1_RUST_OWNER_HEALTH_GATE_2026-10-21.md).

- `host-fmt.log`, `host-clippy.log`, `host-tests.log`, `desktop-check.log`:
  workspace checks, 586 passing tests and desktop compilation.
- `runtime-owners.log`: seven composed integration tests, run serially with
  printed scenario reports. They compile the actual default firmware
  `ProductRuntime` source and use real controller/media session implementations.
- Ten named `*.txt` scenario reports: **host harness observations only**, not
  device telemetry. They demonstrate actual queue backpressure/retry, stale
  product while other publishers remain current, diagnostic queue loss, media
  replacement and current-error recovery with retained historical totals.
- `p4-01.log` through `p4-33.log`, `p4-matrix-summary.log`: partition validation
  and target checks/links, including USB0-only, USB1/product-only and full
  combined `runtime-health` builds. No target was flashed or measured.
- `board-guards.log`: 24 expected feature/board compiler rejections.
- `ui-owner-replay.log`: actual Slint replay matched all 38 existing reviewed
  RGB hashes. There is no UI baseline change; the preceding package's
  [reviewed images](../m1-runtime-health-2026-10-21/README.md) remain applicable.
- `source-manifest.json`: LF-normalized SHA-256 for changed source/docs/workflows
  relative to `f1786a96fcbee17a3f0a25c142deff9f17582010`, excluding this directory.
- `verification.txt`: full external lock-record and original KiCad hash checks.

These reports are not listening, Guition display/touch, USB cancellation/timing,
CPU-watchdog, physical audio, memory-performance or installed-image acceptance.
The target's new owner snapshot API has no new live serial/UI consumer in this
package; the Guition boot report remains immutable. Hosted CI is recorded
separately from the local checks.
