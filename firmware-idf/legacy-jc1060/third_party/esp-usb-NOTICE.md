# Vendored USB Host Library (`espressif/usb` 1.6.0)

`esp-usb-1.6.0/usb/` holds all 28 files of the Espressif component registry
archive `espressif__usb-v1.6.0.zip`, unmodified. The archive was downloaded on
2026-10-31 with the owner's approval from
`https://components-file.espressif.com/components/espressif/usb/1.6.0/`.

- Archive SHA-256: `dad8417428e553efbd886dad0397b84069352114c8eb21c102eb19b3ee26e4d7`
  (162303 bytes)
- Registry `component_hash`: `99eee72ea224d9117d4c342442717a1f20bf8112e2ab5d7d7038b827874f4c92`
- Upstream: `github.com/espressif/esp-usb`, path `host/usb`, commit
  `630f1c4e61f71d62bb57ff1b840931da24c3329f`
- License: Apache-2.0 (the `LICENSE` file and SPDX headers in every source)
- Manifest: `idf >=5.5.3`. The pinned IDF here is 6.1.0.

Review notes:
- The build script only registers sources. It has no downloads, external
  processes or install hooks.
- No source registers a startup hook (`ESP_SYSTEM_INIT_FN` or a constructor),
  so profiles that do not call the library link none of it.
- For IDF 6.x it uses the in-tree `esp_hal_usb` and `esp_hw_support` USB PHY
  driver, which has no ESP32-P4 revision gate.

The project lists this directory in `EXTRA_COMPONENT_DIRS`. Only the USB host
diagnostic requires it. `.gitattributes` keeps the files byte-identical, and
the build manifest hashes them with the other sources. There is no managed or
wildcard registry dependency.
