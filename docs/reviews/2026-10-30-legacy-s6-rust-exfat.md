# S6 shared Rust partition/exFAT fixture profile

Date: 2026-10-30. Unit:JC1060-20261030-COM3, ESP32-P4 v1.3.
This extends the [raw-block transport observations](2026-10-30-legacy-s6-idf-media-transport.md)
with a separate `sd-rust-exfat` profile. Build/physical outcomes are recorded
below after execution. Native minimum 302 and shared/native crate sources are
unchanged; APTA remains OFF/unmerged. No product storage/audio acceptance follows
from a diagnostic file check.

## Shared read path and ownership

The isolated library adds optional `exfat-fixture` and `idf-exfat-fixture`
features. [Fixture orchestration](../../firmware-idf/legacy-jc1060/rust-core/src/exfat_fixture.rs)
reuses `ruzina-media-partition`, `ruzina-media-io`, `ruzina-media-exfat`,
`ruzina-media-fs` and `ruzina-media-session`. SHA-256 is computed in Rust using
the workspace's existing sha2 version 0.10.9; exfat-embedded 0.1.3 remains a
dependency of the shared crate. Exact transitive resolutions/checksums live in
the isolated Cargo.lock. No vendor SDK or native revision gate is patched.

The existing C worker exclusively owns the card/host/LDO4 and 512-byte internal
DMA-capable bounce allocation. Rust's `OwnedBlockDevice` client owns 512-byte
arrays. The new profile gives the dedicated client task 32768 stack bytes;
the retained raw-only profile keeps 16384. No Rust object, filesystem handle,
reference, future or allocator/free responsibility crosses C.

Mount discovery uses the shared MBR/superfloppy parser and bounded streaming
GPT parser. Candidate extents are constrained by the observed card geometry.
The selected exFAT header must parse, its partition offset must match the chosen
LBA and the main boot-region checksum must validate before the file is opened.
The production shared async exFAT reader runs over `AsyncPartitionDevice` and
the same IDF read-only transport. There is no C/FatFs mount/file owner and no
write trait or persistence operation in this adapter. Actual GPT/other media
compatibility is not established by an MBR card observation.

The fixed path `/RUZINA_TEST/Čitanje i seek 64GB.bin` is unchanged from the
owner-copied PC fixture: 8389121 bytes, SHA-256
`1cb18815702b14ff92ad02f4976a162481d8d91beb429ba5607b131d4ab281de`.
Each full pass checks every byte's position pattern and SHA, a zero-byte EOF
read, then 10 nonmonotonic seek/read cases:SIZE-513,0,131071,511,131072,4095,
512,SIZE-1,4096,SIZE. Success is published only after close and a current lease.
Reads use 512-byte chunks and deliberately yield one tick/16 KiB, plus one second
between passes. This is not a throughput benchmark or an audio deadline test.

## Cache lease and cancellation policy

The shared exFAT reader's own lease compares a file with its volume; it cannot
observe an external disconnect on a cache hit. The diagnostic therefore checks
the live transport binding before and after filesystem operations, including
EOF/seek/close and final pass acceptance. Host tests demonstrate that direct
cached file reads can still return bytes after queue invalidation, while this
fixture gate rejects the same stale state. This local policy does not fix or
implicitly qualify every product consumer of the shared filesystem.

Mount borrows an outer block client through `ReadBorrow`. Canceling mount/read
keeps that client's pending ticket and the owner/queue's buffer alive. After
cancellation, discard the volume/file cursor, drain `recover_pending()` and
remount; an interrupted shared volume may have scratch=None and must not be
resumed. The host cancellation test drains the same pending buffer, invalidates
the old lease, mounts a new volume and verifies the full fixture again.
The physical profile itself runs sequential operations on a dedicated task;
it does not yet inject cancellation into the product executor or implement
automatic hot-remount. No canceled filesystem state is presented as resumable.

## Local gates

