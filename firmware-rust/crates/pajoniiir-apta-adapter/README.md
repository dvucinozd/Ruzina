# Experimental APTA result adapter

This first selective adoption imports only result/cache conversion and waveform
views from `codex/native-apta-consumer` at
`fd7ebd592b469b27c9f45337dced2f9b2912966f`. It is a `no_std`, unsafe-free,
allocator-free core adapter over pinned Rust `libapta`
`25d09de364e9507532c0508c838a8fecf0772d13`.

The default crate is a workspace member for explicit tests and portability checks, outside
`default-members`. The desktop simulator and default firmware do not depend on it. Explicit firmware
`apta-usb0-bringup` compiles it only with `integration-check`; no resource or
task startup is provided by that feature. See the
[USB0 service gate](../../../docs/DEVELOPMENT.md).
`check_apta_core_boundary.py` checks exclusion in desktop, default firmware and full audio
owner dependency graphs, plus positive presence for the explicit opt-in profile. Production provider selection and Rekordbox ANLZ remain
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

The opt-in `workers` feature adds the portable broker client, WAV scanner,
cooperative worker, persistent owner and two-bank service. `analysis-client`
gates the extra filesystem endpoint variant, so default product builds retain
their existing filesystem routing. These features are not selected by firmware.
The service keeps the latest Analyze when both banks are occupied; Recycle
automatically resumes it. Cancel and newer Analyze commands supersede that wait.
The lane deliberately serializes jobs: it is not two independent analysis lanes.

`owned-handoff` adds `PendingDelivery`, one persistent caller-owned slot per deck.
It retains an unaccepted bank and optional allocated publication across temporary
transport refusal. Every retry checks current authority, original media lease,
track, catalog revision and load generation. Published/rejected outcomes return
the original bank for bounded recycling; enqueue refusal must retain that bank
outside the command future. `discard` explicitly returns a cancelled bank.
Keep the slot alive across owner turns; dropping it is not resource recovery.

The owned copy uses current `AnalysisBuilder` limits: phases outside 0..3 fail
explicitly, and overview is reduced to at most 400 whole-source peak buckets.
No integer detail-Hz rate is invented from a fractional column rate. This bridge
allocates fallibly outside audio/short locks, caches one copy across deferral,
and does not preserve a full-resolution detail waveform or cache cues.
Allocation/representation failure retains the bank for explicit retry/discard.

The existing firmware ProductRuntime now has a provider-neutral background
publication entry under its existing `library-deck-load` feature. It reserves
the actual loaded identity even when ANLZ is absent, refuses existing analysis
(including ANLZ), rejects stale media/load, and defers queued/pending/playing or
downstream-busy transport. Caller-supplied downstream idle must include actual
queued/in-flight execution, not just successful enqueue. No firmware caller,
task spawn, bank allocation or provider selection is wired in this package.

`LocalTempoStorage::plan` now budgets TWO full beat/waveform output banks:
113,772 bytes for the 320000-frame / FPC 256 / queue 17 / 128-beat diagnostic.
`ServiceStorage::plan` includes the additional 4,096-byte broker I/O buffer,
giving 117,868 bytes. These are array payloads only. Owned copies, pending copies,
Vec/owner/channel/future metadata, allocator overhead, existing audio/FX/display
and stack remain additional. Neither total is a physical device memory budget.

Reproduce the optional lifecycle package from `firmware-rust`:

```sh
cargo clippy --locked -p pajoniiir-apta-adapter --all-features --all-targets -- -D warnings
cargo test --locked -p pajoniiir-apta-adapter --all-features --release
cargo check --locked -p pajoniiir-apta-adapter --lib --features owned-handoff --target riscv32imafc-unknown-none-elf
```

The `library-deck-load` adapter feature is solely the host test's cfg name for
including the actual Library-enabled product source. Native FAT32/exFAT tests
retain real scanner/worker/consumer coverage; the older copied product-owner
composition is replaced by tests of today's product/transport implementation.
USB0/USB1 wiring, production scheduling/capacities, compressed analysis input,
physical recovery/placement/timing and musical accuracy remain open. No native
key/confidence feature or `.apta` writer is activated by this crate.
