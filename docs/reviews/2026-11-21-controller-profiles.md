# JC1060: the donor's other controller profiles

Date: 2026-11-21. Branch: `ogranak/controller-profiles`, from `main` at `010dae0`.

## Source

The owner asked how the donor's other controller profiles would come over. The donor
(Pajoniiir shared P4 core, `0cde4181`) has seven, in `controllers/<name>/profile.json`,
compiled to S3CP binaries. This change takes steps 1 and 2 of that plan: the profiles and
the parser, then the deck choosing a profile by the controller's USB VID:PID. Audio for the
controllers whose USB audio the deck cannot stream yet is step 3, left for when one of
them is at hand.

## What was there

| Controller | Our compiler and parser | USB audio on the deck |
|---|---|---|
| Pioneer DDJ-FLX4 | the same bytes as our golden profile | streams |
| Pioneer DDJ-400 | compiles and maps all 284 inputs | UAC1, not yet read from a real unit |
| Numark DJ2GO2 Touch | 96 of 100 inputs: `deckN.pfl_set` unknown | UAC1 at 44.1 kHz, 16-bit, not yet tried |
| NI Traktor Kontrol S2 MK3 (full) | did not parse: `cc7_to14_inverted` unknown | none: UAC2 |
| Hercules DJControl Inpulse 500 | compiles, but the compiler dropped `native_headphone_controls` | none: asynchronous, with a feedback endpoint |

The deck itself carried only the FLX4's profile, sent no initial SysEx, never set the
mixer's FILTER_ALWAYS, and read the FLX4's SAMPLER notes from any controller.

## Now

- **Compiler:** the donor's current `compile_profile.py`, copied verbatim. Over the M2.5
  one it adds `cc7_to14_inverted` inputs, the `deckN.pfl_set` event, `note_velocity`
  outputs and the `native_headphone_controls` capability. It still reproduces every frozen
  binary byte for byte.
- **Profiles:** the DDJ-400, Numark, S2 MK3 (full) and Hercules (v4) JSON sources, copied
  verbatim, with their compiled binaries. Each binary is the same as the donor's own
  `profile.s3bin`. `fixtures/README.md` records each one's source, VID:PID, the donor's
  hardware status and SHA-256.
- **Parser (`ruzina-controller-profile`):**
  - `RawType::Cc7To14Inverted` (v4): 0/64/127 to 16383/8192/0, the donor's integer
    formula, with the center exactly 8192. It is checked like `Cc7To14` and replays the
    same way.
  - `PROFILE_FLAG_NATIVE_HEADPHONES`: refused before v4 and without USB audio, as the
    donor does. The deck does not act on it yet; it matters only with the Hercules's
    audio, step 3.
  - Semantic ids `0x85`/`0x86` map to `SemanticControl::PflSet`.
- **PFL set (`ruzina-controller-core`, `ruzina-mixer`):** `PflSet` with `Pressed(on)`
  sets the deck's PFL to `on`, for a latching button. `Pfl` still toggles on a press.
- **The deck (`controller_midi.rs`):**
  - **Choosing a profile:** `CONTROLLERS` lists the five profiles with their names, and
    `ruzina_legacy_midi_bind` takes the one whose profile names the attached VID:PID. A
    controller without one is still bound and its events stay unmapped, as before.
  - **Initial SysEx:** the profile's SysEx goes first in the output queue, packed as
    USB-MIDI packets (code index 0x4, then 0x5–0x7 for the end). That puts it ahead of the
    lamp test and the LEDs.
  - **Filter without Smart CFX:** the profile's FILTER_ALWAYS (the DDJ-400) goes to the
    mixer at bind and is cleared at unbind.
  - **SAMPLER trick:** reading the SAMPLER button and pads outside the profile, and
    lighting the pads, is now done only for the FLX4. On another controller those notes
    could mean something else.
- **Settings page:** the first line on the right is now "Controller", which shows the
  attached controller's name, its VID:PID with "no profile", or "none". It used to read
  "Output: 48 kHz to the FLX4".

## Tests

- **`ruzina-controller-profile`:**
  - The donor's profiles compile to the frozen bytes, parse as v4 with the right
    VID:PID, and every input maps to a control.
  - The DDJ-400's SysEx and filter flag, the Numark's PFL set, the Hercules's headphone
    flag and the S2's inverted faders are each checked.
  - The inverted scaling is checked at its detents and for monotonic order.
  - The headphone flag's limits are checked.
- **`ruzina-mixer`:** a latching PFL sets the state it reports.
- **`controller_midi`:**
  - Every built-in profile is found by its own VID:PID.
  - The DDJ-400's SysEx leads the output queue, and the FLX4 sends none.
  - A controller with no profile shows by its VID:PID.
  - SysEx packing is checked, and the SAMPLER split happens only on the FLX4.
- **Clippy:** the whole firmware-rust workspace, and the deck's Slint, Web Remote,
  profile and screen-still feature sets on the target.

## On the board

Image s173 (app `fd4d6e1b…`), the play image flags on this branch, with no controller
attached: the FLX4 is lent out and no other is at hand.

- **Owner:** the boot screen, the Settings page's "Controller: none" and the rest of the
  deck work as on s172 ("Sve radi").
- **Log:** about 2 minutes, no errors.
- **Not yet tested:** the FLX4 on this image, and every other controller. Each needs its
  own controller at the board, and the Hercules and the S2 MK3 also need step 3 for audio.
