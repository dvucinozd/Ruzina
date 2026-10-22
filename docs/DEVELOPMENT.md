# Development status and next steps

Updated: 2026-10-22. This page describes the current Guition Rust development
scope; compilation, host behavior and physical acceptance remain distinct.

For continuation by another agent, read the [handoff](HANDOFF.md) first. It
records the exact branch/worktree, source pins, next batches, constraints and
verification boundaries. RP-01/02 are host-implemented; continue with
RP-03/04/05. This is not complete M2.5 feature parity.

## Implemented software

The shared crates and simulator cover semantic two-deck control, data-driven
controller profiles, durable Hot Cue record/journal policy, loops/Beat Jump/Sync,
Rekordbox PDB/DAT/EXT analysis, bounded WAV/MP3/FLAC source decoding, PCM history,
scratch/Master Tempo, mixer/FX, separate MAIN/headphone routing and output
completion. Host failure tests cover media replacement/removal, stale completions,
backpressure, source errors and persistence containment.

The Slint simulator provides interactive decks, Library, Hot Cues, session
settings, mixer/PFL/FX and health/analysis views. Its media, timing and audio
environment are simulated. The reference output sink does not establish physical
I2S/UAC timing or listening acceptance.

Optional APTA conversion/worker/service/lifecycle logic is adopted selectively.
It has bounded banks and generation-safe publication, stopped-deck scheduling
and captured state/reason diagnostics. The original integration branch is still
unmerged. APTA remains OFF on Guition and outside default dependency graphs.

## M2.5 functional baseline and next software batches

The original foundation was M2.2. Released M2.5 at `20f1c3f0` is now the
functional reference; later shared-P4 beta/master fixes are evaluated separately.
The [pinned parity audit](M2_5_PARITY_PLAN.md) records implementation evidence,
RP-01..20 tasks, requirement mappings and acceptance for each batch:

1. **RP-01/02 (host-implemented):** PDB title index correction, independent
   import fixtures and end-to-end S3CP v2/v3/v4 compatibility, including bounded
   output initialization and the M2.5 Smart CFX/FILTER_ALWAYS channel-filter
   gate. Firmware wiring is compile-only; an upload path remains open.
2. **RP-03/04/05/15/18:** CUE hold-preview/PLAY commit, explicit VINYL/CDJ,
   shared load admission, loop/memory/source cue behavior and reconnect fencing.
3. **RP-06/07/08:** ordered playlist hierarchy, artwork/PWV4 and cold MP3
   seek/duration budgets, preserving existing sample-exact decoder behavior.
4. **RP-11/12/13:** durable settings, C6/Web contracts and signed OTA/readiness
   state machines with host fault injection before physical activation.
5. **RP-09/10/14/16/17:** UAC descriptors/packets/clocks, board-owned USB roles,
   MAIN switching, headphone gain ownership and bounded cache/diagnostic behavior.

The first four batches and host portions of the fifth can proceed before board
arrival. Current host coverage does not close these newly identified gaps.
RP-19 extra-controller/absolute-PFL work and RP-20 Pro DJ Link are optional;
native APTA, recorder and disabled donor UI are not added to R1 by inference.

## Prepared Guition work

- Separate board profiles, scheduler-only BSPs and chip minimum 302.
- Immutable boot/service records over native USB Serial/JTAG.
- Separate basic, flash RDID, PSRAM and combined-memory package variants.
- Raw capture/ingestion with identity, capacity and failed-probe checks.
- Inactive panel/touch adapter contracts and source fingerprints for both models.

PSRAM scratch success covers only the 64 KiB CPU-cached span at 80 MHz. Physical
size/retention, cache maintenance, DMA and production clocks remain unqualified.
Panel tables and touch payloads are not included or selected for an arriving unit.

## Board arrival sequence

1. Record model/unit/PCB/panel markings and observed silicon/flash/PSRAM identity.
2. Capture factory logs, verify a factory backup and establish ROM recovery.
3. Select a board-specific programming procedure, boot the minimal Rust image,
   and retain exact programming/readback/image/log evidence.
4. Qualify flash and PSRAM probes, then actual memory/allocator/cache ownership.
5. Implement/qualify display, backlight and touch per exact panel/unit.
6. Qualify SD power/media lifecycle and USB data/power/backfeed before peripherals.
7. Connect the shared product owners and qualify controller MIDI/UAC and stereo
   MAIN DAC routing, headphones, latency, listening and sustained load.

Both models require their own results. JC1060 touch reset/interrupt mapping,
JC8012 GSL3680 payload provisioning, USB-FS package mapping and shared host-power
policy remain explicit board questions. See the [matrix](boards/GUITION_HARDWARE_MATRIX_2026-10-20.md).

## Further software and release work

General settings persistence/migration, transactional writable media workflows,
signed A/B OTA/rollback/readiness, C6 transport/networking and Web Remote are
unfinished. Physical audio sinks and controller-specific headphone channel maps
are not implemented/qualified. Musical analysis accuracy, device performance,
sanitizer/fuzz campaigns and long-duration physical soak are separate gates.

The only deployed-board targets are the two Guition models. `integration-check`
exists solely to type-check reusable owner combinations; it acquires no board
resources or product tasks. No Guition runtime feature is inferred from it.

See [architecture](M1_RUST_ARCHITECTURE.md), [requirements](M1_RUST_PRODUCT_REQUIREMENTS.md),
[first boot](boards/GUITION_FIRST_BOOT_2026-10-20.md) and [validation](VALIDATION.md).
