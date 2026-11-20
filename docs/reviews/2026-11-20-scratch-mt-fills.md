# JC1060: scratch and Master Tempo fills after the faster render

Date: 2026-11-20. Branch: `ogranak/jc1060-scratch-mt-profile`, from the
[fill stack](2026-11-20-fast-render-soak.md) branch.

## Question

The owner had accepted long USB audio fills while scratching and with Master Tempo
([scratch fill accepted](2026-11-13-master-tempo.md)): up to about 6.3 ms during a fast
scratch, about 6.2 ms with Master Tempo alone (over 40 % of fills above 3 ms), and up to
7.0 ms with Master Tempo on one deck and a scratch on the other. After the
[faster render](2026-11-19-fx-fill-cost.md) the question was which of these still hold.

## Change (profile builds only)

The fill profile kept scratch and Master Tempo in one class, `scratch_mt`. It now has
three:
- `scratch`: a scratch on either deck, no Master Tempo;
- `master_tempo`: Master Tempo on either deck, no scratch;
- `mt_scratch`: both, on either deck.

Master Tempo counts when the deck has it on, whether or not that deck is playing. The
product build has none of this.

## On the board

s150 (app `c50382e9…`) is the `--usb-stick` profile image of the fill stack branch with
this change; its fill chain needs 3120 B of its 4096 B budget. The soak DJ played both
decks, scratching on its own, while the owner switched Master Tempo:
- minutes 0 to 3: nothing;
- 3 to 7: Master Tempo on deck 1, with the tempo moved;
- 7 to 10: on both decks;
- 10 to 12: on deck 1, with fast jog scratches on deck 2;
- 12 to 14: off.

14 minutes, 135 000 fills, 0/0 underruns, no packet or isochronous errors, no fault:

| Class | Fills | Mean fill | Mean graph | Over 3 ms | Longest | Accepted before |
|---|---:|---:|---:|---:|---:|---:|
| `plain` | 37 513 | 1958 µs | 1400 µs | 0.19 % | 4601 µs | |
| `fx_echo` | 1679 | 2419 µs | 1829 µs | 6.4 % | 3681 µs | |
| `pad_fx` | 4253 | 2144 µs | 1571 µs | 0.68 % | 3432 µs | |
| `scratch` | 295 | 2393 µs | 1799 µs | 11 % | 4218 µs | 6.3 ms |
| `master_tempo` | 83 618 | 2560 µs | 1997 µs | 23 % | 5471 µs | 6.2 ms, over 40 % |
| `mt_scratch` | 2935 | 2769 µs | 2190 µs | 28 % | 6016 µs | 7.0 ms |

The profile itself lengthens every fill by roughly a tenth.

## What it means

- A scratch no longer needs its exception: its longest fill is 4.2 ms, under the 5 ms
  rule.
- Master Tempo is the largest cost left. Its keylock (`deck_keylock.rs`, a WSOLA
  stretch) runs inside the render's source stage and adds about 0.6 ms to the mean
  graph. With it, 23 % of fills are over 3 ms and the longest is 5.5 ms alone, 6.0 ms
  with a scratch, against 6.2 and 7.0 ms before.
- The next step is the keylock's own cost.
