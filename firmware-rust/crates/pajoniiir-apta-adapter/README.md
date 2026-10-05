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
The integrated foundation routes FAT32 and exFAT through that broker. A uniquely
owned analysis client, production codecs and scheduling remain separate work. Open/close futures must be driven to completion; cancellation of
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

## Streaming policy and full-task storage — 2026-10-04

`SourceRequest::open_stream_with_limits` takes explicit `StreamLimits`: inclusive
object bytes, duration in milliseconds, first-pass step attempts and second-pass
step attempts. Zero allows none of the corresponding work. The existing
`open_stream` preserves its externally governed profile through `UNLIMITED`.
These values are caller policy; the adapter chooses no firmware defaults.

The opened length is checked before reading, with the handle closed on rejection.
Duration uses exact `frames * 1000 <= duration_ms * sample_rate` wide arithmetic
at complete first-pass framing/close, before source verification and Session/bank
construction. Fractional milliseconds are never rounded down into acceptance.
A step budget is charged before starting work, including a pending read. Resume
of the same future does not charge again. Exhaustion returns `SourceError::Limit`,
poisons the job and prevents delivery; the caller must still finish/abort its
handle. Close failure is reported as I/O failure. Completed step calls are
idempotent and do not spend more budget or perform more I/O. Work counts include
header/trailer processing and PCM backpressure; they are not deadlines and do
not limit filesystem open/close latency or time spent awaiting one operation.

`storage::LocalTempoStorage::plan` preflights known nonempty duration, the existing
Session column geometry and checked target-`usize` array arithmetic against a
caller byte ceiling. It reports onset/flux, queue, native/rendered overview,
**two** retained beat banks and fixed byte/PCM scratch separately. Beat capacity
remains caller policy; conversion can still reject insufficient capacity. The
plan includes no control objects, filesystem, catalog, other tracks, framebuffer,
executor, stack or optional global/key/detail/cache storage. It does not allocate
or select SRAM/PSRAM placement. The tested 320000-frame / 256-frame-column /
17-frame-queue / two 128-beat-bank profile is exactly **108772 B**; one byte less
fails preflight. Output overflow and invalid/unknown geometry fail explicitly.

The host example applies limits to the opened object (replacing a separate
metadata snapshot): 256 MiB, two hours, one million scan steps, four million
processing steps and 32 MiB for the above caller-array plan with 65536 beats per
bank. These are host example policy, not firmware recommendations. The example
uses one active beat bank while the conservative plan reserves space for two.
Native FAT32/exFAT fixtures use exact object length, 40 seconds, 20000 scan steps,
40000 processing steps and the 108772-byte profile. They still compare every beat
and waveform column exactly against whole-object acquisition and drive real
Deck Sync/Beat Jump and raster consumers with retained outputs.

`StreamWorker::verified_worker` borrows the ordinary Worker in place after equal
hashes and successful close. This avoids moving Session through an async caller's
large extraction/error temporaries. Before verification, after error/abort or
cancelled close it rejects access. Existing Worker request/media checks still
apply on every use. The consuming `into_worker` API remains available and keeps
its handle-recovery-on-premature-extraction contract. The example and native
filesystem fixtures now use borrowed verified access.

### Complete coroutine and Embassy diagnostic

External `task-storage-probe` uses pinned Rust 1.95, Embassy executor **0.10.0**
and futures **0.1.2**, matching the firmware's declared versions, with direct
native FAT32/exFAT generic backends and caller-owned banks. It compiles actual
`#[embassy_executor::task]` pools around a complete two-pass coroutine: finite
scan, source verification, checked storage plan, Session configuration, cooperative
yields, streaming, close/hash acceptance, retained conversion, Deck pin and a
neutral analysis/waveform sink. A host test suspends every read once, resumes each
future, and reaches 120 BPM / 80 beats / 4120 green pixels with zero allocations.
This is a software diagnostic; it is not a registered firmware task, a USB broker
client or a running Embassy executor acceptance test.

The naive complete job exposed much larger storage than individual step probes.
Borrowed verified access plus a separate synchronous constructor reduce its
compiler-observed sizes as follows (FAT32 / exFAT, bytes):

