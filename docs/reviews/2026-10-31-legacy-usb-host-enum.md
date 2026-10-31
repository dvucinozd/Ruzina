# JC1060 `usb-host` enumeration diagnostic

Date: 2026-10-31. Branch: from main `926e905` (after PR #64). The device
run, with the owner present, is at the end. Native minimum 302 and APTA unchanged.

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


## Device run (2026-10-31, owner present)

Setup:
- The board was powered only from the PC on the FS (COM3) port, with the
  battery disconnected.
- The owner's DDJ-FLX4 was on bus power, connected C-to-C to the HS port.

Records are in `docs/boards/evidence/jc1060-20261030-com3/s06-usb-host-*`,
indexed by the
[device index](../boards/readiness/jc1060-s06-usb-host-device-20261031.json).

1. **First image.** `2f0c0ab7…` (plan `60403d54…`) flashed with a matching
   readback, and `S6_USB_HOST` started. The owner then power-cycled the board,
   and the heartbeat restart in the capture is that cycle. After an app
   restart with the FLX4 attached, the library reached enumeration and failed
   with `ENUM: Configuration descriptor larger than control transfer max
   length`.
2. **Fix.** `CONFIG_USB_HOST_CONTROL_TRANSFER_MAX_SIZE` went from 256 to 4096,
   and the build tool now requires it for `usb-host`. Image `0133e421…`
   (plan `ed7ae4c0…`, predecessor `60403d54…`) flashed with a matching
   readback.
3. **Stuck controller.** After the failed attempt, the FLX4 did not re-attach
   across a ROM boot or an app restart: there was no attach and no error. One
   cable replug enumerated it.
4. **Enumeration.**
   - The device: `usb_attach address=1 speed=HIGH vid=2b73 pid=0045
     total=451 interfaces=7 max_power_ma=500`, manufacturer "AlphaTheta
     Corporation", product "DDJ-FLX4". The configuration walk completed with
     11 interfaces including alternates, 10 endpoints and 25 class-specific
     descriptors.
   - **Audio:** the IAD 0..2 audio function has its playback endpoint at
     isochronous synchronous `0x01`. Alt 1 is 576 B per 1 ms (4 ch × 24-bit
     × 48 kHz); alt 2 is 384 B (4 ch × 16-bit). Capture is `0x81`, 288 or
     192 B (2 ch).
   - **MIDI:** streaming on interface 4 uses bulk `0x82` in and `0x03` out,
     512 B.
   - **Other interfaces:** HID on interface 5 with interrupt `0x84`/`0x05`,
     and a vendor interface 6 with bulk `0x86`/`0x07`.
5. **Hot-plug.** 3 detach/attach cycles each logged `usb_detach` and then a
   new attach at addresses 2, 3 and 4. Each was high speed with the same
   451-byte configuration and a complete walk. There were 0 errors and no
   fault, and the heartbeat stayed continuous from 141 to 275, so nothing
   reset on attach.

The FLX4's 4-channel playback interface carries MAIN and the headphone pair,
which is the controller headphone route the product requires. The next step
is claiming the MIDI interface, then UAC playback on alt 1 or 2.

Not covered:
- the FLX4's power-only port and its backfeed measurement;
- current measurement;
- a cold boot with the controller attached.