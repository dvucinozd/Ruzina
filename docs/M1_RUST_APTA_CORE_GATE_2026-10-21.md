# Selective APTA result adapter — 2026-10-21

Base main: `e069888ccce3ef23412741607b54058c73d9e82f`.
Branch: `ogranak/apta-core-adapter`.
Selected source: `codex/native-apta-consumer` at
`fd7ebd592b469b27c9f45337dced2f9b2912966f`.

This first adoption brings portable result/cache conversion and waveform views
into the current workspace. It preserves the current Library, Rekordbox ANLZ,
owned analysis publications, transport completions and WAV/MP3/FLAC audio path.
The original APTA branch itself is not merged.

## Adopted scope

`pajoniiir-apta-adapter` exposes `convert`, `convert_cache`,
`convert_cache_with_scratch`, `waveform` and `window` with caller-owned storage.
It uses the pinned no-default-features Rust `libapta` core at
`25d09de364e9507532c0508c838a8fecf0772d13`; the only added upstream core
dependency is that Git-pinned package over the existing `libm` version.
The host workspace lock adds that package and the adapter. The standalone
firmware manifest and lockfile are unchanged.

Converted beat storage survives the original session/cache lifetime. Generation,
fingerprint, source geometry and lineage are retained separately. Conversion
preflights capacities/coordinates before writing and uses checked millisecond
and tempo rounding. Continuous final segment/explicit grids require a single
known final meter and compatible downbeat authority. Unsupported global data
does not silently fall back to local data; hybrid/sparse/mixed-meter grids remain
unsupported. Missing/non-final grid or meter can produce tempo-only data.

The native conversion entry point accepts trusted session/owned result views;
external cache bytes use full upstream parsing/validation with caller limits.
Comparing a supplied expected fingerprint is not computing the source audio
hash. This package has no filesystem cache owner or writer.

Waveform conversion and aligned overview/detail windows use separate caller
banks. Holes, insufficient capacity and invalid coordinates fail before output
mutation. EOF variants accept a partial last column with its explicit source
length. One source column still occupies one output column; automatic layer
selection, arbitrary cropping, band colors and duration-proportional pixel
resampling remain subsequent work.

`Analysis::upgrade` checks a caller-supplied stopped flag and matching identity,
provider and newer generation. It does not replace the product owner's checks
for pending transport, fresh media and immutable publications. The current
`AnalysisBuilder` supports phases 0..3 while the native converter accepts a
numerator 1..32. A future owned bridge must reject unrepresentable meter or
explicitly extend that model and its consumers.

## Product isolation

The crate is a workspace member for explicit validation and is excluded from
`default-members`. Neither the simulator nor firmware references it. There is
no new board feature, provider switch, task, allocator, bank allocation, USB
endpoint, output route or startup action. MAIN retains ready external DAC ->
compatible controller -> mute; headphones remain controller-only.

The first package contains only `src/lib.rs`, `src/window.rs` and three test
files. It excludes the original source scanner, storage planner, broker client,
worker, persistent owner/service, ProductRuntime replacement and USB0/USB1 wiring.
The two reviewed lost-analysis problems therefore remain in the unmerged branch:
Analyze-before-Recycle with two occupied banks, and recycling a valid delivery
after temporary transport refusal without retry. Their fixes belong to the
later owner/service package.

`tools/check_apta_core_boundary.py` resolves normal/build Cargo dependency graphs
for the desktop simulator, default RV32 firmware and full RV32 audio owner
profile. Any `libapta` or adapter package in those product graphs fails the gate.
A dependency-resolution failure also fails instead of reporting an empty graph
as success. CI adds this exclusion check and an explicit no-default-features
portable RV32 library check; all prior host and P4 jobs remain present.

A negative diagnostic applies the reviewed patch to a separate base-main clone,
adds a direct adapter dependency only to that clone's firmware manifest,
resolves its lock, and runs the same exclusion script. The script rejects the
injected `libapta`/adapter path at the default firmware graph. Production source
and firmware lock are not modified by that diagnostic.

## Validation

The adapter contributes seven tests and one compile-fail lifetime doctest.
Generated PCM/native sessions exercise actual neutral Deck Sync/Beat Jump,
local/global cache conversion, immutability after source destruction,
transactional rollback, Q32/tempo overflow/rounding, source/provider/generation
upgrade rejection, PCM backpressure/cancellation, signed waveform endpoints,
sparse windows and explicit EOF columns through the RGB565 rasterizer.
These are synthetic software contracts, not musical accuracy acceptance.

Final gate results and source hashes are retained in the
[evidence manifest](evidence/m1-apta-core-2026-10-21/README.md).

| Local gate | Result |
|---|---|
| Workspace fmt and all-targets Clippy `-D warnings` | PASS |
| Full current workspace | 667 tests including the new lifetime doctest PASS |
| Optimized adapter | 7 tests + 1 doctest PASS, overlapping workspace tests |
| Explicit runtime-owner Clippy and actual owner/failure cases | PASS; 9 + 11 cases |
| Desktop compile and unchanged UI goldens | PASS; 38 snapshots |
| Portable RV32 adapter without default features | PASS |
| Existing P4 check/link matrix and board guards | PASS; 33 profiles + 24 expected rejections |
| Product dependency exclusion | PASS; 3 profiles |
| Injected product dependency in disposable clone | Rejected as required |
| CodeRabbit selective implementation review | 0 issues across all 11 implementation/manifest/CI/README files |

P4 links retain their existing feature-dependent unused-import/dead-code
warnings; that target matrix is not a warnings-denied gate. Compile/link success
does not establish physical board, memory placement or realtime behavior.

The exact staged selective patch was reviewed against base main; its SHA-256
and per-file hashes pin the implementation. Root navigation and this final gate
report/evidence are added after that review and do not change the reviewed code.

Reproduce from `firmware-rust` with the pinned Rust toolchain:

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo test --locked -p pajoniiir-apta-adapter --release
rustup target add --toolchain 1.95.0 riscv32imafc-unknown-none-elf
cargo check --locked -p pajoniiir-apta-adapter --lib --no-default-features --target riscv32imafc-unknown-none-elf
python3 tools/check_apta_core_boundary.py
```

The existing rust-host workflow also runs desktop compilation, actual composed
runtime-owner tests and UI golden checks. The unchanged rust-esp32p4 workflow
defines the full feature/check/link matrix and invalid board-selection guards.

## Open gates

No firmware provider is activated or qualified. The next integration requires
the two lifecycle fixes, an aggregate two-output-bank memory plan, per-deck job
scheduling/retry policy and adaptation to the current owned publication and
transport-completion contracts. The old one-waveform storage plan is not the
whole two-bank service budget.

APTA file acquisition on the original branch is WAV-only. The current playback
codec support does not enable compressed analysis; a future decoder-to-analysis
input must use source coordinates and pre-pitch/pre-FX PCM with an explicit
precision/gapless policy. Native key/confidence publication and cache writing
are not supplied by this package. Pinned upstream musical accuracy and portable
key rounding gates are unchanged.

No corpus run/tuning, sanitizer run, flash/boot, real board/BSP, stack high-water,
PSRAM placement, USB recovery, audio/GUI coexistence timing or listening
acceptance is claimed. Existing KiCad edits and the unmerged source branch are
preserved independently of this adapter.
