# JC1060: the Master Tempo keylock's search spread over the hop

Date: 2026-11-20. Branch: `ogranak/jc1060-keylock-speed`, from
`ogranak/jc1060-scratch-mt-profile`.

## Question

With Master Tempo on, 23 % of USB audio fills were over 3 ms and the longest took
5.5 ms ([scratch and Master Tempo fills](2026-11-20-scratch-mt-fills.md)). The keylock
(`deck_keylock.rs`, a WSOLA stretch) added about 0.6 ms to the mean render graph.

## Measuring

The boot render bench (profile builds only) has three new cases:
- `keylock`: both decks' keylocks alone, one frame each for every output frame, at
  tempos 1.025 and 0.975, on the heap as `deck_mix` keeps them;
- `graph_keylock`: the render graph with both decks through their keylocks, as
  `deck_mix` plays Master Tempo;
- `keylock_internal`: `keylock` with the pair in internal RAM instead of PSRAM.

s151 and s152, µs a fill of 288 frames:

| Case | Mean | Shortest | Longest |
|---|---:|---:|---:|
| `graph` | 1009 | 1006 | 1047 |
| `keylock` | 737 | 375 | 1037 |
| `keylock_internal` | 805 | 426 | 1131 |
| `graph_keylock` | 1681 | 1341 | 2032 |

- **Memory:** internal RAM was no faster, so the keylock's cost is its computing, not
  PSRAM.
- **The search:** the cost swung from 375 to 1037 µs between fills. The search for the
  next grain's start compares 72 candidates (65 coarse, 7 fine) in the first 72 output
  frames of each 512-frame hop, one a frame. A fill that caught them took about 0.7 ms
  longer than one that caught none.

## Change

- **Spread search:** the search compares one candidate every `SPREAD` (7) output frames,
  so its 72 candidates take most of the hop. Once the hop has no more frames left than
  candidates, it compares one a frame. A search therefore finishes in exactly the hops
  where one a frame would, and picks the same start. Waiting for the frames is still
  tried every frame.
- **Window check:** before pulling, the keylock checks inline whether the window already
  holds the frame it reads, instead of calling `fill` every time.

The output is bit-identical. A temporary test ran both keylocks for 600 000 frames:
- at tempos from 0.4 to 2.3, beyond the clamp both ways;
- with stops;
- with a starving source on one deck;
- digesting every output frame, `lag` and `heard_lag`.

The digest was the same before and after. The test caught a broken spread: without the
catch-up at the hop's end, a spread of 9 frames changed the digest. It is not committed.
Legacy tests, clippy for all target feature sets and the host pass. The fill chain needs
3120 B.

## On the board

s153 (app `630a46ce…`), µs a fill:

| Case | Mean before | Longest before | Mean after | Longest after |
|---|---:|---:|---:|---:|
| `keylock` | 739 | 1061 | 642 | 801 |
| `graph_keylock` | 1699 | 2065 | 1599 | 1770 |

The spread took about 0.3 ms off the longest fills. The window check took about 13 % off
the keylock's mean. A Master Tempo run in real play is still to come.
