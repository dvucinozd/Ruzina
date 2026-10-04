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

USB0 High-Speed mass storage is split at an explicit ownership boundary. The P4 USB host/class-driver task owns enumeration, SCSI/BOT/UAS-equivalent command execution, endpoint state and DMA lifetime. `pajoniiir-media-usb-msc` exposes only a bounded, no_std block transport to the media pipeline. A disconnected adapter is terminal and cannot be re-armed; re-enumeration creates a fresh adapter/media generation. Requests carry both a monotonic request ID and the current `MediaLease`; the P4 owner task receives them through bounded request/completion channels, and a completion is accepted only if its original lease still matches the current `MediaSession`. Thus a command that was in flight during disconnect/re-enumeration cannot publish stale data into the new medium generation. Target-side partition discovery is a resumable state machine over the same queue: it owns one exact logical-block scratch buffer, relinquishes that buffer while a read is in flight, and preserves the pending ticket/LBA if its future is cancelled. Resumption drains the original completion before another read is issued, so cancellation cannot duplicate a SCSI command or alias the scratch buffer. A dedicated media worker now consumes USB0 Ready/Detached lifecycle events, owns fixed no-heap 512/1024/2048/4096-byte scratch cells, runs that discovery plan to a generation-safe UsbMscMountedMedia descriptor, and aborts failed mount attempts explicitly. The Media I/O gate is now implemented as an owned-buffer async block contract plus a bounded AsyncPartitionDevice, and a separate filesystem worker owns that adapter. FAT32 probing performs async BPB validation followed by an FSInfo-sector read and validates geometry, signatures, root-cluster bounds and FAT capacity before publishing the volume as probed; exFAT is promoted into the native read-only async filesystem after boot-region validation. The Embassy task deliberately does not call the existing synchronous FAT32/exFAT file APIs. The neutral media contract now exposes a separate native `AsyncFileSystem` / `AsyncWritableFileSystem` surface for executor-owned backends, with async read/seek/directory/close and explicit flush/sync durability boundaries; there is intentionally no blanket adapter from the synchronous traits. A native read-only FAT32 implementation now binds that async contract directly to `OwnedBlockDevice`: one owned logical-sector buffer stays with the filesystem service, FAT-chain traversal and directory scans await the USB queue, detached file/directory handles carry `MediaLease`, ASCII path lookup is case-insensitive, VFAT long filenames are decoded allocation-free into caller storage, and sequential file reads retain cluster position while backward/random seeks remain explicitly non-constant-time. The USB0 filesystem worker promotes a validated FAT32 partition into that native async filesystem object and transfers ownership of the exact logical-sector scratch buffer into it; detach or replacement consumes the filesystem, recovers the buffer and returns it to the fixed scratch pool before another generation can mount. A bounded filesystem broker now serializes `stat/open/read/seek/close` and directory operations through the single owner. `Library`, `DeckA` and `DeckB` each have a dedicated depth-1 completion channel, so one consumer cannot drain another consumer's completion; each client tracks one in-flight lease-bound ticket, and read/directory operations return caller-owned buffers on both success and failure. Paths use an allocation-free 1 KiB `BoundedPath`. exFAT now shares the bounded filesystem broker through backend-neutral lease-bound file and directory handles. The `pajoniiir-rekordbox` crate is a no_std parser foundation for `PIONEER/rekordbox/export.pdb`: it validates bounded headers/table directories, safely walks backward row-slot groups without underflow, decodes DeviceSQL including UTF-16 paths, and parses track rows into fixed-capacity title/audio/ANLZ fields while retaining exact BPM, IDs and duration. The P4 image carries a compile-time anchor for this parser. A dedicated USB0 Library worker now receives overwrite-latest filesystem lifecycle signals, owns a single fixed 8 KiB PDB I/O buffer, opens `export.pdb` only through the `Library` broker client, and publishes generation-bound table roots for Tracks/Artists/Albums/Keys. Header/table-directory parsing requires only the bounded page-zero prefix (at most 540 bytes for 32 tables), so valid Rekordbox page sizes up to 64 KiB no longer require a 64 KiB allocation or a full-page read. The signal is intentionally single-consumer/latest-state, so detach/reinsert can supersede stale lifecycle notifications while any in-flight filesystem operation still completes through its lease-bound broker ticket. `pajoniiir-rekordbox` now also exposes a resumable page-slot walker: it parses the fixed 40-byte page header, plans at most 256 declared rows per batch, and needs at most 576 bytes of row-slot metadata for each step even when the PDB page itself is 64 KiB. Each ingested batch yields fixed-capacity heap offsets and preserves walk progress/truncation state without allocation. The Library worker now wires those slot plans to lease-bound broker `seek`/`read` operations, walks the complete Tracks page chain with the same 8 KiB buffer, validates next-page bounds/loop limits, and records pages plus declared/present/truncated row counts before closing `export.pdb`. No full PDB page is retained. The Rekordbox core now also exposes a fixed 136-byte track-row header decoder that extracts track/artist/album/key IDs, exact BPM, rounded BPM, duration and the four required DeviceSQL relative offsets without a full page. A checked helper converts each nonzero relative string offset into its page-relative position, allowing the Library worker to fetch title/filename/audio-path/ANLZ fields on demand with the same bounded I/O buffer. The Tracks walker now consumes each fixed-capacity slot batch before reusing the I/O buffer: every present heap offset is range-checked against the page, read through the lease-bound Library broker, and decoded through only the 136-byte track header contract. The worker records how many present rows are actual track subtype rows while retaining the same single 8 KiB I/O buffer. The Rekordbox core splits DeviceSQL discovery from decoding: a 1-byte short/empty prefix or 4-byte long-string prefix yields the declared field length and encoding flags before the caller commits to a bounded read. `PdbText` decodes one isolated field allocation-free, and `PdbTrackRow::from_header_and_fields` applies the released title→filename fallback while assembling fixed-capacity audio/ANLZ paths. The Library Tracks walker now follows each decoded row header with lease-bound isolated reads for title, filename, audio path and ANLZ path, verifies that every declared DeviceSQL field remains inside its PDB page and inside the fixed 8 KiB read window, then assembles a complete `PdbTrackRow` before counting the row as decoded. No assembled row is published yet. A new `pajoniiir-media-catalog` crate now defines the publication boundary: fixed-capacity UTF-8 fields, the released 1024-track maximum as a policy constant, duplicate stable/source identity rejection, a `MediaLease`-bound builder, immutable snapshot lookup, and a publisher that keeps the previous snapshot visible until a complete replacement snapshot is ready. The crate is generic over capacity and does not itself allocate or place a 1024-track bank on P4; final retained-catalog storage/PSRAM placement remains a BSP/hardware-qualified decision. FAT32 mounts now derive a domain-separated `VolumeIdentity` from the validated FAT32 volume serial, sector geometry, total sectors and selected partition start. That identity is carried in the Library lifecycle/probe and every fully decoded Track row must successfully derive a stable `MediaTrackId` from `VolumeIdentity + normalized audio path` before it counts as importable; invalid/non-UTF-8 paths fail the import rather than falling back to a row index. The Rekordbox core now also codifies the released Artists/Albums (`id@+4`, DeviceSQL `@+10`, minimum 14 B) and Keys (`id@+0`, DeviceSQL `@+8`, minimum 9 B) row layouts, plus a generic allocation-free `PdbNameMap` with the released 512-name policy bound, duplicate-ID rejection and fixed 96-byte UTF-8 name storage. No 512-entry maps are instantiated on the target yet; staging placement remains a PSRAM/BSP decision. `pajoniiir-rekordbox` now also owns the provider adapter into the neutral catalog: generic Artist/Album/Key maps resolve the row foreign keys, then `PdbTrackRow + PdbResolvedNames + VolumeIdentity` converts to `CatalogTrack` with stable path-derived `MediaTrackId`, exact BPM, millisecond duration and bounded metadata. UTF-8 failures, provider-side truncation and neutral-contract overflow (for example an oversized key) are hard errors; metadata is never silently truncated at the provider→catalog boundary. The audio path is checked for truncation before stable-ID hashing, so two long paths cannot alias merely because their first 256 bytes match. The target Library worker now also walks optional Artists, Albums and Keys page chains through the same lease-bound broker and single 8 KiB buffer. It validates each row layout and DeviceSQL field, counts valid names, records structural slot truncation, counts value truncation and reports whether the released 512-name staging policy would be exceeded; it deliberately retains no name map yet. The catalog and name-map crates now expose CI-enforced footprint constants computed with `core::mem::size_of`: each released 1024-track catalog bank is capped at 1 MiB, builder+published transactional coexistence at 2 MiB, and all three 512-entry name staging maps at 256 KiB. The next boundary is explicitly placing that bounded storage for P4 (expected PSRAM behind a compile-only platform gate), then retaining the name maps, feeding resolved tracks into a staging `CatalogBuilder` and publishing atomically. The worker exposes a lock-free-duration health snapshot through short critical sections only; no mount-state lock is held across an await. The real ESP32-P4 High-Speed host binding remains COMPILE_VERIFIED/HARDWARE_PENDING until EVT hardware exists. The P4 firmware compile gate binds the current esp-hal USB_HS handle and embassy-usb-host MSC LUN API (capacity, TEST UNIT READY, READ/WRITE blocks and SYNCHRONIZE CACHE) to a target-only async adapter; it does not claim enumeration, VBUS, PHY, throughput or hot-plug hardware verification.

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

