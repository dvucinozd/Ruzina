# m1-firmware

Native ESP32-P4 Pajoniiir-M1 firmware integration target.

Active development boards are Guition **JC1060P470C_I_W_Y** and **JC8012P4A1C_I_W_Y** with owner-reported v3.2 silicon. Source-audited profiles and scheduler-only BSP skeletons are implemented; peripheral drivers and hardware qualification are pending. The feature descriptions below record the historical custom-M1 wiring and memory assumptions, including USB FS GPIO27/26, microSD GPIO39..46, 16 MiB flash and the Hot Cue partition. Reconcile those assumptions against each exact board before implementing Guition hardware features. See the [hardware matrix](../../../docs/boards/GUITION_HARDWARE_MATRIX_2026-10-20.md) and [platform decision](../../../docs/M1_GUITION_PLATFORM_DECISION_2026-10-20.md).

## Board selection

`default = []` remains a generic scheduler-only P4 target. Choose at most one of:

- `board-jc1060`: JC1060 candidate profile, no peripheral activation.
- `board-jc8012`: JC8012 candidate profile, no peripheral activation or touch payload.
- `board-legacy-m1`: historical custom-board peripheral ownership and wiring.

Every existing USB/SD/flash/PSRAM bring-up feature now requires `board-legacy-m1`; indirect feature dependencies are checked too. A Guition selector combined with those features is a compile-time error. For example, the historical render check is now `cargo +1.95.0 check --locked --features board-legacy-m1,render-graph-bringup`. Guition profile builds use `cargo +1.95.0 build --locked --release --features board-jc1060` or `board-jc8012` alone. These are compile instructions; no Guition flash/partition configuration is selected.

The firmware is a **standalone Cargo workspace** nested under `firmware-rust`.
It is intentionally excluded from the host workspace dependency graph so
`esp-hal` and its unstable P4 APIs cannot leak into host-tested product crates.

## Embedded baseline

The opt-in `board-diagnostics` feature can accompany `board-jc1060` or `board-jc8012`. It reports profile/image identity, detected chip revision and reset reason through native USB Serial/JTAG, with unmeasured flash/PSRAM explicitly marked. It performs no storage or audio activation. Check the [first-boot procedure](../../../docs/boards/GUITION_FIRST_BOOT_2026-10-20.md) before selecting a service connector, image or partition layout.

- Rust: `1.95.0`
- Target: `riscv32imafc-unknown-none-elf`
- HAL: `esp-hal = 1.2.2`
- Chip feature: `esp32p4`
- Supported silicon baseline: ESP32-P4 revision v3.2 or newer (`ESP_HAL_CONFIG_MIN_CHIP_REVISION=302`)
- `esp-hal/unstable` is enabled only by this final firmware target.
- USB OTG host and MIPI-DSI stay behind the M1 platform/BSP boundary.

## Current gate

`src/main.rs` initializes `esp-hal` and dispatches the selected platform. The
platform starts the scheduler; only the explicit legacy selector starts the
historical feature-gated product owners. The default image does not start USB or microSD owners. USB, display,
audio and the selected boards' pinouts remain hardware-pending.

The dedicated `Rust ESP32-P4 compile gate` workflow uses the committed firmware
lockfile with `--locked` and cross-compiles/links this target. Passing that workflow means
**compile-verified for ESP32-P4**, not hardware-verified.

Hardware verification proceeds per selected Guition board after its BSP and
exact-unit identity are established. Custom PCB fabrication is no longer a prerequisite.


## Canonical audio integration

The legacy `render-graph-bringup` feature now consumes bounded canonical PCM history for normal playback, Master Tempo, scratch and censor. Prefetch is separated from progress and exact source activation; source/gesture epochs guard load/seek/reconnect transitions. History and fixed-depth PCM/delivery queues use static memory. Render output passes through `PlaybackOutput`; progress is published only after reference consumption completes. The explicit reference sink remains software-timed, with no physical I2S/UAC output. See [completed-output progress and remaining gates](../../../docs/M1_RUST_PLAYBACK_PROGRESS_GATE_2026-10-21.md).

## PSRAM storage boundary

Feature `p4-psram-storage` enables the compile-verified ESP32-P4 PSRAM placement boundary. It initializes the PSRAM peripheral through `esp-hal` and registers the mapped region with `esp-alloc = 0.11.0` as external heap storage.

This gate is intentionally narrower than full product-storage qualification: the three 512-entry Rekordbox name maps and two recyclable 1024-track catalog banks now use this heap when the feature is enabled. Cache coherency, DMA suitability, sustained access behavior and the custom M1 board's physical PSRAM remain `HARDWARE_PENDING` until EVT hardware exists.

