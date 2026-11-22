# JC1060: the screen redesign on the fill gap, and what the FLX4 day changed

Date: 2026-11-22. Branch: `ogranak/jc1060-redesign`, rebased on main `232ab00`, after
[the fill gap (#165)](2026-11-22-fill-gap.md).

Before this, the redesign was described in:

- [the Rust palette](2026-11-21-rust-palette.md);
- [the layout and the phase strip](2026-11-21-screen-layout.md);
- [the Library filters](2026-11-21-library-filters.md).

## The rebase

The rebase onto #165 had no conflicts.

- **The lanes** take the fill gap's code at their new geometry by themselves: the lane
  constants feed the strip, the PPA copies and the gate.
- **The phase strip** was new with the redesign. It still wrote its 814×36 pixels to
  PSRAM with the CPU, every frame. Its rows now go through the strip in internal RAM, and
  the PPA copies them in, in blocks of 20 rows.
  - Without a strip, or once a copy fails, the CPU writes them and flushes, as before.

## What the owner saw, and the changes

**s183** (app `a09f5135…`): the lanes and the phase strip looked like they ran at a very
low frame rate.

**The gate waited whatever it cost the frame.** The s169 and s170 logs show it:

- Frames are counted in 20 s windows; the panel refreshes 61 times a second, and a frame
  takes two refreshes, so at most 610 frames fit a window.
- With the gate on, the screen drew 400–480 frames a window; with it off, about 575.

**Change: the gate waits only within the frame's slot.**

- A frame's slot is two refreshes, measured on the board.
- It may wait for gaps only for what the slot leaves after its usual work and a 2 ms
  margin, and only while a wait of a fill period (3 ms on average) still fits.
- **s184p** (app `13b564e7…`): 556–587 frames a window, with 0.5–0.8 ms of waits per
  frame. The stutter stayed, so the waits were not the only cause.

**Next cause: the position the lanes are drawn at.**

- The engine stores each deck's position once per fill.
- A frame reaches the panel 20–40 ms after the screen read that position, a different time
  each frame.
- At the largest zoom, a lane moves about 30 px a frame, so the uneven steps show.

**Change: each deck is drawn where it will be when the frame shows.**

- The live snapshot now carries the time it was stored (`deck_view::Live::stored_us`).
- `deck_view::ahead` carries a playing deck's position forward, at its pitched tempo, to
  the refresh the frame goes up at.
  - It looks at most 40 ms ahead.
  - It does nothing while a deck is paused or looping, or when the frame's time comes
    before the snapshot's.
- A host test covers it.
- **s185p** (app `f86fa512…`): the owner said it still stutters.
  - The log shows 554–579 frames a window. The frame is about 30 ms against a 33 ms slot,
    so about one frame in twelve misses its refresh.

**The owner's asks on s184p–s185p:**

- **Thinner phase strip:** 24 px high instead of 36. It sits between the lanes, at y 228.
- **Taller lanes:** 814×220 at y 8 and y 252, so the bar triangles touch the phase strip.
  - The deck columns follow, and so do the meters (y 48 and 292).
  - The offset box beside the strip is now 24 px high. With room for one line, it shows
    the offset alone, without the word PHASE.
- **FX column in the effect's colour**, as on the donor's overview
  (`ui_overview_fx_effect_color` in `ui_overview.c`):

  | Effect | Colour |
  | --- | --- |
  | FILTER | `#2D85CD` |
  | ECHO | `#EB870F` |
  | FLANGER | `#B44AE0` |
  | DELAY | `#27C7C7` |

  - While the effect is on, these parts take its colour: the panel edge, the FX label
    (which still blinks, now in that colour), the effect, the routed channels, the beat
    and the depth.
  - Off, they stay dim.

**Change: the offset label changes at most four times a second.** It changed by a
millisecond almost every frame.

**s186p** (app `dafb0b2c…`): the owner says "Sve dobro".

- The log shows 558–584 frames a window, about 28 fps, the same as s185p. The label
  change did not shorten the frame.
- Over the 15 minutes there were 0/0 underruns, no panics, and the longest fill was
  4612 µs. No plain fill went over 3 ms. Master Tempo was not on during this run.

## Measurements

The Master Tempo figures are fills over 3 ms and the longest fill.

| Image | Frames per 20 s | Slint step mean | Lanes mean | Waits mean | Master Tempo |
| --- | --- | --- | --- | --- | --- |
| s170 (gate on / off) | 400–480 / ~575 | 4.7 ms | 18.4 ms | 1.5 ms | 5.5 % / 14.0 %, 4.2–4.3 ms |
| s184p | 556–587 | 4.7–7.1 ms | 17–21 ms | 0.5–0.8 ms | 22 %, 4.6 ms (first minutes) |
| s185p | 554–579 | 4.7–7.1 ms | 17–22 ms | 0.6–0.8 ms | |
| s186p | 558–584 | 5.4–5.8 ms | 18.5–20.5 ms | 1.8–2.7 ms | not on |

- With the slot limit, the gate keeps less of the fills out of the frame than on s170:
  about 22 % of Master Tempo fills went over 3 ms on s184p.
- The longest fill stayed under the 5 ms goal, and there were no underruns.

## Open

- **The frame is still close to its slot**, about 28 of a possible 30.5 frames a second.
  - Two lane moves by the PPA take about 8.2 ms each.
  - The Slint step takes 5–7 ms a frame on the Overview (with its updates, about
    7.5 ms), for reasons not yet found.
  - A frame that never misses its refresh needs about 5 ms less work. 60 frames a second
    would need the lane moves made much cheaper.
- **Master Tempo** was not measured with the latest image.

## Tests

- `deck_view`: `a_playing_deck_is_drawn_where_it_will_be_when_the_frame_shows`.
- The legacy host tests (173 and 25), fmt and host clippy pass, and so does target clippy
  on the Slint, profile, still-screen and deck-engine feature sets.
