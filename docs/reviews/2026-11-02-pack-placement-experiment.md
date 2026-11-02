# Bounded packetizer placement experiment

Continues the [peer continuity repairs](2026-11-02-peer-continuity-fixes.md)
and [cache/compute plan](2026-11-02-uac-cache-compute-follow-up.md).
Historical preparation status: OFFLINE_IMPLEMENTED / NOT_FLASHED.
The later [completed device result](2026-11-02-pack-placement-device.md) supersedes
that execution state: both timing gates fail; exact s07 is restored/confirmed.
Native minimum 302, rev103 IDF backend, gain, DSP, priorities and all physical
recovery requirements remain unchanged. No COM3 access is part of this batch.

## Audit decision

Pinned IDF `fff9895c` rev1 `soc/cache_reg.h` exposes distinct L1 instruction
bus0/bus1 enable/clear bits, one shared L1 data-cache with bus0/bus1 counters,
and counter-overflow status/interrupts. The header maps instruction buses to
ICache0/1, but does not prove that each measured count belongs exclusively to
the USB task. Pinned P4 `cache_ll.h` does not expose a counter sampling API;
the supplied perfmon component is Xtensa-only. Searches of component C/assembly
sources found no consumer of these L1 access-count control/register names.
That is a scoped source search, not proof that ROM cannot change them.

Enabling and clearing shared MMIO counters without a qualified ownership,
overflow and core/bus contract would introduce ambiguous attribution. Direct
cache-counter instrumentation is therefore not selected for this campaign.
Cycle CSR readings alone cannot distinguish cache stalls from computation or
IRQs. All existing wall-time/scheduler/IRQ caveats still apply.

Instead, change only link placement of the exact pack/closure/write functions.
The previously inspected valid path calls ROM memset (0x4fc00268 in that ELF)
and these three functions. Panic/bounds helpers remain in flash; this variant
is not a cache-disabled-safe call graph. Fresh generated code must be inspected
again; old addresses and sizes are not new-image evidence.

## Implementation contract

`build_legacy.py --variant deck-engine --uac-profile --uac-pack-iram` explicitly
opts in. Without both profile/variant gates it is rejected. There is no Rust
code/feature change: the same compiled Rust archive supplies both profiles.
The tool derives three exact section renames from actual archive member names
and the three exact mangled symbols. It preserves the original Rust archive
and uses objcopy on a separate link copy to place those sections in IDF's
`.iram1.*` input scheme. Missing/duplicate symbols, merged sections or combined
function code above 2048 bytes reject the build. No archive wildcard or
whole-graph IRAM placement is allowed.

The generated ELF must place all three in `.iram0.text` for the variant and
`.flash.text` for the control. The manifest identifies the placement flag and
hashes the relocation list, linked archive copy, placement table and function
disassembly. Normal builds link the original archive. CI adds a separate
explicit build-only job.

The first pilot attempted an IDF linker fragment and failed parsing: pinned
ldgen mapping object names do not accept dots, and symbol identifiers cannot
express Rust's closure name. Its log/package directory is retained as a failed
pilot. The separate-copy section rename avoids patching global IDF or Rust
source. Archive disassembly/relocations must compare identically after only
normalizing the three section names; archive container bytes need not match.

## Pair acceptance before registration

Require clean published source and CI, then fresh normal/control/variant
packages. Compare control/variant Rust archive SHA-256 and sdkconfig bytes;
both must be identical. Verify source and artifact hashes, exact function and
callee placement, whole ELF section/RAM deltas, logger/queue/stack sizes and
unchanged graph placement. Existing PCM/transport/recovery tests apply to the
unchanged archive. A link/layout change still needs physical qualification.

Record >=6000 us rejection and <=3000 us headroom target without relaxation.
Use at least three equivalent cold observations per image and separate steady
idle windows; no controls, loads or playback in this cause campaign. Preserve
both calibrated attribution paths and record observer cost. Decide once:
repeatable timing change supports an instruction-placement effect, not proof
of a particular cache miss count; no repeatable effect is inconclusive/negative.
Do not warm the product fill or add a second simultaneous variable.

Current history has 33 predecessor plans and reconstructed full state
`4324fee5c109cc9fe28aebbafa8f5cf2116ee4950afce374be5ef6f81aa2ae68`.
No new write plan is registered by this source change. Before physical work,
archive secondary copies and register every transition and return, then require
fresh manual ROM entry, identity/security/RDID and full predecessor equality.
Restore/readback/cold-boot/listen to accepted s07 before ending an unqualified
campaign. I02–I13 remain separate NOT_RUN gates.

## Offline verification completed

