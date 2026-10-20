# ADR-003: Dedicated RGB565 waveform renderer

**Status:** Accepted  
**Date:** 2026-09-24

## Context

Pajoniiir waveforms are high-update-rate visualizations with beat grids, cues and a moving playhead. Representing waveform samples as large numbers of GUI elements is unnecessary overhead.

## Decision

Implement waveforms in a separate no_std-capable Rust crate.

The renderer targets RGB565 and combines:

- direct span writes for high-volume waveform columns;
- embedded-graphics for suitable primitives/markers/grid operations;
- deterministic host/golden-image tests.

Slint reserves waveform rectangles; a compositor renders the waveform after the Slint pass and before the hardware output conversion.

## Consequences

- Waveform code is independent of Slint and MIPI DSI.
- The same renderer can be tested on desktop and used on P4.
- P4 PPA RGB565->RGB888 conversion remains a hardware integration gate.
