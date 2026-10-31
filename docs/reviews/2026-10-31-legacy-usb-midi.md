# JC1060 `usb-midi`: DDJ-FLX4 controls into the Rust session

Date: 2026-10-31. Branch: from main `e466f5f` (after PR #65). Build only: no
device run yet. Native minimum 302 and APTA unchanged.

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
