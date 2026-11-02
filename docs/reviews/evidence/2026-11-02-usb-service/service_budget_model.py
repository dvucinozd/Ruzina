"""Idealized service-budget arithmetic, NOT a USB driver or observed replay.

Four initially queued 6 ms URBs, no IRQ/HCD scheduling cost, perfect scheduling
of pending URBs. CPU callbacks serialize; callback cost includes resubmission.
Actual driver deadlines require completion/dispatch/submission/frame evidence.
"""
import json
from pathlib import Path
import unittest


def simulate(costs_us, dispatch_delay_us=0):
    duration = 6000
    completed_at = [duration * n for n in range(1, 5)]
    cpu_free = 0
    rows = []
    for cost in costs_us:
        if cost < 0 or dispatch_delay_us < 0:
            raise ValueError('Negative time')
        released = completed_at.pop(0)
        start = max(cpu_free, released + dispatch_delay_us)
        end = start + cost
        tail = completed_at[-1]
        gap = max(0, end - tail)
        completed_at.append(max(end, tail) + duration)
        rows.append(dict(released_us=released,callback_start_us=start,
                         resubmit_us=end,old_tail_us=tail,modeled_gap_us=gap))
        cpu_free = end
    return rows


class BudgetTests(unittest.TestCase):
    def test_initial_headroom_is_18_not_24_after_first_completion(self):
        row = simulate([0])[0]
        self.assertEqual(row['old_tail_us'] - row['callback_start_us'], 18000)

    def test_one_17ms_callback_can_fit_but_two_delays_can_exhaust_pool(self):
        self.assertEqual(simulate([17386])[0]['modeled_gap_us'], 0)
        self.assertEqual(simulate([17386,8385])[1]['modeled_gap_us'], 1771)

    def test_dispatch_delay_consumes_the_same_slack(self):
        self.assertEqual(simulate([14000],dispatch_delay_us=6000)[0]['modeled_gap_us'], 2000)

    def test_no_margin_at_6ms_utilization_and_overload_not_fixed_by_pool(self):
        self.assertTrue(all(row['modeled_gap_us']==0 for row in simulate([6000]*50)))
        self.assertTrue(any(row['modeled_gap_us']>0 for row in simulate([7000]*50)))

    def test_short_bounded_work_does_not_exhaust_ideal_pool(self):
        self.assertTrue(all(row['modeled_gap_us']==0 for row in simulate([3000]*100)))

    def test_invalid_negative_delays_rejected(self):
        with self.assertRaises(ValueError): simulate([-1])


if __name__ == '__main__':
    suite=unittest.defaultTestLoader.loadTestsFromTestCase(BudgetTests)
    result=unittest.TextTestRunner(verbosity=2).run(suite)
    if not result.wasSuccessful(): raise SystemExit(1)
    examples={
        'single_outlier_can_fit':simulate([17386]),
        'successive_delays_can_empty_pool':simulate([17386,8385]),
        'dispatch_delay_consumes_headroom':simulate([14000],6000),
    }
    Path(__file__).with_name('service-budget-results.json').write_text(json.dumps(
        dict(status='MODEL_ARITHMETIC_PASS_NOT_TARGET_QUALIFICATION',tests=result.testsRun,
             assumptions=__doc__,examples=examples),indent=2)+'\n')
