# Guition first boot and diagnostic capture

Updated: 2026-10-21. Applies independently to JC1060P470C_I_W_Y and
JC8012P4A1C_I_W_Y. Boards are in transit; no hardware results are recorded.

## Before replacing factory firmware

1. Record unit/model, PCB/module revision, panel markings and connector photos.
2. Capture factory boot/service logs, firmware identity and reset cause. Read
   actual chip revision, flash identity/capacity/mode and PSRAM size/mode when
   available. Unknown readings stay unknown.
3. Establish the exact ROM/service/recovery connector and repeatable reset/
   bootloader entry. Native USB Serial/JTAG and a USB-UART bridge are different
   transports. Preserve the factory image by verified readback with observed
   geometry, and retain its SHA-256.
4. Reconcile the [matrix](GUITION_HARDWARE_MATRIX_2026-10-20.md), power path,
   actual panel and programming layout before any erase/write.

The project provides no board programming command or partition layout. These
must be selected from exact-unit observations and a separate reviewed procedure.

## Build separate packages

Use pinned Rust/Cargo 1.95.0 and target `riscv32imafc-unknown-none-elf` on PATH.
From the repository root, with a NEW output directory:

```sh
python3 firmware-rust/tools/guition_first_boot.py build --output /absolute/new/basic --target-dir /absolute/cargo-target
python3 firmware-rust/tools/guition_first_boot.py build --probe flash --output /absolute/new/flash --target-dir /absolute/cargo-target
python3 firmware-rust/tools/guition_first_boot.py build --probe psram --output /absolute/new/psram --target-dir /absolute/cargo-target
python3 firmware-rust/tools/guition_first_boot.py build --probe memory --output /absolute/new/memory --target-dir /absolute/cargo-target
```

Each run builds both models separately and immediately preserves each ELF. Its
manifest records source/compiler/build environment/command, selected features,
image ID and exact ELF/ZIP/log hashes. Inputs changing during compilation leave
BUILD_FAILED evidence. Outputs cannot be overwritten. Image labels bind source,
compiler/environment digest, board and probe variant. These are BUILD_ONLY
packages, not programmed images or releases.

Basic diagnostics initializes HAL/scheduler and native USB Serial/JTAG only.
Confirm that the unit exposes it; JC8012's CH340 UART is a separate connection.
If unavailable, a separately verified transport adapter is needed.

Flash probe performs bounded, RAM-resident read-only RDID and preserves the raw
24-bit register-order ID and supported decoded density. It does not program,
erase, unlock or select a storage layout. Busy/timeout/unknown encodings remain
explicit and never inherit a profile capacity.

PSRAM is a separate release-only image. It autodetects using the pinned HAL at
80 MHz, rejects invalid/mismatched mappings and overwrites only an exclusive
64 KiB CPU-cached scratch span. It registers no allocator and starts no product
or DMA owners. CPU_SCRATCH_PASSED is not full-memory/cache/DMA/retention or
production-clock acceptance. HAL initialization faults may reset before a report.

## Capture one unit and scenario

Retain an existing raw log:

```sh
python3 firmware-rust/tools/guition_first_boot.py ingest --package /absolute/basic/jc1060 --log /absolute/cold-boot.log --output /absolute/new/unit-A-cold-boot --unit unit-A --scenario cold-boot
```

Optional `capture` uses the same package/output/unit/scenario arguments, an
explicit `--port COMx` or Linux device, `--seconds 30` and optional baud. Only
live capture needs pyserial. It does not scan ports, write bytes, issue reset
commands or flash. DTR/RTS are deasserted before open, but the selected driver's
effect on the actual reset circuit must be established. Capture is bounded to
300 seconds and 1 MiB; serial failure retains partial bytes and its error.

Every output retains raw.log and result.json, package/image/log hashes, unit,
scenario, complete/partial frames, reset reasons and ROM signature observations.
`M1_BOOT_DIAGNOSTIC_V1`, the shared service snapshot and
`M1_MEMORY_DIAGNOSTIC_V1` are separate records. Matching a package requires its
exact model/image/minimum revision and consistent memory/probe observations.
Duplicate/unknown fields, failure states, inconsistent RDID density, wrong
capacity/clock/span and mixed images fail closed. Original boot-only packages
remain readable. Trailing partial frames are counted, not accepted as complete.

Scenarios: factory, cold-boot, warm-reset, service-reconnect and rom-recovery.
Factory/recovery logs may lack application frames; that is not a recovery PASS.
Optional operator PASS/FAIL requires an evidence note. All generated records
retain scenario_acceptance=NOT_QUALIFIED until a separate physical review.
Repeated boot snapshots are not a heartbeat or proof that a power cycle occurred.
A matching image label does not prove exact programmed bytes; preserve the
programming/readback evidence separately.

## Following gates

Qualify memory and allocator/cache ownership, then display/backlight/touch, SD,
USB power/data and audio per model. Panel/touch contracts require exact unit,
source/wiring and external table/payload selection. JC1060 touch pins remain
unresolved; JC8012 GSL3680 provisioning/distribution clearance is not established.
Stereo MAIN uses a ready external DAC with controller fallback. Headphones always
use the controller. Physical latency, listening and soak remain separate.
