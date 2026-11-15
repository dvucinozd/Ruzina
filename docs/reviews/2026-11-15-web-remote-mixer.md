# JC1060: the Web Remote, the mixer, SYNC and seeks

Date: 2026-11-15. Branch: `ogranak/jc1060-web-mixer`, from `main` at `fc26de0`
(after [titles and the library](2026-11-15-web-remote-library.md)).

## Change

- **Every control on the page now works:**
  - channel volume, EQ (low, mid, high), filter, crossfader, master and headphone mix;
  - SYNC, PFL and Tempo;
  - Smart CFX and Smart Fader;
  - the 4-beat loop and its clear.
- **Controls go as the controller sends them.**
  - PLAY and CUE still take the touch path.
  - Every other command goes, whole or not at all, into `deck_view::WEB_EVENTS`. This
    is a single-producer queue of `ControlEvent`s, 32 slots.
  - The USB task hands each event to `Mix::control`, as it does a MIDI event, so the
    engine and the mixer apply it.
- **The status carries the real mix.**
  - After every turn the USB task stores a `MixView` in `deck_view::MIX_VIEW`: the
    mixer's settings and each deck's SYNC and SYNC master.
  - The page's sliders and buttons show it.
- **Seeks:**
  - `/api/control?action=seek` stores the position, within the loaded track, in
    `deck_view::WEB_SEEKS`; the latest seek wins.
  - The USB task calls `Engine::web_seek`. That moves the deck as a beat jump does
    (`set_position_ms`) and asks the producer to reopen the source there.
  - Nothing happens during a scratch, inside a loop or without a track. A deck without a
    track answers "Deck not seekable".
- **USB task side:** all its Web Remote work is now one function, `web_remote::service`,
  called from `refresh_mix`: taps, events, loads, seeks, pending loads and the mix view.

## Evidence

- **Host:** three new or changed tests.
  - A volume command reaches the event queue, not the touch path.
  - Volume, crossfader, EQ, filter and PFL from the page reach a real `Mix` through
    `service`, and come back in `MixView` and the status.
  - A seek needs a loaded track, and stays inside it.

  The legacy CI matrix, the Slint checks and target clippy, also with `idf-web-remote`,
  pass. The IDF build has no warnings.
- **Device:** s109 (app `80bbb245…`), owner verdict "sve radi ok". The test covered the
  mixer from the phone, SYNC and a seek.
  - 274 requests, the longest 258 ms once, while a slider was dragged.
  - Underruns 0/0, `packet_errors=0`, longest fill 6.0 ms, USB task free stack 2832 bytes.
  - Internal RAM: 241 KB free.

## Open

- **A fader moved on the phone leaves the FLX4's fader where it was.** The physical
  fader's next move jumps to its own position; there is no soft takeover.
- **The server reads one connection at a time.** A slow connection holds the others
  back; the longest seen was 1.4 s (s107).
