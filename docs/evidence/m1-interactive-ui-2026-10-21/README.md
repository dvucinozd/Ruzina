# Interactive UI evidence — 2026-10-21

Base: `963e730f614f948830f77ee6c93801e34f6105c7`, separate UI branch above PR #6.

- `host-*`, `desktop-check`, `ui-tests`, `ui-clippy`: format, all-target Clippy,
  workspace tests and desktop compilation. Full workspace total: 559 tests.
- `ui-smoke.log`: actual compiled Slint callbacks, pointer hit testing and
  Space/Return/Tab in both geometries. Play and cue-delete are held across model
  refresh. No physical touch or audio is exercised.
- `ui-replay.log`: a separate process reruns the scene sequence and matches all
  14 frozen raw RGB hashes via `firmware-rust/tools/check_ui_goldens.py`.
- `jc1060-*.png`, `jc8012-*.png`: lossless PNG conversions of those PPM RGB
  buffers. Reviewed empty, loaded/Sync, cue-edit, settings, malformed-analysis
  fallback (Library and Decks), and removed-media states at each documented size.
- `font-environment.json`: raster context and system font file identities;
  fonts themselves are not redistributed. Pixel baselines are environment-bound.
- `p4-01..29.log`, `p4-matrix-summary.log`, `board-guards.log`: P4 workflow and
  partition validation plus 20 expected invalid-feature rejections. These are
  compile/link evidence, not a running Guition UI.
- `source-manifest.json`: normalized source hashes for this change, excluding
  this evidence folder. `verification.txt` also checks unchanged external lock
  records and all 18 pre-existing primary-checkout KiCad file hashes.

Reproduce from `firmware-rust/`:

```sh
cargo run --locked -p desktop-simulator -- --smoke target/ui-smoke
python3 tools/check_ui_goldens.py target/ui-smoke
```

The pixel checker reads RGB PPM directly and uses only the Python standard
library. PNG conversion was performed with Pillow. The semantic host fixture
imports the prior synthetic DAT/EXT files, with no third-party audio or public
export binaries added here. See the [gate and open acceptance boundaries](../../M1_RUST_INTERACTIVE_UI_GATE_2026-10-21.md).
