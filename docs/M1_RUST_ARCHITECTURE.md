# Pajoniiir-M1 Rust Architecture

**Status:** architecture baseline v0.1  
**Date:** 2026-09-24

## 1. Goals

M1 Rust is a clean product implementation for the custom Pajoniiir-M1 hardware while preserving the released Pajoniiir M2.2 feature contract.

The architecture optimizes for:

- deterministic real-time audio;
- data-driven multi-controller support;
- host-testable product logic;
- explicit ownership and bounded queues;
- a provider-neutral media/analysis model;
- Slint UI with a dedicated high-throughput waveform renderer;
- fail-closed storage/update/recovery behavior;
- minimal, auditable hardware-specific and unsafe code.

## 2. Runtime boundary

The permanent application architecture is native Rust `no_std` on ESP32-P4. The firmware entry point runs on the current esp-rtos 0.4 Embassy executor. Hardware-owning tasks are feature-gated: the default image starts the scheduler but does not start USB0, while `usb0-hardware-bringup` compile-verifies the real USB_HS owner-task spawn without upgrading any hardware claim.

ESP-IDF application runtime, FreeRTOS and `esp-idf-sys` are not architectural dependencies. Compatibility with the Espressif boot/partition/OTA ecosystem may be retained using Rust-native bootloader/partition support where appropriate.

Current starting platform:

```text
ESP32-P4NRW32X v3.2+
        |
    esp-hal 1.2.2
        |
   platform / M1 BSP
        |
 product-domain crates
```

Unstable esp-hal APIs are allowed only inside narrow hardware integration crates.

## 3. System ownership

```text
USB DJ controller      Touch        Web Remote
      |                  |              |
      +-------- raw input adapters ------+
                         |
                  ControlEvent
                         |
                Control Scheduler
                         |
               Authoritative State
             /          |          \
          Decks        Mixer       System
             \          |          /
                  State Diff
              /        |         \
       Controller LED  Slint UI   Web state
```

No UI/controller/network adapter mutates deck internals directly.

## 4. High-level crate map

Planned workspace:

```text
firmware-rust/
  apps/
    m1-firmware/
    m1-factory/
    desktop-simulator/
  crates/
    pajoniiir-core/
    pajoniiir-controller-core/
    pajoniiir-controller-profile/
    pajoniiir-controller-usb/
    pajoniiir-controller-audio/
    pajoniiir-control-scheduler/
    pajoniiir-deck/
    pajoniiir-mixer/
    pajoniiir-audio-core/
    pajoniiir-audio-codecs/
    pajoniiir-audio-dsp/
    pajoniiir-media-block/
    pajoniiir-media-fs/
    pajoniiir-media-catalog/
    pajoniiir-media-identity/
    pajoniiir-rekordbox/
    pajoniiir-track-analysis/
    pajoniiir-apta-adapter/
    pajoniiir-waveform/
    pajoniiir-ui-model/
    pajoniiir-ui-slint/
    pajoniiir-settings/
    pajoniiir-networking/
    pajoniiir-web-api/
    pajoniiir-ota/
    pajoniiir-telemetry/
    pajoniiir-firmware-health/
    pajoniiir-m1-bsp/
    pajoniiir-p4-ppa/
```

The first foundation commit intentionally creates only the host-safe crates needed to establish boundaries. Hardware crates are added after their API contracts are separately reviewed.

## 5. UI architecture

Slint owns layout, text, controls, library/settings views and normal UI state.

The large moving waveforms are rendered by `pajoniiir-waveform`, not by constructing thousands of Slint elements.

```text
             immutable UI snapshot
                     |
                Slint render
                     |
                RGB565 frame
                     |
           waveform/compositor pass
          /         |           \
  waveform spans  beat grid   cue markers
          \         |           /
                RGB565 frame
                     |
                P4 PPA
             RGB565 -> RGB888
                     |
                 MIPI DSI
```

On desktop, the PPA/DSI tail is replaced by a simulator presentation path.

