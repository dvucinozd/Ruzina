# S6 concrete IDF/Rust read-only SD transport

Date:2026-10-30. Unit scope:JC1060-20261030-COM3, P4v1.3.
This follows the [host protocol/client gate](2026-10-30-legacy-s6-rust-media-bridge.md).
The protocol snapshot is frozen separately; this report tracks its concrete
IDF implementation. The new `sd-rust-bridge` profile reads raw blocks only.
It does not inherit the earlier C/FatFs fixture/mount acceptance or claim
that the shared Rust filesystem/product load path is connected.

## Ownership and execution

- [C owner](../../firmware-idf/legacy-jc1060/main/sd_bridge.c): one persistent
  FreeRTOS task on core0, priority4,8192-byte stack. It owns SDMMC slot0,
  LDO4, card state and one512-byte internal DMA-capable allocation. Pin/bus
  settings match the prior measured diagnostic: D0..3=39..42, CLK43, CMD44,
  four-bit10MHz, no invented card-detect GPIO. Its transaction wrapper rejects
  the same memory write/erase/lock commands as the earlier diagnostic.
- [Rust transport](../../firmware-idf/legacy-jc1060/rust-core/src/idf_media_bridge.rs):
  the static queue, active request and waker persist until reset. All accesses
  share one IDF portMUX across both cores. The only mutable Rust buffers are
  owned512-byte arrays; no queue/request/future layout crosses C. The C worker
  receives scalar LBA/count, completes synchronous `sdmmc_read_sectors`, then
  lends its DMA-complete buffer for an immediate copy under the short state
  critical section. The pointer is never retained or freed by Rust. No lock is
  held across SDMMC or task notification.
- The diagnostic client task runs on core1, priority3,16384-byte stack. A static
  RawWaker owns no allocation/refcount and only sends a FreeRTOS notification.
  Readiness and waker registration share the queue lock; completion extracts
  the waker and notifies after unlocking. Notifications persist across the
  poll-to-wait window, with a one-second timeout repoll. The diagnostic runner
  sleeps only its own dedicated task; it is not a product executor bridge.

The worker reads exactly one sector per request into its C bounce allocation;
it independently rejects count/range errors. Rust repeats raw sectors0..7,
computes FNV1a and compares later passes with the first observed value. This
checks stable raw block delivery, not known-file content, partition parsing,
filesystem correctness, maximum throughput or audio deadlines. No FatFs or
write trait is used. Whole-card identity/replacement acceptance remains open.

On IO failure, Rust invalidates the lease before returning the active request
as an error. Idle status failure also invalidates/wakes queued work before
teardown. Active work prevents teardown until the worker returns it. If an ABI
invariant prevents completion, hardware/bounce memory are held rather than
freed beneath a request. Both task TCBs stay allocated/suspended after stop so
static notifications cannot target deleted tasks. Reinsertion requires an app
restart. Physical removal and exact-image restart/cold evidence must be recorded
independently; host cancellation tests do not establish physical DMA behavior.

## Boundaries and checks

The C ABI exports use scalars and explicit pointer/length only. Rust rejects
null/misaligned output pointers, overlapping scalar outputs, null/misaligned
success buffers and any success length other than512. Valid pointer memory and
disjoint C/Rust allocations remain explicit unsafe caller obligations, not
something a pointer-value check can establish. Double/no-active completion is
rejected without taking another buffer. IDF owns abort/reset policy.

The [concurrent host FFI test](../../firmware-idf/legacy-jc1060/rust-core/src/idf_media_bridge/tests.rs)
uses a real host owner thread, synchronized port shims and retained condition
notifications. It performs two complete raw-read passes, malformed completion
attempts, a double-completion rejection, then invalidates during the next
request and injects an IO error. The client wakes/errors and teardown remains
blocked until completion. This is a test of the concrete Rust export/state/
waker logic; the C shim is replaced on host, so FreeRTOS task/mux primitives
and SD hardware remain separate runtime gates.

