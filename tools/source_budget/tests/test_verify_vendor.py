import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from verify_vendor import verify


class VendorTests(unittest.TestCase):
    def test_unmodified_files_patch_and_manifest_are_all_pinned(self):
        sha = lambda raw: hashlib.sha256(raw).hexdigest()
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root/'pkg').mkdir()
            (root/'pkg/a.rs').write_bytes(b'upstream')
            (root/'pkg/b.rs').write_bytes(b'patch')
            baseline = json.dumps([dict(package='pkg', unmodified_file_sha256={
                'a.rs': sha(b'upstream'), 'b.rs': sha(b'before')})]).encode()
            (root/'upstream-files.json').write_bytes(baseline)
            policy = dict(baseline_manifest_sha256=sha(baseline), changes={
                'pkg/b.rs': dict(upstream_sha256=sha(b'before'), patched_sha256=sha(b'patch'))})
            self.assertEqual(verify(root, policy)['changed_files'], 1)
            (root/'pkg/b.rs').write_bytes(b'drift')
            with self.assertRaisesRegex(ValueError, 'source drift'):
                verify(root, policy)
            (root/'pkg/b.rs').write_bytes(b'patch')
            (root/'pkg/new.rs').write_bytes(b'new')
            with self.assertRaisesRegex(ValueError, 'untracked'):
                verify(root, policy)
            (root/'pkg/new.rs').unlink()
            (root/'upstream-files.json').write_bytes(b'[]')
            with self.assertRaisesRegex(ValueError, 'manifest changed'):
                verify(root, policy)
