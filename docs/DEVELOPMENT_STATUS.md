# Current development status

Latest [continuous hardware observation](reviews/2026-11-03-continuous-device.md)
admits cold/idle and individual MP3 sources, but rejects both-playing with
78/94 source underruns after 15.88 seconds. S07 is restored by full readback;
software boot pulse completed, application NOT_OBSERVED. No manual recovery
checks repeated. Current history **44**; 42/43 plans consumed. Next is offline
decoder-step/synchronous-read budget attribution, then one independently
reviewed change and fresh registration. Native minimum **302** is unchanged.
The following status entries describe older checkpoints and are superseded
where they name current device state, history or next actions.

New [42/43 continuous-source plans](reviews/2026-11-03-continuous-registration.md)
and contracts pass actual-file offline preflight; full factory/history and
source checks pass. Combined suite: 53 tests per OS. No target access or new
device result. Writer review raised 0 issues; `b92fb23` source CI passes 22/22.
Dual-copy archives and all separate bindings pass. Next: publication/remote and
offline byte verification, actual topology refresh, then the registered candidate
experiment. Older checkpoint counts/status follow.

Latest [producer-profile campaign](reviews/2026-11-03-source-supply-device.md):
cold/idle pass at 1880 us; functional playback NOT_CAPTURED, initial joined
telemetry rejected. S04/S05 NOT_RUN; root cause open. S07 is restored and
owner-confirmed; tools closed. Current history 42/full
`28830827d3e3426bbcd58b9a25d4d2129e28f150bade33fe3c551cd577f93921`.
The [owner recovery policy](RECOVERY_POLICY.md) removes future repeated s07
physical cold/listening/Pause/CUE checks; keep automatic full readback.
The [continuous adapter/analyzer](reviews/2026-11-03-continuous-adapter.md) now has
39 offline framing/readiness/whole-session tests per OS. No COM3 access. Next:
verify latest CI for the reviewed adapter and [software boot action](reviews/2026-11-03-software-boot.md),
new 42/43 plans and exact bindings. Combined suite now has 46 tests per OS. Older checkpoints
below retain historical state.

CodeRabbit connection and combined review are complete: one minor count issue
fixed, final fix review 0 issues. Exact tooling/tests/review attempts have a
verified dual-copy archive; no execution binding or new device result follows.

The [producer campaign](reviews/2026-11-03-source-supply-execution.md) is now
offline registered against 40/41 predecessors. Both plans and dual-copy archive
pass; final tool review has 0 issues, exact CI passes 22/22, and the four-file
binding/dual-copy tools archive is frozen. Publication/preflight precedes access.
No COM3 access or new write at this checkpoint;
actual installed s07/history 40/full 7875f650 remains unchanged.

Active [source-supply diagnostic preparation](reviews/2026-11-03-source-supply-profile.md)
adds explicit OFF-by-default producer/USB wall measurements, independently of
render/IRQ profiling. Final fix review has 0 issues, exact CI passes 22/22,
and clean normal/diagnostic builds pass source/artifact/symbol/stack checks.
The 15118202-byte archive and all secondary hashes match. No new flash or
registration; installed s07/history 40 and its accepted recovery are unchanged.
Next: separate frozen target runbook, 40/41 recovery plans and exact reviewed
execution/analysis tools; the dual-MP3 root cause remains open.

Latest [normal-engine physical result](reviews/2026-11-02-normal-engine-device.md):
**two-deck REJECTED**, s07 restored/full-readback/cold/listening confirmed.
Cold/idle passes at 2397 us; individual MP3 listening passes with max 3214 us
headroom target failure. Both playing: underruns 4765/5003, packet_errors 1,
source supply 0/0, max 4982 us. PFL and N03-N08 stay open/NOT_RUN. Current
executed ancestry **40**, full flash
`7875f650ba7404475e9c26561990851b04c11409b3f309401f6e77a51ddf38a6`.
All monitors closed; dual-copy completed archive verified. Next is offline
source-supply/USB service root-cause measurement design, then new reviewed
40/41-plan registration; no threshold increase, normal promotion or merge.
The paragraphs below describe historical preparation checkpoints.

Latest tool checkpoint: `08a537b` has CodeRabbit **0 issues**, 13 normal monitor
tests per OS (four real-loop synthetic serial cases), and 13 shared-guard WSL
tests. Exact CI passed 21/21 and binding `a35aa0c4...` is frozen/dual-copy
archived; publication precedes execution. No device has been
opened by the normal campaign; the earlier rate-limit entries below are history.

