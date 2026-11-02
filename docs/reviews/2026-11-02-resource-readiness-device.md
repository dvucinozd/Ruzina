# Resource readiness: bounded cold/steady idle timing campaign

Date: 2026-11-02. JC1060 NewPanel rev103, MAC `e8:f6:0a:e7:22:1d`,
RDID `0x184068`, full-speed native service USB-C, FLX4 connected. The owner
confirmed the library returned after each requested complete USB power cycle.
Native minimum 302, output level, priorities, DSP and transfer depth are unchanged.

Status: **COLD_AND_STEADY_IDLE_TIMING_PASS / PRODUCT_NOT_QUALIFIED / S07_RESTORED_AND_CONFIRMED**.
Continues the [reviewed implementation and registration](2026-11-02-resource-readiness.md).
The exact profile is source `d28abd0`, app
`7e62af397730d604b86991a6848f9f58c6705b414c5d124e3ce1019e419f099e`,
ELF `f7be1c0d1ae52e9be3554ed20bd3ff9870e5f32b5ede34c87c0623e0400d3c1f`.
Guarded-tool source `51c6967` and registration `79e251a` have separate successful
21/21 CI; exact-parent source/tool reviews each have 0 issues. Those required
source/registration gates preceded device operations. Later evidence `621e3fc`
and software-entry record `3fe21ce` also passed 21/21 CI, verified afterward;
their later conclusions are not attributed to the earlier programming gate.

## Entry, write and cold-window identities

The owner requested agent-performed ROM entry instead of the physical BOOT
sequence. One pinned native USB `usb-reset` entry succeeded and freshly read
rev103/MAC/security; this is a software entry, not a power-cycle observation.
The frozen programmer then used `no-reset`, freshly checked identity/security/
RDID, and matched all 16 MiB of the expected 36-plan predecessor
`7df030bf565c63503a04bfaf2a9cd114632c60e5f357335da9f49d81c2e544e6`.
Written spans and an independent whole-flash readback match candidate state
`f417a0b35e462406ef954c0d71a8e9aba07826e133a5b1759003ff198751fd35`.
The [programmer result](../boards/evidence/jc1060-20261030-com3/resources-candidate-flash/result.json)
and log are retained; full binary captures have matching secondary copies.

The owner subsequently removed USB power for 5-10 seconds and reconnected
without BOOT/RESET for each of three boots, confirming **"vraća"** each time.
All windows identify the expected ELF, reset reason 1, valid scheduler and
core-0 C-dispatch IRQ calibrations, complete histograms and coherent partitions.
Each lasts 45 seconds after the new expected ELF identity and includes 7000
completed callbacks. No track was loaded/played and controls were untouched.

Raw files include telemetry captured before the new boot. They remain intact;
each separate `boot-window.log` starts at its sole new matching ELF identity.
Its byte offset/hash is recorded by the monitor. This avoids mixing two boots
with generation 1; no conflicting calibration is discarded within a boot.

## Results

| Window | Callback maximum us | >3000 / >=6000 | Complete callbacks |
|---|---:|---:|---:|
| [Cold 1](../boards/evidence/jc1060-20261030-com3/resources-cold-1/summary.json) | 2008 | 0 / 0 | 7000 |
| [Cold 2](../boards/evidence/jc1060-20261030-com3/resources-cold-2/summary.json) | 1847 | 0 / 0 | 7000 |
| [Cold 3](../boards/evidence/jc1060-20261030-com3/resources-cold-3/summary.json) | 1757 | 0 / 0 | 7000 |
| [Separate steady idle](../boards/evidence/jc1060-20261030-com3/resources-steady/summary.json) | 1757 cumulative, unchanged | 0 / 0 new | 9000 new |

Cold callback counts total 21000, all below the unchanged <=3000 us headroom
target and with no >=6000 us rejection. The 60.064-second steady window is bound
to cold 3 and spans callback snapshots 9000 -> 18000. It adds 9000 observed
callbacks with no new maximum/crossing. Every cold/steady window reports zero
USB iso/packet/submit errors and zero idle underruns. These callback maxima
are not independent substage maxima or a claim about playing-source throughput.

