# Continuous producer campaign runbook: 42/43

This runbook defines one new JC1060 rev103 experiment, reusing the exact
producer image from source `e9ce242`. Native minimum remains **302**. No
decoder, buffer, gain, task, peripheral or normal-image promotion is included.
The current accepted s07 state has 42 executed predecessors and full SHA-256
`28830827d3e3426bbcd58b9a25d4d2129e28f150bade33fe3c551cd577f93921`.
Old 40/41 plans are consumed and must not execute again.

## Entry gates

Before any COM3 access, publish and verify new candidate/return 42/43 plans,
the complete factory-to-current ancestry, four factory copies, source/package
hashes, this runbook, capture contract, boot policy and separate reviewed
write/capture/boot bindings. Exact bound source must have completed review
and CI. Retain primary and secondary registration/tool archives with CRC and
every member hash verified. Preparation is offline and not a device result.

Refresh the real unit topology with the owner: COM3/full-speed USB-C,
USB-only board power, FLX4, SD/library, MASTER speakers and controller
headphones. Record current screen/mode. This is an identity/topology refresh,
not another s07 recovery qualification. Use the already owner-authorized
software USB ROM entry where supported, recording exact identity/security.
Do not claim a software reset is a physical power removal. Do not force an
operation after an unexpected identity, security state or flash predecessor.

The new writer remains no-reset and uses the unchanged shared guard. It
requires fresh rev103/MAC/security/RDID, all 16 MiB predecessor equality,
span readback and independent all-16-MiB after equality. No eFuse, erase-all,
force or changed flash mode/frequency/size options. Keep programmer evidence
and close its port. Candidate app SHA-256:
`173933e100e2de34257beb89013556b0937e8b0e94bd36bf797eb755d505d1cd`;
ELF `54dbdc424791bfc3f3516dd194487bd0a09e009bf7215e380c7a57b10d58af11`.

## One passive session

Start the bound continuous adapter against that fresh candidate result proof.
DTR/RTS stay false and no serial commands are sent. Pre-ELF cold wait is at
most 600 seconds; only absent-port/enumeration races may retry. After the
expected ELF, never close/reopen between cases. Any lost port invalidates the
session. Preserve the entire raw stream, receipt journal, exact command files,
frame/case offsets, checkpoints and failed attempts.

For the candidate cold measurement only, prepare the monitor, then ask the
owner to remove USB power for 5-10 seconds and reconnect without BOOT/RESET.
Retain their actual reply using the numbered `confirm-cold` command. Its
deadline is 120 seconds after the 45-second cold bound. No placeholder owner
text may be substituted. Expected ELF/reset_reason=1/rev103, one
PREPARING->READY before UAC start and fresh coherent telemetry are mandatory.

| Case | Duration, including setup | Required observation |
|---|---:|---|
| cold-1 | 45 seconds after ELF | empty decks; >=7000 callbacks |
| idle-1 | 60 seconds after admitted cold | empty decks; >=9000 new callbacks |
| s03-deck1 | 180 seconds | known Disturbed MP3 on deck1, deck2 empty |
| s04-deck2 | 180 seconds | deck1 paused, known Ben Folds Five MP3 on deck2 |
| s05-both | 60 seconds total | both known loaded MP3 sources playing |

Only cold->idle is automatic. Each functional case is offered only after
previous admission. Ask the owner to be ready for that case's selection/load/
play sequence before publishing its actual `ready` reply and `start` token.
The 120-second readiness bound includes the ready->start delay. Then instruct
the requested load/play/pause actions while the live case is active. Keep
normal level and Headphones MIX toward MASTER. No PFL, transport-CUE, extra
browse or other commands during stable observation. Retain actual source
names/formats and listening replies separately from numerical results.

All setup/load/play consumes the stated window. Never start a timer while
the owner is unprepared, extend a window waiting for playback, retrospectively
count a late action or repeat an invalid case to manufacture admission.
Each playing deck needs at least 20 actual source-time seconds in adjacent
stable intervals with frame production. Both-playing needs 20 overlapping
source-time seconds. Requested wall duration is not measured playback.

Checkpoint prefix copying/analysis uses one background worker while serial
reads/fault watching continue. Pending analysis is bounded to 30 seconds;
readiness and analysis wait do not extend any case. No receive-freshness
exception is allowed for analysis. Full session is bounded to 1800 seconds
and 64 MiB; these are ceilings, not permission to extend a case. Revalidate
whole-session hashes, receipt events, offsets and all inter-case reports.
The final entire stream must validate after port close as well.

## Stops and interpretation

Stop extension at the first all-fill >=6000 us, fill failure, USB iso/packet/
submission error, source underrun/media failure, reserve failure, log drop,
unavailable/invalid/saturated sample, unexpected boot/disconnect, malformed or
missing evidence, stale timestamps, failed readiness/admission or user stop.
Preserve the short/failed case and all pending results. Never continue playback
to meet a minimum duration after a fault.

Resource floors remain USB stack 2048 B, SD-client 8192 B, logger 1024 B,
internal heap 65536 B, PSRAM 8388608 B, observed largest blocks internal
8192 B/PSRAM 262144 B. The <=3000-us headroom target is separately reported;
a target miss does not rewrite the independent hard-stop rules. Host/device
receive offsets may spread at most 2 seconds, source timestamps at most
10 seconds from receive anchors; complete reports cover both case edges
within 10 seconds. No aligned slice can erase an invalid whole-session event.

These are wall observations including waiting/preemption and asynchronous
snapshots, not CPU time or hardware IRQ latency. No normal dual-deck/PFL/
reconnect/soak/product acceptance follows automatically. Select a single
root-cause change only from valid source/USB measurements.

## Mandatory automatic exit, without repeated manual s07 checks

After completion, failure or cancellation, close the passive port. If the
candidate was written, use the registered return plan against 43 predecessors
to restore exact accepted s07. Even if the analyzer fails, recovery remains
available independently; admission is never a recovery prerequisite. Retain
fresh identity/security/full candidate equality, written-span readbacks and
independent full s07 readback. Expected s07 app:
`95d45203ec245a9f82455f16f15dd95ed6ad2bb8eaaca9cf85f0816ccd5dbfdd`;
ELF `005f8d1e33aa5f2a8d05af691ce768461f559c917b8804920a912178f5b78ddf`.
Reconstructed whole flash afterward must again be `28830827...`; history
then becomes 44. A mismatch stops automatic actions for inspection.

Follow the [owner recovery policy](../RECOVERY_POLICY.md): **no repeated manual
s07 physical cold, listening, Pause/CUE or confirmation sequence**. Readback
closes the accepted-image recovery step. With the writer/monitor closed, use
the separately bound one-shot software boot action against the new verified
return proof. Record attempted/completed pulse and port close; do not retry
a failed pulse. Success means `BOOT_PULSE_ISSUED_APPLICATION_NOT_OBSERVED`,
not proof of a running app or physical cold boot. No further owner recovery
qualification is requested. Preserve a failed pulse/unknown state explicitly.

Archive completed evidence on both drives, verify CRC/all members/secondary,
record actual executed history/full state and any NOT_RUN cases. Do not merge
or promote an experimental image from preparation or partial results.
