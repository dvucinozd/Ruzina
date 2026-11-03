"""Verify the legacy MP3 source copy and its three explicitly pinned edits."""
import hashlib
import json
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def contained(root, relative):
    path = (root / relative).resolve()
    if not path.is_relative_to(root.resolve()):
        raise ValueError('Vendor path escaped root')
    return path


def verify(root, policy):
    raw = (root / 'upstream-files.json').read_bytes()
    if sha(raw) != policy['baseline_manifest_sha256']:
        raise ValueError('Baseline source manifest changed')
    seen, changed = set(), set()
    for package in json.loads(raw):
        folder = contained(root, package['package'])
        expected = set(package['unmodified_file_sha256'])
        actual = {p.relative_to(folder).as_posix() for p in folder.rglob('*') if p.is_file()}
        if actual != expected:
            raise ValueError('Missing or untracked vendor source file')
        for name, upstream in package['unmodified_file_sha256'].items():
            relative = package['package'] + '/' + name
            entry = policy['changes'].get(relative)
            if entry and entry['upstream_sha256'] != upstream:
                raise ValueError('Patch upstream digest mismatch')
            expected_sha = entry['patched_sha256'] if entry else upstream
            if sha(contained(root, relative).read_bytes()) != expected_sha:
                raise ValueError('Vendor source drift: ' + relative)
            seen.add(relative)
            if entry:
                changed.add(relative)
    if changed != set(policy['changes']):
        raise ValueError('Unmatched patch entry')
    return dict(status='PINNED_VENDOR_BYTES_PASS', files=len(seen), changed_files=len(changed),
                device_access=False)


if __name__ == '__main__':
    policy = json.loads((Path(__file__).with_name('vendor-patch.json')).read_bytes())
    print(json.dumps(verify(REPO / 'firmware-idf/legacy-jc1060/third_party', policy)))
