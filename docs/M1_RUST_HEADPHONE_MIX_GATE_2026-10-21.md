# Controller headphone CUE/master PCM bus

Date: 2026-10-21. Branch: `ogranak/headphone-cue-master`.
Based on main `cb69dbd28a36462b3e9a609478218a5efce0e379`, which merged
[Beat/Pad FX execution, PR #14](https://github.com/dvucinozd/Pajoniiir-M1/pull/14).
Its selected head `2f608a5851876bc39a58d99e54c5088d28db23c0` and the merge
have identical trees. Source hashes and focused FX regressions were rechecked
before merging. The primary checkout was fast-forwarded with its 18 existing
KiCad edits preserved byte for byte.

## Monitor behavior

Previously the reference render owner supplied only deck A's PFL buffer to the
delivery API. `FixedRateRenderGraph` now produces a complete 128-frame stereo
`headphones` buffer alongside MAIN and the two diagnostic PFL taps.

Both deck CUE/PFL switches select their wet signal after trim/EQ/filter/FX and
before channel fader/crossfader/master gain. Selected signals sum with float
headroom, then downmix to mono CUE. MASTER is the existing limited stereo MAIN
signal. Existing semantic controls produce per-block monitor gains:

```text
m = clamp(HeadphoneMix / 16383, 0, 1)
cue = 0.5 * (selected_wet_PFL_sum.left + selected_wet_PFL_sum.right)
master = MasterCue ? limited_stereo_MAIN : silence
headphones.L/R = ((1 - m) * cue + m * master.L/R) * HeadphoneLevel / 16383
```

`HeadphoneMix=0` is CUE-only; `16383` is MASTER-only. MasterCue off removes only
the master contribution without renormalizing the CUE gain. Both PFL signals are
summed, not averaged by selected-deck count. No PFL selection means silent CUE.
Opposite-polarity stereo content cancels when downmixed to mono CUE, as in the
existing C monitor contract. Stereo MASTER preserves that content.

HeadphoneLevel scales the complete monitor mix. At zero it mutes headphones
without changing MAIN, source consumption or output confirmation. Defaults stay
full MASTER mix, full headphone level and MasterCue on, matching MixerState.
Nonfinite render monitor gains fail closed; finite gains are bounded to 0..1.

The cue sum is attenuated before its final PCM conversion. Saturated diagnostic
per-deck PFL taps are never used as inputs to the headphone bus. Final headphone
conversion saturates at signed 16-bit limits, independently of the MAIN limiter
and its overload counters. There is no new headphone dynamics limiter, split-cue
mode, gain ramp or headphone settings persistence in this package. Physical
control-transition clicks remain a listening/timing gate.

The midpoint fixture uses deck A `(2000,6000)` on MAIN and deck B `(10000,10000)`
on PFL with its fader closed. Mix=8192 yields headphones `(6000,8000)`; half
headphone level yields `(3000,4000)`. The first fixture matches the inspected C
`test_full_mix_headphone_mix_blends_cue_to_stereo_master` in DDJ-FFL4 at
`5f41df1bfec994e325ec1ccdd4b3ebf4ad88061e`. This is a source/fixture comparison;
that repository's C suite was not run in this package. Retained integration data
pins the inspected files' hashes.

## Integration and routing

The embedded reference owner publishes `MixerState::headphone_gains()` in the
same snapshot as deck/gain/FX controls. `PlaybackOutput::submit_render` now takes
the graph's composed headphone buffer directly; its former independent cue
argument has been removed. Callers cannot accidentally replace the product bus
with one deck's PFL tap. Low-level OutputDelivery remains a generic lane API.

The 10-second consumer-clock example uses this product bus with CUE-only settings
and equal-polarity input. Its 128-frame MAIN and 48-frame headphone consumers still
complete 480,000 frames each without underrun/late/full events. MAIN peak remains
2000 and headphone peak 8000. The reference target binds only its MAIN null lane;
it computes but does not advertise a physical headphone stream.

Routing policy is unchanged: MAIN prefers ready external DAC -> compatible
controller -> mute. Headphones always require a separately validated controller
headphone pair. A MAIN DAC switch preserves the headphone token/queued PCM;
controller detach revokes it and outstanding receipts, with no DAC fallback.
Reconnect requires a fresh validated route and cannot replay the old queue.

When MAIN is unavailable, controller headphones may remain the existing transport
clock authority. The new composed test renders canonical EOF from both decks,
delivers the monitor bus in 48/48/32-frame packets and confirms source progress
only after completion of the final packet. Software source/receipt fencing and
fault containment remain active. A muted monitor is still a consumed clocked
stream; audible signal level is not used to determine transport authority.

No new heap allocation or effect storage is required. The static/reference output
block adds 512 bytes for the headphone samples; DSP/control state stays bounded.
The FX package's 1,540,704-byte storage budget is unchanged.

## Local verification

**639 host tests** pass, including **11 new headphone PCM/delivery scenarios**:
default stereo MASTER; MAIN independence from monitor controls; individual/both
PFL selection before faders/crossfader/master; midpoint and level scaling;
float cue headroom/cancellation/final saturation; mono phase cancellation;
MAIN-limiter independence; invalid gain bounds; starvation/pause isolation;
wet Echo tail/reset; composed routing through MAIN DAC changes and controller
detach/reconnect; headphone-only canonical EOF completion. Several related
checks share a scenario, so this list is not an additive test count.

Workspace formatting and warnings-denied Clippy, explicit runtime-owner Clippy,
desktop compilation, **33 P4 check/link configurations**, **24 expected invalid
feature rejections**, nine runtime-owner cases, 11 composed failure cases and
**38 unchanged UI screenshot goldens** pass. Focused/owner counts overlap the
workspace total. P4 builds retain existing feature-dependent warnings and are not
a warnings-denied target gate.

Reproduce from `firmware-rust`:

```sh
cargo test --locked -p pajoniiir-audio-runtime --test headphone_mix
cargo test --locked -p pajoniiir-audio-runtime --test sink_delivery
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo clippy --locked -p pajoniiir-service --features runtime-owners --tests -- -D warnings
```

[Retained evidence](evidence/m1-headphone-mix-2026-10-21/README.md) pins source and
log hashes, PR #14 merge/tree proof and preservation checks. PR #14 host/P4 jobs
did not start because of account billing/spending restrictions. Local verification
does not represent hosted CI success.

## Remaining work

Physical I2S/UAC endpoints, packet/channel-map verification, controller disconnect
timing, click-free monitor changes, PSRAM/deadline stress and listening acceptance
remain unrun. Guition selectors still do not activate display/touch/storage/audio.
The previous loop-edit continuity limitation with outstanding physical mixed
buffers remains open. The standalone UI simulator opens no audio device.

Next software work is MP3/FLAC decoding behind the bounded source/transport
interface. Exact-board bring-up and physical consumer pacing follow when the
boards are available. APTA remains separate and unmerged at
`fd7ebd592b469b27c9f45337dced2f9b2912966f`; both lockfiles, historical hardware
and board defaults are unchanged.
