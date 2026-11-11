# JC1060: Hot Cues, the loop and Quantize on the deck screen

Date: 2026-11-12. Branch: `ogranak/jc1060-lane-markers`, from `main` at `c91e5c4`.

## Problem

The lanes showed the waveform and the beat grid, but not:
- where the Hot Cues are;
- where a loop runs.

Quantize, on since [quantize on the beat grid](2026-11-12-quantize.md), had no sign on
the screen. The browse knob zoomed one step a detent only.

## Change

- **Marks:**
  - Outside the audio fill, the deck engine hands the screen each deck's effective Hot
    Cues and running loop through a small slot with a revision. It writes them only
    when they change.
  - New marks make the UI draw the whole lane once in each buffer. A lane whose marks
    differ is never copied by the PPA.
- **Lane:**
  - A Hot Cue is a line through the lane in its pad's colour (A to H). A square flag
    sits at the edge away from the red bar triangles: the top on deck 1, the bottom on
    deck 2.
  - The running loop's ends are orange lines, and its span is shaded behind the
    waveform; beat lines stay visible.
- **Q:** a Q label next to MT lights orange while Quantize is on. To fit it, MT, the key
  and the status (SYNC, LOOP, EMPTY) moved, and the status is a little smaller.
- **Shift + browse** moves the zoom three steps a detent.

## Evidence

- **Host:** 2 new tests:
  - the marks slot keeps its revision until the marks change;
  - the engine publishes a quantized Hot Cue and a 4-beat loop, clears the loop on
    exit, and Shift + browse zooms three steps.

  The legacy feature sets and target clippy (including `idf-slint-ui`) pass.
- **Build:** s76, app `c3656040…`, flashed on 2026-11-12. Not yet tried by the owner.

## Test plan

1. Set several Hot Cues: coloured lines and flags appear and scroll with the waveform.
2. Start a loop: orange ends and shading. Exit it: the shading goes.
3. Q lights on both decks, and nothing in the deck row overlaps.
4. Shift + browse zooms in larger steps.
