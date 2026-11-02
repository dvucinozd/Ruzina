# Normal engine: bounded functional campaign

Date: 2026-11-02. Branch `ogranak/jc1060-normal-engine-campaign`, base
`098a8c5bc7ac4de01d1965dcc235ac1673794ebf`. Follows the
[completed resource-readiness idle campaign](2026-11-02-resource-readiness-device.md).
Status: SOURCE_PREPARATION / NOT_REGISTERED / NOT_FLASHED.

The owner confirms both FLX4 RCA MASTER speakers and controller headphones are
available. Keep the accepted volume policy/levels. External DAC, APTA, network,
native esp-hal and automatic merging remain out of scope. Native minimum is 302;
this unit is the pinned IDF 6.1 JC1060 NewPanel rev103 path only.

## Exact-image gates

The measured profile's 2008/1847/1757 us maxima do not qualify a different normal
binary. The old normal package is retained as build evidence. This candidate
adds the previously missing resource observations before functional testing:
USB-client stack watermark is captured in its client-loop report, outside the
audio callback, and copied through the existing single-snapshot queue. The
low-priority logger reads global current/minimum/free-largest internal/PSRAM
heap and its own separate stack watermark. No formatted I/O or heap walks are
added to refill; the watermark scan runs only on the existing report cadence.
No additional render clock or per-stage/IRQ profiler is enabled in normal mode.

The observation has nonzero cost, including shared-heap locks and stack scanning.
Do not infer dispatcher/queue coverage from fill_max alone or pure allocation
time from printed READY timestamps. Old profile and control evidence stays
immutable. Build a clean normal package with both profile and pack-IRAM false,
check all source/artifact hashes, static stack and partition budgets. Review and
publish source/CI before registration. Four factory copies and all **38** executed
predecessors must reconstruct the independent current full state
`1a45683632f9951f1121ad0866fd7b3050d146022bfc6c719be18346848ad4fe`.
Register candidate and exact s07 return, with 38/39 predecessors respectively,
and archive both packages/plans/payloads on primary and secondary storage.
No old 36/37-predecessor or profile-only execution binding may be reused.

The owner-authorized native USB software ROM method may be used for entry,
with fresh USB/chip/MAC/security/RDID and full 16 MiB predecessor equality.
The guarded writer itself uses no-reset and verifies written spans plus a
whole-flash readback. Cold measurements still require physical power removal.

## Ordered observations and stopping conditions

First freeze the exact source/package/plan/binding in the registration. Retain
raw captures, expected ELF, operator replies and separate case identities. Use
these provisional development gates without raising them after a failure:

- Every phase of Rust fill must remain <6000 us; <=3000 us is the headroom
  target. Report startup/recovery/callback phases separately. Any rejection
  ends extension into subsequent playback cases and triggers the registered
  s07 return. A USB pool reported STREAMING must keep making progress.
- Normal cases require zero USB iso/packet/submit/fill errors, steady-playing
  underruns, unexpected reset, source mismatch or unresolved current transport
  receipt. Count expected injected faults separately; do not waive errors.
- Log at least two resource reports per bounded steady case. Observed USB stack
  reserve >=2048 bytes, SD-client reserve >=8192, logger reserve >=1024; true
  internal heap minimum >=65536 and PSRAM minimum >=8388608 bytes. Smallest
  observed largest block >=8192 internal / >=262144 PSRAM. These are registered
  development reserves, not final production budgets or continuous largest-block
  minima. Missing telemetry leaves its gate NOT_CAPTURED, never PASS. No task
  stack/limit increase is part of this candidate.
- Capture source buffered-time and maximum producer gaps under load; compare
  measured gaps with actual source/pitch ring coverage. The 1 ms decode soft
  budget is not a bound on SD timeouts or filesystem open/read latency.

| Order | Bounded case | Evidence required before continuation |
|---|---|---|
| N01 / I01 | One physical cold boot, 45 seconds after expected normal ELF, then separate 60-second idle | At least 7000 cold callbacks and 9000 new steady callbacks; all-fill timing, no errors, library, normal flags and both prepared stores; resource reserves |
| N02 / I02 | Different known tracks; play deck 1 alone, deck 2 alone, then both; alternate PFL 1/2 and headphone MIX | Owner listens to RCA MASTER and headphones; each source/routes/levels correct; source identities and telemetry agree. About 20 seconds per stable selection, always captured before actions |
| N03 / I03 | Both decks separately: Play/Pause, CUE return/set/hold/release, paused jog, pitch/bend and supported local Hot Cue | Sound, controls, LEDs and positions agree; holding CUE resolves on release; no unexpected peer restart |
| N04 / I04-I05 | Repeated LOAD/Play timing and superseding LOAD; forward/back/repeated seek and EOF outcomes | Every admitted current operation resolves; no stale title/source; replacement/failure stops as documented; peer continuity |
| N05 / I06-I07 | A plays while B loads/seeks MP3/FLAC/WAV, invert decks; 44.1/48 kHz and pitch endpoints; browse details playing then paused | No steady underruns, bounded producer gaps versus supply, resource/timing reserves; latest details remain pending while playing and resolve after pause |
| N06 / I08-I10 | Separately identified SD loss/return and USB/controller disconnect/reconnect, including held controls | Revocation/silence and retired callbacks; current restart-required SD policy, fresh USB generation, no preview auto-resume. Needs new operator steps/captures per case |
| N07 / I11-I12 | Only supported target error/drain injection | If no safe target injector exists, retain NOT_RUN with host evidence separately. No fabricated target pass |
| N08 / I13 | After preceding supported cases pass: 30-minute exact-image two-deck playback with periodic load/seek, then two further cold boots | Raw whole interval, stable resources/source supply/timing, no stream loss; operator listening distinct from counters |

Do not combine an unrecorded control sequence with a different scenario or turn
one listening reply into acceptance of unrelated cases. If a case is not possible
with the available media/control mapping, label it NOT_RUN and state what is
missing. Partial acceptance cannot become full I02-I13 qualification.

## Exit

This is an experimental functional campaign. Always restore/readback/cold boot/
listen to the exact accepted s07 before ending it, even if bounded cases pass;
promotion is a later explicit decision. Update executed ancestry only after
verified writes, preserve longer-image sector tails, close all tools, and leave
an exact current full hash plus clean pushed handoff. At this preparation
checkpoint s07 remains installed and every normal-image case above is NOT_RUN.
