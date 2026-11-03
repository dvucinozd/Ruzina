"""New 42/43 write adapter; unchanged shared no-reset writer, offline by default."""
import argparse
import contextlib
import importlib.util
import json
from pathlib import Path

from campaign import (EspBackend, FLASH_BYTES, MAC, PLANNER_SHA, Prepared, S07_APP,
                      SOURCE, UNIT_ID, check_port, checked, contained, program, require,
                      sha, validate_candidate_manifest)
from continuous_contract import APP, BEFORE, ELF
from software_boot import RESTORED, S07_ELF

FILES = ('continuous_write.py', 'campaign.py', 'continuous_contract.py', 'software_boot.py')
POLICY = dict(chip='esp32p4', before='no-reset', after='no-reset', flash_mode='keep',
              flash_freq='keep', flash_size='keep', erase_all=False, force=False)


def planner(repo):
    path = Path(repo)/'firmware-idf/legacy-jc1060/tools/prepare_flash_plan.py'
    checked(path, PLANNER_SHA)
    spec = importlib.util.spec_from_file_location('continuous_flash_planner', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def factory(repo, module):
    unit_path = Path(repo)/'docs/boards/readiness/jc1060-20261030-com3.json'
    unit = json.loads(unit_path.read_bytes())
    require(unit['unit_id'] == UNIT_ID and unit['observed']['chip_revision'] == 103 and
            unit['observed']['base_mac'] == MAC, 'Wrong backup unit authority')
    paths = [Path(unit['factory'][key]) for key in ('read1_file', 'read2_file')]
    paths += [Path(unit['factory']['secondary_storage_copy']['path'])/path.name for path in paths[:]]
    require(len({path.resolve() for path in paths}) == 4, 'Need four distinct factory copies')
    raw = checked(paths[0], module.BACKUP_SHA)
    require(len(raw) == FLASH_BYTES, 'Wrong factory size')
    for path in paths[1:]:require(checked(path, module.BACKUP_SHA) == raw, 'Factory copy mismatch')
    return raw


def prepare(root, repo, mode, registration_sha):
    require(mode in ('candidate', 'return-s07'), 'Unknown transition')
    root, repo = Path(root).resolve(), Path(repo).resolve()
    reg = json.loads(checked(root/'campaign-registration.json', registration_sha))
    require(reg['status'] == 'REGISTERED_CONTINUOUS_WRITE_RECOVERY' and reg['unit_id'] == UNIT_ID and
            reg['source_commit'] == SOURCE and reg['native_minimum'] == 302 and
            reg['executed_predecessors_at_registration'] == 42 and
            reg['current_full_sha256'] == BEFORE, 'Wrong write registration')
    for key in ('runbook', 'recovery_policy'):
        checked(contained(repo, reg[key]['path']), reg[key]['sha256'])
    entry = reg['transitions'][mode]
    expected_app, expected_elf, count, before_hash, after_hash = (
        (APP, ELF, 42, BEFORE, reg['transitions']['candidate']['expected_after_sha256']) if mode == 'candidate'
        else (S07_APP, S07_ELF, 43, reg['transitions']['candidate']['expected_after_sha256'], RESTORED))
    require(entry['app_sha256'] == expected_app and entry['elf_sha256'] == expected_elf and
            entry['predecessors'] == count and entry['expected_before_sha256'] == before_hash and
            entry['expected_after_sha256'] == after_hash, 'Transition policy mismatch')
    plan_dir = contained(root, entry['plan_dir'])
    plan = json.loads(checked(plan_dir/'plan.json', entry['plan_sha256']))
    require(plan['unit_id'] == UNIT_ID and plan['command_policy'] == POLICY, 'Plan unit/command drift')
    package = Path(entry['package']).resolve()
    manifest_raw = checked(package/'manifest.json', entry['manifest_sha256'])
    manifest = json.loads(manifest_raw)
    require(plan['manifest_sha256'] == sha(manifest_raw) and manifest['sha256'] == expected_app, 'Manifest mismatch')
    artifacts = {name:checked(contained(package, name), digest) for name, digest in manifest['artifacts'].items()}
    app = artifacts['build/ruzina_legacy_probe.bin']
    require(sha(app) == expected_app and sha(artifacts['build/ruzina_legacy_probe.elf']) == expected_elf and
            app[0xb0:0xd0].hex() == expected_elf, 'App/ELF mismatch')
    if mode == 'candidate':
        validate_candidate_manifest(manifest)
        for name, digest in manifest['sources'].items():checked(contained(repo, name), digest)
    else:
        require(manifest['variant'] == 'deck-play' and not manifest['dirty_status'], 'Wrong accepted image')
    module = planner(repo)
    original = factory(repo, module)
    before, link, history = module.overlay_history(original, UNIT_ID, [row['path'] for row in plan['predecessor_chain']])
    require(len(history) == count and history == plan['predecessor_chain'] and
            link == plan['predecessor_plan_sha256'] and sha(before) == before_hash, 'Ancestry/full-state drift')
    require(len(reg['executed_history']) == 42 and history[:42] == reg['executed_history'], 'Executed prefix drift')
    if mode == 'return-s07':
        candidate_path = contained(root, reg['transitions']['candidate']['plan_dir'])/'plan.json'
        checked(candidate_path, reg['transitions']['candidate']['plan_sha256'])
        require(history[-1] == dict(path=str(candidate_path.resolve()),
                                   sha256=reg['transitions']['candidate']['plan_sha256']), 'Return lacks registered candidate')
    # Validate restore/tail bytes for every executed predecessor as well.
    state = bytearray(original)
    for ancestor in plan['predecessor_chain']:
        path = Path(ancestor['path'])
        ancestor_plan = json.loads(checked(path, ancestor['sha256']))
        for row in ancestor_plan['writes']:
            start, end = row['offset'], row['erase_end']
            payload = checked(contained(path.parent, row['staged_file']), row['staged_sha256'])
            require(checked(contained(path.parent, row['restore_file']), row['restore_sha256']) == state[start:end],
                    'Historical restore drift')
            require(payload[row['bytes']:] == state[start+row['bytes']:end], 'Historical tail drift')
            state[start:end] = payload
    require(state == before, 'Independent ancestry replay mismatch')
    args = json.loads(artifacts['build/flasher_args.json'])
    require(set(args['flash_files']) == {'0x2000', '0x8000', '0x10000'}, 'Wrong write artifacts')
    images = {int(address, 0):artifacts['build/'+name] for address, name in args['flash_files'].items()}
    after, writes = bytearray(before), []
    for row in plan['writes']:
        start, end = row['offset'], row['erase_end']
        require(start in images and start not in [offset for offset, _ in writes], 'Unexpected/duplicate span')
        image = images[start]
        expected_end = (start+len(image)+4095)//4096*4096
        require(row['bytes'] == len(image) and end == expected_end and
                end <= {0x2000:0x8000, 0x8000:0x9000, 0x10000:0x110000}[start], 'Span bounds/image size drift')
        payload = checked(contained(plan_dir, row['staged_file']), row['staged_sha256'])
        restore = checked(contained(plan_dir, row['restore_file']), row['restore_sha256'])
        require(restore == before[start:end] and payload == image+before[start+len(image):end],
                'Payload/restore/tail mismatch')
        after[start:end] = payload
        writes.append((start, payload))
    require(len(writes) == 3 and sha(after) == after_hash, 'Incomplete/full-after mismatch')
    return Prepared(mode, registration_sha, entry['plan_sha256'], expected_app, expected_elf,
                    count, bytes(before), bytes(after), tuple(writes))


def binding(path, digest, registration_sha):
    bound = json.loads(checked(path, digest))
    require(bound['status'] == 'REVIEWED_CONTINUOUS_WRITE_TOOLS' and
            bound['registration_sha256'] == registration_sha and set(bound['tools']) == set(FILES), 'Wrong write binding')
    for name, field in bound['tools'].items():checked(Path(__file__).parent/name, field)
    checked(Path(__file__).resolve().parents[1]/'resource_readiness/campaign.py', bound['shared_guard_sha256'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--repo', type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument('--registration-sha256', required=True)
    parser.add_argument('--mode', choices=('candidate', 'return-s07'), required=True)
    parser.add_argument('--binding', type=Path)
    parser.add_argument('--binding-sha256')
    parser.add_argument('--output', type=Path)
    parser.add_argument('--execute', action='store_true')
    args = parser.parse_args()
    prepared = prepare(args.root, args.repo, args.mode, args.registration_sha256)
    if not args.execute:
        print(json.dumps(prepared.summary()))
        return
    require(args.binding and args.binding_sha256 and args.output, 'Execution requires separate write binding and fresh output')
    binding(args.binding, args.binding_sha256, args.registration_sha256)
    args.output.mkdir(parents=True, exist_ok=False)
    from serial.tools import list_ports
    check_port('COM3', list_ports.comports())
    with (args.output/'programmer.log').open('x', encoding='utf-8') as log:
        with contextlib.redirect_stdout(log), contextlib.redirect_stderr(log):
            result = program(prepared, EspBackend('COM3'), args.output/'capture')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
