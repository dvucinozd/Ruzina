# JC1060 UART0 service link (console and flashing without USB Serial/JTAG)

Date: 2026-11-13. Board: Guition JC1060P470C_I_W_Y. Schematic: `JC1060P470C_I_W_Y-V1.0.pdf`
(SHA-256 `ae3f41fe…`, see [the USB topology](JC1060_USB_TOPOLOGY.md)). Measured on the
owner's unit.

## Why

The plan is:
- the USB stick on the FS Type-C port;
- the FLX4 on the HS Type-C port;
- power on the board's 4-pin "UART" connector.

The FS PHY cannot serve USB Serial/JTAG (COM3) and an OTG host at once. So the console
and flashing move to UART0, before the stick takes the FS port.

## Connectors (vendor photo numbers)

| No. | Vendor label | Schematic | Pins (1 to 4) | Use |
|---|---|---|---|---|
| 10 | MX 1.25 4P UART | CN2 | VIN, TX0 (GPIO37), RX0 (GPIO38), GND | **Service link and future power input** |
| 8 | MX 1.25 4P UART1 | J5 | `USB5V_IN`, TX1 (GPIO26), RX1 (GPIO27), GND | Not used |
| 11 | MX 1.25 4P RS-485 | J4 | `USB5V_IN`, A, B, GND | Not used |

Notes:
- **CN2:**
  - VIN reaches `USB5V_IN` through Q2, which conducts both ways.
  - With the PC on FS, the owner measured 5 V on pin 1 and 3.3 V on pin 2 (TX0 idle
    in the ROM).
- **J5:**
  - Pin 1 is `USB5V_IN` with no MOSFET or fuse.
  - RX1 and TX1 are shared with the MAX485 through 0 Ω links: the receiver drives RX1.
  - The ROM does not download over UART1.
  - The owner measured 5 V on pin 1 and about 0.5 V on both signals (undriven).

## Wiring

A 3.3 V USB-UART adapter: a CP210x, which is COM8 on the owner's PC.

| CN2 pin | Adapter |
|---|---|
| 1 VIN | nothing, while the PC powers the board over FS |
| 2 TX0 | RXD |
| 3 RX0 | TXD |
| 4 GND | GND |

**Never two live 5 V sources on `USB5V_IN`.** These are:
- the PC on FS;
- anything sourcing on HS;
- VIN on CN2;
- pin 1 of J4 or J5.

## Firmware console

`sdkconfig.defaults` makes UART0 the console:
- custom UART0 on GPIO37/38, at **921600 baud**;
- USB Serial/JTAG is a secondary console, output only.

What follows:
- **COM8 (UART0)** carries the bootloader and the app log, and takes console input. The
  operator's `R` restart now works only here.
- **COM3** still gets a copy of the app's output while it is connected, but not the
  bootloader's lines, and it takes no input.

## Flashing over UART0

There is no reset line on CN2. To enter the ROM:
1. hold BOOT;
2. press and release Reset;
3. release BOOT.

Then, from `D:\BoardEvidence\JC1060-20261030-COM3`:

```
toolchain\python-idf61\Scripts\python.exe uart0-flash.py <package dir> <new out dir> <app sha256> [baud]
```

What the script does:
1. It backs up every span it erases, checking each copy against the MD5 the chip
   computes.
2. It writes at 460800 baud by default.
3. It checks every span by the chip's MD5.
4. It leaves the chip in the flasher; Reset starts the app.

At 921600 baud, and on large reads at 460800, this adapter and cable lost bytes. Reads
back are therefore checked by MD5 on the chip, not transferred. Compressed writes at
460800 verified on the first try.

Capture over UART0: `s06-usb-host-monitor.py --mode passive --port COM8 --baud 921600 …`,
then press Reset. `--mode boot` needs COM3.

## Evidence (2026-11-13)

- **eFuse (read over COM3):** `UART_PRINT_CONTROL=0` (always print),
  `DIS_DOWNLOAD_MODE=False`, `DIS_FORCE_DOWNLOAD=False`.
- **ROM over COM8:** the stub ran and read MAC `e8:f6:0a:e7:22:1d`, as over COM3.
- **s89** (app `1f239487…`, console on UART0):
  - **Flashed over COM3:** COM8 and COM3 then captured the same 156 app lines; COM8 also
    had the bootloader's.
  - **Flashed again over COM8 at 460800:** all three images were written, each with
    `Hash of data verified`.
  - **After that flash:** it booted from UART0, the FLX4 attached, and audio ran:
    underruns 0/0, longest fill 3.9 ms.
- **Getting there:** the first attempts failed on wiring and on an adapter that kept
  dropping off USB. A loopback test proved the adapter. CN2 was identified by its
  3.3 V TX0 in the ROM.

## Next

- Power from CN2 VIN (5 V, at least 2 A) with the PC off the FS port.
- Then the USB stick on FS, as an OTG FS host.
