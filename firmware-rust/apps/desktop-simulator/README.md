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

**Mixer** exposes both channels' Trim/EQ/Filter/level/PFL and global crossfader,
master, headphones, Master Cue, Smart CFX and Smart Fader. Sliders support drag,
Home/End and arrow keys; a later shared-state publication updates the same
focused slider. Percent readouts are normalized control positions, not dB.

**FX** exposes Beat FX effect/beat/target/depth/On/Clear and two eight-pad banks
per deck. Hold a pad with pointer or Space/Return; release, pointer exit/cancel,
focus loss or page departure stops it. A bank change releases that deck's pad.
Media removal clears pads and Beat FX; mixer configuration remains unchanged.
These controls update semantic state, with no processed audio or fake meters.

**Health** shows the documented board candidate separately from observations,
current media/analysis health, recovery guidance, counters and the last 16 actual
session events. Events scroll when needed. Reconnect clears the current absent-
media fault without erasing warning history. Hardware identity, memory and CPU
measurements remain unknown in this host harness. Product readiness is not
inferred. Counters describe this demo session, not audio/USB driver loss.

Export a fresh, separate host-process report without opening a window:

```sh
cargo run --locked -p desktop-simulator -- --diagnostic-report target/service.txt jc8012
```

This command does not query an already-running UI or a physical board. `--smoke`
also exports the reports from its current Health scene states.

## Headless regression

```sh
cargo run --locked -p desktop-simulator -- --smoke target/ui-smoke
python3 tools/check_ui_goldens.py target/ui-smoke
```

The actual compiled Slint view runs on its software renderer, without X11 or a
physical display. Callback replay plus pointer hit testing/keyboard input cover
both geometries, including a refresh during held Play and cue-delete clicks.
Thirty-eight PPM screenshots are compared with reviewed raw RGB SHA-256 goldens in
`tests/ui-goldens.json`. These pixel baselines require Linux x86_64, Slint 1.18.1,
the pinned Fluent style, DejaVu Sans and scale 1. Font/raster environment changes require visual review,
not automatic baseline replacement. Reviewed PNGs and logs are retained in the
[runtime freshness gate](../../../docs/M1_RUST_RUNTIME_HEALTH_GATE_2026-10-21.md).

The host session publishes a heartbeat after owner work, including while Idle.
Health shows generation, sequence and sample age; age above 1000 ms is Stale.
Reading or drawing Health never renews a publication. Replay separately advances
the observer to exercise a stopped publisher and Idle recovery. This is not an
event-loop watchdog. V2 diagnostic reports include publication metadata; the
device boot report remains a static snapshot.

This package covers interactive decks, Library, cues, mixer/PFL/FX and session settings.
Full M2.2 UI parity, runtime diagnostics/performance owner adapters, arbitrary external
media browsing, real PCM timing/audio and audible FX execution, durable settings/cues and Guition
display/touch rotation/resource qualification remain separate work.
