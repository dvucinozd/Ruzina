"""Bounded passive exact-image capture; defaults to offline preflight."""
import argparse
import json
from pathlib import Path
import re
import time
from campaign import check_port, prepare, require, sha, checked
from program import binding

FATAL = (b'Guru Meditation', b'assert failed', b'Stack protection fault', b'abort()', b'HP_SYS_HP_WDT_RESET')


def fault_reason(raw, limits):
    """Inspect complete report lines only; first stop is retained, never waived."""
    for line in raw.decode('utf-8', errors='replace').splitlines():
        fields = {k:int(v) for k,v in re.findall(r'(\w+)=([0-9]+)(?=\s|$)',line)}
        if line.startswith('usb_uac_stats '):
            for key in ('iso_errors','packet_errors','submit_errors'):
                if fields.get(key,-1)!=0:return 'USB '+key+' nonzero/missing'
        elif line.startswith('deck_mix_stats '):
            match=re.search(r'\bunderruns=(\d+)/(\d+)\b',line)
            if not match or any(int(v) for v in match.groups()):return 'Source underrun/missing'
        elif line.startswith('uac_fill_phase '):
            if fields.get('generation')!=1:return 'Unexpected UAC generation'
            if fields.get('max_us',6000)>=6000 or fields.get('ge_6000',1) or fields.get('failures',1):
                return 'All-fill rejection/failure'
        elif line.startswith('engine_resources '):
            mapping={'usb_stack_free':'usb_stack_min','logger_stack_free':'logger_stack_min',
                'internal_min':'internal_heap_min','psram_min':'psram_heap_min',
                'internal_largest':'observed_internal_largest_min','psram_largest':'observed_psram_largest_min'}
            if any(fields.get(k,-1)<limits[v] for k,v in mapping.items()):return 'Resource reserve/missing'
        elif line.startswith('rust_load '):
            if fields.get('event')==6:return 'Source/media failure'
            if fields.get('stack_free',-1)<limits['sd_stack_min']:return 'SD-client reserve/missing'
        elif line.startswith(('producer_profile ','producer_owner ','producer_usb_service ')):
            if fields.get('unavailable') or fields.get('invalid') or fields.get('saturated'):
                return 'Producer/service unavailable/invalid/saturated'
        elif line.startswith('producer_water ') and fields.get('log_drops',1):
            return 'Logger reports dropped/missing'
    return None


def validate_window(kind, seconds, after_boot, mode):
    require(0 <= after_boot < seconds, 'Invalid capture duration')
    if kind == 'cold':
        require(after_boot == 45 and seconds <= 600, 'Cold window is 45 seconds after ELF, bounded by 600')
    elif kind == 'idle':
        require(after_boot == 0 and seconds == 60, 'Idle window is 60 seconds')
    elif kind == 'functional':
        require(after_boot == 0 and 30 <= seconds <= 300, 'Functional window must be bounded to 30-300 seconds')
    elif kind == 'soak':
        raise ValueError('Soak is not registered for this source diagnostic')
    else:
        raise ValueError('Unknown capture window')


class CaptureCoverage:
    """One uninterrupted capture; pre-boot USB cycling is allowed for cold only."""
    def __init__(self, kind, requested, start):
        self.kind, self.requested, self.start = kind, requested, start
        self.since, self.ended = None, None
        self.interrupted = False

    def opened(self, now):
        if self.kind != 'cold' and self.since is None:
            self.since = now

    def boot(self, now):
        if self.kind == 'cold' and self.since is None:
            self.since = now

    def disconnected(self, now):
        if self.since is not None:
            self.ended, self.interrupted = now, True

    def seconds(self, now):
        return 0 if self.since is None else (self.ended if self.ended is not None else now) - self.since

    def active(self, now):
        if self.interrupted:
            return False
        if self.kind == 'cold':
            return now - self.start < self.requested
        # Bound initial port absence separately; it never counts as capture.
        if self.since is None:
            return now - self.start < 10
        return self.seconds(now) < self.requested


def capture_complete(kind, connected_seconds, requested, post_boot, stopped, interrupted=False):
    if interrupted:
        return False
    if kind == 'cold':
        return post_boot is not None and post_boot >= 45
    if kind == 'functional' and stopped:
        return connected_seconds >= 30
    return connected_seconds >= requested


