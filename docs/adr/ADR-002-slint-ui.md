# ADR-002: Slint as the M1 UI framework

**Status:** Accepted  
**Date:** 2026-09-24

## Context

The released firmware uses LVGL. M1 is a native Rust project, and carrying LVGL forward would require a large C/Rust ownership and FFI surface.

Slint 1.18.1 supports Rust desktop development and a software renderer suitable for no_std MCU environments.

## Decision

Use Slint for M1 UI layout, text, controls, library/settings, deck chrome and normal interaction state.

Use one UI owner/context. Other tasks publish bounded messages/immutable snapshots.

Develop the same views first in an 800x480 desktop simulator.

## Consequences

- LVGL is not part of the planned M1 product architecture.
- Embedded Slint integration still requires P4 framebuffer/display qualification.
- UI parity is tested on desktop before hardware arrives.
- Unsupported/expensive waveform rendering is handled outside Slint per ADR-003.
