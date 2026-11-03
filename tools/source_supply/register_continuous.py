"""Offline 42/43 plan registration. No serial imports or device actions."""
import argparse
import json
from pathlib import Path

from campaign import (MAC, REGISTRATION_SHA, SOURCE, UNIT_ID, checked, contained,
                      require, sha, validate_candidate_manifest)
from continuous_contract import APP, BEFORE, CASES, ELF, LIMITS, validate
from continuous_write import factory, planner, prepare
from software_boot import RESTORED, S07_APP, S07_ELF, policy

RUNBOOK = 'docs/reviews/2026-11-03-continuous-campaign.md'
RECOVERY = 'docs/RECOVERY_POLICY.md'


def write_json(path, value):
    with Path(path).open('x', encoding='utf-8') as output:
        json.dump(value, output, indent=2)
        output.write('\n')


def prior_proof(previous, old, mode, folder):
    entry = old['transitions'][mode]
    path = previous/folder/'capture/result.json'
    raw = path.read_bytes()
    report = json.loads(raw)
    expected = dict(status='WRITTEN_FULL_READBACK_MATCH_ROM_HELD', mode=mode,
                    registration_sha256=REGISTRATION_SHA, plan_sha256=entry['plan_sha256'],
                    predecessors=entry['predecessors'], before_sha256=entry['expected_before_sha256'],
                    after_sha256=entry['expected_after_sha256'], postwrite_sha256=entry['expected_after_sha256'],
                    full_predecessor_match=True, reset_requested=False, efuse_write=False)
    for key, value in expected.items():require(report.get(key) == value, 'Executed predecessor proof drift: '+key)
    identity = report['identity']
    require((identity['chip'], identity['revision'], identity['mac'], identity['rdid']) ==
            ('ESP32-P4', 103, MAC, 0x184068) and identity['security']['flags'] ==
            identity['security']['flash_crypt_cnt'] == 0, 'Executed proof unit/security mismatch')
    for name, digest in (('prewrite-full.bin', entry['expected_before_sha256']),
                         ('postwrite-full.bin', entry['expected_after_sha256'])):
        require(len(checked(path.parent/name, digest)) == 0x1000000, 'Wrong prior full proof size')
    return dict(path=str(path), sha256=sha(raw), predecessors=entry['predecessors'],
                plan_sha256=entry['plan_sha256'])


