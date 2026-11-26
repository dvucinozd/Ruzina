# Captured configuration descriptors

Whole configuration descriptors (`wTotalLength` bytes) that the donor project
captured from real controllers. Each `.bin` holds the bytes of the donor's C
fixture array, unchanged.

| File | Controller | Speed | Donor source (`D:\Documents\DDJ-FFL4`, commit `a19e6841`) |
|---|---|---|---|
| `hercules_inpulse_500_fs.bin` | Hercules DJControl Inpulse 500, `06F8:B12B` | full speed | `tests/controller_usb_audio/fixtures/hercules_inpulse_500_fs_config.h`, captured 2026-10-10 on the M3 bench |
| `s2_mk3_hs.bin` | NI Traktor Kontrol S2 MK3, `17CC:1710`, bcdDevice 0037, MIDI mode | high speed | `tests/controller_usb_audio/fixtures/s2_mk3_hs_config.h`, captured 2026-10-08 on the M3 bench |

What they contain:

- **Inpulse 500**: UAC1. Interface 1 alternate 1 has four channels of 24-bit PCM
  packed in 3 bytes, 44.1 kHz only. Its OUT endpoint `0x01` is asynchronous, with
  a 540-byte max packet. Explicit 3-byte feedback comes on IN `0x81`, with a
  refresh exponent of 5. It has no sampling-frequency control. MIDI is on
  interface 3.
- **S2 MK3**: UAC2. Clock source `0x11` is internal and programmable.
  Interface 1 alternate 1 has four channels of 24-bit audio in 4-byte subslots.
  Its OUT endpoint `0x01` is asynchronous, with a 112-byte max packet sent every
  microframe. Feedback is 4 bytes on IN `0x81`, every 1 ms. MIDI is on
  interface 6. The last eight bytes inside `wTotalLength` are zero.
