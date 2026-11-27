# JC1060: every Text that can change has a height

Date: 2026-11-27. Branch: `ogranak/slint-frames`, from `main` at `b945b42`.

## Problem

A Library page turn took about 56 ms on the board. Of its 30 ms of scene preparation,
the dirty tracking took 14 ms ([big frames](2026-11-26-big-frames.md)).

## Cause

A Text with no height takes its text's own height, and Slint centres it in its parent.
Each new text is then measured again while the frame's changes are worked out. A page
turn changes ten rows of texts, so the dirty tracking measured about 70 texts.

## Change

`ui/overview.slint`: 47 Texts that can change get a height that draws the same pixels.
- A centred Text takes its parent's height.
- A Text placed with `y` gets 1.5 times its font size, since it is drawn from the top.
- The texts of the boot screen, screensaver and UPDATING FIRMWARE are left as they were.
- A comment at the top of the file states the rule.

## Evidence

- **PC** (`examples/render_screens.rs`, fastest of 1000 page turns): the dirty tracking
  went from 245 to 49 µs, and the preparation from 446 to 256 µs. All eight pages are
  pixel-identical to main.
- **Host:** tests and clippy for `idf-soak-autoplay idf-slint-ui`.
- **Device:** two profile images, sent by OTA. The owner did the same run on each:
  Library opened 4–5 times, 15–20 page turns with the browse knob, Settings once.
  - s245p is main `b945b42`.
  - s246p is this branch, `343aa34`.

  | Frame (rows drawn) | s245p | s246p |
  |---|---|---|
  | Library page turn (430), total | 55.8 ms (n=14) | 43.8 ms (n=16) |
  | its preparation | 30.5 ms | 18.2 ms |
  | its dirty tracking | 14.4 ms | 3.2 ms |
  | Library open (600), total | 69.3 ms (n=3) | 70.8 ms (n=2) |

  - Medians are shown.
  - Underruns were 0/0 on both. s245p's 30 ms fill is the known OTA keep write after an
    OTA boot. s246p's longest fill was 2186 µs.
  - Opening the Library did not change. Its 600 rows are mostly Slint's lines (36 ms)
    and the items (14 ms).
- After the run the board went back to s244, the play image.

## Open

- A page turn's items (12 ms, 8 of it texts) and Slint's lines (16 ms) remain.
- Opening the Library is still about 70 ms.
