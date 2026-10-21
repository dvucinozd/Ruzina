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


## Repeatable per-model packages and observation capture

The software-only package tool builds both profiles separately, immediately
copies each ELF before the next build, verifies its embedded image ID and records
raw SHA-256 hashes. Run with the pinned Cargo/rustc on PATH, from any directory:

```sh
python firmware-rust/tools/guition_first_boot.py build --output /absolute/new/firstboot --target-dir /absolute/cargo-target
```

The output must be new. Each `jc1060/` and `jc8012/` contains `firmware.elf`,
`build.log`, `manifest.json` and instructions, plus a separate ZIP and SHA-256
sidecar. Source commit, dirty state, every firmware input hash, compiler, selected
build environment, command and working directory are retained. The image ID also
binds the source/compiler/environment digest and board. Build failure or input
changes leave BUILD_FAILED evidence rather than a completed package. Cargo runs
inside the firmware directory so the pinned target/linker and silicon config are
actually used. CI retains these build-only packages separately from qualification.

The packages provide no flash command or Guition partition assumption. Perform
the unit identification, factory backup and recovery steps above first. An ELF
hash and matching boot label do not by themselves establish the exact bytes
programmed into a unit; retain the programming/readback evidence separately.

Retain one existing plain/raw terminal log per exact unit and scenario:

```sh
python firmware-rust/tools/guition_first_boot.py ingest --package /absolute/firstboot/jc1060 --log /absolute/cold-boot.log --output /absolute/new/unit-A-cold-boot --unit unit-A --scenario cold-boot
```

`capture` accepts the same package/output/unit/scenario options plus an explicit
`--port COMx` (or Linux device), `--seconds 30`, and optional `--baud 115200`.
Only live capture requires pyserial; build and ingest use Python's standard
library. Capture never discovers ports, writes bytes, sends reset commands or
flashes. It deasserts DTR/RTS before opening, but opening a serial driver can still
affect a reset circuit: establish the exact service connector first. Native USB
Serial/JTAG and JC8012's CH340 UART are distinct paths. Serial failure retains
partial bytes and an explicit capture error. Time and data are bounded to 300 s
and 1 MiB.

Every output retains `raw.log` and `result.json`, package/image/log hashes, unit,
scenario, complete and partial record counts, raw reset reasons, ROM signature
presence and parsed identity/memory observations. Matching a complete record
requires the exact model/image, minimum revision policy and documented capacities.
Wrong/mixed identities, duplicate/unknown fields or inconsistent measurements fail
closed. Unmeasured values remain NOT_MEASURED. A trailing partial repeated frame
is counted and never treated as a complete record. Missing/mismatched boot records
return nonzero for application-boot scenarios.

`factory` and `rom-recovery` can retain logs without an application boot report;
their lack of that report is not a failure or a recovery PASS. A ROM marker is
only a captured signature. Optional `--operator-result PASS|FAIL` requires an
`--operator-note`; operator evidence stays separately attributed. All scenarios
retain `scenario_acceptance=NOT_QUALIFIED` until a separate physical qualification
review. No power cycle, reset, reconnect or recovery is inferred from repeated
application output. The unit/scenario directories cannot be overwritten.
