# Continuous producer campaign: singles admitted, dual source underrun

The registered 42/43 campaign ran against published entry head
`5719d5cae22d2cc36a4748ba2923338f94c27989`, using unchanged producer firmware
source `e9ce242ea04c7b95d0ee87f4d6849ba8a4284e09`. Cold/idle and both individual
MP3 cases were admitted. Both-playing failed on the first source-underrun
report. The accepted s07 image was restored by automatic readback; the separate
one-shot software boot action completed. No product/dual-deck qualification
or image promotion follows. Native minimum remains **302**; this unit is
legacy ESP-IDF rev103.

## Entry and observation

The [registration](2026-11-03-continuous-registration.md), exact executable
bindings, reviewed CI-source byte equivalence and both registration/tool
archives were verified before COM3 entry. The owner's fresh reply was
`prikazuje popis pjesama`; other topology was retained from the earlier session,
not individually reconfirmed by that reply. Fresh USB enumeration and ROM
identity/security matched COM3, MAC `e8:f6:0a:e7:22:1d`, rev103, RDID `0x184068`,
no secure boot/encryption. Candidate programming verified the complete before
and after 16 MiB, every written span, and closed its port.

One passive monitor retained the entire stream across physical cold, idle,
source selection/readiness and the failed dual case. The owner performed the
candidate cold cycle and confirmed `vraća`. Actual numbered readiness replies
preceded each functional start. No serial command/reset was issued by capture.

| Case | Registered window | Result | Fill maximum |
|---|---:|---|---:|
| cold-1 | 45 s | Admitted; expected ELF and physical-cold reply | 1908 us |
| idle-1 | 60 s | Admitted | 1908 us |
| s03-deck1 | 180 s including setup | Disturbed MP3; admitted, owner `čujem` | 3635 us |
| s04-deck2 | 180 s including setup | Ben Folds Five MP3; admitted, owner `čujem čisto` | 3635 us |
| s05-both | 60 s including setup | Failed after **15.877 s**, not extended/retried | 4960 us at stop |

The <=3000-us headroom target failed in the source cases independently of the
6000-us hard limit. Cold/idle and individual sources had zero reported source
underruns and USB iso/packet/submit errors. Resource gates passed admitted
checkpoints. Both-playing's terminal report contained underruns **78/94**,
buffered frames **681/93**, and zero USB iso/packet/submit errors. The owner
reported `na početku je bio tih. sad krći`. Capture stopped immediately at the
fault and closed the port; the instruction to keep both playing was withdrawn.

The adapter result remains **INVALID_OR_INCOMPLETE_RESTORE_REQUIRED**, four
admitted cases, `Source underrun/missing`. Four valid prefix checkpoints do
not turn the failed whole stream into a pass. At termination the producer
frame was incomplete and 887 bytes remained unterminated. Preserve these
original bytes; do not repair the frame or use it as a coherent stable interval.
The 20-second both-source overlap gate was **not admitted**.

## What the valid partitions establish

[Derived partitions](evidence/2026-11-03-continuous-device/stable-source-partitions.json)
use only complete, admitted single-source intervals, excluding selection,
load and changing-state intervals. The sum of valid intervals is not a claim
that the entire setup-inclusive case was playing or one uninterrupted run.

| Source | Sum of stable source time | Produced frames/s | Poll wall us/source second | Step wall us/source second | Read wall us/source second |
|---|---:|---:|---:|---:|---:|
| Deck 1 | 156.709 s | 44099.94 | 639557 | 466710 | 160462 |
| Deck 2 | 126.598 s | 44097.14 | 646596 | 467096 | 168199 |

Stable single-source supply remained roughly 1.46-1.49 seconds. The raw fault
snapshot showed zero/12789 us supply when both were active; this is a diagnostic
point, not a completed admitted dual-source interval. Decoder-step and
synchronous-read wall partitions dominate the valid source service cost.
They include waits/preemption; nested decode/poll totals must not be added to
their children. These are **not CPU utilization or IRQ latency measurements**.
Adding separate single-source costs suggests pressure on the serialized owner,
but does not prove CPU saturation, a defective SD card, or a particular remedy.

