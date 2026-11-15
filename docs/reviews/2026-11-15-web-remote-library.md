# JC1060: the Web Remote, titles and the library

Date: 2026-11-15. Branch: `ogranak/jc1060-web-library`, from `main` at `a081d0b`
(after [PLAY and CUE](2026-11-15-web-remote-play-cue.md)).

## Change

- **Titles and durations:** `deck_view::publish_track` also hands the Web Remote its own
  copy of each deck's track: catalog index, title, artist and duration
  (`take_web_track`).
  - The status shows them while the engine has the deck loaded.
  - An empty deck shows nothing again.
- **The library:** `deck_view::publish_library` also hands the Web Remote a copy of the
  catalog with a revision of its own (`take_web_library`); the screen keeps the
  original.
  - `/api/library` pages through it, with title (the artist when the title is empty),
    artist, whole BPM and duration.
  - Each row's `track_key` is its catalog index plus one.
  - The status's `library_revision` is that revision.
- **Loads:**
  - `/api/load` checks the revision (409 if the list changed), the track (404) and the
    load lock (409 for a playing deck).
  - It then stores the catalog index plus one in `deck_view::WEB_LOADS` for that deck,
    one in flight per deck (409 Busy otherwise).
  - The USB task takes it next to the web taps and calls `Engine::request_load`, the same
    admission as a LOAD.
  - The screen's selection does not move.
- The web task keeps the catalog and the tracks between requests (`WebState`).

## Review fix (Codex)

**A load the engine was still running was missed.** `WEB_LOADS` empties as soon as the
USB task takes a request, not when the engine finishes the load. So a second web load,
or one after a controller LOAD, answered 200 while `Engine::request_load` refused it
silently.

The USB task now publishes `deck_view::LOAD_PENDING` per deck after every turn, from the
engine's own pending load, whoever asked for it. While it is set, the Web Remote answers
409 Busy. The library test covers it.

What remains is a controller LOAD in the same millisecond as a web load. That is not
reported back.

## Evidence

- **Host:** two new tests.
  - The page lists three tracks with their keys and BPM. A load queues index 1 on deck 2
    and leaves the screen's selection. A second load is busy, a stale revision answers
    409, an unknown track 404, and a playing deck 409.
  - A loaded track's title and duration appear in the status, and go once the deck is
    empty.

  The legacy CI matrix, the Slint checks and target clippy, also with `idf-web-remote`,
  pass. The IDF build has no warnings.
- **Device:** s108 (app `af4a00be…`), owner verdict "sve je uredno": titles and progress,
  the library, a load, and a load refused on a playing deck.
  - The test's capture: 142 requests, the longest 35 ms. Underruns 0/0.
  - The board had then run 3.3 hours without a reset, mostly without a capture. Its
    counters over that time show 19 lost USB packets and a longest fill of 25.8 ms. The
    first 20 minutes, captured, had none and 6.1 ms. After hours of soak Hot Cue saves
    the journal erases during play again
    ([Hot Cue erase-ahead, Open](2026-11-13-hot-cue-erase-ahead.md#open)), which is the
    likely cause, not shown.

## Open

- Seeks and the mixer, SYNC, PFL and faders still answer 503.
- The long-run packet losses above: the Hot Cue journal's rotation.
- One request on s107 took 1.4 s, most likely a connection the phone opened and filled
  late. The server reads one connection at a time, with a 2 s receive timeout, so such a
  connection holds the others back.
