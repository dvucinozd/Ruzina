# Current development status

Updated 2026-10-30. Main after PR #55: `46728b2a3779d125b50aadae908b293adaafa25a`.
The legacy JC1060 bring-up batch is merged; the next batch starts from main.

Current unit runs the shared Rust partition/exFAT read-only diagnostic, with
full fixture and bounded removal/reinsert/cold-return observations. The shared
[load preflight](reviews/2026-10-30-legacy-s6-media-load-preflight.md) runs
host-tested over that exFAT volume and builds for the target with an IDF PSRAM
allocator; shared PDB catalog import and an opt-in profile are the next gates. Publication/review/CI is recorded in
the [publication checkpoint](reviews/2026-10-30-legacy-publication.md); historical
uncommitted-source fields in runtime archives describe their build-time state.
The publication follow-up accepts exFAT `PartitionOffset=0` with baseline-failing
and mismatch regressions: 29 Rust tests pass. That source fix has a fresh IDF
BUILD_ONLY package and was NOT_FLASHED. After the owner raised the Actions
limit, PR #55 head `00294d18cd59ba5afcd6aaa4b9dcba8465c895b2` passed host,
native P4, all eight legacy IDF profiles and the media-bridge host job, and was
merged.

The Rekordbox catalog import is shared too
([catalog import](reviews/2026-10-31-shared-pdb-catalog-import.md)): native
firmware runs it, and the IDF core imports `export.pdb` from its exFAT volume
in host tests.

The [`sd-rust-load` profile](reviews/2026-10-31-legacy-sd-rust-load-profile.md)
runs catalog import and load preflight on the IDF SD bridge. Its stack-fixed
image ran on JC1060 with one real 25-track export: 25/25 tracks loaded
analysis in every pass, with flat PSRAM and clean removal, reinsertion and
cold return. It is a diagnostic, not qualified. The
[`sd-rust-load-display`](reviews/2026-10-31-legacy-load-display.md) variant adds
a status screen with the track list and passed the same device runs. Touch
selection now shows each track's analysis detail and preview waveform, which
matched the host import for every tapped track. The isolated
[`usb-host`](reviews/2026-10-31-legacy-usb-host-enum.md) diagnostic enumerates
the owner's DDJ-FLX4 at high speed on the HS port, with its UAC 4-channel
playback, MIDI, HID and vendor interfaces, through three hot-plug cycles.
[`usb-midi`](reviews/2026-10-31-legacy-usb-midi.md) claims its MIDI
interface and runs the FLX4 profile through the shared controller session on
the board: buttons, faders, crossfader and jogs, with 0 drops. Its Bulk-OUT
drives a diagnostic LED echo and lamp test. The combined
[`deck-ui`](reviews/2026-10-31-legacy-deck-ui.md) runs SD load, the screen and
the FLX4 together on the board: browse and load drive the library screen.
[`deck-audio`](reviews/2026-11-01-legacy-deck-audio.md) adds the first audible
output: a UAC1 tone on the FLX4 headphone channels, 48 s with 0 isochronous
errors.
[`deck-play`](reviews/2026-11-01-legacy-deck-play.md) replaces the tone with
deck 1's track, decoded from the card by the shared audio runtime and
resampled to 48 kHz. It is built and host-tested; the device run is pending.

