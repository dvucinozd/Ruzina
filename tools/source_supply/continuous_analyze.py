"""Replay the entire continuous journal before any case can grant continuation."""
import argparse
import bisect
import json
from pathlib import Path
import re
import tempfile

from analyze import fields, monotonic, numbers, profiles
from campaign import require, sha
from continuous import ContinuousCapture
from continuous_contract import APP, CASES, ELF, LIMITS, load_contract, validate


def replay(folder, contract):
    """Rebuild byte receipt, framing and operator events; trust no stored offsets."""
    validate(contract)
    folder = Path(folder)
    result = json.loads((folder/'result.json').read_text())
    raw = (folder/'stream.log').read_bytes()
    journal = (folder/'events.jsonl').read_bytes()
    require(result['status'] in ('CHECKPOINT_NOT_QUALIFIED', 'CLOSED_NOT_QUALIFIED') and
            result['error'] is None, 'Invalid session cannot grant continuation')
    require(result['bytes'] == len(raw) and result['raw_sha256'] == sha(raw) and
            result['events_sha256'] == sha(journal), 'Whole-stream/journal hash mismatch')
    require(result['auto_cold'] is True and result['expected_elf_sha256'] == ELF, 'Unanchored image/session')
    events = [json.loads(line) for line in journal.splitlines()]
    require(events and events[0]['event'] == 'session_started' and
            events[-1]['event'] in ('session_checkpoint', 'session_closed'), 'Incomplete journal')
    tokens = iter(row['case']['token'] for row in events if row['event'] == 'case_offered')
    with tempfile.TemporaryDirectory(prefix='continuous-replay-') as temporary:
        capture = ContinuousCapture(Path(temporary)/'capture', LIMITS, ELF,
                                    start=result['start_seconds'], max_seconds=result['max_seconds'],
                                    auto_cold=True, token_factory=lambda: next(tokens))
        end = 0
        rebuilt = None
        try:
            for row in events[1:]:
                kind, now = row['event'], row['received_seconds']
                if kind == 'serial_chunk':
                    require(row['start'] == end and end < row['end'] <= len(raw), 'Chunk gap/overlap/size')
                    data = raw[end:row['end']]
                    require(sha(data) == row['sha256'], 'Chunk hash mismatch')
                    capture.feed(data, now)
                    require(capture.failed is None, 'Replay rejected stream: '+str(capture.failed))
                    end = row['end']
                elif kind == 'physical_cold_confirmed':
                    capture.confirm_physical_cold(row['reply'], now)
                elif kind == 'case_offered':
                    case = row['case']
                    capture.offer_case(case['name'], case['kind'], case['seconds'], now,
                                       readiness_seconds=case['readiness_seconds'])
                elif kind == 'owner_ready':
                    capture.acknowledge_ready(row['token'], row['reply'], now)
                elif kind == 'case_started':
                    capture.begin_case(row['token'], now)
                elif kind == 'case_ended':
                    require(row['case']['state'] == 'BOUND_REACHED_NOT_QUALIFIED', 'Incomplete/fault case')
                    if capture.active is not None:
                        capture.tick(now)
                elif kind == 'session_checkpoint':
                    path = capture.snapshot(row['name'], now)
                    rebuilt = json.loads((path/'result.json').read_text())
                elif kind == 'session_closed':
                    rebuilt = capture.close(now)
                else:
                    require(kind in ('expected_elf_received', 'cold_started', 'complete_report_frame'),
                            'Unknown/invalid journal event')
            require(end == len(raw) and rebuilt == result, 'Stored session/frame/case metadata drift')
            rebuilt_events = (capture.output/'events.jsonl').read_bytes()
            require(rebuilt_events == journal, 'Event replay mismatch')
        finally:
            if not capture.closed:
                capture.close(capture.last)
    for case in result['cases']:
        require(re.fullmatch(r'[a-z0-9][a-z0-9-]{0,63}', case['name']), 'Unsafe case name')
        case_raw = (folder/(case['name']+'.log')).read_bytes()
        require(case_raw == raw[case['start_offset']:case['end_offset']] and
                sha(case_raw) == case['raw_sha256'], 'Case is not an exact whole-stream slice')
    return result, raw, events