Source `605abe78f596dc7f94ccaf04e5e0936ac25a4750`, [PR #78](https://github.com/dvucinozd/Ruzina/pull/78)
above [peer fix PR #77](https://github.com/dvucinozd/Ruzina/pull/77).
Exact-parent CodeRabbit review: 0 issues. Fresh Rust profile: 69 PASS;
Windows Python: 40 PASS and six explicit C skips; all six actual C cases PASS
in WSL. Five placement tests cover exact selection, missing/duplicate/oversized
layout, undefined references, wrong target sections and instruction drift.

Both clean comparison builds have empty dirty status, matching 284 source
hashes each and all 22 combined artifact hashes. Their original Rust archive
SHA-256 is identical:
`70f8e038e66b53d4b1f739a599f8cfdac21068c6db4c859b77a3c4c76f335ebb`.
Their sdkconfig files are byte-identical. The control/variant app hashes are:

- Flash control: `0718c73af20982719904070fc14fcd4c3778ce2924e3798e8a1d55c01b79a4ea`.
- IRAM variant: `b38488e5b1987398e0d9a095357547d14b87eac5d36008e8eab65d2179618ec6`.

Exactly 704 function bytes (316/304/84) move from flash to IRAM; IRAM end
alignment adds another 64 bytes. `.dram0.data` and `.dram0.bss` are unchanged.
Direct Rust stack estimates are 3120 fill / 73616 load bytes for both images;
they exclude indirect/C/kernel frames and do not establish runtime high-water.
Graph stays in `.flash.text`. Fresh IRAM call inspection finds internal pack
calls and ROM memset at 0x4fc00268; bounds/panic helpers remain in flash at
0x40058aa4/0x40058668/0x40058d08. No cache-disabled safety claim is made.

The [pair index](evidence/2026-11-02-pack-placement/pair-verification.json)
records section deltas, exact symbols, hashes and stack estimates. Its ELF
`object` label is not an archive-member identity; use symbol/section plus the
separate generated relocation list for placement authority.
The [archive index](evidence/2026-11-02-pack-placement/archive-index.json)
records the 16750017-byte ZIP, verified CRC/member hashes and identical
secondary copy. Archive SHA-256:
`37297bac9ff82c26222302f0319d84dd02133868b4985a8b38ed64ea2c3ce1a2`.
Primary root: `D:/BoardEvidence/JC1060-20261030-COM3/pack-placement-20261102/`;
secondary: `C:/Users/Daniel/Documents/BoardBackups/JC1060-20261030-COM3/pack-placement-20261102/`.

GitHub CI is separate from these local results and must complete on the
published head before any physical registration. No new recovery plan, COM3
access, target timing or playback acceptance is claimed. The next action is
CI verification, then registration of the full controlled campaign with 33
predecessors and s07 return plans; owner-assisted ROM entry follows registration.

## Registered campaign, operator entry still open

On continuation, final published head `f0ba8caedba35692384f6f80c0796add448e8cd2`
passed **21/21 jobs**: [host #542](https://github.com/dvucinozd/Ruzina/actions/runs/41462344609)
and [legacy #65](https://github.com/dvucinozd/Ruzina/actions/runs/41462344693),
including the explicit IRAM placement job. No failed/skipped job is counted.

The [registration](evidence/2026-11-02-pack-placement/campaign-registration.json)
freezes four paths with complete expected before/after flash reconstructions:

| Path | Required predecessors | App |
|---|---:|---|
| [control](../boards/readiness/jc1060-pack-control-write-recovery-plan-20261102.json) | 33 | Flash packetizer control |
| [control-return](../boards/readiness/jc1060-pack-control-return-write-recovery-plan-20261102.json) | 34 | Accepted s07 if control fails |
| [iram](../boards/readiness/jc1060-pack-iram-write-recovery-plan-20261102.json) | 34 | IRAM variant directly after control |
| [iram-return](../boards/readiness/jc1060-pack-iram-return-write-recovery-plan-20261102.json) | 35 | Accepted s07 after variant |

The two 34-predecessor paths are alternatives, not consecutive writes. If the
control return is executed, do not run the registered IRAM entry: it expects
the control image and must reject the restored state. Future resumed work then
requires a new ancestry-aware entry plan. No alternative transition is inferred.

Four original factory backups match. All 33 historical plans were replayed
with exact restore bytes and preserved sector tails; current full state remains
`4324fee5c109cc9fe28aebbafa8f5cf2116ee4950afce374be5ef6f81aa2ae68`.
All candidate sources/artifacts were reverified. Every new plan and payload
has a byte-identical secondary copy. The [registration archive](evidence/2026-11-02-pack-placement/registration-archive.json)
passes CRC/member-hash/secondary checks: 3144400 bytes, SHA-256
`6c5ab5c903883ad7e9bf765f0be307799a15f31947630cc710b0e09c984db0f5`.
This supplements the unchanged package-pair archive; neither is overwritten.

`program_campaign.py --mode <path> --check-only` verifies plan/package/source,
four backups and the full reconstructed predecessor, and exits before any
serial access. [All four offline preflights pass](evidence/2026-11-02-pack-placement/offline-preflight.json).
Without that flag the guarded programmer requires fresh manual ROM entry,
rev103/MAC/security/RDID and all 16 MiB predecessor equality before any write,
then checks all written spans. It never requests a reset or eFuse operation.
The passive `monitor_campaign.py` sends no bytes and validates COM3 USB identity.
Exact script hashes are registered; scripts and raw records stay external.

Registered measurements: three cold boots per image, 45 seconds after each
expected ELF identity, then separate 60-second steady idle. No track loading,
playback or control movement. The same instrumentation has not had its absolute
observer cost removed; conclusions concern comparable instrumented images.
Reject >=6000 us, target <=3000 us, require valid calibration/windows/partitions.
After variant, or earlier control failure, restore/readback/cold/listen s07.
Physical topology/ROM entry and all device measurements are still NOT_RUN.

The statement above is the frozen pre-device checkpoint. The subsequent
[comparison and recovery](2026-11-02-pack-placement-device.md) executed control,
IRAM and IRAM-return (control-return unused), with 36 current predecessors.
