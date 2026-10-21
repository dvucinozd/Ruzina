"""Check reviewed Slint RGB snapshots (software renderer, DejaVu Sans).

Usage: python3 tools/check_ui_goldens.py OUTPUT_DIRECTORY
Pixel baselines are deliberately immutable here: review changed PNGs before
editing tests/ui-goldens.json. No third-party Python packages are required.
"""
import hashlib
import json
from pathlib import Path
import sys


def main():
    output = Path(sys.argv[1])
    baseline = Path(__file__).resolve().parents[1] / "apps/desktop-simulator/tests/ui-goldens.json"
    records = json.loads(baseline.read_text(encoding="utf-8"))["snapshots"]
    for name, expected in records.items():
        with (output / (name + ".ppm")).open("rb") as file:
            assert file.readline() == b"P6\n", name
            width, height = map(int, file.readline().split())
            assert file.readline() == b"255\n", name
            pixels = file.read()
        assert [width, height] == expected["size"], name + ": geometry changed"
        assert len(pixels) == width * height * 3, name + ": truncated RGB snapshot"
        actual = hashlib.sha256(pixels).hexdigest()
        assert actual == expected["rgb_sha256"], (
            name + ": pixels changed; review screenshot/layout/state and confirm DejaVu Sans font environment. "
            + "Actual SHA-256: " + actual
        )
    print(f"Reviewed Slint screenshot goldens: {len(records)} passed")


if __name__ == "__main__":
    main()