The released library-memory policy is now CI-enforced from actual Rust type sizes rather than hand estimates: one 1024-track catalog builder and one published catalog bank must each remain within 1 MiB, their transactional coexistence within 2 MiB, and all three 512-entry Rekordbox name staging maps within 256 KiB. These are placement budgets, not proof that PSRAM is safe for every access pattern.

The P4 firmware now exposes an explicit `p4-psram-storage` compile gate. Under that feature the ESP32-P4 PSRAM peripheral is initialized through `esp-hal` and its mapped region is registered with `esp-alloc` as external heap storage. CI cross-checks the feature independently and links it together with USB0 bring-up. The USB0 Library worker uses that boundary for three bounded Artists/Albums/Keys staging vectors and two recyclable 1024-track catalog banks. Name tables are retained first, Tracks are converted into provider-neutral `CatalogTrack` rows in the inactive bank, and publication occurs only after the complete PDB scan and file close succeed. Failed imports recycle the build bank without replacing the currently published snapshot; detach/filesystem-failure invalidates the matching lease and returns its bank to the pool. The neutral catalog crate keeps the original fixed-array API while the target uses the storage-generic `CatalogBank` transaction API, avoiding large inline stack moves.

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


### Native async exFAT mount boundary

The USB0 target validates exFAT boot geometry and the main boot-region checksum through the owned-buffer asynchronous block transport, retains the logical-sector scratch buffer with the mounted volume, and derives a domain-separated exFAT volume identity. File/directory broker operations remain fail-closed until native exFAT handles are wired; the synchronous exFAT backend is not used from Embassy.


