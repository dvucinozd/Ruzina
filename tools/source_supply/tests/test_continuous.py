"""Synthetic passive-reader/handshake faults; never board acceptance evidence."""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from continuous import ContinuousCapture, pump
from test_analysis import transcript

ELF = 'a' * 64
BOOT = ('ELF file SHA256: ' + ELF + '\n').encode()
LIMITS = dict(usb_stack_min=2048, logger_stack_min=1024, internal_heap_min=65536,
              psram_heap_min=8388608, observed_internal_largest_min=8192,
              observed_psram_largest_min=262144, sd_stack_min=8192)


def report(i=1):
    return (transcript((0, 0), [i]) +
            'producer_water log_drops=0\n'
            'usb_uac_stats iso_errors=0 packet_errors=0 submit_errors=0\n'
            'deck_mix_stats underruns=0/0\n'
            'uac_fill_first generation=1 seen=1 phase=0 us=1880 after_iso_submit=0\n' +
            ''.join('uac_fill_phase phase=' + phase + ' generation=1 max_us=2000 ge_6000=0 failures=0\n'
                    for phase in ('initial_prefill', 'recovery_prefill', 'callback')) +
            'engine_resources usb_stack_free=4000 logger_stack_free=2000 internal_min=100000 '
            'psram_min=10000000 internal_largest=20000 psram_largest=1000000\n'
            'deck_engine_stats d1_flags=0 d2_flags=0\n').encode()


class ContinuousTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='continuous-synthetic-')
        self.path = Path(self.temporary.name) / 'capture'
        self.capture = ContinuousCapture(self.path, LIMITS, ELF)

    def tearDown(self):
        if not self.capture.closed:
            self.capture.close(self.capture.last)
        self.temporary.cleanup()

    def feed(self, raw, now, size=4096):
        for offset in range(0, len(raw), size):
            self.capture.feed(raw[offset:offset + size], now)

    def start(self, name='s03', seconds=30, kind='functional', now=2):
        token = self.capture.offer_case(name, kind, seconds, now)
        self.capture.acknowledge_ready(token, 'Spreman; učitat ću poznatu pjesmu', now)
        self.capture.begin_case(token, now)
        return token

    def test_arbitrary_chunks_preserve_identical_frames_and_full_stream(self):
        raw = BOOT + report(1) + report(2)
        self.feed(raw, 1, size=7)
        result = self.capture.close(2)
        self.assertIsNone(result['error'])
        self.assertEqual(len(result['frames']), 2)
        self.assertEqual((self.path / 'stream.log').read_bytes(), raw)
        self.assertEqual(result['raw_sha256'], hashlib.sha256(raw).hexdigest())
        for frame in result['frames']:
            self.assertTrue(raw[frame['start']:frame['end']].startswith(b'producer_profile deck=1 '))
            self.assertTrue(raw[frame['start']:frame['end']].endswith(b'deck_engine_stats d1_flags=0 d2_flags=0\n'))
        self.assertEqual(result['status'], 'CLOSED_NOT_QUALIFIED')

    def test_actual_idf_abbreviated_elf_line_and_bad_suffix(self):
        self.feed(('ELF file SHA256:  ' + ELF[:9] + '...\r\n').encode(), 1, size=7)
        self.feed(report(), 2)
        self.assertEqual(len(self.capture.close(3)['frames']), 1)
        second = ContinuousCapture(Path(self.temporary.name) / 'bad-elf', LIMITS, ELF)
        second.feed(('ELF file SHA256: ' + ELF[:9] + '...garbage\n').encode(), 1)
        self.assertIn('Unexpected/malformed boot', second.close(2)['error'])

    def test_same_stream_across_idle_ready_and_two_functional_cases(self):
        self.feed(BOOT + report(), 1)
        self.start('idle', 60, 'idle', 2)
        self.feed(report(2), 8)
        self.capture.tick(62)
        self.start('s03', now=63)
        self.feed(report(3), 69)
        self.capture.tick(93)
        self.start('s04', now=94)
        self.feed(report(4), 100)
        result = self.capture.close(124)
        self.assertIsNone(result['error'])
        self.assertEqual(len(result['frames']), 4)
        self.assertTrue(all(case['state'] == 'BOUND_REACHED_NOT_QUALIFIED' for case in result['cases']))
        self.assertEqual([case['complete_frames'] for case in result['cases']], [[1], [2], [3]])
        stream = (self.path / 'stream.log').read_bytes()
        for case in result['cases']:
            raw = (self.path / (case['name'] + '.log')).read_bytes()
            self.assertEqual(raw, stream[case['start_offset']:case['end_offset']])
            self.assertEqual(case['raw_sha256'], hashlib.sha256(raw).hexdigest())

    def test_not_started_until_owner_ready_and_no_stale_token_reuse(self):
        self.feed(BOOT, 1)
        token = self.capture.offer_case('s03', 'functional', 30, 2)
        with self.assertRaisesRegex(ValueError, 'not owner-ready'):
            self.capture.begin_case(token, 3)
        self.feed(report(), 60)  # owner preparation time is retained but not measured
        self.capture.acknowledge_ready(token, 'Spreman', 61)
        self.capture.begin_case(token, 62)
        self.capture.tick(92)
        new_token = self.capture.offer_case('s04', 'functional', 30, 93)
        with self.assertRaisesRegex(ValueError, 'Stale/missing'):
            self.capture.acknowledge_ready(token, 'old reply', 94)
        self.assertNotEqual(token, new_token)
        self.assertEqual(self.capture.cases[0]['start'], 62)

    def test_ready_timeout_and_delayed_begin_cannot_extend_window(self):
        token = self.capture.offer_case('s03', 'functional', 30, 0)
        self.capture.acknowledge_ready(token, 'Ready', 100)
        with self.assertRaisesRegex(ValueError, 'No ready case'):
            self.capture.begin_case(token, 121)
        self.assertIn('readiness timeout', self.capture.failed)
        self.assertEqual(self.capture.cases[0]['state'], 'READY_TIMEOUT')

    def test_setup_is_in_bound_and_late_read_cannot_donate_frames(self):
        self.feed(BOOT, 1)
        self.start()
        self.feed(report(), 20)  # setup is part of the 30-second functional bound
        self.feed(report(2), 34)  # delivered after deadline 32: stream only
        result = self.capture.close(35)
        case = result['cases'][0]
        self.assertEqual(case['deadline'], 32)
        self.assertEqual(case['complete_frames'], [0])
        self.assertEqual((self.path / 's03.log').read_bytes(), report())
        self.assertEqual(len(result['frames']), 2)

    def test_cut_frame_preserved_in_raw_not_silently_repaired(self):
        self.feed(BOOT, 1)
        self.start()
        prefix, suffix = report().split(b'producer_owner', 1)
        self.feed(prefix, 20)
        self.capture.tick(32)
        self.feed(b'producer_owner' + suffix, 33)
        result = self.capture.close(34)
        self.assertEqual(result['cases'][0]['complete_frames'], [])
        self.assertEqual((self.path / 's03.log').read_bytes(), prefix)
        self.assertEqual(len(result['frames']), 1)

    def test_original_joined_s03_prefix_is_invalid_and_raw_unchanged(self):
        repo = Path(__file__).resolve().parents[3]
        failed = repo / 'docs/boards/evidence/jc1060-20261030-com3/source-supply-playback-not-captured/run.log'
        prefix = failed.read_bytes().split(b'\n', 1)[0] + b'\n'
        self.feed(BOOT, 1)
        self.feed(prefix, 2, size=11)
        result = self.capture.close(3)
        self.assertEqual(result['error'], 'Duplicate telemetry fields')
        self.assertEqual((self.path / 'stream.log').read_bytes(), BOOT + prefix)
        self.assertEqual(result['status'], 'INVALID')

    def test_interleaved_writer_bytes_missing_stage_and_orphan_are_invalid(self):
        broken = [report().replace(b'uptime_us=6000000', b'uptime_us=heartbeat=1 uptime_us=6000000', 1),
                  report().replace(b'stage=step ', b'stage=push ', 1),
                  b'producer_profile_stage deck=1 seq=2 stage=poll calls=1 total_us=1 max_us=1\n',
                  b'broken producer_profile deck=1 generation=1\n']
        for index, raw in enumerate(broken):
            with self.subTest(index=index):
                path = Path(self.temporary.name) / str(index)
                capture = ContinuousCapture(path, LIMITS, ELF)
                capture.feed(BOOT, 0)
                for offset in range(0, len(raw), 4096):
                    if capture.failed:
                        break
                    capture.feed(raw[offset:offset + 4096], 1)
                self.assertIsNotNone(capture.failed)
                self.assertEqual(capture.close(2)['status'], 'INVALID')

    def test_fault_before_readiness_or_minimum_duration_stops_immediately(self):
        self.feed(BOOT, 1)
        self.start()
        raw = b'deck_mix_stats underruns=1/0\n'
        self.feed(raw, 5)
        self.assertIn('underrun', self.capture.failed)
        self.assertEqual(self.capture.cases[0]['state'], 'INVALID')
        result = self.capture.close(6)
        self.assertEqual(result['cases'][0]['end'], 5)
        self.assertEqual((self.path / 's03.log').read_bytes(), raw)

    def test_disconnect_unexpected_boot_and_terminal_fragment_remain_explicit(self):
        self.feed(BOOT + report(), 1)
        self.start()
        self.feed(b'producer_profile deck=1 ', 3)
        self.capture.disconnected(4)
        with self.assertRaisesRegex(ValueError, 'Invalid session'):
            self.feed(b'late reconnect\n', 5)
        result = self.capture.close(6)
        self.assertEqual(result['unterminated_bytes'], len(b'producer_profile deck=1 '))
        self.assertIn('no reopen', result['error'])
        second = ContinuousCapture(Path(self.temporary.name) / 'second', LIMITS, ELF)
        second.feed(BOOT, 1)
        second.feed(BOOT, 2)
        self.assertIn('Unexpected/malformed boot', second.close(3)['error'])

    def test_clock_and_case_bounds_rejected_without_qualifying_evidence(self):
        self.capture.tick(5)
        for now in (4, float('nan'), float('inf')):
            with self.subTest(now=now), self.assertRaises(ValueError):
                self.capture.tick(now)
        for kind, seconds in [('functional', 181), ('both', 61), ('idle', 59), ('soak', 60)]:
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                self.capture.offer_case('case', kind, seconds, 5)
        with self.assertRaisesRegex(ValueError, 'Invalid/reused'):
            self.capture.offer_case('stream', 'functional', 30, 5)
        self.assertEqual(self.capture.close(6)['physical_acceptance'], 'NOT_ASSESSED')

    def test_passive_reader_remains_open_across_cases_and_empty_reads(self):
        capture = self.capture
        clock = [0]
        class Reader:
            closes = 0
            reads = 0
            def read(self, count):
                self.reads += 1
                clock[0] += 1
                return BOOT if self.reads == 1 else (report(self.reads) if self.reads % 6 == 0 else b'')
            def write(self, data):
                raise AssertionError('No writes allowed')
            def close(self):
                self.closes += 1
        reader = Reader()
        def control(session, now):
            if now == 1:
                self.start('s03', now=now)
            if now == 32:
                self.start('s04', now=now)
            return now >= 62
        result = pump(capture, reader, lambda: clock[0], control)
        self.assertIsNone(result['error'])
        self.assertEqual(reader.closes, 1)
        self.assertEqual(len(result['cases']), 2)
        self.assertTrue(all(case['complete_frames'] for case in result['cases']))
        events = [json.loads(line) for line in (self.path / 'events.jsonl').read_text().splitlines()]
        self.assertEqual(sum(row['event'] == 'session_started' for row in events), 1)
        self.assertEqual(sum(row['event'] == 'owner_ready' for row in events), 2)

    def test_reader_exception_and_session_bound_close_without_reopen(self):
        clock = [0]
        class Reader:
            closes = 0
            def read(self, count):
                clock[0] += 1
                if clock[0] == 2:
                    raise OSError('USB disconnected')
                return BOOT
            def close(self):
                self.closes += 1
        reader = Reader()
        result = pump(self.capture, reader, lambda: clock[0], lambda capture, now: False)
        self.assertEqual(reader.closes, 1)
        self.assertIn('USB disconnected', result['error'])
        self.assertEqual((self.path / 'stream.log').read_bytes(), BOOT)
        second = ContinuousCapture(Path(self.temporary.name) / 'bounded', LIMITS, ELF, max_seconds=2)
        second.tick(2)
        self.assertIn('Session time bound', second.close(2)['error'])


if __name__ == '__main__':
    unittest.main()
