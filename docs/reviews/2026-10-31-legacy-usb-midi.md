# JC1060 `usb-midi`: DDJ-FLX4 controls into the Rust session

Date: 2026-10-31. Branch: from main `e466f5f` (after PR #65). The device
run, with the owner present, is at the end. Native minimum 302 and APTA unchanged.

## What it adds

This builds on the [`usb-host` diagnostic](2026-10-31-legacy-usb-host-enum.md),
which enumerates the FLX4 at high speed on the HS port. The new profile goes
one step further: the controller's MIDI goes through the shared semantic
controller stack, as on the native USB1 owner.

- **Rust, `controller_midi.rs`** (feature `idf-controller-midi`):
  - It embeds the M2.5 golden FLX4 profile, the
    `ruzina-controller-profile` fixture `pioneer_ddj_flx4.v2-golden.s3bin`
    (S3CP v2).
  - `ruzina_legacy_midi_bind(vid, pid)` selects the profile only for
    `2b73:0045`. It resets profile state and marks the session connected.
  - `ruzina_legacy_midi_ingest(bytes, len)` passes one lent Bulk-IN payload to
    `ControllerSession::process_usb_midi_transfer`. It then drains every
    delivery and calls the C callback `ruzina_bridge_midi_event` for each
    control event. The callback receives scalars plus the control's name,
    formatted without allocation and lent for the call only.
  - `ruzina_legacy_midi_unbind()` ends the connection, and
    `ruzina_legacy_midi_totals()` exports the batch report.
  - The session is one static used only by the C USB client task.
- **C, `usb_host_enum.c`** (`CONFIG_RUZINA_USB_HOST_MIDI`):
  - After the enumeration report, it finds the first MIDI streaming interface
    (class 01/03, alt 0) with a Bulk-IN endpoint, binds, claims it and keeps
    exactly one Bulk-IN transfer outstanding (512 bytes on the FLX4).
  - Each completed transfer goes to Rust; `midi_event` lines log every
    semantic event.
  - On detach, it waits for the transfer to finish with `NO_DEVICE`, unbinds,
    releases the interface, frees the transfer and logs totals in
    `usb_midi_stop`.
  - It sends no MIDI out (no LEDs, no init SysEx).
- **Configuration:**
  - The option requires `USB_HOST_ENUM`; the guard test gains 2 cases.
  - The build tool gets the `usb-midi` variant: Rust feature, defaults, link
    check for `ruzina_legacy_midi_ingest`, frame guard and the existing USB
    checks.
  - CI adds host tests, clippy and target builds for `idf-controller-midi`,
    and adds the variant to the matrix.

## Evidence (local)

- **Rust tests:** the new host test drives the FLX4 profile with deck 1 and
  deck 2 Play presses (`0x90/0x91 0x0B`). It checks that both reach the
  bridge as `Play` presses for their decks, and that a wrong VID, a
  misaligned payload and an unbound session are rejected. The 39 existing
  tests still pass.
- **Clippy:** host clippy, and target clippy for `idf-exfat-fixture`,
  `idf-media-load` and `idf-controller-midi`, all pass.
- **Tooling:** 24 tests pass.
- **Build:** `build_legacy.py --variant usb-midi` returns `BUILD_PACKAGE_PASS`
  with no warnings. The app is 239792 bytes, SHA-256
  `72b6e03daff3cf12be81f0f1af50a8cd081969455919f9a8a653545e31157f6d`.
- **Offline flash plan:** `a6ba74be…`. The predecessor is the flashed
  `usb-host` image (`ed7ae4c0…`), with a chain of 14 plans. It writes
  `0x2000`, `0x8000` and `0x10000..0x4b000`.

## Device acceptance (needs the owner)

The same safety rules apply as for `usb-host`: PC power on the FS port only,
the battery disconnected and the FLX4 on bus power.

Accept the run when:
- `usb_midi profile=1 interface=4 in=82 out=03 mps=512 claim=ESP_OK` is
  logged.
- Pressing Play and Cue on both decks and moving the tempo sliders, the
  crossfader, the jogs and a channel fader each produce matching
  `midi_event` lines (deck, control, value).
- A detach logs `usb_midi_stop` with `release=ESP_OK free=ESP_OK`, and a
  re-attach rebinds.
- There is no fault, reset or `midi_in` error.


## Device run (2026-10-31, owner present)

Same setup as for `usb-host`: the PC on the FS port only, the battery
disconnected and the FLX4 on bus power, C-to-C on the HS port. Records are in
`docs/boards/evidence/jc1060-20261030-com3/s06-usb-midi-*`, indexed by the
[device index](../boards/readiness/jc1060-s06-usb-midi-device-20261031.json).

1. **First image: stack fault at bind.** `72b6e03d…` (plan `a6ba74be…`)
   enumerated the FLX4, then hit a **Stack protection fault** in `usb_client`
   (4 KiB) inside `ruzina_legacy_midi_bind`.
   - Cause: the lazy `get_or_insert_with` built the whole `ControllerSession`
     (4288 bytes) on the stack, which gave every export a 5.4 KiB frame.
   - Fix: the static is now initialized at compile time
     (`ControllerSession::new` is `const`), and the exports' frames are
     negligible. The client task stack is 8 KiB.
   - The build tool now rejects any Rust frame over 2 KiB in `usb-midi`.
2. **Second image: buttons.** `4ff552ce…` (plan `634538a8…`) logged
   `usb_midi profile=1 interface=4 in=82 out=03 mps=512 claim=ESP_OK`.
   - Two sessions with one replug: Play and Cue on both decks plus
     BrowsePress, each pressed and released, gave 20 `midi_event` lines and
     10 packets per session.
   - All packets decoded and mapped, with 0 dropped and 0 semantic errors.
     Both detaches gave `release=ESP_OK free=ESP_OK`.
   - The monitor's boot mode read 4 KiB blocks with no timeout and lost one
     capture's claim line. The monitor now polls, and the lost reader output
     is kept as `continuous-stalled-reader`.
   - On unplug, the pending IN transfer completed with an error before the
     gone event. The immediate resubmit made the IDF log `Enqueue URB error`.
3. **Third image: error retry.** `ed2d9b6b…` (plan `fec51cfb…`) no longer
   resubmits on an error. It retries from the client task after 100 ms, and
   only if the device is still present; the client wait is bounded to 50 ms.
   - **Restart session:** after an app restart, the FLX4's first session
     ended with one IN error, then a clean stop and detach, and the controller
     re-enumerated on its own.
   - **Continuous controls on the re-enumerated session:**

     | Control | Events | Values |
     |---|---|---|
     | Crossfader | 171 | 0..16383, full 14-bit range |
     | ChannelVolume deck 1 | 282 | 0..16383 |
     | ChannelVolume deck 2 | 212 | 0..16383 |
     | JogScratch deck 1 | 620 | relative −3..+2 |
     | JogScratch deck 2 | 683 | relative −3..+2 |
     | JogBend deck 1 | 13 | relative |
     | JogTouch decks 1 and 2 | on and off | — |

   - **Totals:** 2650 packets; 2647 scheduled; 0 dropped; 3 unmapped; 0
     semantic errors; release and free `ESP_OK`. No `Enqueue URB` error
     appeared.

Tempo sliders, EQ, filter, master and headphone level did not appear in this
capture, so they remain unverified on the device. There is still no MIDI out (LEDs, init SysEx), and
no product state consumes the events yet.

## LED output (MIDI out), build only

The next step closes the loop to the controller. No product LED state exists
yet, so the output is a diagnostic echo through the profile's output map.

- **Rust:**
  - A 256-packet USB-MIDI out queue. Each packet is cable 0, with the CIN set
    to the status nibble.
  - `ruzina_legacy_midi_lamp(on)` queues every profile output: on or off
    values for notes, 127 or 0 for CC. On the FLX4 that is 129 packets, more
    than one 512-byte transfer.
  - The echo: each Play press toggles that deck's Play LED through
    `map_led_update`, and the Cue LED follows the held Cue button.
  - `ruzina_legacy_midi_take_out(buf, cap)` hands whole packets to C and keeps
    the rest queued. `ruzina_legacy_midi_out_overflow()` counts packets lost
    to a full queue.
- **C:**
  - One Bulk-OUT transfer on the MIDI interface's OUT endpoint (`0x03`,
    512 bytes).
  - `midi_flush` submits whenever nothing is in flight: after each ingest,
    after each OUT completion and on every client-loop pass.
  - On claim, a 1.5 s lamp test runs (`midi_lamp on=0` when it clears).
  - Detach waits for both transfers, then frees them. `usb_midi_stop` adds
    `out_transfers`, `out_bytes`, `out_errors` and `out_overflow`.
- **Host test:**
  - Both decks' Play LEDs latch on, and a second press turns deck 1's off.
  - The Cue LED is on while Cue is held and off on release.
  - The lamp test yields 129 packets across a 16-byte and two 512-byte takes,
    every one with a valid CIN, and nothing overflows.
- **Build:** `BUILD_PACKAGE_PASS`, app `34d8fe44…` (242048 bytes). The
  offline plan is `cf7549e5…`, with the flashed `ed2d9b6b…` image as its
  predecessor.

Device acceptance:
- On attach, every FLX4 LED lights for about 1.5 s and then clears.
- Each Play press toggles that deck's Play LED.
- The Cue LED is lit only while Cue is held.
- `usb_midi_stop` reports `out_errors=0` and `out_overflow=0`.