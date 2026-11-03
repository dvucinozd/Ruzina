# UAC phase diagnostic: device execution

Date: 2026-11-01. Unit: JC1060 NewPanel ESP32-P4 v1.3/103 on COM3 only.
This is the owner-assisted continuation of the [registered clean-build checkpoint](2026-11-01-uac-timing-execution.md).
The [earlier 12260 us failure](2026-11-01-review-fix-device-results.md) remains
unchanged. This diagnostic adds measurement only; it cannot establish an
optimization win. Native minimum revision 302 and accepted gain remain intact.

## Registration and entry

- Clean source: `5c579e6e287362af36924f30919d6d0ea911ad59`, published on
  [PR #74](https://github.com/dvucinozd/Ruzina/pull/74); documentation checkpoint
  `1265b079dca53e463e84296d3ae1711a00ed1b86`.
- Candidate app: `d4a577493512df5988927ac4dc765e0e796861f2ff3c9e0e1dbad55455d10902`,
  817344 bytes, clean-source build. Source-head CI 19/19 and committed-diff
  CodeRabbit 0 issues passed before entry.
- Diagnostic plan: `bfef28135fb73f522893d58209cbda40d55e21f3b092efa620b383125dba3335`.
- Return-to-s07 plan: `4312e199ea36a2ef4262f3b5999397be7d9a4b5c57c6e4be3fc7a04b0b040a84`.
- Expected complete current predecessor: 27 plans, full flash SHA-256
  `c11aa1f7a8ff8121888f9d7a70d247a447f381bb2d07b63ebd8cf7f69633f76f`.
- Both sector plans and a CRC/content-hash-verified binary archive have verified
  secondary copies. All four factory backups are checked by the programmer.
- Before the new run, retain the aggregate >=6000 us rejection criterion and
  <=3000 us headroom target. Capture first-connection and boot-cumulative phase
  counters separately. No playback or I02–I13 in this diagnostic run.
- Owner replied **"jesam"** to the instruction to stop playback and perform a
  5–10 second USB disconnect/manual BOOT entry, with the same COM3/full-speed
  connector, FLX4 attached and no added board power. That is a text confirmation,
  not an independently inspected power topology.

## Current state

The guarded programmer confirmed revision 103, MAC `e8:f6:0a:e7:22:1d`,
RDID `0x184068`, security flags 0 and encryption count 0. All four factory
backups passed again. The actual 16 MiB predecessor exactly matched the
registered `c11aa1f7...` full state. Only then was the candidate written;
all three ranges passed readback. No eFuse write or reset was requested.
Raw programmer records and the complete prewrite readback have secondary
copies with matching SHA-256 values.

The owner then cold-booted without BOOT/RESET and confirmed the track list
returned. The passive monitor captured the expected ELF identity
`15c9551e9c868e94568493f552a83a01cf52c5a8aa4b0b43a459ad3c59999b0e`, reset
reason 1 and one started four-transfer stream. No playback was requested.

## Phase result

Raw log SHA-256: `5c29730744e2bc3f7fe85665315e230122768bd6f82509401b4e5e7ae13f500c`,
19759 bytes. The bounded monitor observed 40 seconds after boot identity
(68.9 seconds including operator wait) and six periodic snapshots.

| Phase | Final count | First us | Max us | >3000 us | >=6000 us | Failed fills |
|---|---:|---:|---:|---:|---:|---:|
| initial_prefill | 4 | 1844 | 1844 | 0 | 0 | 0 |
| recovery_prefill | 0 | no sample | no sample | 0 | 0 | 0 |
| callback | 6000 | 912 | 10373 | 4 | 3 | 0 |

First connection fill was initial prefill before any audio submission. Its
maximum-context flag is also before submission. The callback maximum occurred
after an audio submission. All four >3000 us callbacks (including all three
>=6000 us callbacks) are already present in the first 1000-callback snapshot;
their cumulative counts and maximum did not increase through callback 6000.
The aggregate `fill_max_us` also reports 10373, retaining the original meaning.

**Registered aggregate timing criterion: FAIL.** Instrumentation successfully
localized this run's outlier to callback refill. It did not produce a timing
improvement and does not identify the slower substage or distinguish execution
from preemption. The first-fill/lazy-construction hypothesis does not explain
this run's observed maximum; no inference is transferred to the older image's
unclassified 12260 us measurement.

Idle underruns were 0/0 throughout, USB isochronous/packet/submit errors zero,
and there were no fatal markers. Producer maximum gap was 11083 us. Decks were
idle with no buffered source, so `low_us=UINT32_MAX` and prime duration zero
mean no playing-supply/prime observation, not a large measured ring margin.
Recovery timing and all playback cases remain NOT_RUN.

## Return checkpoint

The diagnostic monitor is closed. The first restore attempt passed identity,
security and RDID but lost the USB connection while starting the full prewrite
read (`SerialException`, Windows device-not-functioning error 31). Its result
records `flash_write_attempted=false`, no eFuse write and no requested reset.
The failed log is retained; it is not a flash write or a successful restore.

After the owner confirmed fresh ROM entry, the second restore attempt matched
the entire 16 MiB diagnostic state, SHA-256
`9e7c084f89f4b37940606d2dfd0828922fe7005a6ceca4daf70d4599bc81f9de`.
It then wrote exact accepted s07 app `95d45203ec245a9f82455f16f15dd95ed6ad2bb8eaaca9cf85f0816ccd5dbfdd`
and verified all written sector readbacks. No eFuse changes were made.

The owner cold-booted without BOOT/RESET and confirmed the track list returned.
The boot log has expected s07 ELF `005f8d1e3...`, reset reason 1, no fatal markers
and a started stream with zero USB errors. S07 is now installed. In the separate
playback window the owner replied **"potvrđujem"** to normal sound in both ears,
Pause and CUE at unchanged level. The 65.2-second passive window captured
675072 played output frames (14.064 s at 48 kHz), a 44100 Hz source, zero
underruns and zero USB isochronous/packet errors. The final ring was 65464
frames. These are short restore observations, not a soak or diagnostic-image
acceptance. No boot line is expected in that playback window; the preceding
cold capture binds the image. All monitors are closed.

Future predecessor history contains 29 plans. The expected full restored state
is `3130901f09e304949556d582b46b3b8e417713600317bce755bb819b6f876a40`;
this is reconstructed from the validated prewrite state and readback-verified
written ranges, not a separate post-restore full read. Do not reuse the old
27-plan current-state assumption.

During final evidence verification, the Git blob of the return-plan copy was
found to have normalized line endings in registration commit `1265b07`. The
external plan used by both programmers retained the correct registered hash
`4312e199...`; device payloads were unaffected. An exact-path `-text -whitespace`
attribute and index renormalization restore the repository copy's original
bytes. Both plan blobs must be checked against their recorded hashes before
publication; this correction does not rewrite the historical external plan.

## Evidence and next action

The [structured result](../boards/readiness/jc1060-uac-timing-device-20261101.json)
indexes immutable raw logs/results, exact app/ELF/plan identities and operator
acceptance. All completed raw records and binary readbacks have hash-matched
secondary copies. The [device archive index](evidence/2026-11-01-uac-timing/phase-device-archive.json)
records 45 archived files with CRC/content verification and an identical secondary
archive SHA-256. Full flash and binary payloads stay outside Git.

Current installed app is **s07 deck-play**. The instrumented candidate remains
unqualified; playback and recovery timing on it are NOT_RUN. Implement the
[callback profiling plan](2026-11-01-uac-callback-profiling-plan.md) next. No gain,
DSP, priority or native revision changes were made to obtain these measurements.

Primary evidence: `D:/BoardEvidence/JC1060-20261030-COM3/uac-timing-20261101/`.
Secondary evidence: `C:/Users/Daniel/Documents/BoardBackups/JC1060-20261030-COM3/uac-timing-20261101/`.
