# S6 Rust read-only media bridge: protocol and owned-buffer client

Date:2026-10-30. Scope: device-free continuation of the
[known-file SD campaign](2026-10-30-legacy-s6-sd-file.md).
Status: host protocol/ownership tests and bare Rust target build observed;
IDF transport, filesystem integration and physical bridge execution NOT_RUN.
This describes the frozen protocol-only snapshot; the subsequent
[concrete IDF transport](2026-10-30-legacy-s6-idf-media-transport.md) has its own
implementation/build/runtime scope and evidence.
No COM3 access, reset or flash write occurred in this batch. The board retains
the previously verified read-only C/FatFs file diagnostic, not this bridge.
Native minimum302 and the shared/native product sources remain unchanged.

## Implementation

The optional `media-bridge` feature in the isolated
[Rust static library](../../firmware-idf/legacy-jc1060/rust-core/Cargo.toml)
reuses `ruzina-media-block`, `ruzina-media-io` and `ruzina-media-session`.
There are no new registry dependencies. The new
[protocol and client](../../firmware-idf/legacy-jc1060/rust-core/src/media_bridge.rs)
forbid unsafe code and introduce no allocator, HAL, executor or RTOS dependency.
Existing diagnostic variants do not enable this feature. The panic handler is
excluded only under `cfg(test)` so host tests can use Rust's test harness; target
panic behavior still calls IDF-owned `abort` without unwinding across C.

`ReadQueue<B>` is a bounded single-slot state machine:
Empty -> Queued -> Active -> Complete -> Empty. The worker takes ownership of
the whole `ReadRequest<B>`, not a borrowed pointer. A read is limited to4096
bytes,512-byte sectors and32-bit card sector counts. Geometry, overflow, range,
exact buffer length and capacity are validated before admission. A request
sequence is64-bit and refuses wrap. Tickets bind the request to its shared
media lease, hardware-owner handle, geometry and sequence. Runtime owner
handles must be unique. Only the expected ticket can take a completion; a
foreign worker completion is returned intact for owner fault/drain handling.

Activation means initialized block media; it does not call
`MediaSession::commit_mounted` or claim a mounted Rust filesystem. Invalidation
disconnects the shared session first. Queued work becomes a stale completion
without starting DMA. Active work stays with the worker; hardware teardown is
blocked until it returns the request. A completed buffer may then await client
recovery while hardware is safe to deinitialize. Reactivation waits for the
old slot to drain and yields a new shared lease. Existing shared generation
semantics apply; the bridge's request sequence is never recycled.

`ReadClient<P>` implements only `OwnedBlockDevice`, with no writable trait.
Its `ReadPort` is a nonblocking platform contract, not an implementation of an
IDF queue. The client validates again before submission and checks the lease
after completion delivery. Driver errors and short/oversized completions
return the original buffer as an error. An IO error alone does not infer a
card-detect event; the future worker must explicitly invalidate on media loss.
Higher-level track/load commits must still validate their own media lease.

Canceling a submitted read future keeps the pending ticket in the client and
the buffer in the queue/worker. Another read returns Busy with its own buffer;
`recover_pending()` retrieves the old completion. Canceling recovery also
retains the ticket. Dropping an unpolled future drops its never-submitted
buffer normally. Dropping the client during active IO does not recall or free
the worker's buffer: the persistent owner must drain it. There is no bounded
hardware-abort guarantee yet, and cancellation does not mean DMA has stopped.

## Validation

The [host tests](../../firmware-idf/legacy-jc1060/rust-core/src/media_bridge/tests.rs)
exercise19 scenarios with owned allocation identity/drop counters and controlled
polling. They cover invalid transfers/no submission, successful buffer return,
driver errors/short reads, removal before submission/queued/active/after
completion, pending-future/recovery/client drop, backpressure, wrong tickets,
sequence exhaustion, reinsertion and stale-client rejection. A delivery-race
port returns success then invalidates before the client's post-check, ensuring
that check is tested independently of the queue's stale-result gate. Waker
tests cover completion and queued invalidation; eager completion before the
first wait is also observed. This uses a single-threaded serialized host port,
not a simulated dual-core FreeRTOS scheduler or physical DMA.

