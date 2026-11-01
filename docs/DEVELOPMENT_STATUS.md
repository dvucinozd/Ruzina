# Current development status

Updated 2026-11-01. Development has two distinct backends. Native requires
ESP32-P4 >=302. The arrived JC1060 is v1.3/103 and uses pinned IDF 6.1 with
shared Rust. Active continuation is `ogranak/jc1060-uac-callback-profile`, clean
diagnostic source `8a9fe16d48387b0a3cfbd461659696091b2aa89d`, stacked on the
repair branch through PR #75 -> #74 -> #73 -> #72 -> #71. These PRs remain unmerged.

| Component | Current evidence | Next gate |
|---|---|---|
| Shared product logic | Rust host implementations for transport, media import, mixer/DSP and management contracts | Target adapter wiring and physical qualification per feature |
| Legacy SD/library/display | 25-track real export, shared PDB/ANLZ preflight, NewPanel/touch and combined controller diagnostic observed | Broader fixtures and product UI |
| Legacy deck-play | Accepted record: app `95d45203...`, FLX4 headphones, deck 1 FLAC/MP3/WAV; 90 s rendered audio in 15 min USB stream capture | No transfer of this acceptance to newer images |
| Legacy deck-mix/deck-engine | Host/build implementation; s08/s09 history reconciled; no inherited two-deck acceptance | [Measure startup versus recurring UAC cost](reviews/2026-11-01-uac-timing-follow-up.md) |
| Review repairs A–H / campaign I | Offline validation complete; published PR #73 passed 19/19 CI jobs; exact candidate booted but I01 timing criterion failed at 12260 us | Candidate rejected for this campaign; s07 restored/readback and cold boot verified, normal playback owner-confirmed with zero captured underruns; I02–I13 NOT_RUN |
| UAC phase diagnostic | [Measured](reviews/2026-11-01-uac-timing-device-results.md): prefill max 1844 us, callback max 10373 us, 3/6000 callbacks >=6000 us; zero idle underruns/USB errors | Timing criterion FAIL; s07 restored/cold boot and normal playback confirmed, monitors closed; next is bounded callback substage/preemption profiling |
| UAC callback substage diagnostic | [Measured](reviews/2026-11-01-uac-callback-profiling-device.md): coherent 13087/13039 us records, exclusive partitions valid, zero observed switch deltas; 9000 steady callbacks add no threshold crossings | Aggregate FAIL; s07 restored/readback/cold boot and normal sound/Pause/CUE confirmed, monitors closed. [Cause plan](reviews/2026-11-01-uac-callback-cause-follow-up.md) awaits hook-liveness and IRQ/cache/compute attribution; future plans require 31 predecessors |
| Native >=302 firmware | Separate compile-only backend on this unit | Compatible silicon and independent hardware gates |
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
