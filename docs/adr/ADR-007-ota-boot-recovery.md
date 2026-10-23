# ADR-007: Signed A/B OTA and explicit boot readiness

**Status:** Accepted  
**Date:** 2026-09-24

## Context

Released Pajoniiir has signed update bundles, A/B update behavior, interruption safety and rollback semantics. M2.2 also exposed a post-update reboot panic class that must not be forgotten during the rewrite.

## Decision

Preserve signed application update verification, inactive-slot update, candidate boot, rollback and wired recovery.

A candidate image is marked READY only after critical startup self-tests succeed.

The 2026-10-22 [M2.5 parity audit](../M2_5_PARITY_PLAN.md) makes the unfinished
implementation contract explicit: readiness includes requested networking and
has a bounded timeout/rollback path. Pull OTA validates the entire manifest,
duplicate keys and bounds, signature, application/board/project identity,
comparable newer-only version, expiring offer, size and hash. TLS retains CA and
hostname verification, including valid cross-signed chains. Transfer retries
are bounded and failures expose a stage without credentials. Local signed
service rollback remains a separate policy from newer-only public pull.

Preserve `.ddjota` compatibility where practical; any incompatible successor requires a migration/versioning document.

## Consequences

- OTA is a product state machine with host fault-injection tests.
- Reboot/rollback paths receive dedicated hardware regression tests.
- Firmware health is part of the authoritative system state.
- Host failure injection must interrupt transfer, flash, settings and candidate
  confirmation boundaries before physical OTA qualification. Since 2026-10-23
  these rules exist as host contracts in `pajoniiir-ota` (RP-13 in the parity
  plan); no target partition, transport or TLS adapter exists yet, and no
  OTA, rollback or reboot behavior is hardware-verified.
