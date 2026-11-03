"""Fixed board/image/budget policy for a future 42/43 continuous campaign."""
import json
from pathlib import Path
import re

from campaign import MAC, SOURCE, UNIT_ID, checked, require, sha

APP = '173933e100e2de34257beb89013556b0937e8b0e94bd36bf797eb755d505d1cd'
ELF = '54dbdc424791bfc3f3516dd194487bd0a09e009bf7215e380c7a57b10d58af11'
BEFORE = '28830827d3e3426bbcd58b9a25d4d2129e28f150bade33fe3c551cd577f93921'
LIMITS = dict(usb_stack_min=2048, sd_stack_min=8192, logger_stack_min=1024,
              internal_heap_min=65536, psram_heap_min=8388608,
              observed_internal_largest_min=8192, observed_psram_largest_min=262144)
CASES = [dict(name=name, kind=kind, seconds=seconds, playing=list(playing)) for name, kind, seconds, playing in (
    ('cold-1', 'cold', 45, (0, 0)), ('idle-1', 'idle', 60, (0, 0)),
    ('s03-deck1', 'functional', 180, (1, 0)), ('s04-deck2', 'functional', 180, (0, 1)),
    ('s05-both', 'both', 60, (1, 1)))]
FILES = ('continuous.py', 'continuous_contract.py', 'continuous_adapter.py', 'continuous_analyze.py',
         'campaign.py', 'analyze.py', 'monitor.py', 'program.py')


def validate(contract):
    require(contract.get('status') == 'REGISTERED_CONTINUOUS_SOURCE_CAMPAIGN', 'Unregistered capture contract')
    expected = dict(schema=1, unit_id=UNIT_ID, mac=MAC, chip_revision=103, native_minimum=302,
                    firmware_source=SOURCE, app_sha256=APP, elf_sha256=ELF,
                    candidate_predecessors=42, return_predecessors=43,
                    before_sha256=BEFORE, cases=CASES, resources=LIMITS)
    for key, value in expected.items():
        require(contract.get(key) == value, 'Contract policy mismatch: ' + key)
    for key in ('write_registration_sha256', 'candidate_plan_sha256', 'return_plan_sha256', 'after_sha256'):
        require(isinstance(contract.get(key), str) and re.fullmatch(r'[0-9a-f]{64}', contract[key]),
                'Missing contract digest: ' + key)
    consumed = (
        'fe406eb3003c05c965eaff014ee73a6b6cf773b1b834585764bb56bc4e3f4650',
        'cd76f6c0611a9c587f25b7f02352fa81805a961a99e885c415287898c9d1f85d')
    require(contract['candidate_plan_sha256'] not in consumed and contract['return_plan_sha256'] not in consumed and
            contract['candidate_plan_sha256'] != contract['return_plan_sha256'], 'Consumed/duplicate plan')
    return contract


def load_contract(path, digest):
    return validate(json.loads(checked(path, digest)))


def verify_binding(path, digest, contract_sha):
    bound = json.loads(checked(path, digest))
    require(bound.get('status') == 'REVIEWED_CONTINUOUS_CAPTURE_TOOLS' and
            bound.get('contract_sha256') == contract_sha, 'Binding does not authorize this contract/core')
    require(set(bound['tools']) == set(FILES), 'Incomplete continuous dependency binding')
    for name in FILES:
        checked(Path(__file__).parent/name, bound['tools'][name])
    shared = Path(__file__).resolve().parents[1]/'resource_readiness/campaign.py'
    checked(shared, bound['shared_guard_sha256'])
    return bound


def verify_program(folder, digest, contract):
    """Prove a completed new candidate write from retained bytes, without a port."""
    folder = Path(folder)
    report = json.loads(checked(folder/'result.json', digest))
    expected = dict(status='WRITTEN_FULL_READBACK_MATCH_ROM_HELD', mode='candidate',
                    registration_sha256=contract['write_registration_sha256'],
                    plan_sha256=contract['candidate_plan_sha256'], predecessors=42,
                    app_sha256=APP, elf_sha256=ELF, before_sha256=BEFORE,
                    after_sha256=contract['after_sha256'], postwrite_sha256=contract['after_sha256'],
                    full_predecessor_match=True, reset_requested=False, efuse_write=False)
    for key, value in expected.items():
        require(report.get(key) == value, 'Candidate programmer proof mismatch: '+key)
    identity = report['identity']
    require((identity['chip'], identity['revision'], identity['mac'], identity['rdid']) ==
            ('ESP32-P4', 103, MAC, 0x184068), 'Wrong programmed unit')
    require(identity['security']['flags'] == identity['security']['flash_crypt_cnt'] == 0, 'Security drift')
    before = checked(folder/'prewrite-full.bin', BEFORE)
    after = checked(folder/'postwrite-full.bin', contract['after_sha256'])
    require(len(before) == len(after) == 0x1000000, 'Need full 16 MiB proof')
    require(set(report['captures']) == {'prewrite-full.bin', 'postwrite-full.bin',
            'readback-00002000.bin', 'readback-00008000.bin', 'readback-00010000.bin'}, 'Incomplete span proof')
    for name, entry in report['captures'].items():
        require('/' not in name and '\\' not in name, 'Unsafe proof filename')
        raw = checked(folder/name, entry['sha256'])
        require(len(raw) == entry['bytes'], 'Proof size mismatch')
        if name.startswith('readback-'):
            offset = int(name[9:17], 16)
            require(len(raw) == {0x2000:24576, 0x8000:4096, 0x10000:827392}[offset], 'Wrong span length')
            require(raw == after[offset:offset+len(raw)], 'Span/full proof mismatch')
    require(sha(after[0x10000:0x10000+824720]) == APP and after[0x100b0:0x100d0].hex() == ELF,
            'Candidate app/ELF bytes mismatch')
    return dict(program_result_sha256=digest,
                app_sha256=APP, elf_sha256=ELF, predecessors=42, full_readback_verified=True)
