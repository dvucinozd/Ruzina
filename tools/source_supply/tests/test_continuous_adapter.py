"""Deterministic whole-session adapter/analyzer tests. No serial/device imports."""
import copy
from concurrent.futures import Future
import json
from pathlib import Path
import sys
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from campaign import MAC, SOURCE, UNIT_ID, sha
from continuous import ContinuousCapture
from continuous_adapter import Controller, PassiveReader, execute
from continuous_analyze import analyze, numerical, replay
from continuous_contract import APP, BEFORE, CASES, ELF, LIMITS, validate, verify_binding, verify_program
from test_continuous import report

BOOT = (f'ELF file SHA256:  {ELF[:9]}...\nchip_revision=103\nreset_reason=1\n'
        'resource_prepare state=PREPARING\nresource_prepare state=READY\nusb_uac stream=STARTED\n').encode()


def contract():
    return dict(schema=1, status='REGISTERED_CONTINUOUS_SOURCE_CAMPAIGN', unit_id=UNIT_ID,
                mac=MAC, chip_revision=103, native_minimum=302, firmware_source=SOURCE,
                app_sha256=APP, elf_sha256=ELF, candidate_predecessors=42, return_predecessors=43,
                before_sha256=BEFORE, cases=copy.deepcopy(CASES), resources=LIMITS.copy(),
                write_registration_sha256='1'*64, candidate_plan_sha256='2'*64,
                return_plan_sha256='3'*64, after_sha256='4'*64)


def sample(i, playing=(0, 0)):
    lines = [line for line in report(i, playing).decode().splitlines() if not line.startswith('heartbeat=')]
    lines.insert(0, f'heartbeat={i} uptime_us={i*6_000_000}')
    lines.insert(-1, 'rust_load event=13 stack_free=50000')
    return ('\n'.join(lines)+'\n').encode()


def feed(capture, raw, now):
    for i in range(0, len(raw), 4096):
        capture.feed(raw[i:i+4096], now)


class Gone(Exception):
    pass


class Clock:
    now = 0.0
    def monotonic(self):
        return self.now
    def sleep(self, seconds):
        self.now += seconds


class ImmediateExecutor:
    """Deterministic executor for synthetic clock integration only."""
    def __init__(self, **kwargs):
        pass
    def submit(self, function):
        future = Future()
        try:future.set_result(function())
        except BaseException as error:future.set_exception(error)
        return future
    def shutdown(self, **kwargs):
        pass


class Serial:
    SerialException = Gone
    def __init__(self, clock, provider=None, *, busy=False, race=False, disconnect_at=None):
        self.clock, self.provider = clock, provider or (lambda: b'')
        self.busy, self.race, self.disconnect_at = busy, race, disconnect_at
        self.opens, self.closes, self.visible_at = 0, 0, 0
    def comports(self):
        return [] if self.clock.now < self.visible_at else [SimpleNamespace(
            device='COM3', vid=0x303a, pid=0x1001, serial_number=MAC)]
    def Serial(self, **options):
        owner = self
        class Port:
            dtr = rts = None
            def open(self):
                if self.dtr is not False or self.rts is not False or options['timeout'] != .1:
                    raise AssertionError('Active control lines/unbounded reads')
                owner.opens += 1
                if owner.busy:
                    raise Gone('Port busy')
                if owner.race and owner.opens == 1:
                    owner.visible_at = owner.clock.now+2
                    raise Gone('Enumeration race')
            def read(self, count):
                owner.clock.sleep(1)
                if owner.disconnect_at and owner.clock.now >= owner.disconnect_at:
                    raise Gone('Post-boot disconnect')
                return owner.provider()
            def close(self):
                owner.closes += 1
            def write(self, data):
                raise AssertionError('Serial write forbidden')
        return Port()


class AdapterTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='continuous-adapter-synthetic-')
        self.root = Path(self.temp.name)
    def tearDown(self):
        self.temp.cleanup()

    def cold(self, name='cold'):
        capture = ContinuousCapture(self.root/name, LIMITS, ELF, auto_cold=True)
        feed(capture, BOOT, 0)
        for i in range(1, 8):
            feed(capture, sample(i), i*6)
        capture.confirm_physical_cold('Synthetic physical-cold confirmation, not board evidence', 43)
        capture.tick(45)
        return capture

    def test_cold_checkpoint_replays_exactly_and_reader_can_continue(self):
        capture = self.cold()
        folder = capture.snapshot('cold-1', 45)
        summary = analyze(folder, contract())
        self.assertTrue(summary['continuation_pass'])
        self.assertEqual(summary['completed_cases'], 1)
        self.assertFalse(capture.closed)
        token = capture.offer_case('idle-1', 'idle', 60, 45)
        capture.acknowledge_ready(token, 'Synthetic idle start', 45)
        capture.begin_case(token, 45)
        for i in range(8, 18):
            feed(capture, sample(i), i*6)
        capture.tick(105)
        folder2 = capture.snapshot('idle-1', 105)
        self.assertEqual(analyze(folder2, contract())['completed_cases'], 2)
        capture.close(105)
        self.assertEqual(analyze(capture.output, contract())['completed_cases'], 2)

    def test_rehashed_frame_event_and_case_edits_are_rejected_by_replay(self):
        capture = self.cold()
        folder = capture.snapshot('cold-1', 45)
        capture.close(45)
        result = json.loads((folder/'result.json').read_text())
        original = (folder/'result.json').read_bytes()
        for key in ('frames', 'cases'):
            changed = copy.deepcopy(result)
            if key == 'frames':
                changed[key][0]['end'] += 1
            else:
                changed[key][0]['start_offset'] += 1
            (folder/'result.json').write_text(json.dumps(changed))
            with self.assertRaisesRegex(ValueError, 'metadata drift'):
                replay(folder, contract())
        (folder/'result.json').write_bytes(original)
        data = (folder/'cold-1.log').read_bytes()
        (folder/'cold-1.log').write_bytes(data+b'edited')
        with self.assertRaisesRegex(ValueError, 'exact whole-stream slice'):
            replay(folder, contract())

    def test_missing_confirmation_boot_readiness_or_wrong_reset_fail(self):
        capture = self.cold()
        capture.cold_confirmation = None
        folder = capture.snapshot('cold-1', 45)
        capture.close(45)
        with self.assertRaises(ValueError):
            analyze(folder, contract())
        for index, boot in enumerate((BOOT.replace(b'reset_reason=1', b'reset_reason=3'),
                                      BOOT.replace(b'state=READY', b'state=FAILED'))):
            capture = ContinuousCapture(self.root/f'bad-{index}', LIMITS, ELF, auto_cold=True)
            feed(capture, boot, 0)
            for i in range(1, 8):
                feed(capture, sample(i), i*6)
            capture.confirm_physical_cold('Synthetic', 43)
            capture.tick(45)
            folder = capture.snapshot('cold-1', 45)
            capture.close(45)
            with self.assertRaises(ValueError):
                analyze(folder, contract())

    def test_receive_backlog_cannot_masquerade_as_fresh_source_time(self):
        capture = ContinuousCapture(self.root/'buffered', LIMITS, ELF, auto_cold=True)
        feed(capture, BOOT, 0)
        for i in range(1, 8):
            feed(capture, sample(i), 42)  # All six-second device samples arrived in one host instant.
        capture.confirm_physical_cold('Synthetic', 43)
        capture.tick(45)
        folder = capture.snapshot('cold-1', 45)
        capture.close(45)
        with self.assertRaisesRegex(ValueError, 'Buffered/delayed'):
            analyze(folder, contract())

    def test_silent_case_tail_and_short_source_interval_cannot_pass(self):
        capture = self.cold()
        token = capture.offer_case('idle-1', 'idle', 60, 45)
        capture.acknowledge_ready(token, 'Synthetic', 45)
        capture.begin_case(token, 45)
        for i in range(8, 13):
            feed(capture, sample(i), i*6)
        capture.tick(105)
        folder = capture.snapshot('idle-1', 105)
        capture.close(105)
        with self.assertRaisesRegex(ValueError, 'coverage at case edges'):
            analyze(folder, contract())
        short = b''.join(sample(i, (1, 0)) for i in range(1, 4)).decode()
        with self.assertRaisesRegex(ValueError, '20 measured source seconds'):
            numerical(short, CASES[2])

    def test_disjoint_deck_play_intervals_cannot_pass_both_playing(self):
        staggered = b''.join(sample(i, (int(i <= 5), int(i == 1 or i >= 6))) for i in range(1, 11)).decode()
        with self.assertRaisesRegex(ValueError, '20 overlapping both-playing'):
            numerical(staggered, CASES[4])

    def test_rust_load_fault_and_gap_outside_case_are_never_hidden(self):
        capture = self.cold()
        capture.snapshot('cold-1', 45)
        feed(capture, b'rust_load event=6 stack_free=50000\n', 46)
        result = capture.close(46)
        self.assertIn('Source/media failure', result['error'])
        with self.assertRaisesRegex(ValueError, 'Invalid session'):
            analyze(capture.output, contract())

    def test_passive_open_busy_race_and_post_boot_no_reopen(self):
        for index, options in enumerate((dict(busy=True), dict(race=True), dict(disconnect_at=2))):
            clock = Clock()
            capture = ContinuousCapture(self.root/f'port-{index}', LIMITS, ELF)
            serial = Serial(clock, lambda: BOOT, **options)
            reader = PassiveReader(capture, serial, serial.comports, clock.monotonic, clock.sleep)
            try:
                if options.get('busy'):
                    with self.assertRaises(Gone):reader.read(4096)
                    self.assertEqual(serial.opens, 1)
                elif options.get('race'):
                    for _ in range(30):
                        if reader.read(4096):break
                    self.assertEqual(serial.opens, 2)
                else:
                    feed(capture, reader.read(4096), clock.now)
                    with self.assertRaises(Gone):reader.read(4096)
                    self.assertEqual(serial.opens, 1)
            finally:
                reader.close()
                capture.close(capture.last)

    def test_old_contract_binding_and_program_proof_rejected_offline(self):
        bad = contract()
        for key, value in [('native_minimum', 103), ('candidate_predecessors', 40),
                           ('return_predecessors', 41), ('status', 'OFFLINE_REGISTERED_OPERATOR_GATES_OPEN_NOT_FLASHED')]:
            with self.subTest(key=key), self.assertRaises(ValueError):
                validate({**bad, key:value})
        binding = self.root/'binding.json'
        binding.write_text(json.dumps(dict(status='REVIEWED_EXECUTION_TOOLS', contract_sha256='1'*64)))
        with self.assertRaisesRegex(ValueError, 'does not authorize'):
            verify_binding(binding, sha(binding.read_bytes()), '1'*64)
        (self.root/'result.json').write_text(json.dumps(dict(status='WRITTEN_FULL_READBACK_MATCH_ROM_HELD', mode='candidate',
            registration_sha256='1'*64, plan_sha256='2'*64, predecessors=40)))
        with self.assertRaisesRegex(ValueError, 'predecessors'):
            verify_program(self.root, sha((self.root/'result.json').read_bytes()), contract())

    def test_whole_five_case_controller_campaign_uses_one_passive_port(self):
        clock = Clock()
        pending = []
        output = self.root/'full'
        def provider():
            now = int(clock.now)
            if now == 1:
                return BOOT
            if not pending and now % 6 == 0:
                status = json.loads((output/'status.json').read_text())
                active = status['active_case']
                playing = next((tuple(spec['playing']) for spec in CASES
                                if active and spec['name'] == active['name'] and active['state'] == 'CAPTURING'), (0, 0))
                raw = sample(now//6, playing)
                pending.extend(raw[i:i+4096] for i in range(0, len(raw), 4096))
            # Commands are atomically published between successive polls.
            status = json.loads((output/'status.json').read_text())
            case = status['active_case']
            command = None
            if now == 43:
                command = dict(command='confirm-cold', reply='Synthetic owner confirms physical cold')
            elif case and case['kind'] in ('functional', 'both'):
                if case['state'] == 'OFFERED':
                    command = dict(command='ready', token=case['token'], reply='Synthetic owner ready')
                elif case['state'] == 'READY':
                    command = dict(command='start', token=case['token'])
            if command:
                name = status['next_command']
                path = output/'commands'/name
                if not path.exists():
                    path.write_text(json.dumps(dict(sequence=int(name[:6]), **command)))
            return pending.pop(0) if pending else b''
        serial = Serial(clock, provider)
        with patch('continuous_adapter.ThreadPoolExecutor', ImmediateExecutor):
            result = execute(output, contract(), dict(synthetic=True), serial, serial.comports,
                             clock.monotonic, clock.sleep)
        self.assertEqual(result['status'], 'MEASURED_NOT_PRODUCT_QUALIFIED', result)
        self.assertEqual(result['admitted_cases'], 5)
        self.assertEqual(serial.opens, 1)
        self.assertEqual(serial.closes, 1)
        self.assertEqual(analyze(output, contract())['completed_cases'], 5)

    def test_bad_local_token_or_changed_accepted_reply_stops_controller(self):
        capture = self.cold()
        controller = Controller(capture, contract())
        path = controller.commands/'000001.json'
        path.write_text(json.dumps(dict(sequence=1, command='confirm-cold', reply='duplicate')))
        with self.assertRaisesRegex(ValueError, 'Missing/repeated'):
            controller(capture, 45)
        path.unlink()
        # Accepted bytes are audited again before any subsequent command.
        path.write_text('original')
        (controller.accepted/path.name).write_text('changed')
        with self.assertRaisesRegex(ValueError, 'Accepted control file changed'):
            controller(capture, 45)
        capture.close(45)
        controller.close()

    def test_async_admission_keeps_receiving_and_snapshot_is_a_frozen_prefix(self):
        capture = self.cold()
        controller = Controller(capture, contract())
        entered, release = threading.Event(), threading.Event()
        real_analyze = analyze
        def slow_analyze(folder, policy):
            entered.set()
            if not release.wait(5):raise RuntimeError('Test worker timeout')
            return real_analyze(folder, policy)
        try:
            with patch('continuous_adapter.analyze', slow_analyze):
                self.assertFalse(controller(capture, 45))
                self.assertTrue(entered.wait(5))
                prefix = capture.offset
                for i in range(8, 11):
                    feed(capture, sample(i), i*6)
                    self.assertFalse(controller(capture, i*6))
                self.assertGreater(capture.offset, prefix)
                self.assertEqual(controller.admitted, 0)
                self.assertIsNone(capture.failed)
                release.set()
                controller.pending.result(timeout=5)
                self.assertFalse(controller(capture, 60))
                self.assertEqual(controller.admitted, 1)
                folder = capture.output/'checkpoints/cold-1'
                self.assertEqual((folder/'stream.log').stat().st_size, prefix)
                self.assertEqual(analyze(folder, contract())['completed_cases'], 1)
        finally:
            release.set()
            controller.close()
            capture.close(capture.last)

    def test_original_readiness_bound_replays_at_float_exponent_boundary(self):
        origin = 2**16-100.123456789
        capture = ContinuousCapture(self.root/'float', LIMITS, ELF, start=origin, auto_cold=True)
        try:
            feed(capture, BOOT, origin)
            for i in range(1, 8):feed(capture, sample(i), origin+i*6)
            capture.confirm_physical_cold('Synthetic', origin+43)
            capture.tick(origin+45)
            token = capture.offer_case('idle-1', 'idle', 60, origin+45, readiness_seconds=120)
            self.assertEqual(capture.cases[-1]['readiness_seconds'], 120)
            capture.acknowledge_ready(token, 'Synthetic', origin+45)
            capture.begin_case(token, origin+45)
            for i in range(8, 18):feed(capture, sample(i), origin+i*6)
            capture.tick(origin+105)
            folder = capture.snapshot('idle-1', origin+105)
            self.assertEqual(analyze(folder, contract())['completed_cases'], 2)
        finally:
            capture.close(capture.last)


if __name__ == '__main__':
    unittest.main()
