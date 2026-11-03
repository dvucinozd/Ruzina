# Normal engine device result: two-deck rejection, s07 restored

Date: 2026-11-02, JC1060 NewPanel rev103, COM3, FLX4, USB-only board
supply, RCA MASTER speakers and controller headphones. The owner reconfirmed
this topology and visible library. Native minimum stays **302**; this is the
separate pinned ESP-IDF 6.1/shared Rust path. No other board inherits acceptance.

Status: **NORMAL_TWO_DECK_REJECTED_S07_RESTORED_AND_OWNER_CONFIRMED**.
The registered stopping condition was reached. Do not promote this normal
image, merge on this evidence, or continue N03-N08 before fixing/requalifying
the source-supply/USB problem. APTA, DAC, networking and native HAL remain out
of scope. The accepted audio gain policy was unchanged.

## Exact source, tools and writes

Firmware source `886127e0714c15cb096cc3b86798fbacba1773f7`, clean normal
deck-engine app `0ba716a48d615681da7d58d3dd85976cb14cd886a30c9f785ef3686818b0f011`,
ELF `02b053052ba10e7d7d889028cd56644d128b3b4aa2057873a0e3b83cdb042df2`.
Both profile and pack-IRAM are false. Static load/fill limits were unchanged.
The [execution checkpoint](2026-11-02-normal-engine-execution.md) retains
source, CI, review/fix history, package and 38/39-plan registration. Final
tool source `08a537b` passed 21/21 CI, CodeRabbit **0 issues**, 13 normal tests
on Windows/WSL and 13 shared-guard WSL tests. Publication `3e078cc` and matching
remote SHA preceded device access. Binding SHA-256:
`a35aa0c48ae110668af466ffa4fd72fc09f847bf882e8cbad707a21cfff27b47`.

Both ROM entries used the already owner-authorized native USB software method.
Fresh rev103/MAC/security/RDID matched. Each guarded write first read and
matched all 16 MiB, then checked written spans and independent full readback:

| Executed transition | Existing predecessors | Full state after write |
|---|---:|---|
| Normal candidate | 38 | `155ab6b5db69ae73f3a4498e97c658fc50031d0a69c60275d59fc80a1692b5b1` |
| Exact accepted s07 return | 39 | `7875f650ba7404475e9c26561990851b04c11409b3f309401f6e77a51ddf38a6` |

Current executed history is **40**. Future registrations must include both
new executed plans, preserve longer-image sector tails and start from the
current full hash above. The s07 app remains
`95d45203ec245a9f82455f16f15dd95ed6ad2bb8eaaca9cf85f0816ccd5dbfdd`,
ELF `005f8d1e33aa5f2a8d05af691ce768461f559c917b8804920a912178f5b78ddf`.
Its changed full hash reflects the preserved candidate tails, not a new s07 app.

## Bounded observations

All raw captures remain immutable. Expected-ELF cold identity anchors the
following windows. Connected duration, completion and interruption fields are
recorded; no capture had a disconnect, unexpected reboot or fatal marker.

| Case | Observation | Result |
|---|---|---|
| N01 cold | 45.060 s after expected ELF, 7000 callbacks; max **2397 us**, zero >3000/>=6000 and USB/underrun errors; owner `vraća` | Registered cold admission PASS |
| N01 separate idle | 60.036 connected s; callbacks 11000->20000, 9000 new; same max/crossings; resource gates pass | Registered idle admission PASS |
| N02 deck 1 alone | Owner `čujem. disturbed mp3`; source 44.1 kHz; MAIN and headphone MASTER requested; no USB/underrun errors | Listening confirmed for this case; max **3214 us**, 3 >3000 crossings: headroom target FAIL |
| N02 deck 2 alone | Owner `čujem ben folds five mp3`; source 44.1 kHz; no USB/underrun errors | Listening confirmed for this case; same max, 2 additional >3000 crossings: headroom target FAIL |
| N02 both/PFL attempt | 72.911 connected s; max **4982 us**, no >=6000/fill failures; cumulative >3000 grows 5->172; underruns **4765/5003**, USB packet errors **1** | **REJECTED**, both source supply minima reach 0; stop and return |
| N03-N08 | No extension after rejection | **NOT_RUN**, including transport races, codec/pitch stress, reconnect, injection, 30-minute soak and later cold boots |

