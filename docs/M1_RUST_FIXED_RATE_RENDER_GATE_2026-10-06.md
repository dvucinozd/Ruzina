# M1 Rust Fixed-Rate Render Graph Gate

**Date:** 2026-10-06  
**Branch:** `codex/m1-rust-foundation`  
**Functional checkpoint:** `bbf269656fb4f75f60f81a16465dd281c6d2ffa8`  
**Evidence state:** `HOST_VERIFIED / COMPILE_VERIFIED / HARDWARE_PENDING`

## Gate decision

The first fixed-rate software render-graph gate is **CLOSED**.

This gate moves steady-state PCM ownership out of the transport control plane and
into a dedicated audio render owner:

```text
Deck A/B source owner
        |
bounded PcmStreamBlock queues
        |
PcmBlockFeed
        |
FixedRateRenderGraph @ 48 kHz
        |
source-rate/tempo resampler
        |
pre gain / trim
        |
3-band EQ
        |
filter
        |
PFL tap --------------------> future controller UAC sink
        |
channel post gain / fader / crossfader
        |
dual-deck sum
        |
master gain
        |
MAIN limiter
        |
reference sink -------------> future PCM5102A I2S/DMA sink
```

The current sink is deterministic software/reference output only. No audible
hardware-output claim is made by this gate.

## Ownership correction

Before this gate, the `pcm-transport-bringup` path allowed the transport owner
to consume complete PCM blocks directly. That was useful to prove bounded
source progression, but it is not the permanent audio architecture.

With `render-graph-bringup` enabled:

- the render owner is the **only steady-state PCM data consumer**;
- the transport owner no longer receives audio samples;
- render publishes only compact `PcmStreamProgress` records containing stream
  generation, absolute source-frame position, sample rate, EOF and fault state;
- transport remains the authoritative owner of play/seek/final-position state.

The old direct PCM-consume path is retained only under the earlier
`pcm-transport-bringup` feature so the previous gate remains independently
compile-verifiable.

## Fixed-rate contract

The baseline render contract is:

- output sample rate: **48,000 Hz**;
- render block size: **128 stereo frames**;
- software block period hypothesis: **2,667 microseconds**;
- MAIN and per-deck PFL are explicit, separate output buses.

The software timer is a bring-up cadence only. It is not a hardware audio clock
and is not evidence for I2S/DMA timing accuracy.

## Render control snapshot

The USB1/product owner publishes a compact immutable render-control snapshot
from authoritative product state.

Per deck it carries:

- playing state;
- actual WAV source sample rate;
- pitch in centipercent;
- Master Tempo state;
- scratch/censor arbitration request;
- mixer pre gain;
- channel post gain including fader/crossfader;
- low/mid/high EQ controls;
- effective filter control;
- PFL enable.

Global render state carries the master-stage gain.

No render task mutates product Deck or Mixer state directly.

## Source generation and stale-data protection

Each deck source owner now publishes its active stream generation through an
atomic read-only observation boundary.

Generation changes on:

- successful track prepare;
- successful seek.

Generation becomes zero on:

- explicit source release;
- media-lease invalidation.

The render owner observes this generation before each render block. A change:

1. resets the per-deck `PcmBlockFeed`;
2. binds it to the new generation, or to no source when zero;
3. resets the deck resampler/EQ/filter transient state.

`PcmBlockFeed` additionally validates:

- non-zero stream generation;
- expected generation;
- contiguous absolute source-frame coordinates;
- stable sample rate inside one stream.

Older-generation blocks are dropped and counted. A coordinate discontinuity or
sample-rate discontinuity fails closed into terminal fault progress.

## Render-to-transport progress

Audio data never moves back into the transport task.

After rendering source frames, the render owner emits
`PcmStreamProgress`:

- generation;
- rendered source end-frame;
- source sample rate;
- EOF;
- fault.

The bounded render-progress channel has depth 8.

If transport progress is backpressured, the render owner retains pending
progress and **does not advance to another render block** until that progress is
accepted. Transport position therefore cannot silently fall behind rendered
source progression.

Seek remains a control-plane operation. Transport requests the source seek,
receives the exact new generation/frame/sample-rate tuple and binds its cursor
to that generation before accepting render progress.

## Current DSP path

The baseline reuses already host-tested product DSP primitives:

- `ResamplerState`;
- `EqState`;
- `FilterState`;
- `MixerState::stage_gains()`;
- MAIN `limit_main_sample`.

For normal playback the render order is:

