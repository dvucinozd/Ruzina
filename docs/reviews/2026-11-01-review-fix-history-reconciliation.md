# Review-fix campaign: historical image and recovery reconciliation

Date: 2026-11-01. Repair source: `b4540926540918450fc6dac9780a3b8a14049c6d`, [PR #73](https://github.com/dvucinozd/Ruzina/pull/73). This record continues the [physical campaign preparation](2026-11-01-review-fix-device-campaign.md). It is separate from candidate acceptance.

## Publication

Both workflows passed on the published repair head: [host foundation](https://github.com/dvucinozd/Ruzina/actions/runs/41330197575), and [legacy IDF host plus 17 builds](https://github.com/dvucinozd/Ruzina/actions/runs/41330197587). Total: 19 successful jobs. PRs #71/#72/#73 remain unmerged at this checkpoint.

## Historical evidence verified offline

The external `s08`/`s09` directories have now been inspected and verified, rather than treated as proof merely because they exist:

| Capture | App SHA-256 | Source commit | Observed limits |
|---|---|---|---|
| s08 deck-mix | `55820e840d428023ff2c5d43f601e1068356e109931137b0b22e853c9b6c1058` | `4540ec9ac7cc57b22de924d53855061f67a766fc` | Final underruns 0/0; maximum fill 9682 us; listening acceptance not captured |
| s09 deck-engine | `e9eeae92bc2e29f0ce4ba929a38b661592f6edf88621c0ffc3e5dce34c727a5a` | `3a1dea542e137e6c1a034d43f3610cdf78c64f08` | Final underruns 7592/0; maximum fill 11175 us; listening acceptance not captured |

Both source manifests record clean source trees. Package artifact hashes, plan hashes, programmer-log hashes, every readback sector and the boot-log ELF hash prefixes match their named images. No fatal markers were found in these two captures. The `s08` result JSON hash also matches its raw log. `s09` has a raw runtime log but no pre-existing runtime result JSON; the new reconciliation records its digest without inventing a historical result.

The captured maximum fill times exceed the new campaign's 6000 us rejection criterion. These historical images are not accepted under that criterion. The s09 underrun count is additional negative evidence; zero USB packet errors does not make it healthy. The last repository listening acceptance remains the limited [s07 deck-play checkpoint](2026-11-01-legacy-deck-play.md). No two-deck or MAIN/PFL acceptance is inherited.

All 25 ordered plans from the original diagnostic through s09 passed predecessor SHA linkage, staged-payload hashes, restore-payload hashes, complete restore reconstruction and preserved final-sector tail checks. All four 16 MiB factory backup files (two independent reads on primary storage, plus their secondary copies) again match `27d5782978aeb3c42f8b49659e793d3ca8f59543f62e419d7b1981de2c492060`.

Latest historical plan: `478f6f96281c5d9bcf0d27c8f32b36652cb067c5cba9a6e14b46fb6e2808931a`.

## Candidate preparation

The offline repair deck-engine package is still app `0d4d07fa79d2eef299575304ed11258d1d071d8bdace84f35d793b176d551d85`, 816640 bytes. All 269 source hashes still match the current Windows checkout and all package artifacts pass verification. Its historical manifest retains the actual build-time base commit/dirty provenance; publication did not rewrite that provenance.

An offline sector plan extending the complete s09 ancestry was generated:

- Plan SHA-256: `18022505a4821fda7c4ce55be749bcc8e9eab9bb57c4e590b76631a5380cbc92`.
- Primary evidence: `D:/BoardEvidence/JC1060-20261030-COM3/review-fixes-20261101/`.
- Secondary evidence: `C:/Users/Daniel/Documents/BoardBackups/JC1060-20261030-COM3/review-fixes-20261101/`.
- Files: `history-reconciliation.json`, `reconcile_history.py`, and `campaign-i-offline-plan/`.

This plan's sector restore returns to the historical s09 predecessor, which is not a listening-qualified fallback. A separate candidate-to-s07 plan has therefore been prepared and secondary-copied: `campaign-i-return-s07-plan/plan.json`, SHA-256 `3a7f1d54ec71a51f8df6f2764fe92b37960a8d6f8b2fab85f6b00186ee036c90`. It replays all ancestors including the candidate, then stages the exact accepted s07 package. It has not been executed. The old s07 restore files must not be applied directly to an s09/candidate flash state.

## Live checkpoint

The owner confirmed COM3, the same full-speed USB-C service connector, the track list visible and FLX4 attached. USB enumeration matches VID 303a/PID 1001 and serial/MAC `E8:F6:0A:E7:22:1D`.

A 20-second passive capture requested no reset and sent no serial commands. It showed heartbeat, deck-engine telemetry, idle decks, 0/0 underruns and fill maximum 1903 us. This idle sample does not identify the full image or qualify playback.

The owner then performed manual BOOT entry. The read-only ROM check confirmed ESP32-P4 revision 103, MAC `e8:f6:0a:e7:22:1d`, RDID `0x184068`, security flags 0 and flash encryption count 0. The full 16 MiB readback matches the reconstructed s09 state exactly: SHA-256 `e977c60cb72afddc1eddd8dc3ca925ba8bf1a6762d38cacd0e00c268a88cdb18`, zero differing sectors. Installed app SHA is the s09 `e9eeae92...` above. The full readback and result are secondary-copied and hash-verified.

The first candidate programming session reconnected to the already running stub and failed its flash RDID assertion before any write. Result: `FAILED_STOP_AND_INSPECT`, `flash_write_attempted=false`, `efuse_write=false`, `reset_requested=false`. The actual unexpected RDID was not captured in that first script. Its raw log and result are retained under `campaign-i-flash/`; no guessed ID is assigned. A fresh physical ROM entry is required for the second session, which records the RDID explicitly. The expected ID is not relaxed.

At that entry checkpoint no new firmware had been written. The subsequent [execution record](2026-11-01-review-fix-device-results.md) records the successful second write, candidate cold boot, failed I01 timing criterion and ancestry-aware restoration of s07. Use that record for current image identity; do not reuse this historical s09-only chain as the current predecessor.
