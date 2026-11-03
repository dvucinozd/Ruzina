"""New writer preflight rejects drift before the unchanged destructive guard."""
import copy
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import continuous_write as writer
from campaign import SOURCE, UNIT_ID, sha
from register_continuous import register


class WritePreflightTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='continuous-write-synthetic-')
        self.root = Path(self.temporary.name)
        self.repo = self.root/'repo'
        self.repo.mkdir()
        (self.repo/'runbook.md').write_text('Synthetic only')
        (self.repo/'policy.md').write_text('Synthetic only')
        path = Path(__file__).resolve().parents[3]/'firmware-idf/legacy-jc1060/tools/prepare_flash_plan.py'
        spec = importlib.util.spec_from_file_location('fixture_planner', path)
        self.planner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.planner)
        self.original = bytes(b'\xff'*0x120000)
        self.state = bytearray(self.original)
        self.history = []
        for index in range(42):
            folder = self.root/f'ancestor-{index}'
            folder.mkdir()
            self.make_plan(folder, {0x10000:bytes([index])}, self.history)
        self.baseline = bytes(self.state)
        self.app = bytearray(b'\x12'*400)
        self.elf = sha(b'synthetic ELF')
        self.app[0xb0:0xd0] = bytes.fromhex(self.elf)
        images = {0x2000:b'boot', 0x8000:b'table', 0x10000:bytes(self.app)}
        package = self.root/'package'
        package.mkdir()
        artifacts = {'build/ruzina_legacy_probe.bin':bytes(self.app), 'build/ruzina_legacy_probe.elf':b'synthetic ELF',
                     'build/bootloader.bin':images[0x2000], 'build/partition.bin':images[0x8000],
                     'build/flasher_args.json':json.dumps(dict(flash_files={
                         '0x2000':'bootloader.bin','0x8000':'partition.bin','0x10000':'ruzina_legacy_probe.bin'})).encode()}
        for name, raw in artifacts.items():
            target = package/name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(raw)
        manifest = dict(source_commit=SOURCE, dirty_status='', variant='deck-engine', producer_profile=True,
                        uac_profile=False, uac_pack_iram=False, sources={}, sha256=sha(self.app),
                        artifacts={name:sha(raw) for name, raw in artifacts.items()})
        (package/'manifest.json').write_text(json.dumps(manifest))
        self.campaign = self.root/'campaign'
        self.campaign.mkdir()
        candidate = self.campaign/'candidate-plan'
        candidate.mkdir()
        self.make_plan(candidate, images, self.history[:])
        self.after = bytes(self.state)
        plan = json.loads((candidate/'plan.json').read_text())
        plan['manifest_sha256'] = sha((package/'manifest.json').read_bytes())
        plan['command_policy'] = writer.POLICY.copy()
        (candidate/'plan.json').write_text(json.dumps(plan))
        self.registration = dict(status='REGISTERED_CONTINUOUS_WRITE_RECOVERY', unit_id=UNIT_ID,
            source_commit=SOURCE, native_minimum=302, executed_predecessors_at_registration=42,
            current_full_sha256=sha(self.baseline), executed_history=self.history[:42],
            runbook=dict(path='runbook.md',sha256=sha((self.repo/'runbook.md').read_bytes())),
            recovery_policy=dict(path='policy.md',sha256=sha((self.repo/'policy.md').read_bytes())),
            transitions={'candidate':dict(plan_dir='candidate-plan',package=str(package),
                plan_sha256=sha((candidate/'plan.json').read_bytes()),manifest_sha256=sha((package/'manifest.json').read_bytes()),
                app_sha256=sha(self.app),elf_sha256=self.elf,predecessors=42,
                expected_before_sha256=sha(self.baseline),expected_after_sha256=sha(self.after))})
        self.save_registration()
        self.patches = [patch.object(writer,'planner',return_value=self.planner),
                        patch.object(writer,'factory',return_value=self.original),
                        patch.object(writer,'APP',sha(self.app)), patch.object(writer,'ELF',self.elf),
                        patch.object(writer,'BEFORE',sha(self.baseline))]
        for item in self.patches:item.start()

    def tearDown(self):
        for item in reversed(self.patches):item.stop()
        self.temporary.cleanup()

    def make_plan(self, folder, images, history):
        rows = []
        for offset, image in images.items():
            end = (offset+len(image)+4095)//4096*4096
            restore = bytes(self.state[offset:end])
            payload = image+restore[len(image):]
            write, old = f'write-{offset:08x}.bin', f'restore-{offset:08x}.bin'
            (folder/write).write_bytes(payload)
            (folder/old).write_bytes(restore)
            rows.append(dict(offset=offset,bytes=len(image),erase_end=end,staged_file=write,
                             staged_sha256=sha(payload),restore_file=old,restore_sha256=sha(restore)))
            self.state[offset:end] = payload
        plan = dict(factory_sha256=self.planner.BACKUP_SHA,unit_id=UNIT_ID,predecessor_chain=copy.deepcopy(history),
                    predecessor_plan_sha256=history[-1]['sha256'] if history else None,writes=rows)
        path = folder/'plan.json'
        path.write_text(json.dumps(plan))
        history.append(dict(path=str(path.resolve()),sha256=sha(path.read_bytes())))

    def save_registration(self):
        self.path = self.campaign/'campaign-registration.json'
        self.path.write_text(json.dumps(self.registration))
        self.digest = sha(self.path.read_bytes())

    def prepare(self):
        return writer.prepare(self.campaign,self.repo,'candidate',self.digest)

    def rehash_plan(self):
        self.registration['transitions']['candidate']['plan_sha256'] = sha((self.campaign/'candidate-plan/plan.json').read_bytes())
        self.save_registration()

    def test_exact_42_preflight_and_immutable_payloads(self):
        result = self.prepare()
        self.assertEqual(result.predecessors,42)
        self.assertEqual(result.before,self.baseline)
        self.assertEqual(result.after,self.after)
        self.assertEqual(len(result.writes),3)
        self.assertIs(writer.program,sys.modules['campaign'].program)
        self.assertFalse('esptool' in sys.modules or 'serial' in sys.modules)

    def test_old_count_native_minimum_and_command_drift_fail(self):
        original = copy.deepcopy(self.registration)
        for key,value in [('executed_predecessors_at_registration',40),('native_minimum',103)]:
            self.registration = {**original,key:value}
            self.save_registration()
            with self.subTest(key=key),self.assertRaisesRegex(ValueError,'Wrong write registration'):
                self.prepare()
        self.registration = original
        path = self.campaign/'candidate-plan/plan.json'
        plan = json.loads(path.read_text())
        plan['command_policy']['after'] = 'hard-reset'
        path.write_text(json.dumps(plan))
        self.rehash_plan()
        with self.assertRaisesRegex(ValueError,'command drift'):self.prepare()

    def test_rehashed_wrong_image_and_sector_tail_are_rejected(self):
        path = self.campaign/'candidate-plan/plan.json'
        plan = json.loads(path.read_text())
        row = plan['writes'][-1]
        payload = self.campaign/'candidate-plan'/row['staged_file']
        original = payload.read_bytes()
        for position in (0,len(original)-1):
            raw = bytearray(original)
            raw[position] ^= 1
            payload.write_bytes(raw)
            row['staged_sha256'] = sha(raw)
            path.write_text(json.dumps(plan))
            self.rehash_plan()
            with self.subTest(position=position),self.assertRaisesRegex(ValueError,'Payload/restore/tail'):
                self.prepare()

    def test_historical_restore_damage_is_not_hidden_by_unchanged_full_state(self):
        path = Path(self.history[20]['path'])
        plan = json.loads(path.read_text())
        restore = path.parent/plan['writes'][0]['restore_file']
        restore.write_bytes(b'X'+restore.read_bytes()[1:])
        with self.assertRaisesRegex(ValueError,'Hash mismatch'):self.prepare()

    def test_truncated_and_reordered_executed_prefix_fail(self):
        original = copy.deepcopy(self.registration)
        for history in (original['executed_history'][:-1],list(reversed(original['executed_history']))):
            self.registration = {**original,'executed_history':history}
            self.save_registration()
            with self.assertRaisesRegex(ValueError,'Executed prefix drift'):self.prepare()

    def test_new_output_required_before_registration_reads_prior_data(self):
        with self.assertRaisesRegex(ValueError,'Fresh registration'):
            register(self.root,self.repo,self.root/'nonexistent')

    def test_old_binding_cannot_authorize_new_writer(self):
        path = self.root/'binding.json'
        path.write_text(json.dumps(dict(status='REVIEWED_EXECUTION_TOOLS',registration_sha256=self.digest,tools={})))
        with self.assertRaisesRegex(ValueError,'Wrong write binding'):
            writer.binding(path,sha(path.read_bytes()),self.digest)


if __name__ == '__main__':
    unittest.main()