Local checks:19 bridge tests and21 shared media tests (block6, IO4, session11)
passed. Rustfmt, host test Clippy, target Clippy and release static-library
build for `riscv32imafc-unknown-none-elf` passed with Rust1.95.0; warnings are
errors. Commands, source hashes, logs and artifact identity are preserved in
the evidence index added below. A target static-library build type-checks this
generic protocol; it does not prove a concrete IDF port, final bridge link or
runtime behavior. The new `media-bridge-host` CI job repeats these Rust gates;
remote CI was not run or published in this batch.

Fresh scalar IDF regression and final documentation/unit checks are recorded
in the final evidence index. The existing firmware package on the board and
its full programming ancestry remain immutable. Current protocol sources are
not the source snapshot of that previously flashed package.

The [offline evidence index](../boards/readiness/jc1060-s06-rust-media-bridge-offline-20261030.json)
links the command results/logs and identifies the frozen source/artifact ZIP
and independent secondary copy. Fresh scalar IDF6.1 package generation passed
without enabling the bridge feature; this is a regression of the existing
scalar shell and Rust panic/link boundary, not an IDF bridge link. Its generated
binaries were not flashed. The17 existing Python tooling tests, unit validator,
documentation-link check and `git diff --check` also passed. Full product host/
native gates were not repeated because their sources are unchanged.

## Next implementation gate: concrete IDF transport

1. Add a separate opt-in bridge profile. Keep the current C/FatFs diagnostic
   as the recovery predecessor. Use exactly one persistent IDF SD owner task;
   no duplicate native SD owner or second filesystem/card instance.
2. Implement synchronized storage for the Rust queue, retained pending ticket
   and waker. Serialize submit/take/complete/invalidate across both cores.
   Register/recheck readiness atomically and notify outside the queue lock.
   Never hold that lock across blocking SDMMC or a callback into the executor.
3. Keep card/host/LDO and a bounded internal DMA-capable bounce buffer with the
   IDF task. Borrow only pointer+explicit length for the synchronous worker
   call; the IDF task must finish DMA before copying/returning data. Do not pass
   Rust references, Vec, trait objects, futures or request layouts through C.
   Reject null/alignment/length/overlap and invalid range before driver access.
4. On removal/error, invalidate before teardown, finish/drain active IO, return
   every buffer once and wake the pending client with an error. Prove that an
   orphaned completion is drained if the client was dropped. Missing card-detect
   remains an explicit IO/status-based policy; do not invent a CD GPIO.
5. Connect the Rust partition/exFAT reader through `OwnedBlockDevice`, using the
   existing shared read path. Choose and document mount/buffer ownership before
   replacing C/FatFs. No persistence, write or format path in this profile.
6. Run FFI/queue ownership and cancellation host tests, a fresh exact IDF link
   and no-write-symbol checks; review the complete programming predecessor
   chain before flashing. Then repeat the known fixture SHA/EOF/seek, app
   restarts, active-file removal/reinsert and exact-image cold boot. Keep
   host/build/runtime/operator outcomes separate.

The current physical predecessor is
`D:/BoardEvidence/JC1060-20261030-COM3/s06-sd-file-timing-write-plan/plan.json`,
with app SHA-256 `f501dd087eec7bec7dff4729deb0f213a0d15d18fbe2ae8f1824e38519d9e48e`.
For a future upgrade, replay all five entries in that plan's `predecessor_chain`
oldest-first and then this plan as the immediate predecessor. Do not use the
new scalar regression package as the physical predecessor: it was never flashed.

USB host, audio, C6, animation and concurrent display/SD remain later gates.
APTA stays OFF/unmerged. No commit/push is part of this batch.
