# M1 service/health observations and diagnostics UI

Date: 2026-10-21

Base: `eac8481e2a91f7eaf8971c36bfa8188ba42a3f3e`, open mixer/FX PR #8 above UI
PR #7, Rekordbox PR #6 and audio PR #5. Work is isolated on
`ogranak/service-health-ui`. Dependencies are not merged into main; APTA remains
unmerged. No firmware was flashed or published.

## Shared model and real observations

The new `pajoniiir-service` crate is no_std, allocation-free and independent of
HAL, Slint and clock drivers. A small copied `Snapshot` contains candidate/build
identity, supplied sample time, service counters, current owner-provided media/
analysis state and a 16-entry event tail. Counters/time/sequence saturate; an
overwrite counter makes bounded history loss explicit. Snapshot size is checked
below 2048 bytes. The recorder observes outcomes; it cannot authorize a load,
mutate deck state or recover media itself.

The host session records actual demo connect/remove transitions, load attempts,
success/fallback/rejection, blocked loads, rejected playback and track end.
Delivered semantic events include releases and owner-generated semantic clears;
they are not a count of accepted user changes. The elapsed input clock advances
on every supplied host tick, including while paused. It is not OS uptime, audio
timing, latency, CPU load or a performance measurement. Duplicate reconnect calls
do not create new connect observations. A failed replacement preserves the old
deck/analysis state. Missing/rejected analysis follows the actual importer result
and existing BPM fallback behavior.

Current health is derived from a fresh owner snapshot, not the last historical
warning: reconnect/unload removes the active fault while old rejection counts
and events remain. Health distinguishes boot-only scope, host demo media ready,
analysis fallback, absent media and unit-identity mismatch. Device boot scope
cannot become host-demo-ready, and matching identity never implies product
acceptance. Every text report ends `readiness=NOT_QUALIFIED`.

Candidate board values are separate from observations. Host silicon, flash,
PSRAM and reset readings stay absent. Missing image labels are `NOT_RECORDED`;
labels are caller-supplied metadata, not authenticated hashes of an installed
image. Invalid/oversize report labels are explicit. Identity checks distinguish
unmeasured, partial, documented-value match and mismatch. Documented 16/32 MiB
capacities or the v3.2 policy never substitute for physical measurements.

## UI, reports and boot integration

Health is the seventh navigation destination. Summary shows the profile,
environment, supplied image label, measured-vs-documented identity and actual
session counters. Events shows a scrollable chronological tail; recovery text
and Open Library lead to the existing explicit reconnect/load workflow. CPU,
audio, device adapter and memory observations are not replaced with fake meters.
The selected profile is now bound to the session as well as window geometry.

`--diagnostic-report OUTPUT [jc1060|jc8012]` writes a **fresh host session** report
without opening Slint. It does not attach to a running UI or query a board. The
headless scene harness separately exports reports from its current actual state.
`M1_SERVICE_DIAGNOSTIC_V1` is a versioned text observation record. Formatting is
fallible; callers must discard the entire buffer on error.

The existing opt-in Guition `board-diagnostics` task preserves its legacy
`M1_BOOT_DIAGNOSTIC_V1` report and appends the shared service record. It supplies
the HAL eFuse revision, raw HAL reset code and sample time; flash/PSRAM readings
remain `None`. The output buffer is now 2048 bytes, with a combined worst-label
boot-report fit test. The task still attempts at most 64 nonblocking bytes per
scheduler turn. It freezes one boot sample and repeats those bytes: repeated
emission is **not a heartbeat** and does not refresh its observation timestamp.
Default selectors acquire no new peripheral or product owner.

## Validation

| Local gate | Result |
|---|---|
| Full workspace tests | PASS, 571 tests (+5 service and +2 session cases) |
| Format / all-target warnings-denied Clippy / desktop compile | PASS |
| Compiled Slint callbacks, pointer/keyboard and event scrolling | PASS at 1024×600 and 800×1280 |
| Reviewed RGB pixel goldens, independent replay | PASS, 34 snapshots |
| P4 workflow / partition validator | PASS, 29 check/link configurations |
| Invalid-feature guards | PASS, 20 expected rejections |
| CLI fresh-session report | PASS, JC8012 profile and unknown observations preserved |

Tests cover policy mismatch/partial/missing observations, recovery without
erasing history, ring eviction/order/saturation, unknown/invalid identity text,
report overflow, combined legacy/shared report size, actual load outcomes and
single EOF recording. UI replay exercises navigation, events/scrolling, removal,
reconnect and real malformed-ANLZ fallback. Ten new Health scenes supplement the
24 previous scenes; their bodies remain pixel-identical below expanded navigation.
Reports and reviewed PNGs are in [retained evidence](evidence/m1-service-health-2026-10-21/README.md).

Pixel evidence uses Linux x86_64, Slint 1.18.1 software renderer, Fluent, DejaVu
Sans and scale 1. The workflow retains PPMs and text reports. Hosted Actions
remain blocked before execution by account billing/spending eligibility; local
PASS is not hosted CI PASS. External records in both locks and all 18 pre-existing
primary-checkout KiCad file hashes remain unchanged.

## Open gates

This implements the bounded shared service model, actual host observations,
diagnostics screen and opt-in boot adapter. Full S11/M1R-P0-065 observability and
fault containment are not closed: product/audio/storage/controller owner stats,
publication freshness/liveness, per-subsystem recovery policy, real performance
views, unique physical unit/installed-image identity, Guition display/touch and
physical service/output/resource acceptance still need their own adapters and
evidence. No zero-loss, memory performance, listening or device acceptance is
inferred from these results.

The next bounded software package is generation/freshness-aware runtime health
publication and owner adapters, so a stale observation is explicitly distinct
from an idle subsystem. Exact-unit recovery/diagnostics remains the first board
arrival milestone; headphones remain controller-only.
