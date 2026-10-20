# Completed-output progress local evidence

Captured 2026-10-21 in WSL Ubuntu with Rust 1.95.0. `source-manifest.json` hashes changed files relative to `b3da05ea5422f87da50709dedf5fd08df03fd810`, normalizing CRLF to LF. Its containing commit supplies the resulting source identity. Prior history/routing and delivery evidence remains unchanged.

Host commands from `firmware-rust`:

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo check --locked -p desktop-simulator
cargo test --locked -p pajoniiir-audio-runtime
cargo run --locked -p pajoniiir-audio-runtime --example clocked_output
```

The format log is intentionally empty on success. Host native dependencies use the cached Ubuntu fontconfig/X11 sysroot. P4/host target caches are separate. All 29 Cargo commands and the partition-validator block from `.github/workflows/rust-esp32p4.yml` ran; numbered logs map to `p4-matrix-summary.log`. Board guards use `python3 ../../tools/check_board_selection.py` in the embedded app. Local diagnostic builds use the existing non-release label `M1_IMAGE_ID=audio-history-routing-local-review`.

The simulation now runs canonical PCM history through render, queues, delayed completion receipts, playback bookkeeping and `PcmTransportCursor`. Its final line distinguishes confirmed frame 480000 from rendered 480256 and prefetched 482048. MAIN/PFL amplitude isolation and both 48/128-frame consumer cadences are retained. Fourteen completion-specific tests cover stale/out-of-order/replayed tickets, source/owner replacement, partial EOF, authority, pause/starvation, congestion and malformed source faults.

`verification.txt` records unchanged Cargo locks, SHA-256 comparison of all 18 primary-checkout KiCad edits, test/matrix totals and deterministic simulation output. No firmware image was flashed or published. Reference completion remains synchronous/software-timed; these logs establish no physical DMA/UAC behavior, clock timing, analog latency or listening acceptance.