| Diagnostic | Initial | Final |
|---|---:|---:|
| Complete job future | 12968 / 13272 | 9424 / 9728 |
| Embassy single-task pool including header/alignment | 13336 / 13640 | 9792 / 10096 |
| Task polling frame, excluding nested calls | 33344 / 33280 | 21008 / 21424 |
| Task initialization `write_in_place` frame | 13344 / 13648 | 9792 / 10096 |

Final synchronous preparation separately uses **6976 B**; sector loading remains
**1584 B**. The task polling frame calls preparation, so already that known nested
path uses **27984 / 28400 B before preparation's own callees**, executor caller or
interrupts. A small one-step future never established a safe task stack. The
future is stored inside the task pool: do not add it, Session, StreamWorker or
child I/O futures again to that pool. Caller arrays (108772 B) plus pool and
native backend (592 / 600 B) total **119156 / 119468 B** before Requests/media,
catalog, framebuffers, other live tracks, executor state and stack. These are
identified storage subtotals, not a maximum, placement recommendation or runtime
high-water measurement. Actual firmware executor features and backend/DMA/timing
coexistence still require qualification. No hardware was operated.

Reproduce serially with two build jobs using the external evidence scripts,
including `verify-task-storage-probe.py`. The complete probe, lockfile, dated
source snapshot, object, disassembly and host output are retained under
`/home/shome/.local/share/Pajoniiir-M1/apta-evidence/`. The successful complete-job
probe stamp is **20261004T210659Z**; earlier 210313Z/210423Z/210507Z retain the
initial and intermediate measurements. The test-harness-only initial no_std
prelude compile error was corrected without changing product source.

Production prerequisites remain the foundation's unique analysis-client lifetime,
pending ticket/completion draining and handle/buffer recovery, then target
placement and timing qualification. The foundation now routes exFAT file handles.
No client channel is borrowed, no foundation checkout is changed, and neither
firmware nor Slint selects APTA. Original DSP accuracy and C compatibility remain
independent gates.

Final-source acceptance: `verify-consumer.py` **20261004T210742Z** passes
**405 workspace tests**, **101 focused release tests**, **31 adapter tests plus
one lifetime compile-fail doctest under ASan**, formatting/Clippy, allocation
instrumentation and pinned P4 compilation/dependency/object inspection. Artifact
rerun **20261004T210932Z** preserves exact impulse/silence PPM bytes. Native and
streaming P4 probes pass at **20261004T210933Z / 20261004T210938Z**. Current policy
control sizes are ScanRead **672 / 696 B**, StreamWorker **4264 / 4288 B** and
VerifiedStream **464 B**. Streaming step futures/one-poll frames remain
**1408 / 1440 B** and **1488 / 1520 B**. Compatibility first-pass open/finish
futures are **3984 / 1856 B**; second-pass open/finish stay **2936 / 504 B**.
Broader i686, Windows/MSVC, AArch64 and Libapta-runtime sanitizer matrices were
not rerun. The portable Cargo pin remains unchanged.


## Original lease checks and direct control preparation — 2026-10-05

Whole-object and both streaming passes now recheck filesystem/file leases against
the original mounted media after open/read and after close (not only against
each other). A backend replacing both its lease and handle during an operation
cannot bind replacement bytes to the old catalog job. Regression cases cover
replacement during each operation, rejected delivery and exactly-once close.
Read failure still poisons analysis; backend resource recovery and driving
open/close to completion remain owner obligations.

`VerifiedStream::prepare_local_tempo` constructs the existing Session local-tempo
and meter profile directly into an empty caller `Option<StreamWorker>` using
borrowed `storage::LocalTempoBuffers`. It allocates nothing and chooses no
placement. Occupied slots fail unchanged before touching buffers, preserving
possibly open handles for recovery. Other failures leave the slot empty; working
arrays may have been initialized. The general configured-Session constructor
remains available for other profiles. Storage preflight and beat capacity remain
caller policy. The host WAV example, actual FAT32/exFAT fixtures and complete
coroutine diagnostic now use direct preparation. Exact retained beats, waveform
columns, Deck Sync/Beat Jump and raster output remain unchanged, with no measured
allocations in native acquisition/construction/processing/conversion.

