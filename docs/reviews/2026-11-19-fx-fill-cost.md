# JC1060: what Beat FX costs a USB audio fill

Date: 2026-11-19. Branch: `ogranak/jc1060-fx-fill-profile`, from `main` at `74556be`.

## Question

After [even fills](2026-11-19-even-fills.md), 3.75 % of fills were still over 3 ms. In
the s142 soak they came in bursts of 6 to 18 s, and every large burst began with the
soak DJ's Beat FX gesture (5 s on, then off). Outside those bursts 1 to 2 % of fills
were over 3 ms. The question was how much each effect adds, and whether the time goes to
computing or to memory.

## Change (profile builds only)

`--uac-profile` now sorts every fill into one of eight classes and reports, per class,
the fills, the mean fill and render graph time, the fills over 3 ms and the longest:
- `plain`;
- `fx_filter`, `fx_echo`, `fx_flanger`, `fx_delay`: Beat FX on, by effect;
- `fx_tail`: the 3 s after Beat FX went off, while an echo may still ring;
- `pad_fx`;
- `scratch_mt`: a scratch or Master Tempo on either deck.

A fill takes the first class that matches, from `scratch_mt` down. The effect comes
from the Beat FX word the deck screen reads (`ruzina_legacy_fx_word`), the rest from the
deck flags the profile already recorded. The product build has none of this.

## On the board

s143 (app `a0543bc6…`) is the `--usb-stick` image of `main` with `--uac-profile`. The
soak DJ played both decks for 16 minutes with no touch: 159 000 fills, 0/0 underruns,
`packet_errors=0`, `iso_errors=0`, no fault, 14 240 stick reads. It turned Beat FX on 14
times, through all four effects.

| Class | Fills | Mean fill | Mean graph | Graph over `plain` | Over 3 ms | Longest |
|---|---:|---:|---:|---:|---:|---:|
| `plain` | 129 723 | 2485 µs | 1938 µs | | 15.8 % | 4830 µs |
| `fx_filter` | 2517 | 2757 µs | 2201 µs | +263 µs (+14 %) | 29 % | 4006 µs |
| `fx_echo` | 3358 | 2947 µs | 2378 µs | +440 µs (+23 %) | 43 % | 4500 µs |
| `fx_flanger` | 3356 | 2888 µs | 2314 µs | +376 µs (+19 %) | 38 % | 4259 µs |
| `fx_delay` | 2517 | 2916 µs | 2349 µs | +411 µs (+21 %) | 40 % | 4211 µs |
| `fx_tail` | 6994 | 2551 µs | 2004 µs | +66 µs | 21 % | 4301 µs |
| `pad_fx` | 8762 | 2701 µs | 2143 µs | +205 µs | 26 % | 4084 µs |
| `scratch_mt` | 1773 | 3229 µs | 2633 µs | +695 µs | 49 % | 5681 µs |

The profile itself lengthens every fill, so the shares over 3 ms are higher than in the
product build (18.6 % against 3.75 % in s142). The differences between classes hold.

On the host (release, the same graph with 96-frame blocks), the same effects add 5 to
10 % to the graph. On the board they add 14 to 23 %. The flanger, whose buffer is
2.3 KB per deck, and the filter, which has none, cost relatively as much more as the
delay and echo, whose lines are 192 KB a channel in PSRAM. So the delay lines in PSRAM
are not the main cause.

## What it means

- Beat FX adds 0.26 to 0.44 ms to a fill, on a plain fill that already takes about
  2.5 ms under the profile. With an effect on, the fill sits at the 3 ms target.
- The larger lever is the plain graph: 1938 µs for 288 frames, about 6.7 µs a frame,
  while the host takes about 0.1 µs. The next sections find where that time goes.
- There were no underruns.

## Where the plain graph's time goes

Profile builds now also time the render graph at boot, before USB audio and the media
load start (`render_bench`): each case renders 300 fills of three 96-frame blocks on a
task of its own on core 0, at the USB client's priority. s144 and s145 (app
`80015239…`) showed:
- the graph alone, from a source that is a register, takes 1410 µs a fill;
- reading the source from a ring in PSRAM, with the reader's acquire loads, adds 74 µs;
  a ring in internal RAM adds nothing;
- the parts, for both decks: resampler 321 µs, EQ 215 µs, channel filter at its center
  139 µs, MAIN's limiter with the headphones' conversion 247 µs.

So the graph is computing, not waiting on memory: about 1760 cycles a frame at about
0.75 instructions a cycle. The disassembly showed why. The render calls the DSP crate's
small per-frame functions (EQ, filter, PCM conversions, limiter, resampler helpers, the
effects) across crates, and Rust does not inline a non-generic function across crates
without `#[inline]` (or LTO). Each call saves and restores registers, and nothing stays
in registers between them. The render also worked out the resampler's step (two
divisions a deck) and every gain once a frame, though they change only with the block.

