# M1 consumer-clock PCM delivery gate

Date: 2026-10-20

Base: `8675aeb522e66446be566e908fb5b34989729d23`, following the [history/routing gate](M1_RUST_AUDIO_HISTORY_ROUTING_GATE_2026-10-20.md). This implements the next host-testable slice of S2. No firmware was flashed and no physical I2S/UAC sink is enabled.

## Delivery and clock contract

[`SinkLane` and `OutputDelivery`](../firmware-rust/crates/pajoniiir-audio-runtime/src/sink.rs) provide independent MAIN and controller-only headphone queues. Each borrows caller-owned storage for 1..8 blocks of 128 stereo PCM16 frames. Four blocks retain 2 KiB of PCM per lane plus metadata. Delivery performs no allocation, blocking wait, filesystem access or driver call. Access must be serialized by its owner; it does not itself provide ISR/task synchronization or DMA-safe storage.

A consumer supplies its monotonically consumed-frame clock and requests 1..128 frames into caller-owned storage. These output-frame coordinates are independent of decoded source coordinates. Cadence does not derive from rounded microsecond sleeps. A producer checks `next_block_frame` before rendering; a full authority queue returns no demand, avoiding source consumption solely to discard a block. Overlapping, late, stale or overflowing submissions fail. Future PCM cannot fill an earlier gap, and starvation never replays a cached sample.

Fanout reports each lane independently: a full MAIN queue does not reject a headphone submission, or vice versa. The product still needs an explicit clock authority and a policy for its failure. The simulation uses MAIN as authority and targets two queued blocks. This core does not implement asynchronous clock correction: the faster-headphone fixture remains bounded and reports late/underrun symptoms while MAIN stays intact.

## Route/reset and fade contract

Delivery synchronizes with committed `OutputRouter` routes. Independent route revisions detect an intervening detach/reconnect returning to the same binding, even if the transient unavailable state was not observed. MAIN changes preserve unchanged headphone tokens, queued PCM, clock and fades. Headphones have no external-DAC binding.

Route changes, detach, source replacement, seek and consumer restart require synchronizing/resetting affected queues before further callbacks. `reset_pcm` clears waiting PCM, revokes tokens and starts muted. Source changes affecting the combined mix require invalidating both lanes: a source-generation check before render cannot invalidate PCM already queued at an output. Reset may preserve the current output clock; its coordinate is not the source seek position.

Prime a new binding, set active/idle expectation, then request frame-counted linear fade-in. Planned switches can fade the old connected stream to mute and drain physical buffers before replacement. Detach invalidates immediately instead of fading through absent hardware. Fade duration is bounded to 48,000 frames. Audible transition quality is not established.

Stale, duplicate, out-of-order or invalid callbacks return zeros without advancing the clock. Discontinuous driver clocks require explicit reset/re-prime. Active missing PCM increments underrun frames/events; expected idle and mute are counted separately. Activity is control state, not sample amplitude. Other counters include supplied/clock frames, late/full submissions, discarded queued frames, stale calls, ordering errors and queue high water. Source starvation remains a separate upstream metric.

## Executable simulation and validation

From `firmware-rust`:

```sh
cargo run --locked -p pajoniiir-audio-runtime --example clocked_output
```

This headless example opens no audio device. It connects the real render graph to delivery and simulated 128-frame MAIN and 48-frame headphone consumers sharing a 48 kHz clock. It explicitly selects deck-one PFL as a demonstration cue mix; product cue/master composition and headphone gain remain separate integration work. Ten virtual seconds consume exactly 480,000 frames per lane with zero underrun, late or full events. MAIN peak 2,000 and pre-fader PFL peak 8,000 verify separation. The same scenario runs as a workspace integration test.

| Local check | Result |
|---|---|
| Full host workspace tests | PASS, 522 tests including 13 delivery scenarios |
| Host format and all-target Clippy with warnings denied | PASS |
| Desktop compile | PASS |
| P4 workflow check/link configurations and partition validator | PASS, 29 configurations |
| Invalid board/legacy/diagnostic combinations | PASS, 20 expected rejections |

Fixtures cover partial consumption, ring wrap, active starvation/recovery, idle/mute distinction, congestion in both directions, stale callbacks, route return to the same binding, DAC-only operation, token-role isolation, partially consumed queue discard, overflow, output gaps, fades and differing consumer clocks. Logs and source hashes are in [local evidence](evidence/m1-sink-delivery-2026-10-20/).

## Physical integration remains open

The embedded render owner remains on its historical software-timed reference sink. This gate adds reusable delivery and a connected host simulation, without replacing that task with a physical clock owner. Next establish exact-board USB/I2S mappings and the selected DAC; implement readiness, DMA/UAC buffer ownership, channel packing/sample conversion, controller feedback/drift handling and the clock authority. Serialize route/reset/callback work and connect source epochs to output-queue resets. Decide how consumed/dropped output updates transport; render/prefetch progress is not proof of playback.

Then verify one WAV, MAIN/PFL mapping, DAC fallback, reconnect, clock behavior, listening and combined-load deadlines on each exact image. APTA stays unmerged and existing custom-PCB edits remain outside this change.