1. source-rate + tempo resampling to 48 kHz;
2. pre gain / trim;
3. 3-band EQ;
4. channel filter;
5. PFL tap;
6. channel post gain including fader/crossfader;
7. dual-deck sum;
8. master gain;
9. MAIN-only limiter.

This explicitly preserves the architectural requirement that PFL is tapped
before channel fader/crossfader and that MAIN limiting does not alter the PFL
bus.

## Fail-closed deferred modes

Master Tempo/keylock and scratch/censor source arbitration are **not silently
approximated** in this baseline.

When either mode requires a not-yet-wired arbitration stage, the deck renders
silence for that mode and increments `deferred_mode_frames`.

This is intentional. It avoids claiming M2.2 parity from the existence of
standalone keylock/scratch crates before those crates are connected to the live
canonical PCM history.

## Underflow behavior

Source starvation is explicit:

- missing source input for an active deck produces silence for that output
  frame;
- previous audio is not repeated;
- per-deck underflow counters increase.

The deadline-critical render path performs no filesystem operation and no
network/profile/OTA work.

## Reference sink

`ReferenceSink` provides deterministic host-visible output evidence without
hardware:

- total rendered frame count;
- peak absolute sample;
- stable 64-bit digest over interleaved MAIN samples.

Equivalent render input produces an identical digest. This gives later DSP,
codec and routing gates a compact regression fixture before physical audio
output is trusted.

## Host verification

**Rust host foundation run 363: success**

The run passed:

- `cargo fmt --all --check`;
- Clippy for the full workspace/all targets with `-D warnings`;
- complete workspace unit tests;
- desktop simulator compile gate.

New host coverage verifies:

- deterministic idle silence;
- normal single-deck MAIN rendering;
- source starvation -> silence + underflow count;
- PFL tap before channel post gain;
- MAIN-only limiter behavior;
- fail-closed Master Tempo/scratch handling;
- explicit source-rate/tempo-factor conversion;
- deterministic reference-sink digest;
- render-side contiguous PCM feed;
- stale-generation rejection;
- discontinuity failure;
- old render-progress generation rejection by transport.

## ESP32-P4 compile/link verification

**Rust ESP32-P4 compile gate run 329: success**

The matrix passed all previous gates plus:

- **ESP32-P4 fixed-rate render graph cross compile**;
- **ESP32-P4 fixed-rate render graph release link gate**.

The same run also passed:

- default check/link;
- Hot Cue persistence;
- USB0 check/link;
- USB1;
- Library check/link;
- WAV source check/link;
- bounded PCM transport check/link;
- microSD;
- PSRAM;
- USB0 + PSRAM;
- dual USB + PSRAM;
- final Library + USB0 + USB1 + SDMMC + PSRAM ownership link gate.

This is compile/link evidence only. It does not establish real-time physical
audio timing or audible-output quality.

## Hardware boundary

The following remain `HARDWARE_PENDING`:

- PCM5102A I2S clocks and DMA;
- GPIO/BSP audio mute and safe boot behavior;
- actual block deadline/jitter/underrun behavior;
- PSRAM/cache/DMA interaction under sustained audio load;
- source media throughput under simultaneous USB/display/controller load;
- analog MAIN level/noise/distortion;
- controller UAC/PFL synchronization;
- mixed-rate dual-deck physical playback.

## Deliberately open software boundaries

This gate does **not** yet complete:

- canonical random-access PCM history;
- Master Tempo/keylock on the live render path;
- jog/vinyl scratch history and source arbitration;
- censor/reverse live source arbitration;
- loop-end source rewind in render ownership;
- Pad FX live processing;
- Beat FX live processing;
- controller UAC/PFL sink;
- PCM5102A I2S/DMA sink;
- MP3/FLAC decoding.

## Next gate

The next software gate is:

**canonical PCM history -> source arbitration -> live Master Tempo / scratch**

Recommended acceptance sequence:

1. add fixed-capacity per-deck canonical PCM history with absolute-frame tags;
2. keep normal source progression independent from scratch read position;
3. provide bounded random-access callbacks required by `KeylockState` and
   `ScratchState`;
4. prove scratch has explicit priority while preserving canonical timeline;
5. wire Master Tempo to the existing keylock implementation;
6. add deterministic host fixtures for transition continuity, stale history,
   seek/reset and underflow;
7. compile/link the complete path on ESP32-P4;
8. only then continue with live Pad FX / Beat FX and the physical I2S sink.
