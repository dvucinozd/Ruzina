# Callback substage/scheduler diagnostic: device record

Date: 2026-11-01. Unit: JC1060 NewPanel ESP32-P4 v1.3/103 on COM3 only.
Continues the [registered execution](2026-11-01-uac-callback-profiling-execution.md).
This is an OFF-by-default diagnostic, not a priority/DSP/gain fix or product
qualification. Original 12260 us and phase-run 10373 us failures remain intact.

## Registered identities and policy

- Source `8a9fe16d48387b0a3cfbd461659696091b2aa89d`, [PR #75](https://github.com/dvucinozd/Ruzina/pull/75).
  Source CI 20/20, CodeRabbit 0 issues and clean normal/profile packages passed.
- Registration commit `fc222ce4f648f6e2b9087e3079104c090ca3795e`, pushed with
  matching local/remote SHA before device entry.
- Diagnostic app `46cb70ef201c4be12415931fc3330a186dadf48d0d435121432a1731dda5a4a2`,
  820432 bytes; it explicitly enables the profiling option and matched hooks.
- Diagnostic plan `370faa84843f2ecd14c686550f6f837238d67dedeb580f757cacf97e3d2f75b6`.
- S07 return plan `7dea4155234fd4e1328856f4fcb06ca3c4b7365514f516b1f41a2ac14e8e0441`.
- Registered entry predecessor: 29 plans, expected full flash SHA-256
  `3130901f09e304949556d582b46b3b8e417713600317bce755bb819b6f876a40`.
- Four factory backup copies, source/artifact hashes, exact committed plan
  bytes and secondary binary archive are verified before entry.
- Preserve aggregate >=6000 us rejection and <=3000 us headroom target. Observe
  cold boot/idle and a separate steady idle window only. No track loads,
  playback or controls; no I02–I13. Profile scopes include observer overhead.
- Require valid exclusive partition and scheduler-window flags before
  attribution. `away_us` is completed switch-out interval in a slightly broader
  window, not CPU-only time. ISR time without a task switch is not separated.
  Start-context catalog/deck/pool fields do not identify which task ran later.
- Restore exact accepted s07 and verify cold boot and normal playback before
  ending an unsuccessful/unqualified experimental session.

## Entry and current state

The owner replied **"gotovo"** to fresh manual ROM entry after a 5–10 second
power disconnect and confirmation of COM3, the same connector, FLX4 and USB-only
board power. This is an owner text report, not independent visual inspection.

The guarded programmer confirmed revision 103, MAC `e8:f6:0a:e7:22:1d`,
security flags/encryption count 0 and RDID `0x184068`. All four factory copies
passed again. The actual full 16 MiB predecessor matched the registered
`3130901f...` state exactly before any write. The diagnostic was then written
and all three ranges matched readback; no eFuse write or reset was requested.
The owner then cold-booted without BOOT/RESET and confirmed the track list
returned. The expected diagnostic ELF is
`31a5657b7f4d4b0b7c1947f3d3d50f360ebb961380461fd185004e02f73a4dc4`,
reset reason 1. Cold and a separate steady-idle capture are complete. The later
return to s07 passed readback, cold boot and owner playback confirmation.

## Measured callback records

Cold log: 28495 bytes, SHA-256 `2c2563654974f8cb39d68951eda3acaf121aa6cef4eebc56b6210beb79b4f524`.
The monitor observed 40 seconds after boot identity (70.4 seconds with operator
wait). The final complete phase snapshot is at callback 5000; the following
USB-stat line was cut/interleaved at capture end and is not treated as a full
profile sample.

Initial prefill: 4 attempts, first/max 2024 us, no >3000 or >=6000 us samples.
Recovery prefill has no samples. Callback first time is 989 us; by the first
1000-callback snapshot there are 3 >3000 us calls, of which 2 reach >=6000 us.
Counts/maxima remain unchanged through the complete callback-5000 snapshot.

| Stored callback | Ordinal | Whole us | Rust us | Outer us | Scheduler window us | Away us | Switches |
|---|---:|---:|---:|---:|---:|---:|---:|
| worst | 709 | 13087 | 12935 | 152 | 13124 | 0 | 0 |
| latest >=6000 | 710 | 13039 | 12896 | 143 | 13067 | 0 | 0 |

Both records have generation 1, phase callback, `partition_valid=1` and
`sched_valid=1`. Their exclusive stage times sum exactly to Rust time and Rust
plus outer equals whole time. Start catalog event is 5 (preflight pass complete),
deck flags 0/0, and recorded drops 0. These are start-context fields, not proof
of what other tasks or interrupts did during the call.

| Exclusive stage | Callback 709 us | Callback 710 us |
|---|---:|---:|
| init | 137 | 161 |
| transport | 826 | 1009 |
| render control | 421 | 1015 |
| graph (source pulls included) | 6523 | 5289 |
| progress/commit | 342 | 204 |
| output copy | 486 | 881 |
| PCM pack | 3725 | 3934 |
| other | 475 | 403 |

Callback 709 rendered 3 blocks across 6 packets; callback 710 rendered 2 blocks
across 6 packets. Intervals are wall time and include observer/interrupt/cache
effects. The recorded scheduler off-task interval is zero, so this capture does
not support changing task priority as the first remedy. It does not prove pure
compute cost or exclude ISR latency. Absolute switch totals outside the sampled
window were not logged; runtime hook-liveness across a known yield remains an
additional check before claiming scheduler preemption has been conclusively
excluded. Host hook tests and actual compiled-kernel call verification passed.

**Aggregate >=6000 us criterion: FAIL.** No timing improvement is claimed from
comparing this instrumented image's maximum with another image/boot. Playback,
target recovery timing and I02–I13 remain NOT_RUN.

## Steady-idle window

The separate 60.1-second passive capture has log SHA-256
`94bda3c63cd602fa4c072dcf02846cd9971bf8315b27d1132bb2016784d19596`,
31448 bytes. It reuses the same running generation 1; no new boot line is
expected. Attribution analysis verifies the repeated stored records rather
than misreporting them as new slow callbacks. No new threshold crossings or
maximum were observed. Complete callback snapshots span 15000 through 24000:
9000 intervening callbacks with no new >3000 or >=6000 us events. Final boot-
cumulative callback count is 24000, max 13087 us, >3000 count 3, >=6000 count 2.
The retained slow records still refer to ordinals 709/710, not new steady events.
Final idle underruns are 0/0 and USB iso/packet/submit errors zero. There were
no fatal markers, controller inputs or loaded/playing decks in the diagnostic.

The logger reports 1832 bytes free of its 3072-byte stack. Idle source supply
minima remain unobserved (`UINT32_MAX`), so they are not playback headroom.
The diagnostic monitors are closed. The registered return to s07 and its exit
checks are complete, as recorded below.
The next bounded [cause experiment](2026-11-01-uac-callback-cause-follow-up.md)
is planned only; no priority or DSP optimization is included here.

## Verified recovery and exit

The owner confirmed manual ROM entry. The guarded return matched the entire
diagnostic 16 MiB state, SHA-256 `cfb842e991622f76eaede2dbe71a37e77e29897fc2362e211ce416d8e057143d`,
before writing. Exact s07 app `95d45203ec245a9f82455f16f15dd95ed6ad2bb8eaaca9cf85f0816ccd5dbfdd`
was written through the registered fallback; all three spans match readback.
No eFuse change or reset was requested. The owner cold-booted without BOOT/RESET
and confirmed the library returned. Expected s07 ELF `005f8d1e3...`, reset reason
1 and a started stream are recorded with zero USB errors and no fatal markers.
The separate playback capture records 993312 output frames (20.694 seconds at
48 kHz), source rate 44.1 kHz, zero underruns and zero USB iso/packet errors.
The owner replied **"sve je ok"** to clean sound in both ears, Pause and CUE at
the unchanged accepted level. This is short recovery confirmation, not a soak
or two-deck qualification. All monitors are closed.

The reconstructed current full state after this readback-verified return is
`817c75491f1c6f3b8e482b576ce44cec43693458d540685ede6adae392123e8d`.
It includes preserved tails from later larger images. This is a replay of
the verified predecessor plus written spans, not a separate post-restore full
read. Future diagnostic plans require all 31 predecessors.

Registration head `fc222ce` also passed 20/20 CI jobs:
[host #533](https://github.com/dvucinozd/Ruzina/actions/runs/41358483089) and
[legacy #56](https://github.com/dvucinozd/Ruzina/actions/runs/41358483075).

Primary evidence: `D:/BoardEvidence/JC1060-20261030-COM3/uac-profile-20261101/`.
Secondary evidence: `C:/Users/Daniel/Documents/BoardBackups/JC1060-20261030-COM3/uac-profile-20261101/`.

The [structured result](../boards/readiness/jc1060-uac-callback-profile-device-20261101.json)
records `SUBSTAGES_CAPTURED_S07_RESTORED_AND_CONFIRMED`. Continue only from the
[cause follow-up plan](2026-11-01-uac-callback-cause-follow-up.md); it is not implemented.

The separate [device archive index](evidence/2026-11-01-uac-profile/device-archive.json)
retains 49 files, including complete prewrite/readback binaries, plans, scripts
and captures. ZIP CRC and every member SHA-256 pass; the secondary copy matches
archive SHA-256 `0f32caaeb9a43319a5363db186eaa077861e37fff93ed48c2ac5d7040470a560`.
The earlier clean-package archive is preserved separately.
