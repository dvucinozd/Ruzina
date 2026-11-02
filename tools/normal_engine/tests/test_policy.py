import copy
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT))
import campaign
from monitor import validate_window, capture_complete, CaptureCoverage


class NormalPolicyTests(unittest.TestCase):
    def setUp(self):
        self.manifest=dict(source_commit=campaign.SOURCE,dirty_status='',variant='deck-engine',
                           uac_profile=False,uac_pack_iram=False)

    def test_accepts_only_clean_normal_engine(self):
        campaign.validate_candidate_manifest(self.manifest)
        for key,value in [('source_commit','old'),('dirty_status',' M main.c'),('variant','deck-play'),
                          ('uac_profile',True),('uac_pack_iram',True),('uac_profile',0)]:
            m=copy.deepcopy(self.manifest)
            m[key]=value
            with self.subTest(key=key,value=value), self.assertRaises(ValueError):
                campaign.validate_candidate_manifest(m)

    def test_shared_guard_unchanged_and_no_serial_imports(self):
        self.assertIs(campaign.program,campaign._shared.program)
        self.assertIs(campaign.EspBackend,campaign._shared.EspBackend)
        self.assertNotIn('serial',sys.modules)
        self.assertNotIn('esptool',sys.modules)

    def test_distinct_bounded_windows(self):
        for window,seconds,after,mode in [('cold',600,45,'candidate'),('idle',60,0,'candidate'),
                ('functional',180,0,'candidate'),('functional',180,0,'return-s07'),('soak',1800,0,'candidate')]:
            validate_window(window,seconds,after,mode)
        for window,seconds,after,mode in [('cold',601,45,'candidate'),('cold',60,0,'candidate'),
                ('idle',180,0,'candidate'),('functional',301,0,'candidate'),('soak',1800,0,'return-s07'),
                ('functional',180,45,'candidate'),('soak',1801,0,'candidate'),('other',60,0,'candidate')]:
            with self.subTest(window=window,seconds=seconds,mode=mode), self.assertRaises(ValueError):
                validate_window(window,seconds,after,mode)

    def test_late_boot_and_shortened_windows_are_incomplete(self):
        self.assertFalse(capture_complete('cold',600,600,None,False))
        self.assertFalse(capture_complete('cold',600,600,44.99,False))
        self.assertTrue(capture_complete('cold',600,600,45,False))
        self.assertFalse(capture_complete('idle',30,60,None,True))
        self.assertFalse(capture_complete('soak',1799.99,1800,None,False))
        self.assertTrue(capture_complete('soak',1800,1800,None,False))
        self.assertFalse(capture_complete('functional',29.99,180,None,True))
        self.assertTrue(capture_complete('functional',30,180,None,True))
        self.assertFalse(capture_complete('functional',30,180,None,False))

    def test_port_wait_never_counts_as_connected_idle(self):
        capture=CaptureCoverage('idle',60,0)
        self.assertFalse(capture.active(55))  # Missing port times out after ten seconds.
        capture.opened(5)
        self.assertTrue(capture.active(60))
        self.assertEqual(capture.seconds(60),55)
        self.assertFalse(capture_complete('idle',capture.seconds(60),60,None,False))
        self.assertFalse(capture.active(65))
        self.assertTrue(capture_complete('idle',capture.seconds(65),60,None,False))

    def test_disconnect_invalidates_cold_and_steady_windows(self):
        for kind,requested in [('cold',600),('idle',60),('functional',180),('soak',1800)]:
            capture=CaptureCoverage(kind,requested,0)
            capture.opened(1)
            if kind=='cold':capture.boot(5)
            capture.disconnected(20)
            elapsed=capture.seconds(20)
            self.assertEqual(capture.seconds(1000),elapsed)  # Disconnection cannot advance capture.
            self.assertFalse(capture.active(21))
            self.assertFalse(capture_complete(kind,elapsed,requested,elapsed,True,capture.interrupted))

    def test_cold_preboot_power_cycle_does_not_start_or_invalidate_capture(self):
        capture=CaptureCoverage('cold',600,0)
        capture.opened(1)
        capture.disconnected(2)
        capture.opened(10)
        self.assertFalse(capture.interrupted)
        self.assertEqual(capture.seconds(100),0)
        capture.boot(100)
        self.assertEqual(capture.seconds(145),45)
        self.assertTrue(capture_complete('cold',capture.seconds(145),600,45,False,capture.interrupted))


if __name__=='__main__':unittest.main()
