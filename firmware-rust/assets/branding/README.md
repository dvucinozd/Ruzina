# Ruzina branding

Owner-supplied artwork for the Ruzina product, added on 2026-10-24. These
files are the sources of record; the firmware and simulator use derived forms.

| File | Size | SHA-256 | Use |
|---|---|---|---|
| `ruzina-emblem.png` | 878x887 RGBA | `308ed5b67391be96d7b9bab75d77e8d36ee00809625aca6758bd6fb7ad1ec861` | Fallback cover (34x34 / 40x40 RGB565 via `ruzina-media-catalog/assets/generate_default_artwork.py`); portrait screensaver |
| `ruzina-wordmark.png` | 824x200 RGBA | `7cdf364ceff17f44fdbeb29ef3785066519c6f6c9512bf9ec406590fea18f1a6` | Portrait screensaver, under the emblem |
| `ruzina-screensaver.png` | 1536x1024 RGB | `d505d269ad09e58b9375265766c43ed665757f5cad1dbe5adb4988f85dd32730` | Landscape screensaver (emblem and wordmark on rusted steel) |

The simulator embeds the PNGs at build time. A target display owner will need
its own derived format (for example RGB565 at the qualified panel size);
derive it from these files and record the conversion next to its output.

Do not replace these files with edited versions without updating the hashes
and the derived outputs together.