The prior flash-control cold maxima were 15827/17386/15849 us and all failed.
They are historical observations, not a contemporaneous randomized A/B. The
new candidate supports preparing permanent stores before UAC as a useful next
development step. It does not prove a particular cache/bus mechanism, exclude
every startup cause, or qualify source decoding/rendering during playback.

## Resource observations and limits

| Observation | Cold 1 | Cold 2 | Cold 3 |
|---|---:|---:|---:|
| Printed PREPARING -> READY interval us | 81367 | 81500 | 82068 |
| Global PSRAM free decrease bytes | 1162592 | 1162592 | 1162592 |
| READY PSRAM free bytes | 31157988 | 31157988 | 31157988 |
| READY largest PSRAM block bytes | 30932992 | 30932992 | 30932992 |
| READY internal free bytes | 432748 | 432748 | 432748 |
| READY SD stack free bytes | 79724 | 79724 | 79724 |

The net PSRAM decrease is close to the 1162544-byte both-deck payload plus
allocator overhead. It is a global heap observation, not per-allocation proof.
The lowest logged PSRAM minimum is 28054272 bytes and lowest logged SD stack
reserve 58284 bytes in these idle catalog windows. No task/threshold was raised.
READY is published before telemetry prints, so cross-task print order is not
an atomic startup timestamp. The printed interval includes scheduling/logging.

These snapshots do not measure the minimum largest block throughout the run,
minimum internal heap, USB-client runtime stack high-water, MP3 seek depth or
first deck-2 LOAD under playback. Logger high-water belongs to its own task.
Those omissions remain explicit resource gates for the normal-image campaign.

## Recovery and next step

Registered s07 return completed against exactly 37 predecessors. All 16 MiB
matched candidate full state before writing; every written span and an
independent full-flash readback afterward match s07 state
`1a45683632f9951f1121ad0866fd7b3050d146022bfc6c719be18346848ad4fe`.
This is a fresh whole-flash observation, not only a reconstruction. The
[return result](../boards/evidence/jc1060-20261030-com3/resources-return-flash/result.json)
binds both whole-flash captures. Software USB ROM entry preceded the guarded
`no-reset` programmer; no eFuse or erase-all operation was requested.

The owner then power-cycled physically, confirmed **"vraća"**, and the
[cold capture](../boards/evidence/jc1060-20261030-com3/resources-restore-cold/result.json)
has s07 ELF `005f8d1e33aa5f2a8d05af691ce768461f559c917b8804920a912178f5b78ddf`,
reset reason 1 and started USB stream. Owner response **"sve je ok"** confirms
clean sound in both ears and Pause/CUE/controls at the unchanged level. The
[playback capture](../boards/evidence/jc1060-20261030-com3/resources-restore-playback/result.json)
contains 651168 played output frames, **13.566 seconds at 48 kHz**, with zero
USB iso/packet errors and zero underruns. The requested approximately 20-second
operator smoke is not treated as measured duration; only the captured frame
delta is reported. This is s07 recovery confirmation, not engine qualification.

All programmers and monitors are closed. Current installed s07 app is
`95d45203ec245a9f82455f16f15dd95ed6ad2bb8eaaca9cf85f0816ccd5dbfdd`.
Future plans need **38 executed predecessors**: the historical 36, candidate
and return-s07. The exact current full state is the fresh readback above.
The [completed summary](evidence/2026-11-02-resource-readiness/device-summary.json)
binds raw hashes, calibrations/resources, operator observations and recovery.
The [completed archive](evidence/2026-11-02-resource-readiness/device-archive.json)
is **37697960 bytes**, SHA-256
`74c82abe054a7f0eed7e7437df000267e3993e2409529236f8364c9b85ddc5be`.
CRC, all member hashes and all secondary member/ZIP hashes match. Full binaries
remain external under `D:/BoardEvidence/JC1060-20261030-COM3/resource-readiness-20261102/`
and matching `C:/Users/Daniel/Documents/BoardBackups/JC1060-20261030-COM3/resource-readiness-20261102/`.

Next development is a separately registered clean **normal** engine image for
bounded two-deck MAIN/PFL, LOAD/seek/CUE/EOF, media loss/reconnect and I02-I13.
The current normal package is BUILD_ONLY and has no write plan. Do not promote
the diagnostic to normal operation, merge the stacked PRs automatically, or
add a DAC/APTA/network peripheral based on this idle result.
