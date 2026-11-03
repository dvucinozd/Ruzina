import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare import POLICY, UNIT, exact_return, replay, sha


class ExactReturnTests(unittest.TestCase):
    def candidate(self, root):
        baseline = b'B' * 0x20000
        current = bytearray(baseline)
        folder = root/'candidate'
        folder.mkdir()
        rows = []
        for offset, image in [(0x2000, b'boot'*300), (0x8000, b'part'*50), (0x10000, b'audio'*1600)]:
            end = (offset+len(image)+4095)//4096*4096
            payload = image+baseline[offset+len(image):end]
            name, undo = f'write-{offset:x}.bin', f'restore-{offset:x}.bin'
            (folder/name).write_bytes(payload)
            (folder/undo).write_bytes(baseline[offset:end])
            rows.append(dict(offset=offset, erase_end=end, bytes=len(image), staged_file=name,
                             staged_sha256=sha(payload), restore_file=undo,
                             restore_sha256=sha(baseline[offset:end])))
            current[offset:end] = payload
        plan = dict(unit_id=UNIT, factory_sha256=sha(baseline), predecessor_plan_sha256=None,
                    predecessor_chain=[], command_policy=copy.deepcopy(POLICY), writes=rows)
        path = folder/'plan.json'
        self.write_plan(path, plan)
        return baseline, bytes(current), path, plan

    @staticmethod
    def write_plan(path, plan):
        path.write_text(json.dumps(plan))
        return dict(path=str(path.resolve()), sha256=sha(path.read_bytes()))

    def test_smaller_image_leaves_stale_bytes_but_exact_sector_return_restores_everything(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            baseline, current, path, plan = self.candidate(root)
            ordinary = bytearray(current)
            # Even restoring the entire smaller app erase span leaves the
            # second candidate app sector. Boot/partition restoration isn't enough.
            for start, end in [(0x2000, 0x3000), (0x8000, 0x9000), (0x10000, 0x11000)]:
                ordinary[start:end] = baseline[start:end]
            self.assertNotEqual(bytes(ordinary), baseline)
            rollback = exact_return(plan, path, baseline, current, root/'return', [], 'a'*64)
            history = [dict(path=str(path), sha256=sha(path.read_bytes())),
                       dict(path=str(root/'return/plan.json'), sha256=sha((root/'return/plan.json').read_bytes()))]
            restored, verified = replay(baseline, history)
            self.assertEqual(restored, baseline)
            self.assertEqual(len(verified), 2)
            self.assertTrue(all(r['bytes'] == r['erase_end']-r['offset'] for r in rollback['writes']))
            self.assertTrue(all(r['payload_kind'] == 'BASELINE_SECTOR_SNAPSHOT' for r in rollback['writes']))
            self.assertEqual(rollback['status'], 'LAYOUT_ONLY_UNBOUND_NOT_EXECUTABLE')

    def test_saved_predecessor_corruption_is_rejected_even_with_updated_digest(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            baseline, current, path, plan = self.candidate(root)
            row = plan['writes'][0]
            corrupt = b'X'*(row['erase_end']-row['offset'])
            (path.parent/row['restore_file']).write_bytes(corrupt)
            row['restore_sha256'] = sha(corrupt)
            entry = self.write_plan(path, plan)
            with self.assertRaisesRegex(ValueError, 'Historical restore mismatch'):
                replay(baseline, [entry])
            with self.assertRaisesRegex(ValueError, 'saved baseline mismatch'):
                exact_return(plan, path, baseline, current, root/'return', [], 'a'*64)
            self.assertFalse((root/'return/plan.json').exists())

    def test_link_order_policy_and_escaped_payload_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            baseline, _, path, original = self.candidate(root)
            legacy_root = copy.deepcopy(original)
            del legacy_root['predecessor_plan_sha256']
            del legacy_root['predecessor_chain']
            first = self.write_plan(path, legacy_root)
            self.assertEqual(len(replay(baseline, [first])[1]), 1)
            with self.assertRaisesRegex(ValueError, 'ancestry'):
                replay(baseline, [first, first])
            plan = copy.deepcopy(original)
            plan['predecessor_plan_sha256'] = '0'*64
            with self.assertRaisesRegex(ValueError, 'ancestry'):
                replay(baseline, [self.write_plan(path, plan)])
            plan = copy.deepcopy(original)
            plan['command_policy']['force'] = True
            with self.assertRaisesRegex(ValueError, 'command drift'):
                replay(baseline, [self.write_plan(path, plan)])
            plan = copy.deepcopy(original)
            plan['writes'][0]['staged_file'] = '../outside.bin'
            with self.assertRaisesRegex(ValueError, 'escaped root'):
                replay(baseline, [self.write_plan(path, plan)])

    def test_missing_span_and_changed_candidate_full_state_cannot_form_exact_return(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            baseline, current, path, plan = self.candidate(root)
            changed = bytearray(current)
            changed[0] ^= 1  # Outside the allowed writes: cannot silently restore it.
            with self.assertRaisesRegex(ValueError, 'Exact full return failed'):
                exact_return(plan, path, baseline, changed, root/'return', [], 'a'*64)
            del plan['writes'][0]
            with self.assertRaisesRegex(ValueError, 'Incomplete historical spans'):
                replay(baseline, [self.write_plan(path, plan)])

    def test_cli_has_no_execute_mode_or_device_side_effect(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)/'output'
            result = subprocess.run([sys.executable, str(Path(__file__).resolve().parents[1]/'prepare.py'),
                                     '--root', str(root), '--execute'], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('unrecognized arguments', result.stderr)
            self.assertFalse(root.exists())
