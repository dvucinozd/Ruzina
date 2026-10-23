# Development status and next steps

Updated: 2026-10-23. This page describes the current Guition Rust development
scope; compilation, host behavior and physical acceptance remain distinct.

For continuation by another agent, read the [handoff](HANDOFF.md) first. It
records the exact branch/worktree, source pins, next batches, constraints and
verification boundaries. RP-01..08, RP-11 and RP-15/18 are host-implemented
(RP-07 cover display in the simulator only; RP-11 settings without a target
flash owner); RP-12 and RP-13 have host contracts with fake transports and
slots (no HTTP server, C6 driver, TLS or target flash adapter), and the host
parts of RP-09/10/14/16/17 exist (UAC, USB roles, MAIN change, headphone
packing, MT regression; no USB adapter or board budgets). The remaining work
is board qualification and adapters. This is not complete M2.5 parity.

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
2. **RP-03/04/05/15/18:** CUE hold-preview/PLAY commit and explicit VINYL/CDJ
   (RP-03 host-implemented; tempo nudge/paused scrub still absent), shared load
   admission (RP-04 host-implemented: default-on LOAD LOCK verdict, single-flight
   permits and PLAY-vs-LOAD fencing before audio replacement), loop/memory/source
   cue behavior (RP-05 host-implemented; memory-cue view/recall pending RP-07),
   reconnect fencing (RP-15 host-implemented) and PDB row/EXT cue/loop-range
   edges (RP-18 host-implemented; remote CUE tap waits for the Web Remote).
3. **RP-06/07/08:** ordered playlist hierarchy (RP-06 host-implemented; no
   Slint hierarchy view yet), artwork/PWV4 (RP-07 host-implemented: PWV4 and
   memory-cue markers, artwork references, bounded baseline JPEG decoder and
   cover/logo in the simulator deck; target display open) and cold MP3
   seek/duration (RP-08 host-implemented: bounded frame
   index, exact indexed cold seeks, scanned untagged duration), preserving
   existing sample-exact decoder behavior.
4. **RP-11/12/13:** durable settings (RP-11 host-implemented in
   `pajoniiir-settings`: versioned A/B records, durable-before-publish owner,
   explicit M2.5 import), C6/Web contracts (RP-12 host contracts in
   `pajoniiir-web` with a fake transport) and signed OTA/readiness state
   machines (RP-13 host contracts in `pajoniiir-ota`: signed `.ddjota`,
   strict channel document, verify-before-erase/switch install, pending
   candidate confirm/rollback) with host fault injection before physical
   activation.
5. **RP-09/10/14/16/17:** UAC descriptors/packets/clocks, board-owned USB roles,
   MAIN switching, headphone gain ownership and bounded cache/diagnostic behavior
   (host contracts in `pajoniiir-uac`, `main_change`, board `usb` roles and
   service `resources`, plus the MT starvation regression and the exact-phase
   48 to 44.1 kHz converter; physical streams and board budgets remain open).

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

Target adapters for settings persistence, transactional writable media
workflows, signed A/B OTA/rollback/readiness, C6 transport/networking and Web
Remote are unfinished; their RP-11/12/13 rules exist only as host contracts. Physical audio sinks and controller-specific headphone channel maps
are not implemented/qualified. Musical analysis accuracy, device performance,
sanitizer/fuzz campaigns and long-duration physical soak are separate gates.

The Rust product will be renamed from Pajoniiir to **Ruzina** once the stacked
RP PRs are merged; Pajoniiir remains the C donor's name. The
[handoff](HANDOFF.md) records the scope and the persisted names that need an
explicit decision.

The only deployed-board targets are the two Guition models. `integration-check`
exists solely to type-check reusable owner combinations; it acquires no board
resources or product tasks. No Guition runtime feature is inferred from it.

See [architecture](M1_RUST_ARCHITECTURE.md), [requirements](M1_RUST_PRODUCT_REQUIREMENTS.md),
[first boot](boards/GUITION_FIRST_BOOT_2026-10-20.md) and [validation](VALIDATION.md).
