import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import run_ram_probe as ram
from validate_unit import validate

REPO = Path(__file__).resolve().parents[4]


class Tests(unittest.TestCase):
    def test_ranges(self):
        ram.validate_segments(0x4ff00000, [(0x4ff00000, 32), (0x30100000, 16)])
        for entry, segments in [(0x4ff00000, [(0x4ff00000, 0)]),
                                (0x40000000, [(0x40000000, 32)]),
                                (0x4ff00040, [(0x4ff00000, 32)]),
                                (0x4ff00000, [(0x4ff00000, 32), (0x4ff00010, 32)]),
                                (0x4ff00000, [(0x4ff00000, 2**32)])]:
            with self.subTest(segments=segments), self.assertRaises(ValueError):
                ram.validate_segments(entry, segments)

    def test_wrong_hash_and_variant_under_optimization(self):
        with tempfile.TemporaryDirectory() as temp:
            p = Path(temp)
            (p / 'image').write_bytes(b'bad')
            for variant in ('pure-ram', 'flash-build-only'):
                (p / 'manifest').write_text(json.dumps(dict(variant=variant, bytes=3, sha256='bad')))
                result = subprocess.run([sys.executable, '-O', ram.__file__, '--image', p / 'image',
                                         '--manifest', p / 'manifest'], capture_output=True)
                self.assertNotEqual(result.returncode, 0)

    def test_wrong_header(self):
        from types import SimpleNamespace
        with patch('esptool.bin_image.LoadFirmwareImage', return_value=SimpleNamespace(
                chip_id=18, min_rev_full=302, max_rev_full=399)):
            with self.assertRaisesRegex(ValueError, 'header'):
                ram.verify(b'x', dict(variant='pure-ram', bytes=1, sha256=ram.digest(b'x')))

    def test_execute_failures_close_and_finalize(self):
        with tempfile.TemporaryDirectory() as temp:
            p = Path(temp)
            backup = b'\xff' * 16777216
            for name in ('a', 'b'):
                (p / name).write_bytes(backup)
            unit = dict(factory=dict(read1_file=str(p / 'a'), read2_file=str(p / 'b'),
                                     read1_sha256=ram.digest(backup)),
                        observed=dict(base_mac='expected'))
            for failure in ('identity', 'load', 'capture', 'reset', 'close', 'none'):
                class Fake:
                    closed = False
                    loaded = False
                    def identity(self):
                        return ('ESP32-P4', 103, 'wrong' if failure == 'identity' else 'expected')
                    def load(self, data):
                        self.loaded = True
                        if failure == 'load':
                            raise TimeoutError('load')
                    def capture(self):
                        if failure == 'capture':
                            raise TimeoutError('capture')
                        return b'HP_SYS_HP_WDT_RESET'
                    def reset(self):
                        if failure == 'reset':
                            raise TimeoutError('reset')
                    def close(self):
                        self.closed = True
                        if failure == 'close':
                            raise RuntimeError('close')
                fake = Fake()
                with patch.object(ram, 'verify'):
                    result = ram.execute(b'x', {}, p / failure, 'unused', unit,
                                         'usb-reset-observe', lambda _: fake)
                self.assertTrue(fake.closed)
                self.assertTrue((p / failure / 'result.json').is_file())
                self.assertNotEqual(result['recovery'], 'FACTORY_LOG_OBSERVED')
                if failure == 'identity':
                    self.assertFalse(fake.loaded)
                with patch.object(ram, 'verify'), self.assertRaises(FileExistsError):
                    ram.execute(b'x', {}, p / failure, 'unused', unit,
                                'usb-reset-observe', lambda _: self.fail('opened'))

    def test_rejected_input_never_opens_transport(self):
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaises(ValueError):
                ram.execute(b'bad', {'variant': 'flash-build-only'}, Path(temp) / 'out',
                            'unused', {}, 'usb-reset-observe', lambda _: self.fail('opened'))

    def test_unit_record(self):
        path = REPO / 'docs/boards/readiness/jc1060-20261030-com3.json'
        unit = validate(path)
        for field, value in [('display', 'DISPLAY_ACCEPTANCE_OPEN'), ('link', '../missing.log')]:
            changed = copy.deepcopy(unit)
            if field == 'display':
                changed['recovery']['normal_factory_boot'] = value
            else:
                changed['missing'] = value
            # Patch only primary JSON read; evidence reads still access original files.
            original = Path.read_text
            with patch.object(Path, 'read_text', lambda p, *a, **k:
                              json.dumps(changed) if p == path else original(p, *a, **k)):
                with self.assertRaises(ValueError):
                    validate(path)


if __name__ == '__main__':
    unittest.main()
