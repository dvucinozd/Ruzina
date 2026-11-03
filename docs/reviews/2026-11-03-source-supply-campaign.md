# Frozen source-supply campaign

Date: 2026-11-03. JC1060 NewPanel rev103, full-speed USB-C/COM3, FLX4,
USB-only board power, RCA MASTER and controller headphones. Native minimum
stays 302; use the pinned ESP-IDF 6.1 path only. Scope is the reviewed
producer-profile image from source `e9ce242`, app
`173933e100e2de34257beb89013556b0937e8b0e94bd36bf797eb755d505d1cd`,
ELF `54dbdc424791bfc3f3516dd194487bd0a09e009bf7215e380c7a57b10d58af11`.
Producer profile=true; UAC substage/IRQ profile=false; pack-IRAM=false.

Read the [measurement definitions](2026-11-03-source-supply-profile.md) and
[prior normal rejection](2026-11-02-normal-engine-device.md). This image
measures wall time including waits/preemption, not CPU time or hardware IRQ
latency. It cannot qualify normal two-deck playback, PFL, transport or soak.
Keep gain, task stacks, affinities/priorities, rings, timing limits and media
ownership unchanged. No DAC/APTA/network/native HAL work or automatic merge.

## Immutable registration and recovery

Require both final source review and complete 22-job CI, exact artifact/source
checks and dual-copy build archive. Candidate and exact s07 return must be
registered against **40/41** predecessors. Starting independent whole flash:
`7875f650ba7404475e9c26561990851b04c11409b3f309401f6e77a51ddf38a6`.
Replay every ordered prior plan, verify four distinct factory copies and all
restore/staged/sector tails. Before each write verify fresh rev103/MAC/security/
RDID and all 16 MiB predecessor bytes. Read back every written span plus the
independent whole flash. No automatic eFuse, force, erase-all or programmer
reset. Owner-authorized native USB software ROM entry remains available;
physical cold power removal is separate evidence.

Publish this byte-frozen runbook/registration and exact reviewed execution and
analysis tools before COM3. Previous normal/profile bindings do not authorize
this campaign. Default commands are offline. Do not edit this runbook under
the registration; append actual results in a separate record.

## Ordered bounded cases

| Case | Capture and operator actions | Required observations |
|---|---|---|
| S01 cold | Monitor first, owner removes USB power 5-10 seconds and reconnects without BOOT/RESET. No load/control. 45 uninterrupted seconds after exact ELF, maximum 600 seconds waiting. | One cold reset_reason=1, UAC stream started, >=7000 callbacks, both stores prepared, all normal counters/resources and complete producer/owner/USB observations |
| S02 idle | Separate 60 connected seconds, no controls/load | >=9000 new callbacks; no unexpected boot/generation/error; advancing producer/owner snapshots |
| S03 deck1 | Dedicated 30-180-second capture; owner loads the known Disturbed MP3 on deck 1, deck 2 stays unloaded/paused; play about 20 seconds then pause | Load receipt/index/codec/rate and owner title, >=2 same-state fresh snapshots per deck, current source frames/supply/pitch and service partitions; no errors |
| S04 deck2 | New capture, deck 1 paused; known Ben Folds Five MP3 on deck 2 about 20 seconds then pause | Same requirements, distinct case and listening confirmation |
| S05 both | New capture, both known sources loaded, crossfader centered; play both without extra browse/PFL/transport actions. Maximum 60 connected seconds total including setup; stop at first failure. | Stable same-state intervals if available; producer frame deltas/supply and partitions, USB counters/gaps. Insufficient interval before failure is NOT_CAPTURED, not a reason to repeat or extend |

The passive monitor is bounded to 180 seconds for S03/S04 and 60 seconds total
for S05 including setup; restore promptly at the end of that window. A separate
explicit stop request ends the window after capture
starts. Record actual durations. Windows shorter than 30 seconds are incomplete
evidence even when a failure correctly triggers recovery. Preserve short raw
failure data; never continue running merely to meet the duration requirement.

Do not request PFL isolation, load/seek races, codec/pitch endpoints, media or
USB reconnect, error injection, soak or additional cold campaigns. This
diagnostic ends with the first bounded failure or after S05, then returns s07.

## Admission, integrity and stopping

- All-fill every phase <6000 us; zero unexpected USB iso/packet/submit/fill
  errors, steady-playing underruns, source failure, reset or unresolved current
  transport. Any such error stops extension and triggers registered s07 return.
  <=3000 us remains a separately reported headroom target; a miss is never PASS
  for normal headroom, but is not the >=6000 us mandatory stop by itself.
- Runtime reserves unchanged: USB stack >=2048 B, SD client >=8192 B,
  logger >=1024 B, internal heap minimum >=65536 B, PSRAM minimum >=8388608 B;
  smallest observed largest blocks >=8192 B internal/>=262144 B PSRAM. Missing
  resource observations or reserve failure stops extension. Largest-block
  minima are sampled, not continuous.
- Analyze only exact-image windows anchored to this candidate's verified cold
  boot; generation must stay 1 without reconnect. Complete connected capture
  is mandatory for positive quantitative evidence; no boot/connection gap
  time may count toward measurement. Identity and owner observations remain
  distinct from an analyzer status.
- Each producer header must have all six same-deck/same-sequence stage rows:
  poll/decode/step/read/push/decode_other. Require schema 1, even advancing
  sequence, monotonic timestamps, >=2 coherent samples, max snapshot spacing
  10 seconds, no unavailable rows, nonzero invalid/saturated fields or dropped
  logger reports. Owner and all three USB service stages also need complete,
  advancing observations. Ignore only a leading partial group before the first
  header; never ignore an incomplete later group. Snapshot timestamps are
  asynchronous; do not claim a simultaneous cross-core state.
- Decoder cumulative totals must partition exactly into step+read+push+other;
  poll includes decode and is not added to it. Owner completed wall must equal
  polls+remainder. Check monotonic cumulative totals/calls and baseline deltas;
  never subtract different maximum samples. USB handle/tail are loop partitions,
  callback-entry spacing overlaps them. Logical read bytes are not physical SD
  bytes. No inference of CPU/IRQ time from these wall counters.
- For throughput use only adjacent same epoch/index/rate/playing/pitch samples
  in the requested stable state. Report produced source frames versus source
  rate and pitch over source snapshot time, plus observed supply. Load/prime,
  paused and mixed-state intervals cannot qualify steady production. No stable
  interval or stale/incomplete/invalid telemetry means NOT_CAPTURED/INVALID;
  stop extension, preserve data and restore, rather than infer a cause.

## Exit and next decision

Always execute the registered exact s07 return with full readback, physical
cold boot and owner listening/Pause/transport-CUE check before ending the
experimental campaign. Accepted app
`95d45203ec245a9f82455f16f15dd95ed6ad2bb8eaaca9cf85f0816ccd5dbfdd`,
ELF `005f8d1e33aa5f2a8d05af691ce768461f559c917b8804920a912178f5b78ddf`.
Only verified writes enter executed ancestry; preserve longer candidate tails.
Close all tools, archive raw captures/plans/results/owner replies on both
storage locations with CRC/member hashes, publish exact current state and
clean matching remote SHA. Promotion is a later decision.

Use valid evidence to separate slow logical reads, decoder steps, source
service gaps and USB service observations. If this profile is rejected before
steady two-deck measurement, state that limitation. Choose one measured fix
only after the cause is narrowed; never claim the old normal failure fixed
from an instrumented build or a single listening reply.
