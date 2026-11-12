# JC1060: the Hot Cues page on the deck screen

Date: 2026-11-12. Branch: `ogranak/jc1060-hotcues-page`, from the
[Library page](2026-11-12-library.md) (merged as `2ce78fc`).

## Problem

The deck screen's HOT CUES tab only marked itself. The lanes showed where the cues lie,
but you could set, recall or delete them only with the controller's pads, in Hot Cue
mode.

## Change

- **Page:** each deck gets its own half, with its badge and track title. Its eight
  pads are laid out as on the FLX4: A to D above, E to H below.
  - A set pad is filled with its pad colour, the same colours the lanes use, and shows
    the cue's time to a tenth of a second.
  - An empty pad is dark, with a border in its colour and "TAP TO SET".
  - A deck without a track dims its pads, and they take no taps.
- **Touch:**
  - A tap is the pad going down and up, sent to the engine as the controller's pad
    event in Hot Cue mode, whatever mode the pads are in.
  - So the deck state decides: an empty pad sets a cue at the playhead (on the nearest
    beat with Quantize), and a set pad recalls it.
  - Edits are saved the way pad edits are.
- **DELETE:** each deck has a DELETE button. While it is lit, set pads get a red border,
  and a tap clears that cue, as Shift and the pad do. DELETE then goes out.
- **Touch path:** `touch_events` in `controller_midi.rs` now turns every screen tap into
  controller events, PLAY, CUE and LOAD included, so the conversion is tested on the
  host.
- **Drawing:** like the Library, the page covers the lanes and meters, and Rust stops
  drawing them until the Overview returns. The cue colours now live in one RGB table,
  which both the lanes and the page read.

## Evidence

- **Host:** 1 new test, and 1 extended:
  - new: a Hot Cue tap sets a quantized cue, a delete tap on an empty pad changes
    nothing, and a delete tap clears the cue;
  - extended: the touch queue round-trips Hot Cue taps.

  The legacy feature sets and target clippy (including `idf-slint-ui` and the profile
  build) pass.
- **Device:** s79 (app `8b22feab…`), owner verdict "Radi".
  - Underruns 0/0, `packet_errors=0`, no panics in the capture.

## Test plan

1. Open HOT CUES: both decks show, and the lanes and meters are hidden.
2. Tap an empty pad on a playing deck: it fills and shows the cue's time.
3. Tap a set pad: the deck jumps to its cue.
4. DELETE, then a set pad: the cue goes, and DELETE goes out.
5. A cue set with the controller's pad appears on the page.
6. Back to OVERVIEW: the lanes and meters return, with the cue lines.
