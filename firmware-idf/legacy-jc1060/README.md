# JC1060 legacy ESP-IDF / Rust experiment

This separate experiment addresses the received unit on COM3, whose eFuse
revision is **103 (v1.3)**. It does not relax the native firmware's minimum
revision 302, replace its esp-hal/esp-rtos startup, or activate product owners.
See the [arrival evidence and assessment](../../docs/boards/JC1060_ARRIVAL_2026-10-30.md).

## Reproducible offline tools

S5 adds an independent `--variant memory-task` profile, selected by
`sdkconfig.memory.defaults`. Scalar/pure-RAM profiles continue to reject PSRAM.
Memory/task selects legacy 360 MHz, HEX PSRAM 80 MHz with capability allocation,
no XiP/rodata relocation, an exclusive 64 KiB scratch region, internal comparison
buffer, allocation-failure/heap guards, and same-core then dual-core FP workers.
The optional Rust `memory-task` feature links the existing audio EQ crate.
The FP hold loop keeps ft0/fa1 live, atomically publishes an active flag after
FP initialization, and executes four million exact f32 increments without
voluntary yields. Another same-core task must observe that flag before using
the FPU; zero mismatches and nonzero such observations are required.
The bounded C-owned stereo EQ buffers use a documented pointer/length ABI.
The 256-frame 44.1 kHz block benchmark includes EQ initialization and processing;
its preset deadline is 5804 us, with no audio/DMA qualification implied.

For an upgrade from the S4 baseline, `prepare_flash_plan.py` accepts
`--predecessor-plan /absolute/s04-plan/plan.json`. It verifies the predecessor
payload hashes and overlays them on the factory backup to construct the expected
live sectors and a recovery back to S4. Full-factory recovery remains a separate
origin 0 dump restore. Live sectors must match the expected predecessor before
any write. Neither build nor plan preparation communicates with a device.

Activate the pinned IDF environment (GCC esp-15.2.0_20251204, esptool 5.6.0,
Rust 1.95.0 with rustfmt/clippy and the P4 bare target). Use a NEW output directory:

```sh
python firmware-idf/legacy-jc1060/tools/build_legacy.py --idf "$IDF_PATH" --output /absolute/new/output --variant pure-ram
python firmware-idf/legacy-jc1060/tools/build_legacy.py --idf "$IDF_PATH" --output /absolute/new/flash-output --variant flash-build-only
python -m unittest discover -s firmware-idf/legacy-jc1060/tools/tests -v
python firmware-idf/legacy-jc1060/tools/validate_unit.py docs/boards/readiness/jc1060-20261030-com3.json
```

Windows can use `tools/build_legacy.ps1` with IdF/Output/Variant/Python parameters
or call Python directly. Optional `--wsl Ubuntu --cargo-home /linux/cache/cargo
--rustup-home /linux/cache/rustup` selects WSL Rust; paths are translated by
wslpath, while IDF uses Windows paths. All tools/build environment paths are
operator parameters. The builder always rebuilds the Rust archive first,
checks IDF/submodule cleanliness and pins, runs fmt/Clippy, validates the linked
ELF/header/map and records artifact/source hashes in `manifest.json`.
Generated binaries remain outside Git. Both variants are BUILD_ONLY.

`tools/run_ram_probe.py --image IMAGE --manifest MANIFEST` performs only offline
verification. A manifest must explicitly contain `variant: pure-ram`, expected
`bytes` and `sha256`. Obtain those from a reviewed package; do not auto-approve
an arbitrary image by computing its own hash immediately before execution.
The immutable bytes checked are passed to esptool 5.6.0 without reopening a path.
The CLI requires explicit execute/port/unit/output/reset-policy parameters for
any device access. Execution returns incomplete/failure unless scalar and
factory recovery observations both succeed; no hard reset occurs in cleanup.
The hold policy requires a separately recorded operator power disconnect.

Use the [offline recovery/layout analysis](../../docs/boards/JC1060_LEGACY_RECOVERY.md)
before preparing a hardware operation. The historical scripts and frozen ZIP
are preserved as evidence, not the recommended execution tools.

ESP-IDF **v6.1**, commit `fff9895c82d744c7237be8847347bdd1b07c6643`,
owns the bootloader, legacy memory map, runtime and console. The Rust static
library uses stable **1.95.0**, the existing
`riscv32imafc-unknown-none-elf` target and the actual `ruzina-deck` crate.
No new third-party Rust dependencies are needed. The C caller exercises the
scalar ABI with positive, zero, negative and clamped pitch values. The archive
has no HAL, executor, allocator, mutable global state or peripheral access.
Rust panics call C `abort`; no unwinding crosses the ABI.

