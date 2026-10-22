# ADR-004: Data-driven controller platform

**Status:** Accepted  
**Date:** 2026-09-24

## Context

Released Pajoniiir already contains data-driven FLX4, Hercules Inpulse 500 and Generic MIDI controller profiles. M1 must not regress to a single-controller hard-coded design.

The current profile format can express note/CC based control and LED behavior, but controller USB audio remains too FLX4-specific.

## Decision

Preserve S3CP v2 control/LED compatibility and complete v3/v4 compatibility for
R1 in the Rust table-driven runtime. This 2026-10-22 amendment follows the
[M2.5 parity audit](../M2_5_PARITY_PLAN.md); current code still supports v2 only.

V3 adds center-preserving CC7-to-14-bit scaling. V4 adds NOTE_SELECT, initial
output SysEx bounded to 128 bytes, LED scaling and a profile-owned channel-filter
policy. Parser, compiler, registry, storage and future upload validators must
agree; old v2/v3 golden bytes remain stable and tooling emits the oldest
representable version. Bound initialization and every deferred input/LED output
to the current connection/profile activation. Binary compatibility alone does
not imply that every newer semantic action is implemented.

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
- General input SysEx/MIDI-CI remains deferred; v4's bounded initial output
  SysEx is part of R1 compatibility.
- Latching controllers require a distinct idempotent PFL-set action and
  controller-owned disconnect cleanup, preserving legacy toggle behavior.
  This extension is optional until such a controller is selected for R1.
- UAC descriptor/packet/clock/volume capabilities and native host FIFO budgets
  need their own validation; a profile's USB-audio flag cannot establish them.
