# Source-supply campaign: cold/idle pass, playback not captured

Date: 2026-11-03. JC1060 NewPanel rev103, COM3/full-speed USB-C, USB-only board
power, FLX4 with MASTER speakers and controller headphones. Native minimum
remains 302; this uses the pinned ESP-IDF 6.1 backend. No other board inherits
acceptance. No gain/task/buffer/affinity/timing-limit changes were made.

Status: **SOURCE_SUPPLY_COLD_IDLE_PASS_PLAYBACK_NOT_CAPTURED_S07_RESTORED_OWNER_CONFIRMED**.
The dual-MP3 root cause is still open. No source-throughput, decoder CPU-time,
normal two-deck, PFL, reconnect, soak or promotion result follows from this run.

## Identity and executed recovery

Read the [source record](2026-11-03-source-supply-profile.md),
[registration/tool binding](2026-11-03-source-supply-execution.md) and
byte-frozen [runbook](2026-11-03-source-supply-campaign.md). Final tool source
`bb96132` passed review with 0 issues, 22/22 CI, 12 tests per OS and 13 shared-guard
WSL cases. Publication `0d9746636e0beac3a2fd39e39c83ff232580af97` and matching
remote SHA preceded device access. Binding SHA-256:
`a63241b1c2c64f33a1c0d127e0489fdc1a86064a9c78526dd95c462ea18608dc`.

Candidate app `173933e100e2de34257beb89013556b0937e8b0e94bd36bf797eb755d505d1cd`,
ELF `54dbdc424791bfc3f3516dd194487bd0a09e009bf7215e380c7a57b10d58af11`,
producer-profile=true, UAC/IRQ profile=false, pack-IRAM=false. Owner-authorized
software USB ROM entry freshly confirmed rev103/MAC/security; guarded writes
also checked RDID and all 16 MiB live predecessor bytes before any write.
Each written span and independent full flash matched afterward.

| Executed transition | Prior history | Whole flash afterward |
|---|---:|---|
| Producer candidate | 40 | `95e069e0ebf04452f983f192520789e30d09449d3853f57bd0cb0eaa50a048e9` |
| Exact s07 return | 41 | `28830827d3e3426bbcd58b9a25d4d2129e28f150bade33fe3c551cd577f93921` |

Current executed history is **42**. Exact accepted s07 app remains
`95d45203ec245a9f82455f16f15dd95ed6ad2bb8eaaca9cf85f0816ccd5dbfdd`,
ELF `005f8d1e33aa5f2a8d05af691ce768461f559c917b8804920a912178f5b78ddf`.
The new full hash includes preserved longer-candidate sector tails. Future
plans need all 42 executed predecessors, not the earlier 40/41 chain.

## Captured cases

| Case | Actual evidence | Result |
|---|---|---|
| S01 physical cold | Owner `vraća`; 45.072 post-ELF seconds, 7000 callbacks, all-fill max 1880 us (callback max 1598), zero crossings/errors/underruns; complete coherent producer/owner/USB rows | Admission PASS |
| S02 separate idle | 60.107 connected seconds; callback 11000->20000, 9000 new; same maximum, no errors; all snapshots/partitions/resources pass | Admission PASS |
| S03 deck 1 attempt | 180.093 connected seconds elapsed with no loaded/playing deck or source frame production. First producer line is malformed; frozen analyzer rejects duplicate fields | Playback NOT_CAPTURED; analysis INVALID, no continuation |
| S04 deck2 / S05 both | Not started after S03 incomplete/invalid evidence | NOT_RUN |

Cold/idle observed USB stack minimum 4964 B, SD client 58044 B, logger 1560 B,
internal heap minimum 414548 B, PSRAM minimum 28054272 B; smallest observed largest
blocks 249856 B internal / 28311552 B PSRAM. These pass frozen development
reserves. Largest-block values are asynchronous observations, not continuous
minima. These idle metrics do not qualify playing-source resources.

