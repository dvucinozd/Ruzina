# Mixer/PFL and FX UI evidence — 2026-10-21

Base: `df1524b9a98ba52a740577f718017626619732da` (UI PR #7).

- `host-*`, `desktop-check`, `ui-clippy`, `ui-tests`: 564 workspace tests,
  format, all-target warnings-denied Clippy and desktop compilation.
- `ui-smoke.log`: compiled Slint callbacks and actual pointer/keyboard input
  in both board geometries. Mixer covers PFL, slider Home/End, external value
  publication and a drag spanning refresh. FX covers effect/target/depth/toggle,
  bank identity, hold/release, pointer exit, focus loss and held-pad detach.
- `ui-replay.log`: a fresh process reproduces all 24 reviewed pixel hashes.
- `jc1060-*.png`, `jc8012-*.png`: lossless conversions of raw RGB PPM snapshots.
  New Mixer/FX states supplement the existing deck/Library/cue/settings/recovery
  scenes. Prior scenes are unchanged below the new navigation.
- `font-environment.json`: renderer/style/font identities; fonts are not copied.
- `p4-01..29.log`, `p4-matrix-summary.log`, `board-guards.log`: workflow matrix,
  partition validation and 20 expected invalid-feature rejections.
- `source-manifest.json`: LF-normalized source hashes excluding this evidence
  folder. `verification.txt` checks external lock records and existing KiCad
  hashes as well as the local gate totals.

From `firmware-rust/`, reproduce with:

```sh
cargo run --locked -p desktop-simulator -- --smoke target/ui-smoke
python3 tools/check_ui_goldens.py target/ui-smoke
```

The checker uses Python's standard library; PNG conversion used Pillow. Pixel
baselines require the recorded renderer/Fluent/DejaVu environment and visual
review after intentional changes. These are host semantic/input/render results,
not audible DSP, device or physical acceptance. See the [gate](../../M1_RUST_MIXER_FX_UI_GATE_2026-10-21.md).
