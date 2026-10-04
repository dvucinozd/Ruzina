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

This gate is intentionally narrower than a product-storage implementation: the 1024-track catalog banks and three 512-entry Rekordbox name maps are not yet allocated from this heap. The feature proves API/link integration only. Cache coherency, DMA suitability, sustained access behavior and the custom M1 board's physical PSRAM remain `HARDWARE_PENDING` until EVT hardware exists.

CI checks the feature independently and also links it together with `usb0-hardware-bringup` so PSRAM and USB0 peripheral ownership cannot silently diverge at compile time.