CI checks the feature independently and also links it together with `usb0-hardware-bringup` so PSRAM and USB0 peripheral ownership cannot silently diverge at compile time.


### Bounded Rekordbox name staging

When `p4-psram-storage` is enabled, the USB0 Library worker now allocates three bounded PSRAM-backed `Vec<PdbNameEntry>` stores for Artists, Albums and Keys. Each store reserves the released 512-entry limit before PDB processing starts, so page walking does not trigger heap growth. Allocation failure is reported as a Library failure, duplicate retained IDs fail closed, and capacity overflow remains observable through the existing probe flag. The stores are cleared on failed import, filesystem failure and detach, while their reserved PSRAM capacity is retained for the next media generation.

Without `p4-psram-storage`, the same parser path remains probe-only and does not retain name metadata.


### Transactional PSRAM catalog publication

With `p4-psram-storage`, the Library worker reserves two 1024-track PSRAM banks before import. Artists, Albums and Keys are scanned first; Tracks are then converted through the provider-neutral Rekordbox adapter into `CatalogTrack` values in the inactive bank. A new snapshot is published only after the complete PDB scan and file close succeed. Parse, capacity, duplicate-ID, UTF-8 or close failures abort the build and recycle the inactive bank without exposing a partial catalog. A detach or failed filesystem generation invalidates the matching published snapshot and returns its bank to the two-bank pool.


## USB1 controller ownership boundary

Feature `usb1-controller-bringup` owns the ESP32-P4 Full-Speed OTG controller through `USB_FS` with the M1 board-authoritative pin order `GPIO27 = DP`, `GPIO26 = DM`. The persistent Embassy task enumerates the root-port device and publishes a bounded capability snapshot derived from the configuration descriptor. It also matches the enumerated VID/PID against the latest microSD-backed `ProfileRegistry` and records the matched profile index plus catalog revision. Registry publication is signalled, so a controller already connected before the microSD scan completes is rematched without requiring a USB reconnect.

Descriptor classification is implemented in the HAL-independent `pajoniiir-controller-usb` crate and is host-tested. It recognizes USB Audio Control, Audio Streaming, MIDI Streaming, HID and vendor interfaces, and records the relevant bulk/interrupt/isochronous endpoint directions without binding product behavior to a single VID/PID.

This gate is **COMPILE_VERIFIED / HARDWARE_PENDING**. The target now includes storage-backed profile activation, USB-MIDI Bulk-IN pumping through the active S3CP profile plus `ControllerSession`, bounded `SessionDelivery` dispatch into authoritative Deck/Mixer state, retry-safe Deck-effect owner lanes, and semantic LED feedback through the active profile to optional USB1 Bulk-Out. Controller epochs and active-profile identity guard both ingress and feedback. It still does not claim physical VBUS switching/fault recovery, real controller MIDI/LED interoperability, UAC isochronous streaming or FLX4 qualification. CI also links USB0 HS + USB1 FS + SDMMC + PSRAM together so ownership collisions are caught before EVT hardware exists.


### USB-MIDI transport boundary

The USB1 controller layer now contains a compile-verified USB-MIDI 1.0 bulk transport. The host-tested `pajoniiir-controller-usb` crate locates the first alternate-setting-zero Audio/MIDIStreaming interface, preserves bulk endpoint addresses/max-packet sizes, and encodes/decodes standard 4-byte USB-MIDI event packets without allocation.

On ESP32-P4 the USB1 owner allocates typed Embassy `Bulk/In` and optional `Bulk/Out` pipes for that interface using the enumerated route/split metadata. Full-Speed bulk packet sizes are fail-closed above 64 bytes. While a matched profile is `Active`, the owner races device events, profile/catalog changes and cancellation-safe Bulk-IN reads, then forwards each complete transfer through the immutable active S3CP blob and `ControllerSession`. The HAL-independent session crate host-tests multi-packet USB-MIDI batching, invalid CIN handling, non-zero virtual-cable rejection and SysEx/non-channel-voice ignore behavior. Product delivery uses real accept/retry semantics: Deck/Mixer state is committed only after downstream effect capacity is reserved, and transport/persistence/audio-DSP effects remain queued until their future owners acknowledge them. Semantic LED state is dirty/resync tracked and mapped through S3CP to Bulk-Out; a profile change, disconnect or failed OUT forces resync. Physical MIDI/LED interoperability remains `HARDWARE_PENDING`.


### microSD / controller-profile storage ownership boundary