### Native exFAT directory-entry parsing

The native exFAT path parses File (0x85), Stream Extension (0xC0), and File Name (0xC1) entry sets without allocation. It validates the entry-set checksum, secondary count, allocation flags, UTF-16 name validity, data-length invariants, and the 256 MiB directory-size limit. Benign secondary entries after the required file-name entries are tolerated and included in the checksum; unexpected critical secondary entries fail closed. This parser is deliberately separated from block I/O so directory streaming and path lookup can be host-tested before target broker integration.


### Native read-only exFAT filesystem core

The exFAT crate now implements the target-facing read-only AsyncFileSystem contract directly over OwnedBlockDevice. Detached file and directory handles are lease-bound; directory iteration consumes validated entry sets sequentially; fragmented streams follow the active exFAT FAT; NoFatChain streams use contiguous cluster arithmetic; and bytes between ValidDataLength and DataLength are returned as zeroes. Path lookup is allocation-free and ASCII-case-insensitive while non-ASCII UTF-8 must match exactly until the exFAT Up-case Table is wired. The USB0 broker request/completion ABI is generalized across FAT32 and exFAT handles, with backend-kind mismatches rejected explicitly.


### exFAT broker integration

The USB0 filesystem broker now exposes backend-neutral lease-bound file and directory handles. Stat/open/read/seek/close and directory operations dispatch to either the native FAT32 or native exFAT async backend while preserving the existing one-in-flight-per-client and owned-buffer completion rules. A handle from the wrong filesystem kind fails closed instead of being reinterpreted. The Rekordbox Library worker consumes the same generic file handle, so a validated exFAT Rekordbox medium can follow the same PDB/catalog pipeline as FAT32 using its domain-separated exFAT VolumeIdentity.


### USB1 Full-Speed controller boundary

The M1 controller port is a separate ESP32-P4 Full-Speed OTG host on GPIO27 (DP) / GPIO26 (DM), independent of the USB0 High-Speed storage controller. Feature `usb1-controller-bringup` owns that peripheral in a persistent Embassy task and performs root-port enumeration without assuming a particular controller VID/PID.

The HAL-independent `pajoniiir-controller-usb` crate parses the configuration descriptor into a bounded `ControllerUsbCapabilities` snapshot covering MIDI Streaming, USB Audio Control/Streaming, HID and vendor interfaces plus relevant endpoint directions. This preserves the product rule that transport discovery is separate from S3CP/profile semantics. A controller is not considered physically supported by descriptor compatibility alone.

CI cross-compiles USB1 independently and links USB0 + USB1 + PSRAM together. Physical VBUS/fault behavior, MIDI traffic, LED output and UAC isochronous/feedback behavior remain `HARDWARE_PENDING`.


