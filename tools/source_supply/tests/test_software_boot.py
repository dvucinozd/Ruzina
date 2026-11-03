"""One-shot boot policy/proof fault tests with synthetic bytes and fake ports."""
import copy
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import software_boot as boot
from campaign import MAC, UNIT_ID, sha


def frozen_policy():
    return dict(status='REGISTERED_S07_SOFTWARE_BOOT', schema=1, unit_id=UNIT_ID, mac=MAC,
                chip_revision=103, native_minimum=302, predecessors=43, history_after_return=44,
                app_sha256=boot.S07_APP, elf_sha256=boot.S07_ELF, after_sha256=boot.RESTORED,
                action='ONE_USB_RTS_BOOT_PULSE', manual_requalification=False,
                registration_sha256='1'*64, return_plan_sha256='2'*64, before_sha256='3'*64)


class Ports:
    def __init__(self, *, fail_open=False, fail_close=False, wrong_after_open=False):
        self.opens = self.closes = self.lookups = 0
        self.fail_open, self.fail_close, self.wrong_after_open = fail_open, fail_close, wrong_after_open
    def comports(self):
        self.lookups += 1
        return [SimpleNamespace(device='COM3', vid=0x303a, pid=0x1001,
                                serial_number='wrong' if self.wrong_after_open and self.opens else MAC)]
    def Serial(self, **options):
        owner = self
        class Port:
            dtr = rts = None
            def open(self):
                owner.opens += 1
                if self.dtr is not False or self.rts is not False or options['port'] is not None:
                    raise AssertionError('Control lines must be passive before open')
                if owner.fail_open:
                    raise OSError('Open failed')
            def close(self):
                owner.closes += 1
                if owner.fail_close:
                    raise OSError('Close failed')
            def write(self, data):
                raise AssertionError('No serial data writes')
        return Port()


class SoftwareBootTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='s07-boot-synthetic-')
        self.root = Path(self.temporary.name)
    def tearDown(self):
        self.temporary.cleanup()

    def test_policy_keeps_native_minimum_and_rejects_consumed_return(self):
        path = self.root/'policy.json'
        base = frozen_policy()
        for key, value in [('native_minimum', 103), ('predecessors', 41), ('history_after_return', 42),
                           ('manual_requalification', True), ('action', 'WATCHDOG_RESET'),
                           ('return_plan_sha256', 'cd76f6c0611a9c587f25b7f02352fa81805a961a99e885c415287898c9d1f85d')]:
            path.write_text(json.dumps({**base, key:value}))
            with self.subTest(key=key), self.assertRaises(ValueError):
                boot.policy(path, sha(path.read_bytes()))
        path.write_text(json.dumps(base))
        self.assertEqual(boot.policy(path, sha(path.read_bytes())), base)

    def test_pulse_issued_once_never_claims_running_or_physical_cold(self):
        ports = Ports()
        calls = []
        result = boot.issue(self.root/'boot', dict(synthetic=True), ports, ports.comports,
                            lambda port, **options: calls.append(options))
        self.assertEqual(calls, [dict(hold_delay=.2, post_release_delay=.2, flow_control=False)])
        self.assertEqual((ports.opens, ports.closes), (1, 1))
        self.assertEqual(result['status'], 'BOOT_PULSE_ISSUED_APPLICATION_NOT_OBSERVED')
        self.assertEqual(result['application_running'], 'NOT_OBSERVED')
        self.assertFalse(result['flash_write'] or result['efuse_write'] or result['serial_data_writes'] or
                         result['physical_power_cycle'])

    def test_failed_pulse_stays_unknown_without_retry(self):
        ports = Ports()
        calls = []
        def pulse(port, **options):
            calls.append(options)
            raise OSError('Control-line operation unsupported')
        with self.assertRaises(OSError):
            boot.issue(self.root/'failed', {}, ports, ports.comports, pulse)
        result = json.loads((self.root/'failed/result.json').read_text())
        self.assertEqual(result['status'], 'FAILED_BOOT_STATE_UNKNOWN')
        self.assertTrue(result['reset_attempted'])
        self.assertFalse(result['reset_completed'])
        self.assertEqual(len(calls), 1)
        self.assertEqual(ports.closes, 1)

    def test_partial_open_identity_drift_and_close_failure_are_recorded(self):
        for i, options in enumerate((dict(fail_open=True), dict(wrong_after_open=True), dict(fail_close=True))):
            ports = Ports(**options)
            calls = []
            with self.subTest(options=options), self.assertRaises((OSError, ValueError)):
                boot.issue(self.root/str(i), {}, ports, ports.comports,
                           lambda port, **options: calls.append(options))
            result = json.loads((self.root/str(i)/'result.json').read_text())
            self.assertEqual(result['status'], 'FAILED_BOOT_STATE_UNKNOWN')
            self.assertEqual(ports.closes, 1)
            self.assertEqual(bool(calls), bool(options.get('fail_close')))
            self.assertEqual(result['port_closed'], not options.get('fail_close', False))

    def test_existing_output_and_wrong_usb_identity_never_pulse(self):
        output = self.root/'exists'
        output.mkdir()
        ports = Ports()
        with self.assertRaises(FileExistsError):
            boot.issue(output, {}, ports, ports.comports, lambda *a, **k: self.fail('Pulse'))
        self.assertEqual(ports.lookups, 0)
        with self.assertRaises(ValueError):
            boot.issue(self.root/'missing', {}, ports, lambda: [], lambda *a, **k: self.fail('Pulse'))
        self.assertEqual(ports.opens, 0)

    def test_old_binding_and_old_41_return_proof_are_rejected(self):
        path = self.root/'binding.json'
        path.write_text(json.dumps(dict(status='REVIEWED_EXECUTION_TOOLS', policy_sha256='1'*64)))
        with self.assertRaisesRegex(ValueError, 'Wrong/unreviewed'):
            boot.binding(path, sha(path.read_bytes()), '1'*64)
        path = self.root/'result.json'
        path.write_text(json.dumps(dict(status='WRITTEN_FULL_READBACK_MATCH_ROM_HELD', mode='return-s07', predecessors=41)))
        with self.assertRaisesRegex(ValueError, 'predecessors'):
            boot.return_proof(self.root, sha(path.read_bytes()), frozen_policy())

    def test_full_return_proof_is_checked_without_opening_a_port(self):
        # Synthetic s07-shaped bytes are deliberately substituted only in this
        # test; their hashes never enter a real policy or execution binding.
        app = bytearray(b'\x21'*768176)
        app[0xb0:0xd0] = bytes.fromhex(boot.S07_ELF)
        after = bytearray(b'\xff'*0x1000000)
        after[0x10000:0x10000+len(app)] = app
        before = bytearray(after)
        before[0] = 0
        folder = self.root/'proof'
        folder.mkdir()
        (folder/'prewrite-full.bin').write_bytes(before)
        (folder/'postwrite-full.bin').write_bytes(after)
        for offset, size in ((0x2000, 24576), (0x8000, 4096), (0x10000, 770048)):
            (folder/f'readback-{offset:08x}.bin').write_bytes(after[offset:offset+size])
        with patch.object(boot, 'S07_APP', sha(app)), patch.object(boot, 'RESTORED', sha(after)):
            frozen = frozen_policy()
            frozen['before_sha256'] = sha(before)
            report = dict(status='WRITTEN_FULL_READBACK_MATCH_ROM_HELD', mode='return-s07',
                          predecessors=43, registration_sha256=frozen['registration_sha256'],
                          plan_sha256=frozen['return_plan_sha256'], before_sha256=sha(before),
                          after_sha256=sha(after), postwrite_sha256=sha(after),
                          app_sha256=sha(app), elf_sha256=boot.S07_ELF, full_predecessor_match=True,
                          reset_requested=False, efuse_write=False,
                          identity=dict(chip='ESP32-P4', revision=103, mac=MAC, rdid=0x184068,
                                        security=dict(flags=0, flash_crypt_cnt=0)),
                          captures={path.name:dict(bytes=path.stat().st_size, sha256=sha(path.read_bytes()))
                                    for path in folder.glob('*.bin')})
            path = folder/'result.json'
            path.write_text(json.dumps(report))
            proof = boot.return_proof(folder, sha(path.read_bytes()), frozen)
            self.assertTrue(proof['full_and_span_readback_verified'])
            # A self-consistent span hash still cannot override whole-flash bytes.
            damaged = folder/'readback-00010000.bin'
            damaged.write_bytes(b'X'+damaged.read_bytes()[1:])
            changed = copy.deepcopy(report)
            changed['captures'][damaged.name]['sha256'] = sha(damaged.read_bytes())
            path.write_text(json.dumps(changed))
            with self.assertRaisesRegex(ValueError, 'span/full mismatch'):
                boot.return_proof(folder, sha(path.read_bytes()), frozen)


if __name__ == '__main__':
    unittest.main()
