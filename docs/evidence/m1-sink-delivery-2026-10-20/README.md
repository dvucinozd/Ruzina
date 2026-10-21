# Consumer-clock delivery local evidence

Captured 2026-10-20 with Rust 1.95.0 in WSL Ubuntu. `source-manifest.json` records changed files relative to base `8675aeb522e66446be566e908fb5b34989729d23`, with SHA-256 after CRLF-to-LF normalization. Its containing commit identifies the resulting source revision. Earlier history/routing evidence is retained separately and is not overwritten.

Host commands from `firmware-rust`:

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo check --locked -p desktop-simulator
cargo test --locked -p pajoniiir-audio-runtime
cargo run --locked -p pajoniiir-audio-runtime --example clocked_output
```

The empty format log means success. Native host dependencies use the cached Ubuntu fontconfig/X11 sysroot. P4 and host use separate cached target directories. All 29 Cargo commands and the partition check from `.github/workflows/rust-esp32p4.yml` were executed; numbered logs correspond to `p4-matrix-summary.log`. Feature guards use `python3 ../../tools/check_board_selection.py` from the embedded app. Local diagnostic builds retain the prior `M1_IMAGE_ID=audio-history-routing-local-review` label; it is not release-image provenance.

`verification.txt` records unchanged dependency locks, primary KiCad hashes, totals and the deterministic simulation result. The example runs ten virtual seconds, not ten seconds of physical or real-time playback. Logs establish no flashed image, DMA scheduling, UAC feedback, clock correction, listening or hardware qualification. The embedded reference task remains software-timed.
