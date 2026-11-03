"""Offline sector-level write/recovery plan. Never opens a device or runs esptool."""
import argparse
import hashlib
import json
from pathlib import Path
import struct

FLASH_BYTES = 0x1000000
BACKUP_SHA = '27d5782978aeb3c42f8b49659e793d3ca8f59543f62e419d7b1981de2c492060'


def sha(data):
    return hashlib.sha256(data).hexdigest()


def partitions(data, offset=0):
    rows = []
    pos = offset
    while data[pos:pos + 2] == b'\xaa\x50':
        record = data[pos:pos + 32]
        if len(record) != 32:
            raise ValueError('Truncated partition record')
        _, typ, sub, start, size, label, flags = struct.unpack('<HBBII16sI', record)
        if start < 0x9000 or size <= 0 or start + size > FLASH_BYTES:
            raise ValueError('Partition bounds')
        if any(start < r['offset'] + r['bytes'] and r['offset'] < start + size for r in rows):
            raise ValueError('Partition overlap')
        rows.append(dict(type=typ, subtype=sub, offset=start, bytes=size,
                         label=label.rstrip(b'\0').decode('ascii'), flags=flags))
        pos += 32
    if not rows or data[pos:pos + 16] != b'\xeb\xeb' + b'\xff' * 14:
        raise ValueError('Missing partition MD5 record')
    if hashlib.md5(data[offset:pos]).digest() != data[pos + 16:pos + 32]:
        raise ValueError('Partition MD5 mismatch')
    return rows


def overlay_history(factory, unit_id, plans):
    """Replay a complete hash-linked history; smaller images leave older tails."""
    baseline=bytearray(factory)
    previous_hash=None
    chain=[]
    for path in plans:
        path=Path(path)
        raw=path.read_bytes()
        previous=json.loads(raw)
        if previous['factory_sha256']!=BACKUP_SHA or previous['unit_id']!=unit_id:
            raise ValueError('Predecessor unit/backup mismatch')
        if previous.get('predecessor_plan_sha256')!=previous_hash:
            raise ValueError('Incomplete or unordered predecessor chain')
        ranges=[]
        for row in previous['writes']:
            payload_path=(path.parent/row['staged_file']).resolve()
            if not payload_path.is_relative_to(path.parent.resolve()):
                raise ValueError('Escaped predecessor payload')
            payload=payload_path.read_bytes()
            start,end=row['offset'],row['erase_end']
            if sha(payload)!=row['staged_sha256'] or len(payload)!=end-start:
                raise ValueError('Predecessor payload hash/length mismatch')
            if start<0 or start>=end or end>len(baseline):
                raise ValueError('Predecessor bounds')
            if any(start<b and a<end for a,b in ranges):
                raise ValueError('Overlapping predecessor writes')
            ranges.append((start,end))
            baseline[start:end]=payload
        previous_hash=sha(raw)
        chain.append(dict(path=str(path.resolve()),sha256=previous_hash))
    return baseline,previous_hash,chain


