"""Convert the donor's pre-rendered RGB565 logo arrays into Rust (pixels unchanged)."""
import re
import sys
from pathlib import Path

source, target = Path(sys.argv[1]), Path(sys.argv[2])
text = source.read_text()


def pixels(name):
    body = re.search(r"%s\[(\d+)\] = \{(.*?)\};" % name, text, re.S)
    values = re.findall(r"0x[0-9a-fA-F]{4}", body.group(2))
    assert len(values) == int(body.group(1)), name
    return values


def array(name, values, per_line=12):
    lines = [f"pub const {name}: [u16; {len(values)}] = ["]
    for i in range(0, len(values), per_line):
        lines.append("    " + ", ".join(v.lower() for v in values[i:i + per_line]) + ",")
    lines.append("];")
    return "\n".join(lines)


row, deck = pixels("s_row_pixels"), pixels("s_deck_pixels")
header = '''//! Attributed Pajoniiir logo shown when a loaded track has no usable cover
//! (RP-07, M2.5 default artwork).
//!
//! The operator-supplied logo `assets/default_artwork.svg` (SHA-256
//! cc7213894515a6438de36828c99386e7d9eebf6d5bb31bf70ea79c37bfc2661d) was
//! pre-rendered by the M2.5 tool `tools/generate_default_artwork.cjs` (sharp
//! 0.35.4 / libvips 8.18.6) at donor commit 20f1c3f0. These RGB565 pixels are
//! copied unchanged from that generated source, so no SVG renderer, decoder
//! or file access is needed to show the fallback. Do not hand-edit pixels.

use crate::artwork::ARTWORK_DECK_EDGE;

/// Library row thumbnail edge of the pre-rendered logo.
pub const DEFAULT_ARTWORK_ROW_EDGE: u16 = 40;

const _: () = assert!(ARTWORK_DECK_EDGE == 34, "regenerate the logo after a deck resize");
'''
target.write_text(
    header
    + "\n"
    + array("DEFAULT_ARTWORK_DECK", deck)
    + "\n\n"
    + array("DEFAULT_ARTWORK_ROW", row)
    + "\n"
)
print(target, len(deck), len(row))
