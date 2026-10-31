# JC1060 `deck-audio`: first USB audio to the FLX4 headphones

Date: 2026-11-01. Branch: from main `5121121` (after PR #68). Build only: no
device run yet. Native minimum 302 and APTA unchanged.

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
  - It then keeps three isochronous OUT transfers of eight 1 ms packets in
    flight on endpoint `0x01`, refilling each completion.
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
