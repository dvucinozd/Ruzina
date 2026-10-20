# Local gate evidence

Captured 2026-10-20 in WSL Ubuntu with Rust 1.95.0 and the committed Cargo locks. The modified source files are identified by `source-manifest.json`; hashes normalize CRLF to LF as Git does for these text sources. The manifest records the base commit; its containing commit provides the resulting Git identity.

Host commands from `firmware-rust`:

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo check --locked -p desktop-simulator
```

Host native library paths used a cached Ubuntu sysroot with fontconfig/X11 dependencies. Host and P4 builds used separate cached target directories. The host format log is intentionally empty on success. Embedded changed sources were separately formatted with Rust 2024 `rustfmt --config skip_children=true`.

The P4 matrix runs every Cargo command from `.github/workflows/rust-esp32p4.yml` in order, from `firmware-rust/apps/m1-firmware`; `p4-01.log` through `p4-29.log` correspond to the commands in `p4-matrix-summary.log`. The workflow's partition-validator Python block ran before the matrix. Local P4 diagnostic builds used `M1_IMAGE_ID=audio-history-routing-local-review`, which labels this dirty-tree compile check and is not a release image identity. Feature rejection checks use `python3 ../../tools/check_board_selection.py`.

`verification.txt` records host test totals, matrix results, unchanged complete external lock records, and SHA-256 comparison of the primary checkout's 18 pre-existing KiCad edits against the saved pre-integration snapshot. No hardware files are part of this change.

These logs establish local software checks and P4 linking. They do not establish a booted image, memory/peripheral qualification, real-time performance, audible output or a successful GitHub Actions run. No binary is published or flashed by this gate.
