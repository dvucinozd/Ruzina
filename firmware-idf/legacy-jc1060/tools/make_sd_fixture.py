"""Create deterministic PC-side SD content; never discovers or writes a drive."""
import argparse
import hashlib
import json
from pathlib import Path

SIZE = 8 * 1024 * 1024 + 513
RELATIVE = 'RUZINA_TEST/Čitanje i seek 64GB.bin'


def block(offset, count):
    return bytes((p * 73 + (p >> 8) * 19 + (p >> 17) * 131 + 0x5a) & 255
                 for p in range(offset, offset + count))


def create(output):
    output = Path(output)
    output.mkdir(parents=True, exist_ok=False)
    destination = output / RELATIVE
    destination.parent.mkdir()
    digest = hashlib.sha256()
    with destination.open('xb') as stream:
        for offset in range(0, SIZE, 65536):
            data = block(offset, min(65536, SIZE - offset))
            stream.write(data)
            digest.update(data)
    manifest = dict(schema=1, relative_path=RELATIVE, bytes=SIZE, sha256=digest.hexdigest(),
                    pattern='(p*73+(p>>8)*19+(p>>17)*131+0x5a)&255',
                    scope='deterministic arbitrary binary fixture, not audio/export corpus')
    (output / 'manifest.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
    return manifest


if __name__ == '__main__':
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(create(args.output), ensure_ascii=False, indent=2))
