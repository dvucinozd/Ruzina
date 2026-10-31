# JC1060 `deck-audio`: first USB audio to the FLX4 headphones

Date: 2026-11-01. Branch: from main `5121121` (after PR #68). The device
run, with the owner present, is at the end. Native minimum 302 and APTA unchanged.

## What it adds

`deck-audio` is the [`deck-ui`](2026-10-31-legacy-deck-ui.md) profile plus
UAC1 playback to the controller's headphone channels. The rule that
headphones always use the controller's own UAC channels decides the route:
the tone goes to FLX4 channels 3/4, which the FLX4 profile lists as
`headphone_channels`. MAIN stays silent.

- **Rust, `controller_audio.rs`** (feature `idf-controller-audio`, using the
  shared `ruzina-uac` crate):
  - `ruzina_legacy_uac_select` parses the configuration with
    `parse_playback_formats` and picks a format with `select`, which scores
    48 kHz, then four channels, then the higher alternate. It requires four
    channels and builds the `ChannelMap` with MAIN on 1/2 and headphones on
    3/4.
  - `ruzina_legacy_uac_control` returns the `SET_INTERFACE`, `SET_CUR` 48 kHz
    and zero-bandwidth requests from `ControlRequest::build`.
  - `ruzina_legacy_uac_fill` packs whole 1 ms packets with `Cadence` and
    `pack`.
  - The source is a 1 kHz sine at −24 dBFS (amplitude 2067). It sits on the
    left headphone channel while deck 1's Play latch is on and on the right
    while deck 2's is; otherwise there is silence. The Play latch is the
    `controller_midi` echo state, so the Play LED shows when the tone runs.
- **C, `usb_host_enum.c`** (`CONFIG_RUZINA_USB_HOST_UAC`, which requires
  MIDI):
  - After the MIDI claim, it selects the format and claims the AudioStreaming
    interface at the chosen alternate.
  - It sends `SET_INTERFACE` through `usb_host_transfer_submit_control`, then
    `SET_CUR` when the endpoint advertises frequency control.
  - It then keeps four isochronous OUT transfers of six 1 ms packets in flight
    on endpoint `0x01`, refilling each completion. (Eight packets per
    transfer failed on the device; see below.)
  - It counts transfer and per-packet errors, logs `usb_uac_stats` every 5 s,
    and on detach waits for every transfer, frees them, releases the
    interface and logs `usb_uac_stop`.
- **Configuration and tools:**
  - New defaults `sdkconfig.deck-audio`, the `deck-audio` variant
    (`idf-media-load idf-controller-audio`), a mismatch check for the UAC
    option, and a link check for `ruzina_legacy_uac_fill`.
  - The 2 KiB frame limit now covers the UAC exports too.
  - The guard test gains 2 cases.
  - CI adds the audio feature to the test, clippy and target loops, and the
    variant to the matrix.

## Evidence (local)

- **Rust:** a new host test uses an FLX4-shaped configuration: 24-bit alt 1,
  16-bit alt 2 and MIDI.
  - Selection picks interface 1, alt 2, `0x01`, 384 bytes, 4 × 16-bit at
    48 kHz.
  - The exact `SET_INTERFACE`, `SET_CUR` and zero-bandwidth bytes come out.
  - Eight packets of 384 bytes carry the sine peaks ±2067 on the left
    headphone channel only, with MAIN and right silent.
  - Silence when neither deck plays, and the counters add up.
  - The MIDI test and the 39 core tests still pass. Host clippy, and target
    clippy for every feature set including `idf-media-load
    idf-controller-audio`, pass.
- **Tooling:** 25 tests pass.
- **Build:** `build_legacy.py --variant deck-audio` returns
  `BUILD_PACKAGE_PASS` with no warnings. The app is 508976 bytes, SHA-256
  `716b2f0d17fa197ad89899060ec92ed844a8709653f78f8e4a20a73b6e7a445a`.
- **Offline flash plan:** `7ee86812…`. The predecessor is the flashed
  `deck-ui` image (`07381298…`), with a chain of 20 plans. It writes
  `0x2000`, `0x8000` and `0x10000..0x8d000`.

## Device acceptance (needs the owner)

Safety: on the FLX4, Headphone Level is a MIDI control, not an analog one, so
it does not attenuate this signal. The tone is a fixed −24 dBFS. Start with
the headphones held away from the ears.

Accept the run when:
- `usb_uac interface=1 alt=2 … claim=ESP_OK` and `usb_uac
  stream=STARTED transfers=3` are logged.
- Deck 1 Play gives the tone in the left ear only, and deck 2 Play in the
  right only; Play again stops it. Nothing is heard on MAIN.
- `usb_uac_stats` shows no `iso_errors` or `packet_errors`.
- The screen, browse and load still work.
- A detach logs `usb_uac_stop freed=YES release=ESP_OK`.


## Device run (2026-11-01, owner present)

Setup:
- The PC on the FS port only, with the battery disconnected.
- The 25-track card inserted.
- The FLX4 on bus power on the HS port, with headphones on the FLX4.

Records are in `docs/boards/evidence/jc1060-20261030-com3/s06-deck-audio-*`,
indexed by the
[device index](../boards/readiness/jc1060-s06-deck-audio-device-20261101.json).

1. **First image.** `716b2f0d…` (plan `7ee86812…`) flashed with a matching
   readback.
   - The format choice and claim worked: `usb_uac interface=1 alt=2 ep=01
     mps=384 channels=4 bytes=2 rate=48000 freq_control=1 claim=ESP_OK`.
   - The `SET_INTERFACE` and `SET_CUR` chain completed.
   - Every isochronous submit then failed with `Enqueue URB error:
     ESP_ERR_INVALID_SIZE`, giving `stream=PARTIAL transfers=0`.
   - Cause: at high speed a 1 ms `bInterval` is 8 microframes, and esp-usb's
     `check_xfer_size` allows at most `XFER_LIST_LEN_ISOC −
     XFER_LIST_ISOC_MARGIN = 61` microframes per isochronous transfer. Eight
     packets need 64.
   - Nothing else failed: the screen, load and MIDI ran on, and detach freed
     everything.
2. **Fix and image `840befe9…`** (plan `b0e17fdb…`). Four transfers of six
   packets: 48 microframes each, with 24 ms still queued.
   - It logged `usb_uac stream=STARTED transfers=4 packets_per_transfer=6`.
   - The FLX4's stale first session after boot streamed 30 transfers, then
     stopped cleanly when the controller re-enumerated.
   - The second session streamed continuously.
   - **Owner check:** the owner confirmed the tone was correct, with deck 1
     Play in the left ear and deck 2 Play in the right. There were 3 Play
     presses per deck.
   - **Telemetry:** `usb_uac_stats` every 6 s up to 8000 transfers, which is
     48024 packets, 2305152 frames and 48 s at 48 kHz, with 13470 tone
     packets. `iso_errors=0` and `packet_errors=0` throughout.
   - **Load:** 11 passes ran alongside, each 25/25, with a PSRAM minimum of
     29216864 bytes, 752 bytes below `deck-ui`.

This is the first audible output of the project on JC1060. It is the
diagnostic tone only, with no deck engine, decoding or MAIN route, and the
headphone level does not apply.