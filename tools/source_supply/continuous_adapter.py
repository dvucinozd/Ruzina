"""Passive COM3 adapter; offline by default, new contract/binding/proof required."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import re
import time

from campaign import check_port, require, sha
from continuous import ContinuousCapture, pump
from continuous_analyze import analyze
from continuous_contract import CASES, ELF, LIMITS, load_contract, verify_binding, verify_program


class PassiveReader:
    """Pre-ELF enumeration races may retry; after ELF there is no reopen."""
    def __init__(self, capture, serial, comports, clock, sleep):
        self.capture, self.serial, self.comports = capture, serial, comports
        self.clock, self.sleep, self.port = clock, sleep, None

    def read(self, count):
        require(self.capture.boots or self.clock()-self.capture.start < 600, 'Cold boot wait expired')
        if self.port is None:
            entries = self.comports()
            if not any(entry.device == 'COM3' for entry in entries):
                self.sleep(.1)
                return b''
            check_port('COM3', entries)
            port = self.serial.Serial(port=None, baudrate=115200, timeout=.1)
            try:
                port.dtr = port.rts = False
                port.port = 'COM3'
                port.open()
            except BaseException as error:
                port.close()
                if not isinstance(error, self.serial.SerialException) or self.capture.boots or any(entry.device == 'COM3' for entry in self.comports()):
                    raise
                self.sleep(.1)
                return b''
            self.port = port
        try:
            return self.port.read(count)
        except self.serial.SerialException:
            if self.capture.boots:
                raise
            self.close()
            self.sleep(.1)
            return b''

    def close(self):
        if self.port is not None:
            self.port.close()
            self.port = None


class Controller:
    """Append-only numbered local commands; readiness never comes from a timer."""
    def __init__(self, capture, contract):
        self.capture, self.contract = capture, contract
        self.admitted, self.command_number = 0, 1
        self.commands = capture.output/'commands'
        self.accepted = capture.output/'accepted-commands'
        self.commands.mkdir()
        self.accepted.mkdir()
        self.last_status = None
        self.executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix='capture-admission')
        self.pending = None
        self.analysis_started = None

    def status(self):
        capture = self.capture
        case = capture.cases[capture.active] if capture.active is not None else None
        status = dict(admitted_cases=self.admitted, active_case=case, error=capture.failed,
                      boot_seen=bool(capture.boots), cold_confirmed=bool(capture.cold_confirmation),
                      admission_pending=self.pending is not None,
                      next_command=f'{self.command_number:06}.json', serial_writes=False)
        raw = json.dumps(status, indent=2)+'\n'
        if raw != self.last_status:
            temporary = capture.output/'status.tmp'
            temporary.write_text(raw, encoding='utf-8')
            temporary.replace(capture.output/'status.json')
            self.last_status = raw

    def command(self, now):
        files = list(self.commands.iterdir())
        require(all(re.fullmatch(r'\d{6}\.json', path.name) or path.suffix == '.tmp' for path in files),
                'Unknown control file; publish complete JSON with atomic rename')
        path = self.commands/f'{self.command_number:06}.json'
        if not path.exists():
            return False
        raw = path.read_bytes()
        require(len(raw) <= 8192, 'Oversized control command')
        command = json.loads(raw)
        require(command.get('sequence') == self.command_number, 'Control sequence mismatch')
        kind = command.get('command')
        if kind == 'confirm-cold':
            require(set(command) == {'sequence', 'command', 'reply'}, 'Unexpected confirmation fields')
            self.capture.confirm_physical_cold(command['reply'], now)
        elif kind in ('ready', 'start'):
            keys = {'sequence', 'command', 'token'} | ({'reply'} if kind == 'ready' else set())
            require(set(command) == keys, 'Unexpected readiness fields')
            if kind == 'ready':
                self.capture.acknowledge_ready(command['token'], command['reply'], now)
            else:
                self.capture.begin_case(command['token'], now)
        else:
            require(kind == 'stop' and set(command) == {'sequence', 'command'}, 'Unknown control command')
        # Retain the exact accepted command. Originals remain for comparison.
        with (self.accepted/path.name).open('xb') as output:
            output.write(raw)
        self.command_number += 1
        return kind == 'stop'

    def __call__(self, capture, now):
        # Previously accepted files are immutable; a changed reply is not reused.
        for path in self.accepted.iterdir():
            require(path.read_bytes() == (self.commands/path.name).read_bytes(), 'Accepted control file changed')
        if self.command(now):
            self.status()
            return True
        if capture.boots and capture.cold_confirmation is None:
            require(now < capture.cases[0]['deadline']+120, 'Physical cold confirmation timeout')
        if self.pending is not None:
            require(now-self.analysis_started < 30, 'Admission analysis timeout')
            if not self.pending.done():
                self.status()
                return False
            summary = self.pending.result()
            self.pending = None
            self.admitted = summary['completed_cases']
            if self.admitted == len(CASES):
                self.status()
                return True
            spec = CASES[self.admitted]
            token = capture.offer_case(spec['name'], spec['kind'], spec['seconds'], now)
            if spec['kind'] == 'idle':
                capture.acknowledge_ready(token, 'Automatic idle after admitted physical cold', now)
                capture.begin_case(token, now)
        elif capture.active is None and len(capture.cases) > self.admitted and capture.cold_confirmation:
            case = capture.cases[-1]
            require(case['state'] == 'BOUND_REACHED_NOT_QUALIFIED', 'Incomplete case cannot continue')
            folder, snapshot_job = capture.prepare_snapshot(case['name'], now)
            def admission():
                try:
                    snapshot_job()
                    summary = analyze(folder, self.contract)
                    (folder/'admission.json').write_text(json.dumps(summary, indent=2)+'\n')
                    return summary
                except Exception as error:
                    (folder/'analysis-attempt.json').write_text(json.dumps(dict(status='INVALID', error=repr(error)))+'\n')
                    raise
            self.analysis_started = now
            self.pending = self.executor.submit(admission)
        self.status()
        return False

    def close(self):
        # Called only after the reader/port has closed, including fault exits.
        self.executor.shutdown(wait=True, cancel_futures=True)


def execute(output, contract, authority, serial, comports, clock=time.monotonic, sleep=time.sleep):
    capture = ContinuousCapture(output, LIMITS, ELF, start=clock(), auto_cold=True)
    (capture.output/'authority.json').write_text(json.dumps(authority, indent=2)+'\n')
    reader = PassiveReader(capture, serial, comports, clock, sleep)
    try:
        controller = Controller(capture, contract)
    except BaseException as error:
        capture.fail('Controller setup failed: '+repr(error), capture.last)
        capture.close(capture.last)
        raise
    try:
        result = pump(capture, reader, clock, controller)
    finally:
        controller.close()
    error = result['error']
    if not error and controller.admitted == len(CASES):
        try:
            final_summary = analyze(capture.output, contract)
            (capture.output/'final-admission.json').write_text(json.dumps(final_summary, indent=2)+'\n')
        except Exception as caught:
            error = 'Final whole-session analysis: '+repr(caught)
            (capture.output/'final-analysis-attempt.json').write_text(json.dumps(dict(status='INVALID', error=error))+'\n')
    status = ('MEASURED_NOT_PRODUCT_QUALIFIED' if not error and controller.admitted == len(CASES)
              else 'INVALID_OR_INCOMPLETE_RESTORE_REQUIRED')
    final = dict(status=status, admitted_cases=controller.admitted, raw_sha256=result['raw_sha256'],
                 events_sha256=result['events_sha256'], authority_sha256=sha((capture.output/'authority.json').read_bytes()),
                 serial_writes=False, reset_requested=False, device_access=True,
                 writer_invoked=False, automatic_recovery_invoked=False, port_closed=reader.port is None,
                 error=error)
    (capture.output/'adapter-result.json').write_text(json.dumps(final, indent=2)+'\n')
    return final


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--contract', required=True, type=Path)
    parser.add_argument('--contract-sha256', required=True)
    parser.add_argument('--binding', type=Path)
    parser.add_argument('--binding-sha256')
    parser.add_argument('--program-capture', type=Path)
    parser.add_argument('--program-result-sha256')
    parser.add_argument('--output', type=Path)
    parser.add_argument('--execute', action='store_true')
    args = parser.parse_args()
    contract = load_contract(args.contract, args.contract_sha256)
    if not args.execute:
        print(json.dumps(dict(status='OFFLINE_CONTRACT_POLICY_PASS', device_access=False)))
        return
    require(args.output and not args.output.exists() and args.binding and args.binding_sha256 and
            args.program_capture and args.program_result_sha256, 'Need fresh output, new binding and candidate proof')
    verify_binding(args.binding, args.binding_sha256, args.contract_sha256)
    proof = verify_program(args.program_capture, args.program_result_sha256, contract)
    authority = dict(contract_sha256=args.contract_sha256, binding_sha256=args.binding_sha256,
                     programmer=proof, contract=contract, passive_timeout_seconds=.1)
    # Deliberately unreachable with an old 40/41 contract/binding. No serial
    # imports or device lookup occur until all offline proof checks pass.
    import serial
    from serial.tools import list_ports
    result = execute(args.output, contract, authority, serial, list_ports.comports)
    print(json.dumps(result))
    if result['status'] != 'MEASURED_NOT_PRODUCT_QUALIFIED':
        raise SystemExit(1)


if __name__ == '__main__':
    main()
