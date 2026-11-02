import copy
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT))
import campaign
from monitor import validate_window, capture_complete


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


if __name__=='__main__':unittest.main()
