import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from analyze_uac_cause import analyze, STAGES


def fixture():
    lines = [
        'uac_profile_calibration gen=1 passed=1 before_valid=1 after_valid=1 wall_before=0 wall_after=100 away_before=0 away_after=90 switches_before=0 switches_after=1',
        'uac_profile_irq_calibration gen=1 passed=1 before_valid=1 after_valid=1 entries_before=0 entries_after=1 us_before=0 us_after=10 nested_before=0 nested_after=0 wall_before=0 wall_after=100',
        'uac_profile kind=worst seen=1 gen=1 ordinal=1 whole_us=100 rust_us=100 outer_us=0 partition_valid=1 sched_valid=1 sched_window_us=110 away_us=0 switches=0',
        'uac_profile_cause kind=worst gen=1 ordinal=1 calibration=1 away_before=90 away_after=90 switches_before=1 switches_after=1 irq_calibration=1 irq_valid=1 irq_window_us=110 irq_dispatch_us=30 irq_entries=1 irq_nested=0',
        'uac_profile_distribution gen=1 scope=BOOT samples=1 valid=1',
    ]
    for stage in STAGES:
        lines.append(f'uac_profile_stage kind=worst gen=1 ordinal=1 stage={stage} us={100 if stage == "graph" else 0} segments=1')
    for stage in ('whole', *STAGES):
        lines.append(f'uac_profile_hist gen=1 samples=1 stage={stage} max_us=100 bins=0,1,0,0,0,0,0')
    return '\n'.join(lines).encode()


class AnalysisTests(unittest.TestCase):
    def test_coherent_partition_and_wider_windows(self):
        result = analyze(fixture())
        record = result['records'][0]
        self.assertTrue(record['partition_verified'])
        self.assertTrue(record['scheduler_usable'])
        self.assertTrue(record['irq_dispatch_usable'])
        self.assertEqual(record['irq_dispatch_inside_whole_bounds_us'], [20, 30])
        self.assertTrue(result['distributions'][0]['complete'])

    def test_failed_or_missing_calibration_denies_attribution(self):
        for data in (fixture().replace(b'passed=1', b'passed=0'),
                     b'\n'.join(l for l in fixture().splitlines() if b'_calibration ' not in l)):
            record = analyze(data)['records'][0]
            self.assertFalse(record['scheduler_usable'])
            self.assertFalse(record['irq_dispatch_usable'])

    def test_nested_irq_and_absolute_counter_disagreement(self):
        r = analyze(fixture().replace(b'irq_nested=0', b'irq_nested=1'))['records'][0]
        self.assertTrue(r['scheduler_usable'])
        self.assertFalse(r['irq_dispatch_usable'])
        r = analyze(fixture().replace(b'switches_after=1 irq_calibration', b'switches_after=2 irq_calibration'))['records'][0]
        self.assertFalse(r['scheduler_usable'])

    def test_truncated_or_malformed_irq_calibration_denied(self):
        for key in ('us_before', 'us_after', 'nested_before', 'nested_after', 'wall_before', 'wall_after'):
            lines = fixture().splitlines()
            # Keep the other complete records, like a dropped serial tail.
            lines[1] = lines[1].split((' ' + key + '=').encode())[0]
            self.assertFalse(analyze(b'\n'.join(lines))['records'][0]['irq_dispatch_usable'])
        for value in (b'-1', b'4294967296', b'incomplete'):
            data = fixture().replace(b'us_after=10 ', b'us_after=' + value + b' ')
            self.assertFalse(analyze(data)['records'][0]['irq_dispatch_usable'])
        data = fixture().replace(b'us_after=10 ', b'us_after=101 ')
        self.assertFalse(analyze(data)['records'][0]['irq_dispatch_usable'])

    def test_incomplete_partition_and_histogram_denied(self):
        data = b'\n'.join(l for l in fixture().splitlines() if b'stage=pack' not in l)
        result = analyze(data)
        self.assertFalse(result['records'][0]['partition_verified'])
        self.assertFalse(result['records'][0]['scheduler_usable'])
        self.assertFalse(result['records'][0]['irq_dispatch_usable'])
        self.assertFalse(result['distributions'][0]['complete'])

    def test_geometry_saturation_and_conflicts(self):
        r = analyze(fixture().replace(b'irq_window_us=110', b'irq_window_us=99'))['records'][0]
        self.assertFalse(r['irq_dispatch_usable'])
        d = analyze(fixture().replace(b'samples=1', b'samples=4294967295'))['distributions'][0]
        self.assertFalse(d['complete'])
        with self.assertRaisesRegex(ValueError, 'Conflicting coherent identity'):
            analyze(fixture() + b'\nuac_profile_stage kind=worst gen=1 ordinal=1 stage=graph us=99 segments=1')


if __name__ == '__main__':
    unittest.main()
