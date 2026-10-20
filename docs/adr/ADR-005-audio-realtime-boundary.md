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
- Telemetry uses counters/fixed records and deferred formatting.
- Hardware soak remains mandatory even after host parity passes.
