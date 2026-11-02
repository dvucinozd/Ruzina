"""Default offline preflight; --execute additionally needs a pinned tool binding."""
import argparse
import contextlib
import json
from pathlib import Path
from campaign import EspBackend, check_port, checked, prepare, program, require


def binding(path, digest):
    data = json.loads(checked(path, digest))
    require(data['status'] == 'REVIEWED_EXECUTION_TOOLS', 'Unreviewed execution binding')
    for name in ('campaign.py', 'program.py', 'monitor.py'):
        checked(Path(__file__).parent / name, data['tools'][name])
    return data


def main():
    p = argparse.ArgumentParser(__doc__)
    p.add_argument('--root', type=Path, required=True)
    p.add_argument('--repo', type=Path, default=Path(__file__).resolve().parents[2])
    p.add_argument('--mode', choices=('candidate', 'return-s07'), required=True)
    p.add_argument('--execute', action='store_true')
    p.add_argument('--port', default='COM3')
    p.add_argument('--output', type=Path)
    p.add_argument('--binding', type=Path)
    p.add_argument('--binding-sha256')
    args = p.parse_args()
    prepared = prepare(args.root, args.repo, args.mode)
    if not args.execute:
        print(json.dumps(prepared.summary()))
        return
    require(args.output and args.binding and args.binding_sha256, 'Execute needs fresh output and exact binding')
    bound = binding(args.binding, args.binding_sha256)
    require(bound['registration_sha256'] == prepared.registration_sha, 'Binding for another campaign')
    from serial.tools import list_ports
    check_port(args.port, list_ports.comports())
    # Directory reservation must succeed before opening any device.
    args.output.mkdir(parents=True, exist_ok=False)
    # program owns a fresh child; the console log is never used as validation.
    with (args.output / 'programmer.log').open('w', encoding='utf-8') as log:
        with contextlib.redirect_stdout(log), contextlib.redirect_stderr(log):
            result = program(prepared, EspBackend(args.port), args.output / 'capture')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
