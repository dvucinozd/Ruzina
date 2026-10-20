"""Verify the owner-supplied packages against the audited source manifest.

Run on the machine containing the supplied packages. This reads files only;
it never executes vendor code or copies controller firmware into the project.
"""
import argparse
import hashlib
import json
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--base", type=Path, help="Directory containing both model folders")
args = parser.parse_args()
repo = Path(__file__).resolve().parents[2]
manifest = json.loads((repo / "docs/boards/source-manifest.json").read_text(encoding="utf-8"))
count = 0
for item in manifest["files"]:
    if item["kind"] != "owner-supplied-vendor-bundle":
        continue
    source = Path(item["origin"])
    if args.base:
        model = "JC1060P470C_I_W_Y" if item["id"].startswith("jc1060") else "JC8012P4A1C_I_W_Y"
        # Some additional conflict evidence uses only its complete source path.
        relative = item.get("relative_path") or item["origin"].split(model, 1)[1].lstrip("/\\")
        source = args.base / model / relative.replace("\\", "/")
    data = source.read_bytes()
    if len(data) != item["bytes"] or hashlib.sha256(data).hexdigest() != item["sha256"]:
        raise RuntimeError(f"Audited source changed: {source}")
    count += 1
    print(f"PASS: {item['id']}")
print(f"Owner source hashes: {count} passed")
