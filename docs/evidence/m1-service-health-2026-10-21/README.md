# Service/health evidence — 2026-10-21

Base: `eac8481e2a91f7eaf8971c36bfa8188ba42a3f3e`, mixer/FX PR #8.

- `host-*`, `ui-*`, `desktop-check`: 571 workspace tests, formatting, all-target
  warnings-denied Clippy, desktop compilation and compiled UI replay.
- `ui-replay.log`: a separate process reproduces all 34 reviewed pixel hashes.
- `jc1060-*.png`, `jc8012-*.png`: lossless conversions from RGB PPM frames. The
  new Health states are initial, removed media, event history, recovered media
  and rejected-analysis fallback. Prior 24 scene bodies are unchanged below nav.
- `*-health-*.txt`: reports from those actual headless session states.
  `service-cli-jc8012.txt` comes from a fresh, separate CLI host process; none
  of these reports is a physical device observation.
- `font-environment.json`: renderer/style/font identities; fonts are not copied.
- `p4-01..29.log`, `p4-matrix-summary.log`, `board-guards.log`: target matrix,
  partition validation and 20 expected invalid-feature rejections. Boot adapter
  checks are compilation/link evidence, not records read from a unit.
- `source-manifest.json`, `verification.txt`: source hashes, external lock
  record preservation and all 18 pre-existing primary KiCad file hashes.

From `firmware-rust/`:

```sh
cargo run --locked -p desktop-simulator -- --smoke target/ui-smoke
python3 tools/check_ui_goldens.py target/ui-smoke
cargo run --locked -p desktop-simulator -- --diagnostic-report target/service.txt jc8012
```

The image checker uses Python's standard library; PNG conversion used Pillow.
Review changed pixels and the recorded raster/font environment before changing
baselines. See the [gate](../../M1_RUST_SERVICE_HEALTH_GATE_2026-10-21.md) for
scope, boot-snapshot semantics and unrun physical/runtime acceptance.
