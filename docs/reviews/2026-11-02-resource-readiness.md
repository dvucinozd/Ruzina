# Both-deck resource readiness before UAC startup

Date: 2026-11-02. Branch `ogranak/jc1060-resource-readiness`, based on
`8449b95bd869ec34b0be0f3167d92f142d2deaba` and the
[resource lifecycle decision](2026-11-02-usb-service-and-resource-decision.md).
Status: IMPLEMENTED / OFFLINE_VALIDATED / BUILD_ONLY / NOT_FLASHED.
Draft [PR #80](https://github.com/dvucinozd/Ruzina/pull/80) is stacked above #79;
neither this PR nor the earlier stack is merged by this work.

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
enabled and pass. Rust host profiles pass 39/2/56/57/70/75 tests respectively;
all six host Clippy and nine RISC-V release Clippy configurations pass. Windows
Python passes 40 cases with six C-compiler skips; all six actual C cases pass
separately under WSL. Exact-parent CodeRabbit review of source commit
`d28abd0414a264cce96de4be2a2b783dc58cfd60` completed across ten files with 0 issues.
See the [verification index](evidence/2026-11-02-resource-readiness/checks.json)
and retained raw logs beside it. Initial failed setup/test attempts remain in
the external evidence folder. The first reuse test counted small per-track
allocations too; its corrected scope counts permanent-store-sized allocations.

Clean packages built from that committed source with empty dirty status:

| Package | App bytes | App SHA-256 |
|---|---:|---|
| `clean-normal-d28abd0` | 819424 | `a4379cec9665fec098d68d056e1885502c577609972d25cdcc1dd11bd32de06e` |
| `clean-profile-d28abd0` | 825008 | `7e62af397730d604b86991a6848f9f58c6705b414c5d124e3ce1019e419f099e` |

All 285 source hashes per package and 18 combined artifact hashes match. Both
packages keep the normal flash packetizer placement; no IRAM experiment is
enabled. Relative to the prior clean control profile (`605abe7`), allocated
ELF sections add 1912 bytes `.flash.text` and 72 bytes `.flash.rodata`; RAM
section sizes are unchanged. Debug section deltas are recorded in the index.
Image metadata/code offsets differ too, so this is static accounting only.

The SD task is already configured for **98304 bytes** when DECK_PLAY is enabled;
49152 applies to the non-decoding load profile. The measured static direct-call
estimate is 74640 bytes, up from 73616 in the prior control profile and below
the unchanged 90112-byte build limit (8 KiB reserved for C). The load-run entry
frame is 12160 bytes; the largest individual Rust frame remains 24160, under
the unchanged 24576 limit. USB fill's chain is 3120 bytes, under its unchanged
4096-byte limit on an 8192-byte task. These are disassembly estimates, including
conservative branch/tail-call handling; indirect calls and actual runtime
high-water marks are not established. No task stack or threshold was increased.

`resource_prepare` telemetry records PREPARING/READY uptime, free/largest PSRAM,
free internal RAM and the SD task stack high-water mark. Actual target readiness
duration, minimum heap/largest block and stack remain NOT_RUN. Host allocator
counts and static frame checks do not substitute for them.

## Next gate

Exact-parent review and clean normal/profile builds are complete. The source
head's [host #546](https://github.com/dvucinozd/Ruzina/actions/runs/41492408329)
and [legacy #69](https://github.com/dvucinozd/Ruzina/actions/runs/41492408384)
CI runs are still in progress at this publication checkpoint; do not call the
complete CI matrix passed until their conclusions and exact SHA are verified.
Documentation-only publication after the source commit has separate CI.
Packages remain BUILD_ONLY, without
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
