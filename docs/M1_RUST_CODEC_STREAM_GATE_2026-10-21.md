# Shared WAV, MP3 and FLAC source decoding

Date: 2026-10-21. Branch: `ogranak/audio-codec-streams`.
Based on main `ba5e76c029b82af9c07ca4909b9a384a37d152b7`, which merged
[the headphone CUE/master package, PR #15](https://github.com/dvucinozd/Pajoniiir-M1/pull/15).
The reviewed head `f851981700d6c679b71bed6ea59f2e058a10e6ff` and merged main
have identical trees. Source hashes and focused regressions were checked before
merge; CodeRabbit completed its review with zero issues. The primary checkout
was fast-forwarded while preserving its 18 existing KiCad edits byte for byte.

## Common source interface

`pajoniiir_audio_runtime::decode` owns the codec-neutral `SourceDescriptor`,
`StreamChange`, `SourceDecoder`, caller-owned `DecoderStorage` and `DecodeAction`.
Each step either requests a bounded read at an explicit file offset, returns one
fixed `PcmStreamBlock`, or completes a bounded codec/preroll step so its owner can
yield or cancel. The interface has no filesystem, output clock, allocator or
whole-track storage. Existing `WavStreamChange` remains a compatibility alias.

The WAV branch retains the tested PCM16 decoder/stream implementation, including
split-byte carry, exclusive loop ends and its fixed short-loop cache. MP3 and
FLAC use the same generation, monotonic PCM coordinates, media timeline, terminal
fault, canonical history, render/FX/PFL/headphone bus and output receipt path.

The embedded Library/source integration now stores an optional generic audio
descriptor instead of a WAV-only descriptor. Content probing selects RIFF/WAVE,
native `fLaC`, or MPEG/ID3 input; extensions alone do not establish the codec.
Preparation retains the first successfully decoded block and publishes the
descriptor only after that probe. Product commit activates the exact prepared
generation. Stale publication/activation, replacement and media lease policies
are retained. Stream sample rate/channel changes fail closed.

The historical `wav-audio-source-bringup` feature name now enables all three
formats. Its existing Library/USB/PSRAM dependencies, board guards and defaults
are unchanged. Guition selectors still do not activate storage or audio owners.
The standalone Slint simulator retains its demo transport and opens no audio
device; the shared compressed path is tested headlessly and linked for P4.

## Codec adapters and supported bounds

Pinned dependencies are [nanoaudio-core 0.1.1](https://docs.rs/nanoaudio-core/0.1.1/),
[nanoaudio-mp3 0.1.1](https://docs.rs/nanoaudio-mp3/0.1.1/),
[nanoaudio-flac 0.1.2](https://docs.rs/nanoaudio-flac/0.1.2/) and its
[nanomp3-core 0.2.0](https://docs.rs/nanomp3-core/0.2.0/) frame decoder. These use
no_std decoder APIs without heap growth. Exact versions/checksums are retained
in both updated Cargo lockfiles. No C decoder/FFI or host std decoder is linked
into the P4 source path. Host FLAC builds may use target-specific SIMD helpers;
the RISC-V target uses its scalar implementation.

- WAV retains PCM16 mono/stereo behavior and existing format validation.
- MP3 uses MPEG-1/2/2.5 Layer III decoding. CBR, VBR, mono/stereo and Xing/Info
  gapless encoder delay/padding are covered by fixtures. Layer I/II is disabled.
  Free-format MPEG and additional container variants are not qualified here.
- Native FLAC accepts mono/stereo within the configured 9,216-frame maximum
  block size and at most 192 kHz. The backend handles deeper integer PCM;
  fixtures verify 16-, 24- and 32-bit input. Wider/larger unsupported formats
  fail closed. Ogg FLAC is not included. Frame/header CRCs are checked by the
  backend; whole-stream MD5 verification is not added.
- Canonical output remains stereo PCM16. Mono duplicates at unity. Deeper FLAC
  rounds/saturates to PCM16, so this does not preserve a 24/32-bit output path.

MP3 leading ID3 tags are skipped by checked synchsafe lengths and file bounds,
without reading the entire tag. Embedded cover art containing a valid MP3 stream
therefore cannot become audio. Standard trailing ID3v1 is removed from the final
window. Arbitrary MPEG corruption cannot always be detected without protected
frame checksums: malformed headers/no-audio input and known-length truncation
are tested, but every possible payload bit flip is not guaranteed to fault.

## Seek, loops and lifecycle

Compressed output counts decoded audio frames, including gapless trimming where
metadata provides it. Source length remains optional when MP3 has no Xing/Info
count; no length is invented from byte rate. Seeking past an unknown stream's
actual end faults during preroll. Known-length seeks retain the existing clamp
to the exact end, which produces an empty EOF block.

MP3 stores a complete decoder/reservoir state, pending PCM and encoded input
at the loop origin, plus two recent exact checkpoints at roughly half-second
intervals. A steady wrap restores the origin checkpoint rather than replaying
the track prefix. Checkpoints stay inside the source instance and cannot cross
source replacement. All copies have fixed sizes and allocate nothing during
playback. The late-loop regression checks four or more wraps six seconds into
an eight-second track against linear PCM and bounds the additional file reads.
A separate warm-seek regression verifies exact samples and bounded read count.

An arbitrary **cold seek** may still need preroll from byte zero when none of the
three stored checkpoints precedes its target. Its latency is O(position), so a
full sparse seek index is subsequent optimization work. Preroll yields
cooperatively between bounded steps, without an artificial 1 ms sleep per frame.
No physical cold/warm seek latency or realtime loop deadline is qualified here.

FLAC retains parsed metadata and uses its sample-number-based seek/refinement
requests, checking every requested file offset against the open file length.
Both adapters retain an exclusive loop end and the existing monotonic virtual
timeline. Steady wraps preserve the stream generation. Regions of at most 256
frames warm a fixed PCM cache and then repeat without further file reads. Other
loops seek/restart the codec and discard read-ahead beyond the loop boundary.
No complete-loop or complete-track buffer is allocated.

Seek/loop edits revoke the old generation and queued PCM, prime the replacement
before reporting success, and keep the reported target position separate from
prefetch. File/decode failure uses the existing deck-local failure completion.
Media lease loss and a queued replacement/release cancel priming between bounded
steps; the pending command remains queued for its normal owner turn. Driver I/O
already in progress is still subject to the existing hardware cancellation gate.

The prior loop-edit limitation remains: combined queued output and outstanding
receipts are revoked on generation changes. Smooth physical mixed-buffer handoff
is still unqualified. Steady loop wraps avoid that generation change.

## Memory and target integration

Each source owner allocates its typed decoder context and a **98,304-byte encoded
window** from the existing PSRAM heap before handling source commands. Allocation
is fallible; a failed owner keeps answering Prepare/Control with failure rather
than acknowledging dry playback or hanging a Library request. The context retains
MP3/FLAC state and a fixed 2,304-sample staging area, reusing it across track
changes. Three separately provisioned MP3 checkpoint buffers each retain a
decoder, 2,304 samples and at most 8,192 encoded bytes. They also allocate only
at owner startup. The host layout test bounds the base context below 110 KiB,
each checkpoint below 40 KiB and SourceDecoder below 4 KiB.

The compiled P4 layout requests 102,816 bytes for each base context and 36,560
bytes for each of its three checkpoints. With the encoded window, both decks
request 621,600 source-buffer bytes in total, excluding allocator metadata and
the existing queues/history. The separate FX allocation is unchanged.

Large contexts were initially tested in BSS; the full P4 link rejected that RAM
layout. The final implementation uses a fallible uninitialized typed allocation
and safe `Box::write` cloning from immutable flash templates into PSRAM, without
adding unsafe source code. Large initialization is isolated from the async poll
frame; the assembly audit records the remaining bounded MP3 clone scratch
rather than claiming zero stack use. Selected compiler frames are 39,680 bytes
for the source owner poll, 23,840/23,744 for context/checkpoint provisioning,
23,808 for checkpoint capture/restore and 3,376 for the shared decoder step.
The linked stack reservation is 75,440 bytes. These are selected function
prologues, not a complete call-graph bound or physical stack high-water result.
The final P4 check/link matrix passes with the heap-backed layout. This establishes
link compatibility, not physical initialization, stack high-water or PSRAM timing.

Filesystem transfers remain at most 512 bytes. Existing 16-block per-deck queues,
canonical history and output receipt capacities are unchanged. After startup,
source decoding/seek/loop operations do not grow buffers or allocate per track.
The FX package's 1,540,704-byte allocation is unchanged. Fixed capacities limit
memory, not the CPU time of a codec call or the latency of a long exact MP3 seek.

## Local verification

**659 host tests** pass, including **20 new codec scenarios**. The synthetic CC0
[fixture generator and references](../firmware-rust/crates/pajoniiir-audio-runtime/tests/fixtures/codecs/README.md)
cover independent FFmpeg PCM comparisons, mono duplication, mixed sample rates,
gapless CBR/VBR length, deep FLAC conversion, exact seek/loop samples, late-loop
checkpoint restoration and bounded recent seek reads, odd and
single-byte reads, long/short loops, resize/exit/EOF, stale PCM rejection, dirty
context reuse, malformed input/CRC/truncation, ID3 cover-art isolation and bounded
storage. MP3 references require matching frame counts, >70 dB SNR and no more
than 16 PCM16 units of sample difference; FLAC16 is exact, with at most one unit
of permitted rounding difference for deeper inputs.

Composed two-deck cases decode real MP3 and FLAC at differing sample rates into
canonical history, toggle Master Tempo/pitch, encounter bounded backpressure,
render wet PFL/headphones and publish progress only after output completion.
An injected first-deck fault clears its wet history while the second deck's
render/progress continues. These are software simulations, not USB timing tests.

Formatting, warnings-denied workspace and explicit runtime-owner Clippy, desktop
compilation, **33 P4 check/link configurations**, **24 expected invalid feature
rejections**, 11 composed failure cases, nine runtime-owner cases and **38 unchanged
UI screenshot goldens** pass. Focused/owner counts overlap the workspace total.
P4 builds retain their existing feature-dependent warnings and are not a
warnings-denied target gate.

The initial complete CodeRabbit review raised one major issue with MP3 prefix
replay at every loop wrap. Fixed checkpoint restoration, cooperative preroll and
the two read-count regressions address it. The final full source review completed
with zero issues; both review transcripts and their source scope are retained.

Reproduce from `firmware-rust`:

```sh
cargo test --locked -p pajoniiir-audio-runtime --test codec_streams
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo clippy --locked -p pajoniiir-service --features runtime-owners --tests -- -D warnings
```

[Retained evidence](evidence/m1-codec-streams-2026-10-21/README.md) pins source,
fixture and lock hashes, transcripts, PR #15 integration and preservation checks.
PR #15's hosted host/P4 jobs did not start because of billing/spending restrictions;
local validation is separate from hosted CI.

## Remaining physical gates

No flash, boot, physical filesystem throughput, I2S/UAC/channel-map output,
controller detach timing, PSRAM/stack measurements, sustained deadline soak or
listening acceptance was run. Exact Guition board activation remains pending.
Headphones remain controller-only; MAIN preference remains ready external DAC ->
compatible controller -> mute. Historical hardware and the separate APTA branch
at `fd7ebd592b469b27c9f45337dced2f9b2912966f` are preserved and APTA is unmerged.
