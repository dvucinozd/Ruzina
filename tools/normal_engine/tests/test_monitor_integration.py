"""Exercise the real capture loop with synthetic ports and a deterministic clock."""
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import monitor

ELF = 'a' * 64


class SerialGone(Exception):
    pass


class FakeClock:
    def __init__(self):
        self.now = 0.0

    def monotonic(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds


class SyntheticPort:
    def __init__(self, owner):
        self.owner = owner
        self.port, self.dtr, self.rts = None, None, None
        self.closed = False

    def open(self):
        owner = self.owner
        owner.opens += 1
        if self.dtr is not False or self.rts is not False:
            raise AssertionError('Control lines must be passive before open')
        if owner.busy:
            raise SerialGone('exclusive access')
        if owner.race and owner.opens == 1:
            owner.visible_at = owner.clock.now + 2
            raise SerialGone('disappeared during open')
        owner.opened_at = owner.clock.now

    def read(self, size):
        owner = self.owner
        owner.clock.sleep(1)
        if owner.disconnect_after is not None and owner.clock.now - owner.opened_at >= owner.disconnect_after:
            raise SerialGone('connection lost')
        if owner.kind == 'cold' and not owner.boot_sent:
            owner.boot_sent = True
            return ('ELF file SHA256: ' + ELF + '\n').encode()
        return b'synthetic telemetry; never device evidence\n'

    def close(self):
        self.closed = True

    def write(self, data):
        raise AssertionError('Passive monitor must never write')


class SyntheticSerial:
    def __init__(self, kind, clock, *, visible_at=0, race=False, busy=False, disconnect_after=None):
        self.kind, self.clock = kind, clock
        self.visible_at, self.race, self.busy = visible_at, race, busy
        self.disconnect_after = disconnect_after
        self.opens, self.opened_at, self.boot_sent = 0, None, False
        self.ports = []

    def comports(self):
        if self.clock.now < self.visible_at:
            return []
        return [SimpleNamespace(device='COM3', vid=0x303a, pid=0x1001,
                                serial_number='e8:f6:0a:e7:22:1d')]

    def Serial(self, **kwargs):
        port = SyntheticPort(self)
        self.ports.append(port)
        return port


class MonitorIntegrationTests(unittest.TestCase):
    def capture(self, kind, **scenario):
        clock = FakeClock()
        serial = SyntheticSerial(kind, clock, **scenario)
        seconds, after = (600, 45) if kind == 'cold' else (60, 0)
        prepared = SimpleNamespace(registration_sha='synthetic', elf_sha=ELF, app_sha='b' * 64)
        modules = {'serial': SimpleNamespace(Serial=serial.Serial, SerialException=SerialGone),
                   'serial.tools': SimpleNamespace(list_ports=serial),
                   'serial.tools.list_ports': serial}
        with tempfile.TemporaryDirectory(prefix='normal-monitor-synthetic-') as temporary:
            output = Path(temporary) / 'capture'
            argv = ['monitor', '--root', temporary, '--mode', 'candidate', '--execute',
                    '--output', str(output), '--binding', str(Path(temporary) / 'unused.json'),
                    '--binding-sha256', 'synthetic', '--window', kind,
                    '--seconds', str(seconds), '--after-boot', str(after)]
            error = None
            with patch.dict(sys.modules, modules), patch.object(sys, 'argv', argv), \
                    patch.object(monitor, 'time', clock), patch.object(monitor, 'prepare', return_value=prepared), \
                    patch.object(monitor, 'binding', return_value={'registration_sha256': 'synthetic'}), \
                    patch('builtins.print'):
                try:
                    monitor.main()
                except SerialGone as caught:
                    error = caught
            result = json.loads((output / 'result.json').read_text())
            raw = (output / 'run.log').read_bytes()
        self.assertTrue(all(port.closed for port in serial.ports))
        self.assertFalse(result['serial_writes'])
        return result, serial, error, raw

    def test_late_idle_port_gets_full_connected_minute(self):
        result, serial, error, _ = self.capture('idle', visible_at=5)
        self.assertIsNone(error)
        self.assertEqual(result['status'], 'CAPTURED_NOT_QUALIFIED')
        self.assertGreaterEqual(result['connected_capture_seconds'], 60)
        self.assertGreaterEqual(result['elapsed_seconds'], 65)
        self.assertEqual(serial.opens, 1)

    def test_post_boot_disconnect_cannot_become_complete(self):
        result, serial, error, raw = self.capture('cold', disconnect_after=20)
        self.assertIsNone(error)
        self.assertEqual(result['status'], 'CAPTURE_INCOMPLETE')
        self.assertFalse(result['capture_complete'])
        self.assertTrue(result['connection_interrupted'])
        self.assertLess(result['post_boot_seconds'], 45)
        self.assertEqual(serial.opens, 1)
        self.assertIn(ELF.encode(), raw)

    def test_disappearing_pre_boot_port_retries_then_captures(self):
        result, serial, error, _ = self.capture('cold', race=True)
        self.assertIsNone(error)
        self.assertEqual(result['status'], 'CAPTURED_NOT_QUALIFIED')
        self.assertGreaterEqual(result['post_boot_seconds'], 45)
        self.assertEqual(serial.opens, 2)
        self.assertFalse(result['connection_interrupted'])

    def test_listed_busy_port_fails_without_waiting(self):
        result, serial, error, _ = self.capture('cold', busy=True)
        self.assertIsNotNone(error)
        self.assertEqual(result['status'], 'FAILED_CAPTURE')
        self.assertEqual(serial.opens, 1)
        self.assertLess(result['elapsed_seconds'], 1)


if __name__ == '__main__':
    unittest.main()