Waveform hot paths may write vertical RGB565 spans directly. `embedded-graphics` is preferred for suitable primitives, markers, lines, labels and testable DrawTarget integrations.

Only one runtime context owns Slint. Other workers communicate through bounded messages and immutable snapshots.

## 6. Framebuffer policy

Initial target is full-frame correctness before optimization.

For 800x480:

- RGB565 frame: 768,000 bytes.
- RGB888 DSI frame: 1,152,000 bytes.

Initial hardware hypothesis:

```text
RGB565 composition A
RGB565 composition B
RGB888 DSI output
```

This consumes about 2.69 MiB before alignment/metadata and fits within the 32 MiB PSRAM budget, subject to real DMA/cache-coherency qualification.

Dirty rectangles/partial rendering are optimizations after measured full-frame behavior.

## 7. Audio architecture

Audio is split into pure product DSP/state and hardware sinks.

```text
media source
   -> decoder
   -> canonical PCM timeline/cache
   -> source arbitration (normal / scratch)
   -> resampler / Master Tempo
   -> trim
   -> EQ
   -> filter / Pad FX / Beat FX
   -> PFL branch ----------------------> controller cue/UAC
   -> fader / crossfader
   -> dual-deck sum
   -> master volume / trim
   -> MAIN limiter
   -> PCM5102A MAIN
```

The real-time path does not perform filesystem, network, OTA, logging-format, profile parsing or unbounded allocation.

Audio health is observable through counters rather than logs in deadline-critical code.

## 8. Controller architecture

```text
USB MIDI packets
      |
transport parser
      |
controller profile runtime (S3CP v2 initially)
      |
Semantic ControlEvent
      |
scheduler / reconciler
      |
authoritative product state
      |
Semantic LedState
      |
controller profile output mapper
      |
USB MIDI OUT
```

Controller audio is a sibling capability, not a hidden FLX4 special case.

A controller audio capability describes interface/alternate setting, endpoints, sample formats, channel map, sample rates, headphone mapping, sync/feedback mode and quirks.

S3CP v2 remains the R1 control/LED compatibility format. A future schema extension for UAC metadata must be versioned without silently breaking installed profiles.

## 9. Media architecture

```text
USB HS / microSD / test fixture
             |
         BlockDevice
             |
          Partition
             |
         Filesystem
             |
        Media I/O gate
             |
   +---------+----------+
   |                    |
Library/catalog      Deck source
   |
MediaIdentity
```

All retained references carry a media generation or equivalent validity token so disconnect/reinsert cannot resurrect stale handles.

