# ADR-004: Data-driven controller platform

**Status:** Accepted  
**Date:** 2026-09-24

## Context

Released Pajoniiir already contains data-driven FLX4, Hercules Inpulse 500 and Generic MIDI controller profiles. M1 must not regress to a single-controller hard-coded design.

The current profile format can express note/CC based control and LED behavior, but controller USB audio remains too FLX4-specific.

## Decision

Preserve S3CP v2 control/LED compatibility for R1 and port it to a Rust table-driven runtime.

Separate:

1. USB MIDI transport;
2. profile parser/runtime;
3. semantic ControlEvent model;
4. authoritative scheduler/reconciler;
5. semantic LED state;
6. controller-specific output mapping;
7. generic controller-audio capability.

## Consequences

- Existing profile tooling/fixtures remain valuable.
- Non-FLX4 physical support claims require hardware evidence.
- UAC capability metadata needs a versioned extension/model rather than hidden device-specific code.
- SysEx/MIDI-CI is deferred unless separately specified.
