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
    def test_stack_frames(self):
        from stack_frames import frame_sizes, oversized_rust_frames
        text = '\n'.join([
            '400427f0 <ruzina_legacy_load_run>:',
            '400427f0:\taddi\tsp,sp,-256',
            '4004280c:\tlui\ta0,0x15',
            '4004280e:\taddi\ta0,a0,-928 # 14c60 <x>',
            '40042812:\tsub\tsp,sp,a0',
            '40042816:\taddi\tsp,sp,-4096',
            '40050000 <_ZN5small4poll17h0E>:',
            '40050000:\taddi\tsp,sp,-48',
            '40050002:\tret',
            '40050004:\taddi\tsp,sp,-99999',
            '40060000 <soc_regions>:',
            '40060000:\tlui\ta1,0xfffff',
            '40060004:\tsub\tsp,sp,a1',
            '40070000 <_ZN3big7inlined17h1E>:',
            '40070000:\tlui\tt0,0x8',
            '40070004:\tsub\tsp,sp,t0',
        ])
        self.assertEqual(frame_sizes(text), {'ruzina_legacy_load_run': 85344,
                                             '_ZN5small4poll17h0E': 48,
                                             '_ZN3big7inlined17h1E': 32768})
        self.assertEqual(oversized_rust_frames(text), {'ruzina_legacy_load_run': 85344,
                                                       '_ZN3big7inlined17h1E': 32768})

    def test_deepest_stack(self):
        from stack_frames import deepest_stack
        text = '\n'.join([
            '40000000 <root>:',
            '40000000:\taddi\tsp,sp,-100',
            '40000004:\tjal\t40000100 <small>',
            '40000008:\tauipc\tra,0x0',
            '4000000c:\tjalr\tra # 40000200 <deep>',
            '40000100 <small>:',
            '40000100:\taddi\tsp,sp,-10',
            '40000104:\tret',
            '40000200 <deep>:',
            '40000200:\taddi\tsp,sp,-1000',
            '40000204:\tjal\t40000200 <deep>',
            '40000208:\tj\t40000300 <leaf>',
            '40000300 <leaf>:',
            '40000300:\taddi\tsp,sp,-20',
            '40000304:\tret',
        ])
        self.assertEqual(deepest_stack(text, 'root'), (1120, ['root', 'deep', 'leaf']))
        self.assertEqual(deepest_stack(text, 'small'), (10, ['small']))

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