### USB-MIDI 1.0 transport boundary

Above USB1 enumeration, `pajoniiir-controller-usb` now provides a transport-neutral locator for alternate-setting-zero Audio/MIDIStreaming interfaces plus allocation-free USB-MIDI 1.0 event-packet encode/decode. It preserves cable number, CIN, payload length, channel-voice status and endpoint max-packet sizes rather than collapsing USB framing into semantic controller events.

The ESP32-P4 USB1 adapter allocates Embassy typed bulk IN and optional bulk OUT pipes from the enumerated device address and route. It rejects invalid Full-Speed bulk max-packet sizes and keeps pipe lifetime bounded to the current enumerated device. This layer intentionally stops before S3CP/profile dispatch: USB transport qualification, controller profile selection and product semantics remain separate gates.


### Controller profile registry and binding state

The released controller-profile selection state machine is now represented in the `no_std` Rust profile crate instead of being deferred to target glue. The bounded registry accepts at most 16 validated S3CP v2 profiles, keeps profile IDs to the released ASCII directory policy, matches controllers by exact VID/PID, and retains explicit `Idle / Matched / Activating / Active / Failed / Unsupported` states.

Connection descriptors carry a monotonic wrapping epoch. A descriptor with an old epoch, or the same epoch with a conflicting VID/PID, is rejected before it can replace the live binding. Rescanning profile storage preserves the current physical descriptor but intentionally drops the active profile back to `Matched`, forcing the newly read bytes to be revalidated and reactivated before use.

This is the HAL-independent half of the microSD profile manager. Profile bytes remain external `/controllers/<id>/profile.s3bin` data; they are not embedded into the firmware image. The SD/MMC scanner, transactional upload/backup recovery and target activation worker remain separate platform boundaries.


### microSD SDMMC0 target ownership

The profile-storage platform boundary now has a target-side compile gate matching the Rev A M1 schematic. ESP32-P4 SDMMC slot 0 owns GPIO39/D0, GPIO40/D1, GPIO41/D2, GPIO42/D3, GPIO43/CLK and GPIO44/CMD. GPIO45 controls the TPS22918-backed 3V3_SD rail and GPIO46 is reserved by the captured schematic for active-low card detect.

The `sdmmc-profile-storage-bringup` task constructs the native `esp-hal 1.2.2` SD host, binds the fixed slot-0 IO-MUX pins, owns the power-enable/card-detect pins and switches the slot into async interrupt-driven mode. It then follows the official `sdio 0.5.1` integration pattern: retain an uninitialized `BlockDevice<Card, ..., 512>` and call `reacquire` after the 3V3_SD settle interval. A failed initialization therefore keeps ownership available for retry instead of consuming the slot. The compile baseline is intentionally 25 MHz; higher-speed timing is deferred to EVT measurements.

FAT profile storage, transactional install/recovery and profile activation remain separate follow-on gates. CI independently cross-compiles this SDMMC boundary and also release-links it together with USB0 HS, USB1 FS and PSRAM, proving compile-time resource coexistence only. Electrical rail timing, back-power avoidance, real card initialization, SD signal integrity, hot-plug and filesystem behavior remain `HARDWARE_PENDING`.


### Connection-safe profile activation tickets

Profile activation across microSD I/O now has an explicit stale-binding guard. `ProfileRegistry::begin_activation` captures a bounded ticket containing the matched registry index, connected VID/PID/capability/epoch snapshot, and the validated S3CP metadata expected from storage. After the potentially asynchronous file read, `validate_activation_blob` rechecks that the same physical connection and registry binding are still current and reparses the complete profile before it can be committed active.

A disconnect/reconnect, a conflicting/newer connection epoch, or a registry rescan invalidates the outstanding ticket. Profile bytes whose validated metadata no longer matches the scanned registry entry are also rejected. This ports the released C manager's read-outside-lock / revalidate-before-commit invariant into the Rust domain layer without holding a lock across storage I/O.


### USB-MIDI to semantic-session ingest boundary

The controller session now accepts the transport-level `UsbMidiEvent` directly and converts only validated MIDI channel-voice packets into the existing S3CP runtime. USB framing therefore terminates before semantic scheduling/reconciliation; the profile/session layer never parses endpoint buffers itself.

S3CP v2 has no virtual-cable selector, so R1 fails closed for non-zero USB-MIDI cable numbers rather than silently merging multiple virtual cables. SysEx and other non-channel-voice CIN values remain ignored by the S3CP control runtime, consistent with ADR-004's explicit SysEx/MIDI-CI deferral. These outcomes are typed and host-tested.
