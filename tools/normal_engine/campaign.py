"""Exact JC1060 normal-engine campaign. No serial imports during preflight."""
from dataclasses import dataclass
import hashlib
import importlib.util
import json
from pathlib import Path

REGISTRATION_SHA = '75792be6b74a73f6b4595e1d8f014f0fd0820c12f2a2d293acde0b6642b721e6'
PLANNER_SHA = '13bde13de4fd4a6f62fdc94147c242945696f1615286c2afccbc91416a9c6543'
FLASH_BYTES = 0x1000000
SOURCE = '886127e0714c15cb096cc3b86798fbacba1773f7'
UNIT_ID = 'JC1060-20261030-COM3'
MAC = 'e8:f6:0a:e7:22:1d'
S07_APP = '95d45203ec245a9f82455f16f15dd95ed6ad2bb8eaaca9cf85f0816ccd5dbfdd'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def contained(root, relative):
    root = Path(root).resolve()
    path = (root / relative).resolve()
    require(path.is_relative_to(root), 'Path escapes registered root')
    return path


def checked(path, digest):
    data = Path(path).read_bytes()
    require(sha(data) == digest, 'Hash mismatch: ' + str(path))
    return data


def validate_candidate_manifest(manifest):
    require(manifest['source_commit'] == SOURCE and not manifest['dirty_status'], 'Unfrozen candidate')
    require(manifest['variant'] == 'deck-engine', 'Not normal engine variant')
    require(manifest['uac_profile'] is False and manifest['uac_pack_iram'] is False, 'Wrong normal profile')


@dataclass(frozen=True)
class Prepared:
    mode: str
    registration_sha: str
    plan_sha: str
    app_sha: str
    elf_sha: str
    predecessors: int
    before: bytes
    after: bytes
    writes: tuple

    def summary(self):
        return dict(status='OFFLINE_PREFLIGHT_PASS_NO_DEVICE_ACCESS', mode=self.mode,
                    registration_sha256=self.registration_sha, plan_sha256=self.plan_sha,
                    app_sha256=self.app_sha, elf_sha256=self.elf_sha,
                    predecessors=self.predecessors, before_sha256=sha(self.before),
                    after_sha256=sha(self.after))


def prepare(root, repo, mode):
    """Return immutable verified bytes; never reopen staged files for writing."""
    require(mode in ('candidate', 'return-s07'), 'Unregistered transition')
    root, repo = Path(root).resolve(), Path(repo).resolve()
    reg = json.loads(checked(root / 'campaign-registration.json', REGISTRATION_SHA))
    require(reg['unit_id'] == UNIT_ID and reg['source_commit'] == SOURCE, 'Wrong campaign')
    for relative, digest in reg['scripts'].items():
        checked(contained(root, relative), digest)
    require(sha((repo / reg['runbook']['path']).read_bytes()) == reg['runbook']['sha256'], 'Registered runbook drift')
    entry = reg['transitions'][mode]
    plan_dir = contained(root, entry['plan_dir'])
    plan = json.loads(checked(plan_dir / 'plan.json', entry['plan_sha256']))
    require(plan['unit_id'] == UNIT_ID, 'Wrong plan unit')
    package = Path(entry['package']).resolve()
    manifest_raw = checked(package / 'manifest.json', entry['manifest_sha256'])
    require(sha(manifest_raw) == plan['manifest_sha256'], 'Unbound manifest')
    manifest = json.loads(manifest_raw)
    artifacts = {rel: checked(contained(package, rel), h)
                 for rel, h in manifest['artifacts'].items()}
    app = artifacts['build/ruzina_legacy_probe.bin']
    require(sha(app) == entry['app_sha256'] == manifest['sha256'], 'Wrong app')
    require(sha(artifacts['build/ruzina_legacy_probe.elf']) == entry['elf_sha256'], 'Wrong ELF')
    require(app[0xb0:0xd0].hex() == entry['elf_sha256'], 'App/ELF header mismatch')
    if mode == 'candidate':
        validate_candidate_manifest(manifest)
        for relative, digest in manifest['sources'].items():
            checked(contained(repo, relative), digest)
    else:
        require(entry['app_sha256'] == S07_APP, 'Return is not accepted s07')
    planner_path = repo / 'firmware-idf/legacy-jc1060/tools/prepare_flash_plan.py'
    checked(planner_path, PLANNER_SHA)
    spec = importlib.util.spec_from_file_location('readiness_planner', planner_path)
    planner = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(planner)
    unit = json.loads((repo / 'docs/boards/readiness/jc1060-20261030-com3.json').read_bytes())
    require(unit['unit_id'] == UNIT_ID, 'Wrong backup authority')
    backups = [Path(unit['factory'][key]) for key in ('read1_file', 'read2_file')]
    backups += [Path(unit['factory']['secondary_storage_copy']['path']) / p.name for p in backups[:]]
    require(len({p.resolve() for p in backups}) == 4, 'Four distinct backups required')
    factory = checked(backups[0], planner.BACKUP_SHA)
    require(len(factory) == FLASH_BYTES, 'Factory length mismatch')
    for path in backups[1:]:
        require(checked(path, planner.BACKUP_SHA) == factory, 'Factory copy mismatch')
    paths = [item['path'] for item in plan['predecessor_chain']]
    before, previous, history = planner.overlay_history(factory, UNIT_ID, paths)
    require(len(history) == entry['predecessors'] == (38 if mode == 'candidate' else 39), 'Ancestry count mismatch')
    require(previous == plan['predecessor_plan_sha256'], 'Ancestry link mismatch')
    require(sha(before) == entry['expected_before_sha256'], 'Full predecessor mismatch')
    after = bytearray(before)
    writes = []
    limits = {0x2000: 0x8000, 0x8000: 0x9000, 0x10000: 0x110000}
    for row in plan['writes']:
        start, end = row['offset'], row['erase_end']
        require(start in limits and start < end <= limits[start] and end % 4096 == 0, 'Invalid write bounds')
        require(start not in [offset for offset, _ in writes], 'Duplicate write')
        payload = checked(contained(plan_dir, row['staged_file']), row['staged_sha256'])
        restore = checked(contained(plan_dir, row['restore_file']), row['restore_sha256'])
        require(len(payload) == end - start and restore == before[start:end], 'Write/restore length mismatch')
        require(0 < row['bytes'] <= len(payload), 'Invalid image size')
        require(payload[row['bytes']:] == before[start + row['bytes']:end], 'Sector tail changed')
        after[start:end] = payload
        writes.append((start, payload))
    require({offset for offset, _ in writes} == set(limits), 'Incomplete write spans')
    require(sha(after) == entry['expected_after_sha256'], 'After reconstruction mismatch')
    return Prepared(mode, REGISTRATION_SHA, entry['plan_sha256'], entry['app_sha256'],
                    entry['elf_sha256'], len(history), bytes(before), bytes(after), tuple(writes))


# Reuse the already reviewed destructive boundary without changing its policy.
import sys
_SHARED_PATH = Path(__file__).resolve().parents[1] / 'resource_readiness/campaign.py'
_SHARED_SHA = '1b9e9b7e7f51202b65e34c5d2e212e9c42110ae2b3f8ea3df59756bfc6c2ebfb'
checked(_SHARED_PATH, _SHARED_SHA)
_shared_spec = importlib.util.spec_from_file_location('normal_campaign_shared_guard', _SHARED_PATH)
_shared = importlib.util.module_from_spec(_shared_spec)
sys.modules[_shared_spec.name] = _shared
_shared_spec.loader.exec_module(_shared)
check_port = _shared.check_port
program = _shared.program
EspBackend = _shared.EspBackend
