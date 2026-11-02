# Both-deck resource readiness before UAC startup

Date: 2026-11-02. Branch `ogranak/jc1060-resource-readiness`, based on
`8449b95bd869ec34b0be0f3167d92f142d2deaba` and the
[resource lifecycle decision](2026-11-02-usb-service-and-resource-decision.md).
Status: IMPLEMENTED / OFFLINE_VALIDATION_IN_PROGRESS / NOT_FLASHED.

The SD owner now prepares both producers and both 65536-frame PCM rings before
mount/catalog preflight. Twelve fallible backing allocations reserve the
1162544-byte payload (excluding allocator overhead and producer metadata).
No giant decoder temporary is built on the stack. Unpublished partial state
drops normally; published rings retain permanent storage even if cancellation
wins just before READY. A one-boot release/acquire scalar transitions from
UNINITIALIZED to PREPARING to READY, or terminal FAILED. Repeated preparation
does not reopen READY/FAILED. Hot remount is not added.

The USB owner retains selected scalar format/interface data and polls readiness
from its existing event loop. It does not retain the borrowed configuration
descriptor, spin or wait for the SD task. Interface claim/control and product
prefill/submission are deferred until READY. Failure before EP0 completion also
prevents stream startup. Stop/disconnect clears a pending start. Enumeration,
MIDI and transport ticks keep their existing ownership and event path.

Prepared producers move explicitly into `serve_decks`; neither catalog PASS nor
first deck-2 LOAD creates another permanent resource bundle. Existing transient
SourceDecoder/PCM-block and filesystem/catalog allocations are unchanged.
READY is a memory-lifetime fact, not a media lease: existing typed admission,
revocation, stale receipts and immediate media mute still apply. Mount/import
failure and bridge invalidation make the resource gate FAILED; published
storage is never freed under a reader. While a previously active stream drains,
existing media mute/recovery policy remains in effect.

The candidate changes no gain, priorities, transfer count, DSP, packetizer
placement or native revision minimum 302. Only `deck-engine` (including its
explicit profile) uses the new gate. Lower playback/mix diagnostics retain
their former activation. Native esp-hal is still unsuitable for this rev103 unit.

## Verification

External work logs and packages live under
`D:/BoardEvidence/JC1060-20261030-COM3/resource-readiness-20261102/`.
The working-tree normal pilot passes build/package/link/stack checks. Initial
release Clippy rejected four obsolete engine initialization helpers; they are
now compiled only where used. Initial Windows tests lacked CMake on PATH and
the all-tests WSL Python lacked esptool; reruns use the pinned Windows Python
with CMake plus the actual six C cases under WSL. Those environment failures
are retained, not counted as passes.

New tests cover every backing allocation failure with actual allocator counts
(successful partial allocations equal frees), cancellation/publication races,
repeated initialization, both prepared producers' PCM against the old path for
WAV/FLAC/CBR/VBR MP3, and repeated LOAD/seek/failed LOAD on both decks with zero
allocations at least as large as the smallest permanent store. This last check
does not assert that per-track allocation or decoder reset/copy work is absent.
The real C driver harness covers repeated deferred polls, READY starting once,
terminal failure, disconnect while pending, and failure before EP0 completes;
its existing partial-pool/recovery/drain/stale-generation tests remain enabled.
Existing peer-continuity, media-revocation, transport and fixture tests remain
required. Final matrix/build/review results will be recorded after completion.

`resource_prepare` telemetry records PREPARING/READY uptime, free/largest PSRAM,
free internal RAM and the SD task stack high-water mark. Actual target readiness
duration, minimum heap/largest block and stack remain NOT_RUN. Host allocator
counts and static frame checks do not substitute for them.

## Next gate

Finish exact-parent review, clean normal/profile builds and the complete CI
matrix before registering an experiment. Packages remain BUILD_ONLY, without
an executable device-write registration. No COM3 access has occurred here.
The board remains accepted s07, with 36 executed predecessors and reconstructed
full state `7df030bf565c63503a04bfaf2a9cd114632c60e5f357335da9f49d81c2e544e6`.

Future comparison must use fresh reviewed packages, exact 36-predecessor
diagnostic/return plans, repeated cold observations and a separate steady
window, calibrated existing timing instrumentation and measured resource
budgets. Preserve >=6000 us rejection and <=3000 us headroom target; no timing
improvement or qualification is claimed by this implementation. Verify both
deck loads/MAIN/PFL and I02-I13 only after the timing/resource gate permits it.
If a candidate fails, restore/readback/cold/listening-verify exact s07 before
ending the device campaign. APTA, DAC, OTA and other peripherals remain outside
this candidate.
