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

## Step 4: UAC1 explicit feedback

Branch `ogranak/uac-feedback`, from `main` at `4157093`.

- **Descriptor (`ruzina-uac`):** an asynchronous OUT alternate is now offered at
  full speed when it carries its own explicit feedback endpoint, as USB Audio
  1.0 section 3.7.2.2 defines it:
  - an isochronous IN endpoint with usage "feedback", no sync type of its own,
    3 bytes, every frame;
  - `bRefresh` 1 to 9, so a new value every 2 to 512 ms;
  - named by the OUT endpoint's `bSynchAddress`.
  - Implicit feedback, a second feedback endpoint, a wrong size or interval, a
    synch address elsewhere, and high speed (where feedback counts
    microframes) stay refused with `reason=FEEDBACK`.
  - The Inpulse 500 capture is now selected: alternate 1, 44.1 kHz,
    feedback IN `0x81`, refresh every 32 ms.
- **Feedback (`ruzina_uac::feedback`):** 3 bytes of 10.14 samples per frame
  become Q16 frames per millisecond.
  - A reading more than 5 % from the nominal rate, or one that would need more
    frames than the endpoint holds (45 for 540 bytes), is refused.
  - An empty reply between refreshes is no new value.
- **Cadence:** `Cadence::with_feedback` starts at the nominal rate and, after
  each reading, paces packets by the device's rate, carrying the fraction
  over. Without feedback nothing changes.
- **Firmware:**
  - `ruzina_legacy_uac_select` now returns the feedback endpoint and refresh
    (ten words). `ruzina_legacy_uac_feedback` takes each reading, and
    `ruzina_legacy_uac_feedback_stats` reports updates, empty and refused
    readings and the pace.
  - The C driver keeps one 3-byte isochronous IN reading in flight beside the
    OUT pool while the stream runs, and queues it again after each completion.
    A failed reading is counted and the pace held.
  - During an OUT recovery the reading is not queued again, so the recovery's
    wait for every transfer ends; the restart queues it again. A stop frees it.
  - The log names the endpoint once (`usb_uac_feedback ep=81 refresh_ms=32
    pace=EXPLICIT`) and then reports every 1000 transfers.
  - Packets may carry up to 51 frames, 5 % over 48 kHz, as far as the endpoint
    holds.
- **Tests:**
  - `ruzina-uac`: every refused feedback shape, decoding, and the paced
    cadence (1 % fast gives 441 more frames per second).
  - `rust-core`: the Inpulse 500 capture is selected with `0x81` and refresh 5.
    600 packets follow a 2 % fast reading, and no packet exceeds 540 bytes.
  - C driver: the reading stays in flight, empty and failed readings are
    handled, the recovery waits for it, and stop frees it. A device without
    feedback gets no feedback transfer.
- **Not checked:** playing on an Inpulse 500. None is at hand.

## Step 5: native headphones

Branch `ogranak/native-headphones`, from `main` at `7419a5a`.

- **Why:** the Inpulse 500 blends cue with MAIN and sets the headphone level in
  its own hardware. Its profile says so with `native_headphone_controls` (S3CP
  v4 bit 5), and it maps no software headphone mix or level. The deck was
  sending its software headphone mix at the -12 dB ceiling, which the hardware
  would then blend and attenuate a second time.
- **Now (`deck_mix`):** while the bound profile has the flag, the render graph's
  headphone gains are the cue bus alone at unity (cue 1, MAIN 0, level 1), and
  the -12 dB ceiling is skipped.
  - Channels 3/4 carry the pre-fader cue of the decks whose PFL is on, mono on
    both sides as the graph builds it, or silence when none is on.
  - MAIN on 1/2 is unchanged.
  - The software Headphone Mix and Level stay as they were, and apply again
    when another controller binds or none is bound.
- **Binding (`controller_midi`):** the flag is set when a profile binds, next to
  FILTER_ALWAYS, and cleared on unbind.
- **Tests:**
  - `deck_mix`: with the flag, deck 1's cue comes out at unity (10 000) instead
    of 625, and MAIN is unchanged. With PFL off the headphones are silent;
    without the flag the software mix returns.
  - `controller_midi`: binding the Inpulse 500 sets the flag, and the FLX4 or an
    unbind clears it.
- **Not checked:** an Inpulse 500's headphones, as none is at hand. The FLX4
  has no flag, so nothing changes for it.

## Step 6: the S2 MK3's zero tail and a UAC2 parser

Branch `ogranak/uac2-parser`, from `main` at `f0cde19`.

- **Zero tail:** zero bytes from a descriptor boundary to the end of
  `wTotalLength` are padding, not a descriptor. A zero length followed by
  anything else is still malformed.
