"""Separate one-shot s07 boot pulse after automatic return readback; never flash."""
import argparse
from importlib.metadata import version
import importlib.util
import json
from pathlib import Path
import re
import sys

from campaign import MAC, S07_APP, UNIT_ID, check_port, checked, require, sha

S07_ELF = '005f8d1e33aa5f2a8d05af691ce768461f559c917b8804920a912178f5b78ddf'
RESTORED = '28830827d3e3426bbcd58b9a25d4d2129e28f150bade33fe3c551cd577f93921'
PYTHON = Path('D:/BoardEvidence/JC1060-20261030-COM3/toolchain/python-idf61/Scripts/python.exe')
RESET_SOURCE = '52d636ed8eff0855cf9e8c44deb1c5392ea642be153f9dee2931ad334d474b88'


def policy(path, digest):
    value = json.loads(checked(path, digest))
    expected = dict(status='REGISTERED_S07_SOFTWARE_BOOT', schema=1, unit_id=UNIT_ID,
                    mac=MAC, chip_revision=103, native_minimum=302,
                    predecessors=43, history_after_return=44, app_sha256=S07_APP,
                    elf_sha256=S07_ELF, after_sha256=RESTORED,
                    action='ONE_USB_RTS_BOOT_PULSE', manual_requalification=False)
    for key, field in expected.items():
        require(value.get(key) == field, 'Boot policy mismatch: '+key)
    for key in ('registration_sha256', 'return_plan_sha256', 'before_sha256'):
        require(isinstance(value.get(key), str) and re.fullmatch(r'[0-9a-f]{64}', value[key]),
                'Missing boot digest: '+key)
    require(value['return_plan_sha256'] != 'cd76f6c0611a9c587f25b7f02352fa81805a961a99e885c415287898c9d1f85d',
            'Consumed return plan')
    return value


def return_proof(folder, digest, frozen):
    folder = Path(folder)
    report = json.loads(checked(folder/'result.json', digest))
    expected = dict(status='WRITTEN_FULL_READBACK_MATCH_ROM_HELD', mode='return-s07',
                    predecessors=43, registration_sha256=frozen['registration_sha256'],
                    plan_sha256=frozen['return_plan_sha256'], before_sha256=frozen['before_sha256'],
                    after_sha256=RESTORED, postwrite_sha256=RESTORED,
                    app_sha256=S07_APP, elf_sha256=S07_ELF,
                    full_predecessor_match=True, reset_requested=False, efuse_write=False)
    for key, field in expected.items():
        require(report.get(key) == field, 'Return proof mismatch: '+key)
    identity = report['identity']
    require((identity['chip'], identity['revision'], identity['mac'], identity['rdid']) ==
            ('ESP32-P4', 103, MAC, 0x184068), 'Return proof unit drift')
    require(identity['security']['flags'] == identity['security']['flash_crypt_cnt'] == 0, 'Return security drift')
    before = checked(folder/'prewrite-full.bin', frozen['before_sha256'])
    after = checked(folder/'postwrite-full.bin', RESTORED)
    require(len(before) == len(after) == 0x1000000, 'Need complete return readback')
    names = {'prewrite-full.bin', 'postwrite-full.bin', 'readback-00002000.bin',
             'readback-00008000.bin', 'readback-00010000.bin'}
    require(set(report['captures']) == names, 'Missing automatic return spans')
    for name, entry in report['captures'].items():
        raw = checked(folder/name, entry['sha256'])
        require(len(raw) == entry['bytes'], 'Return proof length mismatch')
        if name.startswith('readback-'):
            offset = int(name[9:17], 16)
            require(len(raw) == {0x2000:24576, 0x8000:4096, 0x10000:770048}[offset] and
                    after[offset:offset+len(raw)] == raw, 'Return span/full mismatch')
    require(sha(after[0x10000:0x10000+768176]) == S07_APP and after[0x100b0:0x100d0].hex() == S07_ELF,
            'Accepted s07 app/ELF proof mismatch')
    return dict(return_result_sha256=digest, return_plan_sha256=frozen['return_plan_sha256'],
                full_and_span_readback_verified=True, history_after_return=44)


