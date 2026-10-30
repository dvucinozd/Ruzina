"""Offline verification by default. Explicit execution never writes flash/eFuse."""
import argparse
import hashlib
import json
from pathlib import Path
import time


def digest(data):
    return hashlib.sha256(data).hexdigest()


def verify(data, manifest):
    from esptool.bin_image import LoadFirmwareImage
    if manifest.get('variant') != 'pure-ram':
        raise ValueError('Only pure-ram manifests accepted')
    if len(data) != manifest['bytes'] or digest(data) != manifest['sha256']:
        raise ValueError('Image length/hash mismatch')
    image = LoadFirmwareImage('ESP32-P4', data)
    if image.chip_id != 18 or (image.min_rev_full, image.max_rev_full) != (100, 199):
        raise ValueError('Wrong chip/revision header')
    validate_segments(image.entrypoint, [(s.addr, len(s.data)) for s in image.segments])
    return image


def validate_segments(entry, segments):
    # IDF v6.1 esp32p4/rev1 memory.ld: SRAM; ROM loader SPM allocation.
    ranges = ((0x30100000, 0x30102000), (0x4ff00000, 0x4ff2bbd0))
    ends = []
    executable = False
    for start, size in segments:
        end = start + size
        if size <= 0 or not any(lo <= start < end <= hi for lo, hi in ranges):
            raise ValueError('Segment outside reviewed legacy SRAM/SPM')
        if any(start < b and a < end for a, b in ends):
            raise ValueError('Overlapping segments')
        ends.append((start, end))
        executable |= 0x4ff00000 <= start <= entry < end
    if not executable:
        raise ValueError('Entry point outside loaded executable SRAM')


class RomTransport:
    def __init__(self, port):
        import esptool
        self.esp = esptool.detect_chip(port, connect_mode='usb-reset', connect_attempts=2)

    def identity(self):
        return (self.esp.CHIP_NAME, self.esp.get_chip_revision(),
                ':'.join(f'{v:02x}' for v in self.esp.read_mac()))

    def load(self, data):
        from esptool.cmds import load_ram
        load_ram(self.esp, data)  # esptool 5.6 accepts immutable bytes, no path reopen

    def capture(self):
        self.esp._port.timeout = 0.1
        deadline = time.monotonic() + 12
        raw = bytearray()
        self.partial = raw
        while time.monotonic() < deadline and len(raw) < 1048576:
            raw.extend(self.esp._port.read(min(4096, 1048576 - len(raw))))
        return bytes(raw)

    def reset(self):
        self.esp.hard_reset()

    def close(self):
        self.esp._port.close()


def execute(data, manifest, output, port, unit, reset_policy, factory=RomTransport):
    verify(data, manifest)
    if reset_policy not in ('hold-for-power-disconnect', 'usb-reset-observe'):
        raise ValueError('Explicit recovery policy required')
    for key in ('read1_file', 'read2_file'):
        backup = Path(unit['factory'][key]).read_bytes()
        if len(backup) != 16777216 or digest(backup) != unit['factory']['read1_sha256']:
            raise ValueError('Backup unavailable or mismatched')
    if unit['factory']['read1_file'] == unit['factory']['read2_file']:
        raise ValueError('Distinct backups required')
    output = Path(output)
    output.mkdir(parents=True, exist_ok=False)
    result = dict(scalar='NOT_RUN', recovery='NOT_RUN', readiness='NOT_QUALIFIED',
                  image_sha256=digest(data), reset_policy=reset_policy)
    transport = None
    raw = b''
    after = b''
    try:
        transport = factory(port)
        if transport.identity() != ('ESP32-P4', 103, unit['observed']['base_mac']):
            raise ValueError('Wrong unit')
        transport.load(data)
        raw = transport.capture()
        expected = (b'chip_revision=103', b'execution=PURE_RAM',
                    b'pitch_raw=0 pitch_centipercent=1000',
                    b'pitch_raw=8192 pitch_centipercent=0',
                    b'pitch_raw=16383 pitch_centipercent=-999',
                    b'pitch_raw=65535 pitch_centipercent=-999',
                    b'result=SCALAR_RUST_CALLS_MATCH')
        result['scalar'] = 'PASS' if all(x in raw for x in expected) else 'FAIL'
        if reset_policy == 'usb-reset-observe':
            transport.reset()
            after = transport.capture()
            result['recovery'] = ('FACTORY_LOG_OBSERVED' if b'esp_brookesia_demo' in after
                                  and b'HP_SYS_HP_WDT_RESET' not in after else 'FAIL')
        else:
            result['recovery'] = 'OPERATOR_POWER_DISCONNECT_REQUIRED'
    except Exception as exc:
        if transport is not None and not raw:
            raw = bytes(getattr(transport, 'partial', b''))
        result['error'] = f'{type(exc).__name__}: {exc}'
    finally:
        try:
            if transport is not None:
                transport.close()
        except Exception as exc:
            result['close_error'] = str(exc)
        (output / 'runtime.log').write_bytes(raw)
        (output / 'after.log').write_bytes(after)
        (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    return result


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('--image', type=Path, required=True)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--execute', action='store_true')
    parser.add_argument('--output', type=Path)
    parser.add_argument('--port')
    parser.add_argument('--unit', type=Path)
    parser.add_argument('--reset-policy', choices=['hold-for-power-disconnect', 'usb-reset-observe'])
    args = parser.parse_args()
    data = args.image.read_bytes()
    manifest = json.loads(args.manifest.read_text())
    verify(data, manifest)
    if not args.execute:
        print('OFFLINE_VERIFY_PASS; no transport opened')
        return
    if not all((args.output, args.port, args.unit, args.reset_policy)):
        parser.error('Execution requires output, port, unit and reset-policy')
    result = execute(data, manifest, args.output, args.port,
                     json.loads(args.unit.read_text()), args.reset_policy)
    raise SystemExit(0 if result.get('scalar') == 'PASS' and
                     result.get('recovery') == 'FACTORY_LOG_OBSERVED' and
                     'error' not in result and 'close_error' not in result else 2)


if __name__ == '__main__':
    main()
