# M1 completed-output transport progress

Date: 2026-10-21

Base: `b3da05ea5422f87da50709dedf5fd08df03fd810`, following the [sink-delivery gate](M1_RUST_SINK_DELIVERY_GATE_2026-10-20.md). This closes the host/reference integration of output completion with transport position and EOF. Physical I2S/UAC adapters and listening acceptance remain open.

## Completion is distinct from submission

[`SinkLane`](../firmware-rust/crates/pajoniiir-audio-runtime/src/sink.rs) now separates `issue`, which fills a driver-owned PCM buffer, from `complete`, which acknowledges its consumption. An opaque receipt carries a lane-instance identity, role/route/reset token, ordered output-frame interval and optional transport checkpoint. Drivers retain receipts with DMA/UAC buffers and return them in issue order only on completion. Copying into a queue or driver buffer does not publish position or EOF.

At most eight receipts per lane can be outstanding. Exhaustion returns an error and zeroed destination without dequeuing more PCM. Duplicate/out-of-order receipts fail without advancing completion. Reset invalidates queued PCM and in-flight tickets. Constructor identities prevent a ticket from an old owner matching a new owner with identical routes, sources and frame coordinates. Counters distinguish issued/completed frames, rejected completions, queue discards and revoked in-flight frames. The older `consume` helper performs immediate completion for synchronous simulations; physical adapters must use the split API.

Checkpoints hold exact per-deck source positions captured at the end of a 128-frame rendered block, including source rate, generation, EOF/fault and consumed-source count. Partial output packets do not interpolate source position: they wait until the entire associated output block completes. This is conservative block-boundary publication, at most 128 output frames (about 2.67 ms at 48 kHz) behind confirmed completion. It avoids inventing intermediate positions for scratch, reverse or keylock. EOF can be delayed through trailing silence in the final block, but cannot precede its completion.

## Serialized playback owner

[`PlaybackOutput`](../firmware-rust/crates/pajoniiir-audio-runtime/src/playback.rs) coordinates delivery, current source generations and bounded pending transport publication. MAIN is the authority while bound; controller headphones become authority when MAIN is absent. Neither route means no render demand or confirmed transport progression. An unchanged headphone route survives MAIN route changes. Secondary completions release their driver slots without double-counting source progress. A shared output-coordinate watermark prevents a delayed older headphone checkpoint undoing already published MAIN progress during handover; source positions may still move backward on a later valid vinyl checkpoint.

Before applying preparation, seek, replacement, release or media invalidation, synchronize current active source generations. Since rendered MAIN/PFL mixes contain both decks, a generation change flushes both output queues and revokes their receipts. Render permits captured before that change cannot submit. Pending/confirmed progress for the changed deck is cleared; the other deck's previously confirmed progress remains valid. The output timeline must remain monotonic within one owner session; a backward reset is rejected before mutating queues.

Discarded PCM is never reported as consumed. This layer does not reconstruct mixed buffers or rewind DSP state after a discard. A later completed block can publish a later source position across an explicitly discarded gap; gapless device recovery/re-priming still needs adapter integration and listening tests. Actual DMA/UAC work must be cancelled/drained by its physical owner: revoking a software receipt cannot erase PCM already held by hardware.

Pause applies at a render boundary: already queued/issued PCM can finish, then paused blocks publish no source movement. Underrun silence and idle/mute output without a source checkpoint do not invent transport progression. Progress congestion coalesces only completed snapshots into one pending item per deck, preserving latest vinyl position/EOF and sticky faults. No completion path waits on the transport channel.

## Connected reference and host paths

The legacy [`audio_render_owner`](../firmware-rust/apps/m1-firmware/src/audio_render_owner.rs) now routes canonical render output through `PlaybackOutput` and the bounded queue. It acknowledges a reference receipt only after `ReferenceSink` accepts the prepared PCM, then publishes completed progress. Fresh active source generations are checked independently of potentially older render-control snapshots. Existing source/gesture gates remain in force.

The reference binding is explicit and advertises no physical DAC/controller readiness; it cannot bind headphones. Its acceptance is immediate, so no PCM is left queued/in flight between reference loop iterations. The task remains software-timed, with statically allocated delivery buffers; ordinary Guition selectors and production defaults are unchanged. This is a linked reference integration, not DMA/audio-clock or physical playback evidence.

The headless `clocked_output` example now connects canonical PCM history, render, both output queues, delayed driver receipts, `PlaybackOutput` and `PcmTransportCursor`. At ten virtual seconds both lanes complete 480,000 output frames with zero underrun; transport confirms frame 480,000 while render has reached 480,256 and source prefetch 482,048. MAIN peak 2,000 and pre-fader PFL peak 8,000 remain separate. These are deterministic fixture results, not device measurements.

## Validation

| Local gate | Result |
|---|---|
| Full host workspace tests | PASS, 536 tests including 14 new playback-completion cases |
| Host format, all-target Clippy with warnings denied, desktop compile | PASS |
| P4 workflow matrix and partition validation | PASS, 29 check/link configurations |
| Invalid board/legacy/diagnostic combinations | PASS, 20 expected rejections |
| Canonical-to-output-to-transport simulation | PASS, ten virtual seconds |

Tests cover submission without completion, partial packets and EOF, ordered/replayed/stale receipts, source replacement, queued/issued old EOF during seek, current-generation markers, preserved other-deck progress, MAIN/phone authority and delayed handover, no-output freeze, pause/idle/starvation, vinyl rewind under publication congestion, bounded outstanding buffers, malformed zero-rate faults and owner restart. Both Cargo locks and all 18 pre-existing primary-checkout KiCad edits remain unchanged. Logs and source hashes are retained in [local evidence](evidence/m1-playback-progress-2026-10-21/).

Next implement and qualify the exact-board physical adapters, DMA/UAC completion semantics, shared output coordinates, readiness, feedback/drift, channel packing and recovery. Per-unit boot/memory/USB-power observations and the selected MAIN DAC/wiring remain prerequisites. Rekordbox analysis and UI can proceed independently while boards are in transit. APTA remains unmerged.