From the repository root, with the pinned Rust toolchain installed:

```sh
cargo +1.95.0 build --locked --release --target riscv32imafc-unknown-none-elf --manifest-path firmware-idf/legacy-jc1060/rust-core/Cargo.toml --target-dir firmware-rust/target/legacy-jc1060/rust
```

Then activate a clean ESP-IDF v6.1 environment. Set `RUZINA_RUST_ARCHIVE`
to the absolute path to
`firmware-rust/target/legacy-jc1060/rust/riscv32imafc-unknown-none-elf/release/libruzina_legacy_idf_core_probe.a`:

```sh
idf.py -C firmware-idf/legacy-jc1060 -B firmware-rust/target/legacy-jc1060/idf -DRUZINA_RUST_ARCHIVE=/absolute/path/to/libruzina_legacy_idf_core_probe.a build
```

The CMake link orders IDF's GCC runtime before the Rust archive. The archive's
precompiled compiler-builtins contains some soft-float C helper objects, even
though its Rust objects use the same single-float ABI as IDF. IDF's matching
runtime must supply those helper symbols. Linker ABI checks remain enabled;
do not suppress mismatch errors or assume this small proof covers all DSP
or runtime helpers. The link map must identify every actually selected object.

For a separate **pure-RAM** build, append `sdkconfig.ram.defaults` after
`sdkconfig.defaults` in `SDKCONFIG_DEFAULTS`, with its own `SDKCONFIG` and build
directory. Pass absolute paths in the semicolon-separated list. This selects
`CONFIG_APP_BUILD_TYPE_RAM=y` and `CONFIG_APP_BUILD_TYPE_PURE_RAM_APP=y`;
the app omits flash access and reports `flash_bytes=NOT_MEASURED`.
IDF's automatically printed `write-flash` instructions are not applicable to
this RAM experiment. The observed run used esptool's `load_ram` operation
directly against ROM, after checking the image hash, revision range,
SRAM/SPM segment ranges and exact unit identity.

Use a new build/config directory for this experiment. Do not reuse a v3.x
sdkconfig. CMake rejects a non-P4 target, a nonlegacy revision range, PSRAM
activation or a CPU clock other than 360 MHz. The C source rejects other IDF
versions. The app itself refuses any observed revision other than 103 or
flash geometry other than 16 MiB in the flash-backed build. The pure-RAM variant
does not initialize or measure flash. None of these checks verifies the physical
model, panel, wiring or recovery path.

On 2026-10-30 the flash-backed image passed **BUILD/LINK ONLY**. The pure-RAM
image also executed on the v1.3 unit at a reported 360 MHz; four scalar calls
returned expected values twice. This is not a flashed or qualified product. PSRAM,
display, touch, SD, USB host, network, storage writes and audio are disabled.
Do not flash generated bootloader/partition/application files until the C01
backup, recovery and board-specific programming-layout review are complete.
The generated default IDF layout is not an approved replacement for factory
partitions. The scalar calls verify only this small ABI boundary;
they do not qualify DSP timing, FPU context switching or product behavior.

**Known recovery limitation:** after pure-RAM execution, the tested native USB
soft reset returned to ROM but normal flash boot repeated HP watchdog resets.
A subsequent ROM flash-attach also failed. Stop repeated software attempts and
perform a complete power disconnect before further probing. The owner did so;
ROM RDID and factory boot worked again, and the owner confirmed the factory
display. Cold recovery and its operator result are in the unit record. Do not
describe this experiment as warm-reset/reconnect-qualified or use it as the
product runtime foundation yet.

## Read-only media bridge contract

The opt-in Rust `media-bridge` feature implements a single-slot read protocol
and an `OwnedBlockDevice` client using the existing shared block/IO/session
crates. It does not enable an IDF peripheral or alter any diagnostic profile.
Reads are limited to 4096 bytes over 512-byte sectors. Invalid ranges/lengths
reject before enqueue. The worker owns a request until IO/DMA stops; completion
returns its original owned buffer on success, short read, error or stale lease.
Invalidation precedes teardown, and remount waits for the old slot to drain.

