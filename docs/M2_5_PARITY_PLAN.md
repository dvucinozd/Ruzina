# M2.5 parity audit and Rust development plan

Reviewed: 2026-10-22. Status: **planning baseline; implementation gaps remain open**.
Targets remain Guition JC1060P470C_I_W_Y and JC8012P4A1C_I_W_Y, silicon v3.2.

## Scope and pinned references

The review compares donor history, changed product modules, controller tooling,
tests, release/qualification records, current master and retained contributor
branches against the current Rust crates, simulator and requirements. It does
not treat every donor file or passing donor test as Rust parity evidence.

| Reference | Exact source | Role |
|---|---|---|
| Original M2.2 | `2c2ec32c253d368765123d7bbf8d37389b790b55` | Historical rewrite foundation |
| Stable M2.5 | `20f1c3f04a615209ae25e9bbdae649d0f5b44e8d` | New released functional reference |
| shared-p4-beta.2 tag | `d259a71ae716217a6448c374bf52c2c798d67ce8` | Separate delivery; retained images have their own source/configuration identities |
| Donor origin/master | `5f41df1bfec994e325ec1ccdd4b3ebf4ad88061e` | Later fixes and development candidates, not the stable release |
| Rust origin/main | `0f2fb1425ef76f2541a8c76841cfa52178dca589` | Audited implementation before this documentation update |

Fetch/tag inspection and GitHub release listing confirm M2.5 is Latest and both
shared-P4 releases are prereleases. M2.2..M2.5 contains 184 commits and changes
485 files; M2.5..reviewed master contains 294 commits and changes 1,506 files.
Counts include documentation, tests, assets and refactoring, not feature counts.
M2.5 source paths below use `firmware/main-deck-p4/components/`; later master
mostly uses `firmware/p4-core/components/`.

