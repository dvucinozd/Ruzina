# JC1060: the Master Tempo keylock's search compares half the frames

Date: 2026-11-20. Branch: `ogranak/jc1060-keylock-search`, from `main` at `7fa9b84`.

## Question

After the [spread search](2026-11-20-keylock-spread.md), Master Tempo was still the largest
cost in the fill. The product image of `main`, s154 (app `53c2f867…`, no profile), played
6 minutes with Master Tempo on one deck and then both, with scratches: 69 000 fills, 0/0
underruns, longest 5370 µs. That is still over the 5 ms rule.

In the s153 disassembly, a hop's search ran about 70 000 instructions per deck: 65 coarse
candidates of 64 compared frames, then 7 fine ones of 128. Spread over the hop's 512
output frames, that is about 140 instructions a frame. The crossfade is much less.

## Change

- The coarse search compares every 16th frame of the overlap instead of every 8th.
- The fine search compares every 8th frame instead of every 4th.
- The candidates do not change: every 8th start, then ±3 frames around the best.

The search does half the work. The output is no longer bit-identical, so a temporary
host test, not committed, compared the joins on a track-like signal at 44.1 kHz (kick,
bass, a chord, hi-hats). Over 60 s at tempos 0.92, 0.97, 1.03 and 1.08, about 5000 hops
each, it measured the picked start's correlation against the best start in reach:

| Compared frames, coarse / fine | Mean loss | Joins more than 0.05 worse |
|---|---:|---:|
| 8 / 4 (before) | 0.0003 | 0.94–1.30 % |
| 16 / 4 | 0.0005 | 1.02–1.37 % |
| 16 / 8 (now) | 0.0006 | 1.02–1.39 % |

At tempos 0.97 and 1.03 the mean loss is negative for all three: the fine search reaches
past the range the best was taken from. Every 32nd frame was clearly worse.

Legacy tests, clippy for all target feature sets and the host pass. The fill chain needs
3120 B in the profile image, as before.

## On the board

s155 (app `1d48d8e1…`) is the `--usb-stick` profile image of this branch.

Boot bench, µs a fill of 288 frames:

| Case | s153, mean / longest | s155, mean / longest |
|---|---:|---:|
| `keylock` | 642 / 801 | 517 / 641 |
| `graph_keylock` | 1599 / 1770 | 1477 / 1622 |

Real play followed the same 10 minutes as on s153 (the
[spread search](2026-11-20-keylock-spread.md#in-real-play)). It ran 113 000 fills with 0/0
underruns, no packet or isochronous errors and no fault:

| Class | Mean graph, s153 → s155 | Over 3 ms, s153 → s155 | Longest, s153 → s155 |
|---|---:|---:|---:|
| `plain` | 1375 → 1321 µs | 0.10 → 0.16 % | 3676 → 3900 µs |
| `scratch` | 1796 → 1582 µs | 12 → 10 % | 4275 → 4236 µs |
| `master_tempo` | 1948 → 1891 µs | 18.3 → 15.3 % | 5246 → 5581 µs |
| `mt_scratch` | 2125 → 2115 µs | 22.6 → 23.8 % | 5554 → 6022 µs |

The owner listened on the FLX4: Master Tempo sounds the same.

## What it means

- The search's cost is down: the keylock's bench mean fell by a fifth, and fewer Master
  Tempo fills are over 3 ms.
- The longest fills did not fall. In s155's worst fill, the render graph took 4167 µs
  against a usual 1.9 ms, with Master Tempo on both decks. The search cannot account for
  that.
- A likely cause, not yet checked: after a jump (SYNC, a loop, a scratch), a keylock starts
  over. Its first search then pulls several hundred frames from the deck in a few output
  frames, up to 64 a frame. That is the next thing to measure.