Twenty-seven Rust tests pass: 19 transport protocol,1 concrete concurrent FFI
and 7 shared-filesystem integration scenarios. The new cases cover the full
UTF-8 fixture SHA/EOF/seeks, corrupted file/boot checksum, canceled mount,
canceled file read with drain/remount/new lease, stale cached reads and lease
loss during a long verify. Synthetic host data uses an MBR with a small
512-byte-cluster exFAT volume; it is distinct from the physical 64 GB card's
128 KiB clusters. Shared regression tests pass 76 cases: block 6, exFAT23,
filesystem 14, IO 4, partition 18 and session 11.

Twenty Python tooling methods pass, including mutually exclusive ownership,
new-profile dependency rejection and agreement between the PC fixture, Rust
digest/path and fixed C success telemetry. Rustfmt, host/target Clippy and a
bare release Rust target build passed with Rust 1.95.0 and warnings as errors.
The fresh IDF builder additionally requires the actual exFAT Rust entrypoint,
matching profile flags, legacy 100..199 header, single-float ABI and no linked
SD write/erase/FatFs APIs. Build/link/runtime outcomes remain separate.

The raw-block recovery image and all earlier archives remain immutable. A new
programming plan must replay the raw transport plan's six ancestors plus that
plan as the current predecessor; do not use a merely built regression package
as physical flash history. Factory backups and secondary copies must reverify
before any new device write. No commit/push or remote CI is part of this batch.

## Exact image and first runtime

Fresh reviewed package:`s06-reviewed-rust-exfat`. App: 244784 bytes,
SHA-256 `ec6b7f0f159c4f6d77cafcb692df84da046bfd2caa0151854c1df693a3f59267`.
The full source/artifact manifest matches, and a separate fresh raw-bridge
regression package also passed without being flashed. The
[write/recovery plan](../boards/readiness/jc1060-s06-rust-exfat-write-recovery-plan-20261030.json)
replays all seven ancestors, verifies all four factory backup files, and keeps
previous bytes past the end of each new component. Live revision/MAC/security/
RDID and predecessor sector hashes matched. Writes at 0x2000/24576,0x8000/4096,
0x10000/245760 bytes all passed readback:
[programming result](../boards/evidence/jc1060-20261030-com3/s06-rust-exfat-flash-result.json).
No full-chip erase, eFuse or SD content write occurred.

The USB software reset from ROM/stub booted the new image. Rust mounted the
physical volume atLBA 2048, validated its main boot checksum, and completed
three full file passes in 48.232 seconds, each with the fixed digest, EOF and
10 seeks. Heartbeats continued 48 times without error/quiesce/fault-held or
panic/HP watchdog:
[first boot result](../boards/evidence/jc1060-20261030-com3/s06-rust-exfat-first-boot-result.json).
Minimum observed client stack remaining was 15248 bytes, and owner telemetry
reported 5352 bytes. No write opcode was submitted. These are bounded exact-
image observations, not throughput/maximum-stack/long-soak qualification.
Subsequent restart and physical lifecycle results are separate gates below.

Three explicit app restarts each remounted atLBA 2048 and completed one full
fixture pass in 15.686/15.677/15.768 seconds, with 15 heartbeats each, minimum
client stack 15248 and no errors/quiesce/fault-held or panic/HP watchdog:
[restart results](../boards/evidence/jc1060-20261030-com3/s06-rust-exfat-app-restarts-result.json).
The elapsed values include startup, deliberate yielding and serial capture;
they are not isolated storage/DSP timings. No firmware change occurred between
first boot and these restarts. Exact-image physical removal/reinsert and cold
boot remain unrun at this checkpoint.

The [evidence package index](../boards/readiness/jc1060-s06-rust-exfat-evidence-20261030.json)
records accepted source/build hashes, offline commands/logs, raw-profile build
regression and boot/restart captures, with a separately verified secondary ZIP.
Factory dumps/private recovery payloads and SD-card contents are excluded from
that evidence archive. All working changes remain local and uncommitted.

## Physical open-file removal