Normal-engine preparation is on [PR #81](https://github.com/dvucinozd/Ruzina/pull/81).
[Source/package and 38/39-plan registration](reviews/2026-11-02-normal-engine-execution.md)
pass offline checks, source and registration each have 21/21 CI, and accepted
s07 remains installed. A major capture-duration finding is fixed/tested, but
the 22:10 review found another major connection-coverage issue. The monitor
now excludes initial port waiting and invalidates post-start disconnects;
seven normal coverage tests pass per OS. Prior tool/publication CI passed
21/21 each; this follow-up's own review/CI is pending. No normal binding,
flash or functional acceptance is claimed before those gates close.

The [resource-readiness device campaign](reviews/2026-11-02-resource-readiness-device.md)
passes three cold idle windows: 7000 callbacks each, maxima 2008/1847/1757 us,
zero >3000/>=6000 crossings, valid calibrations and zero USB/idle errors. A
separate 60-second steady window adds 9000 callbacks without new crossings/max.
This is idle timing evidence, not two-deck playback qualification. Exact s07
is restored with independent whole-flash readback, cold boot and owner sound/
Pause/CUE confirmation (13.566 captured playback seconds, no errors).
Current ancestry is **38**, full readback `1a45683632f9951f1121ad0866fd7b3050d146022bfc6c719be18346848ad4fe`.
All monitors are closed; device archive and secondary hashes pass. Next is a
separately registered normal-image I02-I13/resource campaign. No normal engine
write, automatic merge or peripheral expansion is implied. Earlier checkpoints
below retain their original limits.

Historical offline checkpoint: the [resource-readiness implementation](reviews/2026-11-02-resource-readiness.md)
on `ogranak/jc1060-resource-readiness` ([draft PR #80](https://github.com/dvucinozd/Ruzina/pull/80))
prepares both decks' permanent stores before engine UAC activation. Six host
profiles, nine target Clippy configurations, real C driver tests, exact-parent
review (0 issues), and clean normal/profile builds pass. BUILD_ONLY / NOT_FLASHED:
heap/stack runtime evidence and cold/steady timing remain NOT_RUN. Source
`d28abd0` and publication `a2997bd` each passed 21/21 CI jobs. Candidate/s07
return plans (36/37 predecessors), both offline preflights and a 311-file
dual-copy archive are ready; no plan is executed. Registration `79e251a` and
tool source `51c6967` each pass 21/21 CI; exact-parent tool review has 0 issues
and 13 fake-device tests pass per OS. Reviewed programmer/monitor and their
frozen binding are archived. Fresh operator ROM entry is the next physical gate.
S07, recovery ancestry 36 and native minimum 302 are unchanged. The paragraphs
below retain historical decision/device evidence.

The [service-budget and resource audit](reviews/2026-11-02-usb-service-and-resource-decision.md)
closes the post-placement plan's first three steps. DESIGN_ONLY: defer separate
render ownership and first prepare both decks' permanent resources before
product audio activation. Six arithmetic-model checks pass; these do not
qualify USB timing or implement the resource gate. S07/36-plan recovery is unchanged.

The [packetizer placement comparison](reviews/2026-11-02-pack-placement-device.md)
on `ogranak/jc1060-pack-placement` (PR #78 above #77) is MEASURED_NOT_QUALIFIED.
Three control boots reached 15827/17386/15849 us; three IRAM boots reached
14157/13599/15548 us. Both fail the registered timing gate. S07 was restored,
readback/cold boot verified and owner-confirmed with 20.112 s captured playback,
zero underruns/USB errors. All monitors closed; future ancestry count **36**.
Next is the [offline USB service-budget/design decision](reviews/2026-11-02-post-placement-next-step.md).

Offline continuation after the calibrated campaign: source `eead8c6` on
`ogranak/jc1060-peer-continuity-fixes` fixes peer-deck cached-output continuity
and fail-closed analysis schemas. Host/C tests, exact-parent CodeRabbit review
and clean normal/profile builds pass. See the
[repair record](reviews/2026-11-02-peer-continuity-fixes.md). It has no physical
timing acceptance; s07 remains the last accepted image and I02–I13 stay open.
The dated campaign results below remain historical evidence for their images.

Updated 2026-11-02. Development has two distinct backends. Native requires
ESP32-P4 >=302. The arrived JC1060 is v1.3/103 and uses pinned IDF 6.1 with
shared Rust. Active continuation is `ogranak/jc1060-uac-cause-profile`, clean
diagnostic source `3e20a8e64ef7f2ff855bf18706712e289c6cc747`, stacked on the
repair branch through PR #76 -> #75 -> #74 -> #73 -> #72 -> #71. These PRs remain unmerged.

| Component | Current evidence | Next gate |
|---|---|---|
| Shared product logic | Rust host implementations for transport, media import, mixer/DSP and management contracts | Target adapter wiring and physical qualification per feature |
| Legacy SD/library/display | 25-track real export, shared PDB/ANLZ preflight, NewPanel/touch and combined controller diagnostic observed | Broader fixtures and product UI |
| Legacy deck-play | Accepted record: app `95d45203...`, FLX4 headphones, deck 1 FLAC/MP3/WAV; 90 s rendered audio in 15 min USB stream capture | No transfer of this acceptance to newer images |
| Legacy deck-mix/deck-engine | Host/build implementation; s08/s09 history reconciled; no inherited two-deck acceptance | [Controlled cache/compute cause experiment](reviews/2026-11-02-uac-cache-compute-follow-up.md) before resuming qualification |
| Review repairs A–H / campaign I | Offline validation complete; published PR #73 passed 19/19 CI jobs; exact candidate booted but I01 timing criterion failed at 12260 us | Candidate rejected for this campaign; s07 restored/readback and cold boot verified, normal playback owner-confirmed with zero captured underruns; I02–I13 NOT_RUN |
| UAC phase diagnostic | [Measured](reviews/2026-11-01-uac-timing-device-results.md): prefill max 1844 us, callback max 10373 us, 3/6000 callbacks >=6000 us; zero idle underruns/USB errors | Timing criterion FAIL; s07 restored/cold boot and normal playback confirmed, monitors closed; next is bounded callback substage/preemption profiling |
| UAC callback substage diagnostic | [Measured](reviews/2026-11-01-uac-callback-profiling-device.md): coherent 13087/13039 us records, exclusive partitions valid, zero observed switch deltas; 9000 steady callbacks add no threshold crossings | Aggregate FAIL; s07 restored/readback/cold boot and normal sound/Pause/CUE confirmed, monitors closed. [Cause plan](reviews/2026-11-01-uac-callback-cause-follow-up.md) awaits hook-liveness and IRQ/cache/compute attribution; future plans require 31 predecessors |
| Native >=302 firmware | Separate compile-only backend on this unit | Compatible silicon and independent hardware gates |
| Calibrated UAC cause experiment | [Measured](reviews/2026-11-02-uac-cause-profile-device.md): both target calibrations pass; worst callback 14812 us, switch delta 0, covered IRQ bound 1593–1653 us; no new crossings in 9000 steady callbacks | Aggregate FAIL; s07 restored/readback/cold boot and normal sound/Pause/CUE confirmed, monitors closed. [Cache/compute plan](reviews/2026-11-02-uac-cache-compute-follow-up.md) is PLAN_ONLY; future plans require 33 predecessors |
| JC8012 | Separate BSP/source work | Independent arrival/recovery/peripheral/audio qualification |
| External audio | [Future PCM5122/TLV320 plan](boards/JC1060_EXTERNAL_AUDIO_PLAN.md), including separate I2C and candidate JP1 map | Physical pin/power/module confirmation, drivers and listening/timing campaign |
| C6/Web/settings/OTA | Host/simulator contracts | Real network, durable settings and OTA trust/layout/rollback adapters |
| APTA/recorder/Pro DJ Link | Separately scoped experimental/later work | No automatic product enablement or original APTA branch merge |

The offline repair batch did not contact the device. A subsequent owner-assisted
[physical session](reviews/2026-11-01-review-fix-device-results.md) verified the live
identity and full flash ancestry, tested the candidate and returned to s07 after
the timing gate failed. Future plans must include both the candidate and return
plan in the predecessor history; do not reuse the historical s09-only chain.
Current tests/builds and open gates are in the
[execution log](reviews/2026-11-01-review-fix-execution.md) and the
[review](reviews/2026-11-01-code-review-and-next-steps.md).

## Native/shared host matrix — historical pre-legacy integration snapshot

The table below describes the native/shared owners and simulator. Its target
and hardware columns do not erase the later legacy adapter evidence above.
Use dated per-image reports for legacy physical observations.

PASS = verified at the named layer; PARTIAL = some implementation/wiring only;
OPEN = required work remains; NOT RUN = no physical execution evidence;
N/A = outside the named component. A host PASS never implies hardware PASS.

| RP / component | Shared logic / host tests | Simulator wired | Target | Hardware | Next gate / evidence entry |
|---|---|---|---|---|---|
| 01 PDB title | PASS | PASS demo catalog | compile-only owners | NOT RUN | Real MSC/import; media-import fixtures |
| 02 S3CP v2/v3/v4 | PASS | PASS HTTP→install→rescan over RAM | compile-only owners | NOT RUN | Real controller MIDI/LED/initialization and storage |
| 03 CUE / VINYL / CDJ | PASS | PASS state/transport | compile-only owners | NOT RUN | Physical MIDI/audio; deck tests |
| 04 Load admission | PASS | PASS | compile-only owners | NOT RUN | Real queue/race timing; load-admission tests |
| 05 Memory/loop/source cues | PASS | PASS | compile-only owners | NOT RUN | Audible loop/preview acceptance; deck/runtime tests |
| 06 Playlists | PASS | PASS demo tree | compile-only owners | NOT RUN | Real export import; catalog/browser tests |
| 07 Artwork / PWV4 | PASS | PASS cover/waveform | inactive display contracts | NOT RUN | Panel/PSRAM and actual media; UI goldens |
| 08 MP3 index/seek | PASS | N/A: Session has no audio DSP | compile-only owner | NOT RUN | Broker fairness/latency; codec_streams tests |
| 09 UAC / MAIN clock | PASS contracts | N/A: no USB device transport | standalone crate check; no driver | NOT RUN | Physical USB adapter, descriptor capture, listening |
| 10 USB roles / MAIN changes | PASS contracts | PARTIAL state/demo | no board role activation | NOT RUN | Port/power map and teardown receipts |
| 11 Settings | PASS fake flash / power cuts | PASS HTTP config and RAM-backed flash | standalone crate; no board flash owner | NOT RUN | Durable target adapter |
| 12 Web/C6 | PASS HTTP / upload leases / async JS | PASS settings/profile/resources/log APIs | standalone no_std core; no C6 driver | NOT RUN | C6 network and TLS |
| 13 Signed OTA | PASS incremental HTTP→fake A/B and rollback | Firmware metadata; upload 503 without trust/slot adapter | standalone crate; no flash/TLS adapter | NOT RUN | Configure board trust/project/slot adapter, physical rollback |
| 14 Resources / regression | PASS bounded contracts | PASS Health + resources/log snapshots | runtime-health compile-only | NOT RUN | Measured board budgets; heap values explicitly unavailable |
| 15 Reconnect fencing | PASS | PARTIAL synthetic media lifecycle | compile-only owners | NOT RUN | Real attach/detach/recovery stress |
| 16 Headphone gain | PASS packer/render contracts | N/A: no audible output | no physical UAC packer wiring | NOT RUN | Per-controller channel map/level/listening |
| 17 MT starvation/cache | PASS synthetic regressions | N/A: no audio DSP | compile-only owners | NOT RUN | Guition timing/cache/DMA budgets |
| 18 Import/transport edges | PASS fixtures | PARTIAL demo/session | compile-only owners | NOT RUN | Real export/controller acceptance |
| 19 Extra controllers | OPEN optional | OPEN | OPEN | NOT RUN | Absolute PFL semantics if latching controller enters R1 |
| 20 Pro DJ Link | OPEN optional | OPEN | OPEN | NOT RUN | Separate scoped contract; not phase A |

Guition board selection starts the scheduler and optional diagnostics. It does
not start the product USB/storage/audio owners. `integration-check` compiles
owner modules and intentionally cannot be combined with a board selector.
Standalone crate compilation does not establish a firmware dependency or wiring.

The simulator uses fixed demonstration tracks and per-process RAM settings;
Session controls do not render audible DSP. Decoder/render/packer tests run
separately against actual shared audio logic. `/`, assets, `/api/status`,
`/api/control`, `/api/library`, `/api/load`, profile/config upload and management
GET routes are implemented. Upload bodies are bounded, single-flight and
cancelled on deadline/EOF. Recording routes remain 501. The demo OTA upload
returns 503 because no trust/project/slot adapter has been selected; its full
HTTP→incremental install→boot/rollback path is tested with explicit synthetic
trust and the A/B device model in ruzina-ota.
Recorder remains outside R1; APTA remains OFF/unmerged for the product.

Remaining host/acceptance work includes permitted real-export/capture fixtures
and selecting the product OTA trust/project/slot adapter. The phase B log names
the implemented composition tests and remaining boundaries. Physical work follows
phase C after unit identity, backup and recovery.
Both Guition models need independent qualification. Headphones always remain on
the controller while an initialized external DAC may take stereo MAIN.
