# ADR-005: Audio real-time boundary

**Status:** Accepted  
**Date:** 2026-09-24

## Context

Released M2.2 product quality depends on bounded media/decode/audio behavior and long hardware soaks. Rust does not remove real-time deadline constraints.

## Decision

Define a strict real-time audio boundary.

The deadline path may perform PCM/source arbitration, scratch, resampling/Master Tempo, DSP, mixing, limiter, bounded queue/ring operations and hardware sink submission.

It must not perform filesystem, network, OTA, controller profile parsing, formatted logging or unbounded allocation.

## Consequences

- DSP and state machines are written as host-testable product crates.
- I2S and UAC are sink adapters rather than owners of product logic.
- Owner routing clarification (2026-10-20): MAIN automatically prefers a ready external stereo DAC, then compatible controller MAIN audio, then mute. Headphones/CUE/PFL always use controller UAC headphone channels; no DAC/onboard-codec headphone fallback is permitted. MAIN changes preserve the independent headphone route; controller absence leaves headphones unavailable. Physical sink implementation and channel mapping still require qualification.
- Telemetry uses counters/fixed records and deferred formatting.
- Hardware soak remains mandatory even after host parity passes.
