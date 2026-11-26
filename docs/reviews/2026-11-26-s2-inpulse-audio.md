# JC1060: USB audio for the S2 MK3 and the Inpulse 500

Date: 2026-11-26. Branch: `ogranak/uac-diagnostics`, from `main` at `73d666b`.

## What the donor does

The stable M2.5 donor (`20f1c3f`) streams to the FLX4 only. Audio for these two
controllers came later in the donor (`a19e6841`, local checkout
`D:\Documents\DDJ-FFL4`, commits 2026-10-08 to 10-19). It ran on the M3 bench, not
on a JC1060. Neither controller needs vendor setup before audio, only standard UAC.
The S2 must be plugged in while left FLX is held, which puts it in MIDI mode.

| | Inpulse 500 (`06F8:B12B`) | S2 MK3 (`17CC:1710`) |
|---|---|---|
| Bus and class | full speed, UAC1 | high speed, UAC2 |
| Output format | four channels, 24-bit packed in 3 bytes, 44.1 kHz only | four channels, 24-bit in 4-byte subslots, 48 kHz or 44.1 kHz (clock source `0x11`) |
| OUT endpoint | `0x01`, asynchronous, 540 bytes, every 1 ms | `0x01`, asynchronous, 112 bytes, every 125 µs |
| Feedback | IN `0x81`, 3 bytes (10.14), refresh every 32 ms, empty replies between refreshes | IN `0x81`, 4 bytes (16.16), every 1 ms |
| Setup | SET_INTERFACE only (no frequency control) | clock RANGE, then SET_CUR, then readback (retried up to 500 ms), then validity, then SET_INTERFACE |
| Outputs | MAIN 1/2. Headphones 3/4 take the pre-fader cue at unity, because the hardware mixes and levels them (`native_headphone_controls`) | MAIN 1/2, software headphone mix on 3/4 |
| Quirks | 540 bytes exceeds the HS controller's default 512-byte periodic TX FIFO, so the claim fails; the donor patched it to 640 bytes. Being a full-speed device, it must sit on the root port (esp-usb has no TT) | eight zero bytes end the configuration inside `wTotalLength`; ISO loss and one bus drop were seen on the M3, cause unproven |

## What Ruzina lacks

- **Inpulse 500**
  - The parser refuses asynchronous alternates and feedback endpoints.
  - The C driver has no feedback IN transfer, and packets come from a fixed
    1 ms cadence.
  - The periodic TX FIFO is 512 bytes (balanced bias).
  - `deck_mix` is silent at any rate but 48 kHz. The `Resampler` exists but is
    not wired in.
  - The profile's native-headphones flag is parsed but not used.
- **S2 MK3**
  - The zero tail and the 6-byte UAC2 Format Type make the whole configuration
    `Malformed`.
  - Not implemented yet: the UAC2 parser, clock control, 125 µs packets with
    16.16 feedback, and 24-in-32 packing. The C driver's control buffer (64
    bytes) cannot hold a clock RANGE reply.

## Plan (one PR each)

The owner confirmed this order on 2026-11-26. The Inpulse 500 goes directly on
the HS port, with no hub.

1. Diagnostics and the donor's captured descriptors (this PR).
2. 48 → 44.1 kHz resampling in the packet fill.
3. Periodic TX FIFO for a 540-byte MPS.
4. UAC1 explicit feedback (Inpulse 500).
5. Native headphones (Inpulse 500).
6. S2 zero tail and the UAC2 parser.
7. UAC2 clock control.
8. 125 µs high-speed packets with 24-in-32 packing (S2 MK3).

Neither controller is at hand. Steps 1 to 3 are checked with the FLX4.
Steps 4 to 8 are host-tested until the controllers come.

## This change

- **`ruzina-uac`:**
  - `choose()` is `select()` on a configuration, or a `Rejection` that names
    why nothing was offered: descriptor error and offset, no streaming,
    format, feedback, interval, rate or packet size, with interface and
    alternate.
  - `select()` alone still decides.
- **Fixtures:** the donor's two captures are in
  `firmware-rust/crates/ruzina-uac/tests/fixtures`, and
  `tests/donor_descriptors.rs` records what Ruzina does with them today:
  - The Inpulse 500 parses to the format above and is refused for its feedback
    (interface 1, alternate 1).
  - The S2 MK3 is `Malformed` at its UAC2 Format Type.
- **Firmware:**
  - `select=NONE` now logs its reason, for example
    `usb_uac select=NONE reason=FEEDBACK interface=1 alt=1`.
  - A refused interface claim logs its error, instead of nothing:
    `usb_uac interface=… alt=… ep=… mps=… claim=…`.
  - Nothing that streams changes.
