# Experimental native APTA provider

Direct pinned Cargo dependency on the portable `libapta` core. This crate is
`no_std`, allocator-free and unsafe-free. No C ABI, desktop runtime, IDF or
second DSP engine is involved. ADR-006 / M1R-P1-001 production gates remain in
force: neither the firmware nor the desktop simulator enables this provider.

Run from `firmware-rust`, with an external `CARGO_TARGET_DIR` and two build jobs:

```sh
cargo test --locked -p pajoniiir-apta-adapter
cargo run --locked -p pajoniiir-apta-adapter --example analyze_wav -- input.wav output.ppm
cargo check --locked -p pajoniiir-apta-adapter --lib --target riscv32imafc-unknown-none-elf
```

The host example decodes WAV, runs the existing core outside any audio callback,
converts analysis and renders an 800x128 whole-track waveform through the existing
RGB565 rasterizer. It creates a new PPM only. Its 256 MiB input and 65,536-beat
caps are host example work/output policy, not firmware budgets. Source storage
is now fixed scratch through the two-pass streaming path below.

## Contract

- `convert` takes a trusted session snapshot/owned-result view, not unvalidated
  externally assembled graphs. It validates the consumer's representable subset;
  it is not a replacement for Libapta's complete external graph validation.
- Fractional Q32 sample positions use the source rate and round once to nearest
  millisecond, ties upward. Results exceeding `u32` and coincident rounded beats
  are errors. Millibpm rounds to BPM x100, ties upward, with wide arithmetic.
- Final continuous single/multiple-segment or explicit grids are accepted with one final
  full-source quarter-note meter. Phase is zero-based relative to the actual
  meter downbeat ordinal, modulo its numerator. The selected grid must contain
  that exact downbeat whole-frame/ordinal pair; local meter cannot silently supply
  phase to an unrelated global grid. No implicit four-beat meter.
  Segment beats retain nominal tempo; explicit beats use the selected track tempo
  (zero when absent), without claiming an instantaneous tempo estimate.
  Segment applicability must partition the full source without gaps or overlap,
  and ordinals must advance by exactly one across boundaries. Every segment is
  final with non-provisional phase. Explicit ordinal gaps, sparse grid coverage
  and hybrid authority remain unsupported. A global grid takes precedence; unsupported global data does
  not silently fall back to a different local interpretation.
- Missing/non-final grid or meter, unknown duration, or provisional phase yields
  tempo only. Missing tempo is represented by neutral BPM zero; existing Deck
  fallback policy applies. State/confidence are not promoted into accuracy claims.
- `u64` native generations must fit a nonzero `u32`. Converted `Analysis` retains
  source fingerprint/geometry and lineage separately from `TrackAnalysis`.
  `upgrade` requires a stopped deck, matching provider and present identity/geometry/lineage,
  and a strictly newer generation. The caller supplies actual deck play state.
  New track loads or provider changes establish a new pin. Absent fingerprints cannot authorize an
  upgrade; supplied fingerprints are transport, not proof of hashing.
- Conversion preflights before writing; short capacity or later coordinate errors
  preserve the destination. Keep two caller storage banks for staging and pinned
  analysis. Borrowing prevents mutation while a view is retained. The converted
  view survives destruction of the original session/cache; no pointers into the
  original result are retained. Existing neutral Deck APIs do not themselves
  enforce pinning: the worker/product owner must retain the selected `Analysis`.
- `convert_cache` validates bytes with caller-specified Libapta limits, requires
  an expected present source identity, and supports the local-grid cache profile.
  `convert_cache_with_scratch` also accepts global segment/explicit caches using
  caller-owned `GridSegment`/native `Beat` scratch; scratch may change on failure,
  but converted output stays transactional. Applied revision IDs must match the
  grid; pending revisions are never exposed as a final consumable grid. Hybrid
  remains rejected: populated arrays alone do not identify override ranges.
  Cache generations are assigned by
  the host; `.apta` does not persist native session generations/lineage. Cache and
  live grids are compared exactly by the integration test.
