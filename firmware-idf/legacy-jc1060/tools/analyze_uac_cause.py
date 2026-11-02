"""Fail-closed attribution of raw UAC cause-profile logs; never CPU-only time."""
import argparse
import hashlib
import json
from pathlib import Path
import re

STAGES = ('init', 'transport', 'control', 'graph', 'progress', 'copy', 'pack', 'other')
MASK = 2**32 - 1


def delta(after, before):
    return (int(after) - int(before)) & MASK


def words(fields, names):
    """Require complete unsigned scalar fields, including on truncated lines."""
    values = [fields[name] for name in names.split()]
    return all(re.fullmatch(r'[0-9]+', v) and int(v) <= MASK for v in values)


def analyze(data):
    groups, calibrations, irq_calibrations, distributions, histograms = {}, {}, {}, {}, {}
    partial = 0

    def insert(target, key, value):
        if key in target and target[key] != value:
            raise ValueError('Conflicting coherent identity: ' + repr(key))
        target[key] = value

    for line in data.decode('utf-8', errors='replace').splitlines():
        if not line.startswith('uac_profile'):
            continue
        prefix = line.split()[0]
        f = dict(re.findall(r'\b(\w+)=([^\s]+)', line))
        try:
            if prefix == 'uac_profile_calibration':
                insert(calibrations, f['gen'], f)
            elif prefix == 'uac_profile_irq_calibration':
                insert(irq_calibrations, f['gen'], f)
            elif prefix == 'uac_profile_distribution':
                insert(distributions, (f['gen'], f['samples']), f)
            elif prefix == 'uac_profile_hist':
                insert(histograms.setdefault((f['gen'], f['samples']), {}), f['stage'], f)
            elif prefix in ('uac_profile', 'uac_profile_stage', 'uac_profile_cause'):
                key = (f['kind'], f['gen'], f['ordinal'])
                g = groups.setdefault(key, {'stages': {}})
                if prefix == 'uac_profile_stage':
                    insert(g['stages'], f['stage'], f)
                else:
                    insert(g, 'record' if prefix == 'uac_profile' else 'cause', f)
        except KeyError:
            partial += 1
    results = []
    for (kind, gen, ordinal), g in groups.items():
        r, c, stages = g.get('record', {}), g.get('cause', {}), g['stages']
        if r.get('seen') != '1':
            continue
        item = dict(kind=kind, generation=gen, ordinal=ordinal, raw=g,
                    partition_verified=False, scheduler_usable=False, irq_dispatch_usable=False)
        try:
            whole = int(r['whole_us'])
            item['partition_verified'] = (r['partition_valid'] == '1' and set(stages) == set(STAGES)
                and sum(int(s['us']) for s in stages.values()) == int(r['rust_us'])
                and int(r['rust_us']) + int(r['outer_us']) == whole)
            cal, ic = calibrations.get(gen, {}), irq_calibrations.get(gen, {})
            sched_cal = (cal.get('passed') == '1'
                and words(cal, 'wall_before wall_after switches_before switches_after away_before away_after')
                and cal['before_valid'] == cal['after_valid'] == '1'
                and delta(cal['switches_after'], cal['switches_before']) > 0
                and 0 < delta(cal['away_after'], cal['away_before']) <= delta(cal['wall_after'], cal['wall_before']))
            irq_cal = (ic.get('passed') == '1'
                and words(ic, 'entries_before entries_after us_before us_after nested_before nested_after wall_before wall_after')
                and ic['before_valid'] == ic['after_valid'] == '1'
                and 0 <= delta(ic['nested_after'], ic['nested_before']) < delta(ic['entries_after'], ic['entries_before'])
                and (delta(ic['nested_after'], ic['nested_before']) > 0
                     or delta(ic['us_after'], ic['us_before']) <= delta(ic['wall_after'], ic['wall_before'])))
            absolute = (delta(c['away_after'], c['away_before']) == int(r['away_us'])
                and delta(c['switches_after'], c['switches_before']) == int(r['switches']))
            item['scheduler_usable'] = (item['partition_verified'] and sched_cal and absolute
                and c['calibration'] == r['sched_valid'] == '1'
                and whole <= int(r['sched_window_us']) and int(r['away_us']) <= int(r['sched_window_us']))
            item['irq_dispatch_usable'] = (item['partition_verified'] and irq_cal
                and c['irq_calibration'] == c['irq_valid'] == '1' and int(c['irq_nested']) == 0
                and whole <= int(c['irq_window_us']) and int(c['irq_dispatch_us']) <= int(c['irq_window_us']))
            for valid, window, value, label in (
                (item['scheduler_usable'], r['sched_window_us'], r['away_us'], 'scheduler_away'),
                (item['irq_dispatch_usable'], c['irq_window_us'], c['irq_dispatch_us'], 'irq_dispatch')):
                if valid:
                    padding = int(window) - whole
                    item[label + '_inside_whole_bounds_us'] = [max(0, int(value)-padding), min(whole, int(value))]
        except (KeyError, ValueError):
            item['incomplete_or_malformed'] = True
        results.append(item)
    snapshots = []
    for key, d in distributions.items():
        h = histograms.get(key, {})
        item = dict(generation=key[0], samples=key[1], complete=False, raw=dict(distribution=d, histograms=h))
        try:
            samples, valid = int(d['samples']), int(d['valid'])
            bins = {name: [int(v) for v in row['bins'].split(',')] for name, row in h.items()}
            item['complete'] = (set(bins) == {'whole', *STAGES} and 0 <= valid <= samples < MASK
                and all(len(v) == 7 and all(0 <= n < MASK for n in v) for v in bins.values())
                and sum(bins['whole']) == samples and all(sum(bins[n]) == valid for n in STAGES))
        except (KeyError, ValueError):
            pass
        snapshots.append(item)
    return dict(status='ANALYZED_NOT_QUALIFIED', log_sha256=hashlib.sha256(data).hexdigest(),
                bytes=len(data), partial_lines=partial, records=results, distributions=snapshots,
                policy='Only calibrated valid windows permit bounded attribution. IRQ covers core-0 C dispatch only; nested samples are unusable. Do not subtract scheduler and IRQ intervals or call the residual CPU time.')


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('--log', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        raise ValueError('Fresh analysis output required')
    args.output.write_text(json.dumps(analyze(args.log.read_bytes()), indent=2) + '\n')


if __name__ == '__main__':
    main()
