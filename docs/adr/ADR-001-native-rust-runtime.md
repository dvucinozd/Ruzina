# ADR-001: Native Rust runtime

**Status:** Accepted  
**Date:** 2026-09-24

## Context

M1 is a new ESP32-P4 product and is not constrained to preserve the ESP-IDF/FreeRTOS application architecture used by released M2.2. The selected boards are Guition JC1060P470C_I_W_Y and JC8012P4A1C_I_W_Y. The native Rust runtime decision is unchanged; exact-board BSPs and physical acceptance are separate work.

Current esp-hal 1.2.0 supports ESP32-P4 revision >= v3.x and provides the required low-level direction, although several P4 peripherals needed by M1 are still unstable APIs.

## Decision

Use a native Rust `no_std` application architecture on ESP32-P4.

Do not make ESP-IDF application runtime, FreeRTOS or `esp-idf-sys` permanent product dependencies.

Retain compatibility with boot/partition/OTA formats only where it provides a concrete product benefit.

## Consequences

- Product/domain logic can be host tested without IDF.
- P4-specific unstable APIs must be isolated behind platform/BSP crates.
- Some drivers require project-owned qualification and possibly narrow PAC access.
- Exact toolchain and dependencies must be locked.