## Change

- `#[inline]` on the per-frame DSP functions and on `limit_main_sample`.
- `render_block_frames` works out each deck's step once a block, and `render_with` its
  gains.
- `fill_placement` renames each section once. An `#[inline]` function is made in every
  object that uses it, so the same section name can come from several objects, and
  objcopy refuses a second rename of it.

The audio is bit-identical: a reference render through both decks, pitch, EQ, filter,
all four Beat FX, both Pad FX modes, the limiter driven into overload and PFL gives the
same digest before and after. On the host the release render is 8 % faster.

## On the board, after the change

s146 (app `53b9c9ac…`) is s145 with the change. IRAM grew by 23 KB, and the deepest fill
stack stayed at 3648 B (limit 4096).

| Case | s145 | s146 | Change |
|---|---:|---:|---:|
| graph | 1410 µs | 1175 µs | −17 % |
| graph with echo on both decks | 1760 µs | 1491 µs | −15 % |
| graph from a PSRAM ring with acquire loads | 1484 µs | 1237 µs | −17 % |
| graph with one deck | 1166 µs | 985 µs | −16 % |
| graph without effect storage | 1288 µs | 1085 µs | −16 % |
| resampler | 321 µs | 306 µs | −5 % |
| EQ | 215 µs | 209 µs | −3 % |
| filter at its center | 139 µs | 89 µs | −36 % |
| limiter and PCM | 247 µs | 102 µs | −59 % |

In the same 3-minute soak after boot (28 000 fills each, 0/0 underruns, no packet or
isochronous errors), under the profile:

| Class | Mean fill s145 | Mean fill s146 | Over 3 ms s145 | Over 3 ms s146 |
|---|---:|---:|---:|---:|
| `plain` | 2392 µs | 2120 µs | 5.4 % | 0.66 % |
| `fx_echo` | 2864 µs | 2585 µs | 35 % | 14 % |
| `fx_flanger` | 2775 µs | 2500 µs | 26 % | 9 % |
| `fx_tail` | 2397 µs | 2143 µs | 13 % | 4.8 % |
| `pad_fx` | 2633 µs | 2331 µs | 15 % | 2.2 % |
| `scratch_mt` | 2876 µs | 2716 µs | 24 % | 24 % |
| all | | | 8.1 % | 1.7 % |

After this change the resampler and the EQ were the largest parts, at about 0.5 ms a fill together.

## Staged render

In s146 the EQ and the resampler were still calls a frame: the render loop was too large
for the compiler to inline them, so their state went through memory every frame.
`render_with` now takes the block in chunks of 16 frames. In a chunk each deck runs one
stage at a time over its frames: source and resampler, trim and EQ, channel filter,
effects, then meter, PFL and fader. MAIN and the headphones follow from the two decks'
sums. `EqState::process_frame` and `ResamplerState::next` are `#[inline(always)]`, so
each stage is a loop of its own with its state in registers.

Each deck still sees its frames in order, and each frame the same arithmetic, so the
audio is the same. The temporary digest test gives each deck a source of its own (the
order of pops between the decks changed) and pins both the graph with effect storage
and the one without: the digests from `main`'s render and from the staged render are the
same.

The chunk's buffers are on the USB client's stack, so the chunk is short: with 32 frames
the fill chain needed 4368 B of its 4096 B budget. With 16 it needs 4016 B, and the render
is now the deepest path.

s147 (app `50903496…`) is s146 with the staged render:

| Case | s145 | s146 | s147 |
|---|---:|---:|---:|
| graph | 1410 µs | 1175 µs | 1008 µs |
| graph with echo on both decks | 1760 µs | 1491 µs | 1322 µs |
| graph from a PSRAM ring with acquire loads | 1484 µs | 1237 µs | 1097 µs |
| graph with one deck | 1166 µs | 985 µs | 873 µs |
| graph without effect storage | 1288 µs | 1085 µs | 921 µs |
| resampler alone | 321 µs | 306 µs | 234 µs |

The same 3-minute soak (28 000 fills, 0/0 underruns, no packet or isochronous errors),
under the profile:

| Class | Mean fill s147 | Over 3 ms s145 | Over 3 ms s146 | Over 3 ms s147 |
|---|---:|---:|---:|---:|
| `plain` | 1954 µs | 5.4 % | 0.66 % | 0.17 % |
| `fx_echo` | 2408 µs | 35 % | 14 % | 6.3 % |
| `fx_flanger` | 2309 µs | 26 % | 9 % | 3.1 % |
| `fx_tail` | 1986 µs | 13 % | 4.8 % | 2.3 % |
| `pad_fx` | 2156 µs | 15 % | 2.2 % | 0.53 % |
| `scratch_mt` | 2398 µs | 24 % | 24 % | 8.7 % |
| all | | 8.1 % | 1.7 % | 0.61 % |
