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
  - After Loop In, an orange line marks the In. The shading grows from it to the
    playhead until Out, as the owner asked. The columns the playhead passes are drawn
    again every frame anyway, so the growing shade costs no extra drawing.
- **Q:** a Q label next to MT lights orange while Quantize is on. To fit it, MT, the key
  and the status (SYNC, LOOP, EMPTY) moved, and the status is a little smaller.
- **Shift + browse** moves the zoom three steps a detent.

## Evidence

- **Host:** 2 new tests:
  - the marks slot keeps its revision until the marks change;
  - the engine publishes a quantized Hot Cue and a 4-beat loop, clears the loop on
    exit, and Shift + browse zooms three steps.

  The legacy feature sets and target clippy (including `idf-slint-ui`) pass.
- **Device:**
  - s76 (app `c3656040…`): the owner tried all four points ("radi sve") and asked for the
    shading to start at Loop In.
  - s77 (app `fc0aac16…`) adds that; owner verdict "Radi".
  - Underruns 0/0, `packet_errors=0`.
  - Longest fill 5.6 ms with the soak DJ, within the accepted scratch exception.
  - The worst waveform frame was 29 ms, which is a full lane redraw; new marks cause one.
    With Slint's worst of 9 ms, such a frame can miss one 30 Hz refresh.

## Test plan

1. Set several Hot Cues: coloured lines and flags appear and scroll with the waveform.
2. Start a loop: orange ends and shading. Exit it: the shading goes.
3. Q lights on both decks, and nothing in the deck row overlaps.
4. Shift + browse zooms in larger steps.
