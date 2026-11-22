# JC1060: the deck screen in the owner's Rust palette

Date: 2026-11-21. Branch: `ogranak/jc1060-redesign`, from `main` at `7a5ea74`.

## Source

The first step of the screen redesign agreed with the owner: the colours of the owner's
palette spec (`dj_kontroler_gui_specifikacija_palete_boja_rust_baza.md`, Rust `#B7410E`
base), before the layout changes (icon rail, larger lanes, phase strip). The Web Remote
has used the same palette since #160.

## Decisions taken with the owner

- **Waveform by frequency:** lows `#E64C0C`, mids `#0088FF`, highs `#00E5FF`, with white
  tips on loud peaks.
- **Hot Cues** (option 1 of the three shown):
  - A `#00FF66`, B `#00E5FF`, C `#FF0033` and D `#FFD600`, as the spec has them;
  - E `#A259FF`, F `#FF2EC4`, G `#2F6BFF` and H `#B6FF00`, which the spec leaves open.
- **Memory cues:** warm white `#F5F0ED`, no longer red, since red is Hot Cue C's.
- **Bar markers:** acid green `#39FF14`, the spec's beat grid colour, no longer red.

## Now

- **Background:** `#100C08`, not the spec's `#0F0B09`. RGB565 turns `#0F0B09` into a
  neutral grey (`#080808`); `#100C08` keeps the warm tone.
- **Shell:**
  - Cards are `#1E1511`, alternate rows `#181210`, borders `#38251E`.
  - Text is `#F5F0ED`, secondary text `#A3958E`, disabled `#574B45`.
- **Decks:** deck 1 is `#E64C0C`, deck 2 `#00E5FF`.
- **Tabs and choices:**
  - The active tab is outlined in `#E64C0C` on dark rust.
  - Selected choices are filled with Rust `#B7410E`.
  - Section headings are `#E64C0C`.
- **Transport and status colours:**
  - PLAY is `#00FF66` while playing and Rust while paused. CUE is `#FFD600`.
  - The pitch box and Quantize ON are `#FFD600`, Master Tempo ON `#00FF66`.
  - Loops and the key are `#FF7700`. Delete is neon crimson `#FF2E63`.
- **Track title bar:** a `#1E1511` card instead of the old blue bar.
- **Meters:** acid green `#39FF14` up to 0 dB, deep coral red `#E6193C` above it, laser
  red `#FF0033` at the top, and `#2A1B14` for unlit segments.
- **Lane:**
  - Background `#100C08`, beat lines `#38251E`, bar lines `#574B45`.
  - The playhead is warm white.
  - Loop edges are `#FF7700`, with the loop shade unchanged.
- **Waveform colours:**
  - The lanes come from Rekordbox's detail waveform (PWV3), which has no bands, only
    how white each column is, 0 to 7. Whiter means brighter sound, so 0–2 draw as lows,
    3–4 as mids and 5–7 as highs. This replaces the donor's six-colour hint palette.
  - Where an overview has Rekordbox's colour columns (PWV4), whose red, green and blue
    are bass, mid and treble, the three band colours are mixed in that measure.
- **Not in this step:** true per-band lanes need Rekordbox's colour detail waveform
  (PWV5), which the analysis import does not read yet.

## Tests

The legacy host tests and clippy, with the Slint, Web Remote, profile and screen-still
feature sets on the target. Colours have no host test; the board is the check.

## On the board

Image s174 (app `586c7ec6…`), the play image flags on this branch, without the FLX4.

- **Owner:** every page, the lanes, the overviews, the Hot Cue tiles and the meters
  "look good" ("Izgleda dobro").
- **Log:** a short boot with no errors.
