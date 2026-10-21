# Development status and next steps

Updated: 2026-10-21. This page describes the current Guition Rust development
scope; compilation, host behavior and physical acceptance remain distinct.

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
