# Interactive M1 host simulator

From `firmware-rust/`:

```sh
cargo run --locked -p desktop-simulator -- --board jc1060
cargo run --locked -p desktop-simulator -- --board jc8012
```

Documented board dimensions select 1024×600 (side-by-side decks) or 800×1280
(stacked decks). The board profiles do not activate a physical display or touch
adapter. A Linux X11 environment is required for the current pinned Slint backend.

1. Open **Library**, select a demo track, then **Load Deck 1/2**. Playing decks
   reject replacement. The first track has DAT/EXT; the second has DAT only.
   The last two entries exercise missing and rejected analysis with BPM fallback.
2. **Decks** exposes Play/Pause, Cue, Sync, Beat Jump, Loop In/Out/Exit and Quantize.
   The host clock acknowledges playback requests and advances the real deck
   state; no PCM is decoded or played. A loop wraps this simulated position.
3. **Hot Cues** uses eight effective source/local slots. Click an existing slot
   to seek, an empty slot to store the current position, or **×** to delete it.
   Session-local overrides/tombstones survive track reload, including media
   reconnect in this process. They are lost when the application closes.
4. **Settings** changes elapsed/remaining time, quantization and tempo range.
   It is a session UI, not a durable general settings store or audio-route owner.
5. **Remove Media** stops/unloads both decks and clears analysis. **Reconnect**
   permits a fresh explicit load; no old deck pointer is restored.

Buttons accept pointer input, keyboard focus/Tab and Space/Return. Semantic
actions and `DeckPresentation` are shared with product crates. Slint callbacks
and the timer run on the single UI thread. The shared RGB565 waveform rasterizer
composes imported columns with beat ticks, playhead, effective cue markers and
active loop endpoints before conversion to a Slint image. UI views do not parse
ANLZ or create a Slint item per waveform column.

## Headless regression

```sh
cargo run --locked -p desktop-simulator -- --smoke target/ui-smoke
python3 tools/check_ui_goldens.py target/ui-smoke
```

The actual compiled Slint view runs on its software renderer, without X11 or a
physical display. Callback replay plus pointer hit testing/keyboard input cover
both geometries, including a refresh during held Play and cue-delete clicks.
Fourteen PPM screenshots are compared with reviewed raw RGB SHA-256 goldens in
`tests/ui-goldens.json`. These pixel baselines require Linux x86_64, Slint 1.18.1,
DejaVu Sans and scale 1. Font/raster environment changes require visual review,
not automatic baseline replacement. Reviewed PNGs and logs are retained in the
[UI gate](../../../docs/M1_RUST_INTERACTIVE_UI_GATE_2026-10-21.md).

This package covers the interactive deck/Library/cue/session-settings slice.
Full M2.2 UI parity, mixer/FX/diagnostics/performance screens, arbitrary external
media browsing, real PCM timing/audio, durable settings/cues and Guition
display/touch rotation/resource qualification remain separate work.
