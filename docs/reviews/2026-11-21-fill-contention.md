# JC1060: what slows the USB audio fill in play

Date: 2026-11-21. Branch: `ogranak/jc1060-fill-contention`, from `main` at `830a46b`.

## Question

The s156 profile found that a plain fill takes 1939 µs in play and 1012 µs on the boot
bench, and Master Tempo fills still reach 5.3–5.6 ms. The suspect was memory shared with
the screen and the stick.

The 2× gap was partly a wrong comparison. The 1939 µs is the whole fill, while the bench
times only the graph. Graph against graph, a plain fill took 1388 µs in play and 1092 µs on
the bench with its PSRAM ring, about 27 % slower. The rest of the fill (transport, copy,
pack) is not on the bench at all.

## What the profile image now measures

Both measurements are in profile images only (`--uac-profile`); the product image is
unchanged.

- **What else ran:** each fill records what else ran while it did. That is the screen
  drawing a frame (the Slint draw, the lanes and the flush), a decoder running on the other
  core (outside its waits), and the media owner reading the stick. Each activity counts its
  edges, so a fill knows whether the activity was running when it began, or started or
  stopped while it ran (`uac_profile_sched.h`).
- **Counts by overlap:** plain and Master Tempo fills are counted by that overlap
  (`uac_profile_overlap` lines). Each `uac_profile` record names the overlap of its fill.
- **A still screen:** the new build flag `--screen-still` makes the screen stand still
  every other 30 s. Fills in those phases are marked, so a fill can be compared with and
  without the screen, including any cache the screen leaves dirty.

`uac_driver_test.c` covers the overlap rules, the counts and the logger's copy of them.

## On the board

Image s163 (app `2ce5c0aa…`): `--uac-profile --screen-still --soak-autoplay --slint-ui
--fill-iram --usb-stick`. The run took 15 minutes: the soak DJ alone, then Master Tempo on
both decks. Underruns were 0/0 over 148 reports and nothing crashed. The bench matched
s156 (graph 1012 µs, keylock graph 1518 µs).

Master Tempo fills, both decks through their keylocks:

| While the fill ran | Fills | Mean | Over 3 ms | Longest |
|---|---:|---:|---:|---:|
| Nothing else, screen drawing between frames | 5 173 | 2170 µs | 1 | 3080 µs |
| Nothing else, screen still | 28 225 | 2165 µs | 0 | 2834 µs |
| A decoder, screen still | 24 715 | 2406 µs | 3.6 % | 3914 µs |
| A decoder | 5 966 | 2432 µs | 5.3 % | 3965 µs |
| A screen frame | 14 702 | 2444 µs | 8.4 % | 4740 µs |
| A screen frame and a decoder | 25 720 | 2815 µs | 31.0 % | 5350 µs |

- **The phases:** in the 30 s phases with the screen drawing, 18.3 % of Master Tempo fills
  took over 3 ms and the longest took 5350 µs. In the still phases, 1.7 % did and the
  longest took 3914 µs.
- **The screen:** fills that overlapped a screen frame account for 9302 of the 10 515
  Master Tempo fills over 3 ms (88 %). The rest overlapped only a decoder.
- **The stick:** its reads made no difference. Fills during a read were no slower than
  others.
- **The worst fill** (5350 µs, graph stage 4023 µs) overlapped a screen frame and a decoder.
- **Plain fills** show the same pattern at a smaller scale. With nothing else running they
  take 1459–1507 µs (graph 1049–1101 µs, near the bench). With a screen frame and a decoder
  they take 2178 µs.

## Conclusion

The fill is slowed by what runs on the other core at the same time, not by what that work
leaves behind. With nothing else running, it costs as much with the screen still as with
the screen between frames, and its graph is close to the bench. The long fills come when a
screen frame, and above all a screen frame with a decoder, runs during the fill. Their
memory traffic competes with the fill's.

The next step is to keep the screen's heavy memory work, and if needed the decoders, out
of the fill. The fill's start and end are known, so the other core can wait for its gaps.

s162 stays the image the board keeps. s163 is a profile image only.