def binding(path, digest, policy_sha):
    value = json.loads(checked(path, digest))
    require(value.get('status') == 'REVIEWED_S07_SOFTWARE_BOOT_TOOLS' and
            value.get('policy_sha256') == policy_sha, 'Wrong/unreviewed boot binding')
    require(set(value['tools']) == {'software_boot.py', 'campaign.py'}, 'Incomplete boot dependency binding')
    for name, field in value['tools'].items():
        checked(Path(__file__).parent/name, field)
    checked(Path(__file__).resolve().parents[1]/'resource_readiness/campaign.py', value['shared_guard_sha256'])


def issue(output, authority, serial, comports, pulse):
    """Injected backend for tests; no automatic retry or application acceptance."""
    output = Path(output)
    output.mkdir(parents=True, exist_ok=False)
    result = dict(status='STARTED', authority=authority, flash_write=False, efuse_write=False,
                  serial_data_writes=False, reset_attempted=False, reset_completed=False,
                  physical_power_cycle=False, application_running='NOT_OBSERVED', port_closed=True)
    port = None
    try:
        check_port('COM3', comports())
        port = serial.Serial(port=None, baudrate=115200, timeout=.1)
        result['port_closed'] = False
        port.dtr = port.rts = False
        port.port = 'COM3'
        port.open()
        check_port('COM3', comports())
        result['reset_attempted'] = True
        pulse(port, hold_delay=.2, post_release_delay=.2, flow_control=False)
        result.update(reset_completed=True, status='BOOT_PULSE_ISSUED_APPLICATION_NOT_OBSERVED')
    except BaseException as error:
        result.update(status='FAILED_BOOT_STATE_UNKNOWN', error=repr(error))
        raise
    finally:
        try:
            if port is not None:
                port.close()
                result['port_closed'] = True
        except BaseException as error:
            result.update(status='FAILED_BOOT_STATE_UNKNOWN', close_error=repr(error))
            raise
        finally:
            (output/'result.json').write_text(json.dumps(result, indent=2)+'\n')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--policy', type=Path, required=True)
    parser.add_argument('--policy-sha256', required=True)
    parser.add_argument('--return-capture', type=Path, required=True)
    parser.add_argument('--return-result-sha256', required=True)
    parser.add_argument('--binding', type=Path)
    parser.add_argument('--binding-sha256')
    parser.add_argument('--output', type=Path)
    parser.add_argument('--execute', action='store_true')
    args = parser.parse_args()
    frozen = policy(args.policy, args.policy_sha256)
    proof = return_proof(args.return_capture, args.return_result_sha256, frozen)
    if not args.execute:
        print(json.dumps(dict(status='OFFLINE_S07_RETURN_PROOF_PASS_NO_RESET', proof=proof)))
        return
    require(args.binding and args.binding_sha256 and args.output and not args.output.exists(),
            'Need separate reviewed boot binding and fresh output')
    binding(args.binding, args.binding_sha256, args.policy_sha256)
    require(Path(sys.executable).resolve() == PYTHON.resolve(), 'Use the pinned IDF Python')
    require((version('esptool'), version('esp-pylib'), version('pyserial')) == ('5.6.0', '1.2.0', '3.5'),
            'Pinned reset backend versions required')
    spec = importlib.util.find_spec('esp_pylib.serial_reset')
    require(spec is not None and spec.origin, 'Missing pinned reset backend')
    checked(spec.origin, RESET_SOURCE)
    # Direct low-level pulse: no esptool reset strategy, stub, ROM run/flash
    # commands, watchdog path, swallowed-warning fallback or retry loop.
    import serial
    from serial.tools import list_ports
    from esp_pylib.serial_reset import hard_reset
    authority = dict(policy_sha256=args.policy_sha256, binding_sha256=args.binding_sha256,
                     reset_backend_sha256=RESET_SOURCE, **proof)
    result = issue(args.output, authority, serial, list_ports.comports, hard_reset)
    print(json.dumps(result))


if __name__ == '__main__':
    main()