USB0 High-Speed mass storage is split at an explicit ownership boundary. The P4 USB host/class-driver task owns enumeration, SCSI/BOT/UAS-equivalent command execution, endpoint state and DMA lifetime. `pajoniiir-media-usb-msc` exposes only a bounded, no_std block transport to the media pipeline. A disconnected adapter is terminal and cannot be re-armed; re-enumeration creates a fresh adapter/media generation. Requests carry both a monotonic request ID and the current `MediaLease`; the P4 owner task receives them through bounded request/completion channels, and a completion is accepted only if its original lease still matches the current `MediaSession`. Thus a command that was in flight during disconnect/re-enumeration cannot publish stale data into the new medium generation. Target-side partition discovery is a resumable state machine over the same queue: it owns one exact logical-block scratch buffer, relinquishes that buffer while a read is in flight, and preserves the pending ticket/LBA if its future is cancelled. Resumption drains the original completion before another read is issued, so cancellation cannot duplicate a SCSI command or alias the scratch buffer. A dedicated media worker now consumes USB0 Ready/Detached lifecycle events, owns fixed no-heap 512/1024/2048/4096-byte scratch cells, runs that discovery plan to a generation-safe UsbMscMountedMedia descriptor, and aborts failed mount attempts explicitly. The Media I/O gate is now implemented as an owned-buffer async block contract plus a bounded AsyncPartitionDevice, and a separate filesystem worker owns that adapter. FAT32 probing performs async BPB validation followed by an FSInfo-sector read and validates geometry, signatures, root-cluster bounds and FAT capacity before publishing the volume as probed; exFAT is still limited to boot-sector classification at this stage. The Embassy task deliberately does not call the existing synchronous FAT32/exFAT file APIs. The neutral media contract now exposes a separate native `AsyncFileSystem` / `AsyncWritableFileSystem` surface for executor-owned backends, with async read/seek/directory/close and explicit flush/sync durability boundaries; there is intentionally no blanket adapter from the synchronous traits. A native read-only FAT32 implementation now binds that async contract directly to `OwnedBlockDevice`: one owned logical-sector buffer stays with the filesystem service, FAT-chain traversal and directory scans await the USB queue, detached file/directory handles carry `MediaLease`, ASCII path lookup is case-insensitive, VFAT long filenames are decoded allocation-free into caller storage, and sequential file reads retain cluster position while backward/random seeks remain explicitly non-constant-time. The USB0 filesystem worker promotes a validated FAT32 partition into that native async filesystem object and transfers ownership of the exact logical-sector scratch buffer into it; detach or replacement consumes the filesystem, recovers the buffer and returns it to the fixed scratch pool before another generation can mount. A bounded filesystem broker now serializes `stat/open/read/seek/close` and directory operations through the single owner. `Library`, `DeckA` and `DeckB` each have a dedicated depth-1 completion channel, so one consumer cannot drain another consumer's completion; each client tracks one in-flight lease-bound ticket, and read/directory operations return caller-owned buffers on both success and failure. Paths use an allocation-free 1 KiB `BoundedPath`. exFAT remains boot-classified only. The `pajoniiir-rekordbox` crate is a no_std parser foundation for `PIONEER/rekordbox/export.pdb`: it validates bounded headers/table directories, safely walks backward row-slot groups without underflow, decodes DeviceSQL including UTF-16 paths, and parses track rows into fixed-capacity title/audio/ANLZ fields while retaining exact BPM, IDs and duration. The P4 image carries a compile-time anchor for this parser. A dedicated USB0 Library worker now receives overwrite-latest filesystem lifecycle signals, owns a single 4 KiB PDB page buffer, opens `export.pdb` only through the `Library` broker client, validates the first page and publishes generation-bound table roots for Tracks/Artists/Albums/Keys. The signal is intentionally single-consumer/latest-state, so detach/reinsert can supersede stale lifecycle notifications while any in-flight filesystem operation still completes through its lease-bound broker ticket. PDB page sizes above 4 KiB currently fail explicitly rather than consuming an unbounded buffer. The next implementation boundary is the resumable table/page walker that builds the immutable media catalog. The worker exposes a lock-free-duration health snapshot through short critical sections only; no mount-state lock is held across an await. The real ESP32-P4 High-Speed host binding remains COMPILE_VERIFIED/HARDWARE_PENDING until EVT hardware exists. The P4 firmware compile gate binds the current esp-hal USB_HS handle and embassy-usb-host MSC LUN API (capacity, TEST UNIT READY, READ/WRITE blocks and SYNCHRONIZE CACHE) to a target-only async adapter; it does not claim enumeration, VBUS, PHY, throughput or hot-plug hardware verification.

## 10. TrackAnalysis architecture

Application logic must not depend directly on Rekordbox ANLZ.

```text
Rekordbox PDB/ANLZ ---\
                      -> immutable TrackAnalysis -> Deck/UI/Sync/Waveform
libapta native/cache --/
```

The analysis model owns BPM/grid/waveform/meter/downbeat/key/confidence/provenance data as applicable. A loaded deck pins an immutable generation.

libapta is integrated only after its upstream embedded/release gates are met; the neutral model is implemented before that dependency.

## 11. Networking

```text
ESP32-C6
   |
SDIO transport
   |
ESP-Hosted protocol boundary
   |
network stack
   |
HTTP/API/Web Remote
```

Network code emits semantic product commands and consumes snapshots. It cannot directly manipulate deck or audio structures.

