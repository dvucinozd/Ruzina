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
CI runs both completed SUCCESS, **21/21 jobs** on the exact source SHA. The
publication head `a2997bd82bb0c53869847f1fc99605f0921c2270` also passed **21/21**:
[host #547](https://github.com/dvucinozd/Ruzina/actions/runs/41493301952) and
[legacy #70](https://github.com/dvucinozd/Ruzina/actions/runs/41493302021).
The [CI snapshot](evidence/2026-11-02-resource-readiness/ci-verified.json) records
all 42 successful jobs across those two heads, without skipped jobs. Later
registration/documentation commits have their own workflow status.
Packages remain BUILD_ONLY. No COM3 access has occurred here.
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

## Offline registered candidate and s07 return

The [registration](evidence/2026-11-02-resource-readiness/campaign-registration.json)
binds the exact profile above and the independently accepted s07 package.
Both [offline preflights](evidence/2026-11-02-resource-readiness/offline-preflight.json)
pass. These are prepared paths, not executed writes:

| Transition | Predecessors | Expected full flash after the transition |
|---|---:|---|
| [candidate](../boards/readiness/jc1060-resources-candidate-write-recovery-plan-20261102.json) | 36 | `f417a0b35e462406ef954c0d71a8e9aba07826e133a5b1759003ff198751fd35` |
| [return s07](../boards/readiness/jc1060-resources-return-s07-write-recovery-plan-20261102.json) | 37 | `1a45683632f9951f1121ad0866fd7b3050d146022bfc6c719be18346848ad4fe` |

Current executed ancestry is still **36** and current reconstructed state is
still `7df030bf...`. The prospective return hash differs because bytes after
the shorter s07 app retain the candidate's previously written contents. It
does not imply an executed transition or app mismatch. The return must use
the candidate state as predecessor; do not insert either plan into executed
history until its write/readback has actually completed. No normal engine
write is registered; the normal package is build evidence only.

All 36 historical plans were replayed with verified hashes, exact restore bytes
and preserved sector tails. Four complete factory copies match; every new
plan and payload has an identical secondary copy. The
[archive index](evidence/2026-11-02-resource-readiness/registration-archive.json)
binds 311 files, both engine packages, accepted s07 artifacts and historical
plan/write/restore payloads. ZIP CRC, all member hashes, all secondary members
and the secondary ZIP hash pass. ZIP: **40423786 bytes**, SHA-256
`256fcbbaeb861a7dd8a630f2ed8f71e2bc2eb3c25a259b91bd77452a5a2b9940`.
Primary: `D:/BoardEvidence/JC1060-20261030-COM3/resource-readiness-20261102/`.
Secondary: `C:/Users/Daniel/Documents/BoardBackups/JC1060-20261030-COM3/resource-readiness-20261102/`.
Archives do not contain the complete checked-out toolchains; recorded pins and
separately retained factory images remain required. Original absolute plan
paths must be restored or deliberately remapped and reverified before replay.

`preflight_readiness.py --mode candidate` or `--mode return-s07` has **only** an
offline mode: script/plan/package/source/backup/ancestry and expected before/
after verification. It has no serial entry point. It does not authorize use
of the plan's placeholder raw command templates. Before device work, prepare
and review an execution wrapper bound to these registration hashes, using the
existing guarded programmer's identity/security/RDID/full-predecessor/readback
policy. Bind a passive monitor to the exact expected ELF as a separate action.
Verify the published registration commit's CI first, then refresh actual board
topology and request owner-assisted manual ROM entry. No physical entry is
assumed from an earlier session.

The frozen measurement contract is three cold boots, each 45 seconds after the
expected ELF identity and at least 7000 complete callbacks, then a separate
60-second idle window with at least 9000 new callbacks. Keep tracks unloaded
and controls untouched. Validate one PREPARING-to-READY transition per boot,
no product transfers before READY, readiness duration and heap/stack telemetry,
both calibrations and complete partitions/histograms. Instrumentation overhead
is not independently removed. Historical control observations are context,
not a newly randomized contemporaneous A/B comparison. Any invalid window,
resource fault or >=6000 us callback prevents extension into playback gates.
After this diagnostic, always return/readback/cold-boot/listening-verify s07;
a later normal-image I02-I13 campaign needs independent registration.

## Reviewed execution tools and CI closed

Registration `79e251a999a7afda34f4b8bbfff15aa805bda273` passed 21/21 CI:
[host #548](https://github.com/dvucinozd/Ruzina/actions/runs/41509637593),
[legacy #71](https://github.com/dvucinozd/Ruzina/actions/runs/41509637557).
The [verified snapshot](evidence/2026-11-02-resource-readiness/registration-ci-verified.json)
binds every successful job. Subsequent execution-tool source
`51c6967f2933b61d2b46743bd13c4953762c1a96` also passed 21/21:
[host #549](https://github.com/dvucinozd/Ruzina/actions/runs/41510500324),
[legacy #72](https://github.com/dvucinozd/Ruzina/actions/runs/41510500374), including
the new fake-device fault tests. Its
[snapshot](evidence/2026-11-02-resource-readiness/tools-ci-verified.json) is separate.

The [tool instructions](../../tools/resource_readiness/README.md) now supersede
the earlier missing-wrapper checkpoint. Both commands default to offline
preflight and have no implicit serial import/access. Explicit device execution
additionally needs a fresh output directory and the frozen binding hash. All
checks remain active with Python `-O`. Immutable verified write bytes are held
through execution, with no later payload reopen. Identity/security/RDID/full
16 MiB predecessor equality precede writes; span and whole 16 MiB postwrite
readback follow them. No reset, eFuse change, force or erase-all is requested.
Errors/interruption stop and record evidence; no inferred recovery transition
is executed. The passive monitor sets DTR/RTS off before opening and never
sends bytes. A separate boot-window file preserves complete raw bytes.

Thirteen fake-device tests pass on both Windows and WSL, covering each identity/
security gate, full-predecessor mismatch, partial connect failure, bad span
readback, corruption outside written spans, valid full readback, output reuse,
hash/path drift, port identity, boot-window identity, Python optimization and
passive monitor line/write policy. Exact-parent CodeRabbit review of the seven
tool/workflow files completed with 0 issues; its raw output and test logs are
retained beside the [execution binding](evidence/2026-11-02-resource-readiness/execution-binding.json).

Binding SHA-256:
`8b26e198044cda08c4cc00f1315268ce139cf5ac69ea9a9a85039e53d1416a69`.
The [supplemental archive](evidence/2026-11-02-resource-readiness/execution-tools-archive.json)
has 18401 bytes, SHA-256
`de77bd03f38c4cd43bbf4e022b66e84c12112f3c4b59deab2bd62bd1860e8d8e`.
CRC/member hashes and every secondary copy match. It supplements the unchanged
311-member registration archive; original firmware package bytes are unchanged.
Pinned execution Python remains the external IDF environment (3.13.0), esptool
5.6.0 and pyserial 3.5. Execution binding/tool source is not firmware acceptance.

READY is atomically published **before** the SD task prints its telemetry.
USB and SD log ordering can therefore interleave; a stream/control print before
the READY print is not by itself proof of a readiness violation. The ownership/
ordering contract is tested in Rust/C. PREPARING-to-READY printed uptime is a
global initialization observation including intervening scheduling/logging,
not a pure allocator duration or exact atomic-publication timestamp. Do not
derive causality or CPU time from that difference. Both readiness records and
resource health still need inspection in each physical boot.

At this checkpoint no COM3 access has occurred. The owner reports that the
screen still shows the song list; manual ROM entry remains pending. Current
executed ancestry is 36, s07 remains installed, and no new target timing/heap/
stack result is claimed. After owner-assisted entry, use the frozen programmer,
then the registered cold/steady observations and exact s07 return. Later
evidence-publication CI is independent of the verified registration/tool heads.
