# Guition first-boot capture procedure

Date: 2026-10-20

Applies independently to JC1060P470C_I_W_Y and JC8012P4A1C_I_W_Y. Boards are in transit. This is a prepared procedure, with no hardware results recorded.

## Before replacing factory firmware

1. Photograph the unit, PCB revision/markings, panel label and exposed connectors. Keep a separate record per model/unit.
2. Capture the factory boot/service log and current firmware identity. Record observed chip revision, flash ID/capacity/mode and PSRAM size/mode when available; an unavailable reading stays unknown rather than inheriting the profile's 16 MiB/32 MiB values.
3. Establish the actual ROM/service/recovery connector and a repeatable reset/bootloader procedure. Distinguish native USB Serial/JTAG from a USB-UART bridge. Preserve factory firmware by a verified readback/backup using the observed flash geometry, and hash that backup.
4. Reconcile the package/pad map and power path with the [hardware matrix](GUITION_HARDWARE_MATRIX_2026-10-20.md). Confirm the recovery path and flash layout before any erase/write. Do not reuse the historical custom-M1 `partitions.csv`/`espflash.toml` as Guition instructions.

## Compile the minimal diagnostic image

From `firmware-rust/apps/m1-firmware`, using the pinned toolchain and target:

```sh
M1_IMAGE_ID="$(git rev-parse HEAD)" cargo +1.95.0 build --locked --release --features board-jc1060,board-diagnostics
M1_IMAGE_ID="$(git rev-parse HEAD)" cargo +1.95.0 build --locked --release --features board-jc8012,board-diagnostics
```

Build each separately and preserve/hash each ELF before the next configuration overwrites the target output filename. Record dirty-tree state and any additional build inputs; an embedded source label alone is not proof of image identity. GitHub's diagnostic release builds inject `github.sha`; local builds without `M1_IMAGE_ID` print `UNSPECIFIED` and cannot satisfy the image-identity capture requirement.

This is a compile procedure, not a flash command. Select a board-specific image/partition and programming method only after the observations above. No firmware was flashed in preparing this gate.

## What this image can observe

`board-diagnostics` emits `M1_BOOT_DIAGNOSTIC_V1` over the P4 native USB Serial/JTAG service peripheral. Its bounded task repeats the report without blocking indefinitely if the host is absent. Confirm that the exact unit exposes that peripheral; JC8012's CH340 USB-UART connection is a different transport. If native service is not reachable, prepare a separately verified UART/service adapter instead of guessing pins.

The report distinguishes:

- selected board profile from physical unit identity;
- detected eFuse revision from the required minimum 302;
- documented flash/PSRAM capacities from `NOT_MEASURED` observations;
- a diagnostic boot from `readiness=NOT_QUALIFIED`.

The image initializes only HAL/scheduler and the explicitly selected service transport. It contains no flash erase/write, PSRAM allocator, panel/touch initialization, SD, host USB, audio DAC or amplifier activation.

## Record after the first authorized flash

Capture the source commit and dirty state, build command/environment, exact ELF and flashed-image hashes, programming log, complete boot report and reset reason. Test cold boot, warm reset, service disconnect/reconnect and ROM recovery. Repeated expected logs are not by themselves a PASS: record actual results and any missing observations.

Next add read-only flash identity and separately qualified memory probes, then one peripheral adapter at a time. Establish USB host power/backfeed behavior before attaching a controller or memory stick as a powered host. The MAIN DAC/controller fallback and controller-only headphone routes require later physical audio qualification.