Twenty Rust tests (19 protocol +1 concrete FFI),21 shared-media tests and host/
target Clippy passed. The new CMake negative test rejects simultaneous FatFs,
display, memory/PSRAM and pure-RAM ownership. The fresh builder selects the
matching Rust feature/config together, verifies the legacy100..199 header and
single-float ELF, and rejects linked SD write/erase/FatFs APIs. It also requires
the actual Rust bridge symbol and C owner object in the final image/map.
Build, programming and runtime outcomes are added only after execution below.

## Exact-image results

The reviewed fresh package is `s06-reviewed-idf-rust-bridge`:219328-byte app,
SHA-256 `5a58ab6db4f5ca1f781d9d46b18614be2e9d9d45f3177c934bcd97696b5e67d1`.
All manifest sources/artifacts match. The initial development build also linked,
then the planner's accepted-variant list was extended and a new full package
was built so its source snapshot includes the final planner. No build directory
or previously accepted artifact was overwritten. A passive pre-upgrade capture
still observed three successful fixture passes on the old file diagnostic.

The [recovery plan](../boards/readiness/jc1060-s06-rust-bridge-write-recovery-plan-20261030.json)
replays all six prior plans, verifies four independent/secondary backup files
and preserves predecessor bytes beyond each new image end. Live chip/MAC,
security and RDID matched; every predecessor sector hash matched before writing.
Writes were limited to0x2000/24576,0x8000/4096 and0x10000/221184 bytes, followed
by matching readback hashes:
[programming result](../boards/evidence/jc1060-20261030-com3/s06-rust-bridge-flash-result.json).
No eFuse/full-chip erase or SD content write occurred.

A normal USB software reset from the held ROM/stub booted the image. In35s,
35 complete Rust block passes each matched FNV1a339d5f03, independently known
from the earlier card-sector diagnostic. Heartbeats continued without client
error, quiesce/fault-held, panic or HP watchdog:
[first boot](../boards/evidence/jc1060-20261030-com3/s06-rust-bridge-first-boot-result.json).
Minimum observed client stack remaining was10656 bytes; owner telemetry showed
5848 bytes and0 denied memory-write opcodes. This is bounded smoke, not long
soak, cache stress, timing qualification or maximum stack acceptance.

Three subsequent app restarts each reinitialized card/Rust transport and
completed15 post-init hash passes, with15–16 heartbeats and no observed error:
[restart results](../boards/evidence/jc1060-20261030-com3/s06-rust-bridge-app-restarts-2-result.json).
An earlier capture harness incorrectly included one successful pre-reset pass36
when checking that the new boot starts at pass1. The raw log/result are preserved;
an independent [capture review](../boards/evidence/jc1060-20261030-com3/s06-rust-bridge-restart-capture-review.json)
finds15 consecutive post-init passes. The harness was corrected to scope records
after the new init marker and the three-run campaign repeated. Firmware did not
change for that correction. Do not count the harness assertion as a device FAIL
or silently rewrite the earlier capture.

The [transport evidence index](../boards/readiness/jc1060-s06-idf-media-transport-evidence-20261030.json)
records the frozen accepted sources/build outputs, offline commands/logs,
programming/boot/restart evidence and independently verified secondary archive.
Twenty Rust tests,21 shared tests,18 Python methods, fmt/Clippy and the final
unit/docs checks passed locally. Remote CI and commit/push were not performed.
Old C/FatFs/display evidence is not transferred to this image. Subsequent
physical lifecycle outcomes follow below; the original archive remains a
snapshot before those operations, with a superseding archive added afterward.

## Physical removal

The owner confirmed removing the SD card while leaving USB power connected.
The [raw removal capture](../boards/evidence/jc1060-20261030-com3/s06-rust-bridge-removal-result.json)
observes `QUIESCED error=ESP_ERR_TIMEOUT reads=1176 denied_commands=0`, then
client pass148 fails at sector0 and the client stops with result-2. Six later
heartbeats continue, with no success after quiesce, fault-held, panic or HP
watchdog. The [operator/capture review](../boards/evidence/jc1060-20261030-com3/s06-rust-bridge-removal-review.json)
records owner confirmation separately from the unchanged capture-time field
`AWAITING_USER_REPLY`.

