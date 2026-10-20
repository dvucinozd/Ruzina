# M1 Rust WAV Audio-Source Bring-Up Gate

**Date:** 2026-10-06  
**Branch:** `codex/m1-rust-foundation`  
**Evidence state:** `HOST_VERIFIED / COMPILE_VERIFIED / HARDWARE_PENDING`

## Gate decision

The first native Rust deck audio-source bring-up gate is **CLOSED**.

This gate does not claim complete audio playback. It establishes the ownership and
decode boundary required for the next transport gate:

```text
CatalogTrack
    |
Library/deck-load owner
    |
DeckA / DeckB filesystem owner
    |
RIFF/WAVE scan
    |
PCM16 decode probe
    |
retained lease-bound WAV source
```

The normal `library-deck-load` feature remains format-neutral. The stricter WAV
path is enabled only by `wav-audio-source-bringup`, so MP3/FLAC product behavior
is not silently reclassified as implemented or unsupported by this gate.

## Filesystem ownership closure

Before this gate, `LibraryDeckOwner` temporarily used the `DeckA` and `DeckB`
filesystem clients for audio-file validation and Rekordbox ANLZ Hot Cue reads.
That prevented those lanes from becoming persistent playback owners.

The broker now has four independent logical clients:

- `Library` — Rekordbox PDB import;
- `LibraryLoad` — deck-load preflight and ANLZ Hot Cue reads;
- `DeckA` — reserved for Deck A audio-source ownership;
- `DeckB` — reserved for Deck B audio-source ownership.

Each client retains its own depth-1 completion channel and request sequencing.
The shared filesystem request queue is increased from three to four entries.
A client cannot consume another client's completion.

## Native WAV core

`pajoniiir-audio-runtime::wav` is `no_std`, allocation-free and uses the
canonical `pajoniiir_audio_dsp::PcmFrame`.

The current decoder accepts RIFF/WAVE PCM16 with:

- one or two channels;
- non-zero sample rate;
- exact PCM16 block alignment;
- exact byte-rate consistency;
- little-endian signed 16-bit samples;
- mono duplicated into canonical stereo frames.

It rejects unsupported audio format tags, unsupported channel counts,
non-16-bit PCM, invalid sample rate, invalid block alignment, invalid byte rate
and data chunks that are not frame-aligned.

The decoder supports arbitrary split input reads with at most one partial frame
retained internally. Output backpressure is explicit: input beyond the caller's
PCM output capacity is not silently consumed.

RIFF chunk parsing is bounded and accounts for the mandatory pad byte after
odd-sized chunks.

## ESP32-P4 target ownership

Feature `wav-audio-source-bringup` implies `library-deck-load` and spawns two
persistent Embassy owner tasks:

- Deck A owns `FileSystemClientId::DeckA`;
- Deck B owns `FileSystemClientId::DeckB`.

Each owner has one static 4096-byte read buffer. No heap allocation is required
for WAV scanning or the PCM decode probe.

For a Library load, the selected deck owner:

1. verifies that the candidate `MediaLease` is still current;
2. opens the catalog audio path through its dedicated filesystem client;
3. validates the RIFF/WAVE header and bounded chunk geometry;
4. locates and validates the PCM16 `fmt ` chunk;
5. locates a frame-aligned `data` chunk;
6. decodes at least one real PCM frame through `WavPcm16Decoder`;
7. seeks the retained file handle back to the first byte of audio data;
8. returns a `WavPcm16SourceDescriptor` to the Library/deck-load transaction.

When this bring-up feature is enabled, the product track is not committed until
that preparation succeeds. A failed product commit requests release of the
prepared source. Media-generation changes invalidate retained source ownership;
a stale lease-bound handle is dropped rather than sent to a newer filesystem
generation.

The product-side loaded-track record retains the validated WAV source descriptor
(sample format, data offset and data length) together with the catalog candidate.

## Verification evidence

### Host

Functional WAV-core checkpoint `4851214b8aedb3da74042afd0187fd51333aec77`
passed **Rust host foundation run 339**:

- `cargo fmt --all --check`;
- Clippy with `-D warnings`;
- complete workspace unit tests;
- desktop simulator compile gate.

Host tests cover RIFF/chunk parsing, mono/stereo PCM16 format validation,
unsupported formats, split-read reconstruction, mono-to-stereo conversion,
output backpressure, public-format revalidation and misaligned data rejection.

### ESP32-P4

Target integration checkpoint `42aadc0ed77e2ce4c7f50557c9244f8673025678`
passed **Rust ESP32-P4 compile gate run 306**.

The run passed:

- default check/link;
- Hot Cue flash persistence;
- USB0 check/link;
- USB1 controller check;
- Library deck-load check/link;
- **WAV audio-source bring-up check/link**;
- microSD profile storage;
- PSRAM;
- USB0 + PSRAM;
- dual USB + PSRAM;
- final Library + USB0 + USB1 + SDMMC + PSRAM ownership link gate.

This is compile/link evidence only. It does not upgrade any physical USB,
filesystem, PSRAM or audio claim to `HARDWARE_VERIFIED`.

## Deliberately open boundaries

This gate does **not** yet provide:

- continuous PCM prefetch or a bounded PCM block queue;
- transport-driven WAV read/decode progression;
- seek/loop synchronization between Deck state and the retained source;
- resampling, Master Tempo or scratch integration on the retained file source;
- mixer/DSP consumption of source PCM;
- PCM5102A I2S output;
- controller UAC/PFL output;
- MP3 decoding;
- FLAC decoding;
- 24-bit, 32-bit integer, IEEE-float WAV or RF64;
- real USB-stick throughput or hot-plug qualification.

Those are not hidden limitations: they define the next development gates.

## Next gate

The next software gate is:

**retained WAV source -> bounded PCM block queue -> transport owner**

Acceptance should require:

1. fixed-capacity PCM blocks with explicit producer/consumer backpressure;
2. Deck A and Deck B source progression owned independently;
3. play/pause/seek generation checks so stale reads cannot advance a new track;
4. deterministic source-frame to deck-time conversion;
5. EOF and short-read behavior covered by host tests;
6. no filesystem await in the deadline-critical DSP/sink callback;
7. ESP32-P4 compile/link evidence before adding PCM5102A hardware output.

MP3 and FLAC should then enter behind the same source/PCM contract rather than
creating format-specific Deck behavior.