def open_capture_port(port, coverage, serial_error, port_present):
    """Tolerate enumeration/open races only during the permitted cold pre-boot wait."""
    try:
        port.open()
    except serial_error:
        if coverage.kind != 'cold' or coverage.since is not None or port_present():
            raise
        port.close()
        return False
    return True


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
    p.add_argument('--window', choices=('cold', 'idle', 'functional', 'soak'), default='cold')
    p.add_argument('--after-boot', type=int, default=45, help='0 for windows anchored to a verified boot')
    args = p.parse_args()
    if args.window=='functional':
        require(args.seconds<=180,'Source diagnostic functional bound is at most 180 seconds')
    validate_window(args.window, args.seconds, args.after_boot, args.mode)
    prepared = prepare(args.root, args.repo, args.mode)
    if not args.execute:
        print(json.dumps(prepared.summary()))
        return
    require(args.output and args.binding and args.binding_sha256, 'Execute needs fresh output and exact binding')
    bound = binding(args.binding, args.binding_sha256)
    require(bound['registration_sha256'] == prepared.registration_sha, 'Binding for another campaign')
    import serial
    from serial.tools import list_ports
    args.output.mkdir(parents=True, exist_ok=False)
    raw, port, boot_at = bytearray(), None, None
    start = time.monotonic()
    coverage = CaptureCoverage(args.window, args.seconds, start)
    watch_pending=b''
    stopped_for_fault=None
    limits=json.loads(checked(args.root/'campaign-registration.json',prepared.registration_sha))['measurement']['resources']
    result = dict(status='STARTED', mode=args.mode, window=args.window, requested_seconds=args.seconds,
                  serial_writes=False, reset_requested=False,
                  elf_sha256=prepared.elf_sha, app_sha256=prepared.app_sha, physical_acceptance='NOT_ASSESSED')
    try:
        with (args.output / 'run.log').open('xb') as log:
            while coverage.active(time.monotonic()) and not (args.output / 'stop.request').exists():
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
                    if not open_capture_port(port, coverage, serial.SerialException,
                                             lambda: any(p.device == args.port for p in list_ports.comports())):
                        port = None
                        time.sleep(.2)
                        continue
                    coverage.opened(time.monotonic())
                try:
                    data = port.read(4096)
                except serial.SerialException:
                    coverage.disconnected(time.monotonic())
                    port.close()
                    port = None
                    time.sleep(.2)
                    continue
                raw.extend(data)
                log.write(data)
                log.flush()
                if args.mode=='candidate':
                    watch_pending+=data
                    if b'\n' in watch_pending:
                        complete_lines,watch_pending=watch_pending.rsplit(b'\n',1)
                        stopped_for_fault=fault_reason(complete_lines,limits)
                        if stopped_for_fault:break
                if any(token in raw[-8192:] for token in FATAL):
                    raise RuntimeError('Fatal firmware marker captured')
                if boot_at is None and args.after_boot:
                    matches = re.findall(rb'ELF file SHA256:\s+([0-9a-fA-F]{8,64})', raw[-8192:])
                    if any(prepared.elf_sha.startswith(m.decode().lower()) for m in matches):
                        boot_at = time.monotonic()
                        coverage.boot(boot_at)
                if boot_at is not None and coverage.seconds(time.monotonic()) >= args.after_boot:
                    break
        elapsed = time.monotonic() - start
        connected_seconds = coverage.seconds(time.monotonic())
        post_boot = None if boot_at is None else connected_seconds
        stopped = (args.output / 'stop.request').exists()
        complete = capture_complete(args.window, connected_seconds, args.seconds, post_boot, stopped,
                                    coverage.interrupted)
        result.update(status='CAPTURED_NOT_QUALIFIED' if complete else 'CAPTURE_INCOMPLETE',
                      capture_complete=complete, post_boot_seconds=post_boot, stopped_by_request=stopped,
                      connected_capture_seconds=connected_seconds, connection_interrupted=coverage.interrupted,
                      stopped_for_fault=stopped_for_fault)
        if args.after_boot and boot_at is not None:
            offset, boot = window(bytes(raw), prepared.elf_sha)
            (args.output / 'boot-window.log').write_bytes(boot)
            result.update(boot_window_offset=offset, boot_window_sha256=sha(boot))
        elif not args.after_boot:
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
