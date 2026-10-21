# Guition flash observations, PSRAM scratch and display/touch adapters

Date: 2026-10-21. Base: `8bc11e3d06a6c63c1c0664086a1e8145da24abe8` (PR #20).

This completes the three approved software preparations for JC1060P470C_I_W_Y
and JC8012P4A1C_I_W_Y. No unit has been flashed or physically qualified. Normal
Guition selectors remain scheduler-only; `board-diagnostics` adds only the
existing service transport and immutable boot observations. Legacy owners,
audio routing, APTA activation and the historical flash layout are unchanged.

## Flash probe

`guition-flash-probe` implies `board-diagnostics` and requires a Guition selector.
It consumes the sole FLASH capability at boot, before RTOS/application tasks or
the second core, and uses the P4 SPI1 RDID command. The register-access mechanism
was checked against the pinned `esp-storage 0.10.0/src/hardware.rs`, rather than
assuming its public `capacity()` documentation describes P4's implementation.
Its P4 branch reads RDID; its ROM-global branch applies to ESP32. No new ROM ABI,
fixed address, bootloader-header capacity or flash-storage write API is used.

The read-only command runs in RAM under a critical section. A busy controller is
left alone. A finite 100,000-poll budget reports TIMEOUT instead of spinning
forever; this count is not a measured deadline. No guessed abort/reset is issued.
The raw 24-bit receive-register ID is retained, with manufacturer in the low byte.
Only supported density encodings (including the distinct 0x1f family) yield a
capacity. Unknown IDs/timeouts never substitute the documented 16 MiB capacity.
This is a decoded RDID capacity, not full-array readback or storage qualification.

The low-level P4 command/power/cache behavior still needs physical verification.
The optional image never unlocks, erases, programs or changes flash status/mode.

## PSRAM diagnostic image

`guition-psram-test` is Guition-only, implies diagnostics and rejects debug builds.
It uses the existing pinned `esp-hal 1.2.2` `Psram::new`/`PsramConfig` APIs with
`PsramSize::AutoDetect` and the explicit 80 MHz timing preset; it does not use the
250 MHz default or claim the vendor's documented 200 MHz operating point.

The sole PSRAM capability is initialized before application tasks. A missing,
unaligned, too-small, overflowing or unexpected mapping is reported without
constructing a memory slice or proceeding with a scratch write. A capacity
mismatch retains the HAL-mapped size, rather than silently forcing 32 MiB.
HAL initialization faults may reset before a report; absence of a report is not
a successful probe. The HAL's internal initialization loops are not given a
new timeout guarantee by the bounded scratch test.

For the expected mapping, the first 64 KiB is an exclusive disposable scratch
span. No PSRAM allocator, product buffer, display scanout or DMA owner is started.
Three complete write/read passes use complementary and address-dependent words;
volatile accesses retain the first mismatch index/expected/actual values.
The portable algorithm caps accesses even if supplied a larger span and rejects
an empty span. The physical adapter keeps the HAL owner alive throughout access.

`CPU_SCRATCH_PASSED` means the bounded CPU-visible test completed. Cache writeback,
external-memory traffic, full-array aliasing, retention, DMA coherence, bandwidth
and production-clock stability remain NOT RUN. Volatile operations alone do not
prove those properties. Neither this state nor a detected size qualifies a unit.

## Versioned capture and separate packages

The existing boot frame remains V1. A separate `M1_MEMORY_DIAGNOSTIC_V1` frame
retains board/image identity, flash state/raw ID/decoded size, PSRAM state/mapped
size, reserved scratch bytes, clock, failure details, explicit CPU-cached scope
and `readiness=NOT_QUALIFIED`. A test serializes the actual boot/service/probe
composition into the device's 2048-byte bound, including failure records.

The package tool defaults to `--probe none`. Build each desired variant into a
NEW output folder; the embedded image ID includes the selected variant:

```sh
python firmware-rust/tools/guition_first_boot.py build --probe flash --output /absolute/new/flash-probe --target-dir /absolute/cargo-target
python firmware-rust/tools/guition_first_boot.py build --probe psram --output /absolute/new/psram-test --target-dir /absolute/cargo-target
python firmware-rust/tools/guition_first_boot.py build --probe memory --output /absolute/new/combined-memory --target-dir /absolute/cargo-target
```

Each output keeps separate JC1060/JC8012 ELFs, manifests, source/compiler/build
inputs, logs, ZIPs and SHA-256 sidecars. These are BUILD_ONLY packages, with no
programming command or Guition partition assumption. Keep factory backup and
ROM recovery ahead of selecting any flash procedure.

Ingestion/capture retain raw bytes and parsed probe frames. New packages require
their versioned memory frame; original pre-probe packages remain readable.
Wrong/duplicate/unknown fields, mismatched image/model, timeout, unknown RDID,
failed scratch, inconsistent ID/density, wrong capacity/clock/span and disagreement
between boot/probe observations fail the observation check. Trailing partial
frames remain counted. Operator acceptance and physical scenarios are separate;
even matching records remain NOT_QUALIFIED. See the [first-boot procedure](boards/GUITION_FIRST_BOOT_2026-10-20.md).

## Display and touch preparation

`pajoniiir-board-profile::display` defines separate physical panel and touch ports
and two explicitly invoked, HAL-independent adapters. There is no Guition DSI,
GPIO/backlight or I2C activation in the running firmware. A port implementation,
framebuffer/cache ownership and real scheduler integration are still pending.

The panel adapter is bound to one immutable board profile. Before any port call,
it requires the matching board/controller, unit/panel markings, declared table
digest, explicit distribution/wiring evidence, finite reset timing and a bounded
external program. The digest is a caller-supplied provenance label, not an
on-device cryptographic verification of command bytes or a legal determination.
It sequences backlight OFF, PHY LDO3/2.5 V, reset, DSI configuration, command/delay
records, scanout, then backlight ON. All port failures attempt backlight-off,
stop, reset and power-down; failed cleanup blocks restart until cleanup succeeds.
Physical port implementations must honor their error and finite-delay contracts.

Candidate timings retain exact source IDs/hashes: JC1060 JD9165 uses 1024x600,
52 MHz, 750 Mbps, H 24/136/160, V 2/21/12; JC8012 JD9365 uses 800x1280,
70 MHz, H 20/20/40, V 4/10/30 and a 1000 Mbps macro in its pinned display header.
An explicit unit selection supplies the lane rate; candidate values are not
hardware acceptance. [Source/table fingerprints](boards/guition-display-sources-2026-10-21.json)
identify both owner-bundle arrays without copying their commands or payloads.
Both BSP files label their code Apache-2.0; exact fitted-panel selection is pending.

Touch initialization has its own validation, start/read/stop and failed-cleanup
state. The physical port must use a single shared I2C7/8 bus owner. JC1060 reset/
interrupt remain unresolved (schematic 22/21 versus BSP NC/NC); no defaults drive
them. GT911 address strap and coordinate calibration require unit evidence.
JC8012's 22/21 and GSL3680 address 0x40 remain source candidates. GSL3680 requires
an explicitly provisioned, digest-labelled and distribution-authorized payload;
none is bundled. Local GSL code/payload distribution clearance is not established.
Raw touch samples are not automatically scaled/rotated into UI coordinates.

## Validation and remaining physical gates

Host regressions cover RDID encoding/unknown IDs, stuck bits/address aliasing,
empty/oversized scratch bounds, output-buffer bounds, panel cross-model rejection,
no calls before validation, all failed panel phases and cleanup/restart, and touch
selection/payload/cleanup. Python regressions exercise package isolation and raw
capture retention plus the memory-frame failures described above.

CI adds independent check/link gates for all three probe configurations on each
model, retains diagnostic packages even on failure, and expands invalid board/
debug combinations. Existing legacy/default build gates remain in place.
Recorded local execution and review evidence accompany the final change.

Still NOT RUN: flash RDID on a physical unit, PSRAM initialization/scratch at
80 MHz, factory readback and actual recovery, DSI/backlight/touch behavior,
USB host power/pin mapping, SD, audio, listening and soak. The original APTA
branch stays unmerged; headphones remain controller-only.