- `waveform` converts a final continuous overview for the existing whole-track
  renderer. Q15 magnitude maps to full-range u16, nearest rounding; -32768
  saturates. The caller chooses RGB565 color. `window::overview` and
  `window::detail` select aligned whole-column viewports from published sparse
  overview or level-1 detail data, including adjacent detail tiles. The caller
  explicitly chooses the layer. Holes, unaligned cropping and partial EOF columns in the strict APIs
  return errors before output changes; no silence or interpolated coverage is
  fabricated. These views feed the existing renderer and survive source drop.
  Arbitrary pixel resampling, automatic layer selection and band colors remain open.

## Storage and scheduling

Caller beat storage is `12 * capacity` bytes on the tested host/RV32 layout;
converted waveform storage is `4 * columns`. Two live beat banks double that
storage. The core's 4096 onset bins and 4096 f32 flux entries use 80 KiB before
queue, overview, session object, retained results and stack. The host example
prints its concrete sizes. No large inline beat arrays exist in the library.

PCM tests use a 256-frame queue and `maximum_steps=1`, preserving retry at the
unaccepted suffix. They cover full-queue backpressure, invalid PCM before
mutation, cancellation, conversion rollback, immutable retention, Deck Sync /
Beat Jump, cache identity and actual raster output. Conversion takes two bounded
passes over output beats; cache validation has its separate configured limits.
A step budget bounds cooperative work, not elapsed real-time latency.

P4 cross-compilation and dependency inspection establish a portable software
boundary only. Production worker scheduling, PSRAM banks, internal SRAM scratch placement,
DMA/cache discipline, whole-call stack and timing, and physical target execution
are still required. No firmware deployment or hardware operation is part of this
experiment. Libapta's portable key rounding boundary and original musical
accuracy gates remain open independently of this adapter.

## Experimental media worker

`worker::Requests` is one persistent analysis lane, with a checked monotonic
nonzero generation for each completed analysis job (not a core publication serial). A request carries the originating catalog/media `MediaLease`,
`MediaTrackId` and present source-object identity. It validates that original
lease against the mounted `MediaSession`; do not replace an old catalog lease
with the current medium's lease. New requests supersede in-flight work, and
cancellation invalidates delivery. Keep this owner alive while any job/completion/
pin exists; do not recreate it or mix independent lane owners' tickets.

`worker::Worker` owns a Created, already-configured `Session` over caller storage.
It checks the current request/media before push, processing and completion.
Each step uses at most 256 frames and one core step. Direct PCM callers own asynchronous reads/decoding, must retry the unaccepted
PCM suffix, and must report source failure. The optional catalog acquisition path
below manages these steps for a caller-buffered WAV object. Failed/stale workers cannot publish. Completion requires a completed
session; capacity failures can be retried with a larger conversion bank.
No core processing, filesystem work or allocation belongs in the audio callback.

`worker::Pin` retains the selected immutable conversion bank and track identity.
Load/upgrade inspects the actual `DeckProductState`: both playing and pending
transport requests block replacement. Same-track upgrades additionally require
matching media/source geometry/provider/lineage and a newer generation. A new
track establishes a new pin. Replacement requests leave the old pin usable;
unmount/disconnect hides it, and reinsertion cannot revive it. Product code must
keep one selected pin per deck, clear/replace it on track loads, and obtain the
neutral view through `Pin::track` for each use. A copied neutral view cannot itself
recheck media or prevent bypassing these experimental owner APIs.

The worker borrows existing typed musical getters and overview columns instead
of constructing an unused full-detail snapshot.

Storage stays explicit: two caller beat banks, existing core queue/onset/overview
storage, optional cache segment/beat scratch and rendered waveform banks. Worker
and request objects contain no large inline beat arrays or allocator. The host
WAV example now goes through this worker and pin path, computes SHA-256 over the
exact source object with the existing product identity implementation, and renders
a retained waveform after dropping the worker. Its mounted media session is a
host fixture, not a USB filesystem or Embassy task integration. Production firmware
and Slint provider selection remain unchanged.

## Extended consumer verification — 2026-10-04

The global transport tests compare every beat exactly for single-segment,
multi-segment and explicit global caches, using timing produced by real local
PCM analysis. These are deliberately constructed transport profiles, not S6
accuracy acceptance. Actual S6 output for the 8 kHz/320000-frame uniform impulse
fixture has a final segment ending at frame 262144 and a meter that does not bind
to it; conversion must fail without local fallback. Dynamic/hybrid authority and
DSP behavior were not changed to make that fixture pass.

