# Default artwork

A loaded track without a usable cover shows the Ruzina emblem (RP-07). Its
source of record is the owner-supplied
`firmware-rust/assets/branding/ruzina-emblem.png` (see that folder's README).

`src/default_artwork.rs` holds the 34x34 deck and 40x40 Library RGB565 pixels.
`generate_default_artwork.py` renders them with the Python standard library
only, so the result does not depend on an image tool version:

```text
python3 -I generate_default_artwork.py ../../../assets/branding/ruzina-emblem.png ../src/default_artwork.rs
```

The script decodes the PNG, composites any alpha over black, crops a square
around the visible emblem (luminance above 40, 3% margin) and area-averages it
to each edge. No SVG renderer, decoder or file access is needed at runtime.
Changing the emblem or a thumbnail size means rerunning the script and
reviewing the UI goldens.

Until 2026-10-24 this was the attributed M2.5 Pajoniiir logo
(`default_artwork.svg`, SHA-256
`cc7213894515a6438de36828c99386e7d9eebf6d5bb31bf70ea79c37bfc2661d`); it remains
in the donor and in this repository's history.
