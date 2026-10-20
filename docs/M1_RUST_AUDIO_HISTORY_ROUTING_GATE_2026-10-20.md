# M1 live PCM history, output routing and first-boot preparation

Date: 2026-10-20

Base: `7bad2ea106c6300679f3662731cb9bbc5bb71add`. This implements the first software steps approved after the [development review](M1_RUST_DEVELOPMENT_OPEN_QUESTIONS_2026-10-20.md). Evidence is local host and P4 compile/link only. Boards remain in transit; no image has been flashed or heard.

## Live canonical PCM path

[PcmHistory](../firmware-rust/crates/pajoniiir-audio-runtime/src/history.rs) is a fixed-capacity ring with absolute frame coordinates and an explicit stream generation. Reads reject old generations and evicted coordinates. Appends reject discontinuities and checked-coordinate overflow.

[CanonicalSource](../firmware-rust/crates/pajoniiir-audio-runtime/src/source.rs) separates accepted/prefetched PCM from consumption and audible playhead publication. Each deck has 16,384 stereo PCM16 frames (65,536 bytes) of history, including at most 2,048 frames of future lookahead. Buffers are statically allocated in the embedded target, not on the task stack or through PSRAM. Each source queue is bounded to 16 blocks so a worker turn can fill that lookahead even when a 512-byte stereo read produces only 128 valid frames. This is a capacity argument, not measured USB throughput.

The source modes are:

1. scratch, with priority over censor and Master Tempo;
2. censor/reverse;
3. Master Tempo using the existing Rust keylock engine;
4. normal rate/tempo interpolation.

Normal canonical progress is independent of the scratch read head. Vinyl release hands playback back to the retained scratch head; paused scratch does not advance the background cursor. Censor uses a separate reverse cursor and resumes the canonical timeline. A scratch head whose history is evicted mutes until release instead of reading a reused ring slot. Output silence at a stationary platter is intentional. Audible handoff/crossfade quality and full released-controller feel still need later integration and listening acceptance.

The keylock adapter accepts tempo factors 0.5..2.0 and source/output-rate ratios 0.25..4.0; unsupported combinations fail closed and increment the deferred-mode report. Normal source stepping is bounded to 0.01..16.0. All source reads and prefetch loops are bounded and perform no filesystem I/O inside render.

The existing pop-only render API remains available and still rejects modes requiring random access. The new `render_canonical_block` path uses the live history while preserving trim/EQ/filter, pre-fader PFL taps and the MAIN-only limiter.

## Product and target ownership

The historical `board-legacy-m1,render-graph-bringup` target now uses the canonical path. Guition board selectors still do not activate product peripheral owners.

- Source preparation returns an exact generation token. The source becomes active only after successful Library/product commit. Prepare, release, detach and in-flight seek invalidate prior activation.
- A render-control snapshot must match the current active source generation. Prepared data, including reloading the same track, cannot become audible under an old product snapshot.
- Scratch jogs use the existing bounded, retryable transport lane. Source generations and gesture epochs reject stale commands across track replacement, touch release and controller reconnect. Loop-adjust jogs retain priority over scratch commands.
- Prefetch never advances transport time or publishes early EOF. Paused scratching publishes its audible head; EOF waits for the selected playback path to finish.
- Congested progress publication coalesces into one bounded pending item per deck and does not suspend the render owner. Latest audible position can move backward on a vinyl handoff. Faults remain sticky; stale generations cannot update current transport state.
- A newly activated source can publish paused-scratch progress without a Play command. The transport adopts only the active generation and rejects queued progress from the replaced source before rebinding.

The render owner still writes a reference sink and uses a software timer. This is not a DMA/audio-clock implementation, and it does not establish real-time scheduling, throughput or audible output.

## Modular output selection

[OutputRouter](../firmware-rust/crates/pajoniiir-audio-runtime/src/routing.rs) is a HAL-independent control-plane state machine:

