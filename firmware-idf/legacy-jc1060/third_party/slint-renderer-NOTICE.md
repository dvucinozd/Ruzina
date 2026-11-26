# Vendored Slint software renderer (`i-slint-renderer-software` 1.18.1)

`i-slint-renderer-software-1.18.1/` holds the crates.io package of the same name and
version, the one `slint =1.18.1` already locked. `rust-core/Cargo.toml` puts it in place
of the registry copy with `[patch.crates-io]`.
- Copied from the local cargo registry. Nothing was downloaded for it.
- The package's `.cargo-ok`, `.cargo_vcs_info.json` and `Cargo.lock` are left out.
- **License:** as upstream: `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR
  LicenseRef-Slint-Software-3.0`, the texts in its `LICENSES/`. The vendored copy changes
  nothing about which of these the firmware uses Slint under.
- **Per-file SHA-256** of all 17 files as upstream ships them: `upstream-files.json`. The
  changed files' upstream and patched digests are pinned in
  `tools/source_budget/vendor-patch.json`; `tools/source_budget/verify_vendor.py` checks
  both in CI.

## The changes

Two files, each change under a `Ruzina (JC1060 deck screen)` comment.

### Glyphs at their own size

`draw_functions.rs`, in `draw_texture_line`, under a `Ruzina (JC1060 deck screen)`
comment: a fast path for a glyph drawn at its own size.
- **When it applies:** an alpha map, no rotation, one source pixel per target pixel, and
  whole-pixel offsets. Text on the deck screen always meets these.
- **What it does:** it reads the glyph's row directly. It skips clear pixels and writes
  solid ones without blending.
- **What it drops:** the general path steps through the source in fixed point, with
  divisions and remainders for tiling, for every glyph on every line.
- **Fallback:** anything else, or a row the fast path cannot read in full, takes the
  general path as before.
- **Same pixels:** on the host, the Overview, Library, Hot Cues and Settings pages drew
  the same pixels (frame checksums) with and without the change.

On the host, at `opt-level = 3`:
- a full Library page drew in about 660 µs instead of about 818 µs;
- Settings drew in 150 µs instead of 174 µs.

### Timing the scene's preparation

`lib.rs`: `SoftwareRenderer::set_probe_clock` gives the renderer a clock, and
`SoftwareRenderer::scene_prep` returns where the last `render_by_line` call's preparation
of the scene went (`ScenePrep`):
- the dirty tracking over the whole item tree;
- going over the items to make the scene's commands, and of that, the texts;
- sorting the commands by line;
- the texts gone over and their glyphs.

Without a clock nothing is timed and the drawing is unchanged; with one it reads the clock
about twice per text. The deck screen logs these parts for its slow frames (events 50 to
52).

To move to a newer Slint, copy that version's package and carry both changes over, or
drop the fast path if upstream draws glyphs at their own size without the general path.
