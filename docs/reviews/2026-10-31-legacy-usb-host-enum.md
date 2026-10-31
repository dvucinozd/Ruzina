# JC1060 `usb-host` enumeration diagnostic

Date: 2026-10-31. Branch: from main `926e905` (after PR #64). Build only: no
device run yet. Native minimum 302 and APTA unchanged.

## Topology first

[`JC1060_USB_TOPOLOGY.md`](../boards/JC1060_USB_TOPOLOGY.md) reads the vendor
schematic, rendered locally, and leads to three choices:
- **Host port.** The HS Type-C connector (`ESP_USB_P/N`, UTMI PHY) is the only
  host port that keeps the FS Serial/JTAG console.
- **Power.** Both connectors' VBUS tie directly to `USB5V_IN`, with no switch,
  limit or Rp. That sets owner-applied safety rules against backfeed and limits
  controller power to the PC's port.
- **Controller.** The owner's DDJ-FLX4 connects C-to-C to the HS port. Its
  power-only port is used only after a backfeed measurement.

## USB Host Library

IDF 6.1 no longer ships the host library. With the owner's approval,
`espressif/usb` 1.6.0 was downloaded from the component registry, reviewed
and vendored byte-identically in `third_party/esp-usb-1.6.0/usb`:
- Apache-2.0;
- archive SHA-256 `dad84174…`;
- upstream `630f1c4e`.

Provenance is in `third_party/esp-usb-NOTICE.md`.

- The project lists it in `EXTRA_COMPONENT_DIRS`, and `main` requires `usb` in
  every profile, as it requires PSRAM, so its Kconfig is always present.
- It has no startup hook, so other profiles link none of it. The build tool
  now fails any other variant that defines `usb_host_install`.
- `build_legacy.py` sets `IDF_COMPONENT_MANAGER=0`, because the manager would
  otherwise resolve the vendored manifest and write `dependencies.lock` into
  the sources.

## Profile

`CONFIG_RUZINA_USB_HOST_ENUM`, variant `usb-host`, is isolated and
flash-backed. The CMake guard (with a 7-case test) rejects it together with SD,
display, memory/task, pure RAM or PSRAM.

`main/usb_host_enum.c`:
- installs the host on the default HS peripheral with the internal UTMI PHY;
- runs the library and client tasks;
- for each new device, logs the device descriptor and strings, then walks the
  whole configuration descriptor (IADs, interfaces, endpoints and a count of
  class-specific descriptors);
- on a gone event, closes the device and logs the detach.

It claims no interface and issues no class transfer.

## Evidence (local)

- 24 tooling tests pass, including the new guard test.
- `build_legacy.py --variant usb-host` returns `BUILD_PACKAGE_PASS` with no
  warnings. The app is 201248 bytes, SHA-256
  `2f0c0ab796f81f5eea60ae600ffc8523dde2f8c0d7ceb6aac914ed37260817ec`, built
  from the dirty tree on `926e905`.
- `sd-rust-load-display` still returns `BUILD_PACKAGE_PASS` and does not link
  the library.
- **Offline flash plan:** `60403d54…`. The predecessor is the flashed
  touch-selection image (`8881509f…`), with a chain of 12 plans. It writes
  `0x2000`, `0x8000` and `0x10000..0x42000`. Restoring the predecessor brings
  back the display and load profile.

## Device acceptance (needs the owner)

Setup:
- The board is powered only from the PC on the FS (COM3) port, with the
  battery disconnected.
- Nothing that sources power goes on the HS port.
- The FLX4 runs on bus power first.

Accept the run when:
- `S6_USB_HOST` starts.
- The FLX4 attaches. The log records its speed, VID/PID, the audio and MIDI
  interfaces and endpoints, and a complete configuration walk.
- Unplugging gives `usb_detach`, and replugging enumerates again, three times.
- There is no brownout, reset or fault, and the heartbeat continues.
