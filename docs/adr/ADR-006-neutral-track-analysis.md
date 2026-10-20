# ADR-006: Provider-neutral TrackAnalysis

**Status:** Accepted  
**Date:** 2026-09-24

## Context

Released behavior is Rekordbox-centric, while the long-term product direction includes ordinary media folders and libapta analysis.

Direct ANLZ coupling would make later APTA integration invasive.

## Decision

Create an immutable provider-neutral `TrackAnalysis` model consumed by deck, Sync, Beat Jump, waveform and UI code.

Rekordbox PDB/ANLZ is an importer/provider, not the application model.

libapta becomes another provider only after its embedded/release gates are satisfied.

## Consequences

- Rekordbox parity can be proven with golden fixtures.
- A playing deck pins an analysis generation.
- Analysis provenance/confidence can be represented without changing consumers.
- APTA can be integrated later without rewriting deck/UI logic.
