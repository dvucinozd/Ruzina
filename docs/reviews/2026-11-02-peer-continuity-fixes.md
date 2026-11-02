# Peer continuity and cause-analysis repairs

Continues the [Astra review](2026-11-02-astra-progress-review.md). Source base:
`e4aa31d`. Offline only; installed s07 and native minimum 302 are unchanged.

## R1: preserve the peer's committed audio

Engine revisions are now per deck. Control ticks continue consuming load/seek
receipts without requiring UAC rendering. Ordinary source/seek changes reset
only the affected deck at the next 128-frame render boundary. The cached mixed
suffix is consumed first: its source frames have already been committed to
the rings. This adds at most one block (2.667 ms at 48 kHz) of application
output latency, in addition to existing USB queue latency. It is not a claim
about end-to-end acoustic latency. No reader tail is rewound.

Media revocation retains the immediate cached-output clear and both-deck reset
for this backend's single SD lease. The existing revocation/stale-result and
controller-disconnect tests remain in the matrix.

The new regression compares full MAIN/headphone PCM against a reference for
both deck directions, 44100/48000 Hz, offsets 1/48/127 within a 128-frame block,
pitch, EQ and filter. Peer actions cover pending seek, Ready, Failed, Cancelled,
and successful/failed LOAD completion. Existing dedicated tests cover foreign
and superseded receipts. Physical MAIN/PFL listening remains NOT_RUN.

## R2: require unsigned complete analysis records

Record, cause and stage rows require their numeric schema as unsigned u32s
before they can establish a partition or attribution. Boolean validity fields
must be 0/1; generation/ordinal must be nonzero. Missing IRQ entry counts,
negative balanced stage values, impossible nonzero IRQ duration with zero
entries, and malformed/oversized values cannot grant valid attribution.
Histogram metadata is also checked. Calibration validation remains separate.
Old raw logs and saved JSON are immutable; reanalysis is a new derived result.

## Cache/compute audit checkpoint and next experiment

Pinned local authority: IDF `fff9895c82d744c7237be8847347bdd1b07c6643`
and vendored esp-usb 1.6.0, not a newer online API.

- `controller_audio.rs::fill_from` uses local MAIN/headphone scratch arrays on
  the USB client stack. `usb_host_enum.c` creates that task with the ordinary
  `xTaskCreatePinnedToCore` API and 8192 bytes. Pinned FreeRTOS `heap_idf.c`
  defines ordinary RTOS heap capabilities as INTERNAL|8BIT; allowing external
  stacks does not by itself place this task there.
- `usb_host_transfer_alloc` delegates to `urb_alloc` in `usb_private.c`.
  `CONFIG_USB_HOST_DWC_DMA_CAP_MEMORY_IN_PSRAM` is disabled in the new normal
  sdkconfig. The selected buffer contract is therefore DMA|CACHE_ALIGNED|
  INTERNAL. Rust heap allocations separately request SPIRAM|8BIT.
- Pinned `components/perfmon/CMakeLists.txt` explicitly returns on non-Xtensa
  targets. Its Xtensa perfmon API cannot be reused for this P4.
- `riscv/rv_utils.h::rv_utils_get_cycle_count` reads a 32-bit cycle CSR. It
  includes interrupt/preemption cycles and wraps; it is not a cache-miss or
  task-CPU-time counter. At 400 MHz one wrap takes about 10.74 seconds.
- The rev1 cache register header exposes L1 bus access/miss/conflict counters.
  Register existence alone does not establish bus-to-core ownership, exclusive
  ownership of enable/clear controls, overflow behavior or an IRAM-safe sampling
  contract. These are still OPEN. No direct-register profiler was enabled.

Next bounded action: resolve those counter contracts against rev1 sources and
existing cache users, then select either a calibrated observational counter
profile or one narrow code-placement variant. Do not guess per-core cache
attribution from IBUS0/DBUS0 names. Both comparison images must include these
functional fixes. Use at least three equivalent cold observations per image,
the same instrumentation and logging, and one separate steady window. Record
counter liveness, wrap/invalid intervals and observer cost. After that single
campaign, choose a supported mechanism or document an inconclusive result;
do not add another profiler without a written decision.

Current recovery ancestry remains 33 predecessors, as in the prior handoff.
New source needs review, committed clean normal/profile packages and new plans
before any device write. The old 14812 us observation and failed 6000 us gate
are historical evidence; these repairs have no target timing result yet.

## Validation

External logs/packages:
`D:/BoardEvidence/JC1060-20261030-COM3/peer-fixes-20261102/`.
The initial matrix command lost shell variables while crossing PowerShell/WSL;
its error is retained in `host-matrix.log`. The saved `checks.sh` is the actual
replacement command. Fresh Windows Python has 35 PASS plus six C skips; all
six real C cases pass in WSL. Final Rust/build/review results follow below.

Rust host profiles pass 39/2/56/57/64/69 tests respectively; all host Clippy
checks and normal/profile RISC-V release Clippy pass. Both local builds pass
package/link/stack checks. They are working-tree builds, not registered flash
packages: normal app 817280 bytes, SHA-256
`bba80299de2e49c45410bc9e51b9505d062ff7a07a626779476f70036c80c315`;
profile app 823024 bytes, SHA-256
`427cb0ab1566d72f198c6ce8656898d5bba2cfb4500fa45e9aec4e804e6f1cd0`.
Reanalysis of both historical logs is byte-for-byte equivalent as parsed JSON
to the saved analysis. Documentation checker: 400 links PASS.

The uncommitted CodeRabbit attempt incorrectly scoped 474 files and failed
the 150-file limit. It is not a passed review. Review the frozen commit against
`e4aa31d` before publication or preparing clean flash packages.

Final exact-parent committed review of source
`eead8c6a9832aee63348922dcf9487992a24658a` completed: CodeRabbit raised 0 issues
across the eight changed files. Fresh clean normal/profile builds both pass;
all 282 source hashes per package and all 16 combined artifact hashes match.
Clean normal app SHA-256:
`3d6afe1076a2df5bc4ac091eadfaaedbcecb2db029694d6d1bfd7ba8711ab35f`;
clean profile app SHA-256:
`216dd8df18675ff334b068673f306a04a30065314875dc5905e33aaf99f7f058`.
Working-tree pilot hashes above are retained separately. These clean packages
are BUILD_ONLY / NOT_FLASHED and have no new physical timing acceptance or
registered write plan. [Verification index](evidence/2026-11-02-peer-fixes/checks.json)
records source/artifact verification and raw-log hashes. No new CI result is
claimed for this branch; source publication and CI precede physical registration.
