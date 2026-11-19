# JC1060: three even render blocks per USB audio fill

Date: 2026-11-19. Branch: `ogranak/jc1060-even-fills`, from `main` at `cd2c895`.

## Problem

In the 42-minute soak from the stick ([review](2026-11-19-stick-soak.md)), 11.2 % of USB
audio fills were over the 3 ms target, and the longest took 5950 µs. There was no
underrun. The share stayed at 9 to 16 % in every stretch of 10 000 fills and did not
follow the track loads.

## Cause

A profiled build of the same image (s141, `--uac-profile`, 11 minutes, 109 000 fills,
0/0 underruns) recorded where each fill spends its time:
- The fill was not preempted. In the longest fills the USB client task was away for 0 to
  2 µs, and interrupts took at most 63 µs.
- Nearly all of the time went to the render graph. In a 3048 µs fill the graph took
  2390 µs; in the longest, 4308 µs, it was most of the fill too.
- Every recorded fill over 3 ms rendered three blocks.

The render graph works in blocks of 128 frames. One 6 ms USB transfer takes 288 frames,
so fills rendered two blocks, two, two and then three. The three-block fills were a
quarter of all fills and held all the long ones. The longest fills, 6.3 to 6.6 ms, came
during the soak DJ's scratches and also rendered three blocks.

The stick and the task layout were not the cause.

## Change

- `ruzina-audio-runtime`: `render_block_frames` renders the first `frames` of a block (1
  to 128). `render_block` is unchanged and renders all 128.
- The legacy mix renders 96-frame blocks (`MIX_BLOCK_FRAMES`), exactly three per fill.
  The total work is the same, but no fill takes a third block's peak.
- Waits counted in render blocks keep their length in time: the beat sync's phase check
  and the seek cushion stay at 1 s, and the Master Tempo settle at 32 ms.

A new runtime test renders 384 frames as three 128-frame blocks and as four 96-frame
blocks, with both decks, a pitch, a cut EQ and an open filter. Every frame of MAIN and
the headphones matches.

## On the board

s142 (app `3125b34f…`) is the `--usb-stick` image with this change. The soak DJ played
both decks for 16 minutes with no touch, and loaded the same tracks in the same order as
the 42-minute soak on s140. Both runs are compared at 159 000 fills:

| | s140 (128-frame blocks) | s142 (96-frame blocks) |
|---|---:|---:|
| Fills over 3 ms | 17 941 (11.3 %) | 5 966 (3.75 %) |
| Longest fill | 5950 µs | 5235 µs |
| First fill | 1654 µs | 1340 µs |
| Underruns | 0/0 | 0/0 |
| `packet_errors` / `iso_errors` | 0 / 0 | 0 / 0 |

s142 also read 14 320 times from the stick with no recovery and had no fault. The owner
listened on the FLX4: "Radi".

## Still open

The fills that remain over 3 ms rise and fall with the tracks playing, from 4 to 18 % of
a stretch of 1000 fills. They play no scratch and follow no load. The likely cause is the
fill's PSRAM reads competing with the decoder and the deck screen on the other core. That
is a separate step.
