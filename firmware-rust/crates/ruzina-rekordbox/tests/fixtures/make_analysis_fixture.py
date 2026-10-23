"""Synthetic protocol fixtures, no audio or proprietary export content.

Independent encoding of Deep Symmetry's documented ANLZ field layouts.
Run here to regenerate; analysis-fixtures.json records SHA-256 identities.
"""
from pathlib import Path
import hashlib, json, struct

root = Path(__file__).parent
u32 = lambda value: struct.pack('>I', value)

def tag(name, header, body):
    size = 12 + len(header)
    return name + u32(size) + u32(size + len(body)) + header + body

def container(tags):
    body = b''.join(tags)
    return b'PMAI' + u32(28) + u32(28 + len(body)) + bytes(16) + body

def cue(slot, start, end=0):
    record = bytearray(56)
    record[:4] = b'PCPT'
    record[4:8], record[8:12], record[12:16] = u32(28), u32(56), u32(slot)
    record[28] = 2 if end else 1
    record[32:36], record[36:40] = u32(start), u32(end)
    return bytes(record)

def dat(times, bpm):
    beats = b''.join(struct.pack('>HHI', index % 4 + 1, bpm, time) for index, time in enumerate(times))
    preview = bytes((index & 31) | ((index // 32 % 8) << 5) for index in range(400))
    return container([
        tag(b'UNKN', b'', b'ignored'),
        tag(b'PQTZ', u32(0) + u32(0x80000) + u32(len(times)), beats),
        tag(b'PWAV', u32(400) + u32(0x10000), preview),
        tag(b'PCOB', u32(0) + struct.pack('>HHI', 0, 1, 1), cue(0, 750)),
        tag(b'PCOB', u32(1) + struct.pack('>HHI', 0, 2, 0), cue(1, 1000) + cue(2, 2000, 4000)),
    ])

files = {
    'track-a.DAT': dat([1000 + index * 500 for index in range(8)], 12000),
    'track-b.DAT': dat([8000 + index * 469 for index in range(8)], 12800),
    'track-a.EXT': container([tag(b'PWV3', u32(1) + u32(1025) + u32(0x960000), bytes((index * 3) & 255 for index in range(1025)))]),
}
for name, content in files.items(): (root/name).write_bytes(content)
(root/'analysis-fixtures.json').write_text(json.dumps({
    'reference': 'https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/anlz.html',
    'origin': 'synthetic standards-layout fixtures; not physical Rekordbox export qualification',
    'files': {name: {'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()} for name, data in files.items()},
}, indent=2)+'\n')