def prepare(package, unit_path, output, predecessor_plan=None, ancestor_plans=()):
    package, unit_path, output = map(Path, (package, unit_path, output))
    manifest_path = package / 'manifest.json'
    manifest = json.loads(manifest_path.read_text())
    if manifest['variant'] not in ('flash-build-only', 'memory-task', 'display-touch', 'sd-readonly', 'sd-file-readonly', 'sd-rust-bridge', 'sd-rust-exfat', 'sd-rust-load', 'sd-rust-load-display', 'usb-host', 'usb-midi', 'deck-ui', 'deck-audio', 'deck-play', 'deck-mix', 'deck-engine') or manifest['idf_commit'] != 'fff9895c82d744c7237be8847347bdd1b07c6643':
        raise ValueError('Wrong diagnostic package')
    unit = json.loads(unit_path.read_text())
    if unit['observed']['chip_revision'] != 103 or unit['observed']['base_mac'] != 'e8:f6:0a:e7:22:1d':
        raise ValueError('Wrong unit authority')
    for rel, expected in manifest['artifacts'].items():
        candidate = (package / rel).resolve()
        if not candidate.is_relative_to(package.resolve()) or sha(candidate.read_bytes()) != expected:
            raise ValueError('Artifact mismatch or escaped package: ' + rel)
    backups = [Path(unit['factory'][key]).resolve() for key in ('read1_file', 'read2_file')]
    secondary = Path(unit['factory']['secondary_storage_copy']['path'])
    backups += [secondary / b.name for b in backups]
    if backups[0] == backups[1]:
        raise ValueError('Independent backup paths required')
    for backup in backups:
        data = backup.read_bytes()
        if len(data) != FLASH_BYTES or sha(data) != BACKUP_SHA:
            raise ValueError('Backup length/hash mismatch')
    factory = backups[0].read_bytes()
    old_table = partitions(factory, 0x8000)
    if ancestor_plans and predecessor_plan is None:
        raise ValueError('Ancestors require the current predecessor plan')
    plans=[*ancestor_plans,predecessor_plan] if predecessor_plan is not None else []
    baseline,predecessor_hash,chain=overlay_history(factory,unit['unit_id'],plans)
    args_path = package / 'build/flasher_args.json'
    if 'build/flasher_args.json' not in manifest['artifacts']:
        raise ValueError('Unbound flash arguments')
    args = json.loads(args_path.read_text())
    if set(args['flash_files']) != {'0x2000', '0x8000', '0x10000'}:
        raise ValueError('Unexpected write offsets')
    writes = []
    staged = []
    for address, rel in sorted(args['flash_files'].items(), key=lambda x: int(x[0], 0)):
        path = (package / 'build' / rel).resolve()
        key = 'build/' + rel
        if key not in manifest['artifacts'] or not path.is_relative_to((package / 'build').resolve()):
            raise ValueError('Unbound write artifact')
        payload = path.read_bytes()
        start = int(address, 0)
        end = (start + len(payload) + 4095) // 4096 * 4096
        limits = {0x2000: 0x8000, 0x8000: 0x9000, 0x10000: 0x110000}
        if start % 4096 or not payload or end > limits[start]:
            raise ValueError('Invalid sector span')
        if any(start < r['erase_end'] and r['offset'] < end for r in writes):
            raise ValueError('Write-sector overlap')
        if address != '0x8000':
            from esptool.bin_image import LoadFirmwareImage
            image = LoadFirmwareImage('ESP32-P4', payload)
            if image.chip_id != 18 or (image.min_rev_full, image.max_rev_full) != (100, 199):
                raise ValueError('Wrong image header')
        # Keep bytes beyond the file's end, which esptool's sector erase would lose.
        sector_image = payload + bytes(baseline[start + len(payload):end])
        filename = f'write-{start:08x}.bin'
        restore = bytes(baseline[start:end])
        staged += [(filename, sector_image), (f'restore-{start:08x}.bin', restore)]
        writes.append(dict(offset=start, bytes=len(payload), erase_end=end,
                           staged_file=filename, staged_sha256=sha(sector_image),
                           restore_file=f'restore-{start:08x}.bin', restore_sha256=sha(restore)))
    new_table = partitions((package / 'build/partition_table/partition-table.bin').read_bytes())
    output.mkdir(parents=True, exist_ok=False)
    for filename, payload in staged:
        (output / filename).write_bytes(payload)
    result = dict(schema=1, status='OFFLINE_REVIEWED_OPERATOR_GATES_OPEN', device_access=False,
                  unit_id=unit['unit_id'], manifest_sha256=sha(manifest_path.read_bytes()),
                  factory_sha256=BACKUP_SHA, backup_copies_verified=4,
                  predecessor_plan_sha256=predecessor_hash,
                  predecessor_chain=chain,
                  sector_restore_target='PREDECESSOR_DIAGNOSTIC' if predecessor_hash else 'FACTORY',
                  factory_partitions=old_table, diagnostic_partitions=new_table, writes=writes,
                  command_policy=dict(chip='esp32p4', before='no-reset', after='no-reset',
                                      flash_mode='keep', flash_freq='keep', flash_size='keep',
                                      erase_all=False, force=False),
                  full_restore=dict(offset=0, bytes=FLASH_BYTES, source=str(backups[0]), sha256=BACKUP_SHA),
                  remaining_gates=['Physical connector/power/markings', 'Manual ROM entry evidence',
                                   'Live identity/security recheck', 'Diagnostic image reset telemetry'])
    prefix = ['<PINNED_PYTHON>', '-m', 'esptool', '--chip', 'esp32p4', '--port',
              '<MANUALLY_VERIFIED_PORT>', '--before', 'no-reset', '--after', 'no-reset']
    keep = ['write-flash', '--flash-mode', 'keep', '--flash-size', 'keep', '--flash-freq', 'keep']
    result['command_templates_not_executed'] = dict(
        cwd=str(output.resolve()),
        write=prefix + keep + [arg for r in writes for arg in (hex(r['offset']), r['staged_file'])],
        restore_changed_sectors=prefix + keep + [arg for r in writes for arg in (hex(r['offset']), r['restore_file'])],
        restore_full=prefix + keep + ['0', str(backups[0])],
        readback_full=prefix + ['read-flash', '--flash-size', '16MB', '0', '0x1000000', '<NEW_READBACK_PATH>'])
    (output / 'plan.json').write_text(json.dumps(result, indent=2) + '\n')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('--package', required=True, type=Path)
    parser.add_argument('--unit', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--predecessor-plan', type=Path)
    parser.add_argument('--ancestor-plan', type=Path, action='append', default=[],
                        help='Complete oldest-first ancestry before current predecessor; repeat for each plan')
    parsed = parser.parse_args()
    prepare(parsed.package, parsed.unit, parsed.output, parsed.predecessor_plan, parsed.ancestor_plan)
    print('OFFLINE_WRITE_RECOVERY_PLAN_READY; operator gates open; no device accessed')
