# Explicit USB0 APTA service gate — 2026-10-21

Base main: `e95cd07f2bbbe0b33410ea382fd9be6d5611dd38` (PR #18).
Branch: `ogranak/apta-usb0-service`.
The original `codex/native-apta-consumer` remains unmerged at
`fd7ebd592b469b27c9f45337dced2f9b2912966f`.
Pinned libapta remains `25d09de364e9507532c0508c838a8fecf0772d13`.

## Activation boundary

This third selective package wires the current portable service to real USB0
filesystem messages and today's Library-enabled product owner. Firmware feature
`apta-usb0-bringup` explicitly implies the existing render/Library/USB0/USB1/PSRAM
bring-up chain and optional adapter/libapta dependencies. It does not select a
board. `board-legacy-m1` is required by the existing legacy hardware guards.
Guition JC1060/JC8012 and no-board activation fail at compile time. Default
firmware, simulator and existing full-audio profiles exclude both dependencies.
There is no production-default, flash, release or physical acceptance change.

The original APTA firmware owner/lifetime implementation is not imported. The
new supervisor source-includes into host tests alongside the actual current
ProductRuntime and TransportOwnerState. Historical KiCad changes and the
previous local-only APTA audit are preserved. MAIN/headphone routing is unchanged;
headphones remain controller-only.

## Scheduling and publication

One persistent owner claims its exclusive endpoint once, after all fallible
startup allocations and the first mounted lease. Its Client, Requests, free banks
and scratch outlive jobs, cancellation and media replacement. There is one active
job, one pending owned delivery and one recycle slot. Deck One is considered
before Deck Two; jobs are serialized. No full catalog scan or unbounded backlog
is introduced: only current committed WAV loads without an existing analysis are
considered, once per identity. ANLZ remains preferred; unsupported paths and real
analysis failures do not hot-loop. A paused/cancelled job may retry when idle.

The supervisor requires both decks stopped with no pending playback, no product
Library or transport effects, and actual downstream transport idle. A reservation
counter covers command enqueue through execution (including awaits), rolls back
queue refusal, and releases on stale-command exit. Pending transport completions
also prevent publication. Successful enqueue alone is insufficient.

Playback/busy transport turns off new analysis I/O and enqueues Cancel. A completed
result already handed off is retained across temporary busy state, then freshly
checks lease, provider, track, catalog revision, load generation and authority.
Reload, detach, newer authority and existing ANLZ reject it permanently. It moves
into the existing owned pins without track reload, cue replacement, playback
resolution or source-epoch advance. Full recycle enqueue retains the exact bank
for retry; no owned result is silently replaced.

USB1 drives bounded supervisor turns and wakes on a service signal or 20 ms timer.
The surrounding USB device/MIDI wait future stays pinned through these analysis
wakes; notifications do not recreate an active read. Existing profile/effect
wakes retain their established semantics. The combined runtime-health profile
also keeps the periodic control-owner tick inside the supervised idle/device
wait, so an absent controller does not make a healthy owner appear stale. This is cooperative scheduling, not a
measured 20 ms execution or playback-latency guarantee.

## Concrete broker and recovery

Analysis request and completion queues each have depth 1, independent of the
foreground Library/Deck queue. The command queue has depth 2; output has depth 1.
The filesystem worker polls the foreground receive first. Analysis may starve
under foreground load by design. Each new analysis read is capped at 512 bytes;
Open/Read when paused return AnalysisPaused with the original owned handle/buffer.
Close is always allowed to drain. One already dispatched filesystem/USB operation
can still be in flight; byte bounds do not establish elapsed backend latency.

The same Client drains pending completions and closes before new work. A failed
recovery retains resources and exposes RecoveryBlocked. The supervisor sends at
most three RetryRecovery commands, each separated by 50 owner turns. This is a
bounded retry policy, without a strict one-second timing claim. Unexpected
completions remain quarantined; the supervisor never takes/discards them or
reissues an endpoint. Hung backends and quarantine need explicit investigation.
Reports are a bounded serial-tagged status snapshot, not an operator UI or a
historical event journal. Startup allocation/claim refusal keeps readiness false.

## Conservative profile and memory

Object size is limited to 256 MiB, duration to 600000 ms, scanner attempts to
1000000 and processing attempts to 2000000. Framing, source hashes, closes and
capacity all validate before publication. Format support is the existing WAV
scanner's support, not all WAV variants. Analysis is a two-pass source operation.
No full file or PCM object is retained.

Arrays are sized for 48000 * 600 source frames, FPC 65536, normalized queue 256,
4096 beats per bank and 440 native overview columns. Column rounding means the
array's frame ceiling is 28835840; duration and other limits also apply. Higher
sample rates do not grow storage. Conversion/capacity errors publish nothing.
The current preview bridge reduces full-source coverage to at most 400 peak-max
buckets and represents no detail-Hz layer. This coarse profile is intentional for
bring-up, not a final waveform-resolution or meter-accuracy promise.

| Payload | Bytes |
|---|---:|
| Onset bins + flux | 81920 |
| Normalized queue | 2048 |
| Native overview | 4400 |
| Two rendered waveform banks | 3520 |
| Two beat banks | 98304 |
| Byte + PCM scratch | 6144 |
| Working arrays + both banks | 196336 |
| Actual broker buffer | 512 |
| Actual array + broker payload | 196848 |
| Conservative ServiceStorage plan, using 4096-byte broker | 200432 |

Vec buffers allocate fallibly once before being leaked to the never-restarted
owner. Partial startup allocation failure drops prior allocations before endpoint
claim or any leak. This uses the existing PSRAM allocator; it does not prove DMA,
cache coherency, heap headroom or actual hardware allocation success.

One maximum owned copy is 4096 * 12 + 400 * 4 = 50752 bytes. Up to two accepted
copies and one pending copy must be considered separately. These numbers exclude
Vec/future/channel metadata, allocation overhead, existing ANLZ/audio/FX/catalog,
and stack. Linker sections and task-pool sizes in the retained ELF report are
static-build evidence; they are not runtime high-water measurements.

## Validation and open gates

Local checks:

- Default workspace: 667 tests including doctests; host Clippy with warnings denied.
- Optional adapter release: 68 tests plus 2 doctests. Includes 12 new supervisor,
  broker-policy and transport-accounting tests.
- Focused current-product/policy/service/supervisor suite: 24 tests, Clippy clean.
- Runtime owners and composed failures: 20 tests.
- Simulator compile/smoke and all 38 reviewed screenshot goldens unchanged.
- Pinned RV32 core and owned-worker checks pass.
- Full workflow's 36 ESP32-P4 check/link commands and partition validator pass,
  including the three new explicit-service profiles.
- All 27 invalid board/feature combinations fail with the required diagnostic.
- Dependency check: three default/product exclusions plus explicit opt-in presence.

New host regressions exercise ANLZ priority, two-deck serialization, pending
publication across queued start/playing/actual stop on the other deck, temporary
transport busy state, command/recycle backpressure, cancellation during a pending
read and recovery on the same client, load replacement, detach, startup refusal,
bounded recovery retries, quarantine policy, actual shared foreground-first
selection/read cap, disconnected control-health ticks, and the queue/executing
transport reservation guard. Host
ports substitute bounded in-memory queues and media observation. They do not
execute ESP32 USB drivers or demonstrate physical priority/latency.

Retained logs, source hashes, reviewed patch scope and preservation assertions
are under `docs/evidence/m1-apta-usb0-2026-10-21/manifest.json`. Hosted CI remains
separate; jobs that cannot start due account billing/spending are NOT a CI pass.

Still NOT RUN: boot/flash on the new boards, Guition BSP activation, USB0 physical
media/controller/rail compatibility, heap and stack high-water, measured audio
latency/underruns, paused in-flight I/O on hardware, repeated detach/reinsert,
recovery/quarantine operator procedure, corpus accuracy/performance, sanitizer
campaign, concurrent analysis/playback and long-duration listening/soak. Analysis
while either deck plays is deliberately disabled. A later board-specific gate
must qualify wiring, storage/audio timing and this memory budget before enabling
any default or claiming Guition support.
