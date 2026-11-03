"""Read-only control audit of retained continuous evidence, never admission."""
import argparse
import hashlib
import json
from pathlib import Path
import re

MIDI = re.compile(rb'midi_event deck=(\d+) control=(\w+)\(\d+\) kind=\d+ value=-?\d+ max=\d+\r?\n')
ALLOWED = {
    'cold-1': set(), 'idle-1': set(),
    's03-deck1': {(0, 'BrowsePress'), (0, 'BrowseDelta'), (1, 'Load'), (1, 'Play')},
    's04-deck2': {(0, 'BrowsePress'), (0, 'BrowseDelta'), (2, 'Load'), (1, 'Play'), (2, 'Play')},
    's05-both': {(1, 'Play'), (2, 'Play')},
}


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def audit(raw, capture, adapter):
    if sha(raw) != capture['raw_sha256'] or sha(raw) != adapter['raw_sha256']:
        raise ValueError('Raw stream digest mismatch')
    records, transitions, pending = [], [], []
    offset, previous = 0, 0
    for number, line in enumerate(raw.splitlines(keepends=True), 1):
        end = offset + len(line)
        if line.startswith(b'midi_event '):
            match = MIDI.fullmatch(line)
            if not match:
                raise ValueError(f'Malformed/incomplete MIDI at byte {offset}')
            record = dict(line=number, start=offset, end=end, deck=int(match[1]),
                          control=match[2].decode('ascii'), text=line.decode('ascii').rstrip())
            records.append(record)
            pending.append(record)
        if line.startswith(b'deck_engine_stats '):
            match = re.search(rb'\bunsupported=(\d+)\b', line)
            if not match:
                raise ValueError('Missing unsupported counter')
            current = int(match[1])
            if current != previous:
                transitions.append(dict(line=number, previous=previous, current=current,
                                        midi_since_previous_stats=list(pending)))
            previous = current
            pending = []
        offset = end
    cases, last = [], 0
    for case in capture['cases']:
        start, end = case['start_offset'], case['end_offset']
        if not (0 <= last <= start <= end <= len(raw)):
            raise ValueError('Invalid/overlapping case offsets')
        if sha(raw[start:end]) != case['raw_sha256']:
            raise ValueError('Case slice digest mismatch')
        last = end
        if case['name'] not in ALLOWED:
            raise ValueError('Unknown case policy')
        inside = [r for r in records if start <= r['start'] and r['end'] <= end]
        crossing = [r for r in records if r['start'] < end and r['end'] > start and r not in inside]
        extra = [r for r in inside if (r['deck'], r['control']) not in ALLOWED[case['name']]]
        cases.append(dict(name=case['name'], control_count=len(inside), extra_controls=extra,
                          boundary_crossing_controls=crossing,
                          status='EXTRA_CONTROLS_OBSERVED' if extra else 'NO_EXTRA_CONTROLS_OBSERVED'))
    return dict(status='OFFLINE_CONTROL_AUDIT_NOT_ADMISSION', raw_sha256=sha(raw),
                device_access=False, previous_admission_modified=False,
                scope='Allowed pairs describe requested setup; not timing/order or listening qualification',
                cases=cases, unsupported_transitions=transitions)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--capture', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    raw = (args.capture/'stream.log').read_bytes()
    result_bytes = (args.capture/'result.json').read_bytes()
    adapter_bytes = (args.capture/'adapter-result.json').read_bytes()
    result = audit(raw, json.loads(result_bytes), json.loads(adapter_bytes))
    result.update(capture_result_sha256=sha(result_bytes), adapter_result_sha256=sha(adapter_bytes))
    with args.output.open('x', encoding='utf-8') as output:
        output.write(json.dumps(result, indent=2, ensure_ascii=False)+'\n')
    print(json.dumps(dict(status=result['status'], cases=[dict(name=c['name'], extra_controls=len(c['extra_controls']))
                                                        for c in result['cases']])))


if __name__ == '__main__':
    main()
