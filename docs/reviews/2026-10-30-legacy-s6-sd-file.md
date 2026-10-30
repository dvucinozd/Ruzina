# S6 deterministic SD file/seek and active-load removal

Date: 2026-10-30. Unit JC1060-20261030-COM3 / P4 v1.3. This extends the
[empty-card mount/removal diagnostic](2026-10-30-legacy-s6-sd-readonly.md) with
an opt-in `sd-file-readonly` profile. The known mount-only image remains the
immutable recovery predecessor. Display, USB host, audio and C6 remain inactive.

## PC fixture and operator transfer

The generator `firmware-idf/legacy-jc1060/tools/make_sd_fixture.py` creates a NEW
PC-side output directory and never discovers or writes removable drives.
Prepared location: `C:/Users/Daniel/Desktop/RUZINA-SD-READ-TEST`.
The owner confirms copying the `RUZINA_TEST` subdirectory into the card root via
a reader, retaining the Unicode filename, safely ejecting and reinserting the
same card in the board. This host copy is the only intentional SD content
addition in the campaign; the firmware remains read-only.

Relative path: `RUZINA_TEST/Čitanje i seek 64GB.bin`.
Length: 8389121 bytes (8 MiB + 513).
SHA-256: `1cb18815702b14ff92ad02f4976a162481d8d91beb429ba5607b131d4ab281de`.
Byte at position p is `(p*73+(p>>8)*19+(p>>17)*131+0x5a)&255`.
This is arbitrary deterministic binary, not copyrighted audio, a Rekordbox
export or a codec fixture. No fixture generator/metadata writes run on the board.

## Verification contract

The profile requires the private read-only SD owner and reuses its no-write/
format/trim boundaries. It opens exactly the known nested UTF-8/space filename
using FA_READ, requires exact length, reads 4096-byte chunks, checks every byte's
expected position pattern, computes SHA-256 through IDF 6.1's public PSA Crypto
API and compares the full digest to the fixed independent PC-side value.
Short reads/errors/incorrect bytes/digest or close failures fail the pass.
EOF is checked by a read returning 0, then 10 nonmonotonic seek/read cases cover
the final 513 bytes, position 0, sector 511/512,4095/4096, cluster 131071/131072,
last byte and exact EOF. Seek offsets are diagnostic, not audio sample positions.

The verifier deliberately yields one RTOS tick after each 16 KiB for scheduler
responsiveness and an observable open-file removal window. Measured f_read IO
time, worst individual read, worst seek+read and wall time are reported
separately; wall time includes these artificial delays and hashing. This is
not a real-time audio deadline or maximum card throughput benchmark.

After the first pass, the same single task reopens and verifies the file on
repeated passes. No borrowed FIL/card/buffer escapes, no asynchronous completion
queue exists and local generation remains scoped to that owner. On failed IO,
the file closes, then the outer owner invalidates ready/generation and unmounts/
deinitializes as before. A stale successful pass must not be emitted after the
terminal quiesce event. Reinsertion requires explicit app restart.

## Offline results and build development

Seventeen test methods pass, including fixed fixture SHA/size, UTF-8 path/digest
agreement with the device, boundary chunks, no overwrite and fixture/SD owner
configuration exclusions. Target fmt/Clippy and linked no-write-symbol checks
are required by the fresh builder before any physical image is accepted.

An early compile used an obsolete public `mbedtls/sha256.h`; IDF 6.1 exposes
PSA Crypto instead. The implementation switched to psa_hash_setup/update/finish
and declared the mbedtls dependency unconditionally for IDF's requirement graph.
These are compile fixes without vendor SDK changes or diagnostic suppression.
Failed development build outputs are retained, not accepted as runtime packages.

