# USB service budget and next implementation decision

Date: 2026-11-02. Audit base `7a444e32bc9cdb4cd9816ef7e579ca09c1bd3c22`.
Status: **OFFLINE_AUDIT_COMPLETE / DESIGN_ONLY / NOT_IMPLEMENTED**.
Follows the [completed placement campaign](2026-11-02-pack-placement-device.md)
and closes steps 1–3 of the [next-decision plan](2026-11-02-post-placement-next-step.md).
No COM3 access, new build, firmware change or new physical claim in this audit.
S07 remains the accepted installed image; future recovery ancestry is 36.

## 1. Actual USB service contract

Authority is vendored esp-usb 1.6.0 and pinned IDF `fff9895c82d744c7237be8847347bdd1b07c6643`.
The [source index](evidence/2026-11-02-usb-service/source-index.json) records exact
file hashes and audit observations; local primary source outranks a current
online library version for this decision.
The prior completed-campaign head `7a444e3` now has successful
[host #544](https://github.com/dvucinozd/Ruzina/actions/runs/41470926932) and
[legacy #67](https://github.com/dvucinozd/Ruzina/actions/runs/41470927017)
workflows. Those results belong to the prior head; no new firmware was built
or qualified by the design-only arithmetic checks here.

| Layer | Confirmed implementation | Implication |
|---|---|---|
| Application pool | `uac_driver.inc`: four URBs, six 1 ms packets each; fill then submit each | Nominal 24 ms initially queued, not a guaranteed per-call delay allowance |
| Application callback | `uac_iso_done`: clear busy, inspect packet statuses, render/pack six packets, submit again | Whole-fill timing excludes later submit/cache-sync and much dispatch work |
| Client dispatch | `usb_host.c::_handle_pending_ep`: dequeue all done URBs and invoke callbacks sequentially | A long refill delays other completed audio callbacks and MIDI/events |
| Client event loop | `usb_host_client_handle_events` drains pending endpoints, EP0 and then event messages | 50 ms app timeout is an idle wait limit; it is not 50 ms granted refill latency |
| HCD | `hcd_dwc.c`: two hardware buffers plus pending/done URB lists | Four application URBs do not mean four independent active hardware buffers |
| HCD completion IRQ | `_intr_hdlr_chan`: start prepared next buffer, parse old, fill another from pending | Pending URBs can keep hardware supplied while client callbacks are blocked |
| HCD isochronous schedule | `_buffer_fill` links to prior next-start index when available; otherwise reschedules from current frame plus margin | A rescheduled stream need not expose a simple application-pool deadline |
| Packet statuses | `_buffer_parse_isoc` maps NOT_EXECUTED to SKIPPED; app counts all non-COMPLETED packets | Existing packet counters cover skipped submitted descriptors, not every possible period with no queued data |
| Submission | `hcd_urb_enqueue` performs cache synchronization before enqueue/critical section | `fill_max_us` alone omits some service cost even on a correct image |

Let S=6000 us of audio per completed transfer. At callback dispatch, define
Q as remaining continuously scheduled audio duration after the completed URB,
not the number of application `busy` flags. A useful service requirement is:

`dispatch latency + refill + submit + scheduling margin < available coverage`.

Under an ideal freshly full four-URB pool, the first completion leaves at most
18 ms after that completion, before accounting for dispatch latency and real
HCD behavior. If more URBs have completed before dispatch, the available
coverage is lower. `busy` remains set until each application callback runs,
so the retained busy mask cannot establish exact Q. Long successive callbacks
must be assessed together; a single call below 18 ms is not proof of safety.
Steady average serialized service must be below 6 ms/URB to leave capacity for
MIDI, control work and scheduling. The historical <6 ms rejection criterion
and <=3 ms headroom target remain unchanged; neither becomes a new pool-depth
acceptance threshold in this audit.

The executable [arithmetic model](evidence/2026-11-02-usb-service/service_budget_model.py)
has six passing checks and [saved examples](evidence/2026-11-02-usb-service/service-budget-results.json):

- With ideal initial queueing, a 17386 us first callback finishes at 23386 us,
  before the old 24000 us tail; that individual callback need not empty the pool.
- A following 8385 us callback finishes at 31771 us, after the replenished 30000 us
  tail: the ideal model has a 1771 us gap.
- A 6000 us dispatch delay plus a 14000 us refill similarly exceeds initial coverage.
- At exactly 6000 us/callback the ideal CPU budget is saturated; 7000 us recurring
  callbacks eventually exhaust finite pool slack.

These are illustrative assumptions, not replay of observed completion times
or a driver simulator. Real logs lack IRQ completion-to-client dispatch and
submission timestamps/frame coverage. Zero recorded errors/idle underruns do
not establish the full service bound; the recorded criterion still failed.

## 2. Work coincident with the early outliers

`probe.c` starts the asynchronous SD bridge, then USB enumeration. The SD Rust
client is pinned to core 1 at priority 3; USB client is core 0 at priority 4.
In `idf_media_bridge.rs`, `report(PASS, loaded)` precedes `serve_decks`.
Inside `serve_decks`, initial `attach_ring(0)` and `Producer::new()` occur while
USB already streams. Deck 2 allocates the same resources lazily at first LOAD.

`attach_ring` allocates and zeros 65536 stereo i16 frames (262144 bytes).
`Producer::new` allocates/copies static DecoderStorage and three MP3 checkpoint
stores and allocates a 98304-byte encoded buffer. Exact control ELF symbol
sizes are 111144 bytes for INITIAL_STORAGE and 36560 for INITIAL_CHECKPOINT.
Payload per deck therefore totals:

`262144 + 111144 + 3*36560 + 98304 = 581272 bytes`.

In control cold 1 the PASS log has 29701324 bytes PSRAM free; later producer
telemetry has 29120028: a 581296-byte decrease, close to that concrete payload.
This is a global heap observation, not a per-allocation accounting proof.
All six retained worst callbacks occur around ordinals 470–480 and catalog
phase 5. The phase remains 5 after PASS until another tracked event; it does
not timestamp allocator entry/exit. Two cores sharing external memory/cache
and copying flash-resident initializers is a plausible contention source,
consistent with zero scheduler-switch deltas. Causality is **NOT_PROVEN**.
P4 internal RAM also has L1/cache behavior; the already-audited internal DMA
buffer contract does not imply stall-free execution.

Crucially, none of the measured diagnostic decks was loaded. MP3 decoding,
resampling of an active track and playing-source starvation cannot be assigned
as the cause of these idle events. The pack placement experiment shows that
moving only those instructions does not remove the problem; it does not tell
us which remaining contention or computation path dominates.

## 3. Decision: defer render-ahead; prepare resources before audio activation

**DECISION:** do not migrate Mix/Engine into another task in the next change.
First implement one bounded startup-resource lifecycle change: prepare both
decks' permanent rings/decoder/checkpoint/encoded resources before any product
UAC prefill/submission. Reuse them for every LOAD/seek and avoid first-use deck-2
bulk allocation while the other deck plays. Keep USB transfer count, priorities,
DSP algorithms, gain, native minimum and packetizer's normal flash placement.
No queue-depth increase or render-graph IRAM move belongs in this candidate.

This is a hypothesis-driven candidate, not an asserted fix. It changes startup
ordering and persistent resource timing, not playback state. Allocate/initialize
storage only; never call product fill/render, advance cadence/decks/transport,
open dummy tracks or run a state-mutating warmup to hide an outlier.

Required owner/state contract for that implementation:

1. SD-side initialization exclusively owns unpublished resources. Use explicit
   UNINITIALIZED -> PREPARING -> READY or FAILED states. Publish READY with
   release/acquire semantics only after *both* decks' storage is valid. A USB
   caller may read scalar readiness/failure, never mutate the producer/resources.
2. Producer/file/volume ownership stays on the SD owner; Engine/Mix, controls,
   render graph, source readers and output remain exclusively on the USB client.
   Preserve all existing typed load/seek permits, leases, receipts and revocation.
   Do not introduce shared `&mut Mix`, locks around DSP or another filesystem owner.
3. USB enumeration/MIDI/event handling must remain responsive while PREPARING.
   Defer product UAC initialization/prefill using an explicit nonblocking state
   and client-loop poll. Retain selected scalar format/interface state, not a
   borrowed descriptor pointer after its documented lifetime. No busy wait,
   callback allocation, delayed-action recursion or false STREAMING state.
4. Distinguish media readiness from resource readiness. No card, mount failure,
   init failure, removal and controller disconnect need explicit terminal paths.
   READY resources alone do not authorize a media lease or LOAD. FAILED must
   leave audio unavailable and keep display/MIDI usable; no partial ring is read.
5. Initialization is fallible and idempotent. Current Box/vec allocations abort
   on OOM; use an audited fallible backing allocation/guard strategy, never
   construct huge temporary decoder values on the stack. Partial failure frees
   unexposed memory; after ring publication never free memory a reader may hold.
   Current `attach_ring` allocates before testing its existing pointer, so merely
   retrying that function leaks an extra allocation. Introduce explicit one-time
   ownership rather than reusing it as a retry primitive. No hot remount added.
6. Budget approximately 1162544 bytes payload for both decks before allocator
   overhead. Relative to currently initialized deck 1, reserve another 581272
   bytes earlier. That is timing of already-required second-deck capacity, not
   evidence of the final measured heap cost. Measure actual peak/minimum heap,
   largest allocation and stack; do not raise stack thresholds to pass.
7. Both producers consume their prepared stores without repeating bulk setup.
   Preserve fixture/deck-play/deck-mix/engine feature behavior explicitly; scope
   the initial new gate to the chosen product engine candidate and keep lower
   diagnostic profiles building. Returning to known s07 remains independent.

Prefer an SD-task-owned resource bundle initialized before the existing
preflight/serve loop and moved into that loop once. Pass ownership explicitly
to `serve_decks`; a global mutable producer object shared with USB is unnecessary.
Readiness publication needs only a scalar status/generation. This keeps raw
decoder/file types out of the C ABI and makes partial-init cleanup testable.

### Why a separate render owner is deferred

Current `deck_mix::Shared` is `UnsafeCell<Mix>` with an unsafe Sync promise of a
single USB-task caller. MIDI drain/control, connection handling, load admission,
LED refresh and audio fill all call it directly. Running only graph.render on
another task would violate that promise. A real separation requires moving all
Engine/Mix mutation to the render owner and replacing USB accesses with bounded
typed command/result mailboxes and atomic status publication. LED acceptance
must follow committed effects, not enqueue success. Startup/recovery needs stream
generations and prefill handshakes. Transport processing must continue when no
stream runs, as already required for seek completion.

For scale, 2048 frames of stereo MAIN plus stereo headphones require 16384 bytes
payload at i16, plus metadata. A target ahead of 1024 frames adds 21.333 ms to
the existing nominal 24 ms USB pool; a full 2048 frames adds 42.667 ms. Neither
latency is currently approved for CUE/jog/seek behavior. A queued mixed block
cannot simply be discarded on one deck's source change: the R1 peer skip would
reappear over a larger queue. Selective invalidation needs per-deck contributions
or an explicitly accepted drain-to-boundary contract. Global media revocation
still needs bounded silence and may discard both decks for this single lease.

Further, current pack wall time alone can exceed 6 ms even in IRAM. Copy/pack
remaining in the callback is not proven bounded by moving graph work away.
The proposed PCM queue must address shared-bus stalls and producer throughput;
it cannot manufacture CPU/bus capacity. These ownership/latency uncertainties
make task separation a larger unproven first change than resource initialization.
If the narrow startup candidate fails, revisit this decision with its result;
do not silently combine the two architectures in one target campaign.

## Concrete acceptance tests before implementation is called complete

| Test | Required evidence |
|---|---|
| Readiness interleavings | USB tick during every init phase; no prefill/submit until both READY; no task waits for a lower-priority writer |
| Allocation failure | Inject each allocation failure; zero partial publication, correct frees, explicit FAILED, responsive MIDI/display |
| Repeat/init wrap | Repeated start/poll, disconnect and failure paths do not allocate twice, leak rings or reuse stale generation |
| Resource reuse | Count backing allocations/copies: none at catalog PASS, first deck-2 LOAD or later LOAD/seek for permanent stores |
| PCM identity | Real 44.1/48 kHz WAV/MP3/FLAC producer/render/packet paths equal reference after readiness, including priming and failed exits |
| Peer continuity | Both directions with partial blocks, LOAD outcomes, seek outcomes, EQ/filter/pitch; existing R1 regression retained |
| Transport without stream | CUE/seek/LOAD receipts and disconnect still resolve while UAC is deferred/stopped |
| Lifetime | Media removal during PREPARING/READY/playing, stale receipts and queued USB drain retain existing fence/ownership policy |
| Driver startup/recovery | Actual C harness: readiness deferred, gone/EP0 failure, partial allocation, recovery retry/drain timeout and reattach |
| Budgets | Full feature build matrix, exact code/static/heap/stack accounting; normal image excludes pack-placement diagnostic |

No production implementation is made here. The model tests are mathematical
checks only; none closes these implementation acceptance tests.

## Next execution sequence and physical gates

Implement the startup-resource change alone, with failing interleaving tests
first. Use the normal flash packetizer baseline, compare against the same
functional peer-fix baseline, and retain the failed placement images unchanged.
Review/publish/CI, fresh clean packages and a new 36-predecessor registration
come before any device access. Measure readiness time and first/steady callback
timing with the existing calibrated instrumentation; keep >=6000 us rejection
and <=3000 us headroom target. Require repeated cold observations and actual
heap/stack evidence. Do not treat a disappeared single spike as qualification.

After a candidate passes those gates, test both decks, MAIN/PFL, LOAD races,
CUE/seek/EOF, SD loss, USB reconnect and sustained exact-image playback (I02–I13).
New target failures require exact s07 restore/readback/cold/listening before
ending the campaign. No APTA, DAC, networking/OTA or extra peripherals are added.
