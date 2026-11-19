# Current development status

Updated 2026-11-19. The board keeps image s140 (app `df3ae93c…`), the `--usb-stick` build
of `main`; the SD build of the same code is s136 (app `a666ac6e…`).

## What runs on the board

The owner's JC1060 (ESP32-P4 v1.3, rev103) runs the legacy path: ESP-IDF 6.1, with the
shared `no_std` Rust core in [`firmware-idf/legacy-jc1060`](../firmware-idf/legacy-jc1060). Each row
below has a device log and the owner's verdict in its review. The
[handoff](HANDOFF.md) has the full log, newest last.

| Area | On the board | Review |
|---|---|---|
| Two MP3 decks | 59 s clean, 0 underruns, at `opt-level = 3` | [dual MP3](reviews/2026-11-03-dual-mp3-opt3.md) |
| Storage and speed | A read-ahead: 8 KiB per stick read, up to 16 sectors from an SD card; QIO 80 MHz flash; 200 MHz PSRAM | [SD reads](reviews/2026-11-04-sd-multisector.md), [flash](reviews/2026-11-04-flash-qio80.md), [PSRAM](reviews/2026-11-04-psram-200m.md) |
| Seeks | Indexed MP3 seeks, untagged VBR included | [MP3 index](reviews/2026-11-04-mp3-seek-index.md), [VBR](reviews/2026-11-05-untagged-vbr-seek.md) |
| Long runs | Soaks of 35 to 42 minutes, and 45 minutes of scripted gestures, 0/0 underruns | [soak](reviews/2026-11-04-soak-autoplay.md), [every control](reviews/2026-11-06-full-control-soak.md), [stick soak](reviews/2026-11-19-stick-soak.md) |
| Music from a USB stick | exFAT stick behind a hub on the HS port, with the FLX4; MEDIA REMOVED when pulled out, an app restart when put back; no SD card | [stick behind a hub](reviews/2026-11-19-usb-stick-hub.md), [8 KiB reads](reviews/2026-11-19-stick-8k-reads.md) |
| Loops and scratch | From a played-audio history, with 5.8 ms crossfades | [loops](reviews/2026-11-05-loops.md), [scratch](reviews/2026-11-05-scratch.md) |
| Beat FX and Pad FX | In the render graph; FX page by touch | [FX](reviews/2026-11-05-fx.md), [FX by touch](reviews/2026-11-12-fx-touch.md) |
| Hot Cues | From Rekordbox, own ones in a flash journal, erased ahead | [Hot Cues](reviews/2026-11-06-hot-cues.md), [erase ahead](reviews/2026-11-13-hot-cue-erase-ahead.md), [page](reviews/2026-11-12-hot-cues-page.md) |
| Memory cues | Rekordbox memory cues on the lane, the HOT CUES page and the SAMPLER pads | [memory cues](reviews/2026-11-17-memory-cues.md) |
| FLX4 LEDs | Pad modes, SYNC, PFL, Smart buttons, Beat FX ON | [LEDs](reviews/2026-11-06-controller-leds.md) |
| SYNC and Quantize | Beat sync, continuous SYNC, Quantize | [beat sync](reviews/2026-11-11-beat-sync.md), [continuous](reviews/2026-11-13-continuous-sync.md), [Quantize](reviews/2026-11-12-quantize.md) |
| Master Tempo | Its own keylock; with SYNC, phase within 1 ms | [Master Tempo](reviews/2026-11-13-master-tempo.md), [stops](reviews/2026-11-13-mt-phase.md), [controller](reviews/2026-11-14-mt-on-controller.md), [with SYNC](reviews/2026-11-16-sync-mt-line.md) |
| Deck screen | Slint, with waveforms drawn by Rust; marks, Library, covers, Croatian letters | [screen](reviews/2026-11-10-deck-screen.md), [marks](reviews/2026-11-12-lane-marks.md), [Library](reviews/2026-11-12-library.md), [letters](reviews/2026-11-13-latin-glyphs.md) |
| Screen speed | Big frames 84 → 61 ms (Library), 50 → 41 ms (Settings) | [draw speed](reviews/2026-11-16-screen-draw-speed.md), [UI pool](reviews/2026-11-16-ui-pool.md) |
| Playlists and covers | Rekordbox's playlist tree in the Library, each deck's cover | [playlists and covers](reviews/2026-11-16-playlists-covers.md) |
| Settings | LOAD LOCK, VINYL/CDJ, brightness, MAIN trim, kept on flash | [settings](reviews/2026-11-17-settings.md) |
| Wi-Fi and Web Remote | The deck's own network (C6 via ESP-Hosted); play, cue, library, mixer, SYNC, four connections, soft takeover | [network](reviews/2026-11-15-wifi-in-the-deck-image.md), [Web Remote](reviews/2026-11-15-web-remote-play-cue.md), [takeover](reviews/2026-11-15-fader-takeover.md), [connections](reviews/2026-11-16-web-connections.md) |

## Still to come

The final layout: the board's own power on CN2, the stick on the FS port and the log on
UART0.

## Open

- **USB audio fill:** over the 3 ms target, with no underruns: 3.2 ms while mixing, and
  5.95 ms at most in the 42-minute stick soak, where 11 % of fills were over 3 ms.
- **Screen:** Slint's scene preparation is still 16 to 26 ms of a big frame.
- **SD (SD build only):** an SD clock failure at the first boot after a flash. Since s132 the card is
  tried three times ([retry](reviews/2026-11-18-sd-retry.md)), not yet seen working on a
  failing boot. Since s116 it no longer
  takes the Wi-Fi's slot down with it ([SD slot](reviews/2026-11-16-sd-slot-teardown.md)).
- **Not in a device log yet:** the Delay effect and the Beat FX channel switch.

## Evidence layers

- **Physical:** the JC1060 above, on the legacy path, each with its image and log.
- **Host and simulator:** [`firmware-rust`](../firmware-rust/README.md), with the shared
  crates' tests and the desktop simulator.
- **Native `esp-hal` firmware** (`apps/m1-firmware`, silicon rev 302 and up): compiles
  only; this unit is rev103.
- **The JC8012:** not qualified.

The bring-up campaign of 2026-11-01 to 2026-11-03 (s07, history 44) is
[archived](reviews/2026-11-03-checkpoint-archive.md).

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
