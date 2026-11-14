# JC1060: a smoother Library page

Date: 2026-11-14. Branch: `ogranak/jc1060-library-smooth`, from `main` at `f03b703`.

## Problem

While the owner browsed the Library and loaded tracks, single Slint frames took up to
113 ms ([Latin letters](2026-11-13-latin-glyphs.md#cost)). On s91:
- one came after a burst of 17 browse knob steps, the other after a track load;
- the decoder's longest gap in those seconds was 116 ms and 236 ms. The screen task
  runs above it on the same core, so the time was the screen's own work.

Each change of the Library state (the selection, the page, a deck's track) built a new
model of all 12 rows. Slint then dropped the rows and made them again, and drew the
whole list.

## Change

- **A lasting model:** the Library page keeps one model. `show_library` replaces only
  the lines that differ, and adds or removes rows at the end. A selection step within the
  page changes two lines; a page step still changes all of them.
- **Slow frames logged:** a Slint frame over 30 ms now logs two events:
  - 23: its draw time, with the pixel rows drawn as `pass`;
  - 24: the page updates before it, with the open page as `pass`.

## Evidence

- **Host:** the Slint host tests and clippy, and target clippy, pass. The IDF build has
  no warnings.
- **Device:** s95 (app `62b4b442…`), owner verdict "Glatko".
  - **At boot**, the only frames over 30 ms were the two that draw the whole screen:
    600 rows, 53 and 43 ms. The page updates before them took under 0.5 ms. Drawing
    itself costs about 85 µs per full row.
  - **Owner's test:** browsing across pages and loading tracks with the Library open.
    - Browsing: no frame over 30 ms; the worst per 20 s was 14 to 15 ms.
    - **A track load** with the Library open still makes two slow frames, one in each
      buffer: 49 and 44 ms (236 pixel rows), and later 40 and 35 ms (231 rows). The page
      updates before them took 0.5 to 2 ms. A load changes rows far apart: the deck
      marks in the list and the deck's title. The redrawn area spans them, at about
      190 µs per row of text.
  - Over 15 + 30 minutes before that, with the soak DJ: underruns 0/0,
    `packet_errors=0`, longest fill 6.3 ms, 25 Hot Cue saves.
  - **Lost USB packets:**
    - The board ran on without a capture between 45 and 71 minutes. In that time one
      fill reached 24 ms and 2 USB packets were lost; the cause is not in a capture.
    - In the third capture (71 to 86 minutes) 4 more packets were lost. Most came within
      seconds of a soak Hot Cue save; one came after a SYNC.
    - After 70 minutes of soak saves the journal is past one rotation per bucket, and
      erases during play again ([Hot Cue erase-ahead, Open](2026-11-13-hot-cue-erase-ahead.md#open)).
      The change here does not touch flash or audio.

## Test plan

1. LIBRARY: turn the browse knob quickly across several pages: the list follows without
   a stall.
2. Load tracks into the decks with the Library open: D1/D2 marks follow.
3. Tap rows: the tapped row is selected.

## Open

- A track load with the Library open still draws two frames of 35 to 49 ms.
- With the Library open, the average Slint frame is about 9 ms, against 4.4 ms on the
  Overview (s91 showed the same). Something on that page is redrawn every frame.
