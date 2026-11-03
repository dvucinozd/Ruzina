import json
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest

TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))
import campaign
from campaign import Prepared, program, check_port, contained, checked, FLASH_BYTES
from monitor import window


class Fake:
    def __init__(self, before):
        self.flash = bytearray(before)
        self.info = dict(chip='ESP32-P4', revision=103, mac=campaign.MAC,
                         security=dict(flags=0, flash_crypt_cnt=0), rdid=0x184068)
        self.closed = self.written = False
        self.bad_readback = self.corrupt_outside = self.fail_connect = False

    def connect(self):
        if self.fail_connect:
            raise OSError('connect failed')
        return self.info

    def read(self, offset, size):
        data = bytes(self.flash[offset:offset+size])
        if self.written and self.bad_readback:
            return b'X' + data[1:]
        return data

    def write(self, writes):
        self.written = True
        for offset, data in writes:
            self.flash[offset:offset+len(data)] = data
        if self.corrupt_outside:
            self.flash[-1] = 1

    def close(self):
        self.closed = True


class CampaignTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.before = bytes(FLASH_BYTES)
        after = bytearray(self.before)
        after[0x10000:0x11000] = b'A' * 4096
        self.prepared = Prepared('candidate', 'reg', 'plan', 'app', 'elf', 36,
                                 self.before, bytes(after), ((0x10000, b'A' * 4096),))

    def tearDown(self):
        self.temp.cleanup()

    def run_failure(self, backend):
        out = self.root / 'capture'
        with self.assertRaises((ValueError, OSError)):
            program(self.prepared, backend, out)
        result = json.loads((out / 'result.json').read_text())
        self.assertEqual(result['status'], 'FAILED_STOP_AND_INSPECT')
        self.assertTrue(backend.closed)
        return result

    def test_each_identity_security_gate_prevents_writes(self):
        bad = [('chip', 'OTHER'), ('revision', 302), ('mac', '00:00:00:00:00:00'),
               ('rdid', 0), ('security', dict(flags=1, flash_crypt_cnt=0)),
               ('security', dict(flags=0, flash_crypt_cnt=1))]
        for key, value in bad:
            with self.subTest(key=key, value=value):
                backend = Fake(self.before)
                backend.info[key] = value
                with tempfile.TemporaryDirectory(dir=self.root) as td:
                    with self.assertRaises(ValueError):
                        program(self.prepared, backend, Path(td) / 'out')
                    self.assertFalse(backend.written)
                    self.assertTrue(backend.closed)

    def test_full_predecessor_mismatch_never_writes(self):
        backend = Fake(self.before)
        backend.flash[-1] = 1
        result = self.run_failure(backend)
        self.assertFalse(backend.written)
        self.assertFalse(result['flash_write_attempted'])

    def test_partial_connection_is_closed_and_recorded(self):
        backend = Fake(self.before)
        backend.fail_connect = True
        self.run_failure(backend)
        self.assertFalse(backend.written)

    def test_bad_span_readback_is_not_success(self):
        backend = Fake(self.before)
        backend.bad_readback = True
        result = self.run_failure(backend)
        self.assertTrue(result['flash_write_attempted'])

    def test_full_postwrite_detects_corruption_outside_written_spans(self):
        backend = Fake(self.before)
        backend.corrupt_outside = True
        result = self.run_failure(backend)
        self.assertIn('Postwrite complete flash differs', result['error'])

    def test_success_saves_full_before_and_after_and_closes(self):
        backend = Fake(self.before)
        result = program(self.prepared, backend, self.root / 'capture')
        self.assertEqual(result['status'], 'WRITTEN_FULL_READBACK_MATCH_ROM_HELD')
        self.assertTrue(backend.closed)
        self.assertEqual((self.root / 'capture/postwrite-full.bin').read_bytes(), self.prepared.after)
        self.assertFalse(result['reset_requested'])
        self.assertFalse(result['efuse_write'])

    def test_output_reuse_never_connects(self):
        out = self.root / 'capture'
        out.mkdir()
        backend = Fake(self.before)
        with self.assertRaises(FileExistsError):
            program(self.prepared, backend, out)
        self.assertFalse(backend.closed)
        self.assertFalse(backend.written)

    def test_hash_and_path_drift_rejected(self):
        path = self.root / 'x'
        path.write_bytes(b'original')
        digest = campaign.sha(b'original')
        self.assertEqual(checked(path, digest), b'original')
        path.write_bytes(b'tampered')
        with self.assertRaises(ValueError):
            checked(path, digest)
        for escape in ('../outside', str(self.root.parent / 'outside')):
            with self.assertRaises(ValueError):
                contained(self.root, escape)

    def test_usb_port_identity_must_match(self):
        good = SimpleNamespace(device='COM3', vid=0x303a, pid=0x1001, serial_number=campaign.MAC.upper())
        check_port('COM3', [good])
        for port, entries in [('COM4', [good]), ('COM3', []), ('COM3', [good, good]),
                              ('COM3', [SimpleNamespace(device='COM3', vid=0, pid=0, serial_number='')])]:
            with self.assertRaises(ValueError):
                check_port(port, entries)

    def test_boot_window_excludes_earlier_image_but_rejects_second_boot(self):
        elf = 'a' * 64
        prior = b'ELF file SHA256: bbbbbbbb\nold telemetry\n'
        boot = b'ELF file SHA256: aaaaaaaa\nnew telemetry\n'
        offset, data = window(prior + boot, elf)
        self.assertEqual(offset, len(prior))
        self.assertEqual(data, boot)
        for raw in [prior, boot + boot, boot + prior]:
            with self.assertRaises(ValueError):
                window(raw, elf)

    def test_checks_survive_python_optimization(self):
        # Safety checks must remain active under Python -O (unlike assert).
        code = "from campaign import require; require(False, 'still guarded')"
        result = subprocess.run([sys.executable, '-O', '-c', code], cwd=TOOLS, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('still guarded', result.stderr)

    def test_esptool_policy_uses_no_reset_and_exact_keep_options(self):
        from unittest.mock import Mock
        backend = campaign.EspBackend.__new__(campaign.EspBackend)
        backend.port = 'COM3'
        backend.esp = None
        esp = Mock()
        esp.CHIP_NAME = 'ESP32-P4'
        esp.get_chip_revision.return_value = 103
        esp.read_mac.return_value = bytes.fromhex(campaign.MAC.replace(':', ''))
        esp.get_security_info.return_value = dict(flags=0, flash_crypt_cnt=0)
        esp.run_stub.return_value = esp
        backend.esptool = Mock()
        backend.esptool.detect_chip.return_value = esp
        backend.cmds = Mock()
        backend.connect()
        backend.esptool.detect_chip.assert_called_once_with('COM3', connect_mode='no-reset', connect_attempts=2)
        backend.write(self.prepared.writes)
        backend.cmds.write_flash.assert_called_once_with(esp, list(self.prepared.writes), flash_freq='keep',
            flash_mode='keep', flash_size='keep', erase_all=False, force=False, no_progress=True)

    def test_passive_monitor_never_writes_and_sets_lines_before_open(self):
        from unittest.mock import Mock, patch
        import monitor
        expected = SimpleNamespace(elf_sha='a' * 64, app_sha='app', registration_sha='reg')
        serial_port = Mock()
        serial_port.read.return_value = b'ELF file SHA256: aaaaaaaa\n'
        lines = []
        serial_port.open.side_effect = lambda: lines.append((serial_port.dtr, serial_port.rts))
        serial_module = SimpleNamespace(Serial=Mock(return_value=serial_port), SerialException=OSError)
        ports = SimpleNamespace(comports=lambda: [SimpleNamespace(device='COM3', vid=0x303a,
            pid=0x1001, serial_number=campaign.MAC)])
        argv = ['monitor.py', '--root', str(self.root), '--mode', 'candidate', '--execute',
                '--output', str(self.root/'monitor'), '--binding', 'binding.json', '--binding-sha256', 'hash']
        with patch.object(sys, 'argv', argv), patch.object(monitor, 'prepare', return_value=expected), \
             patch.object(monitor, 'binding', return_value={'registration_sha256':'reg'}), \
             patch.dict(sys.modules, {'serial': serial_module, 'serial.tools':SimpleNamespace(list_ports=ports)}), \
             patch.object(monitor.time, 'monotonic', side_effect=[0, 1, 2, 47, 48]), \
             patch('builtins.print'):
            monitor.main()
        self.assertEqual(lines, [(False, False)])
        serial_port.write.assert_not_called()
        serial_port.close.assert_called_once()
        result = json.loads((self.root/'monitor/result.json').read_text())
        self.assertFalse(result['serial_writes'])
        self.assertEqual(result['boot_window_offset'], 0)


if __name__ == '__main__':
    unittest.main()
