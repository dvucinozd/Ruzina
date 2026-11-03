# Continuous COM3 adapter and whole-stream analyzer

This continues the [offline capture core](2026-11-03-continuous-capture.md).
The adapter/analyzer are implemented and tested offline; **no new campaign is
registered or authorized by the old bindings**. No COM3 connection, flash,
reset or candidate measurement was performed. Installed state remains s07,
history 42/full `28830827...`. Native minimum remains 302. The prior `ad28067`
publication passed all 22 GitHub jobs, including separate normal-engine and
producer-profile builds; this source's CI is separate.

## Passive adapter

[`continuous_adapter.py`](../../tools/source_supply/continuous_adapter.py)
defaults to offline contract validation. Execution additionally requires a new
hash-pinned contract, `REVIEWED_CONTINUOUS_CAPTURE_TOOLS` binding and completed
candidate programmer proof against 42 predecessors. The binding covers all
eight transitive Python tool files and the unchanged shared writer. The old
40/41 plans/binding are rejected. No serial import or device lookup happens
until offline proof checks pass.

The [contract validator](../../tools/source_supply/continuous_contract.py)
fixes the exact JC1060 unit/MAC/rev103, source/image, resources and native 302
minimum. Candidate proof checks the new registration/plan, identity/security,
both full 16 MiB files, all three readback spans and app/ELF bytes. These are
retained file checks; the capture adapter does not perform flash operations.
Future registration still needs the complete planner ancestry and recovery
artifacts. A capture contract is not a replacement for writer preflight.

The passive reader sets DTR/RTS=false before opening COM3 with a 0.1-second
timeout. Before the expected ELF, absent-port/enumeration races may retry under
the 600-second cold wait. A listed busy port fails immediately. After the ELF,
any disconnect stops the session; the reader never reopens it. It never writes
serial commands, resets or invokes a programmer. Close failures stay errors
and cannot be reported as a closed port.

## Cases, readiness and admission

| Case | Bound including setup | Requested playing state |
|---|---:|---|
| cold-1 | 45 seconds after expected ELF | empty / empty |
| idle-1 | 60 seconds | empty / empty |
| s03-deck1 | 180 seconds | playing / paused or empty |
| s04-deck2 | 180 seconds | paused / playing |
| s05-both | 60 seconds total | playing / playing |

The cold case starts automatically on the expected ELF. Admission also needs
the owner's physical-power confirmation, reset_reason=1, chip_revision=103,
PREPARING->READY before the single UAC stream start and sufficient complete
fresh telemetry. Software reset is not physical cold evidence. The confirmation
deadline is 120 seconds after the cold case bound. Idle starts automatically
only after cold admission.

Each completed case creates an immutable checkpoint while the serial reader
remains open. A reader-thread metadata snapshot fixes the exact byte prefixes;
a single background worker copies only those prefixes and runs analysis while
the reader continues receiving and watching faults. It never reads the live
capture's mutable metadata. No latency exception is added to freshness checks.
Pending admission has a 30-second bound, separate from case durations. Analysis
must pass before the next case is offered. Failures
retain `analysis-attempt.json`, invalidate the session and close the port.
There is no repeated or extended window to manufacture missing playback.
The adapter stops after five admitted cases or the first fault/invalid input.
After closing the port it reanalyzes the final entire session, including bytes
received during the last pending admission. A valid older checkpoint cannot
override a failing final stream. Background work is joined only after closing
the reader, including fault exits.
The existing <6000-us, USB, underrun, resource and log-drop gates remain. A
3000-us headroom miss remains separately reported, not a rewritten hard gate.

`status.json` exposes the current case, token and next numbered command.
Publish local commands as complete UTF-8 JSON using a temporary file followed
by atomic rename into `commands/000001.json`, then the next number. The tool
retains exact accepted bytes in `accepted-commands/` and detects later edits.
Do not pre-fill owner replies or reuse tokens from another case.

```json
{"sequence":1,"command":"confirm-cold","reply":"ACTUAL owner reply about physical cold boot"}
```

