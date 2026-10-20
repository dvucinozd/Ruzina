# M1 Rust Bounded PCM Transport Gate

**Date:** 2026-10-06  
**Branch:** `codex/m1-rust-foundation`  
**Functional checkpoint:** `cb28def1d2551788f7762fb75b575c907c0ba2da`  
**Evidence state:** `HOST_VERIFIED / COMPILE_VERIFIED / HARDWARE_PENDING`

## Gate decision

The first bounded source-to-transport PCM gate is **CLOSED**.

This gate extends the previously closed WAV source-preparation boundary into a
continuous bounded producer/consumer contract:

```text
lease-bound WAV file
        |
Deck A/B source owner
        |
512-byte bounded filesystem reads
        |
WavPcm16Decoder
        |
PcmStreamBlock
        |
depth-3 per-deck PCM channel
        |
transport owner
        |
generation-safe position / EOF state
```

It does **not** claim audible real-time playback. There is still no PCM5102A
I2S sink, controller UAC sink or hardware audio clock in this path.

## Portable PCM stream contract

`pajoniiir-audio-runtime::pcm_stream` is `no_std`, allocation-free and
unsafe-free.

A `PcmStreamBlock` contains:

- a non-zero stream generation;
- absolute first source-frame index;
- sample rate;
- up to 256 canonical stereo `PcmFrame` values;
- explicit valid-frame count;
- EOF flag;
- fault flag.

The transport cursor keeps generation, absolute source-frame position, sample
rate, consumed-block count, stale-block count and ended state.

Frame/time conversion is integer and deterministic. Millisecond-to-frame seeks
are clamped to the source frame count; frame-to-millisecond conversion
saturates at `u32::MAX`.

## Producer ownership and backpressure

Each deck source owner keeps:

- one lease-bound filesystem handle;
- one `WavPcm16Decoder`;
- one 512-byte static filesystem read buffer;
- one monotonically changing stream generation;
- one absolute next-frame coordinate;
- one optional pending PCM block.

Each deck has an independent depth-3 PCM channel.

The 512-byte input bound is intentional:

- mono PCM16: at most 256 decoded frames;
- stereo PCM16: at most 128 decoded frames.

Therefore one filesystem read always fits one 256-frame PCM block without
requiring hidden unconsumed input storage.

If the PCM channel is full, the producer retains exactly one pending block and
does not advance the source again until that block is accepted. PCM is not
silently dropped under downstream backpressure.

## Prepare and media-generation behavior

A new track prepare:

1. drains queued PCM from the previous source;
2. closes the previous file when its media lease is still current;
3. advances the stream generation, skipping zero;
4. validates and opens the new WAV source;
5. resets the decoder at the WAV data start;
6. begins bounded prefetch.

If the USB media lease changes, the prepared source, pending block and queued
PCM are invalidated. A stale file handle is not sent into a newer filesystem
generation.

## Seek contract

Transport seek is a control-plane boundary, not a real-time audio callback.

For a seek:

1. the source PCM queue and local pending block are discarded;
2. milliseconds are converted to a clamped absolute source frame;
3. the source file is repositioned to
   `data_offset + frame * block_align`;
4. the decoder is recreated for the remaining data span;
5. the stream generation advances;
6. the source owner returns the new generation/frame/sample-rate tuple;
7. the transport cursor binds to that exact generation before consuming new
   PCM.

A block from an older generation is classified as stale and cannot advance
transport position.

The transport task may wait for this explicit seek transaction. Steady-state
PCM consumption itself performs no filesystem operation and never awaits USB
storage.

## EOF and source-fault behavior

The source owner emits explicit terminal state:

- normal data may carry `eof=true` on the last block;
- an empty terminal EOF block is emitted when the decoder reaches the end
  exactly between reads;
- an unexpected zero-length read before decoder completion emits a faulted
  terminal block.

On accepted EOF, the transport owner:

- commits the final deterministic deck position;
- clears its playing state;
- sends a stream completion to `ProductRuntime`;
- updates authoritative Deck state and LED state.

A source fault also stops the transport and is counted separately from a normal
EOF completion.

## Transport consumer boundary

Feature `pcm-transport-bringup` implies `wav-audio-source-bringup`.

The transport owner consumes PCM only while the corresponding deck is playing.
At playback start the cursor may adopt the first current source generation.
After a seek it accepts only the explicitly returned generation.

The current bring-up scheduler checks the bounded PCM queues from the transport
task on a millisecond timer and may drain available blocks. This is sufficient
to prove ownership, generation, backpressure, EOF and position semantics, but
it is **not** an audio-clock pacing implementation and must not be interpreted
as such.

Real consumption cadence belongs to the downstream audio sink/DMA boundary.

## Host verification

**Rust host foundation run 351: success**

The run passed:

- `cargo fmt --all --check`;
- Clippy with warnings denied;
- complete workspace unit tests;
- desktop simulator compile gate.

New host coverage includes:

- bounded frame/time conversion;
- generation adoption;
- stale-generation rejection;
- seek-generation binding;
- EOF transport completion;
- source-fault completion;
- PCM block capacity rejection;
- generation wrap with zero skipped;
- Deck stream completion stopping playback and committing final position.

## ESP32-P4 compile/link verification

**Rust ESP32-P4 compile gate run 317: success**

The matrix passed all previous gates plus:

- **ESP32-P4 bounded PCM transport cross compile**;
- **ESP32-P4 bounded PCM transport link gate**.

The same run also passed the final combined Library + USB0 + USB1 + SDMMC +
PSRAM ownership link gate.

This proves target compilation and linking only. It does not prove USB
throughput, scheduling latency, PSRAM/cache behavior, DAC timing or audible
output on physical M1 hardware.

## Deliberately open boundaries

This gate does **not** yet provide:

- an audio-clock-driven PCM consumer;
- PCM5102A I2S/DMA output;
- mixer/DSP output consumption from these blocks;
- sample-rate conversion into a fixed sink rate;
- Master Tempo/keylock integration on the target stream;
- scratch history fed from the live stream;
- loop-end source rewinds;
- controller UAC/PFL output;
- MP3 decoding;
- FLAC decoding;
- measured underrun/latency/throughput acceptance on EVT hardware.

## Next gate

The next software gate is:

**bounded PCM transport -> real audio render graph -> sink contract**

Recommended order:

1. define a fixed-rate render/sink contract and explicit block cadence;
2. feed Deck A/B PCM into the existing resampler/keylock/scratch boundaries;
3. apply channel DSP and mixer state without storage I/O in the render loop;
4. provide a host sink/reference harness with deterministic output fixtures;
5. add a target I2S/PCM5102A owner behind a separate feature;
6. compile/link on ESP32-P4;
7. keep latency, DMA/cache coherency, clock accuracy and audible quality
   `HARDWARE_PENDING` until EVT.
