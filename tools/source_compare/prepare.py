"""Offline matched-control layout proposal. There is no device/execute path."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SOURCE = 'a2453350624bd6c207a921850354c5df4dd23316'
UNIT = 'JC1060-20261030-COM3'
MAC = 'e8:f6:0a:e7:22:1d'
FULL = '28830827d3e3426bbcd58b9a25d4d2129e28f150bade33fe3c551cd577f93921'
S07_APP = '95d45203ec245a9f82455f16f15dd95ed6ad2bb8eaaca9cf85f0816ccd5dbfdd'
S07_ELF = '005f8d1e33aa5f2a8d05af691ce768461f559c917b8804920a912178f5b78ddf'
PLANNER_SHA = '13bde13de4fd4a6f62fdc94147c242945696f1615286c2afccbc91416a9c6543'
HISTORY = 'docs/reviews/evidence/2026-11-03-continuous-device/executed-history-44.json'
HISTORY_SHA = '9a8ed60c214c220ced61fc81498f65ecc1ffd0a03f38b7e21e2170db0f2b0108'
RUNBOOK = 'docs/reviews/2026-11-03-source-compare-runbook.md'
ROOT = Path('D:/BoardEvidence/JC1060-20261030-COM3/reset-inplace-20261103')
ACCEPTED = Path('D:/BoardEvidence/JC1060-20261030-COM3/s07-deck-play-build-1')
PACKAGES = {
    'control': ('clean-producer-control-2', 'd8c5f65c0b4ef646490ba8d63298d408a28ea01deb689a0eb0499575a0cead37',
                '2535f9ccec7a0ae1fb5b1df4a19330050d790317330d35ae4b6618346be96b5d',
                '86f6f47d668452811ea925dca17ae48adb95872511d9202e4be22c87d285ab4a'),
    'opt3': ('clean-producer-opt3', 'a583bc4ecd543dd4a97fab90d19da0a5b816abb0ed2119bd26e2a977b23921c2',
             '2fc3180eac6aae781f529589d6fe695b2ba3b72b5d5fd0e44c4a35d3401d44a7',
             '5c81ce8c281c96030242957063dc7f46d5d364d40dae491330e880bf39b53c0f'),
}
LIMITS = {0x2000: 0x8000, 0x8000: 0x9000, 0x10000: 0x110000}
POLICY = dict(chip='esp32p4', before='no-reset', after='no-reset', flash_mode='keep',
              flash_freq='keep', flash_size='keep', erase_all=False, force=False)


def require(value, message):
    if not value:
        raise ValueError(message)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def contained(root, relative):
    path = (Path(root)/relative).resolve()
    require(path.is_relative_to(Path(root).resolve()), 'Path escaped root')
    return path


def checked(path, digest):
    raw = Path(path).read_bytes()
    require(sha(raw) == digest, 'Digest mismatch: '+str(path))
    return raw


def save(path, value):
    with Path(path).open('x', encoding='utf-8') as output:
        output.write(json.dumps(value, indent=2)+'\n')


def replay(original, history, unit=UNIT):
    """Validate payloads AND their saved pre-write spans, oldest first."""
    state, link, verified = bytearray(original), None, []
    for entry in history:
        path = Path(entry['path']).resolve()
        plan = json.loads(checked(path, entry['sha256']))
        require(plan['unit_id'] == unit and plan['factory_sha256'] == sha(original), 'Wrong ancestry unit/factory')
        require(plan.get('predecessor_plan_sha256') == link, 'Incomplete/reordered ancestry')
        # The first historical plans predate the embedded chain field. Their
        # predecessor hash still binds every link; present embedded chains must match.
        require('predecessor_chain' not in plan or plan['predecessor_chain'] == verified,
                'Incomplete/reordered embedded ancestry')
        require(plan['command_policy'] == POLICY, 'Historical command drift')
        seen = set()
        for row in plan['writes']:
            start, end = row['offset'], row['erase_end']
            require(start in LIMITS and start not in seen and start % 4096 == 0 and
                    start < end <= min(LIMITS[start], len(state)) and end % 4096 == 0, 'Invalid historical range')
            seen.add(start)
            payload = checked(contained(path.parent, row['staged_file']), row['staged_sha256'])
            restore = checked(contained(path.parent, row['restore_file']), row['restore_sha256'])
            require(len(payload) == end-start and restore == state[start:end], 'Historical restore mismatch')
            require(0 < row['bytes'] <= len(payload) and payload[row['bytes']:] == state[start+row['bytes']:end],
                    'Historical sector tail drift')
            state[start:end] = payload
        require(seen == set(LIMITS), 'Incomplete historical spans')
        link = entry['sha256']
        verified.append(dict(path=str(path), sha256=link))
    return bytes(state), verified


def exact_return(candidate_plan, candidate_path, baseline, current, output, history, accepted_manifest_sha):
    """Restore every candidate erase span, including bytes past s07's image."""
    output = Path(output)
    require(len(baseline) == len(current), 'Return full-state size mismatch')
    require(candidate_plan['command_policy'] == POLICY, 'Candidate command drift')
    require(candidate_plan['predecessor_chain'] == history, 'Candidate ancestry drift')
    output.mkdir(parents=True, exist_ok=False)
    rows, after, seen = [], bytearray(current), set()
    for source in candidate_plan['writes']:
        start, end = source['offset'], source['erase_end']
        require(start in LIMITS and start not in seen and start < end <= min(LIMITS[start], len(current))
                and end % 4096 == 0, 'Return range drift')
        seen.add(start)
        require(checked(contained(candidate_path.parent, source['restore_file']), source['restore_sha256']) == baseline[start:end],
                'Candidate saved baseline mismatch')
        require(checked(contained(candidate_path.parent, source['staged_file']), source['staged_sha256']) == current[start:end],
                'Candidate staged/full mismatch')
        payload, restore = baseline[start:end], current[start:end]
        name, undo = f'write-{start:08x}.bin', f'restore-{start:08x}.bin'
        (output/name).write_bytes(payload)
        (output/undo).write_bytes(restore)
        # bytes is the entire saved sector snapshot, not an s07 artifact size.
        rows.append(dict(offset=start, bytes=len(payload), erase_end=end, payload_kind='BASELINE_SECTOR_SNAPSHOT',
                         staged_file=name, staged_sha256=sha(payload), restore_file=undo, restore_sha256=sha(restore)))
        after[start:end] = payload
    require(seen == set(LIMITS) and bytes(after) == baseline, 'Exact full return failed')
    candidate_sha = sha(candidate_path.read_bytes())
    chain = [*history, dict(path=str(candidate_path.resolve()), sha256=candidate_sha)]
    plan = dict(schema=1, status='LAYOUT_ONLY_UNBOUND_NOT_EXECUTABLE', device_access=False,
                unit_id=UNIT, factory_sha256=candidate_plan['factory_sha256'], manifest_sha256=accepted_manifest_sha,
                predecessor_plan_sha256=candidate_sha, predecessor_chain=chain, command_policy=POLICY,
                writes=rows, rollback_of_plan_sha256=candidate_sha, rollback_target_full_sha256=sha(baseline),
                expected_before_sha256=sha(current), expected_after_sha256=sha(baseline),
                execution='NOT_RUN; no execution adapter or binding authorizes this sector-snapshot return')
    save(output/'plan.json', plan)
    return plan


