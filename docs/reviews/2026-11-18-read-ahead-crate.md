# The SD read-ahead moves into `ruzina-media-io`

Date: 2026-11-18. Branch: `ogranak/media-io-read-ahead`, from `main` at `bf01dbb`.

## Why

The legacy JC1060 path had its own multi-sector read-ahead (`read_ahead.rs`, added in
the [SD multi-sector review](2026-11-04-sd-multisector.md)). It serves the exFAT
volume's one-sector reads from 16-sector lines, so a sequential read costs one card
request per line instead of one per sector.

The module has no `unsafe` and depends only on `ruzina-media-block` and the
`OwnedBlockDevice` contract. An outside review marked it as the one clean backport
from the legacy path. In the shared crate, the simulator, the native firmware and a
later USB stick path can use the same layer.

## Change

- `ruzina-media-io` gains `read_ahead` (`Line`, `ReadAhead`, `Error`, `LINE_SECTORS`,
  `LINE_BYTES`, `LINES`), unchanged apart from its imports.
- It needs a heap, so it sits behind a new `read-ahead` feature. Without it the crate
  stays `no_std` with no `alloc`, as the native firmware and the APTA adapter use it.
- The legacy core enables `ruzina-media-io/read-ahead` with `media-load`, and
  `idf_media_bridge.rs` uses `ruzina_media_io::read_ahead`.
- Tests:
  - the four unit tests moved with the module, with their own in-memory card;
  - the exFAT check, which needs the legacy fixture, stays in
    `exfat_fixture/tests.rs` as
    `the_fixture_verifies_through_the_read_ahead_with_far_fewer_requests`.
- Host CI runs clippy and the tests of `ruzina-media-io` with `read-ahead`, since the
  workspace run does not enable it.

## Evidence

- **Host:**
  - `ruzina-media-io` with `read-ahead`: 8 tests, clippy clean;
  - without it: 4 tests;
  - the APTA adapter still checks for `riscv32imafc` without default features;
  - workspace clippy is clean;
  - the legacy tests and clippy for three feature sets pass, and so does target
    clippy for every CI feature set.
- **Device:** s134 (app `af38de11…`) after a cold power cycle:
  - the SD card came up at the first try at 40 MHz, 4 bits, `max_sectors=16`;
  - the owner loaded and played both decks and turned Library pages: "Radi";
  - every one of the 7 reports had `packet_errors=0` and underruns 0/0.

## Test plan

1. Flash s134, play both decks and turn Library pages: playback and loading behave
   as on s133.
2. The log keeps `packet_errors=0` and no underruns.