Real worker PCM drives Deck Beat Jump/Sync and two retained generations; cancellation,
replacement, stale delivery/catalog leases, unmount/reinsert, source failure,
pending transport and capacity rollback are covered. Test-only allocator
instrumentation observes zero allocations during worker creation, PCM processing
and conversion. Real sparse PCM crosses a detail-tile boundary into the renderer;
hole and alignment rejection preserve the output bank. All library modules remain
`no_std` and forbid unsafe; the counting allocator is confined to the test harness.

Extended checkpoint evidence (before the subsequent foundation integration):
`verify-consumer.py`, stamp `20261004T181028Z`, passes 373 workspace tests,
78 focused release tests, 8 adapter integration tests under ASan plus the lifetime
compile-fail doctest, formatting/Clippy, pinned Rust 1.95 P4 library/object checks.
`extended-stack-final-20261004T181128Z.log` measures individual RV32 frames:
worker completion 1104 B, waveform 112 B, push 1776 B, step 352 B; global cache
conversion 2608 B, overview window 128 B, detail window 48 B. Completion/waveform
previously used 7712/7600 B with a full detail snapshot; typed getters remove that
unused work. These exclude nested callees, caller placement and interrupt/runtime
costs and are not task-stack recommendations. Final-source acceptance after integrating the collaborator's exFAT implementation
and host-check fix: stamp `20261004T181337Z` passes **382 workspace tests**,
**78 focused release tests**, **8 adapter ASan integration tests + 1 compile-fail
doctest**, formatting, Clippy, P4 compilation/dependency inspection and optimized
object generation. No caller memory placement or hardware execution is claimed.
`verify-extended-artifacts.py` reruns the worker WAV/raster cases serially; stamp
`20261004T181426Z` verifies byte-identical PPM output after the stack optimization:
120.00 BPM / 80 beats / 4120 green pixels, and silence BPM zero/no grid / 800-pixel
centerline. Both images are 800x128. The final object report records the same
individual worker frames. All logs/artifacts remain in the external evidence root.

## Dated software verification — 2026-10-04

Evidence: `/home/shome/.local/share/Pajoniiir-M1/apta-evidence/`, final stamp
`20261004T172847Z`; `verify-consumer.py` reproduces serial checks with two jobs.
Rust 1.95.0, committed lockfile and the pinned remote Libapta dependency were used.

- Full workspace: **364 tests**, including the desktop simulator's existing tests.
- Focused adapter/Deck/TrackAnalysis/waveform release run: **74 tests**.
- Adapter ASan/leak run: **4 integration tests + 1 lifetime compile-fail doctest**.
- Workspace formatting and Clippy with warnings denied pass.
- Actual P4 `riscv32imafc-unknown-none-elf` library check and optimized object
  generation pass. The normal dependency graph excludes std/Libapta runtime,
  alloc, IDF and RTOS application dependencies. This is not a firmware link/run.
- WAV example: 12000 BPM x100, 80 beats; actual 800x128 RGB565-to-PPM output has
  4,120 green pixels. A separate silence WAV yields BPM zero/no grid and only the
  renderer's 800-pixel centerline. `wav-example-*` and `silence-example-*` retain
  output; `fixture-20261004T172847Z.png` is the inspected raster artifact.
- RV32 layout: Beat 12 B, rendered column 4 B, onset bin 16 B, Session 3536 B,
  SessionSnapshot 3768 B (`p4-layout-*`). Host Session is 3648 B. These are object
  sizes, not simultaneous live-memory/stack maxima.
- RV32 LLVM individual stack frames (`p4-stack2-*`): conversion 336 B, its beat
  closure 304 B, milliseconds 112 B, cache conversion 2480 B, waveform 0 B,
  upgrade 16 B. These exclude nested callees, caller result placement, interrupts
  and task/runtime costs. Full task-stack acceptance remains open. The first
  stack-tool lookup failed because the version-named toolchain lacked LLVM tools;
  the corrected report uses the installed stable LLVM reader on the same object.

The inherited catalog formatting change is already on the foundation branch;
this PR does not change collaborator firmware/catalog implementation. The Slint
build's missing development metadata was supplied in an external user-local
package extraction for host verification; no host service/system installation.

## Catalog filesystem acquisition — 2026-10-04

