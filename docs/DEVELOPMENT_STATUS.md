# Current development status

Updated 2026-11-25. This page is the one place that names the board's images; the other
documents link here.

## The board's images

- **Kept for play:** s205 (app `2f9cd29e…`), the `--slint-ui --fill-iram --usb-stick`
  image without the soak DJ. Its firmware is `ogranak/web-remote-settings`: main
  `cac3bab` (smooth lanes, a stick put back read again: #168, #169 and #170) with the
  [Web Remote's SETUP page](reviews/2026-11-25-web-remote-setup.md).

  It was built with the
  [play image recipe](../firmware-idf/legacy-jc1060/README.md#build-recipes-for-the-deck-screen-image),
  less `--soak-autoplay`. The owner used SETUP from a phone on it on 2026-11-25 ("Sve
  radi").
- **Before it:**
  - s203 (app `17de0ea8…`), the play image of #170. On it the owner tried Delay and the
    Beat FX channel switch ("Radi").
  - s201p and s202p (apps `35074ecd…`, `89f53247…`), profile images of the stick work. On
    them the owner pulled the stick out and put it back ("Radi").
  - s200 (app `86c0351a…`), the play image of #169.
  - s199p (app `08c53e6f…`), the same firmware as a profile image with the soak DJ. On
    it the owner tried short loops at the largest zoom, the pitch fader and the zoom ("Sve
    glatko").
  - s198 (app `1f8e8267…`), the play image of #168.
  - s188 (app `eb4d42c3…`), main `d6ef0d0`, the play image of 2026-11-22.
- **Last documented measurements** (profile images, the FLX4):
  - **s189p to s199p (2026-11-25):** the lanes, from main to smooth
    ([smooth lanes](reviews/2026-11-25-smooth-lanes.md)).
  - **s169 and s170:** the fill gap against s163
    ([fill gap](reviews/2026-11-22-fill-gap.md)).
  - **s184p, s185p and s186p:** the redesign's frame rate
    ([redesign on the fill gap](reviews/2026-11-22-redesign-on-fill-gap.md)).
- **Where this comes from:** the dated reviews and the board log in
  `D:\BoardEvidence\JC1060-20261030-COM3` (`FLX4-DAY.md`), as of 2026-11-25.
  - This page does not read the flash: what is on the board right now is whatever its
    boot log's app hash says.
  - The handoff lists the older images and where each flash backup is kept.
- **The SD build** s136 (app `a666ac6e…`) is from before even fills.

## What runs on the board

The owner's JC1060 (ESP32-P4 v1.3, rev103) runs the legacy path: ESP-IDF 6.1, with the
shared `no_std` Rust core in [`firmware-idf/legacy-jc1060`](../firmware-idf/legacy-jc1060). Each row
below has a dated review with its device evidence, and each review says which kind:
- a device log with counters (underruns, fill times, read times);
- the owner's observation on the board, seen or heard;
- or both.

The long soaks were scripted or unattended, with logs and counters only: nobody listened to
them. The owner's listening checks are separate, shorter runs. The
[handoff](HANDOFF.md) has the full log, newest last.

| Area | On the board | Review |
|---|---|---|
| Two MP3 decks | 59 s clean, 0 underruns, at `opt-level = 3`; owner listened | [dual MP3](reviews/2026-11-03-dual-mp3-opt3.md) |
| Storage and speed | A read-ahead: 8 KiB per stick read, up to 16 sectors from an SD card; QIO 80 MHz flash; 200 MHz PSRAM | [SD reads](reviews/2026-11-04-sd-multisector.md), [flash](reviews/2026-11-04-flash-qio80.md), [PSRAM](reviews/2026-11-04-psram-200m.md) |
| Seeks | Indexed MP3 seeks, untagged VBR included | [MP3 index](reviews/2026-11-04-mp3-seek-index.md), [VBR](reviews/2026-11-05-untagged-vbr-seek.md) |
| Long runs | Soaks of 35 to 42 minutes, and 45 minutes of scripted gestures, 0/0 underruns; logs and counters only, nobody listened | [soak](reviews/2026-11-04-soak-autoplay.md), [every control](reviews/2026-11-06-full-control-soak.md), [stick soak](reviews/2026-11-19-stick-soak.md) |
| Music from a USB stick | exFAT stick behind a hub on the HS port, with the FLX4; MEDIA REMOVED when pulled out, read again from the start when put back, with no restart; no SD card | [stick behind a hub](reviews/2026-11-19-usb-stick-hub.md), [8 KiB reads](reviews/2026-11-19-stick-8k-reads.md), [put back](reviews/2026-11-25-stick-reinsert.md) |
| Loops and scratch | From a played-audio history, with 5.8 ms crossfades | [loops](reviews/2026-11-05-loops.md), [scratch](reviews/2026-11-05-scratch.md) |
| Beat FX and Pad FX | In the render graph, Delay and the channel switch included (s203); FX page by touch; the Overview's FX column in the effect's colour | [FX](reviews/2026-11-05-fx.md), [FX by touch](reviews/2026-11-12-fx-touch.md), [redesign](reviews/2026-11-22-redesign-on-fill-gap.md) |
| Hot Cues | From Rekordbox, own ones in a flash journal, erased ahead | [Hot Cues](reviews/2026-11-06-hot-cues.md), [erase ahead](reviews/2026-11-13-hot-cue-erase-ahead.md), [page](reviews/2026-11-12-hot-cues-page.md) |
| Memory cues | Rekordbox memory cues on the lane, the HOT CUES page and the SAMPLER pads | [memory cues](reviews/2026-11-17-memory-cues.md) |
| FLX4 LEDs | Pad modes, SYNC, PFL, Smart buttons, Beat FX ON | [LEDs](reviews/2026-11-06-controller-leds.md) |
| SYNC and Quantize | Beat sync, continuous SYNC, Quantize | [beat sync](reviews/2026-11-11-beat-sync.md), [continuous](reviews/2026-11-13-continuous-sync.md), [Quantize](reviews/2026-11-12-quantize.md) |
| Master Tempo | Its own keylock; with SYNC, phase within 1 ms | [Master Tempo](reviews/2026-11-13-master-tempo.md), [stops](reviews/2026-11-13-mt-phase.md), [controller](reviews/2026-11-14-mt-on-controller.md), [with SYNC](reviews/2026-11-16-sync-mt-line.md) |
| USB audio fill and the screen | The screen waits for gaps between fills and draws Slint's lines and the lane caches through the PPA from internal RAM; on s170, 5.5 % of Master Tempo fills over 3 ms (s163: 18.3 %), all under 5 ms | [fill contention](reviews/2026-11-21-fill-contention.md), [fill gap](reviews/2026-11-22-fill-gap.md) |
| Deck screen | Slint, with waveforms drawn by Rust; the redesign: Rust palette, icon rail, 814×220 lanes with a phase strip between them, deck cards; Library sort, BPM ±4 % and key colours; Croatian letters | [screen](reviews/2026-11-10-deck-screen.md), [palette](reviews/2026-11-21-rust-palette.md), [layout](reviews/2026-11-21-screen-layout.md), [Library filters](reviews/2026-11-21-library-filters.md), [redesign](reviews/2026-11-22-redesign-on-fill-gap.md), [letters](reviews/2026-11-13-latin-glyphs.md) |
| Screen speed | Big frames 84 → 61 ms (Library), 50 → 41 ms (Settings); the lanes and the phase strip move at every refresh (61 a second) | [draw speed](reviews/2026-11-16-screen-draw-speed.md), [UI pool](reviews/2026-11-16-ui-pool.md), [redesign](reviews/2026-11-22-redesign-on-fill-gap.md), [smooth lanes](reviews/2026-11-25-smooth-lanes.md) |
| Playlists and covers | Rekordbox's playlist tree in the Library, each deck's cover | [playlists and covers](reviews/2026-11-16-playlists-covers.md) |
| Settings | LOAD LOCK, VINYL/CDJ, brightness, MAIN trim, screensaver wait, kept on flash | [settings](reviews/2026-11-17-settings.md), [screensaver](reviews/2026-11-21-screensaver.md) |
| Controllers | The FLX4, checked again on s182 (2026-11-22). DDJ-400, DJ2GO2 Touch, S2 MK3 and Inpulse 500 profiles are chosen by VID:PID; they are untested on this hardware, and the S2 MK3 and Inpulse 500 have no audio yet | [profiles](reviews/2026-11-21-controller-profiles.md) |
| Boot screen and screensaver | The owner's wordmark, its letters lit in turn; a press only wakes it | [screensaver](reviews/2026-11-21-screensaver.md) |
| Wi-Fi and Web Remote | The deck's own network (C6 via ESP-Hosted). A console page that works upright and sideways: overviews, meters, Hot Cues 1–8, EQ knobs, mixer, library, four connections, soft takeover; a SETUP view for the deck's settings and information | [network](reviews/2026-11-15-wifi-in-the-deck-image.md), [takeover](reviews/2026-11-15-fader-takeover.md), [connections](reviews/2026-11-16-web-connections.md), [console](reviews/2026-11-22-web-remote-console.md), [SETUP](reviews/2026-11-25-web-remote-setup.md) |

## Still to come

- **The final layout:**
  - the board's own power on CN2, and the stick on the FS port. Today the FLX4 and the
    stick share a hub on the HS port. The console is already on UART0 (CN2, GPIO37/38,
    921600 baud; see the [UART0 service link](boards/JC1060_UART0_SERVICE.md));
  - with that power, the brightness cap (50 %) can rise.
- **The RTC:** the Epson RX8025T at I2C 0x32 is on the board, not yet used.
- **External audio:** see the [external audio plan](boards/JC1060_EXTERNAL_AUDIO_PLAN.md).

## Open

- **USB audio fill:** no underruns, and since the fill gap every measured Master Tempo
  fill stays under 5 ms. Some still go over the 3 ms target. Each figure belongs to its
  own image and workload:
  - **s170, profile image, gate always on** (the soak DJ, then Master Tempo on both decks,
    15 minutes; [fill gap](reviews/2026-11-22-fill-gap.md)): 5.5 % over 3 ms, longest
    4168 µs.
  - **s184p, profile image, gate limited to the frame's slot** (the first minutes of a
    run): 22 % over 3 ms, longest 4.6 ms
    ([redesign on the fill gap](reviews/2026-11-22-redesign-on-fill-gap.md)).
  - **s189p, profile image of main `fd4cbb7`** (15 minutes): 15.3 % over 3 ms, longest
    4.24 ms.
  - **s197p, profile image with smooth lanes** (8 minutes, 4 without Master Tempo and 4
    with it; [smooth lanes](reviews/2026-11-25-smooth-lanes.md)): 13.5 % over 3 ms,
    longest 4.81 ms.
- **Screen:**
  - The lanes and the phase strip move at every refresh
    ([smooth lanes](reviews/2026-11-25-smooth-lanes.md)).
  - Text drawn straight into the shown buffer can tear for one refresh.
  - Loops, zooms and the pitch fader keep the lanes moving: each lane has a spare cache
    (s199p).
  - Slint's scene preparation is still 16 to 26 ms of a big frame (Library, Settings).
- **Boot:** the FLX4 drops and enumerates again once at every boot
  (`Enqueue URB error: ESP_ERR_INVALID_STATE`); it then works.
- **SD (SD build only):** an SD clock failure at the first boot after a flash.
  - Since s132 the card is tried three times ([retry](reviews/2026-11-18-sd-retry.md)).
    This has not yet been seen working on a failing boot.
  - Since s116 the failure no longer takes the Wi-Fi's slot down with it
    ([SD slot](reviews/2026-11-16-sd-slot-teardown.md)).
- **Web Remote:**
  - SETUP covers the deck's settings; there are no network, controller profile or firmware pages.
  - OTA exists on the host only (no partition, radio or TLS adapter).
  - The network password is a build setting.

## Evidence layers

- **Physical:** the JC1060 above, on the legacy path, each with its image and its kind of
  evidence (log and counters, the owner's observation, or both).
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