def receive_freshness(raw, events):
    chunks = [row for row in events if row['event'] == 'serial_chunk']
    ends = [row['end'] for row in chunks]
    anchors, snapshots = [], []
    offset = 0
    for line in raw.splitlines(keepends=True):
        offset += len(line)
        if not line.endswith(b'\n'):
            continue
        text = line.decode('utf-8').strip()
        if text.startswith(('heartbeat=', 'producer_profile ', 'producer_owner ')):
            host = chunks[bisect.bisect_left(ends, offset)]['received_seconds']
            uptime = numbers(fields(text), ('uptime_us',))['uptime_us']
            (anchors if text.startswith('heartbeat=') else snapshots).append((host, uptime))
    require(len(anchors) >= 2, 'Missing host/device receive anchors')
    require(all(b[1] > a[1] for a, b in zip(anchors, anchors[1:])), 'Heartbeat clock regression')
    offsets = [host*1_000_000-uptime for host, uptime in anchors]
    require(max(offsets)-min(offsets) <= 2_000_000, 'Buffered/delayed receive anchors exceed 2 seconds')
    for host, uptime in snapshots:
        closest = min(anchors, key=lambda anchor: abs(anchor[0]-host))
        require(abs((uptime-closest[1])-(host-closest[0])*1_000_000) <= 10_000_000,
                'Stale producer/owner source timestamp')


def numerical(text, spec):
    playing = tuple(spec['playing'])
    integrity = profiles(text, playing)
    lines = text.splitlines()
    def rows(prefix, keys):
        return [numbers(fields(line), keys) for line in lines if line.startswith(prefix)]
    phases = {phase: rows('uac_fill_phase phase='+phase+' ',
                          ('generation', 'count', 'max_us', 'gt_3000', 'ge_6000', 'failures'))
              for phase in ('initial_prefill', 'recovery_prefill', 'callback')}
    require(all(len(values) >= 2 for values in phases.values()), 'Missing fill observations')
    for values in phases.values():
        require(all(row['generation'] == 1 and row['max_us'] < 6000 and
                    row['ge_6000'] == row['failures'] == 0 for row in values), 'Fill timing hard stop')
        monotonic(values, ('count', 'max_us', 'gt_3000', 'ge_6000', 'failures'))
    callbacks = phases['callback']
    require(callbacks[-1]['count'] > callbacks[0]['count'], 'No callback progress')
    if spec['kind'] == 'cold':
        require(callbacks[-1]['count'] >= 7000, 'Cold callback bound')
    if spec['kind'] == 'idle':
        require(callbacks[-1]['count']-callbacks[0]['count'] >= 9000, 'Idle callback bound')
    resources = rows('engine_resources ', ('usb_stack_free', 'logger_stack_free', 'internal_min',
                                          'psram_min', 'internal_largest', 'psram_largest'))
    require(len(resources) >= 2, 'Missing resource observations')
    loads = rows('rust_load ', ('stack_free',))
    require(loads and min(row['stack_free'] for row in loads) >= LIMITS['sd_stack_min'], 'Missing/low SD reserve')
    usb = rows('usb_uac_stats ', ('transfers', 'iso_errors', 'packet_errors', 'submit_errors'))
    require(len(usb) >= 2 and usb[-1]['transfers'] > usb[0]['transfers'] and
            all(row['iso_errors'] == row['packet_errors'] == row['submit_errors'] == 0 for row in usb),
            'Missing USB progress/errors')
    monotonic(usb, ('transfers',))
    engines = rows('deck_engine_stats ', ('d1_flags', 'd2_flags'))
    require(len(engines) >= 2, 'Missing engine observations')
    overlap_us = None
    if spec['kind'] in ('cold', 'idle'):
        require(all(row['d1_flags'] == row['d2_flags'] == 0 for row in engines), 'Source active during idle admission')
    else:
        require(any(tuple(int(bool(row[key]&2)) for key in ('d1_flags', 'd2_flags')) == playing for row in engines),
                'Requested engine state absent')
        for deck in (1, 2):
            if playing[deck-1]:
                stable = integrity['stable_intervals'][str(deck)]
                require(sum(row['source_elapsed_us'] for row in stable) >= 20_000_000 and
                        sum(row['frames'] for row in stable) > 0, 'Need 20 measured source seconds with production')
        if playing == (1, 1):
            stamps = {deck:{} for deck in (1, 2)}
            for line in lines:
                if line.startswith('producer_profile '):
                    row = numbers(fields(line), ('deck', 'seq', 'uptime_us'))
                    stamps[row['deck']][row['seq']] = row['uptime_us']
            intervals = {deck:[(stamps[deck][row['start_seq']], stamps[deck][row['end_seq']])
                               for row in integrity['stable_intervals'][str(deck)]] for deck in (1, 2)}
            overlap_us = sum(max(0, min(a1, b1)-max(a0, b0))
                             for a0, a1 in intervals[1] for b0, b1 in intervals[2])
            require(overlap_us >= 20_000_000, 'Need 20 overlapping both-playing source seconds')
    maximum = max(row['max_us'] for values in phases.values() for row in values)
    return dict(integrity=integrity, all_fill_max_us=maximum, both_source_overlap_us=overlap_us,
                headroom_target_pass=maximum <= 3000 and all(row['gt_3000'] == 0 for values in phases.values() for row in values),
                callback_first=callbacks[0], callback_last=callbacks[-1],
                resource_minima={key:min(row[key] for row in resources) for key in resources[0]},
                sd_client_stack_min=min(row['stack_free'] for row in loads))


