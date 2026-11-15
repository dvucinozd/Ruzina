# JC1060: Wi-Fi and the Web Remote in the deck image

Date: 2026-11-15. Branch: `ogranak/jc1060-wifi-default`, after
[the mixer, SYNC and seeks](2026-11-15-web-remote-mixer.md).

## Change

**The deck screen image now carries the deck's Wi-Fi and the Web Remote.** Before, they
came only with `--wifi-probe`.

- **Build:** `build_legacy.py --variant deck-engine --slint-ui` builds them in, and
  `--no-wifi` leaves them out.
- **Names:** the probe's names are gone.

  | Before | Now |
  |---|---|
  | `main/wifi_probe.c` | `main/wifi_remote.c` |
  | `sdkconfig.wifi-probe.defaults` | `sdkconfig.wifi.defaults` |
  | `CONFIG_RUZINA_WIFI_PROBE` | `CONFIG_RUZINA_WIFI_REMOTE` |
  | `RUZINA_WIFI_PROBE` (environment) | `RUZINA_WIFI` |
  | `ruzina_wifi_probe_start` | `ruzina_wifi_start` |
  | `wifi_probe ...` log lines | `wifi ...` |

  The manifest records `wifi`.
- **CI:** the legacy workflow tests and clippies the Rust core with `idf-web-remote`. Its
  target loop also builds the deck screen image with it, with and without the fill
  profile. The CI images built with `--slint-ui` now include Wi-Fi.

## The USB host error at boot

`Enqueue URB error: ESP_ERR_INVALID_STATE` appeared on some boots with Wi-Fi (s101,
s104, s105, s109), so it looked like ESP-Hosted's doing. It is not.

- The FLX4 drops once and enumerates again, on every boot in the captures from s95 to
  s109, with or without Wi-Fi. That is `usb_detach n=1`, then a second attach.
- The message is the MIDI IN transfer queued just as that drop happens. ESP-Hosted's C6
  reset only moves the timing.
- Audio started normally after it every time.

The drop itself predates the Wi-Fi and stays open.

## SD and the C6

The SD card (slot 0) and the C6 (slot 1) share the SDMMC host. Across s101 to s109, with
scans, a phone polling every second and loads, there were no underruns, and the decoders
kept their rings full.

## Evidence

- **Host:** the legacy CI matrix, the Slint checks, the Web Remote tests and target
  clippy pass, including the two new combinations with `idf-web-remote`.
- **IDF builds:**

  | Build | Result |
  |---|---|
  | `--slint-ui` | 2.83 MB, Wi-Fi, no warnings (s110, app `e0ac39b5…`) |
  | `--slint-ui --no-wifi` | 2,442,784 bytes, no ESP-Hosted: the same size as s100, before Wi-Fi |
  | `--slint-ui --uac-profile` | 2.84 MB, Wi-Fi, no warnings (the CI profile image) |
- **Device:** s110 (app `e0ac39b5…`), built with no Wi-Fi flag. Owner verdict: "radi".
  - Wi-Fi came up as before: `Ruzina-AB2D`, WPA2, Web Remote listening, C6 firmware
    2.3.2.
  - The log lines carry the new names.
  - The phone's page made 89 requests, the longest 85 ms.
  - Underruns 0/0, `packet_errors=0`, longest fill 6.1 ms.

## Open

- The FLX4's drop and second enumeration at every boot.
- The rest as in [the mixer review](2026-11-15-web-remote-mixer.md#open): fader
  takeover, and one connection at a time.