The synchronous non-inlined boundary avoids returning a Session-bearing Result
through the caller's polling frame. Pinned Rust 1.95 complete-task diagnostics
(FAT32 / exFAT, bytes) now report:

| Quantity | Previous | Direct preparation |
|---|---:|---:|
| Job future | 9424 / 9728 | 9440 / 9744 |
| Embassy pool including header/alignment | 9792 / 10096 | 9808 / 10112 |
| Polling frame | 21008 / 21424 | 13328 / 13776 |
| Synchronous preparation frame | 6976 / 6976 | 7136 / 7136 |
| Identified poll + preparation path, before further callees | 27984 / 28400 | 20464 / 20912 |

The pool grows 16 bytes while removing 7488–7520 bytes from that identified
nested path. Arrays + pool + diagnostic backend are 119172 / 119484 bytes;
future/Session/worker/child I/O are already inside the pool and must not be
counted twice. Initialization frames are 9808 / 10112 bytes. These are compiler
diagnostics, not whole-stack maxima, placement advice or runtime high-water data.

The actual firmware resolves Embassy executor 0.10.0 through esp-rtos 0.4.0 with
no scheduler-priority/deadline features; it uses panic-abort and forced frame
pointers. A second external diagnostic compiles all dependencies with those
code-generation settings and records 1487 function frames plus disassembly.
Its polling frames are 13328 / 13760, preparation 7040, Session::new 16,
raw Executor::poll 16, native sector load 1584, Worker::step 112 and its
Session::process_inner 144 bytes. Relocations identify constructor and core
processing calls. Unused functions in that inventory are not live storage;
indirect calls, compiler helpers, actual esp-rtos caller/interrupt paths,
initialization placement and simultaneous workload timing still need analysis.
The diagnostic is not linked into or executed on production firmware.

Foundation advances integrated here route exFAT and FAT32 through backend-neutral
broker handles. The obsolete ExFatUnavailable blocker is closed. Unique analysis
client lifetime, pending completion draining, original lease enforcement and
exclusive handle/buffer recovery remain prerequisites; no existing client lane
is borrowed and neither firmware nor Slint selects APTA. ADR-006 remains gated.

Final software acceptance: consumer **20261005T050057Z** passes **430 workspace
tests**, **104 focused release tests**, **34 adapter tests + one lifetime
compile-fail doctest under ASan**, fmt/Clippy, allocation instrumentation and P4
compile/dependency/object checks. Artifacts **20261005T050216Z** match prior PPM
bytes exactly (120 BPM / 80 beats / 4120 green pixels; silence unchanged); the
impulse PNG was visually inspected. Native/streaming probes are
**20261005T050217Z / 20261005T050222Z**. Complete-task final evidence is
**20261005T050332Z**, including the pending-I/O host test. Its script now emits
to an explicit dated object path: newest-file selection could pick a cached
object from another flag profile. Supersede the 050223Z mixed-object diagnostic;
retain it and the failed 050307Z reproduction for audit.

`verify-task-nested.py` reproduces the second inventory in external
`task-nested-20261005T050039Z`; source, lock, configuration, rlibs, frames.json
and disassembly are retained. All scripts/evidence live under
`/home/shome/.local/share/Pajoniiir-M1/apta-evidence/`. Run verification serially
with two build/test jobs. The unchanged portable core passed the combined
`consumer-construction-baseline-20261005.log`: 123 C tests, 279 ordinary Rust
and 55 external-C groups per debug/release, 34 WAV interchange and two all-feature
CLI cases. Its immutable source pin is unchanged. Broader core/runtime/C
sanitizers and i686/Windows/AArch64 matrices were not rerun. No hardware,
deployment, DSP algorithm, C header/ABI/container or numerical backend changed.


## Cooperative second pass and caller control — 2026-10-05