The first prewrite guard blocked without writing because the new image expands
beyond the smaller SD mount image. An older, larger display image left tail
sectors there; reconstructing only the immediate predecessor on factory bytes
was incomplete. The planner now replays a complete oldest-first SHA-linked
history through repeatable `--ancestor-plan` arguments, rejects missing/
unordered/altered history and includes the current predecessor last. A regression
test preserves the larger image's tail after a smaller upgrade, then verifies
missing ancestors and modified payloads reject. A read-only physical check
confirmed the complete reconstruction against live sectors. The rejected
attempt remains at `s06-sd-file-first-flash`, with flash_write_attempted=false.
The corrected no-write check is at `s06-sd-file-chain-readonly-check`.
This fix also changes the planning requirement for later diagnostic upgrades:
do not omit older ancestor plans merely because the most recent image is known.

Exact build, programming/readback, runtime and archive outcomes follow below.
Native minimum 302 and shared product crate sources remain unchanged.
File/seek smoke cannot qualify complete exFAT robustness, fragmented files,
replacement-media identity, two-deck streams or the product cancellation adapter.

## Initial physical result and telemetry refinement

Accepted first file image: 233776 bytes, SHA-256
`b1afe33804197726899ba8eedb6c8f2bc9eabec401962dad322bc61febd6a073`,
fresh build `s06-reviewed-sd-file-chain`. Corrected preimage comparison and
write readback passed. The 60-second runtime capture observed nine full content/
SHA-256/EOF/10-seek passes without fixture failure, quiesce or HP watchdog:
[first result](../boards/evidence/jc1060-20261030-com3/s06-sd-file-first-boot-result.json).
The nested UTF-8/space path therefore opened successfully. Root-level prefix
sampling still reports no root file because the fixture lives in a subdirectory;
it is a separate observation from the explicit successful full-file verification.

**Correction after reviewing the complete raw log:** the earlier report read
the wall time incorrectly as about 5123 us. All nine original complete records
actually report 5123096..5131023 us, consistently above IO 2389959..2392044 us.
The alleged four-int64 printf failure is withdrawn. A strict full-line reparse
checked the raw log SHA against the original capture result, nine sequential
pass numbers and all timing inequalities:
[independent recheck](../boards/evidence/jc1060-20261030-com3/s06-sd-file-original-timing-recheck.json).
No firmware/runtime defect is demonstrated by these records; arbitrary varargs
are still outside this bounded observation. No additional device access was
needed for this correction. The raw logs remain unchanged.

The already-flashed refinement additionally computes/checks wall>=IO and prints
one 64-bit value per call, including start/end timestamps for independent
arithmetic validation. This provides extra diagnostic visibility; it is not
evidence of a required ABI workaround. The current source comment is corrected
accordingly; this comment-only change is not a rebuilt or reflashed image.

## Final image and active-read removal

Accepted refined image: 234224 bytes, SHA-256
`f501dd087eec7bec7dff4729deb0f213a0d15d18fbe2ae8f1824e38519d9e48e`,
fresh build `s06-reviewed-sd-file-timing`. The complete predecessor chain was
validated again and P4 flash readback matched:
[programmer result](../boards/evidence/jc1060-20261030-com3/s06-sd-file-timing-flash-result.json),
[exact recovery plan](../boards/readiness/jc1060-s06-sd-file-write-recovery-plan-20261030.json).
The earlier mount-only and file-v1 image snapshots remain independent evidence.

The refined image observed nine complete file/SHA/EOF/10-seek passes in 60 s,
without fixture failure or HP watchdog:
[runtime result](../boards/evidence/jc1060-20261030-com3/s06-sd-file-timing-boot-result.json).
Each pass's independently printed end-start equals wall duration, with wall>=IO:
[timing validation](../boards/evidence/jc1060-20261030-com3/s06-sd-file-timing-validation.json).
Typical accumulated read IO is about 2.39 s, wall about 5.12 s per 8 MiB + 513 pass;
the first cold-cache read reached 3049 us and seek+read peaks about 755 us.
These are observed bounded profile timings, not deadlines or an audio stress PASS.
The separate timing records are physically observed. The original multi-value
records are also consistent, as documented in the correction above.

