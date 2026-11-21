# JC1060: does a keylock start make the longest Master Tempo fills?

Date: 2026-11-21. Branch: `ogranak/jc1060-keylock-start-profile`, from `main` at `f2a61fd`.

## Question

On s155 the longest Master Tempo fills reached 5.6 ms, and 6.0 ms with a scratch. In the
worst fill, the graph stage took 4.2 ms. The deck flags showed a deck playing through its
keylock, with no scratch, loop or replay. The halved search does not explain a spike that
size.

The suspect was the keylock starting over. A start happens on a jump, a source change, a
scratch starting or ending, or Master Tempo being switched. The keylock then refills its
window, pulling up to 64 source frames per output frame (`PULL_BURST`).

## What the profile now records

This is for profile builds only (`uac-profile`). The product image does not change.

- **`deck_mix`:** before the graph runs, it notes each deck that plays through its keylock
  while that keylock is not running. That deck's keylock starts over during this fill.
- **Reading it:** the C driver reads and clears the note after the fill, through
  `ruzina_legacy_keylock_starts()`. It sets deck flag bit 9 (512) on that deck.
- **New class:** `mt_start` (class 10). It takes priority over every other class, so a fill
  with a start counts only there.
- **Test:** the C driver test checks that bit 9 picks `mt_start` even when a Master Tempo
  scratch is also under way.

## On the board

Image s156 (app `d4124d57…`), about 10 minutes, 128 000 fills. The owner played to a
schedule:

1. Master Tempo on both decks, untouched.
2. Master Tempo on both decks, with hot cues, beat jumps and SYNC.
3. Master Tempo on deck 1, with a scratch on deck 2.
4. Master Tempo off.

There were 0/0 underruns, and no fill took 6 ms or more.

| Class | Fills | Over 3 ms | Mean | Longest |
|---|---:|---:|---:|---:|
| `mt_start` | 18 | 8 | 3018 µs | 5412 µs |
| `master_tempo` | 63 934 | 18.2 % | 2561 µs | 5296 µs |
| `mt_scratch` | 1 174 | 21.6 % | 2700 µs | 5564 µs |
| `scratch` | 596 | 10.9 % | 2382 µs | 4323 µs |
| `plain` | 51 074 | 0.2 % | 1939 µs | 4249 µs |

- **The worst fill** (5564 µs, graph stage 4054 µs) had deck 1 playing through its keylock
  (flags 387) and deck 2 scratching (flags 7). Neither deck had bit 9.

## Conclusion

A start costs about 0.5 ms more than a steady Master Tempo fill, but it is rare: 18 fills in
10 minutes. Spreading its refill would not move the longest fills. Those come while the
keylock plays steadily.

There is a larger gap between real play and the boot bench. A plain fill takes 1939 µs in
play and 1012 µs on the bench, which runs the same graph. That points to the fill waiting
on memory shared with the screen and the stick, more than to the keylock's own work. That
is the next thing to look at.

The start flag stays in the profile, for later measurements.
