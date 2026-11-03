# Review-fix candidate: physical campaign record

Date: 2026-11-01. Scope: the observed JC1060 NewPanel ESP32-P4 v1.3/103 unit on COM3 only. Native minimum revision 302 remains unchanged. No APTA integration or output-gain change is included.

Read the [campaign procedure](2026-11-01-review-fix-device-campaign.md) and [history reconciliation](2026-11-01-review-fix-history-reconciliation.md) together with this execution record. The owner authorized continuation and confirmed the physical actions below. Historical evidence is retained unchanged.

## Registered candidate and criteria

- Published source: `b4540926540918450fc6dac9780a3b8a14049c6d`; [PR #73](https://github.com/dvucinozd/Ruzina/pull/73), 19/19 CI jobs passed before this device campaign.
- Candidate app SHA-256: `0d4d07fa79d2eef299575304ed11258d1d071d8bdace84f35d793b176d551d85`, 816640 bytes.
- Candidate plan SHA-256: `18022505a4821fda7c4ce55be749bcc8e9eab9bb57c4e590b76631a5380cbc92`.
- Full historical predecessor SHA-256: `e977c60cb72afddc1eddd8dc3ca925ba8bf1a6762d38cacd0e00c268a88cdb18`; app `e9eeae92bc2e29f0ce4ba929a38b661592f6edf88621c0ffc3e5dce34c727a5a`.
- Candidate-to-s07 fallback plan SHA-256: `3a7f1d54ec71a51f8df6f2764fe92b37960a8d6f8b2fab85f6b00186ee036c90`, prepared and secondary-copied, not executed.
- Before candidate runtime, register the proposed timing criterion: any observed fill >=6000 us fails this campaign; <=3000 us is the initial headroom target. A result between these values misses the headroom target even if it avoids the rejection bound. No new limit is inferred from old captures.
- Normal playback requires zero steady underruns, packet/transfer errors, unexpected resets and source-identity mismatches. Listening acceptance and telemetry are separate. Controlled fault-injection outcomes and scenarios without a supported injection path remain separately recorded.

## Operator and entry observations

1. Owner: track list visible, FLX4 connected, board is COM3 through the same full-speed USB-C service connector. Previous power description is PC USB only; a fresh full controller/power topology description remains open.
2. Passive serial capture: no reset and no command bytes, 20 seconds, idle deck-engine telemetry. This is not listening acceptance.
3. Owner performed manual BOOT entry after a 5–10 second power disconnect.
4. Read-only identity/security and whole-flash check: expected MAC/rev103/RDID; all 16 MiB match the reconstructed 25-plan predecessor state. Primary and secondary copies match.
5. First programmer attempt failed its RDID assertion after reconnecting to an existing stub. No flash/eFuse writes and no reset were requested. Unexpected RDID was not logged; no guessed value is assigned.
6. Owner confirmed a second complete USB power disconnect and BOOT entry. Second programmer invocation records the RDID explicitly and keeps the original expected ID and predecessor checks.

## Current execution state

Second programming session passed all identity/security/RDID and complete predecessor-sector checks. All three written ranges passed readback. No eFuse changes were made.

A programmatic RTS reset returned to ROM download (`boot:0x4`, waiting for download). The initial capture's generic `FAILED_BOOT_INSPECT` status means no application boot was observed in that reset mode; it is not an application crash. The owner confirmed the screen was unlit. A subsequent owner-assisted cold boot without BOOT/RESET started the expected candidate (`ELF babdee193...`, reset reason 1), returned the track list and started UAC generation 1 with four transfers.

The 30-second post-boot observation recorded four mix snapshots, all with maximum fill 12260 us, underruns 0/0, and zero USB isochronous/packet/submit errors. Producer maximum gap snapshots were 10004 and 10008 us. No fatal markers were present. No track was played in this candidate session.

**I01 timing criterion: FAIL.** Boot identity/display passed their limited checks, but the registered all-fill bound was exceeded. The aggregate maximum covers both startup prefill and callback refills; this log cannot prove which phase produced the outlier or that an active-stream deadline was missed. The criterion is not retroactively relaxed. I02–I13 remain NOT_RUN. See the [timing follow-up plan](2026-11-01-uac-timing-follow-up.md).

The candidate was therefore removed. The owner performed fresh manual ROM entry, the programmer verified candidate predecessor sectors and restored the exact s07 app `95d45203...` using the registered candidate-to-s07 plan. All written sectors passed readback. The owner then cold-booted without BOOT/RESET and confirmed the track list returned. The boot log matches s07 ELF `005f8d1e3...`, reset reason 1, no fatal markers, and a started UAC stream with zero packet/isochronous errors.

The fresh restore playback check is complete: the owner replied **"sve je ok"** to the request to check a known deck-1 track, clean sound in both ears, Pause and CUE at the unchanged level. The corresponding passive capture records 751968 played output frames (15.666 s at 48 kHz), a 44100 Hz source, zero underruns and zero USB isochronous/packet errors. The final ring had 65281 frames buffered. These are limited restore observations, not candidate qualification or an extended soak. The first 180-second playback-request capture contained idle only; the following 168-second capture contains the actual playback. Both are retained. No boot line is expected in these passive windows; the immediately preceding cold-boot capture binds the restored image.

Current installed image: **s07 deck-play**, not s09 and not the review-fix candidate. Future flash history must replay the 25 historical plans, the candidate plan and the return-to-s07 plan, in that order (27 plans total). Source code and accepted gain were unchanged during this physical session.

All serial monitors are closed. Finished raw records, the operator confirmation and staged plans are hash-verified on primary and secondary storage. The [structured campaign summary](../boards/readiness/jc1060-review-fix-campaign-20261101.json) indexes the captured raw files and their SHA-256 values. The [candidate plan](../boards/readiness/jc1060-review-fix-write-recovery-plan-20261101.json) and [return plan](../boards/readiness/jc1060-review-fix-return-s07-write-recovery-plan-20261101.json) retain their exact original bytes. Full flash readbacks and binary payloads stay outside Git.

External primary evidence lives under `D:/BoardEvidence/JC1060-20261030-COM3/review-fixes-20261101/`; the secondary root is `C:/Users/Daniel/Documents/BoardBackups/JC1060-20261030-COM3/review-fixes-20261101/`.
