# JC1060: the ESP32-C6 link, a first probe

Date: 2026-11-14. Branch: `ogranak/jc1060-c6-wifi`, from `main` at `40b7a87`.

## Goal

The Web Remote (`firmware-rust/crates/ruzina-web`) needs Wi-Fi, and the P4 has no radio.
The board's ESP32-C6 is wired to the P4 over SDIO. Until now the firmware never touched
it. This first step only brings the link up and scans, while the deck plays. It joins no
network and runs no web server.

## What the board has

Taken from Guition's IDF 6.1 demo for this board (`esp_brookesia_phone`), and confirmed
on the board:

| Signal | P4 GPIO |
|---|---|
| CMD | 19 |
| CLK | 18 |
| D0, D1, D2, D3 | 14, 15, 16, 17 |
| C6 reset (active high) | 54 |

- **Bus:** SDIO on slot 1 of the SDMMC host, 4 lines, 40 MHz.
- **Shared host:** the SD card uses slot 0 of the same host, on GPIO39 to 44.
- **C6 firmware:** ESP-Hosted 2.3.0. Guition ships `JC-C6-slave_v2.3.2.bin` in its
  board package.

## Change

- **ESP-Hosted host 2.11.7 is vendored**, the version Guition's demos use
  ([notice](../../firmware-idf/legacy-jc1060/third_party/esp-hosted-NOTICE.md)). Only
  the build files, `host/` and `common/` are kept. IDF 6.1 already carries the Wi-Fi
  Remote side in `esp_wifi/remote`.
- **`build_legacy.py --wifi-probe`** (deck-engine only) adds
  `sdkconfig.wifi-probe.defaults` and puts the component in the build.
  - ESP-Hosted's constructor resets the C6 and starts SDIO in any image that links it,
    so no other image has it.
  - The probe turns off restart on link failure, the hosted console and auto-connect.
  - lwIP's task runs on core 1 at priority 2.
- **`main/wifi_probe.c`** starts last, once USB audio runs:
  - it reads the C6 firmware version and starts Wi-Fi as a station;
  - it scans every 30 s, and prints only counts, signal and channels, never network
    names.
- **ESP-Hosted's tasks** start at priority 23 with no core affinity, above the USB audio
  tasks (4 and 5), and that value is fixed in its source. The probe lowers each one it
  finds to 2.

## Evidence

- **Host:** the vendored files match the archive's hashes
  (`third_party/esp-hosted-files.json`). The pinned MP3 vendor check passes. The IDF
  build has no new warnings.
  - The image grew by 330 KB, to 2.77 MB in the 12 MB app partition.
  - Static internal RAM used: 137 KB of 572 KB.
- **Device:** s101 (app `28972f9f…`), 15 minutes with the soak DJ, owner verdict "Zvuk je
  čist".
  - **Link:** up 4.2 s after boot. The C6 identified itself as esp32c6, with Wi-Fi and
    BLE.
  - **Scans:** 28, all successful, about 2.5 s each, finding 7 or 8 networks.
  - **Audio:** underruns 0/0, `packet_errors=0`, longest fill 6.1 ms, 14 Hot Cue saves.
  - **USB task free stack:** 2760 bytes, against 2832.
  - **Internal RAM:** 268 KB free, against 337 KB without Wi-Fi.
  - **Hosted tasks:** 6 of 7 were found and lowered. `rpc_supp_cb` only starts when it is
    needed.

## Open

- **The C6 firmware is 2.3.0 against a 2.11 host.** ESP-Hosted warns that RPC calls may
  time out until the C6 is upgraded. The archive holds the C6 firmware sources, and
  ESP-Hosted can update the C6 over the SDIO link.
- **The signal of 0 on s101 was the probe's fault** (Codex review). ESP-Hosted's
  `rpc_wifi_scan_get_ap_records` copies the records but leaves the count at the
  caller's capacity, so the probe read empty records too, with a signal and a channel of
  0. The probe now reads only the networks the scan found. On s102 (app `1dd745f5…`) the
  scans found 7 to 9 networks, with a best signal of −46 to −50 dBm on channels 1, 2,
  6, 7 and 11. Audio stayed clean, and the boot had no USB host error this time.
- **The probe asked for the C6 version too early.** It asked before the link was up
  (`ESP_FAIL`); the version above comes from ESP-Hosted's own log line.
- **One USB host error at boot on s101:** `Enqueue URB error: ESP_ERR_INVALID_STATE`, at
  2.8 s. s100 and s102 had none. Audio started normally after it.
- **An SD error cuts the C6 link.** `sd_bridge.c`'s error path calls
  `sdmmc_host_deinit()`, which removes both slots.
- **Next steps:** joining a network (credentials from the owner, kept out of the
  repository), then the Web Remote.
