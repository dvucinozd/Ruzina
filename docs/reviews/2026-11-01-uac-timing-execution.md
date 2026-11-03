# UAC phase timing: execution record

Date: 2026-11-01. Implements steps 1–2 and the phase-diagnostic preparation of
the [follow-up plan](2026-11-01-uac-timing-follow-up.md). Status at source
publication: HOST_VALIDATED / CLEAN_BUILD_AND_DEVICE_MEASUREMENT_PENDING.
The later clean-build checkpoint below supersedes that initial publication status.

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
  cumulative/reconnect semantics. The plugin was initially unavailable; after
  it became available, the committed 16-file diff passed CodeRabbit with
  **0 issues**, exit 0. The first default all-files run failed `too_many_files`
  (475 versus 150); its log remains separate from the passing committed run.

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

## Clean published diagnostic checkpoint

Source commit `5c579e6e287362af36924f30919d6d0ea911ad59` is pushed on
`ogranak/jc1060-uac-fill-timing`, [PR #74](https://github.com/dvucinozd/Ruzina/pull/74),
stacked on #73. Local and remote SHA were checked equal. Both workflows passed:
[host foundation #529](https://github.com/dvucinozd/Ruzina/actions/runs/41337405604)
and [legacy IDF #52](https://github.com/dvucinozd/Ruzina/actions/runs/41337405803),
19/19 jobs including all 17 IDF variants. No merge was performed.

Four local packages were freshly built from that clean published tree. Each
manifest records that exact commit and empty dirty status. All 270 source hashes
and all 32 artifact hashes were checked against their files.

| Variant | Bytes | App SHA-256 |
|---|---:|---|
| deck-audio | 510512 | `ac4508efd988c94f261b772252bc6e9eb9d19ada2366658f56c681d938717627` |
| deck-play | 774208 | `5ab984c1fb48392c37077693c4412255d27fa8c9828627cc7015865e6532a621` |
| deck-mix | 796016 | `dcbaa4648448a07938253f174016cc0550ddfd2aff8c4f26bc97363fdf0956eb` |
| deck-engine | 817344 | `d4a577493512df5988927ac4dc765e0e796861f2ff3c9e0e1dbad55455d10902` |

The deck-engine direct-call stack estimates remain UAC fill 3120 bytes and
load 73616 bytes. They pass existing limits and exclude indirect calls; they
are not runtime high-water measurements.

All 27 current predecessor plans were replayed and checked for payload/restore
hashes and preserved tails. All four factory backup copies passed again. The
expected full current restored-s07 state is SHA-256
`c11aa1f7a8ff8121888f9d7a70d247a447f381bb2d07b63ebd8cf7f69633f76f`.
This full state differs from the original historical s07 full flash because
larger later images left preserved bytes outside its written range.

- [Diagnostic plan](../boards/readiness/jc1060-uac-timing-write-recovery-plan-20261101.json):
  `bfef28135fb73f522893d58209cbda40d55e21f3b092efa620b383125dba3335`.
- [Ancestry-aware s07 return plan](../boards/readiness/jc1060-uac-timing-return-s07-plan-20261101.json):
  `4312e199ea36a2ef4262f3b5999397be7d9a4b5c57c6e4be3fc7a04b0b040a84`.
- [Registration](evidence/2026-11-01-uac-timing/diagnostic-registration.json),
  package manifests, CI metadata and raw review logs are retained together.
- Verified binary archive and secondary-copy hashes are in the
  [archive index](evidence/2026-11-01-uac-timing/clean-build-diagnostic-archive.json).

Status: **BUILD_ONLY / NOT_FLASHED / PHASE_MEASUREMENT_NOT_RUN** at this
checkpoint. Run only a diagnostic cold boot/idle, preserving the aggregate
>=6000 us rejection bound; do not start playback or change gain. Fresh physical
identity/topology, manual ROM entry, live security/RDID and complete current
flash equality are still required. After measurement, return to s07 through
the new plan and verify normal sound and controls before ending an unsuccessful
experimental session. A later run must record its own outcomes separately.

## Subsequent phase measurement

The owner-assisted [device run](2026-11-01-uac-timing-device-results.md) later
booted this exact image and localized the measured 10373 us maximum to callback
refill (three >=6000 us samples among 6000 callbacks). Initial prefill maximum
was 1844 us; recovery had no samples. The registered aggregate criterion failed.
The above NOT_FLASHED/NOT_RUN status is the historical clean-build checkpoint,
not the later device outcome. No source optimization has been made.

Registration head `1265b07` also passed 19/19 CI jobs:
[host #530](https://github.com/dvucinozd/Ruzina/actions/runs/41353947606) and
[legacy #53](https://github.com/dvucinozd/Ruzina/actions/runs/41353947651).
The s07 recovery exit is complete: full predecessor match, written-sector
readback, cold boot and operator-confirmed normal playback with zero captured
underruns/USB errors. All monitors are closed. The next implementation is the
[bounded callback profiling plan](2026-11-01-uac-callback-profiling-plan.md).
