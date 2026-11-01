# UAC phase timing: execution record

Date: 2026-11-01. Implements steps 1–2 and the phase-diagnostic preparation of
the [follow-up plan](2026-11-01-uac-timing-follow-up.md). Status at source
publication: HOST_VALIDATED / CLEAN_BUILD_AND_DEVICE_MEASUREMENT_PENDING.

The board remains on accepted s07 app `95d45203...`; no device access has taken
place in this instrumentation batch. Native minimum revision 302, accepted gain,
render/DSP code, source lifecycle, USB recovery policy and original campaign
results are unchanged. No timing improvement or target cause is claimed.

## Changes

`main/uac_driver.inc` passes the phase explicitly from initial stream startup,
recovery startup and completion callback. Every measured Rust fill attempt,
including a failed fill, updates one phase and the original all-call maximum.
Durations remain whole-call wall time. The timer is not reset or moved to
exclude initialization.

`main/uac_timing.h` stores fixed-size, boot-cumulative phase counts, first/max
durations, failure counts and threshold crossings. Counts saturate at
`UINT32_MAX`; wall times larger than that clamp instead of wrapping. The first
attempt in each connection has separate generation-tagged metadata. Context
flags say whether an audio submission already succeeded in that connection;
they do not mean a transfer is currently in flight. EP0 is excluded.

The existing latest-snapshot queue carries a value copy to the priority-1
logger. Three phase lines and one first-connection line are formatted only
there. Recovery retries, halt/flush/drain/clear, resource hold and stale callback
fencing keep their existing behavior. Allocation failure before initial fill
correctly yields a recovery-phase first sample if a retry succeeds.

## Verification before publication

- Actual driver host harness, both uninstrumented and full deck-engine timing
  configurations: PASS. The controlled clock separates a 12260 us initial fill
  from 6000/6001 us callbacks, records failed fills, verifies recovery and
  reconnect, snapshot contents, stale callbacks, before-first-submit context,
  exact threshold boundaries, long duration clamping and counter saturation.
- Existing policy harness: PASS. Existing resource-hold, bounded recovery,
  allocation/submit/fill failure and callback-generation scenarios also run in
  the timing configuration.
- Five Rust host profiles: 39 / 2 / 56 / 57 / 63 tests PASS; format and Clippy
  with warnings denied PASS. Rust production files did not change.
- Windows tooling with pinned Python/esptool and CMake: 26 PASS, 3 C-host skips.
  All three skipped C cases PASS separately under WSL with `cc -Werror`.
- Initial attempts without the login-shell Cargo path, esptool or CMake failed
  as environment errors; logs are retained and are not represented as passes.
- Manual diff review covered explicit phase call sites, ownership, unchanged
  recovery behavior, no callback allocation/formatting, snapshot bounds and
  cumulative/reconnect semantics. No new CodeRabbit result is claimed: the
  previously used plugin/skill is unavailable in this session.

Primary logs: `D:/BoardEvidence/JC1060-20261030-COM3/uac-timing-20261101/`.
`offline-checks.log` is the passing WSL run; `windows-tools.log` is the passing
pinned Windows run. Initial environment logs are named explicitly.

## Required continuation

1. Publish the reviewed source and verify the remote SHA. Fresh-build the four
   UAC profiles from the clean published tree, verify package/source hashes and
   stack limits; check the full CI matrix independently.
2. Prepare the diagnostic plan from the full 27-plan current history, including
   the earlier repair candidate and the return-to-s07 plan. Verify all backups
   and prepare another ancestry-aware s07 fallback before a device operation.
3. Register this as a phase-measurement run, not qualification. Preserve the
   existing >=6000 us aggregate rejection result and capture exact-image cold
   boot/idle. No playback or I02–I13 until the timing cause and acceptance
   decision are documented. Do not add sub-stage profiling unless phase data
   warrants it.
4. If the diagnostic remains experimental or exceeds the registered bound,
   restore s07 and verify normal operation before stopping. Keep operator
   acceptance separate from host/build and phase telemetry.
