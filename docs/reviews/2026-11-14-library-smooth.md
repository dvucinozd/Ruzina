# JC1060: a smoother Library page

Date: 2026-11-14. Branch: `ogranak/jc1060-library-smooth`, from `main` at `f03b703`.

## Problem

While the owner browsed the Library and loaded tracks, single Slint frames took up to
113 ms ([Latin letters](2026-11-13-latin-glyphs.md#cost)). On s91:
- one came after a burst of 17 browse knob steps, the other after a track load;
- the decoder's longest gap in those seconds was 116 ms and 236 ms. The screen task
  runs above the decoding client on the same core, so the time was the screen's own
  work.

## What the board showed

With per-area logging (s96, s97), the slow frames were these:

| Change | Area drawn | Frame |
|---|---|---|
| Opening LIBRARY | the page, 472 pixel rows | 104 ms, then 86 ms in the other buffer |
| A browse step past the page's edge | every row, about 395 rows | about 110 ms, in each buffer |
| A track load | two rows and the title strip, 122 rows | 34 ms |

- **Text is the cost.** A host benchmark of the page's full draw showed the titles and
  artists to be half of it. The rows' borders, backgrounds and `elide` made no
  difference. A page of text costs about 2.5 times an Overview pixel.
- **Every step past the edge redrew the list.** The page moved by one row, so all 12
  rows changed.
- **Audio, s97:** during the owner's browsing, six such frames in a row kept the decoders
  off their core for 252 ms. Deck 2, which had just loaded, ran dry: 57 underruns.
- **Each change was also made twice.** Each change of the Library state built a new
  model of all 12 rows, so Slint made the rows again.
- **Each open built the page again.** The page was made on each open, about 18 ms.

## Change

- **A lasting model:** the Library page keeps one model. `show_library` replaces only
  the lines that differ, and adds or removes rows at the end. A selection step within
  the page changes two lines.
- **The page is made once:** `LibraryPage` is always there, shown only on its tab.
- **The list turns a page at a time:** when the selection leaves the page, the first
  row becomes the selection's multiple of 12, at most the last full page. Scrolling
  redraws every row once a page, not at every step.
- **The screen steps aside after a long frame:** after a Slint frame over 30 ms, it
  waits for two panel refreshes (about 33 ms), so a run of long frames cannot hold the
  decoders off.
- **Slow frames logged**, for a Slint frame over 30 ms:
  - 23: its draw time, with the pixel rows drawn as `pass`;
  - 24: the page updates before it, with the open page as `pass`;
  - 25: each area drawn (first and last row, first and last column);
  - 26: the lines and pixels drawn.
- **Test fix:** the Shift + BEAT SYNC engine test (#118) started from rings an earlier
  test left. The new deck readers synced to a source whose start the ring had long
  passed, and the ring's free count overflowed: about 1 run in 20 failed. It now starts
  each ring with a fresh source. That passed 30 runs out of 30.

## Tried and dropped

- **Drawing each change once (s97, s98):** Slint drew into one buffer, and the other got a
  copy of the drawn rows the next frame.
  - The copy of a page was a PSRAM-to-PSRAM burst of about 1 MB (32 ms).
  - It raised the USB audio fill, on the other core, from about 6 to 9 to 10.6 ms.
  - Without the copy (s96, s100), opening the Library left the fill at 5.7 to 6.3 ms.
- **The screen level with the decoding client (s98, priority 3):**
  - no underruns, and the decoder's longest gap fell from 252 to 127 ms;
  - but 14 USB packets were lost in 20 minutes, spread out, with no saves or slow frames
    near them, against 0 on s93 to s97. The likely path: a freer decoder issues more SD
    reads, and the SD owner shares priority 4 with the USB client on the other core.
    This is not proven; the priority stays 4.

## Evidence

- **Host:** the legacy CI matrix, the Slint host tests and clippy, and target clippy pass.
  The IDF build has no warnings.
- **Device:** s100 (app `b22cf0ea…`), owner verdict "Radi".
  - **Browsing across pages:** one page turn draws all rows, about 88 ms, then about
    77 ms in the other buffer. Steps within the page took 30 to 48 ms when a slow frame
    came at all.
  - **Opening LIBRARY:** 102 ms, then 88 ms.
  - **Loads with the Library open:** 30 to 34 ms.
  - **Audio, over the whole 20 minutes** (7 Hot Cue saves): underruns 0/0,
    `packet_errors=0`, longest fill 5.7 ms, USB task free stack 2832 bytes.

## Test plan

1. LIBRARY: turn the browse knob quickly across several pages: the list turns a page at
   a time, and the sound does not break.
2. Load tracks into the decks with the Library open: D1/D2 marks follow.
3. Tap rows: the tapped row is selected.
4. Open HOT CUES, SETTINGS and FX, then back to OVERVIEW: nothing of the last page is
   left.

## Open

- A page turn, and opening the Library, still draw for 80 to 100 ms per buffer. Halving it
  needs a cheaper copy between buffers (the 2D DMA, say) or fewer rows of text.
- The Settings page redraws its figures twice a second, at about 50 ms a buffer.