def register(root, repo, previous):
    root, repo, previous = Path(root).resolve(), Path(repo).resolve(), Path(previous).resolve()
    require(not root.exists(), 'Fresh registration output required')
    old = json.loads(checked(previous/'campaign-registration.json', REGISTRATION_SHA))
    require(old['unit_id'] == UNIT_ID and old['transitions']['candidate']['predecessors'] == 40 and
            old['transitions']['return-s07']['predecessors'] == 41, 'Wrong completed campaign')
    proofs = [prior_proof(previous, old, mode, folder) for mode, folder in
              (('candidate', 'candidate-execution-1'), ('return-s07', 'return-execution-1'))]
    last_entry = old['transitions']['return-s07']
    last = contained(previous, last_entry['plan_dir'])/'plan.json'
    last_plan = json.loads(checked(last, last_entry['plan_sha256']))
    history = [Path(row['path']) for row in last_plan['predecessor_chain']]+[last]
    require(len(history) == 42 and sha(last.read_bytes()) == proofs[-1]['plan_sha256'], 'Need all 42 executed plans')
    module = planner(repo)
    original = factory(repo, module)
    baseline, _, verified_history = module.overlay_history(original, UNIT_ID, history)
    require(sha(baseline) == BEFORE and last_entry['expected_after_sha256'] == BEFORE,
            'Current full-state reconstruction mismatch')
    candidate = Path(old['transitions']['candidate']['package']).resolve()
    accepted = Path(last_entry['package']).resolve()
    candidate_manifest = json.loads(checked(candidate/'manifest.json', old['transitions']['candidate']['manifest_sha256']))
    validate_candidate_manifest(candidate_manifest)
    require(candidate_manifest['sha256'] == APP, 'Wrong retained candidate')
    for name, digest in candidate_manifest['sources'].items():checked(contained(repo, name), digest)
    accepted_manifest = json.loads(checked(accepted/'manifest.json', last_entry['manifest_sha256']))
    require(accepted_manifest['sha256'] == S07_APP, 'Wrong retained s07')
    runbook = (repo/RUNBOOK).read_bytes()
    recovery = (repo/RECOVERY).read_bytes()
    root.mkdir(parents=True, exist_ok=False)
    unit = repo/'docs/boards/readiness/jc1060-20261030-com3.json'
    module.prepare(candidate, unit, root/'candidate-plan', last, history[:-1])
    module.prepare(accepted, unit, root/'return-s07-plan', root/'candidate-plan/plan.json', history)
    entries = {}
    for mode, folder, package, manifest in (
            ('candidate', 'candidate-plan', candidate, candidate_manifest),
            ('return-s07', 'return-s07-plan', accepted, accepted_manifest)):
        path = root/folder/'plan.json'
        plan = json.loads(path.read_bytes())
        before, _, _ = module.overlay_history(original, UNIT_ID, [row['path'] for row in plan['predecessor_chain']])
        after, _, _ = module.overlay_history(original, UNIT_ID, [row['path'] for row in plan['predecessor_chain']]+[path])
        entries[mode] = dict(plan_dir=folder, package=str(package), manifest_sha256=sha((package/'manifest.json').read_bytes()),
                             plan_sha256=sha(path.read_bytes()), app_sha256=manifest['sha256'],
                             elf_sha256=manifest['artifacts']['build/ruzina_legacy_probe.elf'],
                             predecessors=len(plan['predecessor_chain']), expected_before_sha256=sha(before),
                             expected_after_sha256=sha(after), execution='NOT_RUN')
    require(entries['candidate']['predecessors'] == 42 and entries['return-s07']['predecessors'] == 43 and
            entries['candidate']['expected_after_sha256'] == entries['return-s07']['expected_before_sha256'] and
            entries['return-s07']['expected_after_sha256'] == RESTORED, 'New ancestry/return mismatch')
    reg = dict(schema=1, status='REGISTERED_CONTINUOUS_WRITE_RECOVERY', device_access=False,
               unit_id=UNIT_ID, source_commit=SOURCE, native_minimum=302,
               executed_predecessors_at_registration=42, current_full_sha256=BEFORE,
               transitions=entries, executed_history=verified_history, executed_proofs=proofs,
               runbook=dict(path=RUNBOOK, sha256=sha(runbook)),
               recovery_policy=dict(path=RECOVERY, sha256=sha(recovery)),
               execution='NOT_RUN; needs separate reviewed write/capture/boot bindings and complete CI')
    write_json(root/'campaign-registration.json', reg)
    registration_sha = sha((root/'campaign-registration.json').read_bytes())
    contract = dict(schema=1, status='REGISTERED_CONTINUOUS_SOURCE_CAMPAIGN', unit_id=UNIT_ID,
                    mac='e8:f6:0a:e7:22:1d', chip_revision=103, native_minimum=302,
                    firmware_source=SOURCE, app_sha256=APP, elf_sha256=ELF, candidate_predecessors=42,
                    return_predecessors=43, before_sha256=BEFORE, cases=CASES, resources=LIMITS,
                    write_registration_sha256=registration_sha, candidate_plan_sha256=entries['candidate']['plan_sha256'],
                    return_plan_sha256=entries['return-s07']['plan_sha256'], after_sha256=entries['candidate']['expected_after_sha256'])
    validate(contract)
    write_json(root/'capture-contract.json', contract)
    boot = dict(schema=1, status='REGISTERED_S07_SOFTWARE_BOOT', unit_id=UNIT_ID,
                mac=contract['mac'], chip_revision=103, native_minimum=302, predecessors=43,
                history_after_return=44, app_sha256=S07_APP, elf_sha256=S07_ELF, after_sha256=RESTORED,
                action='ONE_USB_RTS_BOOT_PULSE', manual_requalification=False,
                registration_sha256=registration_sha, return_plan_sha256=entries['return-s07']['plan_sha256'],
                before_sha256=entries['return-s07']['expected_before_sha256'])
    write_json(root/'boot-policy.json', boot)
    policy(root/'boot-policy.json', sha((root/'boot-policy.json').read_bytes()))
    preflights = [prepare(root, repo, mode, registration_sha).summary() for mode in ('candidate', 'return-s07')]
    write_json(root/'offline-preflight.json', preflights)
    summary = dict(status='OFFLINE_REGISTERED_NOT_EXECUTED_OR_BOUND', device_access=False,
                   registration_sha256=registration_sha, capture_contract_sha256=sha((root/'capture-contract.json').read_bytes()),
                   boot_policy_sha256=sha((root/'boot-policy.json').read_bytes()), transitions=entries,
                   source_inputs_verified=len(candidate_manifest['sources']), complete_executed_predecessors=42,
                   four_factory_copies_verified=True, new_write_capture_boot_bindings='NOT_CREATED')
    write_json(root/'registration-summary.json', summary)
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--repo', type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument('--previous', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(register(args.root, args.repo, args.previous)))


if __name__ == '__main__':
    main()
