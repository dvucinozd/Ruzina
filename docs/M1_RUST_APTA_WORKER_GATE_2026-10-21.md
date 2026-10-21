# Opt-in APTA worker lifecycle and owned publication — 2026-10-21

Base main: `8039edc3577df7d6c31727710a15b2de1111d264` (selective core PR #17).
Branch: `ogranak/apta-owner-lifecycle`.
Native source branch: `fd7ebd592b469b27c9f45337dced2f9b2912966f`.
Pinned upstream Rust core remains `25d09de364e9507532c0508c838a8fecf0772d13`.

This second package selectively adopts the portable owned-buffer broker client,
WAV source/streaming acquisition, cooperative worker, persistent owner and
two-bank service. It fixes the reviewed bank-starvation/lost-delivery cases,
adds an owned publication bridge, and tests the actual current firmware product
and transport transitions. It does not merge the original branch or activate
a firmware analysis provider.

## Features and production boundary

The default adapter remains the core/cache/window package. `workers` explicitly
enables the imported background components and dependencies. `owned-handoff`
adds fallible copies into the current `AnalysisPublication` model. Adapter feature
`library-deck-load` is the host test's cfg name for including the actual current
Library-enabled firmware source. It does not enable a board/task/provider.

`pajoniiir-media-fs/analysis-client` gates both the portable client module and
the extra `FileSystemClientId::Analysis` variant. This avoids the incomplete
production completion-routing match found during the full-branch audit. The
standalone firmware does not select that feature; neither firmware nor simulator
depends on the adapter/libapta. The existing dependency exclusion script still
checks desktop, default firmware and full audio-owner graphs. Its success is
separate from portable RV32 checks that explicitly select workers/owned handoff.

No USB0 endpoint claim, routing branch/channel, allocator setup, analysis task,
Library-to-service caller or provider switch is wired into main. USB1 remains
the existing owner, not the older lifetime-parametrized APTA variant. Production
audio MAIN/headphone policy, existing cues, ANLZ import and codec execution remain.
The host lockfile gains the exact embassy-futures dependency and local crate
edges; the firmware manifest/lockfile remain unchanged.

## Bank-starvation fix

Previously, `Analyze` received while both output banks were retained produced
`NoBank` and discarded the selection. Returning a bank afterwards did not retry.

The service now cancels previous request authority, retains the latest received
selection and sleeps on the command lane while no bank is free. `Recycle` wakes
that exact selection automatically; a newer `Analyze` or `Cancel` supersedes it.
It neither spins nor reopens the source while starved. Requests generations
still belong exclusively to the persistent receiving owner and never wrap.
The lane deliberately serializes jobs with latest-received policy; it does not
promise independent concurrent analysis for both decks or a FIFO backlog.

The retained-bank regression loads both banks, requests another analysis,
observes NoBank, preserves existing borrowed views, then returns a bank without
repeating Analyze. The new result must publish. Existing queue-refusal,
cancellation/recovery, quarantine, stale-media and refused-output cases remain.

## Temporary delivery fix and current product integration

The old firmware caller recycled valid output whenever transport temporarily
prevented acceptance. This package does not import that caller. `PendingDelivery`
instead holds one unaccepted output and optional built publication per caller
slot. The platform must retain at most one slot per deck and explicitly keep it
across turns. A deferred product callback returns the owned publication to the
same slot, without further decoding or rebuilding it. Each retry rechecks
authority, original lease, track, catalog revision and load generation.

Published and permanently rejected outcomes return the original bank for
bounded recycling. A full recycle queue must return ownership to the caller;
the caller retains it and retries. `discard` explicitly returns cancelled output.
Dropping a pending slot is not cleanup. Bank/Requests lifetime and backend
resource recovery stay explicit; there is no async Drop.

The bridge passes the bank's actual target DeckId to the product callback.
Its Selection revision is bound to the already committed product load generation;
catalog revision is captured from the original catalog snapshot. An old A->B->A
job therefore cannot authorize a different load just because track/lease match.

The existing Library load now reserves `AnalysisIdentity` even when no ANLZ
publication is present. `analysis_view` still returns None until data actually
exists and checks current media. The new provider-neutral
`publish_background_analysis` hook under the existing Library feature:

- requires the exact currently loaded identity and a fresh available media lease;
- refuses existing analysis, including Rekordbox ANLZ, rather than replacing it;
- rejects unsupported provider or stale load/media permanently;
- defers product-queued transport, a pending start/stop, playing state or a busy
  downstream transport boundary;
- moves the publication into the existing owned pin without reloading the track,
  advancing source epoch, changing cues or resolving playback requests.

The hook has no firmware caller yet. Its caller-supplied `transport_idle` must
include actual command queues/in-flight operations; successful enqueue alone is
not execution. Host tests source-include today's ProductRuntime and
TransportOwnerState. They keep a valid first-deck result through a second-deck
Seek, execute the exact downstream command, then retry successfully. Separate
tests check queued start, enqueue with unresolved PlaybackRequest, playing,
and actual stop completion. Stale load/media/authority/catalog and ANLZ priority
are tested without mutating the existing publication.

Test-only media observation substitutes for `usb0_library::current_media_lease`;
it does not emulate the embedded USB broker or execute a physical callback.
Native FAT32/exFAT tests still scan/hash/decode real fixture files and compare
all beats/waveform columns with the whole-object worker. Their obsolete old
ProductRuntime loop is removed and replaced by the current-product tests.

## Owned copy representation and memory

The optional bridge allocates with the current fallible AnalysisBuilder outside
audio and short locks. Beat phases outside 0..3 remain explicit errors; no 4/4
folding is invented. Full-source waveform becomes at most 400 peak-max buckets
in current preview storage. No detail layer or fractional integer-Hz rate is
fabricated. This is an intentional preview representation change; core window
conversion still provides caller-owned full-resolution data separately.
Allocation/capacity/representation errors retain the original bank for explicit
retry or discard. An existing publication is never partially changed.

`LocalTempoStorage::plan` now includes TWO complete beat/waveform output banks.
For 320000 frames, FPC 256, queue 17 and 128 beats per bank:

| Array payload | Bytes |
|---|---:|
| Onset bins + flux | 81,920 |
| Normalized queue | 136 |
| Native overview | 12,500 |
| Two rendered waveform banks | 10,000 |
| Two beat banks | 3,072 |
| Source bytes + PCM scratch | 6,144 |
| LocalTempoStorage total | **113,772** |
| Separate broker I/O buffer | 4,096 |
| ServiceStorage total | **117,868** |

The old 108,772-byte ceiling is rejected. A new regression allocates the actual
working arrays, two complete output-bank shapes and broker scratch, compares
their measured slice payload with ServiceStorage, and rejects a one-byte-short
ceiling. Checked arithmetic applies on host and RV32. These totals exclude
Vec/owner/future/channel metadata, allocator overhead, owned product/pending
copies, existing audio/FX/history/display/catalog and all stack/interrupt costs.
For this maximum-128-beat/400-preview profile each owned copy can add 3,136
payload bytes; provision both accepted and pending slots separately. No total
device/PSRAM/timing budget or physical high-water result is inferred.

## Reproduce and evidence

From `firmware-rust` with pinned Rust 1.95.0:

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo clippy --locked -p pajoniiir-apta-adapter --all-features --all-targets -- -D warnings
cargo test --locked -p pajoniiir-apta-adapter --all-features --release
cargo test --locked -p pajoniiir-media-fs --features analysis-client
cargo clippy --locked -p pajoniiir-media-fs --features analysis-client --all-targets -- -D warnings
cargo check --locked -p pajoniiir-apta-adapter --lib --features owned-handoff --target riscv32imafc-unknown-none-elf
python3 tools/check_apta_core_boundary.py
```

The host workflow adds the optional lifecycle/release/portable checks. Existing
runtime-owner, UI golden and P4/board-selection matrices remain. Default host
workspace tests do not select optional worker suites; report their counts
separately and do not add overlapping focused/release runs as unique coverage.

[Retained source and validation evidence](evidence/m1-apta-workers-2026-10-21/README.md)
pins the exact patch, source hashes, provenance and gate logs.

| Final local software gate | Result |
|---|---|
| Default workspace fmt/Clippy/tests | PASS; 667 tests including doctests |
| Opt-in all-feature adapter Clippy/release | PASS; 56 tests + 2 doctests |
| Focused policy/product/service lifecycle | PASS; 12 tests, overlapping release |
| Explicit optional broker unit suite and Clippy | PASS; 15 tests, including cleanup-ticket exhaustion |
| Existing actual runtime owners/composed failures | PASS; 9 + 11 cases |
| Desktop compile/UI | PASS; 38 unchanged goldens |
| Core and owned-handoff RV32 library checks | PASS |
| Existing P4 matrix and board selection | PASS; 33 profiles + 24 expected invalid selections |
| Production dependency exclusion | PASS; three graphs |
| Final CodeRabbit implementation review | 0 issues; all 31 staged code/manifest/CI/README files |

The 15-test broker-enabled suite overlaps the default media-fs tests. It is not
15 new unique tests. Feature-specific P4 unused-import/dead-code warnings remain;
the target matrix is not warnings-denied. A compile/link gate is not actual
analysis execution or resource acceptance on P4.

The reviewed staged patch is pinned by raw SHA-256 and per-file normalized hashes.
Final root navigation, this report and evidence are added after the implementation
review without changing reviewed code. No finding remains unresolved in that
review; this does not close the explicit physical/integration gates below.

## Remaining gates

The portable source profile is WAV-only. Main playback WAV/MP3/FLAC support does
not automatically enable compressed analysis. Future wiring must define true
downstream idle, bounded notification/recycle queues, resource allocation,
multi-minute object/duration/work caps and USB0 playback contention policy.
The one-lane supersession policy invalidates older unaccepted deliveries; it
must be reflected in product scheduling instead of promising two independent jobs.
Owned publication currently fills an empty slot only; automatic native upgrades
or ANLZ replacement are not part of this package.

No source DSP algorithm changes, real music corpus/tuning, key/confidence feature,
new cache writer, physical flash/boot/USB cancellation, PSRAM placement, stack
high-water, realtime deadline/GUI/audio coexistence soak or listening acceptance
is claimed. The default firmware does not enable APTA. Original APTA history and
all pre-existing KiCad edits remain preserved.
