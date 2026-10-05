# m1-firmware

Native ESP32-P4 Pajoniiir-M1 production firmware target.

The firmware is a **standalone Cargo workspace** nested under `firmware-rust`.
It is intentionally excluded from the host workspace dependency graph so
`esp-hal` and its unstable P4 APIs cannot leak into host-tested product crates.

## Embedded baseline

- Rust: `1.95.0`
- Target: `riscv32imafc-unknown-none-elf`
- HAL: `esp-hal = 1.2.2`
- Chip feature: `esp32p4`
- Supported silicon baseline: ESP32-P4 revision v3.2 or newer (`ESP_HAL_CONFIG_MIN_CHIP_REVISION=302`)
- `esp-hal/unstable` is enabled only by this final firmware target.
- USB OTG host and MIPI-DSI stay behind the M1 platform/BSP boundary.

## Current gate

`src/main.rs` is deliberately minimal: it initializes `esp-hal` and creates
compile-time anchors to the HAL-independent product crates. This gives CI a real
ESP32-P4 architecture gate without pretending that USB, display, audio or board
pinout have been hardware-qualified.

The dedicated `Rust ESP32-P4 compile gate` workflow generates a firmware lock
artifact and cross-compiles this target. Passing that workflow means
**compile-verified for ESP32-P4**, not hardware-verified.

Hardware verification remains blocked until the custom Pajoniiir-M1 board with a
supported P4 revision is available.


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

This gate is **COMPILE_VERIFIED / HARDWARE_PENDING**. The target now includes storage-backed profile activation and USB-MIDI Bulk-IN pumping through the active S3CP profile plus `ControllerSession`, guarded by controller epochs and active-profile identity. It still does not claim physical VBUS switching/fault recovery, real controller MIDI interoperability, product-side `SessionDelivery` execution, LED output, UAC isochronous streaming or FLX4 qualification. CI also links USB0 HS + USB1 FS + SDMMC + PSRAM together so ownership collisions are caught before EVT hardware exists.


### USB-MIDI transport boundary

The USB1 controller layer now contains a compile-verified USB-MIDI 1.0 bulk transport. The host-tested `pajoniiir-controller-usb` crate locates the first alternate-setting-zero Audio/MIDIStreaming interface, preserves bulk endpoint addresses/max-packet sizes, and encodes/decodes standard 4-byte USB-MIDI event packets without allocation.

On ESP32-P4 the USB1 owner allocates typed Embassy `Bulk/In` and optional `Bulk/Out` pipes for that interface using the enumerated route/split metadata. Full-Speed bulk packet sizes are fail-closed above 64 bytes. While a matched profile is `Active`, the owner races device events, profile/catalog changes and cancellation-safe Bulk-IN reads, then forwards each complete transfer through the immutable active S3CP blob and `ControllerSession`. The HAL-independent session crate host-tests multi-packet USB-MIDI batching, invalid CIN handling, non-zero virtual-cable rejection and SysEx/non-channel-voice ignore behavior. Product-side `SessionDelivery` dispatch and LED feedback on Bulk-Out remain follow-on software gates; physical MIDI interoperability remains `HARDWARE_PENDING`.


### microSD / controller-profile storage ownership boundary

Feature `sdmmc-profile-storage-bringup` now binds the M1 board-authoritative ESP32-P4 SDMMC slot-0 path: `GPIO39..42 = D0..D3`, `GPIO43 = CLK`, `GPIO44 = CMD`, `GPIO45 = SD_PWR_EN`, and `GPIO46 = SD_CARD_DETECT`. The task uses the native `esp-hal 1.2.2` `SdHostController`, converts slot 0 to interrupt-driven async mode, waits for the controlled 3V3_SD rail to settle, and constructs the official `sdio 0.5.1` SD-card block device. Initialization uses the conservative 25 MHz product baseline and the recoverable `new_uninit_sd_card + reacquire` path so a failed attempt does not discard bus ownership.

This remains **COMPILE_VERIFIED / HARDWARE_PENDING**. The target now continues past card initialization: it reads the partition table, accepts FAT32 superfloppy/MBR media, validates the FAT32 boot sector + FSInfo, mounts the native async FAT32 filesystem and scans the released `/controllers/<id>/profile.s3bin` layout into a bounded `ProfileRegistry`. The 16 KiB profile blob scratch and maximum filename scratch live in one static `ProfileScanBuffers` workspace rather than the Embassy task state. The completed registry is published to USB1 through a revisioned bounded catalog and live controller VID/PID matches are refreshed when that catalog changes. When USB1 requests activation, the SDMMC owner reloads the exact scanned profile into the same bounded scratch, then the activation ticket and full blob are revalidated before an immutable active-profile copy is published. GPT discovery, physical rail timing, hot-plug/remove/reinsert and rail power-cycle recovery remain explicit follow-on gates. CI additionally links SDMMC together with USB0 HS, USB1 FS and PSRAM so peripheral/pin ownership conflicts are caught before EVT hardware exists.


### SDIO owned-block adapter

The microSD target now has a zero-allocation adapter from the initialized `sdio::BlockDevice<Card, ..., 512>` into Pajoniiir's existing `OwnedBlockDevice` contract. Its caller-owned sector buffer is a 4-byte-aligned 512-byte value matching the `block-device-driver` requirement, and the geometry is frozen from the SD card's reported block count after successful initialization.

This deliberately reuses the same native async FAT32 path already exercised by USB0 instead of introducing a second filesystem implementation for controller profiles. The adapter, FAT32 mount and controller-profile scan are compile gates only; real sector reads, card interoperability and media-error recovery remain hardware-pending.
