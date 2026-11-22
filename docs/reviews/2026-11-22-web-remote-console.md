# JC1060: the Web Remote as a full console, upright and sideways

Date: 2026-11-22. Branch: `ogranak/web-remote-console`, from main `25f18ff`.

## Source

The owner sent a mock-up of a phone-sized Web Remote and asked for the page to follow it.
The page also has to work held upright and held sideways.

The mock-up is laid out top to bottom:

- the top bar: RUZINA, WEB REMOTE, the master clock and LIVE;
- tabs for DECK 1, DUAL / MIX and DECK 2;
- each deck's overview waveform;
- two deck cards (BPM, pitch, SYNC and tempo);
- the mixer: EQ knobs, filter, PFL, meters, faders, master, headphones and the two SMART buttons;
- each deck's pads (HOT, CUE, PLAY, 4B LOOP and CLEAR);
- a crossfader bar fixed at the bottom.

## Now

- **Every figure is real.** Where the page had no data, the API now gives it:
  - **`/api/wave?deck=N`** (new) gives the loaded track's overview as 400 columns, each its
    stretch's loudest peak from the analysis preview, in hex. Each deck's status gives a
    `track_key`, and the page fetches the overview once per track. An answer for another
    track is dropped, and the page asks again a second later.
  - **The deck status** also gives:
    - `tempo_x100`, the playing tempo to a hundredth of a BPM;
    - `level`, the channel meter's 0 to 10 lit segments, with the same steps as the screen's
      meter, from its own peak counter so the screen and the LEDs keep theirs;
    - `hot_cues`, the pads that hold a cue, as bits.
  - **`action=hot_cue&value=1..8`** taps a Hot Cue pad as the controller does: it jumps to
    the cue, or sets one where the pad has none.
- **Left out:** what the deck does not have is not on the page:
  - the mock-up's ASIO figure; LATENCY is the page's own round trip to the deck, smoothed;
  - REC;
  - the settings gear;
  - BEATMATCH ALIGN;
  - the fader's dB marks (the scale shows plain ticks).
- **Master clock:** it shows the sync master's tempo. With no sync master it takes a playing
  deck's tempo, and otherwise deck 1's.
- **Held upright** (as in the mock-up), from top to bottom:
  - the overviews, one above the other;
  - the two deck cards side by side;
  - the mixer;
  - the two pad panels side by side;
  - the library.
- **Held sideways or wide** (a landscape phone from 640 px wide, or any screen from 1000 px):
  - the overviews sit side by side, with DECK 1 | MIXER | DECK 2 under them;
  - each deck's pads sit under its card, and the mixer runs the full height.
  - On a low screen (sideways phone, up to 520 px high):
    - the tabs join the top bar;
    - the line with track names and latency goes;
    - the crossfader bar is a single line;
    - the overviews, faders and pads are a step smaller.
- **The tabs:** DECK 1 and DECK 2 show one deck, its channel strip, the master section and
  the crossfader. The page remembers the choice in the browser.
- **Knobs:** EQ HI, MID and LOW are knobs. Drag up or down to turn one, and double-tap to
  centre it. Each keeps its range input for the keyboard and screen readers. EQ ▾ folds a
  channel's knobs away.
- **Overview:** the part already played is bright and the rest dim, with a white playhead.
  Between polls a playing deck's playhead moves on at its tempo, for at most a second. A tap
  on the overview seeks, as the old bar did, with the track's load generation.
- **Hot Cue pads:** a pad lights in the deck screen's cue colour (A to H) while it holds a
  cue.
- **PLAY** shows what a tap does next: PLAY / READY while paused, PAUSE / ACTIVE while
  playing.
- **Size:** the largest asset is `style.css`, at about 22 KB (the old page's largest was 12 KB).
  Each connection's answer buffer is the largest asset plus 1 KB, in PSRAM.

## Tests

- **ruzina-web:**
  - the status JSON with the new fields;
  - the wave as hex, and an overflow;
  - `/api/wave` for a deck with an overview, one without, and a bad deck;
  - `hot_cue` as a Hot Cue pad tap, with pads 0, 9 and non-numbers refused;
  - the page uses `/api/wave`.
- **The page** (`node --test`):
  - the existing ten cases still pass. The seek case now taps the overview.
  - The tempo shows to a hundredth, and the master clock follows the sync master.
  - The overview is fetched once per track, and a stale answer is retried a second later.
- **The legacy host tests and clippy**, and target clippy on the four feature sets.
- **The desktop simulator and ruzina-ota** (other `WebBackend`s) build and pass unchanged.
  The simulator gives `tempo_x100` too.
- **In a browser,** against a stand-in deck at:
  - 360×740 and 390×844 upright;
  - 844×390 sideways;
  - 768×1024 (tablet);
  - 1280×800.

  At each size there was no sideways scroll and no console error, and the knob drag and the
  tabs worked.

## Open

- The manual's Web Remote screenshots (`docs/manual/web-remote-*.webp`) show the old page.

## On the board

- **s182** (app `818c8df5…`): s181 (the screen redesign, the Library filters and the FX
  blink) with this branch merged on a local test branch.
  - The flash read back to match.
  - The Web Remote started with a 23 267-byte answer buffer.
  - The owner checked the page on the phone, upright and sideways, and says "Sve radi".