`StreamWorker::run` drives an unopened worker through reopen, bounded processing,
second close and full-object hash acceptance. It borrows the worker and existing
512-f32 scratch. Each incomplete step wakes and yields once even when every I/O
operation is immediately ready; backend pending futures resume without duplicate
reads or budget charges. It adds no executor dependency, allocation, deadline or
firmware scheduling policy. Manual open/step/finish remains available.

Processing errors trigger close; a cleanup error takes precedence. Dropping a
pending read or cooperative yield prevents subsequent delivery and leaves caller
control available for abort. Backend operation/buffer recovery must happen first
where required. Open and close futures still must run to completion: this driver
cannot recover a handle consumed by a cancelled backend open/close.

Keep the `Option<StreamWorker>` outside a driving coroutine when control must
survive cancellation. Pass a mutable borrow to preparation and to `run`, and keep
the source/core arrays alive until that control is dropped. This also avoids
putting the large control value in the coroutine's own state. The external slot
still consumes storage and must be counted once alongside the task pool.

The host WAV example and actual native FAT32 fragmented / exFAT contiguous and
fragmented consumers now use `run`. Retained beats and waveform columns still
match whole-object processing exactly and reach actual Deck Sync/Beat Jump and
RGB565 rendering. Cancellation tests cover both cooperative yields and pending
reads, including a backend that advances before returning Pending, plus wakeups,
zero allocation, work exhaustion and exactly-once close/cleanup-error reporting.

The immutable Libapta pin is `e93e667ba010d7256e06582f05cf5da0b94846e5`.
Its only core change permits cross-crate inlining of the existing borrowed
`Session::new` wrapper, eliminating large construction temporaries in the measured
consumer without a new API or algorithm. Source/test behavior, C, ABI and wire
format remain unchanged. Compiler frame measurements are not portable guarantees.
Final-source verification and expanded diagnostics are recorded below.


### Complete-task and executor evidence

The revised complete two-pass diagnostic passes both scanner and worker control
slots by mutable reference. Occupied slots are rejected before I/O. Cancellation
of a pending first-pass read leaves its poisoned scanner handle available for
abort, independently of the dropped job future. No slot fixes backend open/close
cancellation. The host tests also execute a real Embassy 0.10 task pool and raw
executor, including task initialization, pending I/O and cooperative wakeups,
with zero measured allocations after fixture setup. This host executor test is
not an execution of esp-rtos or P4 firmware. Diagnostic-only unsafe executor
polling and host static fixture allocation stay outside product/library source.

Pinned Rust 1.95 standard RV32 measurements (FAT32 / exFAT):

| Storage/frame | Previous | Current |
|---|---:|---:|
| Complete job future | 9440 / 9744 B | 4040 / 4288 B |
| Embassy task pool, header/alignment included | 9808 / 10112 B | 4416 / 4664 B |
| External scanner control | included in old pool | 672 / 696 B |
| External worker control | included in old pool | 4264 / 4288 B |
| Task polling frame | 13328 / 13776 B | 8432 / 8800 B |
| Synchronous preparation frame | 7136 B | 128 B |
| Identified poll + preparation subtotal | 20464 / 20912 B | 8560 / 8928 B |
| Task initialization `write_in_place` frame | 9808 / 10112 B | 4416 / 4672 B |

Initialization frames and pool sizes are distinct measurements; the exFAT
initialization frame includes 8 B beyond its pool. Arrays + pool + external
controls + diagnostic backend total **118716 / 119020 B**, compared with
119172 / 119484 B previously. The 108772 B caller arrays are unchanged. Do not add
child futures or a second Session/worker: their storage is already accounted for.
These subtotals exclude other owner state, stack, catalog, other tracks and display.
No maximum stack, memory placement or runtime high-water claim follows.