`source::SourceRequest` selects the exact audio path and track ID from either an
immutable `CatalogSnapshot` or the foundation's storage-backed `CatalogBankSnapshot`.
It reserves the existing persistent analysis lane before I/O, retaining the
original catalog lease. The selected path is copied; recycling the catalog bank
cannot change it. `open` uses the existing native `AsyncFileSystem` contract.
`SourceRead::step` performs one read of at most **4096 bytes** into caller-owned
whole-object storage and hashes only the returned bytes using the existing product
SHA-256. Short reads continue; zero, oversized counts, changed length/position,
stale lease, I/O failure and cancelled pending reads fail closed. A cancelled
read remains terminal even if the backend advanced its handle or wrote storage.
Always consume an open reader with `finish` or `abort`, including after failure.
There is no hidden asynchronous cleanup in Drop.

`finish` requires complete byte coverage, successful close and valid nonempty WAV
before binding the computed identity to the reserved job. Hashing includes RIFF
headers, ancillary and trailing bytes. The identity describes the captured object
bytes; it does not prove that externally mutable media stayed unchanged afterward.
No incomplete/unverified job can construct the core worker. Replacement or
unmount before delivery still invalidates it. Checked generation exhaustion
preserves the previous request and never wraps or resets the lane.

`VerifiedWav::worker` checks the configured session's rate/channels/duration against
the actual decoded source. `WavWorker::step` decodes at most **256 frames**, pushes
only accepted frames and performs one core step. Immutable source bytes allow
retry of a partial suffix without losing or duplicating PCM. Nonfinite float
input poisons the worker. Completion and waveform output retain only their caller
banks; actual Deck Beat Jump/Sync and raster rendering work after the worker,
source bytes and filesystem are destroyed. No new codec or DSP engine is added.

The host example now uses this acquisition path through a host-only std-file
fixture; its immediately-ready blocking filesystem operations must never be used
inside Embassy. Host tests separately exercise the **actual native async FAT32
and exFAT implementations** over generated owned-block images, including reversed
fragmented chains and exFAT NoFatChain storage. Those fixtures supply constructed
boot geometry and test file acquisition, not on-disk boot/mount qualification.
The P4 USB broker has dedicated Library/DeckA/DeckB channels and pending tickets;
this adapter does not take over one of those channels or enable a production lane.
The foundation's exFAT broker wiring, production codecs and scheduling remain its
own workstream. Open/close futures must be driven to completion; cancellation of
those operations requires backend-owned resource recovery. Poisoning the reader
prevents bad analysis but cannot itself restore a backend's in-flight DMA buffer.

Storage is explicit and checked before reading: source bytes must fit the supplied
slice (no allocation/growth), PCM scratch is **2048 B**, and each backend owns its
sector scratch. Final WAV parsing is bounded by the object size, not by the 4 KiB
read step. Existing core/beat/render banks are additional. Compile-time object
budgets cap `SourceRequest` at **384 B** and `WavWorker` at **4096 B** on host and
P4; these exclude all borrowed banks, backend futures and nested calls. The tested
RV32 objects are 324 B for the request, 232/256 B for FAT32/exFAT readers, and
3704 B for the WAV worker. Full-file storage is an experimental bounded profile,
not a suitable default for arbitrary tracks in 32 MiB PSRAM. A future streaming
codec/source contract is required before that production use.

`window::overview_with_eof` and `detail_with_eof` accept a shorter last column only
when the requested end equals the declared EOF from the same trusted result.
The start stays column-aligned and coverage must extend to that EOF. Each selected
source column, including the shorter tail, occupies one output column. There is
no duration-proportional pixel resampling. The original strict APIs still reject
partial columns; arbitrary crops, holes and automatic layer selection stay out.
EOF validation and short-capacity errors preserve the destination.

Final-source acceptance: `verify-consumer.py`, stamp **20261004T190329Z**, passes
**393 workspace tests including doctests**, **89 focused release tests including
doctests**, and **19 adapter tests + one lifetime compile-fail doctest under ASan**.
Formatting, Clippy, allocation counters, pinned Rust 1.95 P4 library compilation,
dependency inspection and optimized object generation pass. Nine source tests
include real FAT32/exFAT reads, forced short reads/PCM acceptance, I/O/close/count/
position/length failures, pending resume/cancellation, stale catalogs/completions,
geometry mismatch and terminal invalid-float decoding. Generation exhaustion and
real 300-frame partial-EOF overview/detail rendering have separate regressions.

