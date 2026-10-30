# S6 read-only SD/exFAT diagnostic

Date: 2026-10-30. Unit JC1060-20261030-COM3, P4 v1.3. The owner inserted a
64 GB exFAT card and reports it contains no files. This campaign must not create
test files or reformat it. The prior [display/touch diagnostic](2026-10-30-legacy-s6-display-touch.md)
remains an exact recovery baseline, with its accepted static image and polling
touch evidence. SD uses its own profile; display/touch is inactive during it.

## Source, power and ownership

The owner-supplied New_Panel BSP uses SDMMC slot 0, four lines, no established
card-detect/write-protect GPIO, and on-chip LDO4. The source-audited schematic
maps D0..3 to 39..42, clock 43, command 44 and ESP_LDO_VO4. The diagnostic starts
at 10 MHz (below the vendor's 40 MHz), no UHS/DDR/1.8V override, and uses the pinned
IDF 6.1 power-control driver. It does not drive arbitrary detect/VBUS pins.

One task owns card, filesystem, read buffers and lifecycle. No filesystem or
card pointer escapes. The local generation increments and ready flag clears
before unmount/deinit on error/removal; reinsertion requires an explicit app
restart, not implicit reuse of the old card object. This is a diagnostic local
fence, not yet the shared product media/session adapter. No C6 SDIO owner or
other card consumer runs concurrently.

## Read-only enforcement and exFAT

Pinned IDF's default FatFs R0.16 config has FF_FS_EXFAT0 and FF_FS_READONLY0.
The diagnostic therefore copies the pristine ff.c/ff.h/ffunicode.c/diskio.h
into its build directory, retaining license notices, with a private ffconf:
read-only 1, exFAT1, LFN UTF-8,512-byte sectors, one volume, no format/trim.
The global IDF checkout and shared Rust filesystem code are not modified.
The generic IDF read/write FatFs/VFS component is not linked. Build manifests
hash the four pristine sources and ELF checks reject f_write/f_mkfs/unlink/
rename or SD write/erase sector APIs in the linked image.

The disk adapter returns write-protected status and always rejects disk_write
and unsupported ioctl, including TRIM. A self-test calls those rejection paths
without dispatching any host write/erase command. A second host transaction
guard rejects memory program/erase/lock/general-write opcodes. Initialization
may configure volatile bus width/speed; read-only means no block/filesystem
content programming, not absence of SD protocol configuration commands.

No format fallback, mount-time repair, VFS writable mount or test-file creation
exists. The empty card can qualify identity, mount, root enumeration and raw
read stability, but not existing audio/export file reads. Root enumeration is
bounded to 64 entries, does not log filenames/content, and any opportunistic
root-file prefix read is bounded to 64 KiB. An empty root produces an explicit
NOT_RUN_NO_ROOT_FILE, never a file-read PASS. Directories such as host-created
system folders count as entries, not user audio files.

## Intended bounded scenarios

1. Verify the exact profile/build/map/symbols; preserve S6 display recovery.
2. Live identity/security, prior-sector comparison, write reviewed P4 firmware
   sectors and readback. These are P4 internal-flash writes, not SD writes.
3. Observe card geometry, repeated raw read hash, exFAT mount and root entries.
4. Poll card status and first sectors; removal/error must quiesce the single
   owner. No active DMA/read continues after its terminal state. No GPIO-based
   removal detection is claimed.
5. Operator reinserts the same card; app restart creates fresh host/card/volume
   objects, then repeat mount/read smoke. Product cancellation stress, card
   replacement with another volume and file-level generation races stay open.

## Initial execution

Thirteen offline tests pass, including SD profile ownership guards. Fresh
`s06-reviewed-sd-readonly` build passed Rust fmt/Clippy, ABI/header/map/source
checks and the no-write-symbol gate. App215360 bytes, SHA-256
`53cdfb5126220aa19a1eb0dddcdd7079a2776fde2de3731eab5e700b677ce546`.
The first development build exposed a signed-capacity printf mismatch and
IDF's FF_DIR naming, corrected without warning suppression before the accepted
fresh build. The pristine SDK/FatFs tree was not patched.

The P4 bootloader/table/app sector payloads were written only after live
identity/security and prior display-diagnostic sector comparisons; readback
matched every written sector:
[programmer result](../boards/evidence/jc1060-20261030-com3/s06-sd-first-flash-result.json),
[recovery plan](../boards/readiness/jc1060-s06-sd-write-recovery-plan-20261030.json).
The generic writer's legacy descriptive "no BSP peripheral access" string
applies only to its programmer stage; the SD application later activates the
explicitly reviewed SDMMC/LDO4 resource set.

On the first USB-reset boot, the card reported 63,864,569,856 bytes,
124,735,488 sectors of 512 bytes. The exFAT volume atLBA 2048 mounted with 256-sector
(128 KiB) clusters. Root enumeration completed with 0 files and 1 directory;
the directory's name was intentionally not logged or inferred. No test file
was created and file reading remains NOT_RUN_NO_ROOT_FILE.
[Initial runtime](../boards/evidence/jc1060-20261030-com3/s06-sd-first-boot-result.json)
observed 45 heartbeats, stable repeated reads of the first 8 sectors
(FNV-1a339d5f03), and no HP watchdog. The task's remaining stack was 6296 bytes.
This is not a hash of the entire card or a file/media streaming benchmark.

`write_calls=1` is the adapter rejection self-test, not a write submitted to
the card. It remained 1 throughout health observations; host-denied remained 0,
showing no forbidden opcode was attempted by initialization/mount/read paths.
The explicitly rejected TRIM self-test also had no transaction side effect.
The private compile-time configuration and linked-symbol checks supply the
independent no-write boundary; a stable initial-sector hash alone does not
prove whole-card immutability.

## Removal and reinsertion

The owner removed only the SD card with USB power retained. The task timed out
on its next card transaction, cleared ready, advanced local generation to 2,
unmounted, deinitialized the host and released its LDO handle. It emitted
QUIESCED and self-deleted, without a watchdog or further SD polling:
[removal result](../boards/evidence/jc1060-20261030-com3/s06-sd-removal-result.json).
The terminal count was 195 reads,1 deliberately rejected write self-test and 0
forbidden host commands. Polling/timeout detection is not a hardware CD signal;
removal during an open file or concurrent load is not covered by this empty-card
scenario. The initial mount/root operations had already completed.

The owner reinserted the same card; explicit app restart created fresh card/
host/filesystem objects. Three app restarts each observed exFAT mount/root,
scalar match and 11 heartbeats without quiesce/watchdog:
[restart results](../boards/evidence/jc1060-20261030-com3/s06-sd-reinsert-result.json).
All runs retain the same geometry/root counts/initial-sector hash. These are
three app startups following one physical reinsertion, not three physical
removal cycles or a replacement-card identity test. The local counter restarts
at 1 after app reset; it is not a globally persistent media generation.

The scalar regression build also passed, confirming the private FatFs config
did not become a global SDK/product option. It was not flashed. Native minimum
302 remains unchanged. No full-product/physical audio acceptance follows.

## Cold startup and remaining scope

The owner left the same card inserted, removed the sole USB power for 5–10 s,
and reconnected without BOOT/RESET. The monitor missed the new startup/mount
lines, so its direct `sd_match` marker parser correctly returned false.
However, the raw stream contains one uptime drop and continuing new SD health
read counts after it, without QUIESCED or watchdog. The
[corroboration record](../boards/evidence/jc1060-20261030-com3/s06-sd-cold-corroboration.json)
documents this separate runtime evidence. Do not relabel the raw marker result
as a directly captured mount/reset-reason PASS. Source control flow enters
this health loop only after successful card init/exFAT mount/root enumeration.
This supports post-cold operation, with an explicitly missing startup prefix.

The board remains on the SD-only read-only profile with the card inserted;
display/touch is inactive. No file or card block was written by this diagnostic.
The initial header-sector comparison is a stability probe, not a whole-card
backup or independent verification of every sector's immutability.

Remaining gates: a known file/export fixture for bounded file reads and exact
content validation, file seeks/stream throughput, removal during active file
load, repeated/remapped/replacement media, shared Rust generation/cancellation
adapter, simultaneous display/SD load and stress/soak. Current physical media
identity is the owner-confirmed same card/geometry, not a CID-based product key.
The diagnostic counter is local to one task/app instance. No SD writes or
persistence/format feature should be enabled as a shortcut to fill these gates.
Next action is read-only known-file testing once the owner supplies content on
the empty card; no further peripheral owner was activated by this batch.

## Preservation and final checks

The [SD evidence package index](../boards/readiness/jc1060-s06-sd-evidence-package-20261030.json)
records a5,368,318-byte ZIP with 91 entries and a matching secondary copy under
BoardBackups. All member SHA-256 values and ZIP CRC were verified. Archive SHA:
`a53297709a3a8a6675d2907130d44da4b967f2159279968b2a18c9e54e6e8822`.
The package contains exact source/artifact/readonly-symbol/runtime snapshots
and the pristine licensed FatFs sources; it contains no factory or SD card dump.
Final checks: 13 offline methods PASS, unit record validator PASS,191 local
documentation links PASS before this final package link, whitespace diff PASS.
The current application/shared Rust crate sources and native minimum 302 were
not modified. Current work remains local/uncommitted; no push or remote CI run.

Reference: [Espressif SD protocol/host lifecycle](https://docs.espressif.com/projects/esp-idf/en/v6.1/esp32p4/api-reference/storage/sdmmc.html)
plus the pinned local FatFs/config/SD driver sources and owner vendor BSP.