## 12. Persistence

Every persistent binary format is explicitly versioned:

```text
magic
schema version
payload length
payload
CRC / integrity field
```

Migrations are explicit. Rust memory layout is never serialized as the storage ABI.

This applies to settings, Hot Cues/loops, media catalog, analysis sidecars, seek indexes and controller profile metadata as applicable.

## 13. OTA / boot health

The intended state machine preserves the released product properties:

```text
download/upload
 -> validate container/manifest/signature
 -> write inactive slot
 -> activate candidate
 -> reboot
 -> critical startup self-tests
 -> mark READY
```

Failure before READY leaves rollback possible.

The M2.2 post-OTA reboot panic class must have a regression gate in the M1 test plan.

## 14. Diagnostics

Producers emit bounded fixed-size events/counters. Formatting/storage occurs outside the real-time path.

M1 health includes:

- reset/boot reason;
- running image / rollback status;
- USB0/USB1 power/fault/recovery;
- controller MIDI/UAC health;
- PCM/audio timing;
- internal RAM/PSRAM reserves;
- media generation/errors;
- C6/network state;
- INA238 system voltage/current when fitted/enabled;
- UI frame/render health;
- OTA state.

## 15. Multicore starting hypothesis

Until measured on hardware:

**Core 0 / real-time bias**
- controller UAC deadlines;
- decode deadline work;
- canonical PCM/timeline;
- resampler/Master Tempo;
- DSP/mixer;
- PCM5102A sink.

**Core 1 / system bias**
- USB MIDI/control;
- USB MSC/library;
- storage background work;
- analysis;
- Slint/waveform/PPA coordination;
- C6/network/Web;
- OTA and service work.

This is a hypothesis, not a frozen scheduling contract. Hardware profiling may move tasks, but the real-time/non-real-time boundary remains.

## 16. Memory policy

Internal SRAM is preferred for latency-critical descriptors, small queues and DMA/USB/audio structures that require it.

PSRAM is preferred for large retained data:

- UI framebuffers/assets;
- waveforms;
- library metadata;
- analysis/cache objects;
- bounded PCM blocks only after DMA/cache behavior is proven safe.

No subsystem may assume PSRAM is DMA-safe without an explicit hardware/platform proof.

## 17. Unsafe policy

Application/domain crates use `#![forbid(unsafe_code)]` where practical.

Unsafe code is restricted to audited boundary crates such as:

- low-level P4 PPA/PAC access;
- DMA/cache wrappers when required;
- tightly scoped external-library FFI if retained.

An unsafe boundary must document the invariant it upholds.

## 18. Testing strategy

Host tests are the primary development engine before hardware exists.

Required test families:

- controller profile golden/parity;
- reconnect/replay/state reconciliation;
- deck/Hot Cue/loop/Beat Jump/Sync;
- PDB/ANLZ fixtures;
- media identity properties;
- WAV/MP3/FLAC format/error fixtures;
- DSP deterministic PCM fixtures;
- waveform golden images;
- settings/persistence corruption/migration;
- OTA manifest/signature/failure matrix;
- Web API contract;
- storage/partition/filesystem fault injection.

Fuzz targets are planned for PDB, ANLZ, WAV/FLAC metadata, partition tables, controller profiles, OTA manifests and HTTP/parser boundaries.

## 19. Hardware gate boundary

The following cannot be considered closed without physical M1/P4 hardware:

- PSRAM/cache/DMA placement assumptions;
- PPA throughput and coherency;
- MIPI DSI timings/scanout;
- touch electrical/runtime behavior;
- PCM5102A clocks/output;
- USB0 HS MSC sustained throughput/hotplug;
- USB1 MIDI;
- USB1 isochronous UAC and feedback/drift behavior;
- C6 SDIO/ESP-Hosted;
- VBUS power/fault recovery;
- INA238 measurements;
- dual-core deadline behavior;
- thermal/load/soak tests.

Host success is necessary but not a substitute for these gates.