`verify-extended-artifacts.py`, stamp **20261004T190445Z**, reproduces exact prior
PPM bytes for both fixture and silence through the new catalog read/hash path.
`verify-native-source-probe.py`, stamp **20261004T190446Z**, compiles and instantiates
the actual native filesystem acquisition generics on P4 with an asynchronous
owned-buffer device callback. Its one-poll diagnostic frames are FAT32 read
**2336 B**, exFAT read **2368 B**, FAT32 open **4720 B**, FAT32 finish **336 B**;
FAT32 nested sector loading is separately **1584 B**. These probes intentionally
create/poll/drop futures for object inspection, are not an executor, and are not
task-stack recommendations. The WAV step wrapper tail-calls the adapter; zero
wrapper frame does not mean zero worker stack. All evidence remains under
`/home/shome/.local/share/Pajoniiir-M1/apta-evidence/`. Intermediate failed formatting,
Clippy and diagnostic probes are retained alongside successful final runs.

## Bounded two-pass WAV acquisition — 2026-10-04

`SourceRequest::open_stream` preserves the same copied catalog path, originating
lease and persistent request generation. `ScanRead::step` reads at most 4096 bytes
into caller scratch and feeds the portable `WavScanner` while hashing all bytes.
`finish` consumes/closes the handle and requires complete valid nonempty WAV
framing before returning `VerifiedStream`. Data may precede format; all supported
mono/stereo S16/S24/S32/F32 and extensible profiles use the existing decoder.

Use `VerifiedStream::source` to size/configure the existing Session, then call
`VerifiedStream::worker` synchronously with a separate or reused `[u8;4096]`.
`StreamWorker::open` borrows that control in place and reopens the exact path
without a seek, requires the same length and Session geometry, and owns that
handle until finish/abort. `StreamWorker::step` reads at most once, decodes at
most 256 frames into caller `[f32;512]`, preserves partial-byte and unaccepted
PCM suffixes, and performs one core step. This is a cooperative work-unit bound,
not a latency bound. Filesystem operations and all processing stay outside audio.

`StreamWorker` deliberately exposes no completion/waveform getters. After all
bytes and core stages finish, `finish` consumes/closes its handle and requires
its second full-object SHA-256 to equal the first pass. Only then does synchronous `into_worker` release
an ordinary `Worker` for conversion and retained Deck pins. Premature extraction
returns the original streaming worker so its handle can still be aborted. This rejects changed
headers, ancillary chunks, PCM or trailing bytes even if geometry stays equal.
Hashes identify captured bytes; they cannot certify future external immutability.
Two sequential passes cost twice the object I/O but avoid whole-track retention
and linear-cost native random seeks. Non-WAV codecs are outside this profile.

Read cancellation poisons either pass before I/O, including cancellation after a
backend wrote bytes/advanced position. A failed reader/worker must still be
consumed with `finish` or `abort`. Open and close futures must run to completion.
The backend owner must recover pending operations, handles and DMA buffers;
analysis poisoning provides no filesystem cancellation guarantee. Reopening is
an ownership boundary, not permission to mix filesystem instances/client lanes.
Use the same filesystem owner and persistent `Requests` throughout both passes.

The host example now uses this path. Its blocking std-file fixture remains
host-only. Tests run both whole-object and streaming paths through native FAT32
fragmented and exFAT contiguous/fragmented files, real local PCM analysis, Deck
Sync/Beat Jump and exact RGB565 pixels. Additional tests exercise one-byte reads,
stereo packed24, data-before-format, partial EOF, short queue acceptance, changed
objects, stale requests/media, invalid counts/geometry, pending resume/drop and
close failure. Instrumented acquisition, processing and retained conversion
perform zero allocations; independent output banks outlive the filesystem.

### Live storage and placement boundary

Caller source scratch is **4096 bytes**; decoded PCM scratch is **2048 bytes**.
The same byte bank can serve the first pass and second pass because their
lifetimes do not overlap. Source scratch no longer scales with object length.
Compile-time control budgets exclude backend handles and caller storage:
`ScanRead<()> <=768 B`, `StreamWorker<()> <=4864 B` (includes Session control).
The portable scanner is <=192 B. These ceilings are not runtime stack sizes.

