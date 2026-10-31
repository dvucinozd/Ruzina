"""Check local Markdown file targets across the tracked project documentation."""
from pathlib import Path
import re
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[2]


def main():
    files = [ROOT / "README.md", *sorted((ROOT / "docs").rglob("*.md")),
             *sorted((ROOT / "firmware-rust").rglob("README.md")),
             *sorted((ROOT / "firmware-idf").rglob("README.md"))]
    checked = 0
    errors = []
    for file in files:
        # Vendored upstream trees keep their own README links byte-identical.
        if any(part in file.relative_to(ROOT).parts for part in ("target", "build", "third_party")):
            continue
        for destination in re.findall(r"\]\(([^)]+)\)", file.read_text(encoding="utf-8")):
            destination = destination.strip().strip("<>")
            parsed = urlsplit(destination)
            if parsed.scheme or not parsed.path:
                continue
            path = (file.parent / unquote(parsed.path)).resolve()
            if not path.is_relative_to(ROOT) or not path.exists():
                errors.append(f"{file.relative_to(ROOT)}: {destination}")
            checked += 1
    if errors:
        raise SystemExit("Broken local documentation links:\n" + "\n".join(errors))
    print(f"Documentation links: {checked} passed")


if __name__ == "__main__":
    main()
