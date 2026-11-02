# Packetizer placement: completed device comparison

Date: 2026-11-02. Exact JC1060 NewPanel rev103 unit on COM3, MAC
`e8:f6:0a:e7:22:1d`, flash ID `0x184068`. Full-speed service USB-C, FLX4
connected, USB-only board power, as confirmed by the owner. Native minimum
remains 302. No output-level, DSP or priority change.

Status: **MEASURED_NOT_QUALIFIED / BOTH_TIMING_GATES_FAIL / S07_RESTORED**.
Continues the [registered experiment](2026-11-02-pack-placement-experiment.md).
Source `605abe78...`; published pair/evidence head `f0ba8ca...` passed 21/21 CI
jobs before registration. The four plans were published in `8c794cf` before
device work. Local source/artifact checks and exact-parent CodeRabbit review
are recorded in the preparation report; they are not target qualification.

## Entry and measurement contract

The guarded control write verified identity/security and the entire 16 MiB
33-plan predecessor (`4324fee5...`); all written ranges passed readback. The
owner cold-booted three times, confirming the library each time. Captured ELF
`5981be59...` matches control app
`0718c73af20982719904070fc14fcd4c3778ce2924e3798e8a1d55c01b79a4ea`.

After control measurements, fresh manual ROM entry preceded the IRAM write.
All 16 MiB matched the 34-plan control state (`67892cc8...`), with unchanged
rev103/MAC/security/RDID. All three written spans passed readback. Three more
owner-assisted cold boots matched IRAM ELF `cc47d1ee...`, app
`b38488e5b1987398e0d9a095357547d14b87eac5d36008e8eab65d2179618ec6`.
Both programmers requested neither reset nor eFuse changes.

Each cold window continued 45 seconds after the expected ELF boot identity;
the last complete snapshot contains 7000 callbacks. Each variant then had a
separate passive 60-second idle window. No tracks were loaded or played and
no controls were moved in either diagnostic. Reset reason 1, both target
calibrations, exclusive partitions and complete histogram denominators pass
in all six boots. No nested IRQ invalidated the retained records. Covered IRQ
is still only core-0 C-dispatch wall time; the unaccounted remainder is not
pure computation. No task switch was observed in the retained worst records.

## Results

| Image / cold boot | Whole maximum us | >=6000 / 7000 | Worst graph us | Worst pack us | Covered IRQ inside worst, us |
|---|---:|---:|---:|---:|---:|
| Control 1 | 15827 | 2 | 7022 | 5624 | 1046–1102 |
| Control 2 | 17386 | 3 | 6382 | 6101 | 2226–2287 |
| Control 3 | 15849 | 2 | 7054 | 5596 | 979–1049 |
| IRAM 1 | 14157 | 3 | 7417 | 4042 | 958–1013 |
| IRAM 2 | 13599 | 2 | 4805 | 5873 | 1438–1498 |
| IRAM 3 | 15548 | 3 | 6952 | 5564 | 797–848 |

There are 7/21000 control and 8/21000 IRAM observed rejection-bound crossings.
Median boot maxima are 15849/14157 us respectively. The IRAM observations have
lower maxima, but still fail every cold boot and do not reduce the observed
crossing total. This small, sequential, non-randomized campaign does not
establish a cache-miss mechanism or an accepted performance optimization.
The three retained stage values are wall times of each whole-call maximum,
not independent stage maxima or a CPU partition after IRQ subtraction.

Control steady snapshots span 13000 -> 22000 callbacks; IRAM spans 9000 ->
18000. Each adds 9000 intervening callbacks with no new >3000 or >=6000 us
crossing and no new maximum. Repeated retained slow records are not new events.
All cold and steady windows report zero USB iso/packet/submit errors and zero
idle underruns. These are not playing-source headroom or sustained playback
evidence. I02–I13 and target recovery timing remain NOT_RUN.

**Decision:** the 704-byte narrow pack placement plus 64-byte alignment cost is
insufficient to close the unchanged >=6000 us rejection / <=3000 us headroom
gate. Retain the diagnostic as evidence, without enabling it for the product.
No further placement variant or broad graph IRAM move is part of this campaign.
The [next offline decision](2026-11-02-post-placement-next-step.md) audits the
actual USB service budget and feasibility of separating render work from refill.

## Evidence-window correction

Monitors open before owner power cycles. Several raw files therefore include
valid telemetry from the preceding boot before the new boot identity. Since
both boots restart generation 1, feeding the whole file to the single-boot
analyzer correctly raises a conflicting-calibration identity error.

The complete raw files are immutable. A separate `boot-window.log` starts at
the sole expected new ELF identity line; its byte offset and SHA-256 are in
the scenario summary. It retains all new calibration/prefill/callback telemetry.
No original evidence bytes were removed. Control boot 1 was originally analyzed
as the complete single-boot file; later windows are explicitly sliced. The
first postprocessing assertion also incorrectly required all three retained
kinds, even when no 3000–6000 sample exists; it was corrected to require the
worst record and validate every observed record, without inventing headroom
events. These are postprocessing corrections, not firmware changes.

## Exact s07 return

Fresh manual ROM entry preceded the registered `iram-return`. All 16 MiB
matched the expected 35-plan IRAM state
`ce3a4a05495a5454cca308b94d00cd7d1f33831d5642a73e3f145ccf973c6267`.
Accepted s07 app `95d45203...` was written; all three ranges passed readback.
The owner cold-booted and confirmed the library. Captured ELF
`005f8d1e33aa5f2a8d05af691ce768461f559c917b8804920a912178f5b78ddf`,
reset reason 1 and started USB stream match s07, without fatal markers.

The owner then replied **"sve radi"** to clean sound in both ears, Pause/CUE
and normal controls at the unchanged level. The passive playback log captures
965376 output frames = **20.112 seconds at 48 kHz**, source 44.1 kHz, zero
underruns and zero USB iso/packet errors. This is recovery confirmation only.
Its raw SHA-256 is
`4e8d132709b9d0a20c804125841e747d1a9a9cde5a77c5becbef9832cd9178a8`.
All programmers/monitors are closed; installed s07 is owner-confirmed.

Future plans must include **36 predecessors**: the historical 33, control,
IRAM and IRAM-return. The unused control-return is an alternative and must not
be inserted in that executed chain. Current reconstructed full state is
`7df030bf565c63503a04bfaf2a9cd114632c60e5f357335da9f49d81c2e544e6`.
This is the verified predecessor plus written/readback-verified spans and
preserved tails, not an independent whole-flash post-restore read.

Raw logs and derived results are under `docs/boards/evidence/jc1060-20261030-com3/pack-*`;
the [comparison index](evidence/2026-11-02-pack-placement/device-comparison.json)
and [completed summary](evidence/2026-11-02-pack-placement/device-summary.json)
bind hashes, operator observations and exact-image results. Full flash binaries
remain outside Git under `D:/BoardEvidence/JC1060-20261030-COM3/pack-placement-20261102/`
and matching `C:/Users/Daniel/Documents/BoardBackups/JC1060-20261030-COM3/pack-placement-20261102/`.
The [completed archive index](evidence/2026-11-02-pack-placement/device-archive.json)
binds full prewrites/readbacks, captures, scripts and confirmations: ZIP
28974200 bytes, SHA-256
`354e8afc8b137ac22d0aa1cd7a7b0ecfb7fd8ca7eb11a3fd16678653ee9144ab`.
CRC, every member hash and the secondary archive hash pass; earlier build and
registration archives remain separate and unchanged.
