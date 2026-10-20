# ADR-007: Signed A/B OTA and explicit boot readiness

**Status:** Accepted  
**Date:** 2026-09-24

## Context

Released Pajoniiir has signed update bundles, A/B update behavior, interruption safety and rollback semantics. M2.2 also exposed a post-update reboot panic class that must not be forgotten during the rewrite.

## Decision

Preserve signed application update verification, inactive-slot update, candidate boot, rollback and wired recovery.

A candidate image is marked READY only after critical startup self-tests succeed.

Preserve `.ddjota` compatibility where practical; any incompatible successor requires a migration/versioning document.

## Consequences

- OTA is a product state machine with host fault-injection tests.
- Reboot/rollback paths receive dedicated hardware regression tests.
- Firmware health is part of the authoritative system state.