Canceling a submitted future retains its pending ticket in the client and its
buffer in the task/queue. Call `recover_pending()` before another read; dropping
the client requires the persistent owner to drain the completion. Dropping an
unpolled future drops its never-submitted buffer normally. The protocol provides
no write trait or persistence API and contains no unsafe code. Its synchronous
queue methods must be serialized by the future platform transport, with waker
registration/recheck and notification on completion/invalidation.

Reproduce the host and target type/build gates (Rust 1.95.0):

```sh
cargo +1.95.0 test --locked --manifest-path firmware-idf/legacy-jc1060/rust-core/Cargo.toml --features media-bridge
cargo +1.95.0 clippy --locked --manifest-path firmware-idf/legacy-jc1060/rust-core/Cargo.toml --features media-bridge --tests -- -D warnings
cargo +1.95.0 build --locked --release --manifest-path firmware-idf/legacy-jc1060/rust-core/Cargo.toml --features media-bridge --target riscv32imafc-unknown-none-elf
```

The separate `sd-rust-bridge` builder variant enables `idf-media-bridge` and
`CONFIG_RUZINA_SD_RUST_BRIDGE`. It supplies a concrete IDF owner task on core 0
and a diagnostic Rust client task on core 1. The C owner alone holds card/host/
LDO4 and a512-byte internal DMA-capable bounce buffer. Rust retains its own
owned 512-byte arrays and copies from the DMA-complete C buffer synchronously;
no Rust request/future/layout or retained buffer pointer crosses C. State uses
one shared IDF portMUX, with wake notifications delivered after unlocking.
The static task waker targets a persistent diagnostic task, never a freed TCB.
Only that dedicated client task waits on notifications; SDMMC never blocks a
shared async executor. This is a diagnostic task runner, not product execution.

The bridge repeats a Rust-owned read of raw sectors 0..7 and compares FNV1a on
each pass. It does not link FatFs, mount Rust exFAT or open the known fixture.
Read failure invalidates the lease before returning completion/host teardown.
Queued/active buffers remain in persistent Rust state; teardown is held if an
active request cannot be returned. Reinsertion requires app restart. Host
FFI shims exercise the Rust export boundary/concurrent protocol; actual IDF
SMP, DMA/cache and physical removal remain exact-image runtime gates. Do not
transfer C/FatFs file acceptance to this raw-block profile. The shared Rust
filesystem path and asynchronous product executor remain subsequent work.

`sd-rust-exfat` is a separate opt-in extension selecting the shared Rust
partition/exFAT reader and pure-Rust SHA-256 for the same deterministic fixture.
It retains the transport's one-sector C bounce buffer and uses a32768-byte
diagnostic client stack. MBR/superfloppy and bounded GPT discovery reuse the
shared partition parser; the chosen exFAT header/partition bounds and main boot
checksum must validate. No C/FatFs mount, file handle or write API is involved.
The file check covers every byte, full SHA-256, EOF and the same 10 seek cases.
One tick/16 KiB is deliberately yielded, so this is not a throughput benchmark.

The external media lease is checked before/after filesystem operations and
before publishing success, including cache/EOF/seek paths. Canceling mount or
read retains the outer block client; discard the interrupted volume/file, drain
`recover_pending`, and mount again. The host tests exercise this with a real
shared exFAT reader over synthetic sector data and pending transport reads.
Device runtime, physical removal and product executor/commit gates are recorded
separately; they are not established by the synthetic tests or a target build.

## Product load preflight (host-tested)

The opt-in `media-load` feature runs the product's Library load preflight over
the same mounted exFAT volume: audio-file stat, Rekordbox ANLZ DAT/EXT import
and the candidate's source cues. The preflight itself lives in the shared
`ruzina-media-load` crate, which the native firmware's Library load lane also
uses, so both platforms share one implementation. `FsLoadFiles` checks the
live lease and catalog publication around every filesystem operation, cache
hits included. `media_load::preflight` returns analysis only if both are still
current after the last read; the caller rechecks them at its own commit.

The catalog, the deck's load gate and the commit stay with the caller. Analysis
publication allocates, and this staticlib has no global allocator yet, so no
IDF profile selects `media-load` and it has no target build. Host tests use a
real exFAT image read through `ReadQueue`/`ReadClient`: removal during an
analysis read, cancellation followed by drain and a reload under a new lease,
and a warm volume cache after removal.

```sh
cargo +1.95.0 test --locked --manifest-path firmware-idf/legacy-jc1060/rust-core/Cargo.toml --features idf-exfat-fixture,media-load
```
