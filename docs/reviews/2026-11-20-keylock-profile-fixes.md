# JC1060: keylock and fill profile fixes from an outside review

Date: 2026-11-20. Branch: `ogranak/jc1060-keylock-profile-fixes`, from `main` at `d964a67`.

## Source

A code review by another AI agent of `main` at `7fa9b84` listed eleven findings. Each was
checked against the code, and all eleven hold. This change takes the three that touch the
keylock and the fill profile, the ones the next measurement depends on. It also takes the
review's advice on the [halved search](2026-11-20-keylock-search.md): a lasting test on
fixed signals.

## The search may read the run before it (finding 7)

At the start of a run the keylock's positions begin at `HOP`. The search's earliest start
was `HOP`, and the fine search reaches 3 frames before that. Those frames are not this
run's. The window still held them from the track or run before, since `stop` does not
clear it.

It only mattered at the slowest tempos, under about 0.508, at a run's first hop, and when
the search had nothing to choose by. In that case every candidate scores the same and the
earliest one wins: silence, or channels that cancel in mono.

- **Change:** the earliest start is now `HOP + THIN`. The fine search's reach then stays
  within the run.
- **Test:** `a_run_does_not_hear_the_run_before_it` plays one keylock round its whole window
  first. It then compares that keylock with a fresh one on a track whose channels cancel.
  Without the change it fails at tempo 0.5, frame 513, as the review found.

## The profile's Master Tempo class (finding 8)

The fill profile's `master_tempo` and `mt_scratch` classes looked at the Master Tempo
setting. A deck that scratches or stands still does not play through its keylock, so a
scratch on a Master Tempo deck counted as `mt_scratch` without any keylock running.

- **Change:** `deck_mix` publishes, for profile builds only, whether each deck plays
  through its keylock. The deck flags carry it as bit 8 (256). The setting stays bit 7
  (128).
- **Classes:** `master_tempo` and `mt_scratch` now go by bit 8, so they count fills with a
  keylock actually playing. A scratch on a Master Tempo deck counts as `scratch`.

## The Beat FX tail after 71.6 minutes (finding 9)

The profile keeps the 3 s after Beat FX goes off as `fx_tail`. It compared 32-bit
microsecond uptimes, which come round every 71.6 minutes. Without another Beat FX, the
difference fell under 3 s again once a round later, and plain fills counted as tail.

- **Change:** once the 3 s have passed, the tail ends for good.
- **Test:** the C driver test adds the case of a full round later.

## The fine search back to every 4th frame

The review asked for lasting tests on fixed signals, bright ones among them, for the halved
search. `a_bright_tone_stays_whole_across_the_grain_joins` stretches harmonics of 1200 Hz up
to 8.4 kHz. A perfect stretch repeats every 40 frames, so the test measures what the joins
break.

Residual energy, tempo 0.87 and 1.13 / 0.97 and 1.03:

| Compared frames, coarse / fine | Residual |
|---|---:|
| 8 / 4 (before #151) | 0.0114 / 0.0025 |
| 16 / 8 (#151) | 0.0148 / 0.0041 |
| 16 / 4 (now) | 0.0114 / 0.0025 |

The coarse search loses nothing at every 16th frame, but the fine search lines up the high
frequencies. The fine search therefore compares every 4th frame again. The test fails
above 0.0125. The fine search is the smaller part of the search, so most of #151's saving
stays. The board has not measured this.

## Checks

The legacy host tests, the C driver tests, and clippy for all target feature sets and the
host all pass. Not flashed.

The other eight findings are open. In the order the review proposed:
- the Web Remote's load and seek outcomes (1, 4, 5);
- the USB stick's semaphores and reset recovery (2, 3);
- then 6, 10 and 11.
