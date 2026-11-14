# JC1060: Master Tempo on the controller

Date: 2026-11-14. Branch: `ogranak/jc1060-mt-button`, from `main` at `e4db976`.

## Problem

Master Tempo could only be turned over by a tap on the deck screen's MT box
([Master Tempo](2026-11-13-master-tempo.md)). The DDJ-FLX4 has no Master Tempo button.

## Change

The owner chose Shift + BEAT SYNC. The FLX4 sends it as the deck's `TempoRange` button.
- **A short press** steps the tempo range (6/10/16 %), as before. It now takes effect
  on release instead of on press.
- **Held for a second**, it turns the deck's Master Tempo over instead, through the same
  `DeckExtAction::MasterTempo` as the screen. It happens while still held, so the MT box
  lights at once; the release then does nothing, and the range stays.
- **Timing:** the engine counts the hold in rendered frames, in `Engine::progress`
  (48 000 frames is one second).
- **Scope:** the FLX4 profile is unchanged; this is the legacy engine only.

## Evidence

- **Host:** a new engine test.
  - A short press steps the range on release and leaves Master Tempo alone.
  - A hold turns it on just after a second, once per hold, and the range stays.
  - A second hold turns it off.

  The legacy CI matrix, the Slint checks and target clippy pass. The IDF build has no
  warnings.
- **Device:** s94 (app `06714ef5…`), owner verdict "Radi".
  - The hold turns Master Tempo on and off.
  - A short press steps the range.
  - In the first minutes: underruns 0/0, `packet_errors=0`, longest fill 6.1 ms.

## Test plan

1. Hold Shift + BEAT SYNC for a second: MT lights, and the key stays when the tempo moves.
2. Hold it again: MT goes out.
3. A short Shift + BEAT SYNC steps the tempo range, and MT stays as it was.
