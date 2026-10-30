import hashlib
import json
from pathlib import Path
import re
import unittest


class PanelTableTests(unittest.TestCase):
    def test_exact_reviewed_records(self):
        app = Path(__file__).resolve().parents[2]
        source = (app / 'main/new_panel_commands.h').read_text()
        records = []
        for cmd, params, length, delay in re.findall(
                r'\{(0x[0-9A-Fa-f]+),\s*\{([^}]+)\},\s*(\d+),\s*(\d+)\}', source):
            payload = [int(x.strip(), 0) for x in params.split(',')]
            self.assertEqual(len(payload), int(length))
            self.assertLessEqual(int(length), 14)
            records.append([int(cmd, 0), payload, int(length), int(delay)])
        self.assertEqual(len(records), 60)
        self.assertEqual(hashlib.sha256(json.dumps(records, separators=(',', ':')).encode()).hexdigest(),
                         'a784b0c9c5311e406e6ed3d982b8eba07609d9ca3caac744d1e4b48efc11664e')
        self.assertEqual(records[-2:], [[0x11, [0], 1, 120], [0x29, [0], 1, 20]])
