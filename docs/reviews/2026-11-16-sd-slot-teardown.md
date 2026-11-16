# JC1060: a failed SD card leaves the Wi-Fi's slot alone

Date: 2026-11-16. Branch: `ogranak/jc1060-wifi-after-sd-fail`, from `main` at `fa9c83b`.

## Problem

s115's first boot after its flash failed twice over:
- **The SD card did not take its clock** (`failed to set clk`, `ESP_ERR_TIMEOUT`), as on
  s85 before the Wi-Fi.
- **ESP-Hosted then asserted** in `sdmmc_host_get_slot_width` (`SD_HOST: invalid
  argument: null pointer`).
- **COM3 then timed out on writes** until a cold power-up.

The SD card and the C6 share one SDMMC controller:
1. ESP-Hosted creates the controller and adds its slot 1 before `app_main`.
2. The SD card then adds slot 0 to the same controller.
3. On a failure, the SD path called `sdmmc_host_deinit()`, which removes **both** slots
   and deletes the controller.
4. ESP-Hosted's card init then asked for slot 1's width; the slot was gone.

## Change

`main/sd_bridge.c` and `main/sd_readonly.c`:
- **On a failure, remove only the card's slot** with `sdmmc_host_deinit_slot(0)`. IDF
  deletes the controller only once no slot is left on it.
- **Nothing is removed when slot 0 was never added.** In a build without Wi-Fi this
  leaves an empty controller after a failure that already needs a restart.
- **The card's on-chip LDO 4 still goes.** The C6's slot uses no LDO
  (`ESP_HOSTED_SD_PWR_CTRL_LDO_INTERNAL_IO` is off).

The SD card's failure itself is not addressed here.

## Evidence

- **Host:** the IDF build has no warnings.
- **Device:** s116 (app `a7add45b…`). It is built from `main`, so it lacks
  [SYNC on the tempo's line](2026-11-16-sync-mt-line.md).
  - **Without the SD card, after a cold power-up:**
    - no assert, and the uptime kept counting;
    - the 30-second `wifi_ap` report came;
    - the owner saw the network `Ruzina-AB2D`.

    Before this change the same failure path asserted.
  - **With the card back, after a cold power-up:**
    - the SD card reads (1920 reads);
    - the soak DJ plays with underruns 0/0 and the longest fill 5.2 ms;
    - the Wi-Fi reports.

## Test plan

1. Without the SD card, power up: no crash, the network `Ruzina-AB2D` shows.
2. With the card, power up: tracks load and play, and the network shows.
