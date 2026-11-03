# Calibrated callback cause diagnostic: device results

Date: 2026-11-02. Exact unit: JC1060 NewPanel, ESP32-P4 v1.3/103, COM3.
Status: MEASURED_NOT_QUALIFIED; aggregate timing FAIL; s07 restored and confirmed.
Continues the [registered execution and IRQ audit](2026-11-02-uac-cause-profile-execution.md).
Native minimum remains 302. No gain, DSP, task-priority or peripheral change.

## Identity and entry

Source `3e20a8e64ef7f2ff855bf18706712e289c6cc747` passed 20/20 source CI jobs:
[host #536](https://github.com/dvucinozd/Ruzina/actions/runs/41420822848) and
[legacy #59](https://github.com/dvucinozd/Ruzina/actions/runs/41420822855).
Registration and archive correction were published before device access through
`d06c9a9d66f76e6e386db1b4e1fe701d98f485e5`, with matching local/remote SHA.
The review findings were resolved as recorded in the execution document; later
documentation CI is independent of this source-head result.

Exact candidate app SHA-256:
`562788ee5fd8de05d6bad98af44f1e8b4e299c2eed2fd056f6b2c1430e65db35`.
Expected ELF:
`6486219942c5b32148604a55979712e82a2311e98f4a09700c0383b796509ada`.
The owner reported the previous library screen and manual download mode. The
guarded no-reset programmer independently verified rev103, MAC
`e8:f6:0a:e7:22:1d`, security/encryption flags 0 and RDID `0x184068`. Four factory
copies passed. The entire 16 MiB predecessor matched the 31-plan reconstruction
`817c75491f1c6f3b8e482b576ce44cec43693458d540685ede6adae392123e8d`
before writing. All three written spans matched readback; no eFuse or reset.

The owner cold-booted without BOOT/RESET and confirmed the library returned.
Expected candidate ELF and reset reason 1 were captured. No controls, loaded
decks or playback were exercised in the diagnostic.

## Calibration and coherent slow records

Both target calibrations passed before audio submission:

- Scheduler: absolute switches 0 -> 1; away time 0 -> 4785 us, within wall
  2037342 -> 2042198 us. This validates a completed actual tracked switch-out.
- Core-0 C-dispatch: entries 0 -> 42, time 0 -> 150 us, nested count 0 -> 0,
  within wall 2037372 -> 2042197 us. Dispatcher liveness is observed.

All three retained records have exact exclusive partitions, calibrated valid
scheduler/IRQ windows and zero observed task-switch deltas. Absolute scheduler
switches are 563 before/after, away time 2386654 us before/after. Generation 1,
catalog event 5 (preflight pass complete), deck flags 0/0, logged drops 0.
These are start-context fields, not a causal statement about the catalog task.

| Record | Ordinal | Whole us | Rust us | Outer us | Scheduler window us | IRQ window us | Dispatch us | IRQ entries |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| worst | 475 | 14812 | 14702 | 110 | 14916 | 14872 | 1653 | 4 |
| latest >=6000 | 476 | 10450 | 10362 | 88 | 10541 | 10495 | 1014 | 4 |
| latest >3000/<6000 | 477 | 3954 | 3888 | 66 | 4012 | 3982 | 0 | 0 |

No nested IRQ was observed in these windows. Since IRQ timing brackets a
slightly wider window, dispatch time inside the whole call is bounded:
1593–1653 us for ordinal 475, 969–1014 us for 476 and 0 us for 477. The analyzer
checks these bounds rather than assigning the full broader interval to the fill.

| Exclusive Rust stage | 475 us | 476 us | 477 us |
|---|---:|---:|---:|
| init | 155 | 105 | 40 |
| transport | 920 | 596 | 367 |
| control | 428 | 382 | 272 |
| graph/source pulls | 5793 | 3756 | 1835 |
| progress/commit | 347 | 228 | 150 |
| output copy | 591 | 414 | 138 |
| PCM pack | 5017 | 3929 | 951 |
| other | 1451 | 952 | 135 |

Worst/latest-over render two blocks, latest-headroom three blocks; all have six
packets. The graph/pack values are wall time, not pure computation. Covered
dispatch explains part of the outlier, but at least 13159 us of the worst whole
call lies outside that covered dispatch interval. Uncovered vector/exit work,
cache/bus effects, computation and observer costs remain together. Scheduler
and IRQ intervals must not be added/subtracted as disjoint CPU categories.
There is no evidence here supporting a task-priority fix.

## Cold and separate steady idle

Cold log: 54893 bytes, SHA-256
`1c62041c0e0143289697f931daf3c1b44ade9a0ecc129e5a284cb4d637965133`.
Capture lasts 45 seconds after boot identity (150.4 seconds with operator wait).
Initial prefill: four samples, first/max 2098 us, no threshold crossings.
Recovery prefill: no samples. Callback first 1006 us, max 14812 us, >3000 count
4 and >=6000 count 2; these appear by callback 1000 and remain unchanged through
the complete callback-7000 snapshot. All seven histogram snapshots have exact
denominators and valid partitions. At 7000, whole bins are
`0,0,0,0,6928,68,4` for upper bounds 50/100/250/500/1000/3000/infinity us.

Separate steady log: 61147 bytes, 60.1 seconds, SHA-256
`4cc7eba4926ada85e6aef92a1e132608076b87cbf89a5db606d3db1b099eeab8`.
Complete snapshots span callback 16000 -> 25000: 9000 intervening callbacks
with no new >3000 or >=6000 crossings or maximum. All ten histogram snapshots
are complete. Stored ordinals 475/476/477 repeat; they are not new slow events.
No new boot line is expected in this passive window; it shares generation 1.

Both windows report zero idle underruns, USB iso/packet/submit errors and fatal
markers. Profile logger free stack reaches 2072/4096 bytes. Idle source-supply
minima remain unobserved, so they are not playing-source headroom evidence.
The aggregate >=6000 us gate remains **FAIL**. No improvement/regression claim
is made by comparing differently instrumented images or different boots.
Diagnostic playback, target recovery timing and I02–I13 are NOT_RUN.

## Code placement inspection and next decision

The exact ELF places pack (316 bytes), its closure (304), write helper (84) and
render graph (2204) in `.flash.text`; original dispatcher and wrapper are in
`.iram0.text`. The inspected packetizer performs integer PCM writes, bounds
checks and a zero-fill call; no floating-point instructions appear in its
captured 704-byte pack/closure/write region. This is placement/code evidence,
not proof of cache causality or a performance win. The [next bounded plan](2026-11-02-uac-cache-compute-follow-up.md)
starts with memory/cache-counter feasibility and an unchanged instrumented
control. It does not authorize a broad IRAM move or speculative DSP rewrite.

## Verified return and exit

The owner entered ROM manually again. The entire diagnostic predecessor matched
`24084391f2b03fb095a913468ad4dec63a3f1e2da40a003253a6c4c891289542`
before the registered return wrote accepted app
`95d45203ec245a9f82455f16f15dd95ed6ad2bb8eaaca9cf85f0816ccd5dbfdd`.
All three ranges passed readback. Cold boot then matched s07 ELF `005f8d1e3...`,
reset reason 1 and a started stream, without fatal markers or USB errors. The
owner confirmed the library returned and then replied **"sve radi"** to clean
sound in both ears, Pause and CUE at unchanged level.

The recovery playback log contains 580032 played output frames (12.084 seconds
at 48 kHz), source rate 44.1 kHz, zero underruns and USB iso/packet errors. This
is short recovery confirmation; it does not claim a captured 20-second playback
or a soak. Playback log SHA-256:
`2c22230199b219475bb3e82e2b933517da4c1d6c2329209c96fb3b0cde2ca02c`.
All programmers/monitors are closed. The board is on owner-confirmed s07.

Future plans require **33 predecessors**. Current reconstructed full state:
`4324fee5c109cc9fe28aebbafa8f5cf2116ee4950afce374be5ef6f81aa2ae68`.
This replays verified predecessor plus readback-verified writes and preserved
tails; it is not an independent complete post-restore read.

See the [structured result](../boards/readiness/jc1060-uac-cause-profile-device-20261102.json).
The separate [device archive](evidence/2026-11-02-uac-cause/device-archive.json)
contains 52 files / 20811665 bytes, including complete prewrites/readbacks,
scripts, plans, code inspection and captures. CRC, member hashes and secondary
SHA-256 match `b76195bdac2c29a46a812fbef00ec167377efba7d50e82c86b3ff6c701dd529e`.
The registered build archive remains separate and unchanged.
