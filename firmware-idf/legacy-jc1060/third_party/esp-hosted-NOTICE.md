# Vendored ESP-Hosted host (`espressif/esp_hosted` 2.11.7)

`esp-hosted-2.11.7/esp_hosted/` holds 176 files of the Espressif component registry
archive `espressif__esp_hosted-v2.11.7.zip`, unmodified:
- the root build files: `CMakeLists.txt`, `Kconfig`, `LICENSE`, `README.md`,
  `CHANGELOG.md`, `idf_component.yml` and `sdkconfig.rename`;
- `host/` and `common/`.

The archive's docs, examples, tools and C6 firmware sources (`slave/`) are left out, and
so is `common/protobuf-c/.git`, a submodule pointer git cannot track.
The owner approved the download on 2026-11-14, from
`https://components-file.espressif.com/components/espressif/esp_hosted/2.11.7/`. The
archive itself is kept outside the repository, in `D:\BoardEvidence\downloads`.

- Archive SHA-256: `447ea48f078e7260200a63614c0423dc788d7135eb5e3ba491da91ff676b34b5`
  (10057000 bytes)
- Upstream: `github.com/espressif/esp-hosted-mcu`, commit
  `f8cb41c25a6dbcc34e2c6ce29c0916994c7b7059`
- License: Apache-2.0 (the `LICENSE` file and SPDX headers)
- Manifest: `idf >=5.3`. The pinned IDF here is 6.1.0, which carries the Wi-Fi Remote
  side (`components/esp_wifi/remote`) in tree.
- Per-file SHA-256: `esp-hosted-files.json`, checked against the files when written.
- Guition's IDF 6.1 demos for this board use this same version.

Review notes:
- The build script only registers sources. It has no downloads, external processes or
  install hooks.
- **A constructor starts it at boot.**
  `host/port/esp/freertos/src/port_esp_hosted_host_init.c` calls `esp_hosted_init()`
  before `app_main`, with `ESP_ERROR_CHECK`. That init resets the C6 (GPIO54) and
  starts SDIO on slot 1 of the SDMMC host the SD card also uses (slot 0). The
  component is therefore only in the images with Wi-Fi: the deck screen image, unless built
  with `--no-wifi` (`RUZINA_WIFI=1` in the environment; `CMakeLists.txt` and
  `main/CMakeLists.txt`).
- **Its tasks start at priority 23, with no core affinity.** That is above the USB audio
  tasks (4 and 5), and the value is fixed in
  `host/port/esp/freertos/include/port_esp_hosted_host_os.h`. The probe lowers each
  task by name (`main/wifi_remote.c`).
- **The Wi-Fi images turn off three things:**
  - restart on transport failure, which would reboot the deck;
  - the hosted console, which would take over ours;
  - auto-connect on station start.

  It also leaves the SD LDO control off (`ESP_HOSTED_SD_PWR_CTRL_LDO_INTERNAL_IO`).
- **The SD error path can cut the C6 off.** On an SD failure, `sd_bridge.c` calls
  `sdmmc_host_deinit()`, which removes both slots and so cuts the C6 link too.