The owner removed the card during repeated verification. Pass19 failed at
offset 1617920, fresult 1 (FR_DISK_ERR), and its read-only close returned 0.
The outer owner emitted QUIESCED generation 2, with 37646 read calls, exactly 1
deliberate write rejection and 0 denied host opcodes. No HP watchdog or successful
pass 19/after-quiesce completion was reported:
[removal result](../boards/evidence/jc1060-20261030-com3/s06-sd-file-removal-result.json).
This is an expected injected media-removal failure, not a content regression.
The monitor's first partial line spans the serial-session boundary and is not
counted as a complete pass. Other complete passes remain separately recorded.
The owner then confirms reinserting the same card; fresh app-restart results
follow below. No SD file write, repair or format was enabled.

## Reinsertion and final scope

After owner-confirmed reinsertion, three explicit app restarts each remounted
the card, opened the same nested UTF-8 filename, completed four full verified
passes and continued 24–25 heartbeats without fixture failure/quiesce/watchdog:
[reinsertion results](../boards/evidence/jc1060-20261030-com3/s06-sd-file-reinsert-result.json).
The 12 complete pass/timing sets were independently checked against fixed SHA
and start/end arithmetic. This is one physical reinsertion plus three app
startups, not three replacement-media tests or the shared async load pipeline.

The scalar build regression also passed. Seventeen offline test methods,
source/artifact/predecessor/readback guards and final unit/docs checks pass.
No native application/shared crate source or native revision 302 gate changed.
No commit/push or remote CI was performed. The final current-image generic
status fields now point to this package; cold/reconnect observations of older
images remain scoped separately and are not transferred to this image.

At the end of this campaign the device remained on the read-only file diagnostic, repeating verification
with no active display/audio/USB-host/C6. No firmware SD writes, repair or
format were enabled; the owner copied the fixture using a PC reader.
The local generation fence drains one synchronous owner; cancellation with
queued shared Rust requests remains unqualified. Fragmentation, files over 4 GiB,
arbitrary Unicode, real media/export parsing, audio deadlines, display/SD
concurrency, long soak and cold boot of this exact final image remain open.
The later [IDF/Rust transport campaign](2026-10-30-legacy-s6-idf-media-transport.md)
temporarily replaces it with a separate raw-block profile; this file campaign's
observations remain scoped to its own exact image.

## Preservation and next gate

The [evidence package index](../boards/readiness/jc1060-s06-sd-file-evidence-package-20261030.json)
identifies the verified ZIP and independent secondary copy. The initial archive
is retained as a historical snapshot, including its subsequently withdrawn
timing interpretation. The superseding v2 archive contains the correction,
recheck script/result and unchanged accepted build/source snapshots. CRC and
every archived member SHA-256 plus secondary-copy SHA-256 are checked. Neither
archive contains factory flash data or an SD-card dump; the deterministic PC
fixture is included. Private recovery payloads remain outside Git.

Next: implement the shared Rust read-only block/media adapter and its
generation/cancellation acceptance. Reuse `ruzina-media-block`,
`ruzina-media-io` and `ruzina-media-session`; retain the C diagnostic as BSP
evidence rather than implicitly selecting it as the product filesystem owner.
First define the single IDF task/queue and owned-buffer handoff: IDF retains
card/DMA allocation ownership and Rust receives only a bounded read contract.
Reject invalid range/length before submission, return the owned buffer on all
terminal outcomes, invalidate the lease before teardown and reject late
completions from the old generation. Host tests must cover removal before
submission, during a pending read, cancellation/drop, late completion and
reinsertion with a new lease. Do not put a blocking SDMMC call on the shared
async executor. Only then build a separate opt-in bridge profile, review its
full flash predecessor chain and repeat known-file/removal tests on that exact
image. Current physical evidence does not establish these future queue gates.

Final local checks: 17 offline methods, unit-record validator, documentation
links and `git diff --check` pass. No shared/native sources, minimum 302, device
firmware, commit or remote state changed during the evidence recheck.
