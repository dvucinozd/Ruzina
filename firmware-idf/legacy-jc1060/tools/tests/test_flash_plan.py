import hashlib
from pathlib import Path
import struct
import sys
import unittest
import json
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_flash_plan import partitions, overlay_history, sha, BACKUP_SHA


def table(records):
    data = b''.join(struct.pack('<HBBII16sI', 0x50aa, 1, 2, start, size, b'nvs', 0)
                    for start, size in records)
    return data + b'\xeb\xeb' + b'\xff' * 14 + hashlib.md5(data).digest()


class FlashPlanTests(unittest.TestCase):
    def test_history_preserves_tail_after_smaller_upgrade(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp)
            (root/'old.bin').write_bytes(b'A'*32)
            old=dict(factory_sha256=BACKUP_SHA,unit_id='test',writes=[dict(offset=8,erase_end=40,
                     staged_file='old.bin',staged_sha256=sha(b'A'*32))])
            (root/'old.json').write_text(json.dumps(old))
            (root/'small.bin').write_bytes(b'B'*8)
            small=dict(factory_sha256=BACKUP_SHA,unit_id='test',predecessor_plan_sha256=sha((root/'old.json').read_bytes()),
                       writes=[dict(offset=8,erase_end=16,staged_file='small.bin',staged_sha256=sha(b'B'*8))])
            (root/'small.json').write_text(json.dumps(small))
            data,_,chain=overlay_history(b'\xff'*64,'test',[root/'old.json',root/'small.json'])
            self.assertEqual(data[8:16],b'B'*8)
            self.assertEqual(data[16:40],b'A'*24)
            self.assertEqual(len(chain),2)
            for paths in ([root/'small.json'],[root/'small.json',root/'old.json']):
                with self.assertRaisesRegex(ValueError,'chain'):
                    overlay_history(b'\xff'*64,'test',paths)
            (root/'old.bin').write_bytes(b'C'*32)
            with self.assertRaisesRegex(ValueError,'hash'):
                overlay_history(b'\xff'*64,'test',[root/'old.json',root/'small.json'])

    def test_valid_table(self):
        self.assertEqual(partitions(table([(0x9000, 0x6000)]))[0]['bytes'], 0x6000)

    def test_invalid_table(self):
        for payload in (table([(0x9000, 0x6000), (0xa000, 0x1000)]),
                        table([(0xfff000, 0x2000)]),
                        table([(0x9000, 0)]),
                        table([(0x9000, 0x6000)])[:-1] + b'\0', b''):
            with self.subTest(payload=payload[:8]), self.assertRaises(ValueError):
                partitions(payload)
