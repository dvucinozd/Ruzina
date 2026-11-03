"""Exact JC1060 resource-readiness campaign. No serial imports during preflight."""
from dataclasses import dataclass
import hashlib
import importlib.util
import json
from pathlib import Path

REGISTRATION_SHA = 'a83f6f162536964f389507d610618cb255b9b08572dd1d2ed249e8f761d9ca9a'
PLANNER_SHA = '13bde13de4fd4a6f62fdc94147c242945696f1615286c2afccbc91416a9c6543'
FLASH_BYTES = 0x1000000
SOURCE = 'd28abd0414a264cce96de4be2a2b783dc58cfd60'
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
        require(manifest['source_commit'] == SOURCE and not manifest['dirty_status'], 'Unfrozen candidate')
        require(manifest['uac_profile'] is True and manifest['uac_pack_iram'] is False, 'Wrong profile')
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
    require(len(history) == entry['predecessors'] == (36 if mode == 'candidate' else 37), 'Ancestry count mismatch')
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


def check_port(port, entries):
    require(port == 'COM3', 'Only the registered COM3 is allowed')
    matches = [p for p in entries if p.device == port]
    require(len(matches) == 1, 'Registered serial port missing/ambiguous')
    p = matches[0]
    require((p.vid, p.pid, (p.serial_number or '').lower()) ==
            (0x303a, 0x1001, MAC), 'COM3 USB identity changed')


def program(prepared, backend, output):
    """Backend injected for host fault tests; all destructive work is gated here."""
    output = Path(output)
    output.mkdir(parents=True, exist_ok=False)
    result = dict(prepared.summary(), status='STARTED', device_access=True,
                  flash_write_attempted=False, reset_requested=False, efuse_write=False)
    connected = False
    try:
        connected = True  # close even when connect/identification partially fails
        info = backend.connect()
        result['identity'] = info
        require((info['chip'], info['revision'], info['mac']) == ('ESP32-P4', 103, MAC), 'Chip identity changed')
        require(info['security']['flags'] == 0 and info['security']['flash_crypt_cnt'] == 0, 'Security state changed')
        require(info['rdid'] == 0x184068, 'Flash RDID changed')
        actual = backend.read(0, FLASH_BYTES)
        (output / 'prewrite-full.bin').write_bytes(actual)
        require(actual == prepared.before, 'Live complete flash differs; no write permitted')
        result['full_predecessor_match'] = True
        result['flash_write_attempted'] = True
        backend.write(prepared.writes)
        for offset, payload in prepared.writes:
            actual = backend.read(offset, len(payload))
            (output / f'readback-{offset:08x}.bin').write_bytes(actual)
            require(actual == payload, 'Written span readback mismatch')
        actual = backend.read(0, FLASH_BYTES)
        (output / 'postwrite-full.bin').write_bytes(actual)
        require(actual == prepared.after, 'Postwrite complete flash differs')
        result.update(status='WRITTEN_FULL_READBACK_MATCH_ROM_HELD', postwrite_sha256=sha(actual))
        return result
    except BaseException as error:
        result.update(status='FAILED_STOP_AND_INSPECT', error=repr(error))
        raise
    finally:
        try:
            if connected:
                backend.close()
        finally:
            for path in output.glob('*.bin'):
                result.setdefault('captures', {})[path.name] = dict(bytes=path.stat().st_size, sha256=sha(path.read_bytes()))
            (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')


class EspBackend:
    def __init__(self, port):
        # Imports occur only after explicit execute, offline preflight and port check.
        import esptool
        from esptool import cmds
        require(esptool.__version__ == '5.6.0', 'Pinned esptool 5.6.0 required')
        self.esptool, self.cmds, self.port, self.esp = esptool, cmds, port, None

    def connect(self):
        self.esp = self.esptool.detect_chip(self.port, connect_mode='no-reset', connect_attempts=2)
        info = dict(chip=self.esp.CHIP_NAME, revision=self.esp.get_chip_revision(),
                    mac=':'.join(f'{v:02x}' for v in self.esp.read_mac()), security=self.esp.get_security_info())
        # Check before running a stub as well as at the write boundary.
        require((info['chip'], info['revision'], info['mac']) == ('ESP32-P4', 103, MAC), 'Chip identity changed')
        require(info['security']['flags'] == 0 and info['security']['flash_crypt_cnt'] == 0, 'Security state changed')
        self.esp = self.esp.run_stub()
        self.cmds.attach_flash(self.esp)
        info['rdid'] = self.esp.flash_id()
        return info

    def read(self, offset, size):
        return bytes(self.cmds.read_flash(self.esp, offset, size, no_progress=True))

    def write(self, writes):
        self.cmds.write_flash(self.esp, list(writes), flash_freq='keep', flash_mode='keep',
                              flash_size='keep', erase_all=False, force=False, no_progress=True)

    def close(self):
        if self.esp is not None:
            self.esp._port.close()
