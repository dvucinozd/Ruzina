from pathlib import Path
import json, struct

root = Path(r'C:\Users\Daniel\.codex\worktrees\m1-guition-bsp\research')
manifest = json.loads((root/'anlz-public-corpus/manifest.json').read_text())
def u32(data, at): return struct.unpack_from('>I', data, at)[0]
def sections(data):
    assert data[:4] == b'PMAI' and u32(data,8) == len(data)
    at = u32(data,4)
    while at < len(data):
        tag, header, total = data[at:at+4], u32(data,at+4), u32(data,at+8)
        assert 12 <= header <= total and at+total <= len(data)
        yield tag.decode('ascii'), data[at:at+header], data[at+header:at+total]
        at += total
    assert at == len(data)
def fnv(data):
    result = 0xcbf29ce484222325
    for byte in data: result = ((result ^ byte) * 0x100000001b3) & ((1<<64)-1)
    return f'{result:016x}'
def waveform(data):
    result = bytearray()
    for byte in data:
        white = byte >> 5
        result.extend(struct.pack('<HH', (byte & 31)*65535//31, ((white*31//7)<<11) | ((white*63//7)<<5) | 31))
    return fnv(result)
results = []
for pair in manifest['pairs']:
    dat = list(sections((root/'anlz-public-corpus'/pair['files'][0]['local']).read_bytes()))
    ext = list(sections((root/'anlz-public-corpus'/pair['files'][1]['local']).read_bytes()))
    tags = {tag:(header,body) for tag,header,body in dat+ext}
    header, raw_beats = tags['PQTZ']; beats = list(struct.iter_unpack('>HHI',raw_beats))
    assert u32(header,20) == len(beats)
    beat_bytes = b''.join(struct.pack('<IHI', time, phase-1, bpm) for phase,bpm,time in beats)
    preview, detail = tags['PWAV'][1], tags['PWV3'][1]
    cues = set()
    for tag, header, body in dat:
        if tag != 'PCOB' or u32(header,12) != 1: continue
        for at in range(0,len(body),56):
            record = body[at:at+56]
            assert record[:4] == b'PCPT'
            slot = u32(record,12)
            if 1 <= slot <= 8: cues.add(slot)
    expected = {'beats':len(beats), 'preview':len(preview), 'detail':len(detail), 'cues':len(cues), 'bpm_x100':beats[0][1],
                'beat_hash':fnv(beat_bytes), 'preview_hash':waveform(preview), 'detail_hash':waveform(detail)}
    lines = (root/f"public-anlz-{pair['id']}.log").read_text().splitlines()
    actual = json.loads(next(line for line in lines if line.startswith('{')))
    assert actual == expected, (pair['id'], actual, expected)
    released_lines = (root/f"m22-anlz-{pair['id']}.log").read_text().splitlines()
    released = json.loads(next(line for line in released_lines if line.startswith('{')))
    assert actual == released, (pair['id'], actual, released)
    results.append({'id':pair['id'], 'result':'PASS', 'tags':sorted(tags), 'actual':actual, 'expected_independent_wire_decode':expected, 'released_m22_normalized':released})
output = {'source':manifest, 'released_reference':json.loads((root/'m22-anlz-reference/source.json').read_text()), 'results':results, 'scope':'bounded parser/canonical data comparison; no device/audio or full export-version qualification; real pairs contain no imported hot cues'}
(root/'public-anlz-results.json').write_text(json.dumps(output,indent=2)+'\n')
print(f"PASS: {len(results)} pinned public DAT/EXT pairs match independent decoding and released M2.2 beat/waveform data")