The separate actual combined firmware diagnostic builds/links USB0 + USB1 +
SDMMC profile storage + PSRAM with panic-abort and forced frame pointers. It
records 4211 compiled frames, resolved rlibs, a dated object and linked ELF plus
disassembly under `firmware-frames-20261005T064414Z` in the external evidence root.
The observed entry/executor chain includes `_start_rust` 32 B, `hal_main` 16 B,
`main` 32 B, esp-rtos `Executor::run_inner` 48 B and raw `Executor::poll` 16 B.
Interrupt assembly saves 64 B before dispatch; handler/callee/context-switch and
nesting costs are additional. This is not a complete interrupt-stack bound.
APTA is absent from that firmware, so these numbers are not a combined APTA
firmware qualification. The inventory also exposes SD/profile owner 21184 B,
Library polling 12800 B and filesystem worker 9264 B individual frames; these
are foundation-owned integration concerns, not newly enabled APTA tasks or
established maximum paths. Indirect callees, simultaneous workload timing,
initial storage placement, SRAM/PSRAM, DMA/cache and physical gates remain open.

Reproduction adds `verify-firmware-frames.py` to the existing external scripts.
Source/native probe scripts now use Cargo-resolved rlib paths; raster/stack
verification emits an explicit dated object. No script selects the newest cached
object or rlib. Earlier evidence and failed experiments remain preserved.


The later foundation integration adds the existing bounded `DeckEffectBroker`
consumer to the native filesystem fixtures. A full transport lane refuses a
staged Beat Jump without committing Deck state; after draining it, the retained
pin supplies the same analysis and the retry enqueues the exact seek once. This
uses the foundation's owner-lane contract without enabling APTA in its production
`ProductRuntime`. Controller profile/MIDI, product dispatch and semantic LED
implementation remain foundation-owned; production analysis still needs its own
media and retained-bank ownership agreement.


Final-source acceptance after integrating foundation dispatch/LED work through
`a078c23`: `verify-consumer.py` **20261005T064308Z** passes **446 workspace tests**,
**110 focused release tests**, **36 adapter tests plus one lifetime compile-fail
doctest under ASan**, formatting, Clippy, allocation instrumentation and pinned
Rust 1.95 P4 checks. Artifacts **20261005T064400Z** retain exact impulse/silence PPM
bytes (impulse visually inspected through its byte-identical preceding render).
Native and streaming probes **20261005T064402Z / 20261005T064407Z** pass.
Complete-task **20261005T064408Z** passes three host tests, including actual Embassy
execution and cancelled first-pass recovery, plus P4 pool/object/disassembly.
Nested profile **task-nested-20261005T064412Z** records 1493 library/task frames
with panic-abort/forced-frame-pointer settings; polling/preparation match the
standard values above. All final reproduction scripts ran serially with two
build/test jobs. Exact script/probe hashes and publication/CI evidence are in
`cooperative-acceptance-20261005.json` and the new complete handoff
`/home/shome/.local/share/libapta-audio/handoffs/pajoniiir-libapta-native-rust-continuation-2026-10-05-0645.md`.
No hardware, deployment, production provider selection, C compatibility or new
DSP accuracy acceptance is claimed. Broader runtime/C sanitizers and i686/Windows
matrices were not rerun; Libapta separately passed its combined suite, 38 optimized
ASan tests and portable AArch64 compilation for the constructor change.

## Cooperative first pass and acknowledged effects — 2026-10-05

`ScanRead::run` borrows caller-owned scan control and source scratch, drives the
existing bounded `step`, and self-wakes/yields once after each incomplete step.
It does not reset work limits, close a handle, or authorize a verified source.
Call `finish` after success; call `abort` after failure, after recovering any
backend pending operation. Both close futures must run to completion. Cancelling
a pending read **or this driver's cooperative yield** poisons the scan and retains
its handle in caller control. Manual `step` callers retain their existing contract;
an arbitrary yield written by a caller does not automatically poison that scan.
Completed driver calls perform no further I/O, spend no budget and do not yield.

The host example, native fragmented FAT32 and contiguous/fragmented exFAT fixtures,
and complete-task diagnostic use both cooperative pass drivers. Exact captured
hashes, every retained beat/column, 120 BPM, 80 beats and 4120 green raster pixels
remain unchanged. No executor dependency, allocation, source codec, portable-core
change, or firmware/Slint provider selection was introduced.

