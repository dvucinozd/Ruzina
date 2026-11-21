# JC1060: the deck screen's new layout

Date: 2026-11-21. Branch: `ogranak/jc1060-redesign`, after [the Rust palette](2026-11-21-rust-palette.md).

## Source

This is the second step of the screen redesign. The layout follows the donor firmware's
ui-next Perform page, as agreed with the owner: an icon rail instead of tabs, larger lanes
with the phase strip between them, and deck cards at the bottom. Our own elements stay.
The owner's mockup is the "Ruzina deck screen" design canvas.

## Now

- **Rail:** a 48 px rail at the left, with icons only, for Overview, Library, Hot Cues,
  FX and Settings, 120 px each.
  - The open page is shown by a rust bar and a rust icon; the others are grey.
  - The icons are 26 px white masks (`ui/icons/make.sh`, ImageMagick) that Slint
    colours, so they add no font size.
  - The owner narrowed the rail from 72 px on s175: it had too much empty space around
    the icons.
- **Lanes:** two lanes of 814×210 at x 136, y 8 and y 262 (they were 872×201).
  - Left of each lane: D1/D2, PLAY, CUE and the cover in a 66 px column.
  - The meter sits at x 122, centred on the lane.
- **Phase strip:** the 36 px between the lanes holds the phase strip's place, with its
  PHASE label. The strip itself is the next step.
- **FX panel:** right of the lanes, 64×464. While the effect is on, its FX label blinks
  green (`#00FF66` and dark green, every 400 ms, a Slint `Timer`), as the owner asked;
  off, it stays dark.
- **Deck cards:** two cards of 482×116 at the bottom, on every page.
  - Each card has the deck's colour bar and title, then elapsed, remaining, BPM (rust),
    pitch, MT, Q, the key and the deck status, then the overview of 466×45.
  - They replace the full-width title bar and the info rows.
- **Pages:** Library, Hot Cues, Settings and FX open right of the rail, 952×474, above
  the cards.
  - Their columns are narrowed to fit, and nothing else on them changes.
  - On Hot Cues the pads are taller (200 px).
- **Rust's areas:** `LANES`, `METERS` and `MINIS` in `slint_ui.rs` match the dark
  rectangles in `overview.slint`. The waveform's height and time scale follow
  `LANE_WIDTH` and `LANE_HEIGHT`.

## The phase strip

The third step fills the strip between the lanes. Rust draws it every frame, like the
lanes, as a direct area (`PHASE`, 814×36 at y 222).

- **Beats:** deck 1's beats are 2-column ticks in rust in the upper half, deck 2's in
  cyan in the lower half. A line runs between the halves, and the playhead is in the
  middle.
- **Scale:** the strip spans four beats of the master in real time, each deck's pitch
  included (`deck_phase::beat_columns`). When the decks are in phase, their ticks stand
  above one another.
- **Master:** deck 1, unless deck 1 alone has SYNC on, in which case deck 2 is master.
- **PHASE label:** shows how far the other deck's beat is ahead of the master's, at the
  master's tempo and within half a beat (`deck_phase::beat_offset_ms`), for example
  "+12 ms" in neon yellow. Without both beat grids it shows a dash.
- **The maths:** it comes from the local `ogranak/jc1060-deck-link` branch, whose link-bar
  figures the owner dropped; only the beat offset was kept.

## Tests

- The legacy host tests and clippy, with the Slint, Web Remote, profile and screen-still
  feature sets on the target.
- `deck_phase`: the beat offset (three tests) and where beats fall on the strip,
  pitched or not.
- The layout has no host test; the board is the check.

## On the board

- **s175** (app `13c5c47a…`), with the 72 px rail: the owner asked for a narrower rail.
- **s176** (app `72498c59…`), with the 48 px rail and 814 px lanes: the owner says
  "Izgleda dobro". A short boot log has no errors.
- **s177** (app `4041be8c…`), with the phase strip, the soak DJ playing: the owner says
  "Izgleda dobro". The log has no errors.
- **s181** (app `f47e264d…`), with the blinking FX label: the owner says "Sve radi". The
  log has no errors.