- **UAC1 walk:** only protocol-0 streaming alternates are walked. A UAC2
  alternate (protocol 0x20) has other class descriptors, so it is no longer
  read as a malformed UAC1 one.
- **`ruzina_uac::uac2`:**
  - The first pass collects each UAC2 AudioControl interface's clock sources and
    input terminals.
  - The second reads each playback alternate: AS_GENERAL v2 (terminal link,
    PCM, channels), Format Type v2 (subslot and bits), the isochronous OUT
    endpoint and its explicit feedback endpoint.
  - It then links the alternate through its terminal to the terminal's clock
    source, on the same AudioControl interface.
  - `supported(speed)` holds only for what the deck can drive:
    - high speed, PCM in 16/2 or 24/4, two or four channels;
    - one packet per microframe, at least 7 frames per packet;
    - asynchronous OUT with its own 4-byte feedback endpoint;
    - a clock whose frequency is settable and whose validity is readable.
- **Choice:** a configuration without UAC1 playback is now reported by its UAC2
  alternates:
  - `reason=UAC2 interface=… alt=…` for one the deck can drive once it speaks
    UAC2 (steps 7 and 8);
  - `reason=UAC2_UNSUPPORTED` when none qualifies.
  - Nothing streams UAC2 yet.
- **The S2 MK3 capture** (high speed) now parses:
  - interface 1 alternate 1: 4 channels of 24 bits in 4 bytes;
  - OUT `0x01` of 112 bytes every microframe, feedback IN `0x81` of 4 bytes
    every 1 ms;
  - terminal 1 on clock `0x11` of AudioControl interface 0, settable and with
    readable validity.
  - The deck logs `reason=UAC2 interface=1 alt=1` (full speed:
    `UAC2_UNSUPPORTED`).
- **Tests:**
  - a driveable function and each undriveable variant;
  - padding and truncation;
  - the UAC1 walk with padding and with a UAC2 alternate;
  - the S2 capture end to end;
  - the firmware's reason codes 12 and 13.

## Step 7: UAC2 clock negotiation

Branch `ogranak/uac2-clock`, from `main` at `5b5f918`.

- **`ruzina_uac::clock`:** the donor's sequence as a state machine that
  touches no USB. Each step is a class request to the clock's AudioControl
  interface (`wIndex = clock << 8 | interface`):
  1. GET RANGE of the sampling frequency, 2 bytes, then the whole `2 + 12 n`
     bytes, checked whole (1 to 8 subranges, none empty).
     - 48 kHz must lie in a subrange; if it does not, 44.1 kHz is used.
     - With neither, the negotiation fails with `NO_RATE`.
  2. SET CUR with the rate, 4 bytes.
  3. GET CUR, 4 bytes.
     - While the clock reports another rate, the read is repeated every 50 ms
       for up to 500 ms.
     - If the clock settles at 44.1 kHz, the range has it, and 48 kHz was
       asked for, it is set once more at 44.1 kHz.
     - Otherwise the negotiation fails with `READBACK`; the deck never streams
       one rate labelled as another.
  4. GET CUR of clock validity, 1 byte, until it reads 1, in the same window.
- **Firmware:**
  - `ruzina_legacy_uac_select` returns 2 for a driveable UAC2 alternate, with
    the clock source and its AudioControl interface (now twelve words).
  - `ruzina_legacy_uac2_clock_begin`, `_done` and `_status` run the state
    machine. Rust copies each reply before it writes the next request.
- **The C driver** (`UAC_CLOCK`):
  - Its control buffer is now 128 bytes, enough for an eight-subrange
    GET RANGE.
  - It sends each request Rust builds and hands back the reply bytes. A read
    that waits for the clock goes out on a later client-loop turn
    (`uac_clock_poll`).
  - At the end it logs
    `usb_uac_clock id=11 interface=0 state=READY|FAILED rate=… retries=… failure=…`.
  - When ready, it logs `usb_uac stream=UAC2_NOT_YET`: streaming needs step 8.
    A failure holds the fault.
  - Either way, stop releases the interface.
- **Tests:**
  - `clock`: each path, at once, settling, stuck at 44.1 kHz with and without
    it in the range, a range without 48 kHz, and every failure.
  - `controller_audio`: the S2 MK3 capture is selected as UAC2 and negotiated
    against a clock that reports the old rate twice, then a stalled request.
  - C driver: send, wait, poll before and after the wait, ready, a stall,
    release on stop.
- **Not checked:** an S2 MK3's clock, as none is at hand. The FLX4 stays UAC1,
  so nothing changes for it.
