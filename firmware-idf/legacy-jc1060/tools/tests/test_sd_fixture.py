import hashlib
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from make_sd_fixture import SIZE, RELATIVE, block, create


class FixtureTests(unittest.TestCase):
    def test_rust_fixture_and_c_success_telemetry_agree(self):
        import re
        app=Path(__file__).resolve().parents[2]
        rust=(app/'rust-core/src/exfat_fixture.rs').read_text(encoding='utf-8')
        digest=re.search(r'pub const SHA:.*?=\s*\[(.*?)\];',rust,re.S).group(1)
        expected=bytes(int(x,16) for x in re.findall(r'0x([0-9a-f]{2})',digest)).hex()
        self.assertEqual(expected,'1cb18815702b14ff92ad02f4976a162481d8d91beb429ba5607b131d4ab281de')
        self.assertIn('/'+RELATIVE,rust)
        self.assertIn('sha256='+expected,(app/'main/sd_bridge.c').read_text())

    def test_fixed_fixture_and_no_overwrite(self):
        with tempfile.TemporaryDirectory() as temp:
            output=Path(temp)/'kit'
            manifest=create(output)
            payload=(output/RELATIVE).read_bytes()
            self.assertEqual(len(payload),8389121)
            self.assertEqual(hashlib.sha256(payload).hexdigest(),
                             '1cb18815702b14ff92ad02f4976a162481d8d91beb429ba5607b131d4ab281de')
            self.assertEqual(manifest['sha256'],hashlib.sha256(payload).hexdigest())
            for offset in (0,511,512,4095,4096,131071,131072,SIZE-513,SIZE-1):
                self.assertEqual(payload[offset:offset+513],block(offset,min(513,SIZE-offset)))
            with self.assertRaises(FileExistsError):
                create(output)
            self.assertEqual((output/RELATIVE).read_bytes(),payload)

    def test_host_and_device_names_and_digest_agree(self):
        source=(Path(__file__).resolve().parents[2]/'main/sd_fixture.c').read_text(encoding='utf-8')
        self.assertIn('0:/'+RELATIVE,source)
        import re
        digest=source.split('expected_sha[32] = {',1)[1].split('};',1)[0]
        self.assertEqual(bytes(int(x,16) for x in re.findall(r'0x([0-9a-f]{2})',digest)).hex(),
                         '1cb18815702b14ff92ad02f4976a162481d8d91beb429ba5607b131d4ab281de')
