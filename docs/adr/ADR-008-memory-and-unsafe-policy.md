# ADR-008: Memory placement and unsafe-code policy

**Status:** Accepted  
**Date:** 2026-09-24

## Context

M1 has 32 MiB in-package PSRAM and uses USB, audio, PPA and DSI workloads where DMA/cache placement matters. Large UI/media allocations should not consume scarce internal SRAM, while latency-critical/DMA structures cannot be placed blindly in PSRAM.

## Decision

Use explicit memory classes:

- internal SRAM for latency-critical queues/descriptors and hardware structures that require it;
- PSRAM for large framebuffers, assets, waveforms, library/analysis caches and verified-safe bounded PCM storage.

Forbid unsafe code in product/domain crates where practical. Concentrate unavoidable unsafe operations in reviewed low-level platform/PAC/FFI wrappers with documented invariants.

## Consequences

- Memory budgets become testable contracts.
- PSRAM DMA/cache assumptions remain HARDWARE_PENDING until measured.
- Unsafe auditing is tractable because it is structurally localized.
