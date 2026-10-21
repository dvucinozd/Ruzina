# Rekordbox analysis evidence

Captured 2026-10-21 on `ogranak/rekordbox-analysis`, base `6ea7b977aa2ea0390bcbd578e99118affd78c234` (open audio PR #5). Rust 1.95.0, WSL Ubuntu and committed locks were used. `source-manifest.json` identifies changed source relative to that base: text hashes normalize CRLF to LF; DAT/EXT hashes retain raw bytes. Its containing commit supplies the final Git identity.

Host commands from `firmware-rust`:

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo check --locked -p desktop-simulator
cargo test --locked -p pajoniiir-rekordbox -p pajoniiir-track-analysis -p pajoniiir-waveform -p pajoniiir-deck
cargo run --locked -p pajoniiir-rekordbox --example analysis_inspect -- FILE.DAT FILE.EXT
```

The 29 numbered P4 logs correspond to `p4-matrix-summary.log` and the current workflow commands. Its partition-validator block ran before the matrix; board guards use `python3 ../../tools/check_board_selection.py` in the embedded app. Host/P4 target caches are separate. Local diagnostic links use the existing non-release label `M1_IMAGE_ID=audio-history-routing-local-review`. The empty format log indicates success; embedded changed owners were separately formatted with Rust 2024 rustfmt.

`public-anlz-results.json` pins all six public corpus URLs, repository commit, byte lengths and raw hashes. It records independent Python decoding, actual Rust output and normalized M2.2 C-parser output for all three pairs. Baseline C source is pinned to commit `2c2ec32c253d368765123d7bbf8d37389b790b55` in `dvucinozd/Pajoniiir`. `m22-anlz-compare.c` is the new comparison harness; no released C implementation is copied here. Raw public fixtures and C reference sources remain outside this repo. The repository license is recorded for provenance; no blanket license claim is made about third-party data.

The synthetic fixtures/generator live in `firmware-rust/crates/pajoniiir-rekordbox/tests/fixtures/`. Public-corpus comparisons cover beat/monochrome waveform values and empty source-cue lists. Nonzero Hot Cues, loop semantics, local overrides and tombstones use synthetic/deck fixtures, not a claim of exhaustive real-export cue coverage. Unsupported EXT tags are listed in the result manifest and remain unsupported.

`verification.txt` records external lock-package preservation, all 18 primary KiCad hashes and local totals. Host checks, direct parser comparisons and P4 linking do not establish physical media behavior, PSRAM/resource qualification, integrated UI, physical audio or successful hosted CI. APTA is untouched.