For each functional case, ask the owner to be ready for the requested known
track selection/load/play sequence. After their actual reply, use that case's
token in separate ready and start commands, in sequence:

```json
{"sequence":2,"command":"ready","token":"CURRENT token from status.json","reply":"ACTUAL owner readiness reply"}
```

```json
{"sequence":3,"command":"start","token":"SAME current token"}
```

The 120-second readiness maximum includes delay between ready and start.
Readiness does not count as measured playback. Setup/load/play consume the
same case bound after start. A stop command contains only sequence and
`"command":"stop"`; partial results never become a complete five-case run.

## Analyzer

[`continuous_analyze.py`](../../tools/source_supply/continuous_analyze.py)
first verifies the whole raw/journal hashes and replays every byte chunk,
framing event, owner exchange, case transition and checkpoint into a fresh
capture. Stored frame/case offsets, times, hashes, final metadata and journal
must exactly match recomputation. Case files must equal the original raw byte
ranges. Invalid whole sessions cannot be rescued by an aligned slice.

Each nonempty read now has a chunk hash and host receive time in the journal.
Complete reports between cases also undergo numerical integrity checks. The
runtime watcher now checks every complete line, including `rust_load` media
failures/reserves outside report frames; the initial core missed that path.
Old core archives are retained unchanged. New journal data is required by this
analyzer; no retrospective timestamps are invented for earlier captures.

Heartbeat/device clocks must advance, with host/device offset spread <=2 s;
source/owner timestamps must remain within 10 s of receive anchors. Complete
reports must cover each case's first/last 10 seconds. This rejects buffered
backlogs and silent measurement tails. These are receive/freshness checks,
not CPU-time or hardware IRQ-latency measurements.

The existing same-window partitions, monotonic counters, coherent snapshots,
requested state and resource/callback checks remain. Cold needs >=7000
callbacks; idle needs >=9000 new callbacks. Playing cases additionally need
20 actual source-time seconds in adjacent stable intervals and produced
frames. Both-playing needs >=20 s of overlapping stable source intervals;
two separate one-deck episodes cannot count. No requested wall duration is
reported as measured playback. Snapshots remain asynchronous observations.
Outputs are development continuation evidence, not product/PFL acceptance.

## Offline validation and next work

37 source-supply tests pass on Windows and Linux. The full synthetic five-case
controller run uses exactly one passive port open/close and passes whole-stream
replay. Negative cases cover altered metadata/slices, receive backlog, silent
case tails, short source intervals, disjoint both-playing intervals, wrong
reset/readiness, missing physical confirmation, media faults between cases,
busy/enumerating/disconnected ports, edited owner commands and consumed plans.
No synthetic transcript or owner placeholder is board evidence.

The first connected CLI review produced three issues (one major, two minor),
then failed with `Connection failed: WebSocket closed`; it is not a completed
review. Its issues were verified and addressed: background snapshot/analysis
instead of blocking reads, storing the exact readiness bound for replay rather
than reconstructing it by floating-point subtraction, and fixing handoff prose.
Two additional regressions cover a deliberately held worker while receiving
and whole-journal replay across a floating-point exponent boundary. Combined
source-supply suite now has 46 cases per OS, including the separate boot tests.
Ruzina's GitHub CodeRabbit installation is confirmed by its PR82 bot response;
the PR remains draft. Successful repeat review is still required separately.

The separate [software boot action](2026-11-03-software-boot.md) is now
implemented with seven further tests (44 source-supply tests total per OS).
No boot action or target access was executed. The first review attempt was
blocked by the CodeRabbit CLI rate limit; dashboard installation and tests do
not constitute a review pass.

Before target use: complete review/CI and retain exact tool evidence, then build/register/publish new
42/43 plans, contract and binding. Reuse the retained exact producer image;
do not optimize the decoder based on the failed S03 attempt. Candidate physical
cold and source measurements keep their own requirements. Apply the
[owner recovery policy](../RECOVERY_POLICY.md): automatic span/full readback
and separately recorded software boot, without repeated manual s07 tests.
The adapter closes its port and reports restoration required; it does not
perform or silently claim recovery. The campaign orchestrator owns that exit.