The <=3000 us value remains the headroom target, separately from the registered
mandatory stop on >=6000 us, resource failure or unexpected steady error.
Single-deck target failures were preserved, never relabelled as full admission
PASS. Continuing the next bounded case did not waive that target. The dual
case hit the independent underrun/USB stop even though fill remained <6000 us.
PFL1/PFL2 isolation was interrupted and has **no owner acceptance**. Do not
infer a routing pass from a requested control sequence or individual MASTER
monitor listening. Individual full track titles were not independently resolved;
the owner descriptions above are retained exactly.

Observed minima across these captures: USB stack **4820 B**, SD client
**28468 B** under playback (58252 idle), logger **2140 B** (2204 idle), internal
heap minimum **415720 B**, PSRAM minimum **28054272 B**. Smallest observed
largest blocks: internal **249856 B**, PSRAM **28311552 B**. These pass the
frozen development reserves. Largest-block values are asynchronous samples,
not continuous minima. No stack/heap exhaustion is demonstrated.

During the dual attempt producer supply fell from about 1.44 s per source
to zero; both rings were almost empty while both deck flags showed playing.
The cumulative producer maximum gap was 270015 us, already observed in the
single-deck/load interval. It is not a dual-only decode time and does not locate
the bottleneck. Fill timing alone does not establish USB service deadlines or
explain the packet error. CPU, decoder, PSRAM/cache, SD service and task
contention remain hypotheses, not confirmed root causes.

## Recovery and evidence

After the rejection the registered s07 return passed all span/full checks.
The owner performed physical power removal and confirmed `vraća`; the passive
capture contains one reset_reason=1 and the exact s07 ELF/UAC stream. The owner
then confirmed `sve ok` for known deck 1 playback, clean both-ear sound and
Pause/transport CUE. Captured playback-frame progress is **14.052 seconds**,
with zero USB iso/packet errors and zero underruns. This is a restore check,
not a claimed 20-second measurement or a two-deck acceptance. All tools closed.

The [device summary](evidence/2026-11-02-normal-engine/device-summary.json),
[current progress](evidence/2026-11-02-normal-engine/device-progress.json),
[archive index](evidence/2026-11-02-normal-engine/device-archive.json), owner
replies and [raw dual rejection](../boards/evidence/jc1060-20261030-com3/normal-two-deck-rejection/run.log)
are retained. `completed-normal-device.zip` is **37668270 bytes**, SHA-256
`500b8398dc6ce4bd20a7a1eaf6f78d2940ed2fc0506e0a0e26d1e4a3e06bd6a1`.
ZIP CRC, every member and both copies pass. Primary:
`D:/BoardEvidence/JC1060-20261030-COM3/normal-engine-20261102/`;
secondary: `C:/Users/Daniel/Documents/BoardBackups/JC1060-20261030-COM3/normal-engine-20261102/`.

## Next ordered work

1. Keep this failed image/evidence immutable. Start offline from the current
   source and captured per-deck supply, codec/rate/position and USB counters.
   Check the two-producer path in `idf_media_bridge.rs`, `producer_schedule.rs`
   and `deck_play.rs`; preserve fairness, lease/epoch and load/seek receipts.
   Its 1 ms poll budget is soft and cannot preempt a decoder step or SD read.
2. Design bounded, non-overlapping producer measurements: per-deck poll wall
   time, decoder-step versus read/wait time, produced frames/work turns,
   maximum single blocking operation, supply/pitch and other owner work.
   Separate time spent preempted/waiting from CPU work where measurable;
   do not subtract incomparable asynchronous maxima. Also observe USB service
   gaps/packet outcome so the packet error is not automatically blamed on SD.
   Record cumulative baselines/deltas and dropped observations. No formatting,
   heap walks or blocking instrumentation in refill.
3. Host-test the measurement/fairness boundaries and error paths with real
   producer fixtures and controlled slow reads. Run the existing actual C,
   Rust/Clippy, stack/static-RAM and offline package checks. Review and publish
   exact source/CI before any new target image. Do not fix by enlarging rings,
   tasks, limits or hiding underruns without measured justification.
4. Register a new diagnostic candidate and exact s07 return using **40/41**
   predecessors from full `7875f650...`; dual-copy archive, pinned tool review
   and publication precede COM3. Measure one deck then the same two MP3 sources
   under bounded load, stopping on the unchanged gates and restoring s07.
5. Choose one evidence-based fix only after separating decode throughput,
   blocking SD, task contention and USB service. Requalify the exact normal
   image, including <=3000 us headroom, then resume N02 PFL and N03-N08 in order.
   A profiled build's result cannot close normal two-deck or soak acceptance.