Feature `sdmmc-profile-storage-bringup` now binds the M1 board-authoritative ESP32-P4 SDMMC slot-0 path: `GPIO39..42 = D0..D3`, `GPIO43 = CLK`, `GPIO44 = CMD`, `GPIO45 = SD_PWR_EN`, and `GPIO46 = SD_CARD_DETECT`. The task uses the native `esp-hal 1.2.2` `SdHostController`, converts slot 0 to interrupt-driven async mode, waits for the controlled 3V3_SD rail to settle, and constructs the official `sdio 0.5.1` SD-card block device. Initialization uses the conservative 25 MHz product baseline and the recoverable `new_uninit_sd_card + reacquire` path so a failed attempt does not discard bus ownership.

This remains **COMPILE_VERIFIED / HARDWARE_PENDING**. The target continues past card initialization: it discovers FAT32 superfloppy, MBR and bounded GPT media, validates GPT header CRC32 and the complete partition-entry-array CRC32 before accepting GPT candidates, validates the FAT32 boot sector + FSInfo, mounts the native async FAT32 filesystem and scans the released `/controllers/<id>/profile.s3bin` layout into a bounded `ProfileRegistry`. The 16 KiB profile blob scratch and maximum filename scratch live in one static `ProfileScanBuffers` workspace rather than the Embassy task state. The completed registry is published to USB1 through a revisioned bounded catalog and live controller VID/PID matches are refreshed when that catalog changes. When USB1 requests activation, the SDMMC owner reloads the exact scanned profile into the same bounded scratch, then the activation ticket and full blob are revalidated before an immutable active-profile copy is published.

GPIO45 is now owned explicitly as the 3V3_SD rail control and GPIO46 as the active-low card-detect input. Card removal first publishes an empty profile registry so USB1 cannot continue using an active profile from absent media, then dismantles the filesystem/partition stack, recovers the same SDIO device owner, powers the rail down and later uses `reacquire()` after a debounced reinsertion and controlled rail cycle. The current 10 ms power-up settle, 20 ms rail-off and 50 ms detect-debounce values are software baselines only; their electrical adequacy remains `HARDWARE_PENDING`. CI additionally links SDMMC together with USB0 HS, USB1 FS and PSRAM so peripheral/pin ownership conflicts are caught before EVT hardware exists.


### SDIO owned-block adapter

The microSD target now has a zero-allocation adapter from the initialized `sdio::BlockDevice<Card, ..., 512>` into Pajoniiir's existing `OwnedBlockDevice` contract. Its caller-owned sector buffer is a 4-byte-aligned 512-byte value matching the `block-device-driver` requirement, and the geometry is frozen from the SD card's reported block count after successful initialization.

This deliberately reuses the same native async FAT32 path already exercised by USB0 instead of introducing a second filesystem implementation for controller profiles. The adapter, FAT32/GPT discovery, mount, profile scan, activation reload and remove/reinsert recovery are compile-verified software gates only; real sector reads, signal integrity, rail timing, card-detect behavior and card interoperability remain hardware-pending.


## Internal flash / Hot Cue persistence

The M1 schematic specifies a W25Q128JVPIQ 128-Mbit (16 MiB) QSPI flash. Native
Rust therefore reserves the final 1 MiB exclusively for the Hot Cue journal:

- partition table: `0x00008000`
- factory application: `0x00010000..0x00F00000`
- `hotcue` data partition: `0x00F00000..0x01000000` (1 MiB)
- custom data subtype: `0x40`

`partitions.csv` is the single source of truth and is referenced directly by
`espflash.toml`. Current `espflash` accepts CSV partition tables and performs
the required binary conversion during image generation/flashing, so no stale
generated `partitions.bin` is stored in the repository.

The `hotcue-flash-storage` firmware feature uses `esp-storage = 0.10.0`. It
does not trust the compile-time constants alone: before the first Hot Cue erase
or write, the target reads the flashed partition table at `0x8000` and requires
a non-encrypted `hotcue` data entry with exactly the expected subtype, offset
and size. A missing or mismatched table permanently faults the persistence owner
for that boot and leaves the queued Deck effect unacknowledged.

The journal itself uses 128 hash buckets with two 4 KiB sectors per bucket.
Entries and replacement-sector headers have separate commit words written last.
An interrupted append is ignored; an interrupted compaction leaves the previous
committed generation authoritative. Committed CRC/format corruption fails closed.

USB1 controller bring-up implies `hotcue-flash-storage`, so a controller Hot Cue
change is dequeued only after the persistence owner reports a successful durable
journal operation. Target load/merge on track load remains part of the subsequent
Library/deck-load ownership gate.
