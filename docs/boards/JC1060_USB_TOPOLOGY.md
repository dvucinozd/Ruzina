# JC1060 USB host topology (offline, from the vendor schematic)

Date: 2026-10-31. Source: vendor schematic `JC1060P470C_I_W_Y-V1.0.pdf`
(SHA-256 `ae3f41fe9a66b201358ef8cf1f531fdced68c63f84c8bde36e23cde5132a23ee`),
one A-size sheet, rendered locally with poppler 24.02. The rendered crops are
kept outside the repository in
`D:\BoardEvidence\JC1060-20261030-COM3\usb-topology-20261031`. No device access.
This is a schematic reading, not a physical measurement of the received unit.

## Connectors and PHYs

| Connector | Data nets | Module pins (JC-ESP32P4-M3) | P4 block |
|---|---|---|---|
| `USB3` "High speed USB", Type-C | `ESP_USB_P/N` (A6/A7, B6/B7), TVS D6/D7 | 40 `ESP_USB_P`, 39 `ESP_USB_N` | USB 2.0 OTG HS with the UTMI PHY |
| `USB2` "Full Speed USB", Type-C | `USB1P1_P/N`, TVS D4/D5 | 43 `USB1_P1_P`, 42 `USB1_P1_N` | the single FS/LS PHY (`SOC_USB_FSLS_PHY_NUM` 1) |

- The FS connector is the USB Serial/JTAG link used for COM3, the console and
  all flashing so far. The FS PHY cannot serve both Serial/JTAG and OTG FS at
  once. Using that port for a controller would remove the console.
- **The HS connector is the only host port that keeps the console.** The HS
  OTG controller also serves full- and low-speed devices, which covers typical
  DJ controllers.
- The native firmware maps USB0 to MSC media and USB1 to the controller. On
  JC1060 the controller goes on the HS port instead, and media stay on SD.

## VBUS and power

- **No VBUS switching or limit.** On both connectors, all four VBUS pins (A4,
  A9, B4, B9) tie directly to one net, `USB5V_IN`. There is no load switch,
  diode, current limit, fault flag or GPIO enable.
- **Sink-only CC.** CC1 and CC2 on each connector have only a 5.1 kΩ pull-down
  (R62/R63 on HS, R54/R56 on FS). Both ports present as sinks (Rd); nothing
  presents Rp or negotiates a source role.
- **Sources of `USB5V_IN`:**
  - either connector's VBUS;
  - `VIN` on CN2 (MX1.25 4-pin, with UART0) through the P-MOSFET Q2 (AO3401,
    gate to GND), clamped by the SMBJ6.0CA D2. With the gate grounded, Q2
    conducts both ways once on, so `USB5V_IN` also appears on the CN2 VIN pin.
- **Board supply:** `USB5V_IN` feeds only the IP5306 (U5) VIN, a charger with a
  power path. Its `VOUT-BAT` goes through R22 to `VCC5V` and the TLV62569 3.3 V
  buck, which powers the board.
- **Battery only:** the IP5306 boosts the battery to `VOUT-BAT` but does not
  drive its VIN. `USB5V_IN`, and so the controller's VBUS, then stays
  unpowered.

## Consequences for the USB host bring-up

1. **Controller power.** A bus-powered controller on the HS port is powered
   only while another source drives `USB5V_IN`, which in practice is the PC on
   the FS (COM3) port. The PC port then carries the board, display backlight,
   controller and any IP5306 charge current, with no limit on the board. A
   brownout or PC port overcurrent is possible with a high-draw controller.
2. **Backfeed.** Any two live 5 V sources on `USB5V_IN` are shorted together:
   - a PC on FS plus a host, charger or power bank on HS;
   - a PC plus `VIN`.

   Because the board shows Rd, a Type-C source or charger on the HS port will
   enable its VBUS into the PC's VBUS. This must never be connected.
3. **Cables.** The board does no CC logic, so VBUS is present whenever
   `USB5V_IN` is.
   - A USB-C (male) to USB-B cable or a C-to-A female OTG adapter works for a
     USB-B or USB-A device.
   - A Type-C controller on a C-to-C cable that expects Rp from the host may
     not attach.
4. **Software stack.** IDF 6.1 has the USB OTG HAL and UTMI PHY support for
   ESP32-P4, but no USB host library. That moved to the `espressif/usb`
   registry component. A diagnostic profile needs an exactly pinned and
   reviewed copy of it, because the build forbids wildcard registry
   dependencies.

## Owner's controller: Pioneer DDJ-FLX4

The owner will use a DDJ-FLX4. The owner reports that it is powered over
USB-C. Its data port is USB-C, so the connection is C-to-C to the board's HS
port. That port shows only Rd and never Rp; it attaches only through the
hard-wired VBUS. Whether the FLX4 enumerates without seeing Rp is part of the
first run.

The FLX4 also has a separate USB-C power-only port. Before any run that feeds
that port from a charger, the owner measures with a multimeter, with only the
power port supplied and the data cable unplugged, whether 5 V appears on the
data port's VBUS. If it does, a charger on the FLX4 would backfeed through the
board into the PC, and that setup is not used. The first run uses bus power
from the PC only, with the battery disconnected.

That run (2026-10-31) enumerated the FLX4 at high speed with C-to-C and no Rp.
It then passed three hot-plug cycles with no reset on attach. See the
[enumeration review](../reviews/2026-10-31-legacy-usb-host-enum.md#device-run-2026-10-31-owner-present).

## Test-safety rules for the first host run (owner-applied)

- Power the board only from the PC on the FS (COM3) port. Nothing that can
  source power goes on the HS port: no PC, charger, power bank or powered hub.
  `VIN` is not used.
- Disconnect the battery during host tests, so IP5306 charging does not add to
  the PC port load.
- Prefer a controller with its own power supply. With a bus-powered one,
  record its rated current first and watch for brownout and reset reasons.
- These rules are product-relevant. The board cannot itself offer a qualified,
  current-limited host VBUS, so any production use needs external power
  management.
