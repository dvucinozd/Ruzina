import copy
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT))
import campaign


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


if __name__=='__main__':unittest.main()
