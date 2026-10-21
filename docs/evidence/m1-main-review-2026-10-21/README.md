# Main integration review evidence

See the [review scope and corrections](../../M1_RUST_MAIN_REVIEW_GATE_2026-10-21.md).

- `stack-history-audit.json`: nine original source manifests checked at their
  exact commits, 191 matching file hashes, preserved external lock records and
  original primary-checkout KiCad hashes.
- `stack-github-review.json`: original PR heads, inline findings and all 14
  original jobs' verified payment/spending-limit not-started annotations.
- `review-regressions-before.log`: **expected failure on original session
  source**, with the new tests demonstrating both cue loss and 41 EOF events for
  a single track end. This is not a failure of the final corrected source.
- `host-fmt.log`, `host-clippy.log`, `host-tests.log`, `desktop-check.log`:
  final corrected host checks and 591 passing tests.
- `runtime-owners.log`: nine passing composed-owner integration cases, including
  actual ProductRuntime shared cues and pre-Ready media cleanup/recovery.
- `p4-01.log` through `p4-33.log`, `p4-matrix-summary.log`, `board-guards.log`:
  all 33 target checks/links and 24 expected configuration rejections.
- `ui-owner-replay.log`: all 38 existing reviewed pixel hashes match. No UI
  baseline or screenshot change was needed.
- `source-manifest.json`: LF-normalized SHA-256 of review correction source/docs,
  relative to `e1891a93e332d7d0b9e5d9d5eb7346c8a622ea60`, excluding this directory.
- `verification.txt`: final scope and preservation checks.

All device/physical acceptance gates remain open. Regression/owner reports in
these logs are host observations. Actual merge identities and subsequent CI
runs are recorded on the linked GitHub PRs; hosted CI is separate from local PASS.
