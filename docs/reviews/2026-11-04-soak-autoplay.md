# JC1060: a 40-minute two-deck soak, and a cushion for Play after Load

Date: 2026-11-04. Branch: `ogranak/jc1060-soak-autoplay`, stacked on
`ogranak/jc1060-psram-200m` (`8b32524`).

## Why

The new configuration had not played for longer than 5 minutes:

- multi-sector SD reads;
- QIO 80 MHz flash;
- 200 MHz PSRAM.

A long run needs both decks to keep playing past the end of their tracks
without anyone at the controller. The board has no command to load a track
from the PC; the serial console only takes `R` (restart).

## Soak DJ (`soak-autoplay`, diagnostic only)

- **Build and features:**
  - `tools/build_legacy.py --variant deck-engine --soak-autoplay` builds with
    the Rust feature `idf-soak-autoplay`.
  - The product build has none of it.
  - The option excludes the profiling diagnostics, and the manifest records it.
- **Track order (`rust-core/src/soak_autoplay.rs`):**
  - Deck 1 walks the catalog's even indices and deck 2 its odd ones, wrapping
    at the end.
  - When a deck's track plays out, that deck loads its next track. It then
    presses Play through the same product deck state a controller Play goes
    through. The other deck keeps playing.
  - The load task publishes the catalog size once its catalog pass is done.
- **Failures and the operator:**
  - A failed load, or a Play the engine does not confirm, moves on.
  - After 3 in a row the deck is left alone.
  - A deck the operator paused stays paused.
  - Losing the media returns the deck to idle.
- **Where it runs:** on the USB client loop's MIDI refresh, outside the USB
  audio fill.
  - A first version ran inside the fill. That raised the fill's static
    stack chain from 3040 B to 4096 B, exactly at the guard, through
    `Engine::control`.
  - Moved out of the fill, the chain is back to 3040 B.
- **Tests:** 9 host tests cover:
  - catalog order and wrap;
  - the next load while the other deck plays;
  - an operator pause;
  - failed loads and the stop after 3;
  - a failure count reset by a success;
  - an unconfirmed Play;
  - a refused load;
  - losing the media.

## Soak (s21)

The image is `deck-engine --soak-autoplay` with QIO 80 MHz flash and 200 MHz
PSRAM (app `14cb300b…`). The FLX4 was connected and nobody touched it.

| | |
|---|---|
| Duration | 40 min (2394 s of streaming, 399030 fills) |
| Track loads | 22 (2 initial, 20 after a track played out), tracks 0 to 23 |
| Failed or refused loads | 0 / 0 |
| Longest fill | 3017 µs; 1 fill over 3 ms, none of 6 ms or more |
| `packet_errors` / `iso_errors` / `submit_errors` | 0 / 0 / 0 |
| Faults, resets | none |
| Underruns | 7/1 |
| PSRAM minimum | 27.99 MB, unchanged after the first loads |
| Internal RAM minimum | 408972 B |
| USB task stack free | 4928 B |
| Load task stack free (lowest) | 47900 B |

The USB detach in the log happens at boot, before the catalog. It is in
every run since s15.

**Every underrun came right after an automatic load:**

| Deck | Track | Underruns |
|---|---:|---:|
| 2 | 9 | 1 |
| 1 | 8 | 3 |
| 1 | 14 | 2 |
| 1 | 18 | 2 |

The other 16 loads had none, and no underrun came mid-track.

- **Cause:** a deck's producer publishes Ready once it has opened the new
  track's ring epoch with one decoded block. The soak DJ pressed Play at
  once, so the deck rendered while the producer was still priming.
- **Same problem as seeks:** this is what a Hot Cue seek did before the seek
  cushion. A DJ who presses Play right after Load would hit it too.

## Fix: a load starts with the seek cushion

`deck_engine.rs`: when a load commits, the deck's cushion is set to the new
ring epoch. This is the same cushion a completed seek uses. A Play then waits
until the ring holds 4096 frames (about 93 ms), with a cap of one second. A
Play pressed later finds the ring already full and does not wait.

- **New host test:** `a_play_right_after_a_load_waits_for_a_buffered_cushion`.
  - A Play right after a load with only its first block buffered is held,
    with no underrun and nothing consumed.
  - Once the producer catches up, the track plays from its start.
  - Without the fix the test fails with 5 underruns.
- **Feature sets:** tests pass for `idf-media-bridge` 20, `idf-exfat-fixture`
  29, `idf-media-load` 44, `idf-deck-play` 63, `idf-deck-mix` 64+24,
  `idf-deck-engine` 80+24, `idf-producer-profile` 87+24 and
  `idf-soak-autoplay` 89+24. Clippy passes with `-D warnings` for all of them.

## Soak with the cushion (s22)

The image is the same soak build with the load cushion (app `08b3bff9…`). The
FLX4 was connected and nobody touched it.

| | s21 (no cushion) | s22 (load cushion) |
|---|---:|---:|
| Duration | 40 min | 35 min (2094 s of streaming) |
| Fills | 399030 | 349028 |
| Track loads (automatic) | 22 (20) | 18 (16) |
| Failed or refused loads | 0 / 0 | 0 / 0 |
| Underruns | 7/1 | **0/0** |
| Longest fill | 3017 µs | **2769 µs** |
| Fills over 3 ms | 1 | **0** |
| `packet_errors` / `iso_errors` / `submit_errors` | 0 / 0 / 0 | 0 / 0 / 0 |
| Worst prime, FLAC / MP3 (`prime_max_us`) | 22 / 28 ms | 21 / 38 ms |
| PSRAM minimum | 27.99 MB | 27.99 MB |
| USB task stack free | 4928 B | 4792 B |
| Load task stack free (lowest) | 47900 B | 47900 B |

- **Start of each track:** every track started without an underrun.
- **Faults:** there were none. The only USB detach is the usual one at boot.

## Open

- **Real controller use:** this soak exercised loads and play-through, not
  seeks, scratch or EQ. A long run with a DJ at the controller is still to
  do.
- **Untagged VBR MP3** seeks still decode from the start.