Foundation integration now uses `DeckEffectBroker::transport_front` plus
`complete_transport`. The real native consumer fixture covers queue refusal before
Deck commit, retention of the original Pause on downstream failure, retry from the
same retained analysis pin, and retention of the exact Seek until acknowledgement.
Successful acknowledgement drains it once; failed acknowledgement never advances
the queue. This verifies the neutral consumer transaction, not a production audio
owner or exactly-once execution of an external device command.

The broker still exposes only Library/DeckA/DeckB completion clients. Its pending
ticket survives cancelled receive and can be drained by that client, but client
construction does not establish a unique analysis-owner lifetime. A production
analysis client still needs foundation-owned unique lifetime, original lease
checks, unexpected/pending completion recovery and exclusive handles/buffers.
The integrated GPT integrity, profile hotplug and controller LED changes do not
establish those analysis contracts. exFAT dispatch is already implemented.

### Complete-task construction and final evidence

The external diagnostic previously marked the full-job async constructor
`#[inline(never)]`. Allowing it to inline removes a large coroutine construction
copy in the enclosing Embassy task poll; it does not change the core or decrease
resident task storage. The controlled pre-inline run is `20261005T073202Z` and
final reproduction is `20261005T073653Z`, under the existing external evidence root.

Pinned Rust 1.95 standard RV32 diagnostic, FAT32/exFAT:

| Quantity | Before | After |
|---|---:|---:|
| Complete job future | 4040 / 4288 B | 4040 / 4288 B |
| Embassy pool | 4416 / 4664 B | 4416 / 4664 B |
| Poll frame | 8432 / 8800 B | 4432 / 4560 B |
| Preparation frame | 128 B | 128 B |
| Identified poll + preparation subtotal | 8560 / 8928 B | 4560 / 4688 B |
| Initialization write_in_place frame | 4416 / 4672 B | 4416 / 4672 B |

External scan slots remain 672/696 B; worker slots 4264/4288 B. Caller arrays
remain 108772 B for the existing profile. Arrays + pool + both external controls
+ diagnostic backend remain 118716/119020 B, excluding other owners and stack.
Do not count the coroutine again inside its pool. Future-size probe functions now
return constants with zero stack frames; those zero frames are not zero-sized
futures. Constructor inlining is a measured compiler result, not a portable bound.

The panic-abort/forced-frame-pointer nested run `task-nested-20261005T073656Z`
records 1491 frames. Its poll frames are 4432/4544 B and preparation 128 B; do not
mix its exFAT number with the standard profile. Actual combined firmware compile/
link `firmware-frames-20261005T073659Z` records 4213 frames with APTA absent. Both
inventories include unused functions and are not complete call-path maxima.
Initialization, indirect callees, interrupt nesting, placement, DMA/cache and
simultaneous hardware workload timing remain unqualified.

Final-source software checks, all run serially with two build/test workers:

- Consumer `20261005T073545Z`: 454 ordinary workspace tests and one lifetime
  compile-fail doctest; 114 focused release tests and that doctest; 39 adapter
  ASan tests and that doctest. Formatting, Clippy, allocation instrumentation,
  P4 compile/dependency/object checks pass.
- Raster `20261005T073645Z`: impulse/silence PPM bytes exactly match the prior
  inspected output; native source `20261005T073647Z` and streaming source
  `20261005T073652Z` probes pass.
- Complete task `20261005T073653Z`: three host tests pass, including actual
  Embassy initialization/execution with a second self-yielding task progressing
  during analysis, zero measured allocations after setup, and retained first-pass
  recovery on cancellation at either a pending read or a cooperative yield.
  This proves host executor progress, not fairness deadlines or target execution.

Reproduce with the existing `verify-consumer.py`, `verify-extended-artifacts.py`,
`verify-native-source-probe.py`, `verify-streaming-source-probe.py`,
`verify-task-storage-probe.py`, `verify-task-nested.py` and
`verify-firmware-frames.py`, serially. Explicit dated objects and Cargo-resolved
libraries remain mandatory. Earlier failed/superseded logs remain archived.
No hardware, broad runtime/C sanitizer, i686 or Windows acceptance is claimed.
