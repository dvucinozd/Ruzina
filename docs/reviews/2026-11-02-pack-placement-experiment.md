# Bounded packetizer placement experiment

Continues the [peer continuity repairs](2026-11-02-peer-continuity-fixes.md)
and [cache/compute plan](2026-11-02-uac-cache-compute-follow-up.md).
Status: OFFLINE_IMPLEMENTED / NOT_FLASHED; target result NOT_RUN.
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
