# JC1060: review fixes and a soak with every control

Date: 2026-11-06. Branch: `ogranak/jc1060-codex-fixes`, from `main` at `5fb588e`.

## Review fixes

Codex reviewed the merged #94, #95 and #96 after they went in, and found three real
defects:

- **Scratch and loop adjust (#94):** touching the jog in loop adjust started a scratch
  and silenced the deck, while the jog was editing the loop. Scratch now leaves that
  mode alone.
- **Pad FX (#95):** a pad pressed while a filter pad was still fading out was cancelled
  256 frames later. The runtime wrapper hid this on the board, but the shared `PadFx`
  API was wrong.
- **Hot cues (#96):** a bank offered to the flash owner left the engine's queue, and the
  owner waits for silence. Loading that track on the other deck during playback read the
  older bank from flash and lost the edit. The engine now keeps the bank until the owner
  reports it done, and offers one at a time.

## Soak DJ

`soak_autoplay.rs` gains `Exerciser`. Every 1 to 4 s it does one of 12 gestures on a
playing deck, through the same path as a controller event: scratch, set or jump to a cue,
delete a cue, loop, Beat Jump, Pad FX 1 and 2, Beat FX, EQ and filter, CUE, Sync, and
pausing both decks, which gives the flash journal its window. The product build has
none of this.

## Result (`s40-soak-dj-*`, app `c8414441…`)

| | |
|---|---:|
| Length | 45 min |
| Gestures | 626, at least 14 of each per deck |
| Seeks | 119 |
| Tracks loaded, played to the end | 5, 3 |
| Underruns, `iso_errors`, `packet_errors` | 0/0, 0, 0 |
| Faults, journal failures | 0, 0 |
| Longest fill | 4466 µs |
| Fills over 3 ms | 2556 of 449030 (0.6 %) |
| Fills of 6 ms or more | 0 |
| Flash writes | 44, median 10.9 ms, longest 21.7 ms |
| Longest seek or prime | 50 ms |

- **Fill time:** 4466 µs is the longest yet, and it rose during a cue delete. It is under
  the agreed 5 ms limit, so the graph stays out of IRAM for now, but the margin to the 6 ms
  budget is down to 1.5 ms.
- **Flash:** the two longest writes took 18.8 and 21.7 ms, so they may have erased a sector;
  the log does not say. All writes happened with no deck playing.

## Evidence

- **Host:** 2 new tests for the fixes, one of them in `ruzina-audio-dsp`, a longer hot cue
  test, and 2 for the soak DJ. All 8 legacy feature sets, target clippy on 6 of them, and
  the `ruzina-audio-dsp` and `ruzina-audio-runtime` tests and clippy pass.
- **Build:** soak `deck-engine`, 876560 B. UAC fill chain 3408 B, load chain 51712 B.
- **Production image** (`s41-production-*`, app `8a264d4d…`, 874160 B), owner verdict "Radi":
  - **In the log:** scratch on both decks, Pad FX 1 on deck 1, Hot Cue pads on deck 2, a
    loop, and the same track loaded on both decks.
  - **Log:** underruns 0/0, `packet_errors=0`, no faults. Longest fill 4426 µs, 8 of 23029
    fills over 3 ms, none of 6 ms or more.

## Not covered

- **Sound:** nobody listened to the soak. The owner's verdict on the production image rests
  on a shorter run with no Beat FX in its log.
- **The review fixes on the device:** two of the three are proven on the host only. The soak
  DJ does not reach them, and the production log has no entry into loop adjust and no cue
  set on deck 1 before that track loaded on deck 2.
- **Sync:** the deck gets no analysis on this firmware, so Sync matches tempo from the BPM
  and does not align beats.
- **Tempo fader and channel faders:** the soak DJ does not move them.