| Availability | MAIN | Headphones/CUE/PFL |
|---|---|---|
| Ready external DAC and compatible controller | External DAC | Controller headphone channels |
| No ready DAC; compatible controller MAIN and headphone streams | Controller MAIN channels | Controller headphone channels |
| Ready external DAC without controller headphone capability | External DAC | Unavailable |
| Neither MAIN output ready | Muted | Controller headphones only if independently ready |

The headphone route type has no external-DAC or onboard-codec alternative. Ready streams require valid connection/stream epochs, a supported PCM format, 48 kHz delivery and bounded stereo channel maps. Controller MAIN cannot overlap its headphone channels or mix inconsistent controller epochs. Different controller interfaces can have independent stereo maps.

Availability changes invalidate changed bindings immediately. A MAIN-only change preserves an unchanged headphone route. A pending transition has a revision; a delayed preparation acknowledgement cannot reactivate a disconnected or superseded stream. Physical drivers must perform old-route fade/mute and drain, new-stream preparation/priming, a fresh readiness update and acknowledgement before activation.

This is a tested routing contract, not physical automatic switching. DAC I2C detection/initialization, actual I2S and UAC sinks, controller channel/feedback/drift handling, cue composition, output fades and device hot-plug qualification remain open. A device ACK alone is not analog-output acceptance. No external DAC module or GPIO map is selected by this gate.

## First-boot preparation

An opt-in `board-diagnostics` feature is available with either Guition selector. It starts a bounded native USB Serial/JTAG report task alongside the scheduler. It does not enable flash access, PSRAM allocation, host USB, SD, panel/touch, DAC or amplifier output. Default features remain empty; ordinary Guition selectors remain scheduler-only.

The report includes board profile, build-provided image ID, detected eFuse chip revision, minimum policy and reset reason. Documented flash/PSRAM capacities are printed separately from observations. Until actual probes exist, observations explicitly say `NOT_MEASURED`, and readiness says `NOT_QUALIFIED`.

The transport uses nonblocking byte writes and nonblocking flush, with at most 64 attempts per scheduler turn. It cannot wait forever in a blocking serial flush when no host is attached. Whether either exact board exposes this native service peripheral is still a physical wiring question. JC8012's CH340 UART service connector is not this transport.

The [first-boot procedure](boards/GUITION_FIRST_BOOT_2026-10-20.md) records capture, backup, recovery and exact-image requirements. No Guition partition layout or flash command is approved by the compile gate.

## Validation

Local validation uses Rust 1.95.0 and committed locks:

| Check | Result |
|---|---|
| Host format, workspace/all-target Clippy with warnings denied | PASS |
| Full host workspace tests | PASS, 509 tests |
| Desktop simulator compile | PASS |
| P4 partition validator and workflow matrix | PASS, 29 checks/builds |
| Invalid board/legacy/diagnostic feature combinations | PASS, 20 expected compiler rejections |

The host fixtures cover retained/evicted/generation-tagged history, prefetch/EOF separation, starvation/recovery, dual-deck independence, stale product/jog tokens, paused scratch, vinyl handoff, censor and controller-only output routing. Live keylock runs across ring wrap and coordinate rebase with a starting coordinate of 10^12 frames at 24/44.1/48/96 kHz and tempo factors 0.5/1/1.5/2. A 440 Hz tone remains within the test's 20 Hz estimation tolerance with Master Tempo at 1.5x, while normal playback shifts toward 660 Hz. This is deterministic software evidence, not listening or audio-quality certification.

Logs and source identity are retained in [gate evidence](evidence/m1-audio-history-routing-2026-10-20/). Both lock changes are limited to local Rust graph additions/reordering; external package versions and checksums remain unchanged. Embedded warnings inherited from the bring-up target remain separate from warning-free host Clippy.

## Remaining work

Before the first audible milestone, establish per-unit identity, recovery, actual memory configuration, panel/touch, USB host mapping/power and the selected MAIN DAC wiring. Implement real I2S/UAC sinks and audio-clock-driven consumption with channel/clock/fade acceptance. Then close loop-end rewinds, live FX, MP3/FLAC and mixed-load playback. Analysis/UI, settings, OTA and C6/Web Remote remain separate development gates. APTA remains unmerged, and custom PCB files are unchanged.
