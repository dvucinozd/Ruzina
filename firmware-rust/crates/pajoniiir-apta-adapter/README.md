# Experimental APTA result adapter

This first selective adoption imports only result/cache conversion and waveform
views from `codex/native-apta-consumer` at
`fd7ebd592b469b27c9f45337dced2f9b2912966f`. It is a `no_std`, unsafe-free,
allocator-free adapter over pinned Rust `libapta`
`25d09de364e9507532c0508c838a8fecf0772d13`.

The crate is a workspace member for explicit tests and portability checks, outside
`default-members`. Neither the desktop simulator nor firmware depends on it.
`check_apta_core_boundary.py` checks the desktop, default firmware and full audio
owner dependency graphs. Production provider selection and Rekordbox ANLZ remain
under their existing owners. Headphones remain controller-only.

## Supported contracts

- `convert`: trusted native result views to borrowed neutral `TrackAnalysis`.
  Q32 frame positions round once to milliseconds; millibpm rounds to BPM x100.
  Capacity/coordinate errors preserve destination beats. Retained views borrow
  only their converted bank, independently of the original result lifetime.
- Final continuous segment/explicit grids require a single final full-source
  quarter-note meter. A global grid takes precedence; unsupported global data
  does not silently become local data. Hybrid, sparse grids and meter changes
  are rejected. Missing/non-final grid or meter can produce tempo-only output.
- `convert_cache` / `convert_cache_with_scratch`: parse and validate `.apta`
  bytes with caller limits, verify the supplied expected identity, and convert
  supported local/global grids using caller scratch. Identity comparison does
  not hash the source audio. Cache parsing is not filesystem cache ownership.
- `waveform` and `window`: independent caller-owned overview/detail display
  columns. Holes and unaligned cropping fail before output mutation. Explicit
  EOF variants accept a shorter final column; every source column still occupies
  one output column, so this is not duration-proportional resampling.
- `Analysis::upgrade` requires a caller-supplied stopped state, matching source,
  geometry, lineage and provider, and a newer generation. The caller must also
  exclude pending transport and preserve the bank. This helper does not replace
  the actual product owner's stronger media/transport/publication checks.

Native conversion accepts a trusted session/owned result, not an arbitrary
external graph. Untrusted cache bytes must go through the validated cache API.
Storage capacity bounds work, not elapsed time; run conversion outside audio and
other deadline-sensitive callbacks. Empty BPM remains zero without inventing a
tempo or implicit 4/4 downbeat.

This package does not adapt to `AnalysisPublication` automatically. The current
owned builder accepts phases 0..3; APTA can represent a meter numerator 1..32.
A later product bridge must explicitly reject unsupported meter or extend that
model and its consumers. Do not fold phase into four beats to force acceptance.

## Reproduce

From `firmware-rust`, using the committed Rust toolchain and lockfile:

```sh
cargo test --locked -p pajoniiir-apta-adapter
cargo clippy --locked -p pajoniiir-apta-adapter --all-targets -- -D warnings
rustup target add --toolchain 1.95.0 riscv32imafc-unknown-none-elf
cargo check --locked -p pajoniiir-apta-adapter --lib --no-default-features --target riscv32imafc-unknown-none-elf
python3 tools/check_apta_core_boundary.py
```

Tests use generated in-memory PCM, native sessions, cache bytes, the existing
Deck Sync/Beat Jump APIs and RGB565 rasterizer. No private corpus or audio assets
are included. These verify representation, lifetime and consumer contracts;
they do not qualify musical accuracy or physical P4 execution.

## Deferred integration

The original branch's broker, source scanning, worker, service, two-bank owner,
ProductRuntime modifications and USB0/USB1 integration are excluded. Its lost
analysis on `NoBank` and temporary transport refusal still require fixes before
that later adoption. Aggregate bank memory planning, WAV/MP3/FLAC analysis input,
scheduling/recovery and physical resource/timing/listening gates remain open.
No native key/confidence feature or `.apta` writer is activated by this crate.
