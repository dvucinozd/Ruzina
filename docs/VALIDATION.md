# Validation scope

Host and compile evidence is separate from physical qualification. Every result
belongs to its exact source tree, feature set, toolchain and image. A skipped,
unstarted or waived check is not PASS.

The functional corpus now targets released M2.5. The [parity audit](M2_5_PARITY_PLAN.md)
records the pinned donor/Rust sources, existing 315-test focused run and open
RP-01..20 work. Existing regression success is not a differential parity result.
Each batch must add independent donor-behavior fixtures and failure cases before
its gap can be closed. New shared-P4 beta/master repairs retain separate source
provenance and do not inherit stable M2.5 or other-board hardware acceptance.

## Repeatable checks

From `firmware-rust`:

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo test --locked -p pajoniiir-apta-adapter --all-features --release
cargo test --locked -p pajoniiir-service --features runtime-owners --test runtime_owners --test composed_failures
python3 -m unittest discover -s tools/tests -v
python3 tools/check_docs.py
python3 tools/check_apta_core_boundary.py
cargo run --locked -p desktop-simulator -- --smoke target/ui-smoke
python3 tools/check_ui_goldens.py target/ui-smoke
python3 tools/run_apta_stress.py --iterations 25 --output /absolute/new/stress
```

The stress campaign pins sources/executable and requires actual lifecycle
markers; 25 iterations cover 1,600 cancel/reload cycles and 800 detach lifetimes
in the host harness. These do not demonstrate physical USB timing or buffer/DMA
behavior. UI goldens cover 54 representative simulator images across the two
documented geometries.

From `firmware-rust/apps/m1-firmware`:

```sh
python3 ../../tools/check_board_selection.py
cargo +1.95.0 build --locked --release --features board-jc1060,board-diagnostics
cargo +1.95.0 build --locked --release --features board-jc8012,guition-flash-probe,guition-psram-test
cargo +1.95.0 check --locked --features integration-check,render-graph-bringup,sdmmc-profile-storage-bringup,runtime-health
```

[P4 CI](../.github/workflows/rust-esp32p4.yml) lists the complete current
compile/link matrix and independent Guition package variants. Its integration
configurations only compile owner modules; they do not establish a product
runtime or hardware task wiring. [Host CI](../.github/workflows/rust-host.yml)
retains UI/stress/runtime artifacts. Inspect exact run annotations if hosted jobs
do not start; account billing/spending errors are infrastructure failures.

## Evidence retention

Keep generated captures, packages and campaign outputs outside source control
or in CI artifacts. Package manifests bind source/compiler/environment, image ID,
ELF/archive hashes and raw logs. Unit records preserve failure/partial data and
stay NOT_QUALIFIED until the separate physical procedure is reviewed.

Validation history remains available in Git. Current raw validation records and
the pre-cleanup archive are retained outside the checkout, including complete
source/ref snapshots and exact SHA-256 manifests. Build products are not binary
releases and no board was flashed during software preparation or cleanup.

Still NOT RUN: physical first boot/recovery, factory backup/readback, flash/PSRAM
probes, display/touch, USB/SD power and reconnect behavior, audio/UAC/DAC clocks,
latency, listening and soak on either Guition model.