The 180-second S03 window ended before a source load/play was captured. It was
not repeated or extended. Its first line is retained exactly:

```text
producer_profile deck=1 generation=1 schema=1 seq=248 uptime_us=heartbeat=140 uptime_us=140372185
```

The analyzer correctly reports `Duplicate telemetry fields`; no fabricated
summary or repaired raw data is used. The source of this joined/partial prefix
is unconfirmed. Buffer loss/stale bytes around port reopening and concurrent
stdout writers are hypotheses; this line alone does not prove concurrency or
a decoder fault. Cold/idle rows were complete and validated; they remain
separate evidence. The capture boundary and operator timing both need work
before another functional attempt.

The owner's `ekran se ugasio` reply arrived after software ROM entry for the
registered return. Current screen-off was expected during programming; the
exact earlier display-change time is unknown. No fatal marker or application
reboot was captured. Do not count that reply as source listening acceptance.

## Restored state and archive

The s07 return matched every span and all 16 MiB. Physical cold boot captured
the exact s07 ELF/reset_reason=1/UAC stream; owner confirmed `vraća`. Known deck 1
playback/Pause/transport-CUE was owner-confirmed `sva je ok`. Actual captured
played-frame progress is **9.276 seconds**, zero USB iso/packet errors and zero
underruns. This is a restore check, not a measured 20-second or two-deck pass.
All programmers/monitors are closed.

[Device summary](evidence/2026-11-03-source-supply/device-summary.json),
[current progress](evidence/2026-11-03-source-supply/device-progress.json),
[analysis policy](evidence/2026-11-03-source-supply/analysis-policy.json) and
[archive index](evidence/2026-11-03-source-supply/device-archive.json) preserve
the exact run. `completed-source-supply-device.zip`: **37678594 bytes**,
SHA-256 `4576676bf886ba3150bcc464678814bf5a65cf3dcd4c2acda14f30122e2eccbd`.
CRC, every member and every secondary member/archive match. Primary:
`D:/BoardEvidence/JC1060-20261030-COM3/source-supply-20261103/`;
secondary: `C:/Users/Daniel/Documents/BoardBackups/JC1060-20261030-COM3/source-supply-20261103/`.

After this completed return, the owner removed future repeated s07 cold/
listening/Pause/CUE recovery tests. Follow the [new recovery policy](../RECOVERY_POLICY.md).
Keep automatic readback; preserve the historical evidence/runbook unmodified.

## Next ordered work

1. Reproduce port-close/reopen, delayed reads, partial/stale prefixes and
   concurrent writer fragments offline with deterministic fake serial data.
   Keep invalid data explicit; no heuristic repair that grants a pass. Inspect
   the pinned Picolibc/USB Serial-JTAG output path if writer interleaving needs
   confirmation. No target decoder optimization is justified yet.
2. Prefer one continuous passive serial session across cold/idle/source cases,
   with explicit immutable offsets/markers and complete-frame boundaries for
   per-case evidence. Test continuity, bounded failure stop, invalid frames,
   owner-ready coordination and exact cold identity. Retain the original full
   stream, not only extracted case slices. Do not blindly add large logger
   stack buffers or change the firmware to hide dropped telemetry.
3. Coordinate readiness before starting each bounded functional window. Allow
   the requested track selection/load, then request no additional browse/PFL
   actions during the stable measurement. Capture setup versus playback times
   separately. No load/play after the window may become evidence for it.
4. Review/test/publish the revised capture/analysis tools. Register a new exact
   producer-profile candidate and s07 return against **42/43** predecessors,
   starting from full `28830827...`. Apply the new automatic-readback recovery
   policy; prepare any software boot action separately from the no-reset writer.
5. Repeat only under that new registration: cold/idle, each known MP3 source,
   then bounded both-playing. Stop on unchanged integrity/resource/timing/error
   gates. Use valid measured partitions/supply to choose one root-cause fix;
   normal two-deck/PFL/transport/soak still need their own later qualification.
