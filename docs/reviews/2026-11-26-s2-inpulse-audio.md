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

## Step 2: 48 → 44.1 kHz in the fill

Branch `ogranak/uac-44k1`, from `main` at `a090534`.

- **`deck_mix`:** a packet for a device at 44.1 kHz now carries the mix. Before
  this change it was silence.
  - The packet takes exactly the 48 kHz render frames its 44 or 45 output frames
    need (`Resampler::input_needed`). MAIN and headphones are converted together,
    in one four-channel frame, so they share the phase.
  - Render work per fill stays the same: 6 ms of audio is still 288 render frames.
  - The resampler and its two 64-frame buffers live in `Mix`, not on the USB
    callback's stack.
  - It is dropped at 48 kHz and when the media is muted.
  - Any rate other than 44.1 or 48 kHz still gets silence.
- **IRAM:** `--fill-iram` also moves `ruzina_uac::resample` to internal RAM.
- **Board check without an Inpulse 500:**
  - `build_legacy.py --uac-44k1` (Cargo feature `uac-44k1`, deck-engine only)
    makes the deck try 44.1 kHz first. The FLX4 offers it, so the FLX4 then
    streams through the conversion.
  - `ruzina-uac` gains `select_preferring` and `choose_preferring` for this;
    `select` and `choose` still put 48 kHz first.
  - Play images are built without the flag.
- **Tests:**
  - The `deck_mix` test plays 200 packets at 44.1 kHz. Deck 2 comes out at unity
    on MAIN, and the render takes 48 frames per millisecond.
  - Back at 48 kHz the resampler is gone; at 96 kHz, 32 kHz and 0 the output is
    silent.
  - CI runs the host tests and clippy with `uac-44k1` too.

## Step 3: the periodic TX FIFO for a 540-byte packet

Branch `ogranak/uac-fifo`, from `main` at `dc2c950`.

- **The limit:** esp-usb's balanced split gives the HS controller's periodic OUT
  FIFO 128 lines (512 bytes). The Inpulse 500's isochronous OUT packet is 540
  bytes, so the HCD refuses its interface claim. The deck now logs that refusal
  (step 1).
- **The split:** `main/usb_fifo.h`, the donor's JC1060 split, in 4-byte lines:
  - non-periodic TX: 256 lines, as balanced (two 512-byte bulk OUT packets for
    the stick and MIDI);
  - periodic TX: 160 lines (640 bytes);
  - RX: the rest, 32 lines fewer than balanced, and never below two 512-byte
    bulk IN packets with their status lines.
  - A FIFO too small for this keeps esp-usb's own split.
- **The total:** esp-usb splits the FIFO only at install, and its total is in
  the controller's hardware configuration (`GHWCFG3`), readable only once
  installed. `ruzina_usb_host_enum_start` therefore installs twice:
  1. A first install leaves the root port unpowered, so no device is seen, and
     reads the total.
  2. After `usb_host_uninstall`, a second install gives the deck's split.
  - If the uninstall fails, the first install stays, its root port is powered,
    and esp-usb's split applies.
  - The boot log shows the outcome:
    `S6_USB_FIFO total_lines=… split=DECK|DEFAULT rx=… nptx=… ptx=… install=…`.
- **Tests:** `tools/tests/test_usb_fifo.py` checks the split against a 4 KiB
  FIFO, the 540-byte packet, the RX minimum and the smallest FIFO that takes it.
