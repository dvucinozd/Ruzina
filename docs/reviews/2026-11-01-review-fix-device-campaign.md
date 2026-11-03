# JC1060 review-fix candidate: physical campaign preparation

Date: 2026-11-01. Status: PREPARED / NOT_RUN. This implements preparation for step I of the [repair plan](2026-11-01-review-fix-implementation-plan.md). It does not authorize or record a flash, COM3 read, listening test or physical acceptance.

Subsequent owner-assisted execution is recorded separately in the [device results](2026-11-01-review-fix-device-results.md): I01 timing failed and s07 was restored. The preparation status above describes the original checkpoint, not the current installed image.

## Entry gates

1. Finish the offline A–H execution record, including test/tool logs and a package manifest whose source hashes still match the checkout. Use the final verified package list, not the intermediate step-A package.
2. Read the [s08/s09 reconciliation](2026-11-01-review-fix-history-reconciliation.md): offline ancestry is verified; historical captures have timing/underrun limits and no captured listening acceptance. Complete the live identity gate before any write.
3. At the later device session, verify the actual running image and unit identity. This is the observed JC1060 NewPanel v1.3/103 backend; native minimum 302 remains unchanged. Do not inherit JC8012 or native-kit readiness.
4. Check both whole-flash factory backups, secondary copies, physical BOOT/RESET recovery path and the current flash-plan ancestry using the existing recovery procedure. Preserve tail sectors from larger predecessor images. Do not guess that `deck-play` is still installed.
5. Create a fresh flash plan for the chosen final `deck-engine` package with the actual current predecessor chain. Archive its SHA and the exact source diff; it is an uncommitted-source build until a later publication creates a matching commit. Build-only manifests are not flash plans.
6. Prepare the serial monitor before requesting owner actions. Record firmware app SHA, reset reason if captured, source/package/plan identity, card/export identity, controller VID/PID/descriptor/channel map and physical power topology.

## Ordered cases

| Case | Procedure | Required observation |
|---|---|---|
| I01 boot/idle | Flash only after entry gates; verify readback, then normal startup | Expected image/variant and owners, no boot fault; silent idle; recovery remains available |
| I02 separate outputs | Known different tracks on both decks; listen to MASTER and headphones; alternate PFL and headphone mix | MAIN and PFL route correctly; gains match accepted policy; neither stream copies the other |
| I03 transport | Both decks: Play/Pause, CUE return/set/hold/release, paused jog seek, pitch and bend, local Hot Cue | Sound, displayed position, LEDs and transport agree |
| I04 LOAD race | Repeated LOAD then Play at different delays; LOAD while playing; repeated load presses | Reserved load can be canceled before replacement; active replacement rejects transport; no stale title/source or unexpected unload |
| I05 seek receipts | Forward/backward, rapid repeated seek, exact/beyond EOF, seek from EOF, seek during a subsequent load | Every current seek terminates; old receipt cannot move the new source; failure is stopped/faulted, not playing silently |
| I06 concurrent production | A plays while B loads/seeks long MP3/FLAC/WAV; invert decks; 44.1/48 kHz, pitch endpoints | Active rings maintain margin, no steady underruns; bounded per-turn work; large RIFF/ID3 metadata does not monopolize the producer |
| I07 details policy | Browse/tap while playing, then pause both | UI reports pending details during playback; latest selection is served after pause; playback stays stable |
| I08 SD loss | Separate runs: full ring, pause, active seek, second-deck load, pending detail | Old lease revoked; cached render clears; no late success; affected decks stop; UI reports loss. Already queued USB packets may drain |
| I09 SD return | Reinsert same card; follow current app-restart-required policy | Fresh init/catalog and clean read/playback after restart; no claim of automatic hot remount |
| I10 controller loss | Unplug in normal playback and with held CUE/jog; replug after retirement | Holds released, no automatic preview resume; old transfers retired before reuse; fresh connection generation |
| I11 USB error recovery | Use controlled fault injection if available; otherwise label NOT_RUN | Completion/fill/submit/partial-start error reaches recovery or explicit fault; no dead stream reported as healthy; no free while busy |
| I12 drain failure | Host evidence exists; target only with a supported injection mechanism | Resources and device held if callback retirement cannot be proven; no new session overwrites their contexts |
| I13 cold/restart/soak | Operator power cycles and extended exact-image playback after earlier cases pass | Explicit restart counts, reset telemetry, stable memory/stack, no stream loss; operator listening acceptance recorded separately |

## Telemetry and provisional thresholds

- Observe `deck_mix_stats`, `deck_engine_stats`, `usb_uac_stats`, `producer_stats`, media/load terminal reports, heap minima and per-task stack watermarks. Count a missing/stopped transfer pool as a failure even if underrun counters are zero.
- Four six-packet transfers provide 24 ms of scheduled audio. After source loss, new fills must be silent; packets already owned by USB may still drain. Measure actual mute latency and capture detection/queue timing; 24 ms is queue coverage, not a certified end-to-end loss bound.
- Reject any fill reaching 6000 us. An initial target is at most 3000 us worst fill for 50% headroom; this is a proposed campaign criterion requiring registration before execution, not an existing measurement.
- Require zero steady-playing underruns, unexpected resets, transfer/packet faults and source-identity mismatches in normal cases. Injected faults must be counted and terminate in the documented recovery/fault state.
- Record minimum buffered time after rate/pitch conversion and maximum producer gap. Require measured refill gaps to stay below available ring coverage with explicit margin. Synchronous filesystem open/read and driver timeouts remain physical latency limits; do not claim a 1 ms hard deadline from the soft decode budget.
- Keep existing build stack/frame and partition limits. Their static direct-call estimates exclude indirect calls; verify actual high-water marks on the candidate.
- Bounded USB retries are three per connection. A fourth fault remains unavailable/held until controlled teardown/reconnect; no unlimited retry loop.

## Evidence and exit

Archive raw captures, scenario definitions, operator replies, app/package/plan/source hashes and structured per-case outcomes on the primary and secondary evidence storage. Historical logs and acceptance remain immutable. Use PASS / FAIL / NOT_RUN / NOT_CAPTURED distinctly.

If the candidate is experimental or fails acceptance, restore the reconciled known accepted image through its recorded plan and verify its normal behavior. Close the monitor and state exactly which image remains installed. This runbook is preparation only; every physical case above was NOT_RUN at preparation. Read the separate device results for later case outcomes.
