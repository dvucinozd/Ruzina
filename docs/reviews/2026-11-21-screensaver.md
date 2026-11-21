# JC1060: the boot screen and the screensaver

Date: 2026-11-21. Branch: `ogranak/jc1060-screensaver`, from `main` at `52abad4`.

## Source

The owner asked for the donor firmware's boot screen, which is also its screensaver
(Pajoniiir `splash_screen.c`, `dj_ui.c` and `ui_idle.c`). The donor shows its name with
the letters lit in turn, for 3 s at boot and again after 2 minutes with nothing touched
and no deck playing.

## What the owner chose

- **Look:** the owner's own wordmark from `firmware-rust/assets/branding`, its letters lit
  in turn as the donor does. Shown three ways (the emblem picture, the wordmark, DejaVu
  Sans Bold); the owner picked the wordmark.
- **Wait:** a Settings entry, OFF, 2, 5 or 10 minutes, kept on flash; 2 minutes by default.
- **The controller:** a button, pad, jog or browse turn only wakes the screen. Faders and
  knobs always act, so the mix stays where the controller's faders are.

## Now

- **Timing:** `screen_idle.rs` uses the shared `ruzina-ui-model::idle::IdleScreen`, the M2.5
  timing the desktop simulator already runs. A playing deck holds the screensaver off and
  counts as activity, so it does not show the moment a long track ends. Changing the wait
  restarts the count.
- **Boot screen:** the first 3 s of the screen task, without the caption. A touch does not
  cut it short. The wait counts from the deck's first frame after it.
- **Wake sources:** a touch on the screen task; the controller and the Web Remote on the USB
  client task, through two atomic flags the screen task reads once a frame.
  - **Touch:** the cover lies over the whole screen and takes the tap, so a waking tap
    presses nothing below it.
  - **Controller:** `controller_midi::deliver` asks `screen_idle::controller`. While the
    screensaver shows, a press, pad, jog or browse turn goes no further. A release, a fader
    or a knob goes on.
  - **Web Remote:** taps, controls, loads and seeks wake the screen and act, since the page
    shows what they do.
- **Drawing:** under the cover Rust draws nothing: no lanes, meters or overviews, and Slint
  keeps no area for them. They are drawn again in each buffer once it goes.
- **The letters:** pictures, cut from the wordmark at 720 px wide by
  `rust-core/ui/wordmark/make.sh`. Text at this size would not do: the software renderer
  embeds every character at every size it is used at. A 120 px text grew the generated
  screen code from 8.0 to 20.4 MB, about 3 MB of firmware, and it embeds only the
  regular face, not bold. The pictures add 0.35 MB to the image (3.36 MB against s171's
  3.01 MB).
- **The setting:** `ProductSettings::screensaver`, in byte 9 of the settings record, which
  was reserved and so 0. 0 means 2 minutes, so every older record reads as the default and
  the format stays version 1. Firmware from before reads any other value as a reserved
  byte set and falls back to its defaults, leaving the record as it is. The deck packs it
  in bits 16 and 17 of its settings word.

## Tests

- `screen_idle::tests` (5): the boot screen shows first and a touch does not cut it short;
  a touch wakes it and restarts the wait; a press, pad or turn only wakes it, while a
  fader and a release go on; the Web Remote wakes it; a playing deck holds it off, a
  longer wait counts from when it was set, and OFF keeps it down.
- `ruzina-settings`: the screensaver round-trips, an unknown value is refused, and a record
  from before (byte 9 is 0) reads as 2 minutes. `deck_settings`: every value survives the
  packing.
- Clippy on the target for the Slint, Web Remote, profile and screen-still feature sets.

## On the board

Image s172 (app `d0863dcc…`), the play image flags of s171 on this branch, without the
FLX4 (lent out).

- **Owner:** the boot screen, the Settings entry, the screensaver after 2 minutes, and
  waking it by touch and from the Web Remote all work ("Sve radi").
- **Log:** about 6 minutes, no errors; the settings record loaded as stored.
- **Not yet tested:** waking from the FLX4, which comes back later.
