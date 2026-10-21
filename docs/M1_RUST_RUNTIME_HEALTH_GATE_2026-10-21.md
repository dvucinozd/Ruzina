# M1 runtime publication freshness and owner observations

Follow-up: [composed runtime owner integration and recovery](M1_RUST_OWNER_HEALTH_GATE_2026-10-21.md).
The available-adapter scope below records this earlier source revision.

Date: 2026-10-21. Branch: `ogranak/runtime-health-freshness`, based on
`4252d6d48eaaec33eb5b0f03403591cb579387d5` (`ogranak/service-health-ui`).
This is a dependent software package. The earlier stack and APTA are not merged.

## Publication contract

`pajoniiir-service::publication::Slot<T>` retains one Copy publication without
allocation or locks. The coordinator binds a nonzero, strictly increasing owner
generation. Rebinding invalidates the retained sample; unbinding also clears it
but preserves the generation watermark. A publisher cannot select its own new
generation by sending a sample. Sequence starts at one and increases strictly.
Neither sequence nor generation may wrap; exhaustion fails closed.

Samples carry generation, sequence, **source sample time** and Idle/Active
intent. Sample and observation time must use the same process/boot monotonic
millisecond domain. The serialized coordinator is responsible for that clock
and for binding a new generation on owner reset/replacement. This module does
not provide task synchronization, a watchdog, wall-clock conversion, or a
persistent identity across boots.

Wrong-generation, zero/duplicate/out-of-order sequence, future-time and
regressing-sample-time publications are rejected without changing the retained
value or deadline. Delayed samples can be accepted for inspection but remain
stale according to their original sample time. Observations explicitly report
Unbound, Awaiting, Fresh, Stale or ClockReversed. Exactly `max_age_ms` is still
fresh; greater age is stale. Idle has the same heartbeat deadline as Active.
Last values remain inspectable when stale and must not be used as current health.

## Integrated and available adapters

The desktop session publishes its actual media/analysis state after control,
load, reconnect, detach and transport ticks. Detach/reconnect invalidate the
old publication generation, while repeated reconnect does not create another
generation. Its heartbeat budget is 1000 ms; normal UI ticks are 50 ms.
Diagnostic/UI reads never publish or renew a sample. The Health page shows
publication state, owner activity, generation, sequence and age. Missing/stale
runtime observations take precedence over old media/analysis readiness.

The simulator and observer share a normal event loop. Headless replay advances
the observer separately to test a stopped publisher; this does **not** demonstrate
detection of a frozen event loop or a physical task watchdog. A resumed owner
publishes current Idle state without requiring playback or erasing event history.

The opt-in `owner-adapters` feature captures existing real owner counters from
`EventScheduler`, `FixedRateRenderGraph` and `SinkLane`. Sink observations also
carry its opaque token, queue depth and output frame clock. Integrators must
rebind on owner reset and sink route/PCM token replacement. Tests exercise real
queue overflow, render source starvation, muted/idle/active sink consumption,
PCM reset identity and rejection of old-generation publications. These are raw
lifetime counters, not automatic current-fault classifications or counter rates.

These three adapters are **available APIs with host owner tests**, not wired
Guition tasks or live simulator audio/USB telemetry. The desktop harness has no
PCM sink or controller scheduler; Health continues to say NO DEVICE ADAPTER.
The adapters are enabled by desktop dev-dependencies for host checks and do not
add audio dependencies to the embedded boot diagnostic build.

## Reports and compatibility

Shared reports are now `M1_SERVICE_DIAGNOSTIC_V2`, adding `runtime_publication`
with observation metadata. Consumers should dispatch on this version header.
The legacy `M1_BOOT_DIAGNOSTIC_V1` remains unchanged. Direct synchronous snapshots
have `runtime_publication=None`; this is not a heartbeat. The opt-in device task
continues to repeat one immutable boot snapshot, whose health remains BootOnly.
Both boot report sections still fit the checked 2048-byte device buffer.
All reports continue to end in `readiness=NOT_QUALIFIED`.

## Verification

Local checks: 577 host tests, workspace formatting/clippy, desktop build,
29 P4 check/link configurations and 20 board-selection rejection guards.
Headless replay covers both board geometries with 38 reviewed pixel goldens,
including stale Idle and recovered Idle reports. All 24 earlier non-Health
scene bodies remain unchanged. Logs, screenshots, reports and the source hash
manifest are retained in [evidence](evidence/m1-runtime-health-2026-10-21/README.md).
External dependency records and primary-checkout KiCad changes are preserved.
Hosted CI status is recorded separately from local checks.

## Remaining development gates

Next software step: connect publications to serialized product/controller/media
owner loops with bounded handoff and explicit restart/recovery ownership, then
exercise missed publications, reconnect and backpressure in the composed host
harness. Per-owner freshness budgets and counter-delta fault policies need that
integration; lifetime totals alone must not declare an active fault.

Exact-unit boot/identity, Guition display/touch, memory/resource/timing evidence,
audio drivers, output switching and listening acceptance await physical boards.
MAIN remains ready external DAC -> compatible controller -> mute. Headphones,
CUE and PFL remain controller-only. No DAC choice or pin assignment is implied.
