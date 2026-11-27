# Vendored ESP-Hosted host (`espressif/esp_hosted` 2.12.11)

`esp-hosted-2.12.11/esp_hosted/` holds 183 files of the Espressif component registry
archive `espressif__esp_hosted-v2.12.11.zip`, unmodified:
- the root build files: `CMakeLists.txt`, `Kconfig`, `LICENSE`, `README.md`,
  `CHANGELOG.md`, `idf_component.yml` and `sdkconfig.rename`;
- `host/` and `common/`.

The archive's docs, examples, tools, CI config and C6 firmware sources (`slave/`) are left
out, and so is `common/protobuf-c/.git`, a submodule pointer git cannot track.
The owner approved the download on 2026-11-27, from
`https://components-file.espressif.com/components/espressif/esp_hosted/2.12.11/`. The
archive itself is kept outside the repository, in `D:\BoardEvidence\downloads`.

- Archive SHA-256: `cc8b0c9fef20339242cec79de841b1012132f4134dfd377a5daa3cc03a24b02f`
  (10073069 bytes)
- Upstream: `github.com/espressif/esp-hosted-mcu`, commit
  `da7412f9b5a31b54bd57acb140247e95d54c7eed`
- License: Apache-2.0 (the `LICENSE` file and SPDX headers)
- Manifest: `idf >=5.3`. The pinned IDF here is 6.1.0, which carries the Wi-Fi Remote
  side (`components/esp_wifi/remote`) in tree.
- Per-file SHA-256: `esp-hosted-files.json`, checked against the files when written.
- It matches the C6, which the owner flashed with ESP-Hosted 2.12.11 on 2026-11-27.
  The host it replaces, 2.11.7, logged a version mismatch at every boot.

Review notes:
- The build script only registers sources. It has no downloads, external processes or
  install hooks. Since 2.12 it also lists IDF's `openthread` component as a private
  requirement; OpenThread itself stays off (`ESP_HOSTED_HOST_OT_ENABLE` defaults to n).
  An `ESP_HOSTED_CI_PEDANTIC` environment variable would add `-Werror`; we never set it.
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
- **The Wi-Fi images turn off four things:**
  - restart on transport failure, which would reboot the deck;
  - the hosted console, which would take over ours;
  - auto-connect on station start;
  - its SDIO buffer pool (`ESP_HOSTED_USE_MEMPOOL`). Since 2.12 the pool takes about
    48 KiB of internal DMA RAM in the boot constructor, before IDF adds the 384 KiB
    startup region to the heap. On s243 that failed and the image asserted
    (`sdio_drv.c`, `buf_mp_g`) and rolled back. Without the pool, each buffer comes
    from the heap when needed.

  It also leaves the SD LDO control off (`ESP_HOSTED_SD_PWR_CTRL_LDO_INTERNAL_IO`).
- **The SD error path can cut the C6 off.** On an SD failure, `sd_bridge.c` calls
  `sdmmc_host_deinit()`, which removes both slots and so cuts the C6 link too.
