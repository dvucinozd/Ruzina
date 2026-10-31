# Rust M2.5 development handoff

Updated: 2026-10-30. [PR #55](https://github.com/dvucinozd/Ruzina/pull/55)
merged the legacy JC1060 bring-up batch to main at
`46728b2a3779d125b50aadae908b293adaafa25a` (PR head
`00294d18cd59ba5afcd6aaa4b9dcba8465c895b2`). Before the merge, that exact head
passed host, native P4, all eight legacy IDF profiles and the isolated Rust
media-bridge host job. Start the next batch on a new branch from current main;
the `ogranak/jc1060-c01-c02` worktree at
`C:/Users/Daniel/.codex/worktrees/m1-guition-bsp/M1_PCB` holds only merged work.
Publication and review are recorded in the
[publication checkpoint](reviews/2026-10-30-legacy-publication.md). CodeRabbit
reported two minor issues; the merged follow-up fixes numeric prose spacing and
exFAT `PartitionOffset=0` handling with regressions. That code fix is build/test
only, NOT_FLASHED. This merge does not change the current board image.

**Current unit:** the read-only shared Rust partition/exFAT diagnostic is
running on JC1060 v1.3. Full fixture SHA/EOF/seeks, app restarts, open-file
removal/reinsert and owner-confirmed cold return are observed. Initial cold
boot/reset-reason prefix was missed. Product media/load, UI/audio/USB-host/C6
remain unwired. The Library load preflight is now the shared
`ruzina-media-load` crate, used by native firmware and, host-tested, by the IDF
core over its exFAT volume ([media/load preflight](reviews/2026-10-30-legacy-s6-media-load-preflight.md)).
An IDF PSRAM allocator now gives it a target build (`idf-media-load`).
Next implementation: shared PDB catalog import, then a separate opt-in profile. Native minimum 302 is unchanged.

**Next-agent instructions (review 2026-10-30):** read the
[legacy v1.3 review and step-by-step implementation plan](reviews/2026-10-30-jc1060-legacy-review-and-instructions.md).
S0–S3 implementation is recorded in the
[offline execution log](reviews/2026-10-30-legacy-s0-s4-execution.md): evidence
reconciliation, fresh build/package tooling, guarded RAM loader and dedicated CI.
The [S4 recovery/layout analysis](boards/JC1060_LEGACY_RECOVERY.md) uses only
existing backups. Check the execution log for actual validation outcomes before
hardware work. That S0–S3 batch did not access COM3.
The subsequent [S4 runtime campaign](reviews/2026-10-30-legacy-s4-runtime.md)
has programmed/readback-verified the legacy diagnostic, observed three owner
power cycles, three app restarts, three normal USB resets and three serial
session reopens. Use its scenario definitions/limits before continuing.
S4 left that scalar diagnostic on the device; it is now a retained recovery baseline.
S5 is now implemented and has first-boot, three app restarts and owner cold-boot evidence:
[bounded PSRAM/task/FPU/EQ report](reviews/2026-10-30-legacy-s5-memory-task.md).
S5 left its separate memory/task profile on the device; scalar S4 recovery sectors
remain archived. The subsequent [S6 display/touch campaign](reviews/2026-10-30-legacy-s6-display-touch.md)
ran its own New_Panel diagnostic on the device; the owner confirmed
static stripe colors/corners and cold return, with five-zone/named-bottom
touch samples, three app restarts and one cold startup observed. Read its
limits before continuing. The subsequent [read-only SD campaign](reviews/2026-10-30-legacy-s6-sd-readonly.md)
then ran its own private read-only exFAT diagnostic on the device, with display
inactive. The inserted 64 GB exFAT card mounted with 0 root files/1 directory;
one removal quiesced the owner, reinsertion plus 3 app restarts remounted it,
and post-cold runtime SD reads were observed (startup prefix missed).
The subsequent [known-file campaign](reviews/2026-10-30-legacy-s6-sd-file.md)
now verifies the 8 MiB + 513 nested UTF-8 fixture, SHA/EOF/10 seek cases, active-file
removal and reinsertion with 3 app restarts. At that checkpoint firmware was the
read-only file diagnostic; display was inactive. Rechecking all nine original raw timing
records withdrew the printf-failure claim: wall_us is 5123096..5131023, above IO.
The [Rust media bridge protocol](reviews/2026-10-30-legacy-s6-rust-media-bridge.md)
implements the owned-buffer client and queue with host cancellation/generation
coverage. The subsequent [concrete IDF transport](reviews/2026-10-30-legacy-s6-idf-media-transport.md)
subsequently ran a separate raw-block diagnostic on the board: 35 first-boot
passes plus 3 app restarts with 15 passes each match the known first-sector hash.
Display and all filesystems were inactive in that raw-only profile. One physical
idle-status removal quiesced the owner/client with no late success; after owner
reinsertion, 3 app restarts each returned 15 matching passes. Owner-confirmed cold
return also has an uptime drop and 15 matching passes 2..16; native USB missed
the initial init/reset-reason prefix, so direct cold init remains NOT_CAPTURED.
The raw campaign's monitor is closed. The subsequent
[shared Rust partition/exFAT fixture profile](reviews/2026-10-30-legacy-s6-rust-exfat.md)
now runs on the board: three first-boot full SHA/EOF/10-seek passes, then three
app restarts each remounting and completing one full pass. It uses one Rust
filesystem owner over the same IDF SD transport, with live lease checks around
cached operations. 27 local Rust tests include canceled mount/read drain/remount;
76 shared-media regressions and 20 tooling tests pass. The raw profile remains a
recovery baseline. Physical open-file removal caused pass 11 read timeout/error
and owner/client quiesce with no late success; same-card reinsertion plus 3 app
restarts each verified a full file again. Owner-confirmed cold return completed
new pass 1 SHA/EOF/10 seeks with an uptime drop and 15 heartbeats; initial boot/
reset-reason prefix was missed. All monitors are closed. Current image is the
Rust exFAT diagnostic, app SHA `ec6b7f0f159c4f6d77cafcb692df84da046bfd2caa0151854c1df693a3f59267`.
Next: IDF product media/load owner and catalog integration with live lease and
buffer-drain cancellation gates. The diagnostic has not wired that product path.
Full flash history is
now mandatory in the upgrade planner; see that report's ancestry regression.
USB host/audio/C6 remain inactive. S5's bounded results do not qualify these.

**Arrival blocker:** the received JC1060 on COM3 reads ESP32-P4 **v1.3 (103)**
and 16 MiB flash. The native target requires 302 and esp-hal excludes pre-v3
silicon. Do not flash its ready kits or reduce the revision gate. The owner
selected a separate ESP-IDF assessment and then requested a newer IDF; the
isolated experiment now targets stable **6.1**. Read the
[unit evidence, feasibility assessment and ordered next gates](boards/JC1060_ARRIVAL_2026-10-30.md)
before any hardware operation. Backup/recovery state is recorded there and in
the unit JSON; legacy diagnostic programming is observed, while native C02
remains blocked and unrun. Both independent
16 MiB factory backups match and have verified secondary copies. A 6.1 / Rust
pure-RAM probe executed scalar calls twice. The owner completed a full power
disconnect; subsequent ROM RDID and factory boot logs passed, and the owner
confirmed the factory demo before the later diagnostic flash. S5 is completed
as a separate bounded diagnostic; the next adapter gate is described above. Soft reset
after RAM execution produced repeated ROM HP watchdog resets; ROM flash-attach
also failed before the power cycle. PURE_RAM remains unsupported; do not
transfer the later flash-backed reset results to it or repeat RAM runs without
an operator available for cold recovery. The owner reports
New_Panel; both vendor BSP variants contain a stale
EK79007 log label but call JD9165, so that label is not panel identity.

The reviewed A+B implementation is `cb70a425d691d0fd10ec097b263567fdefe70070`.
See the [final review and C01/C02 preparation report](reviews/2026-10-27-publication-and-bringup.md)
for CodeRabbit evidence, ready diagnostic kits and immutable package hashes.
Read the [current status](DEVELOPMENT_STATUS.md),
[implementation plan](reviews/2026-10-27-ruzina-fix-implementation-plan.md)
and [execution log](reviews/2026-10-27-astra-implementation-log.md).
Phase B host integrations and remaining gates are recorded in the
[management log](reviews/2026-10-27-phase-b-log.md) and
[HTTP contract](WEB_INTEGRATION_CONTRACT.md). Product OTA trust/slot configuration,
permitted real fixtures, board runtime wiring and physical qualification remain open.

## Historical handoff — 2026-10-24 and earlier

The following branch identities and test totals are historical snapshots.
Use the current status and execution log above for continuation.
Host-implemented and merged to `main` on 2026-10-23 (PRs #23-#36, each with
host and ESP32-P4 CI passing on `main`): RP-01/02 (PR #23, `ogranak/m25-rust-parity-plan`),
RP-04 load admission (PR #24, `ogranak/m25-rp04-load-admission`), RP-03 CUE
hold/VINYL-CDJ (PR #25, `ogranak/m25-rp03-cue-jog`), RP-05 memory/loop cues
and source restore (PR #26, `ogranak/m25-rp05-cues`), RP-15/18 reconnect
fencing plus PDB row/EXT cue/loop-range edges (PR #27,
`ogranak/m25-rp15-18-edges`), RP-06 playlist hierarchy (PR #28,
`ogranak/m25-rp06-playlists`), RP-07 part 1: PWV4, memory-cue markers,
artwork references and owner (PR #29, `ogranak/m25-rp07-artwork-pwv4`),
RP-08 MP3 frame index/cold seeks/duration (PR #30, `ogranak/m25-rp08-mp3-seek`),
RP-07 part 2: baseline JPEG decoder and cover/logo display (PR #31,
`ogranak/m25-rp07-cover-decoder`), RP-11 durable settings (PR #32,
`ogranak/m25-rp11-settings`), RP-12 Web/C6 contracts (PR #33,
`ogranak/m25-rp12-web-contracts`), RP-13 signed OTA/readiness contracts
(PR #34, `ogranak/m25-rp13-ota`), the RP-09/10/14/16/17 host contracts
(PR #35, `ogranak/m25-rp09-17-audio-platform`) and the UAC 48 to 44.1 kHz
converter (PR #36, `ogranak/m25-uac-resampler`). See the parity
plan rows for evidence and open items (HTTP adapter for uploads, Web Remote admission/tap
semantics, physical controller/
import/listening acceptance). The sections below still describe the original
baseline.

## Read first and locate the checkout

1. [M2.5 parity audit and acceptance plan](M2_5_PARITY_PLAN.md): 20 open work
   items, pinned sources, implementation evidence, priorities and source links.
2. [Development status](DEVELOPMENT.md) and [product requirements](M1_RUST_PRODUCT_REQUIREMENTS.md).
3. [Architecture](M1_RUST_ARCHITECTURE.md), [validation](VALIDATION.md),
   [controller ADR](adr/ADR-004-controller-profile-platform.md) and
   [OTA ADR](adr/ADR-007-ota-boot-recovery.md).
4. [Board matrix](boards/GUITION_HARDWARE_MATRIX_2026-10-20.md) and
   [first-boot procedure](boards/GUITION_FIRST_BOOT_2026-10-20.md) before hardware work.

Repository: `https://github.com/dvucinozd/Ruzina.git`.
Continuation branch: **`ogranak/m25-rust-parity-plan`**. Audit/planning commit:
`9e1c4344833b808bfaf5305399f09156f59b56a7`; this handoff is a subsequent
documentation-only commit on that branch. Read the branch's current HEAD rather
than treating the earlier audit commit as the complete handoff.

On this Windows machine the active continuation checkout is:

```text
C:\Users\Daniel\.codex\worktrees\m1-guition-bsp\M1_PCB
```

Reuse that worktree after checking its current status. The primary checkout
`D:\AI\M1_PCB` was clean on `main` at
`0f2fb1425ef76f2541a8c76841cfa52178dca589` when this handoff was prepared.
This branch is already checked out in the worktree; do not force it into the
primary checkout or reset either tree. Main was not merged/updated by this task.
Fetch and inspect current refs before relying on these snapshots. In a different
clone, fetch origin and select the continuation branch (create a tracking branch
only if it does not exist locally). Preserve any intervening user changes.

## Frozen donor and target boundaries

- Donor: `https://github.com/dvucinozd/Pajoniiir.git`.
- Original M2.2 foundation: `2c2ec32c253d368765123d7bbf8d37389b790b55`.
- New stable functional reference M2.5:
  `20f1c3f04a615209ae25e9bbdae649d0f5b44e8d`.
- Audited newer donor master: `5f41df1bfec994e325ec1ccdd4b3ebf4ad88061e`.
- Local donor checkout: `D:\Documents\DDJ-FFL4`; use pinned `git show`/diffs
  and preserve its independent changes/branch. Its contents were not modified.
- Stable M2.5, shared-P4 beta images and current master have different scopes.
  The parity plan names which later fixes supply useful regression contracts.
  Do not claim beta features or controller acceptance belong to stable M2.5.
- Targets are Guition **JC1060P470C_I_W_Y** and **JC8012P4A1C_I_W_Y**,
  owner-reported silicon v3.2; configured minimum revision 302. Boards remain
  in transit at handoff. All physical acceptance remains pending.

Native Rust/no_std and Slint remain the architecture. Existing shared host code
already covers two-deck control, Hot Cue journaling, media identity/generations,
sample-exact decoding/seek, loops, scratch, Master Tempo, mixer/FX and independent
MAIN/PFL routing. The audit identifies specific gaps; do not restart these
foundations or copy the C donor tree wholesale.

## Implementation order and acceptance

1. **RP-01/02: parser and profile compatibility.** First correct
   `firmware-rust/crates/ruzina-rekordbox/src/lib.rs` title lookup from index
   18 to 17. Prove it with independent 17/18/19 strings, UTF-16 and fallback/
   malformed fixtures. Preserve existing PQTZ 1..4 -> 0..3 conversion. Then
   complete S3CP v2/v3/v4 through parser, fixture compiler, registry and storage:
   CC7 scaling, NOTE_SELECT, bounded initial output SysEx, LED scaling and filter
   capability. Preserve old bytes and replay behavior, reject malformed/version-
   incompatible records and fence initialization to the current binding. See
   RP-02 for complete bounds; future upload validators must share the contract.
2. **RP-03/04/05 plus RP-15/18: transport/cues.** Implement CUE hold preview,
   release/return and PLAY commit; independent VINYL/CDJ actions; one load-lock
   admission/commit path, including PLAY/LOAD races; full loop cue recall,
   separate memory cues and source restore/local/tombstone precedence. Add
   pending input/LED reconnect fencing and relevant EXT/remote-tap/loop bounds
   regressions. Simulator-only load checks are not shared async admission.
3. **RP-06/07/08: Library/media.** Add ordered nested playlists, bounded artwork
   and fallback, optional PWV4 and predictable cold MP3 seek/duration discovery.
   Preserve sample-exact gapless/reservoir behavior and accepted-output progress;
   decoded duration must remain independent of catalog length/analysis span.
4. **RP-11/12/13: services.** Versioned durable settings, transactional write/
   migration policy, C6 fake-transport/Web contracts and signed A/B OTA with
   strict manifest/identity/TLS/retry/readiness/rollback behavior. Host fault
   injection comes before real networking/flash activation.
5. **RP-09/10/14/16/17: audio/platform.** Host descriptor/packet/clock/teardown
   work can proceed now. Actual UAC/DAC output and product-owner activation
   require exact BSP boot/memory/peripheral/power qualification. Retain later
   donor headphone gain, MT fallback and bounded cache/diagnostics regressions.
6. **RP-19/20 and other P1:** explicitly scoped extra controllers, absolute
   latching PFL, Pro DJ Link, extended UAC and ordinary folder/tag/sidecar work.
   These do not replace unfinished P0 acceptance.

Use separate reviewable commits/PRs for implementation batches. Run targeted
tests and the relevant shared/Slint/target checks before merge. The handoff push
does not merge this branch or authorize a firmware release. The audit defines
acceptance criteria; RP-01..20 remain open until their behavior is implemented
and verified. No test result should be inferred from a requirement table.

## Rename to Ruzina (2026-10-24)

The Rust product is **Ruzina**: crates `ruzina-*`, identifiers `ruzina_*`, the
firmware binary `ruzina-m1-firmware` and the repository `dvucinozd/Ruzina`.
**Pajoniiir** remains the name of the C donor (`dvucinozd/Pajoniiir`, M2.5)
wherever the text means the donor.

Owner decisions on persisted and wire names:

- The media identity domains Rust owns are now `ruzina.track.v1`,
  `ruzina.volume.fat32.v1` and `ruzina.volume.exfat.v1`. No Rust unit had
  stored identities yet, so nothing needs a migration.
- `pajoniiir.hotcue.v2` stays: it is the donor's domain
  (`media_identity.c`), which Rust uses to compute donor Hot Cue identities.
- The Web Remote hostname is `ruzina.local` (only), and the planned media
  sidecar directory is `/RUZINA`. Any donor `/PAJONIIIR` data is never
  overwritten.
- The OTA test bundles are signed for project `ruzina-m1-test`. Production
  projects are board-specific `ruzina-*` names.

Deliberately unchanged: donor URLs, commits and M2.5 quotes, `.ddjota`,
fixture provenance and fixture data (for example the `Pajoniiir test fixture`
vendor strings inside S3CP JSON).

Branding: the owner-supplied artwork is in `firmware-rust/assets/branding`
(emblem, wordmark and a splash composition, with SHA-256 records).

- The emblem is the fallback cover (34x34/40x40 RGB565, generated by a
  standard-library script).
- The M2.5 idle screensaver is ported: `ruzina-ui-model::idle` has the
  timing, wake and input rules (2 min default, playback and recording inhibit
  and count as activity, a wrap-safe clock, a physical wake press consumed,
  a remote mutation delivered).
- The simulator shows the splash on the landscape JC1060 and the emblem over
  the wordmark on the portrait JC8012, with two new reviewed goldens.
- The target display screensaver still needs a display owner and a derived
  RGB565 image.

A case-insensitive search for the old name may show only the donor
references listed above.

## Real Rekordbox export comparison (2026-10-24)

One owner export (Rekordbox 6, 189 tracks, no playlists) was copied off the
stick read-only and compared outside the repository. The Rust side is the
`export_inspect` and `analysis_inspect` examples in `ruzina-rekordbox`. The
M2.5 side is `rekordbox_pdb.c`/`rekordbox_anlz.c` from donor `20f1c3f0`,
built with gcc.

- **Fixed:** every Rust PDB page walk (tracks, names, playlists) followed a
  table past its `last_page`. Rekordbox links the last page to the table's
  empty candidate, then to page 0 and past the end, so the firmware import
  failed with `PageOutOfRange` on this export. `PdbPageHeader::next_in_table`
  now ends the chain at `last_page`, and index pages (`page_flags & 0x40`)
  yield no rows.
- **PDB:** all 189 tracks match M2.5 in title, key, BPM, duration, paths and
  cover. Rust additionally resolves 9 non-ASCII artists and the album table,
  which M2.5 drops.
- **ANLZ:** beat grids, waveforms, color previews and memory cues match for
  all 189 tracks, and hot cues for 188. In the last track, cue D exists only in
  the EXT PCOB; Rust imports it and M2.5 drops it.
- **Still open:** a real export with playlists, and physical USB import on the
  board.

The remaining RP-18 import edges followed the same day:

- A rejected EXT is discarded and the DAT analysis is kept.
- A cut-off trailing PWV4 degrades to PWV3.
- Names are bounded to their own row.

The real export is unchanged by them: no name row out of bounds, and the same
PDB and ANLZ results. A real EXT cut inside PWV4 or PWV3 matches the donor.

## Policies that must survive continuation

- **Headphones/CUE/PFL always use controller UAC.** A ready external stereo DAC
  takes MAIN priority, then compatible controller MAIN, then mute. MAIN switching
  must not reroute headphones. No onboard-codec/external-DAC headphone fallback.
- Keep APTA OFF in Guition/default product graphs. Preserve the original
  `codex/native-apta-consumer` branch; do not merge it. Already selected portable
  adapter/lifecycle work does not close native analysis or accuracy gates.
- `integration-check` only compiles owner combinations. It acquires no board
  resources, registers no heap regions and starts no product tasks. No Guition
  selector currently runs a complete DJ product runtime.
- Board-specific USB instances/roles, SD power/presence, journal partition layout,
  display/touch, DMA/cache and C6 transport need explicit qualified ownership.
  Do not reintroduce retired custom PCB/KiCad artifacts or hard-coded donor pins.
- JC8012 touch-payload distribution rights and exact panel/unit qualification
  remain open. Preserve licence/provenance for imported code, fixtures and assets.
- Recorder stays LATER/OFF; donor `ui_next` is a disabled demo prototype, not
  accepted Slint product scope. No other-board hardware result transfers here.

## Verification at handoff

The audit ran these existing Rust tests successfully at the unchanged firmware
source (`0f2fb1425`); only documentation has changed since:

```sh
cd firmware-rust
cargo test --locked -p ruzina-controller-profile -p ruzina-controller-session \
  -p ruzina-deck -p ruzina-rekordbox -p ruzina-audio-runtime -p ruzina-mixer
```

**315 passed, 0 failed, 0 ignored.** This is not an independent M2.5 differential
suite and does not close the discovered title/profile/transport gaps. Test log
is retained locally outside the repository at
`C:\Users\Daniel\.codex\worktrees\m1-guition-bsp\research\m25-audit-tests.log`.
The audit also verified all eight pinned donor source links against Git objects,
20 RP items, unique requirement IDs, local Markdown targets and whitespace.
For this handoff, the local documentation checker passed all 70 links. No CI
result is claimed for the branch-only documentation push.

Recheck documentation before commit:

```sh
python3 firmware-rust/tools/check_docs.py
git diff --check
git diff --cached --check
```

Use committed Rust 1.95.0 and lockfiles. Full host/Clippy/UI, runtime-owner and
P4 checks are listed in [validation](VALIDATION.md) and the two CI workflows.
They were not rerun for this documentation-only handoff; neither were donor
builds or physical procedures. Hosted CI must be inspected per actual run;
unstarted billing-limited jobs are not PASS.

Physical first boot/recovery, factory backup/readback, flash/PSRAM, display/touch,
USB/SD power and reconnect, UAC/DAC timing, latency, listening, OTA and soak on
both boards remain NOT RUN. M2.5's donor soak waiver never transfers to Rust.
