"""Bounded passive exact-image capture; defaults to offline preflight."""
import argparse
import json
from pathlib import Path
import re
import time
from campaign import check_port, prepare, require, sha
from program import binding

FATAL = (b'Guru Meditation', b'assert failed', b'Stack protection fault', b'abort()', b'HP_SYS_HP_WDT_RESET')


def window(raw, elf):
    """Keep preceding bytes immutable, analyze only the new expected boot."""
    identities = list(re.finditer(rb'ELF file SHA256:\s+([0-9a-fA-F]{8,64})', raw))
    expected = [m for m in identities if elf.startswith(m[1].decode().lower())]
    require(len(expected) == 1, 'Expected exactly one new matching ELF boot')
    start = expected[0].start()
    require(not any(m.start() > start for m in identities), 'Another boot followed the capture window')
    return start, raw[start:]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--root', type=Path, required=True)
    p.add_argument('--repo', type=Path, default=Path(__file__).resolve().parents[2])
    p.add_argument('--mode', choices=('candidate', 'return-s07'), required=True)
    p.add_argument('--execute', action='store_true')
    p.add_argument('--port', default='COM3')
    p.add_argument('--output', type=Path)
    p.add_argument('--binding', type=Path)
    p.add_argument('--binding-sha256')
    p.add_argument('--seconds', type=int, default=300)
    p.add_argument('--after-boot', type=int, default=45, help='0 for separate passive steady window')
    args = p.parse_args()
    require(1 <= args.seconds <= 600 and 0 <= args.after_boot < args.seconds, 'Invalid capture duration')
    prepared = prepare(args.root, args.repo, args.mode)
    if not args.execute:
        print(json.dumps(prepared.summary()))
        return
    require(args.output and args.binding and args.binding_sha256, 'Execute needs fresh output and exact binding')
    bound = binding(args.binding, args.binding_sha256)
    require(bound['registration_sha256'] == prepared.registration_sha, 'Binding for another campaign')
    if args.after_boot == 0:
        # A steady capture must be associated with a verified cold capture later.
        require(args.seconds == 60, 'Registered steady duration is 60 seconds')
    else:
        require(args.after_boot == 45, 'Registered cold duration is 45 seconds')
    import serial
    from serial.tools import list_ports
    args.output.mkdir(parents=True, exist_ok=False)
    raw, port, boot_at = bytearray(), None, None
    start = time.monotonic()
    result = dict(status='STARTED', mode=args.mode, serial_writes=False, reset_requested=False,
                  elf_sha256=prepared.elf_sha, app_sha256=prepared.app_sha, physical_acceptance='NOT_ASSESSED')
    try:
        with (args.output / 'run.log').open('xb') as log:
            while time.monotonic() - start < args.seconds and not (args.output / 'stop.request').exists():
                if port is None:
                    entries = list_ports.comports()
                    if not any(p.device == args.port for p in entries):
                        time.sleep(.2)
                        continue
                    check_port(args.port, entries)
                    port = serial.Serial(port=None, baudrate=115200, timeout=.1)
                    port.dtr = False
                    port.rts = False
                    port.port = args.port
                    port.open()
                try:
                    data = port.read(4096)
                except serial.SerialException:
                    port.close()
                    port = None
                    time.sleep(.2)
                    continue
                raw.extend(data)
                log.write(data)
                log.flush()
                if any(token in raw[-8192:] for token in FATAL):
                    raise RuntimeError('Fatal firmware marker captured')
                if boot_at is None and args.after_boot:
                    matches = re.findall(rb'ELF file SHA256:\s+([0-9a-fA-F]{8,64})', raw[-8192:])
                    if any(prepared.elf_sha.startswith(m.decode().lower()) for m in matches):
                        boot_at = time.monotonic()
                if boot_at is not None and time.monotonic() - boot_at >= args.after_boot:
                    break
        result['status'] = 'CAPTURED_NOT_QUALIFIED'
        if args.after_boot:
            offset, boot = window(bytes(raw), prepared.elf_sha)
            (args.output / 'boot-window.log').write_bytes(boot)
            result.update(boot_window_offset=offset, boot_window_sha256=sha(boot))
        else:
            result['requires_matching_cold_identity'] = True
    except BaseException as error:
        result.update(status='FAILED_CAPTURE', error=repr(error))
        raise
    finally:
        if port is not None:
            port.close()
        result.update(bytes=len(raw), raw_sha256=sha(raw), elapsed_seconds=time.monotonic()-start,
                      fatal_markers=[token.decode() for token in FATAL if token in raw])
        (args.output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