The stream also reports `unsupported=8` and an early inactive deck-2 position
of 27 ms. The frozen analyzer admitted the stable single-source intervals;
these extra events are retained and should be traced before another campaign.
Do not infer that the owner violated instructions or silently waive a newly
discovered contract issue.

## Automatic recovery and retained state

The registered return ran against 43 executed predecessors. Fresh full
candidate equality, all written-span readbacks and independent full s07
equality passed. Current executed history is **44**. Candidate/return 42/43
plans are consumed; do not replay them even though the full s07 hash repeats.

- Accepted app: `95d45203ec245a9f82455f16f15dd95ed6ad2bb8eaaca9cf85f0816ccd5dbfdd`.
- Accepted ELF: `005f8d1e33aa5f2a8d05af691ce768461f559c917b8804920a912178f5b78ddf`.
- Full flash: `28830827d3e3426bbcd58b9a25d4d2129e28f150bade33fe3c551cd577f93921`.
- Boot result: **BOOT_PULSE_ISSUED_APPLICATION_NOT_OBSERVED**; one pulse,
  port closed, no retry. This does not prove application execution or cold boot.
- No repeated manual s07 cold/listening/Pause/CUE check, following the
  [owner policy](../RECOVERY_POLICY.md). All writers/monitors are closed.

[Summary](evidence/2026-11-03-continuous-device/device-summary.json),
[executed 44-plan prefix](evidence/2026-11-03-continuous-device/executed-history-44.json),
[archive index](evidence/2026-11-03-continuous-device/device-archive.json)
and exact programmer/boot/owner records accompany this report. The original
registration/runbook, earlier NOT_RUN preparation records and archives remain
unchanged historical evidence; the execution records now supersede their
current-state status.

Primary: `D:/BoardEvidence/JC1060-20261030-COM3/continuous-source-20261103/`.
Secondary: `C:/Users/Daniel/Documents/BoardBackups/JC1060-20261030-COM3/continuous-source-20261103/`.
`completed-continuous-device.zip`: **39722727 bytes**, **119 members**, SHA-256
`b188a18b862f695a34c8b0bb17ecd0b7cf69f1f82d0d72287a803d8c5a4fd733`.
CRC, every member hash and secondary copies match. Registration/tool archives
retain their previous verified hashes. Large raw files remain outside Git.

## Next ordered work, offline first

1. Reproduce the source-pressure mechanism with existing permitted MP3 fixtures.
   Inspect `deck_play::decode_inner`, `VolumeFiles::read_at`, the stream decoder
   and actual packaged release flags. Trace the `unsupported=8` events against
   MIDI/transport/producer state; distinguish unsupported controls from media
   failures. Use separate evidence, without rewriting this capture's admission.
2. Build a deterministic two-source service test: alternate/supply-order polls,
   consume at 44.1 kHz, exercise the actual action/time budgets, inject explicit
   read delays, and record supply loss/fairness. Pair it with a release-mode
   in-memory versus delayed-reader decoder benchmark using identical bytes and
   exact flags. Host timing is host evidence, not ESP32-P4 timing. Keep simulated
   costs explicit; do not infer target CPU from wall partitions. Verify seek,
   EOF, cancellation and removal semantics if the selected fix touches them.
3. Select **one** justified change from the isolated result. Read coalescing,
   decoder compilation/compute work and scheduling are competing hypotheses,
   not a bundle to land together. Do not merely grow buffers, raise deadlines,
   remove fault gates or enable extra owners. Keep normal defaults and native
   minimum 302. Review, relevant host checks and both target builds precede
   any new device transition.
4. Only after that evidence, register fresh **44/45** candidate/return plans
   against the exact 44-plan prefix and unchanged current full hash, bind exact
   reviewed tools/images, verify dual-copy archives and publication, then run
   the unchanged bounded source sequence. Keep automatic-readback s07 recovery
   and separately bound boot, without repeated manual return qualification.

Normal dual-deck, MAIN/PFL isolation, transport/reconnect and soak qualification
remain open. No further flash or peripheral development is part of this result.
