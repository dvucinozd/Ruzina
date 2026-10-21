# Beat/Pad FX execution in the PCM render graph

Date: 2026-10-21. Branch: `ogranak/pcm-fx-render`.
Based on main `3e56319d912f389168534217e697c06a2e7a7c62`, which merged
[the streaming PCM loop package, PR #13](https://github.com/dvucinozd/Pajoniiir-M1/pull/13).
The reviewed head `ce3936a91093526aa3ad6d12b750cc90d4e79b7d` and that merge
have identical trees. The primary checkout was fast-forwarded while preserving
its 18 pre-existing KiCad edits byte for byte.

## Implemented path

`FixedRateRenderGraph::with_fx` connects the existing no_std PadFx, FilterState,
DelayFx and FlangerFx processors to real PCM samples for each deck:

```text
canonical source / pitch / Master Tempo / scratch / Censor
  -> trim -> EQ -> mixer filter / Smart CFX
  -> Pad FX -> Beat filter -> Echo/Delay -> Flanger
  -> wet PFL tap -> channel fader / crossfader
  -> sum -> master gain -> MAIN limiter
```

PFL remains before channel faders and master gain. MAIN routing preference stays
ready external DAC -> compatible controller -> mute. Headphones/CUE/PFL remain
controller-only. This package adds neither a physical sink nor a complete
headphone cue/master mix: the reference target still submits deck A's PFL tap as
its cue buffer. Both per-deck wet PFL taps are tested separately.

The render owner reads one authoritative product snapshot per block. Beat target,
type, depth, BPM/beat timing, on/off state and held Pad FX use the existing semantic
state and effect mapping. Historical queued DSP commands are drained for diagnostics
in the render variant; only the current snapshot configures its audible state.
Non-render legacy variants retain their diagnostic-only DSP owner. The standalone
Slint simulator retains its demo/host transport path without audio-device playback.

`new()` remains the explicit compatibility graph without FX storage. Product FX
requires `with_fx`; the embedded reference render owner now uses it.

## Source ownership and tails

- Effect state is scoped by `(source_epoch, stream_generation)`. Replacement,
  seek or loop edit resets the effect rack, EQ/filter history and pop resampler.
  Unloaded, mismatched-generation and faulted canonical sources revoke the rack
  and cannot emit a previous track's echo. A steady PCM loop wrap preserves it.
- Pause, starvation and EOF supply silence to the rack so existing delay tails
  can finish. Ordinary Echo release rings for at most two seconds; one-shot Delay
  release rings for one delay period, following the existing DSP contracts.
- Beat target removal clears that deck's Beat FX history. Beat Clear advances
  a product reset epoch even when its visible controls were already at defaults;
  it clears Beat history without resetting a held Pad effect.
- Releasing a Pad snapshot uses the actual held pad identity, even though the
  product snapshot has already removed it. This preserves Echo pad release and
  its bounded tail. Unrelated Beat control changes cannot renew that tail.
  A new Pad activation clears previous Pad state. Controller disconnect clears
  held Pad state on both decks while preserving playback and global Beat controls.

## Memory and realtime boundary

The caller supplies two fixed effect storage slices. At 48 kHz each deck has two
48,001-frame stereo delay lines (Beat time and Pad echo) plus a 292-frame stereo
flanger line. Total storage is **385,176 f32 samples = 1,540,704 bytes**, excluding
small processor/control state and existing source/output buffers.

The legacy reference target reserves this memory once from its existing PSRAM
heap before rendering. A failed reservation records `fx_storage_failed` and stops
the reference consumer from advancing. It does not silently fall back to dry
playback. Operator presentation of this internal status remains future work.

Delay and Flanger reset now invalidate old samples in O(1). Every delayed read,
including each fractional flanger interpolation tap, checks that its history has
been overwritten since reset. No delay-line clearing or heap growth happens in
the render loop. Dirty/NaN initial storage and previous-source data are covered by
DSP regression tests. PSRAM bandwidth/cache behavior, executor scheduling and the
128-frame audio deadline still require physical measurement; successful linking
does not qualify any of them.

## Local verification

**628 host tests** pass: 14 more than the merged loop baseline, comprising 11 PCM
FX scenarios, two dirty-history DSP reset regressions and one Beat Clear epoch
scenario. Existing controller disconnect coverage now also checks held pads.
The 11 composed failure cases and nine runtime-owner cases remain passing and
are included in the workspace total, not added again.

The PCM tests cover disabled-rack equality with the previous EQ/filter/mixer path,
exact one-shot timing (including the pop resampler's existing startup frame),
stereo wet PFL before faders/master, finite paused Echo tails, immediate Beat Clear,
Pad identity release, independent other-deck rendering under starvation, source
epoch/generation/unload/fault reset, real Beat filter/flanger and both Pad filter
banks, a delay crossing a cached PCM loop wrap, explicit memory limits and semantic
product snapshots through controller loss.

Workspace formatting, warnings-denied host and explicit runtime-owner Clippy,
desktop compilation, **33 P4 check/link configurations**, **24 expected invalid
feature rejections**, both UI replay geometries and **38 unchanged pixel goldens**
pass. P4 builds retain existing warnings in some feature variants; they are not a
warnings-denied target gate. No lockfile, hardware or board-default changes occur.

Reproduce from `firmware-rust`:

```sh
cargo test --locked -p pajoniiir-audio-runtime --test fx_render
cargo test --locked -p pajoniiir-audio-dsp -p pajoniiir-deck
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo clippy --locked -p pajoniiir-service --features runtime-owners --tests -- -D warnings
```

[Retained evidence](evidence/m1-pcm-fx-2026-10-21/README.md) pins changed source
hashes, transcripts, PR #13 integration and preservation checks. PR #13's hosted
host/P4 jobs were confirmed not started because of account billing/spending
restrictions. These local results do not represent a hosted CI pass.

## Remaining gates and next work

This is software PCM execution with the reference/null consumer. No flash, boot,
physical I2S/UAC output, headphone mix acceptance, listening, pop/click evaluation,
PSRAM stress or two-deck realtime timing claim is made. Guition display/touch,
storage and audio selectors remain inactive pending exact-board bring-up.

The previous loop-edit limitation remains: edits revoke combined queued output
and old receipts. Smooth edits while a physical driver owns outstanding mixed
buffers still need continuity work and physical evidence.

Next software packages are the complete controller headphone cue/master bus and
MP3/FLAC decoding behind the bounded source interface, followed by physical sink
pacing and board-specific peripheral activation when hardware is available.
The independent APTA branch remains unmerged at
`fd7ebd592b469b27c9f45337dced2f9b2912966f`.