Latest [known-file SD evidence](reviews/2026-10-30-legacy-s6-sd-file.md): full
fixture SHA/content/EOF/seek checks and active-read removal/reinsertion are
observed on the isolated IDF diagnostic. The product media adapter remains
unwired. Rechecking all nine original raw timing records withdrew the earlier
printf-failure claim: wall time is about 5.12 million microseconds and exceeds IO.
Current file-image cold/reconnect evidence is not inherited. The
[Rust read-only bridge protocol](reviews/2026-10-30-legacy-s6-rust-media-bridge.md)
has 19 host ownership/cancellation tests. Its [concrete IDF transport](reviews/2026-10-30-legacy-s6-idf-media-transport.md)
adds a concurrent host FFI test and now runs on the unit as a raw-block
diagnostic: 35 first-boot passes plus 3 app restarts with 15 passes each match the
known sector hash. Physical idle removal quiesced without late success;
same-card reinsertion plus 3 app restarts each added 15 matching passes. An
owner-confirmed cold return has a captured uptime drop and 15 later matching
passes; initial init/reset-reason prefix was missed. Shared Rust filesystem
integration was the next gate in that raw-only campaign. The subsequent
[shared Rust exFAT profile](reviews/2026-10-30-legacy-s6-rust-exfat.md) now mounts
the physical card, opens the UTF-8 fixture and verifies full SHA/EOF/10 seeks:
three first-boot passes plus three app restarts with one pass each. Host tests
cover canceled filesystem mount/read with buffer drain and remount; the actual
product load/executor path is still unwired. On this Rust exFAT image, open-file
removal quiesced the owner/client without late success, reinsertion plus 3 app
restarts each reverified the file, and owner-confirmed cold return completed a
new full pass (boot prefix missed). No product storage/audio qualification is
implied; media/load owner integration is the next software gate.

The latest [read-only SD/exFAT campaign](reviews/2026-10-30-legacy-s6-sd-readonly.md)
mounted the 64 GB card and enumerated 0 root files/1 directory; file-read testing
was unrun on that historical mount-only image and is covered by the later
known-file profile above. It uses a separate SD-only diagnostic, temporarily
replacing the display profile. Lifecycle results are recorded per scenario;
the shared product media owner is not yet connected to this driver.

S6 [display/touch diagnostic](reviews/2026-10-30-legacy-s6-display-touch.md)
has observed New_Panel initialization and GT911 config 1024x600, with operator
confirmation of the static colors/corners. Touch/boot scenario outcomes are
unit-specific; no product UI is active, and that display profile has no SD owner.

S5 [memory/task evidence](reviews/2026-10-30-legacy-s5-memory-task.md) now
records bounded PSRAM 80 MHz, scratch/allocator, C/Rust FP preemption and EQ
measurements on the separate IDF backend. It does not promote product rows
below to hardware PASS. Native 302 target requirements remain unchanged.

Latest: the separate [S4 IDF diagnostic campaign](reviews/2026-10-30-legacy-s4-runtime.md)
has written and readback-verified a scalar image, observed runtime heartbeats
after operator power removal and passed three app restarts. Native revision 302
and all product-peripheral hardware gates remain unchanged. Refer to the S4
report for individual USB/reset/reconnect outcomes; the RAM reset failure below
is historical evidence of that specific mode.

The received JC1060 on COM3 is **v1.3 (103), 16 MiB flash**. Native C02 is
blocked by incompatible silicon; its minimum remains 302. The owner requested
a separate ESP-IDF assessment, using current stable 6.1. See the
[arrival record and implementation gates](boards/JC1060_ARRIVAL_2026-10-30.md).
Both factory backups match with verified secondary copies. The isolated IDF
6.1 / Rust probe executed scalar calls twice from RAM, but native USB warm reset
then repeated ROM watchdog resets. The owner completed power disconnect;
subsequent ROM/factory boot succeeded and the factory display is confirmed.
Warm reset remains unqualified. No product firmware has been
flashed. JC8012 is still unobserved. The product
hardware rows below remain NOT RUN; read-only ROM identity is not product PASS.
Validation outcomes for the working changes are recorded in the
[execution log](reviews/2026-10-27-astra-implementation-log.md).
Phase B progress and gates: [management integration log](reviews/2026-10-27-phase-b-log.md)
and [HTTP contract](WEB_INTEGRATION_CONTRACT.md).
The [parity audit](M2_5_PARITY_PLAN.md) supplies detailed source/test evidence.

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