Stable M2.5's physical release scope is JC4880/DDJ-FLX4. Its final 180-minute
soak was explicitly waived, not passed. JC1060, DDJ-400 and real Pro DJ Link
peers were not qualified by that release. No donor acceptance establishes
acceptance on either new Guition v3.2 board. See the pinned
[M2.5 release record](https://github.com/dvucinozd/Pajoniiir/blob/20f1c3f04a615209ae25e9bbdae649d0f5b44e8d/docs/validation/M2_5_RELEASE_20261006.md).

## Stable behavior to retain or complete

`P0` below means required R1 functional/reliability work. `P1` means optional
scope after P0; a feature present but disabled/unqualified in the donor does not
automatically become an R1 requirement. Status describes inspected Rust code;
existing tests passing does not close a newly identified gap.

| Work item | Current Rust evidence / gap | Required delta and acceptance | Requirements |
|---|---|---|---|
| RP-01 P0: PDB titles and beat numbering | **Host-implemented 2026-10-22.** Title reads slot 17. Fixtures written from raw slot offsets cover distinct 17/18/19 strings, UTF-16 with surrogate pairs, absent/empty title -> filename (never slot 18) and catalog conversion. As in M2.5, a malformed or non-UTF-8 title (offset, DeviceSQL or UTF-16) falls back to the filename in both the page and streamed USB readers; path fields and storage I/O errors stay fail-closed. Simulator check: imported PQTZ bar 1 renders the tall downbeat tick. PQTZ 1..4 -> 0..3 conversion unchanged. | Real-export comparison remains with RP-06/18. Physical USB import not run. | 050, 052, 065 |
| RP-02 P0: S3CP v2/v3/v4 end to end | **Host-implemented 2026-10-22; target compile-only.** Parser/runtime accept exactly v2/v3/v4 with version-gated CC7_TO14, NOTE_SELECT, FILTER_ALWAYS, init SysEx (F0..F7, <=128 B, inside size/CRC) and v4 LED `value_scale`; v2/v3 reserved fields stay ignored. Fixture compiler is the pinned M2.5 tool; it reproduces both v2 golden binaries byte for byte and froze v3/v4 fixtures. Registry metadata includes version/CRC/SysEx length; init-SysEx profiles enter `Initializing` and only the still-current ticket opens MIDI/LED. USB worker sends USB-MIDI SysEx packets with a per-packet binding recheck; failure closes activation as FAILED. Channel filter now follows the M2.5 policy: audible only while Smart CFX is on unless the fully active profile declares FILTER_ALWAYS (the curve still follows Smart CFX only); detach/initializing/older profiles restore the gate. | No upload path exists yet; it must reuse `Profile::parse`. Real controller SysEx/LED and filter listening acceptance not run. | 030..037, 101 |
| RP-03 P0: CUE hold and VINYL/CDJ | **Host-implemented 2026-10-22; firmware wiring compile-only.** Shared deck CUE press/release follows M2.5 B12: playing (or PLAY pending) returns to cue; paused away from cue sets the current/quantized cue; paused at cue previews while held, release returns, PLAY during preview commits; duplicate presses/unmatched releases do nothing, refused or post-release preview acknowledgements cannot restart it. Disconnect releases an active preview through atomic transport effects; track load/unload clears CUE ownership. Per-deck idempotent `JogVinyl`/`JogCdj` (profile ext actions 8/9): CDJ ignores platter touch and releases an active scratch on switch; mode survives track load. CUE and VINYL touch claim the RP-04 load gate. Simulator touch CUE uses press/release/cancel/focus loss. | Rust still has no tempo nudge or paused jog scrub in either mode (M2.5 B13a); CDJ deltas are no-ops until that audio work exists. No touch jog-mode selector or persistence. Web Remote needs explicit tap semantics. Physical CUE feel and preview audio not run. | 010, 013, 014, 035, 100 |
| RP-04 P0: load admission | **Host-implemented 2026-10-22; firmware wiring compile-only.** `DeckProductState::load_admission` is the one default-on LOAD LOCK verdict (playing, pending PLAY, jog/scratch ownership; explicit lock-off allows any load) and `commit_track_load` rechecks it with a typed `LoadRefusal`. A lock-free per-deck `DeckLoadGate` gives single-flight permits: controller/touch Library loads reserve at admission, PLAY/jog touch withdraws an unstarted reservation (old session kept, `Superseded`), the worker calls `begin_replacement` immediately before `Prepare` releases the old source, and transport claims are refused while replacement runs. Simulator and firmware share this path; reasons reach service events and product counters. | Web Remote does not exist yet and must call the same admission. CUE hold-preview ownership joins the claim set with RP-03. Lock switch is not persisted or exposed in UI. Real touch/controller race timing not run. | 010, 021, 043, 067, 100 |
| RP-05 P0: memory cues and loop hot cues | **Host-implemented 2026-10-22; firmware wiring compile-only.** ANLZ PCOB list 0 imports up to 16 memory points/loops in export order, separate from the eight slots, with an explicit truncation flag; malformed/slotted entries or a duplicate list reject the DAT transaction. Loop hot-cue recall publishes one atomic Seek + SetLoop (single cue: Seek + ClearLoop when a loop is active), keeps play state and clears loop-adjust. Explicit `RestoreSourceCues` (profile ext action 7) persists an override-free bank; source/local/tombstone precedence, reload, reboot record round-trip, second-deck refresh and cross-track rejection are covered. Nothing else drops local edits. | Memory cues are imported data only: no memory-cue view, recall or LED yet (RP-07). No touch restore control. Loop bounds against decoded duration are now enforced (RP-18). Real Rekordbox memory lists not run. | 013, 044, 050..053, 082 |
| RP-06 P0: Library hierarchy | **Host-implemented 2026-10-22; firmware wiring compile-only.** PDB playlist tree (table 7: parent @0, sort @8, id @12, folder @16, inline name @20) and entries (table 8: index @0, track @4, playlist @8) import into a bounded index (256 nodes, 8192 entries, 96-byte names) validated against the same catalog publication. As in M2.5, malformed, duplicate-id, unreachable (missing or non-folder parent, self/cyclic chain) and dangling (folder target, unknown track) rows are dropped and counted and never fail the track catalog; a reached bound or broken playlist page chain marks the hierarchy truncated. Siblings order by sort order, name, id; entries by entry index then export order; repeated tracks stay. `LibraryBrowser` resolves rows, LOAD and BACK only for the catalog revision and playlist index it was built on. Firmware: LOAD on a folder/playlist opens it without load admission (M2.5), LOAD on a playlist track uses the RP-04 path, SHIFT+BROWSE press is PLAYLISTS/BACK. | No Slint/touch hierarchy view yet (the simulator demo library has no catalog), no artwork table (RP-07). SHIFT+BROWSE as BACK is a Rust mapping; the donor's controller only shows the Library and uses a touch PLAYLISTS button. Ordinary folder libraries and editable playlists stay P1. Real export comparison not run. | 040..046, 048, 050, 065 |
| RP-07 P0: artwork and analysis presentation | Neutral preview/detail and PQTZ/PCOB are implemented. No bounded JPEG artwork owner, artwork references, optional PWV4 or memory-cue view. | Bounded artwork read/decode and PSRAM budget, cancellation on deck/media change, missing/corrupt-cover fallback with attributed Pajoniiir logo; never publish an old cover onto a new track. Import optional PWV4 color preview, retain PWV3 timing at 150 Hz independent of decoded duration, display imported key/cue types. Missing optional assets degrade safely. | 012, 050..053, 062..067, 084 |
| RP-08 P0: duration and cold MP3 seeks | WAV/FLAC/MP3 decoding, exact VBR seek, EOF drain, loop resize/prefix, recent and loop-origin MP3 decoder checkpoints already have host tests. MP3 retains only three checkpoints; a cold late seek may decode from the start and no-Xing length starts unknown. | Preserve correctness and generation/accepted-output accounting. Add a bounded complete/sparse MP3 indexing or equivalent solution for predictable cold-seek latency and actual file duration; distinguish unknown/exact duration from PDB length and analysis span. Cancel/yield I/O outside audio; test tagged/untagged CBR/VBR, wrong PVBR, cold late seek, EOF and competing deck reads. Do not copy the C index if it loses Rust codec reservoir/gapless correctness. | 011..014, 019, 020, 070..075 |
| RP-09 P0: physical UAC and MAIN clock | Generic USB crate currently classifies interfaces/endpoints; audio routing/reference-sink contracts do not implement a UAC stream. | Descriptor-validated format/channel/clock capability, actual PCM16/packed24 packet bytes, fractional 44.1/48 kHz cadence, optional control requests, bounded producer backpressure, accepted-frame receipts and transfer/teardown epochs. Start with explicit supported formats; reject unsupported audio while keeping MIDI usable. EP0 timeout must not free a transfer still owned by USB. Qualify clock drift and MAIN/PFL separation for each controller/sink. | 039, 070..076 |
| RP-10 P0: board-owned USB roles and sink changes | Guition host-power/FS mapping and physical sinks remain unqualified; owner modules are compile-only. | Assign storage/controller roots, PHY and FIFO budgets in each qualified BSP, not from enumeration order or donor root numbers. Primary storage is the qualified HS path. External DAC detection means ready/initialized capability, not an I2C ACK on an arbitrary device. MAIN change/removal needs a bounded mute/drain/rebind policy under PLAY/LOAD/scratch, with no stale completion credit and no headphone fallback. | 002, 004, 040, 047, 073..075 |
| RP-11 P0: durable settings and identity | Hot Cue journal, stable volume/path identity and legacy identity helpers exist. General settings and transactional writable-media workflows remain unfinished. Rust identity/record formats are not automatically donor NVS/cache byte-compatible. | Versioned settings, atomic network/OTA configuration with CRC, publish RAM only after durable success, clear/tombstone and interrupted migration tests. Specify explicit import/migration policy for donor cue/cache data; retain full identity verification and reject collisions/stale records. Do not adopt C raw structs or silently reinterpret existing Rust IDs. | 044, 045, 080..082; P1-003 |
| RP-12 P0: network/Web lifecycle | C6 transport and Web Remote are unfinished; current simulator controls already use product semantics. | Bounded C6/network interface, Wi-Fi off/on without resetting shared SD ownership, coherent catalog/deck/status generations, stale HTTP response rejection, latest-value control ordering and a MAIN meter from accepted final output with decay. Preserve Host/Origin/request-marker protections and bounded uploads; redact credentials. Host fake transport first, then actual C6 qualification. | 047, 081, 083..085, 100..103 |
| RP-13 P0: OTA/recovery | Requirements/ADR exist, no implemented Rust signed A/B OTA subsystem. | Strict complete JSON/duplicate-key/depth/size validation; signed header/manifest and app/board/project/version/size/SHA binding; newer-only pull with comparable versions and expiring offer; bounded header/body/network retries and phase errors. Preserve verified TLS CA/hostname handling including cross-signed chains. Local signed service rollback is a separate policy. Candidate readiness must include requested networking and critical tasks, time out, and physically roll back an unconfirmed image. Interrupt every write/transfer stage in host tests before board testing. | 085, 090..094, 101..103 |
| RP-14 P0: resource and regression gates | Bounded service health, queue/read failures, UI goldens and target checks exist. No physical task/driver/DMA timing or per-board product-runtime budget is qualified. | Low-priority precomputed diagnostics: no heap walks, formatting or decoder locks in deadlines. Record internal/DMA largest/free reserves, PSRAM/stack high-water marks, reset/image/startup identity, active-vs-idle UAC faults, USB loss/recovery and dropped logs. Qualify DMA block geometry, cache operations and scheduler starvation; run exact-image mixed-rate MT/loop/scratch/FX/MAIN/cue plus removal/reconnect/OTA listening matrix. | 020, 070..075, 083..085, 091..093 |

All numeric references in the final column abbreviate `M1R-P0-xxx` unless a P1
prefix is explicit. Source/acceptance additions are tracked by RP IDs; they are
not completion claims for the parent requirements.

## Later master repairs worth adopting as contracts

These changes are newer than stable M2.5. Incorporate their regression behavior
where relevant during P0 work, with separate provenance. Do not claim that a
stable M2.5 image contains them.

| Work item | Donor change / Rust disposition | Closure |
|---|---|---|
| RP-15 P0: reconnect fencing | **Host-implemented 2026-10-22.** `ControllerSession::set_connected(false)` now retires the old connection's unaccepted discrete/continuous scheduler work, its downstream retry and any partial reconnect snapshot (counted in `retired_on_disconnect`; scheduler loss history is kept). Only held-state releases and the disconnect notification survive. Each in-flight item records the connection generation it was taken under; a late completion from an older generation cannot recreate a retry or advance the new snapshot cursor. Tests cover A->detach->B with queued PLAY/LOAD/jog, a failed PLAY retry, a partial snapshot and a late in-flight failure. Firmware LED output and init SysEx were already fenced by `controller_epoch`/activation ticket. | No physical hot-plug or wake-consumption timing run; deferred profile activation across a real A->B swap is covered only by the existing epoch checks. |
| RP-16 P0: headphone gain ownership | `605bcd20`: dedicated FLX4 headphone PCM was attenuated a second time, losing 12 dB. Current Rust mixer already has an independent headphone render path; no physical packer exists. | At future UAC boundary apply headphone mix/level exactly once, test distinct MAIN/PFL samples, silence/isolation and full-scale packing. Any MAIN-mirror calibration stays separate and controller-specific; do not add a blanket gain boost or copy the old attenuation. Physical listening/level acceptance remains mandatory. |
| RP-17 P0: MT fallback and bounded cache | `2fa22631` reanchors MT after consumer fallback; `1556e8d7` bounds task cache maintenance; `3459a97a` avoids TLSF walks in runtime observations. Rust source resets/rebases keylock but has different owners/drivers. | Compare starvation->fallback->MT resume under loops and accepted-output clocks; add missing behavioral regression, not IDF lock code. Set per-operation/cache/DMA budgets for Guition and measure interrupt gaps once hardware exists. |
| RP-18 P0: source and transport edges | **Host-implemented 2026-10-22 (except remote tap).** PDB artist (subtype 0x60, id @4, name ref @9) and album (subtype 0x80, id @12, name ref @21) rows now use their own layouts with near u8 or far u16 name offsets; the old shared layout read album ids/names from the artist offsets. DeviceSQL 0x40 long-ASCII strings are no longer read as empty. EXT may enrich DAT with PCOB/PCPT cues: an identical slot is idempotent, a different cue in an occupied slot rejects the import, memory cues de-duplicate. Loop admission against the decoded duration (`frames*1000/rate`, set on load) refuses out-of-range loop OUT/reloop/resize/size/beat-loop before any deck state changes; a loop hot cue beyond the end recalls as a single cue, so UI and audio cannot diverge and a failed SetLoop no longer unloads the deck. | Remote CUE tap waits for the Web Remote (RP-12). Truncated trailing PWV4 tolerance and bounding name refs to the row end are not ported; an EXT failure still fails the whole import (strict Rust policy; the donor keeps DAT-only data). Real-export comparison remains open. |
| RP-19 P1: extra controllers / absolute PFL | Post-beta2 `47b322fd` + `abffb203`: Numark latching PFL requires idempotent `pfl_set` and clearing controller-owned level on disconnect. Rust `Pfl` only toggles on press. Hercules/S2/DDJ-400 also have later mappings/capability changes. | Add explicit absolute-set semantic action, bounded durable latest-state delivery and lifecycle cleanup while preserving legacy toggle behavior. Version/capability-check profile semantics, not just binary format. Use final master profiles/fixtures; qualify each real controller's MIDI/LED/UAC separately. Move the contract earlier if a latching controller becomes an R1 target. |

The later donor supports some FS feedback and HS UAC2 cases that M2.5 rejected.
Those formats, S2 USB port swaps and native headphone mixer/volume capabilities
are P1 extensions to RP-09, requiring descriptor captures, controller-specific
gain ownership and host/FIFO budgets. Descriptor acceptance, software fixtures
and contributor listening reports have different scopes. DDJ-400 and repaired
Numark support must not be inferred on our arriving boards.

## Optional work and explicit exclusions

| Area | Disposition |
|---|---|
| Pro DJ Link discovery/claim/DBServer/NFS/download/cache/sync | **RP-20 P1, after local playback/networking.** M2.5 already contains software modules but real peers were unqualified. If selected: provider-neutral network media, full source/peer/volume/track identity, bounded/cancellable download, manifest-last SD commit, pinned cache entries and peer epochs; loss holds tempo rather than periodic reseeks. Gate Ethernet/latency separately on JC1060; JC8012 has no qualified equivalent path. No automatic P0 scope expansion. |
| SD diagnostics / recorder | Keep recorder LATER and OFF. Port bounded I/O/admission/power-loss lessons when writable media is implemented. SD power sequence/idle errata must be derived for native esp-hal and exact BSP, not copied IDF hooks. |
| Native APTA | Existing selected portable ownership work remains. Accuracy/upstream/embedded gates are separate; default and Guition stay OFF and original integration branch stays unmerged. Donor ancestry does not authorize merging our APTA branch. |
| New `ui_next` | Disabled donor prototype with demo data and no production bindings/touch. Use ideas only after Slint product review; do not port LVGL or make recorder/four-deck controls release requirements. |
| Retained contributor branches | `numark-dj2go2-touch`, `codex/remaining-branch-integration`, `codex/jc-board-preparation`, `codex/jc8012-source-research` retain work already adapted/reviewed on master. `kay/jc1060-hw-bench` supplies scoped contributor evidence, not our hardware acceptance. `jc8012p4a1-v3` is not wholesale merged: touch firmware rights/PPA isolation and board-specific Wi-Fi/SD changes remain separate. |
| Board/framework internals | Keep native Rust/no_std, Slint, selected v3.2 BSP boundaries and controller-only headphones. Do not import JC4880/M3 pinouts, older-silicon GPIO assumptions, C runtime/FreeRTOS, IDF USB/cache patches, old PCB/CAD, partition addresses or a whole donor tree. Existing board source/license gates remain open. |

Any copied/adapted code, fixtures, artwork or fonts must retain upstream
provenance/licences. MIT project licensing does not establish distribution
rights for the JC8012 touch payload or third-party binary assets.

## Execution order

The next software work can proceed while the boards are in transit:

1. **Parser/compatibility batch:** RP-01, then RP-02. Freeze independent M2.5
   fixtures and v2/v3/v4 ABI/tooling tests first. Do not regenerate expectations
   from the same Rust constants being tested.
2. **Transport batch:** RP-03/04/05, plus RP-15/18 failure cases. Route simulator,
   controller and future Web through the same admission/commit path. Preserve
   existing audio-session and durable Hot Cue containment tests.
3. **Library/media batch:** RP-06/07/08. Add real-format synthetic and permitted
   real-export comparisons, explicit bounds, cancellation, load/seek latency
   budgets and cross-media/cache tests. Keep decoded time independent of analysis.
4. **Product services batch:** RP-11/12/13. Implement settings, fake-network Web
   contracts and signed OTA/boot state machines on host before actual C6/flash.
5. **Audio/platform batch:** host descriptor/packet/clock/teardown work for
   RP-09/10/14/16/17 can start now. Activate product owners only after each
   exact BSP's boot, memory, power and peripheral gates. DAC removal/switching
   and controller detach never reroute headphones.
6. **Optional extensions:** RP-19/20, ordinary folder/tag/sidecar work and any
   additional UAC format/controller selected for the product. They must not
   obscure incomplete P0 work or silently become support claims.

The [board arrival sequence](DEVELOPMENT.md#board-arrival-sequence) continues in
parallel. `integration-check` remains compile-only. Each implementation batch
requires its targeted regressions and relevant workspace/UI/target checks;
physical acceptance, signing and public release require their own exact-image
records. A release has no inherited soak waiver from the C donor.

## Verification of this audit

Donor origin and tags were fetched; release identities and current master were
read without changing its files. Rust checks at the audited source passed:

```sh
cargo test --locked -p pajoniiir-controller-profile -p pajoniiir-controller-session \
  -p pajoniiir-deck -p pajoniiir-rekordbox -p pajoniiir-audio-runtime -p pajoniiir-mixer
```

Result: **315 tests passed, 0 failed, 0 ignored**. These are existing targeted
tests, not a new differential M2.5 corpus. This documentation change does not
close RP-01..20, change runtime behavior or establish hardware results. The full
workspace, target matrix, donor builds and physical procedures were not rerun
for this documentation-only audit. Local Markdown links and whitespace were
checked separately.

## Source map

Use the frozen tree when creating independent Rust fixtures. Preserve the source
commit in every adapted fixture, and record intentional differences explicitly.

- [Stable integration ledger, packages A-L](https://github.com/dvucinozd/Pajoniiir/blob/20f1c3f04a615209ae25e9bbdae649d0f5b44e8d/docs/FORK_IMPROVEMENTS.md): RP-01..14 and optional Link/SD.
- [S3CP schema v2-v4](https://github.com/dvucinozd/Pajoniiir/blob/20f1c3f04a615209ae25e9bbdae649d0f5b44e8d/docs/CONTROLLER_PROFILE_SCHEMA.md): parser/runtime, compiler, storage, upload and initialization contract.
- [Transport/media closure](https://github.com/dvucinozd/Pajoniiir/blob/20f1c3f04a615209ae25e9bbdae649d0f5b44e8d/docs/validation/FORK_IMPROVEMENTS_PACKAGE_B_SOFTWARE_20261004.md): RP-03/04/05/08 and tests `audio_track_length`, `audio_seek_skip`, `audio_cue_preroll`, `deck_load_lock`, `deck_core_dual`.
- [UAC/sink closure](https://github.com/dvucinozd/Pajoniiir/blob/20f1c3f04a615209ae25e9bbdae649d0f5b44e8d/docs/validation/FORK_IMPROVEMENTS_PACKAGE_F_SOFTWARE_20261004.md): RP-09/10; actual stream harness in `tests/controller_usb_audio`.
- [Post-M2.2 hardening](https://github.com/dvucinozd/Pajoniiir/blob/20f1c3f04a615209ae25e9bbdae649d0f5b44e8d/docs/validation/IMPLEMENTATION_REPORT_M2_2_REVIEW_20260923.md): identity, parser, Web ordering/meter, atomic settings and strict OTA; RP-11..14.
- [Pinned current source tree](https://github.com/dvucinozd/Pajoniiir/tree/5f41df1bfec994e325ec1ccdd4b3ebf4ad88061e): RP-15..19 implementation commits named above; `tests/audio_fw_runtime`, `controller_runtime`, `controller_led_runtime`, `deck_core_dual`, `controller_usb_audio` and `usb_storage` provide candidate regressions.
- [Later branch integration boundaries](https://github.com/dvucinozd/Pajoniiir/blob/5f41df1bfec994e325ec1ccdd4b3ebf4ad88061e/docs/reviews/2026-10-19-branch-integration.md): repaired Numark semantics, disabled UI prototype and retained contributor work.