The owner confirmed removing the card while the repeated Rust file verification
was active. Pass10 completed; pass 11 entered READING, followed by an actual
`sdmmc_read_sectors_dma` command timeout 0x107. The owner quiesced with 164386
read calls and 0 denied write commands; Rust reported pass 11 event 4/value 1
(`Error::Read`) and the client stopped with result-5:
[raw result](../boards/evidence/jc1060-20261030-com3/s06-rust-exfat-removal-result.json),
[operator and independent capture review](../boards/evidence/jc1060-20261030-com3/s06-rust-exfat-removal-review.json).
Six later heartbeats continued with no pass 11 success, success after quiesce,
fault-held, panic or HP watchdog. The captured low-level read failure propagated
through the awaited shared filesystem operation. The exact instant of card
removal relative to DMA start was not timestamped, and no explicit future-drop
cancellation was injected physically. Those facts remain separate from the
host canceled-future tests. No reset/flash or SD write was needed for removal.
Same-card reinsertion and a new mount are the next physical gate.

## Reinsertion, cold return and handoff

The owner confirmed reinserting the same card. Three explicit app restarts
each freshly mounted atLBA 2048 and verified the complete fixture SHA/EOF/10
seeks in 15.838/15.684/15.761 seconds, with 15 heartbeats each and no failure:
[raw results](../boards/evidence/jc1060-20261030-com3/s06-rust-exfat-reinsert-result.json),
[independent reparse and operator record](../boards/evidence/jc1060-20261030-com3/s06-rust-exfat-reinsert-review.json).
This is one physical same-card reinsertion and three startups, not replacement
media acceptance or an automatic hot-remount path.

The owner then confirmed disconnecting sole USB power for 5–10 seconds and
reconnecting without BOOT/RESET, leaving the SD inserted. The passive monitor
sent no reset or console command. USB disconnected/reopened, uptime dropped,
and new pass 1 completed the entire 8389121-byte fixture SHA/EOF/10 seeks with 15
new heartbeats, no error/quiesce/fault-held or panic/HP watchdog:
[raw cold capture](../boards/evidence/jc1060-20261030-com3/s06-rust-exfat-cold-result.json),
[independent reparse and operator record](../boards/evidence/jc1060-20261030-com3/s06-rust-exfat-cold-review.json).
Minimum reported post-cycle client stack was 15232 bytes. Initial ROM/app init/
reset-reason output was missed during native USB re-enumeration. This is an
operator-confirmed cold return with a captured low-uptime full file pass;
direct cold init/reset-reason capture is NOT_CAPTURED. Do not change that into
a direct boot-prefix PASS or transfer it to another image.

The superseding lifecycle ZIP in the evidence index retains the exact accepted
source/build snapshots, original pre-lifecycle archive, new raw captures and
separate operator reviews. ZIP CRC, every member hash and independent secondary
archive SHA-256 are checked. The device remains on this read-only Rust exFAT
diagnostic; the final monitor is closed. No firmware/source/SD contents changed
during removal/reinsertion/cold tests. Unit/status/README/handoff are current.

Next: connect this validated diagnostic read path to the product media/load
owner on the separate IDF platform, retaining one filesystem owner and its
live lease around cached reads and final load commits. Preserve the cancellation
policy: discard interrupted file/volume, drain pending buffers, then remount
under a fresh lease; do not resume partial cursor/scratch state. Add integration
tests against the actual load/catalog request and cancellation contracts before
running any real track workload. Product executor/drop races, automatic media
remount, replacement cards, GPT runtime, fragmented/>4 GiB files, general Unicode,
long soak and concurrent audio/display remain open. USB host/audio/C6 and SD
persistence are outside this read-only gate. Native 302 and APTA remain unchanged.

For the next programming plan, replay this plan's seven predecessors and then
this plan as the immediate physical predecessor. The merely built raw regression
package was never flashed and is not flash ancestry. All changes remain local,
uncommitted/unpushed; remote CI and release publication were not run.