def analyze(folder, contract):
    result, raw, events = replay(folder, contract)
    require(result['boot_count'] == 1 and result['cold_confirmation'], 'Missing physical cold confirmation')
    require(re.findall(rb'chip_revision=(\d+)', raw) == [b'103'] and
            re.findall(rb'reset_reason=(\d+)', raw) == [b'1'], 'Wrong revision/cold reset identity')
    prep = list(re.finditer(rb'resource_prepare state=(PREPARING|READY)\b', raw))
    require([row[1] for row in prep] == [b'PREPARING', b'READY'] and
            raw.count(b'usb_uac stream=STARTED') == 1 and
            prep[-1].start() < raw.index(b'usb_uac stream=STARTED'), 'Cold readiness/UAC ordering')
    receive_freshness(raw, events)
    frames = result['frames']
    require(len(frames) >= 2, 'Missing complete report frames')
    # Validate every complete frame, including those between case slices.
    profiles(raw[frames[0]['start']:frames[-1]['end']].decode(), (0, 0))
    summaries = []
    require(0 < len(result['cases']) <= len(CASES), 'Missing/unregistered cases')
    for case, spec in zip(result['cases'], CASES):
        require(all(case[key] == spec[key] for key in ('name', 'kind', 'seconds')) and
                case['state'] == 'BOUND_REACHED_NOT_QUALIFIED' and
                case['end'] >= case['deadline'], 'Case order/bound/integrity mismatch')
        indexes = case['complete_frames']
        require(len(indexes) >= 2, 'Short case: need two complete frames')
        first, last = frames[indexes[0]], frames[indexes[-1]]
        require(first['received_start'] <= case['start']+10 and last['received_end'] >= case['deadline']-10,
                'Missing fresh report coverage at case edges')
        window = raw[first['start']:last['end']].decode()
        summary = numerical(window, spec)
        summaries.append(dict(name=spec['name'], raw_sha256=case['raw_sha256'], **summary))
    return dict(status='CONTINUOUS_SOURCE_OBSERVATION_NOT_PRODUCT_QUALIFIED', continuation_pass=True,
                raw_sha256=result['raw_sha256'], events_sha256=result['events_sha256'],
                cases=summaries, completed_cases=len(summaries), app_sha256=APP, elf_sha256=ELF,
                physical_acceptance='NOT_ASSESSED', wall_includes_wait_preemption=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--capture', required=True, type=Path)
    parser.add_argument('--contract', required=True, type=Path)
    parser.add_argument('--contract-sha256', required=True)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    contract = load_contract(args.contract, args.contract_sha256)
    result = analyze(args.capture, contract)
    with args.output.open('x', encoding='utf-8') as output:
        json.dump(result, output, indent=2)
        output.write('\n')
    print(json.dumps(dict(status=result['status'], completed_cases=result['completed_cases'])))


if __name__ == '__main__':
    main()
