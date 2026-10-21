# Streaming PCM loops and confirmed media position

Date: 2026-10-21. Branch: `ogranak/pcm-loop-execution`.
Based on main `d81a300784f81da4e3c7288135c6504e8d6f8623`, which merged
[the composed failure package, PR #12](https://github.com/dvucinozd/Pajoniiir-M1/pull/12).

## Implemented path

The WAV producer now executes loops rather than only recording SetLoop/ClearLoop
metadata. The same no_std WavStreamDecoder runs behind the embedded filesystem
owner and in the host PCM tests. It decodes only up to the exclusive loop end,
discards any read-ahead bytes beyond that boundary, then seeks the existing file
to the loop start before producing more PCM. A steady wrap preserves the stream
generation, decoder output sequence and render history.

Decoded PCM uses monotonically increasing virtual frame coordinates. PcmTimeline
maps those coordinates back to the original media position, including a linear
prefix before the loop. Canonical history, pitch, Master Tempo, scratch and Censor
therefore see continuous PCM, while confirmed transport position wraps within the
loop. A loop endpoint is not playback EOF. Real EOF returns after exiting the loop.

Loops longer than the 16,384-frame history are streamed; no whole-loop allocation
is made. Loops of at most 256 frames use a fixed 256-frame cache with per-frame
validity. After warming that cache, full PCM blocks can be generated without a
filesystem seek per sample. This also keeps a one-frame loop useful within the
existing 16-block queue and bounded render-prefetch budget. Each WavStreamDecoder
is under 2 KiB in the tested host layout. The existing 512-byte reads, per-deck
PCM queue depths and canonical history capacities are unchanged.

The lower WAV decoder also retains a sub-frame input tail when its output block
is exactly full. Otherwise an initial partial mono read followed by 512 bytes
could lose half of the next sample or fault the stream. The regression test
checks this boundary as well as ordinary split-byte reads through loop ends.
The new mono regression was also run with the original `d81a300` WAV decoder
temporarily substituted: it reproduced the stream fault. Restoring the exact
corrected file made that same focused test pass; the final source was preserved.

## Controls and lifecycle

- SetLoop, ClearLoop and Seek use the serialized source-control owner in both
  `pcm-transport-bringup` and `render-graph-bringup`. Enabling, resizing or clearing
  a loop rebinds at the latest confirmed media frame; prefetched file position is
  never used as audible position. These edits create a new stream generation and
  reject old PCM, progress and outstanding output receipts.
- Steady loop wraps do not rebind or reset either deck. Ordinary nonterminal
  progress now reaches ProductRuntime, so subsequent Loop IN/OUT and cue controls
  use confirmed media position instead of a position updated only at EOF.
- Seeking while looped retains the loop. Positions before its start play a
  linear prefix; positions at or after its end normalize into the loop. Clearing
  the loop resumes linear playback from the confirmed position.
- Millisecond control boundaries convert to source frames by integer flooring.
  Empty/reversed, sub-frame, zero-rate or out-of-track loops are rejected.
  A failed reconfiguration follows the existing deck-local source-failure policy:
  revoke/release the source and unload the affected deck, rather than silently
  playing an invalid loop or resetting the chip.
- A failed steady rewind/read produces a generation-bound terminal fault.
  Source replacement or media removal invalidates its history/cache and old
  completions. The other deck retains its independent source and product state.
- The transport-only integration drains a bounded batch of up to 16 PCM blocks
  per deck rather than stopping after every nonterminal position publication.

The standalone Slint simulator still uses its host transport/demo media path;
this package does not add host audio-device playback or Guition peripherals.

## Local verification

**614 host tests** pass, including **12 new PCM loop scenarios** and one new
composed firmware-product loop-control scenario. The composed failure suite now
has 11 cases; the nine owner-health cases remain passing. Counts are included in
the workspace total, not added again.

The loop scenarios check mono/stereo sample order, exclusive ends inside blocks,
one-frame and cached loops, split-byte/carry boundaries, loops longer than history,
resize/seek/exit/EOF, two-deck rendered receipt confirmation, pitch and Master Tempo
at differing sample rates, Censor/scratch release, stale PCM/receipt rejection,
failed rewind/read and invalid timeline/range handling. Product composition checks
that wrapped confirmed position drives real Loop IN/OUT, source reconfiguration
and ClearLoop while the second deck remains playing.

Workspace formatting, warnings-denied host and explicit runtime-owner test Clippy,
desktop compilation, **33 P4 check/link configurations**, **24 expected feature
rejections**, both UI replay geometries and **38 unchanged pixel goldens** pass.
Both lockfiles, historical hardware, board defaults and APTA are unchanged.

Reproduce from `firmware-rust`:

```sh
cargo test --locked -p pajoniiir-audio-runtime --test loop_playback
cargo test --locked -p pajoniiir-service --features runtime-owners --test composed_failures
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

[Retained evidence](evidence/m1-pcm-loop-2026-10-21/README.md) pins the baseline,
changed source hashes, transcripts and preservation checks. The PR #12 host/P4
jobs were again confirmed not started because of account billing/spending limits;
local results do not represent a hosted CI pass.

## Qualification and next work

All sample-accuracy results above concern the software PCM stream and the
reference output consumer. Real filesystem rewind latency, USB throughput,
executor jitter, DMA/UAC pacing, power/removal cancellation and listening remain
hardware gates. Injected outcomes do not emulate ESP driver timing.

Loop edits reuse the existing seek/generation handoff, which revokes combined
queued output and old receipts. The reference target completes each render block
immediately; smooth edits and continuous other-deck audio with a future physical
driver's outstanding mixed buffers are not qualified by these tests. Steady wraps
avoid that handoff entirely. Musical boundary clicks, clock drift and sustained
two-deck/pitched/FX workloads still require physical listening and timing evidence.

Guition selectors remain inactive for display/touch, storage and audio. MAIN
preference remains ready external DAC -> compatible controller -> mute, and
headphones/CUE/PFL remain controller-only. Audible Beat/Pad FX execution, MP3/FLAC,
physical I2S/UAC and complete operator error presentation remain subsequent work.
APTA remains unmerged.