def load_planner(repo):
    path = repo/'firmware-idf/legacy-jc1060/tools/prepare_flash_plan.py'
    checked(path, PLANNER_SHA)
    spec = importlib.util.spec_from_file_location('paired_layout_planner', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def proof(entry, predecessor, plan_sha):
    path = Path(entry['path'])
    result = json.loads(checked(path, entry['sha256']))
    require(result['status'] == 'WRITTEN_FULL_READBACK_MATCH_ROM_HELD' and
            result['predecessors'] == predecessor and result['plan_sha256'] == plan_sha and
            result['full_predecessor_match'] is True and result['reset_requested'] is False and
            result['efuse_write'] is False, 'Executed proof drift')
    identity = result['identity']
    require((identity['chip'], identity['revision'], identity['mac'], identity['rdid']) ==
            ('ESP32-P4', 103, MAC, 0x184068) and identity['security']['flags'] ==
            identity['security']['flash_crypt_cnt'] == 0, 'Prior identity/security drift')
    require(set(result['captures']) == {'prewrite-full.bin', 'postwrite-full.bin',
            'readback-00002000.bin', 'readback-00008000.bin', 'readback-00010000.bin'}, 'Incomplete prior captures')
    before = checked(path.parent/'prewrite-full.bin', result['before_sha256'])
    after = checked(path.parent/'postwrite-full.bin', result['after_sha256'])
    require(len(before) == len(after) == 0x1000000 and result['postwrite_sha256'] == sha(after), 'Prior full proof mismatch')
    for name, row in result['captures'].items():
        raw = checked(contained(path.parent, name), row['sha256'])
        require(len(raw) == row['bytes'], 'Prior capture length mismatch')
        if name.startswith('readback-'):
            offset = int(name[9:17], 16)
            require(raw == after[offset:offset+len(raw)], 'Prior span/full mismatch')
    return before, after


def package(repo, leg):
    folder, digest, app_sha, elf_sha = PACKAGES[leg]
    path = ROOT/folder
    manifest = json.loads(checked(path/'manifest.json', digest))
    require(manifest['source_commit'] == SOURCE and not manifest['dirty_status'] and
            manifest['variant'] == 'deck-engine' and manifest['producer_profile'] is True and
            manifest['uac_profile'] is False and manifest['uac_pack_iram'] is False and
            manifest['mp3_core_opt3'] is (leg == 'opt3'), 'Wrong matched profile/source')
    require(manifest['rust_release_overrides'] == ({'nanomp3-core': {'opt-level': 3}} if leg == 'opt3' else {}),
            'Wrong compiler override')
    require(len(manifest['sources']) == 313, 'Source inventory changed')
    for name, expected in manifest['sources'].items():checked(contained(repo, name), expected)
    artifacts = {name: checked(contained(path, name), expected) for name, expected in manifest['artifacts'].items()}
    app = artifacts['build/ruzina_legacy_probe.bin']
    require(sha(app) == app_sha == manifest['sha256'] and len(app) == manifest['bytes'] and
            sha(artifacts['build/ruzina_legacy_probe.elf']) == elf_sha and app[0xb0:0xd0].hex() == elf_sha,
            'Matched image/ELF drift')
    return path, manifest


def prepare(root, repo=REPO):
    root, repo = Path(root).resolve(), Path(repo).resolve()
    require(not root.exists(), 'Fresh proposal directory required')
    history_doc = json.loads(checked(repo/HISTORY, HISTORY_SHA))
    history = history_doc['executed_history']
    require(history_doc['current_executed_predecessors'] == len(history) == 44 and
            history_doc['full_sha256'] == FULL, 'Need exact 44-plan executed prefix')
    module = load_planner(repo)
    unit_path = repo/'docs/boards/readiness/jc1060-20261030-com3.json'
    unit = json.loads(unit_path.read_bytes())
    require(unit['unit_id'] == UNIT and unit['observed']['chip_revision'] == 103 and
            unit['observed']['base_mac'] == MAC, 'Wrong factory unit')
    backups = [Path(unit['factory'][key]) for key in ('read1_file', 'read2_file')]
    backups += [Path(unit['factory']['secondary_storage_copy']['path'])/p.name for p in backups[:]]
    require(len({p.resolve() for p in backups}) == 4, 'Four distinct backups required')
    factory = checked(backups[0], module.BACKUP_SHA)
    require(len(factory) == 0x1000000, 'Need full 16 MiB factory')
    for path in backups[1:]:require(checked(path, module.BACKUP_SHA) == factory, 'Backup mismatch')
    baseline, verified = replay(factory, history)
    require(sha(baseline) == FULL and verified == history, 'Executed baseline mismatch')
    proofs = history_doc['new_execution_proofs']
    require(len(proofs) == 2, 'Need last two executed proofs')
    before42, after42 = proof(proofs[0], 42, history[-2]['sha256'])
    before43, after43 = proof(proofs[1], 43, history[-1]['sha256'])
    require(after42 == before43 and after43 == baseline and replay(factory, history[:-2])[0] == before42,
            'Executed proof chain mismatch')
    accepted_digest = '09ff098c6cc97681b68322125e5c025b1be845b376b8aa1806ac896d65703752'
    accepted = json.loads(checked(ACCEPTED/'manifest.json', accepted_digest))
    require(accepted['variant'] == 'deck-play' and not accepted['dirty_status'], 'Wrong accepted package')
    for name, digest in accepted['artifacts'].items():checked(contained(ACCEPTED, name), digest)
    accepted_app = checked(ACCEPTED/'build/ruzina_legacy_probe.bin', S07_APP)
    require(accepted['sha256'] == S07_APP and baseline[0x10000:0x10000+len(accepted_app)] == accepted_app and
            baseline[0x100b0:0x100d0].hex() == S07_ELF, 'Baseline does not contain accepted s07')
    packages = {leg: package(repo, leg) for leg in PACKAGES}
    runbook = (repo/RUNBOOK).read_bytes()
    recovery = (repo/'docs/RECOVERY_POLICY.md').read_bytes()
    root.mkdir(parents=True, exist_ok=False)
    chain, entries = list(history), []
    for leg in ('control', 'opt3'):
        folder, manifest = packages[leg]
        plan_dir = root/(leg+'-plan')
        module.prepare(folder, unit_path, plan_dir, Path(chain[-1]['path']), [Path(e['path']) for e in chain[:-1]])
        plan_path = plan_dir/'plan.json'
        plan = json.loads(plan_path.read_bytes())
        plan.update(status='LAYOUT_ONLY_UNBOUND_NOT_EXECUTABLE', execution='NOT_RUN')
        # No runnable command templates are published by this proposal tool.
        plan.pop('command_templates_not_executed', None)
        plan['remaining_gates'] = ['Firmware and tool review/CI', 'Separate write/capture/boot contracts and bindings',
                                   'Dual-copy publication and fresh live identity/full-before equality']
        plan_path.write_text(json.dumps(plan, indent=2)+'\n')
        candidate_entry = dict(path=str(plan_path), sha256=sha(plan_path.read_bytes()))
        current, _ = replay(factory, [*chain, candidate_entry])
        return_dir = root/(leg+'-return-s07-plan')
        rollback = exact_return(plan, plan_path, baseline, current, return_dir, chain, accepted_digest)
        return_path = return_dir/'plan.json'
        return_entry = dict(path=str(return_path), sha256=sha(return_path.read_bytes()))
        restored, _ = replay(factory, [*chain, candidate_entry, return_entry])
        require(restored == baseline, 'Return failed independent history replay')
        # Demonstrate why simply writing the smaller s07 image is insufficient.
        ordinary = bytearray(current)
        image_bytes = {0x2000: (ACCEPTED/'build/bootloader/bootloader.bin').read_bytes(),
                       0x8000: (ACCEPTED/'build/partition_table/partition-table.bin').read_bytes(),
                       0x10000: accepted_app}
        for offset, image in image_bytes.items():ordinary[offset:offset+len(image)] = image
        entries.append(dict(leg=leg, candidate_predecessors=len(chain), return_predecessors=len(chain)+1,
                            package=str(folder), manifest_sha256=sha((folder/'manifest.json').read_bytes()),
                            app_sha256=manifest['sha256'], elf_sha256=manifest['artifacts']['build/ruzina_legacy_probe.elf'],
                            candidate=candidate_entry, return_s07=return_entry, before_sha256=FULL,
                            candidate_full_sha256=sha(current), return_full_sha256=sha(restored),
                            ordinary_s07_rewrite_full_sha256=sha(ordinary),
                            ordinary_s07_rewrite_restores_full_baseline=bytes(ordinary) == baseline,
                            rollback_snapshot_bytes=sum(r['bytes'] for r in rollback['writes']), execution='NOT_RUN'))
        chain += [candidate_entry, return_entry]
    result = dict(status='OFFLINE_PAIRED_LAYOUT_PROPOSAL_NOT_EXECUTABLE', device_access=False, native_minimum=302,
                  unit_id=UNIT, source_commit=SOURCE, executed_predecessors=44, current_full_sha256=FULL,
                  proposed_transition_predecessors=[44, 45, 46, 47], proposed_final_history_count=48,
                  executed_history=history, executed_history_authority_sha256=HISTORY_SHA,
                  runbook=dict(path=RUNBOOK, sha256=sha(runbook)), recovery_policy_sha256=sha(recovery),
                  entries=entries, execution_bindings='NOT_CREATED', live_identity='NOT_REFRESHED',
                  firmware_legacy_ci='NOT_YET_CONFIRMED_PASS', tool_review_ci='REQUIRED',
                  manual_s07_requalification=False, physical_acceptance='NOT_RUN')
    save(root/'proposal.json', result)
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', required=True, type=Path)
    args = parser.parse_args()
    result = prepare(args.root)
    print(json.dumps({key: result[key] for key in ('status', 'executed_predecessors', 'proposed_transition_predecessors',
                                                 'proposed_final_history_count', 'device_access')}))
