# Continuous producer capture: offline core

The next capture implementation keeps one passive reader open across idle and
functional cases, preserves the whole stream and binds case boundaries to an
explicit owner-ready exchange. This addresses capture/reopen and coordination
risks from the [previous attempt](2026-11-03-source-supply-device.md). It does
not establish the cause of the joined producer/heartbeat line or fix dual-MP3
playback. No COM3 access or target write is part of this checkpoint. Current
device state remains accepted s07, executed history 42/full `28830827...`.

## Implemented contract

[`continuous.py`](../../tools/source_supply/continuous.py) is a separate core and
offline replay command. The historical four bound campaign files and the
shared writer remain unchanged. Existing execution bindings cannot authorize
this new file. The reader pump accepts an already-open reader; it never opens,
reopens, writes to, resets or programs a device. A lost connection stops the
session. The future adapter must verify identity/control lines before opening
and impose a read timeout of at most 0.1 seconds.

`stream.log` retains every delivered byte, including the entire chunk containing
a fault. `events.jsonl` appends host receive times, byte offsets, complete report
boundaries and case transitions. Final hashes cover both files. Fresh output
directories, unique safe case names and exclusive file creation prevent reuse.
The core has a separate 1800-second session ceiling and 64 MiB byte ceiling;
neither extends a case. They are host bounds, not target timing budgets.

Framing follows the actual producer-profile logger order: both deck headers
with six stages each, owner, three USB service rows, water/stats, first fill,
three fill phases, resources and deck-engine state. Complete heartbeat/media
lines can occur between these rows. Arbitrarily split reads are reassembled;
missing, repeated or reordered report rows, mismatched stage sequences,
duplicate fields, embedded report prefixes and invalid UTF-8 fail closed.
Runtime fault watching retains the previous error/resource/6000-us gates and
runs even while waiting for readiness. An incomplete final frame/line remains
explicit in the result and original bytes; it never becomes a complete frame.

Each case has a unique token. Offer -> owner-ready reply -> explicit start are
separate recorded events. A stale token cannot start the next case. Readiness
has a 120-second maximum, including delay between acknowledgement and start;
it never counts as a measurement. Once started, setup/load/play all consume the
same bound: idle 60 seconds, one-deck functional 30-180 seconds, both-playing
60 seconds total. The next registered runbook will choose exact durations.
No automatic wait for playing state extends a case.

The case `.log` retains every byte received during its host window, including
partial boundary frames. Complete-frame offsets are an additional view of the
same stream, never edited replacement evidence. A frame crossing a boundary
stays in the whole stream but cannot donate evidence to either case. Data
received after a deadline cannot enter the expired case. Host receipt time
does not prove wire emission time or device freshness.

All outputs are **NOT_QUALIFIED**, or **INVALID** on an error. A complete frame
means structural completeness only. This module does not grant continuation,
cold acceptance, stable throughput, PFL acceptance or source/resource reserves.
Numerical partitions, counters, stable requested source state, boot readiness,
source-time freshness and cold identity still require the registered analyzer.

## Verification and output-path inspection

The 25 source-supply tests pass on Windows pinned Python and WSL Linux: the
existing 12 plus 13 new framing/coordination/pump regressions. They exercise
seven-byte reads, delayed/empty reads, one reader across cases, stale replies,
readiness expiry, setup timing, late reads, cut frames, disconnects, unexpected
boots, injected writer fragments, clock faults and immediate runtime stops.
The exact first line from the retained failed S03 capture is a regression input:
it remains invalid with `Duplicate telemetry fields`, without repair or skip.
Synthetic tests and replay times are never board evidence.

Local pinned ESP-IDF `fff9895c82d744c7237be8847347bdd1b07c6643` inspection found
`components/esp_libc/src/picolibc/picolibc_init.c` configures buffered stdout,
and `components/esp_driver_usb_serial_jtag/src/usb_serial_jtag_vfs.c` has a
write lock. Its no-driver transmit function explicitly drops bytes after a
host-drain timeout; the driver variant also has bounded character sends.
These are possible loss paths during a closed console. They do not prove
which path caused S03, or prove cross-task interleaving. No libc/firmware
locking, buffer or stack changes were made on this evidence alone.

## Remaining work before another target attempt

1. Bind a live adapter to this core: exact port/control-line checks, physical
   cold wait/ELF/reset-reason/readiness verification, one uninterrupted reader
   afterward, local control-file token validation and bounded fault stops.
   It must not reopen the port between cold, idle and source cases. A missing
   cold identity, disconnect, invalid report or readiness timeout triggers exit.
2. Add a continuous-stream analyzer that revalidates hashes/offsets/events,
   all case bytes and global faults. Require a valid same-session cold anchor;
   numerical/source freshness and existing continuation gates remain mandatory.
   No use of an aligned slice can erase an invalid whole-session event.
3. Prepare a separately reviewed/registered software boot action for an exact
   s07 return. Preserve the no-reset writer and automatic span/full readback;
   follow the [owner recovery policy](../RECOVERY_POLICY.md), with no repeated
   manual s07 cold/listening/Pause/CUE tests.
4. Register new 42/43 predecessor plans, runbook and all new tool hashes,
   review/test/CI, then freeze/archive/publish before COM3. Old 40/41 plans are
   consumed. Do not change the native minimum 302 or accepted audio behavior.
5. Run only the registered candidate cases with the owner ready for source
   selection/loading before each bounded functional window. Source isolation
   precedes both-playing. Root-cause changes follow valid measurements.
