# Rust M2.5 development handoff

Updated: 2026-10-22. Next action: **RP-05 memory/loop cues and source restore**.
Host-implemented stack: RP-01/02 (PR #23, `ogranak/m25-rust-parity-plan`),
RP-04 load admission (PR #24, `ogranak/m25-rp04-load-admission`) and RP-03
CUE hold/VINYL-CDJ (`ogranak/m25-rp03-cue-jog`), each stacked on the previous.
See the parity plan rows for evidence and open items (upload path, Web Remote
admission/tap semantics, tempo nudge/paused scrub, physical controller/import/
listening acceptance). The sections below still describe the original baseline.

## Read first and locate the checkout

1. [M2.5 parity audit and acceptance plan](M2_5_PARITY_PLAN.md): 20 open work
   items, pinned sources, implementation evidence, priorities and source links.
2. [Development status](DEVELOPMENT.md) and [product requirements](M1_RUST_PRODUCT_REQUIREMENTS.md).
3. [Architecture](M1_RUST_ARCHITECTURE.md), [validation](VALIDATION.md),
   [controller ADR](adr/ADR-004-controller-profile-platform.md) and
   [OTA ADR](adr/ADR-007-ota-boot-recovery.md).
4. [Board matrix](boards/GUITION_HARDWARE_MATRIX_2026-10-20.md) and
   [first-boot procedure](boards/GUITION_FIRST_BOOT_2026-10-20.md) before hardware work.

Repository: `https://github.com/dvucinozd/Pajoniiir-M1.git`.
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
   `firmware-rust/crates/pajoniiir-rekordbox/src/lib.rs` title lookup from index
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
cargo test --locked -p pajoniiir-controller-profile -p pajoniiir-controller-session \
  -p pajoniiir-deck -p pajoniiir-rekordbox -p pajoniiir-audio-runtime -p pajoniiir-mixer
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
