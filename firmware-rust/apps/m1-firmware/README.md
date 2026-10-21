# Guition ESP32-P4 firmware

This standalone embedded workspace pins Rust 1.95.0, esp-hal 1.2.2 and target
`riscv32imafc-unknown-none-elf`. The minimum chip revision is 302.

## Board selection

`default = []` runs the scheduler only. Select at most one board:

- `board-jc1060`: JC1060P470C_I_W_Y.
- `board-jc8012`: JC8012P4A1C_I_W_Y.

Each selector starts an idle BSP unless diagnostics are explicitly requested.
No product storage, USB host, audio, display or touch owner is spawned.

From this directory:

```sh
cargo +1.95.0 build --locked --release --features board-jc1060
cargo +1.95.0 build --locked --release --features board-jc8012,board-diagnostics
```

No board programming layout is selected. Use the
[unit procedure](../../../docs/boards/GUITION_FIRST_BOOT_2026-10-20.md) before
choosing a service connector or programming method.

## Diagnostic variants

| Feature | Behavior |
|---|---|
| `board-diagnostics` | Immutable boot/service snapshot through native USB Serial/JTAG. |
| `guition-flash-probe` | Adds bounded, RAM-resident, read-only RDID; retains raw ID and decoded capacity. |
| `guition-psram-test` | Release-only autodetection at 80 MHz and a reserved 64 KiB CPU-cached scratch test. No heap-region registration. |

Probe features imply diagnostics and require a Guition selector. Ordinary
diagnostics activates neither probe. `M1_BOOT_DIAGNOSTIC_V1` and
`M1_MEMORY_DIAGNOSTIC_V1` stay separate from the shared versioned service snapshot.
Repeated boot records are not live heartbeats or readiness acceptance.
Timeout, unknown ID, mapping/capacity mismatch and scratch failure remain explicit.
PSRAM initialization faults can reset before a report. Scratch success does not
prove physical traffic through cache, DMA coherence or full-array stability.

The packaging tool provides basic, flash, PSRAM and combined-memory variants
independently for each board. It retains exact build inputs/image IDs/ELF/ZIP
hashes. Capture/ingestion retain raw failures and reject inconsistent records.

## Compile-only product integration

`integration-check` enables compilation of reusable owner modules, without taking
their physical resources, initializing a product heap or spawning their tasks.
An image built with it runs only the scheduler. It cannot accompany a Guition
selector and must not be presented as a working product firmware image.

```sh
cargo +1.95.0 check --locked --features integration-check,render-graph-bringup,sdmmc-profile-storage-bringup,runtime-health
cargo +1.95.0 check --locked --features integration-check,apta-usb0-bringup
```

The owner features preserve bounded Library/deck/controller/media, WAV/MP3/FLAC,
render/FX/PFL, persistence and optional analysis integration for compilation and
host composition tests. They require `integration-check` until a separate Guition
runtime gate qualifies the required resources.

Resource acquisition is explicit: the controller owner accepts a prepared USB
instance; SD profile storage accepts an SD slot and BSP power/presence policy;
Hot Cue persistence accepts a validated `JournalFlashLayout`, with no default
capacity, table location or journal address. Actual partition entries and their
overlap are still checked before opening persistent storage.

## Remaining work

Physical first boot/recovery and observed memory identity, Guition SD/USB power
and pin binding, display/touch ports and framebuffer/cache ownership, product
allocator, controller UAC and stereo DAC output remain unqualified. Inactive
panel/touch contracts retain source fingerprints, explicit unit/table/payload
selection, initialization order and cleanup/restart handling. No GSL3680 payload
is bundled. Headphones always use the controller.

See [architecture](../../../docs/M1_RUST_ARCHITECTURE.md),
[development](../../../docs/DEVELOPMENT.md) and [validation](../../../docs/VALIDATION.md).
