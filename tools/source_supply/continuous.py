"""Offline-tested continuous capture core. No device opener or execution permission.

A future registered adapter supplies passive bytes and monotonic receive times.
The replay CLI exercises framing only; it cannot qualify a board or a case.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import secrets

from analyze import HEADER, STAGES, USB_STAGES, fields, numbers
from campaign import require, sha
from monitor import FATAL, fault_reason

REPORTS = ('producer_profile', 'producer_profile_stage', 'producer_owner',
           'producer_usb_service', 'producer_water', 'usb_uac_stats',
           'deck_mix_stats', 'uac_fill_first', 'uac_fill_phase',
           'engine_resources', 'deck_engine_stats')
ORDER = []
for _deck in (1, 2):
    ORDER.append(('producer_profile', str(_deck), None))
    ORDER.extend(('producer_profile_stage', str(_deck), stage) for stage in STAGES)
ORDER += [('producer_owner', None, None)]
ORDER += [('producer_usb_service', None, stage) for stage in USB_STAGES]
ORDER += [(key, None, None) for key in
          ('producer_water', 'usb_uac_stats', 'deck_mix_stats', 'uac_fill_first')]
ORDER += [('uac_fill_phase', None, phase) for phase in
          ('initial_prefill', 'recovery_prefill', 'callback')]
ORDER += [('engine_resources', None, None), ('deck_engine_stats', None, None)]


class ContinuousCapture:
    """One connection, append-only raw/event files, fail closed on bad evidence.

    Times describe host receipt, never device emission or CPU time. The caller
    must poll tick() even on empty reads, and disconnected() on any lost port.
    Cold power/identity, source freshness and numerical acceptance remain the
    registered adapter/analyzer's responsibility. No positive acceptance here.
    """
    def __init__(self, output, limits, expected_elf, *, start=0.0, max_seconds=1800):
        require(re.fullmatch(r'[0-9a-f]{64}', expected_elf), 'Need exact ELF SHA')
        require(math.isfinite(start) and 0 < max_seconds <= 1800, 'Invalid session bound')
        self.output = Path(output)
        self.output.mkdir(parents=True, exist_ok=False)
        self.raw_file = (self.output / 'stream.log').open('xb')
        self.events = (self.output / 'events.jsonl').open('x', encoding='utf-8', newline='\n')
        self.limits, self.expected_elf = limits, expected_elf
        self.start, self.last, self.max_seconds = start, start, max_seconds
        self.offset, self.line_offset = 0, 0
        self.digest = hashlib.sha256()
        self.pending = b''
        self.tail = b''
        self.frame = None
        self.frames, self.cases = [], []
        self.active = None
        self.failed = None
        self.closed = False
        self.boots = 0
        self.event('session_started', start, serial_writes=False, reset_requested=False)

    def event(self, kind, now, **extra):
        self.events.write(json.dumps(dict(event=kind, received_seconds=now,
                                          offset=self.offset, **extra)) + '\n')
        self.events.flush()

    def fail(self, reason, now):
        if self.failed is None:
            self.failed = reason
            self.event('session_invalid', now, reason=reason)
            if self.active is not None:
                self.end_case(now, 'INVALID')

    def tick(self, now):
        require(not self.closed, 'Session closed')
        require(math.isfinite(now) and now >= self.last, 'Receive clock regression/nonfinite')
        self.last = now
        if now - self.start >= self.max_seconds:
            self.fail('Session time bound reached', now)
        if self.active is not None:
            case = self.cases[self.active]
            if case['state'] in ('OFFERED', 'READY') and now >= case['ready_deadline']:
                self.end_case(now, 'READY_TIMEOUT')
                self.fail('Owner readiness timeout', now)
            elif case['state'] == 'CAPTURING' and now >= case['deadline']:
                self.end_case(now, 'BOUND_REACHED_NOT_QUALIFIED')

    def feed(self, data, now):
        require(isinstance(data, bytes) and len(data) <= 4096, 'Read chunks must be <=4096 bytes')
        self.tick(now)
        require(self.failed is None, 'Invalid session cannot receive more evidence')
        # Retain the entire delivered chunk even when its first line is invalid.
        self.raw_file.write(data)
        self.raw_file.flush()
        self.digest.update(data)
        self.offset += len(data)
        if self.offset > 64 * 1024 * 1024:
            self.fail('Session byte bound reached', now)
            return
        self.tail = (self.tail + data)[-8192:]
        if any(marker in self.tail for marker in FATAL):
            self.fail('Fatal firmware marker', now)
            return
        self.pending += data
        while b'\n' in self.pending and self.failed is None:
            line, self.pending = self.pending.split(b'\n', 1)
            begin = self.line_offset
            self.line_offset += len(line) + 1
            try:
                self.line(line.rstrip(b'\r').decode('utf-8', errors='strict'),
                          begin, self.line_offset, now)
            except (ValueError, UnicodeError) as error:
                self.fail(str(error), now)
        if len(self.pending) > 16384:
            self.fail('Unterminated/oversized serial line', now)

    def line(self, text, begin, end, now):
        if 'ELF file SHA256:' in text:
            self.boots += 1
            match = re.search(r'ELF file SHA256:\s+([0-9a-fA-F]{8,64})(?:\.\.\.)?\s*$', text)
            require(self.boots == 1 and match is not None and
                    self.expected_elf.startswith(match[1].lower()), 'Unexpected/malformed boot ELF')
            self.event('expected_elf_received', now, line_offset=begin)
        key = text.split(' ', 1)[0]
        # A joined prefix/suffix must not hide a known telemetry/fatal row.
        if key not in REPORTS:
            require(not any(report + ' ' in text for report in REPORTS), 'Embedded telemetry row')
            return
        require(self.boots == 1, 'Telemetry arrived before expected boot ELF')
        row = fields(text)  # Reject duplicates, including the real failed S03 prefix.
        reason = fault_reason((text + '\n').encode(), self.limits)
        require(reason is None, reason or 'Runtime fault')
        if self.frame is None:
            require(key == 'producer_profile' and row.get('deck') == '1', 'Orphan report/frame prefix')
            self.frame = dict(start=begin, received_start=now, rows=0, decks={})
        frame = self.frame
        expected = ORDER[frame['rows']]
        descriptor = (key, row.get('deck') if key in ('producer_profile', 'producer_profile_stage') else None,
                      row.get('stage') if key in ('producer_profile_stage', 'producer_usb_service') else
                      row.get('phase') if key == 'uac_fill_phase' else None)
        require(descriptor == expected, 'Missing/duplicate/out-of-order report line')
        if key == 'producer_profile':
            header = numbers(row, HEADER + ('deck',))
            require(header['schema'] == 1 and header['generation'] == 1 and
                    header['seq'] > 0 and header['seq'] % 2 == 0, 'Bad producer identity')
            frame['decks'][row['deck']] = header
        elif key == 'producer_profile_stage':
            require(row.get('seq') == str(frame['decks'][row['deck']]['seq']), 'Stage sequence mismatch')
            numbers(row, ('calls', 'total_us', 'max_us'))
        frame['rows'] += 1
        if frame['rows'] == len(ORDER):
            record = dict(start=frame['start'], end=end, received_start=frame['received_start'],
                          received_end=now, decks=frame['decks'])
            self.frames.append(record)
            self.event('complete_report_frame', now, frame=record)
            self.frame = None

    def offer_case(self, name, kind, seconds, now, *, readiness_seconds=120):
        self.tick(now)
        require(self.failed is None and self.active is None, 'Session invalid or case already active')
        require(re.fullmatch(r'[a-z0-9][a-z0-9-]{0,63}', name) and
                name not in ('stream', 'events', 'result') and
                name not in {case['name'] for case in self.cases}, 'Invalid/reused case name')
        require((kind == 'idle' and seconds == 60) or
                (kind == 'functional' and 30 <= seconds <= 180) or
                (kind == 'both' and seconds == 60), 'Unregistered case duration')
        require(0 < readiness_seconds <= 120, 'Invalid readiness bound')
        token = secrets.token_hex(16)
        self.cases.append(dict(name=name, kind=kind, seconds=seconds, token=token,
                               offered=now, ready_deadline=now + readiness_seconds, state='OFFERED'))
        self.active = len(self.cases) - 1
        self.event('case_offered', now, case=self.cases[self.active].copy())
        return token

    def acknowledge_ready(self, token, owner_reply, now):
        self.tick(now)
        require(self.failed is None and self.active is not None, 'No offered case')
        case = self.cases[self.active]
        require(case['state'] == 'OFFERED' and token == case['token'] and
                isinstance(owner_reply, str) and owner_reply.strip(), 'Stale/missing readiness reply')
        case.update(state='READY', ready=now, owner_reply=owner_reply)
        self.event('owner_ready', now, token=token, reply=owner_reply)

    def begin_case(self, token, now):
        self.tick(now)
        require(self.failed is None and self.active is not None, 'No ready case')
        case = self.cases[self.active]
        require(case['state'] == 'READY' and token == case['token'], 'Case not owner-ready')
        case.update(state='CAPTURING', start=now, start_offset=self.offset,
                    deadline=now + case['seconds'])
        self.event('case_started', now, token=token, deadline=case['deadline'])

    def end_case(self, now, state):
        case = self.cases[self.active]
        case.update(state=state, end=now, end_offset=self.offset)
        self.event('case_ended', now, case=case.copy())
        self.active = None

    def disconnected(self, now):
        self.tick(now)
        self.fail('Serial connection lost; no reopen permitted', now)

    def close(self, now):
        self.tick(now)
        if self.active is not None:
            self.end_case(now, 'INCOMPLETE')
        self.raw_file.close()
        result = dict(status='INVALID' if self.failed else 'CLOSED_NOT_QUALIFIED',
                      error=self.failed, raw_sha256=self.digest.hexdigest(), bytes=self.offset,
                      boot_count=self.boots, serial_writes=False, reset_requested=False,
                      expected_elf_sha256=self.expected_elf, physical_acceptance='NOT_ASSESSED',
                      unterminated_bytes=len(self.pending), incomplete_frame=self.frame,
                      frames=self.frames, cases=self.cases)
        # A case slice preserves *all* its bytes. Complete frame offsets are an
        # additional view, never a way to repair or qualify an invalid session.
        for case in self.cases:
            if 'start_offset' not in case:
                continue
            begin, end = case['start_offset'], case['end_offset']
            with (self.output / 'stream.log').open('rb') as source:
                source.seek(begin)
                raw = source.read(end - begin)
            with (self.output / (case['name'] + '.log')).open('xb') as target:
                target.write(raw)
            case['raw_sha256'] = sha(raw)
            case['complete_frames'] = [i for i, frame in enumerate(self.frames)
                                      if begin <= frame['start'] and frame['end'] <= end and
                                      case['start'] <= frame['received_start'] <= frame['received_end'] <= case['deadline']]
        self.event('session_closed', now, status=result['status'], raw_sha256=result['raw_sha256'])
        self.events.close()
        self.closed = True
        result['events_sha256'] = sha((self.output / 'events.jsonl').read_bytes())
        (self.output / 'result.json').write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')
        return result


def pump(capture, reader, clock, control):
    """Drive one already-open passive reader; never open, reset, write or retry.

    A future bound adapter must set a <=0.1s read timeout, verify the port and
    DTR/RTS=false before opening, and supply immutable operator-control events.
    control(capture, now) returns True to stop, including between cases.
    Reader/control exceptions invalidate the session and always close it.
    """
    try:
        while capture.failed is None:
            capture.tick(clock())
            if capture.failed or control(capture, capture.last):
                break
            capture.feed(reader.read(4096), clock())
    except Exception as error:
        capture.fail('Reader/control exception: ' + repr(error), capture.last)
    finally:
        try:
            reader.close()
        except Exception as error:
            capture.fail('Reader close exception: ' + repr(error), capture.last)
        result = capture.close(capture.last)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--replay', type=Path, required=True)
    parser.add_argument('--resources', type=Path, required=True, help='JSON object of reserve limits')
    parser.add_argument('--elf', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    capture = ContinuousCapture(args.output, json.loads(args.resources.read_text()), args.elf)
    now = 0.0
    try:
        with args.replay.open('rb') as source:
            while chunk := source.read(4096):
                now += .001  # Synthetic receive times, never device evidence.
                capture.feed(chunk, now)
                if capture.failed:
                    break
    finally:
        result = capture.close(now)
    print(json.dumps(dict(status=result['status'], error=result['error'],
                          complete_frames=len(result['frames']), replay_only=True)))
    if result['error']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
