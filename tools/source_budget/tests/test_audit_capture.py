import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from audit_capture import audit, sha


class ControlAuditTests(unittest.TestCase):
    def fixture(self):
        raw = (b'midi_event deck=1 control=Load(40) kind=1 value=1 max=1\n'
               b'midi_event deck=2 control=JogScratch(2) kind=2 value=1 max=0\n'
               b'deck_engine_stats unsupported=1\n')
        capture = dict(raw_sha256=sha(raw), cases=[dict(name='s03-deck1', start_offset=0,
                        end_offset=len(raw), raw_sha256=sha(raw))])
        return raw, capture, dict(raw_sha256=sha(raw))

    def test_extra_control_and_counter_are_retained_without_admission_mutation(self):
        raw, capture, adapter = self.fixture()
        before = copy.deepcopy(capture)
        result = audit(raw, capture, adapter)
        self.assertEqual(capture, before)
        self.assertEqual([r['control'] for r in result['cases'][0]['extra_controls']], ['JogScratch'])
        self.assertEqual(result['unsupported_transitions'][0]['current'], 1)
        self.assertFalse(result['device_access'])
        self.assertEqual(result['status'], 'OFFLINE_CONTROL_AUDIT_NOT_ADMISSION')

    def test_raw_and_case_tampering_are_rejected(self):
        raw, capture, adapter = self.fixture()
        with self.assertRaisesRegex(ValueError, 'Raw stream digest'):
            audit(raw+b'x', capture, adapter)
        capture['cases'][0]['raw_sha256'] = '0'*64
        with self.assertRaisesRegex(ValueError, 'Case slice digest'):
            audit(raw, capture, adapter)

    def test_overlap_and_incomplete_control_are_rejected(self):
        raw, capture, adapter = self.fixture()
        capture['cases'].append(dict(capture['cases'][0]))
        with self.assertRaisesRegex(ValueError, 'overlapping'):
            audit(raw, capture, adapter)
        malformed = b'midi_event deck=1 control='
        with self.assertRaisesRegex(ValueError, 'Malformed/incomplete'):
            audit(malformed, dict(raw_sha256=sha(malformed)), dict(raw_sha256=sha(malformed)))

    def test_boundary_control_is_not_silently_assigned_to_a_case(self):
        raw, capture, adapter = self.fixture()
        capture['cases'][0]['start_offset'] = 1
        capture['cases'][0]['raw_sha256'] = sha(raw[1:])
        result = audit(raw, capture, adapter)
        self.assertEqual(len(result['cases'][0]['boundary_crossing_controls']), 1)
        self.assertEqual(result['cases'][0]['status'], 'INCOMPLETE_BOUNDARY_CONTROL_AUDIT')
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root/'stream.log').write_bytes(raw)
            (root/'result.json').write_text(json.dumps(capture))
            (root/'adapter-result.json').write_text(json.dumps(adapter))
            completed = subprocess.run([sys.executable, str(Path(__file__).resolve().parents[1]/'audit_capture.py'),
                                        '--capture', str(root), '--output', str(root/'audit.json')],
                                       capture_output=True, text=True, check=True)
            case = json.loads(completed.stdout)['cases'][0]
            self.assertEqual(case['status'], 'INCOMPLETE_BOUNDARY_CONTROL_AUDIT')
            self.assertEqual(case['boundary_crossing_controls'], 1)


if __name__ == '__main__':
    unittest.main()