For the tested local-tempo profile, simultaneous retained/caller banks cost:
`81920 + 8*queue_frames + 14*overview_columns + 24*beat_capacity` bytes, plus
6144 source/PCM scratch and worker/backend/future/control storage. The 81920 is
4096 OnsetBins plus 4096 flux entries; 14 per overview column covers the native
10-byte column and rendered 4-byte column. The beat term includes **two** 12-byte
banks. At 17 queue frames, 1250 columns and two 128-beat banks this is **108772 B**
of caller arrays including source/PCM scratch, before control/backend/futures,
stack, catalog and existing pinned-track banks. Duration-dependent capacities
must be checked before opening the second pass. Optional global/key/detail/cache
profiles require their own additional storage and are not covered by this sum.

The original whole-object APIs remain available for explicitly bounded immutable
buffers. Neither the 256 MiB host cap nor these diagnostic profile sizes establish
a production 32 MiB PSRAM policy. Allocate latency-critical/DMA structures only
in qualified internal memory; PSRAM bank/cache placement requires ADR-008 review
and measurement. Future storage, nested call chains, executor/task overhead and
interrupt stacks must be included in final live/peak budgets. Production broker
analysis-lane ownership, exFAT handle wiring and provider enablement stay gated.


P4 diagnostics exposed a large intermediate future when Session moved through
async open/finish. The final API constructs the worker synchronously, borrows it
for I/O and extracts it synchronously after verification. The early open/finish
futures were 14592/8464 B; current FAT32 futures are 2936/504 B. Their one-poll
frames fell from 18672/12656 B to 3136/224 B. The synchronous worker control grows
slightly to retain the reopen path; this avoids large Session copies in suspendable
I/O storage without adding allocation or another runtime.


Final-source serial verification: `verify-consumer.py`, stamp
**20261004T203840Z**, passes **400 workspace tests**, **96 focused release tests**
(including doctests), and **26 adapter tests plus one lifetime compile-fail doctest
under ASan**. Formatting/Clippy, pinned Rust 1.95 P4 compile/dependency/object
checks and zero-allocation instrumented native filesystem/analysis paths pass.
The dependency is pinned to Libapta `a242c4ab1ada6ac2547f63332ef7f7c0f36276fa`.
`verify-extended-artifacts.py` stamp **20261004T203948Z** reproduces exact prior
impulse/silence PPM bytes through streaming (120 BPM, 80 beats, 4120 green pixels;
silence no grid and 800 centerline pixels). Source tests additionally compare every
retained beat and waveform column between whole-object and streaming paths.
The unchanged whole-object P4 probe passes at **20261004T203949Z**.

`verify-streaming-source-probe.py`, final stamp **20261004T204041Z**, instantiates
actual FAT32/exFAT generics, pending owned-buffer callbacks and the pinned target:

| RV32 diagnostic | FAT32 bytes | exFAT bytes |
|---|---:|---:|
| ScanRead including handle | 632 | 656 |
| StreamWorker including handle/Session | 4256 | 4280 |
| Native filesystem with diagnostic 512-byte backend | 592 | 600 |
| Streaming step future | 1408 | 1440 |
| One-poll scan frame | 1408 | 1456 |
| One-poll streaming frame | 1488 | 1520 |

VerifiedStream is 456 B and WavScanner 128 B. FAT32 first-pass open/finish futures
are 3568/1776 B; second-pass open/finish futures are 2936/504 B. Disassembly
confirms nested native filesystem read/cluster/sector-load calls: their distinct
frames include adapter read 96/112 B, backend read 80 B, FAT32 next-cluster 32 B
and sector loading 1584 B. No flat worker-frame number covers this chain. The
one-poll probes place their futures on the stack: do not add future storage again
to that probe frame as if it were separate. Real executor placement differs.

Synchronous construction/extraction probe frames are 3648/16 B.
These are compiler diagnostics, not complete task-stack budgets, timing bounds
or hardware recommendations. Full constructor/caller/executor/library/interrupt
paths and concurrent Deck/UI/audio storage remain to be qualified. Logs, source,
objects, disassembly and reproducible scripts live only under
`/home/shome/.local/share/Pajoniiir-M1/apta-evidence/`. Initial larger-future
measurements/source are preserved alongside the final report. Broader i686,
Windows/MSVC, AArch64 and Libapta-runtime sanitizer matrices were not rerun.
