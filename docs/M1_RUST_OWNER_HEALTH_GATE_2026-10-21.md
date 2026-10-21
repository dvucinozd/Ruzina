# Composed runtime owner health and recovery

Date: 2026-10-21. Branch: `ogranak/runtime-owner-health`, based on
`f1786a96fcbee17a3f0a25c142deff9f17582010` (`ogranak/runtime-health-freshness`).
This follows the [publication foundation](M1_RUST_RUNTIME_HEALTH_GATE_2026-10-21.md).
It implements owner integration and composed fault/recovery tests. It does not
merge the preceding PR stack, activate Guition peripherals or merge APTA.

## Data ownership and bounded handoff

The shared no_std `handoff` module gives each owner two diagnostic queue slots
and one retained publication. `try_send` never waits: full queues drop telemetry
and increment a dedicated diagnostic counter, without holding up controls or
I/O. Each observer turn drains at most two samples per owner, independently.
All data is Copy, allocation-free, serialized by the caller. Queue fullness is
diagnostic loss, not an audio underrun or lost controller input.

Only a lifecycle coordinator restarts a channel and issues its producer token.
Restart invalidates the old publication, discards queued old observations and
resets the error-delta baseline. It preserves cumulative handoff history. Old
producers cannot rebind or fill the new queue; wrong-generation samples are
rejected before the capacity check. Source timestamps, sequence validation and
non-wrapping exhaustion follow the publication foundation. Invalid/replayed
samples never renew freshness or erase the last accepted error interval.

`ProductRuntimeStats` now uses the shared `ProductCounters` type. The actual
firmware product code captures its counters, four effect queue depths, deck
playback state and dispatch backpressure. Controller observations capture the
actual `ControllerSession` and scheduler counters, connection/transport failure
and current dispatch backpressure. Media observations retain the actual session
lease, handle, mounted status, worker phase and cumulative worker failures.

## Actual target wiring

The **OFF by default** `runtime-health` firmware feature attaches to the
existing explicitly selected **legacy** owner loops. It does not enable any USB
owner, transfer peripherals or assign Guition pins. It requires
`board-legacy-m1` and at least one explicitly enabled USB owner. Guition and
ownerless configurations fail at compile time.

- Product/controller samples originate in USB1 dispatch and readiness/idle
  waits. A terminal controller state transition and reconnect restart both
  diagnostic generations. An independent observer does not re-read those
  owners to fabricate a heartbeat.
- USB0 transport enumeration/detach immediately invalidates the media channel,
  including while its media worker is suspended in an old operation. A new
  token is transferred only for the matching lease, handle **and** Ready versus
  Detached event kind. Superseded worker samples cannot renew current health.
- The media worker publishes actual phase changes and idle heartbeats. Awaited
  mount/discovery/filesystem handoffs do not receive synthetic progress. A
  suspended operation can therefore become stale.
- The observer runs every 250 ms and drains at most six samples total per turn
  in short critical sections. It never executes controls or resets hardware.
  `runtime_health::snapshot()` recomputes current sample ages without publishing.

Software progress budgets are 2000 ms for product/controller and 5000 ms for
media. These are explicit software contracts, **not measured device latency**.
Heartbeat waits add a timer branch to existing cancellable event/read waits.
Timing, cancellation and queue-pressure behavior still need board qualification.
This is not a watchdog for a stalled executor or CPU, and freshness alone does
not establish useful progress of every queued operation.

## Current state versus historical totals

Each view first reports Unbound, Awaiting, Stale or ClockReversed when relevant.
Fresh samples then distinguish Idle, Active, Disconnected, Failed and current
Backpressured state. Product library failures and controller semantic/scheduler
loss counters produce `NewErrors` only when they increase relative to a prior
accepted sample in the same generation. Historical nonzero totals alone do not
latch a current fault. Expected coalescing/retries are retained separately.

`new_errors` is the sum of increases in the last nonempty accepted drain batch,
not a per-second rate or a permanent fault latch. A later accepted normal batch
clears that indication; an empty/rejected drain does not. An observed aggregate
fault-counter decrease in one generation reports CounterReset. The first sample
after restart establishes a new baseline; it does not reinterpret old totals as
new faults. This policy does not prove zero loss or a complete fault taxonomy.

## Evidence and acceptance

The composed host harness includes the **same default ProductRuntime source**
as the firmware and real `ControllerSession`/`MediaSession` implementations. It
exercises actual product effect-queue saturation and a held controller delivery
that succeeds exactly once after a slot is freed; independent publication loss;
full diagnostic queues while controls continue; controller reconnect with
historical errors; media replacement with old Ready/Detached observations;
error-delta recovery; invalid publication rejection; and bounded reports.

The harness does not emulate USB electrical behavior, Embassy scheduling or a
mounted target filesystem. The product's `library-deck-load` conditional variant
has P4 compile/link evidence, not an added host filesystem integration test.

Local verification: **586 host tests**, workspace fmt and warnings-denied host
clippy, desktop compile, **33 P4 check/link configurations**, **24 expected
configuration rejections**, and **38 unchanged reviewed UI pixel goldens**.
The added target matrix covers USB0 only, USB1/product only and full combined
render ownership with `runtime-health`. External dependency records and all 18
pre-existing primary-checkout KiCad file hashes are preserved.

The shared `M1_OWNER_HEALTH_V1` report is separate from the existing immutable
boot snapshot and session report. It remains `readiness=NOT_QUALIFIED`. The new
runtime target snapshot API is available to a future device diagnostic/UI
consumer; no new live serial endpoint or Guition runtime UI is claimed here.
The desktop UI continues to show its host session and no physical device data.
See [retained logs, scenario reports and source hashes](evidence/m1-owner-health-2026-10-21/README.md).
Hosted CI status remains separate from these local results.

## Next integration gate

Review and consolidate the dependent development PR stack in order, retaining
the tested source/evidence boundaries and resolving hosted CI account limits.
APTA remains separate. Before adding more features, review composed Library,
source, transport/render and persistent-write failure policies against the
existing owner channels. These are not closed by the diagnostic tests above.

On board arrival: preserve factory/recovery state, identify actual silicon and
memory, then qualify display/touch, storage, USB and physical audio independently.
MAIN remains ready external DAC -> compatible controller -> mute. Headphones,
CUE and PFL remain exclusively on controller UAC. No DAC or pin choice changes.