1176 calls equal147 completed eight-sector passes. No driver read from pass148
was submitted: an idle status query detected removal and invalidated the lease
before the client's next read. This is physical idle-removal acceptance for
this diagnostic; it is not a measurement of removal during active DMA. The
concurrent host FFI/protocol tests cover their separately scoped in-flight
case. No flash/reset or SD write was performed for this removal observation.

## Same-card reinsertion

The owner confirmed reinserting the same card. Three explicit app restarts each
initialized the card and concrete Rust transport, then completed15 consecutive
post-init passes with hash339d5f03 and15 heartbeats, without client error,
quiesce/fault-held, panic or HP watchdog:
[raw results](../boards/evidence/jc1060-20261030-com3/s06-rust-bridge-reinsert-result.json),
[independent reparse and operator record](../boards/evidence/jc1060-20261030-com3/s06-rust-bridge-reinsert-review.json).
All45 complete records and per-log SHA-256 were checked independently. This
is one physical reinsertion and three app startups, not three card replacements
or an automatic hot-remount implementation. No firmware/SD content changed.
The exact image's cold power-cycle capture is the next physical gate.

## Owner-confirmed cold return and final handoff

The owner left the card inserted, disconnected the sole USB power cable for
5–10 seconds and confirmed reconnecting without BOOT/RESET. The passive
monitor sent no reset or console command. USB disconnected/reopened and uptime
dropped from92001541us to1000838us. Fifteen complete post-cycle raw passes2..16
matched339d5f03, with client stack remaining10752 bytes and continued heartbeats;
no quiesce, fault-held, panic or HP watchdog appeared:
[raw capture](../boards/evidence/jc1060-20261030-com3/s06-rust-bridge-cold-result.json),
[independent reparse and operator record](../boards/evidence/jc1060-20261030-com3/s06-rust-bridge-cold-review.json).

The initial ROM/application init/reset-reason prefix and pass1 were missed
during native USB re-enumeration. Accordingly, the raw capture's post-init
counter is0 while its uptime-drop fallback detected15 later matching passes.
The independent review counts those records after the first new heartbeat.
This is operator-confirmed cold return with observed low-uptime runtime;
direct cold init/reset-reason capture is NOT_CAPTURED. Neither the original
capture result nor historical raw logs were rewritten to manufacture it.

The superseding lifecycle archive in the transport evidence index adds the
removal/reinsertion/cold captures and operator reviews, retaining exact accepted
firmware/source snapshots and the original pre-lifecycle archive. All ZIP CRCs,
member hashes and secondary-copy SHA-256 are verified. Current unit, status,
README and handoff point to this raw-block image. No new flash operation or
firmware/source change was made during these physical lifecycle steps; native
minimum302 and APTA remain unchanged. The device stays on the bounded read-only
raw-block diagnostic, not the previous C/FatFs file image or a product build.

Next: add an opt-in shared Rust partition/exFAT fixture profile using this
`OwnedBlockDevice` transport and one filesystem owner. Keep the raw profile as
a recovery baseline. Before a new flash, replay this plan's six predecessors
and this plan itself as the immediate predecessor. Then verify exact known
UTF-8 fixture length/content/SHA/EOF/seek and full-filesystem cancellation on
the new image; this raw-block campaign does not qualify those future gates.
Active-DMA physical removal, replacement cards, long soak, full filesystem
future cancellation and product load commit/executor integration remain open.

Next after bounded raw-block qualification: connect the existing shared Rust
partition/exFAT reader and repeat full fixture SHA/EOF/seek/removal tests.
Cancellation of the full filesystem future, generation checks at product
load commit, orphan drains and shared executor integration remain open. No
display/audio/USB-host/C6 work or native minimum302 change belongs to this gate.
