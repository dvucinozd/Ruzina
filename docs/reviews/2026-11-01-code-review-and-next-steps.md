# JC1060 code review and development checkpoint

Date: 2026-11-01. Outcome: **CHANGES_RECOMMENDED_BEFORE_DEVICE_EXPANSION**.
This is a review and proposed repair order, not an implementation or hardware run.

## Scope and exact state

- Checkout: `D:/AI/M1_PCB`, branch `ogranak/jc1060-deck-engine`.
- Reviewed HEAD: `b35dcd39f4c1ddb9f4b253d6af89110ee68a2335`.
- Fetched `origin/main`: `bd52402160af27a3e234f7647d703a847644817b`.
- [PR 71](https://github.com/dvucinozd/Ruzina/pull/71), `deck-mix`, is open against main; head `4540ec9ac7cc57b22de924d53855061f67a766fc`.
- [PR 72](https://github.com/dvucinozd/Ruzina/pull/72), `deck-engine`, is open against PR 71's branch; head equals the reviewed HEAD.
- Primary review: the 29-file committed diff from main, the surrounding legacy producer/USB/media lifecycle, shared deck/load/render contracts, tests and development records. This is not an exhaustive audit of every shared crate or vendored ESP-IDF/USB source file.
- Existing uncommitted external-audio planning edits were preserved. No production source was edited, staged, committed, pushed, merged or flashed. COM3 was not opened.

## Validation and evidence

- Current legacy `idf-deck-engine`: **52 tests PASS**, host Clippy with `-D warnings` PASS. [Log](evidence/2026-11-01-code-review/legacy-tests-clippy.log).
- Selected shared crates (`ruzina-deck`, `ruzina-audio-runtime`, `ruzina-mixer`, `ruzina-uac`, `ruzina-media-load`, `ruzina-controller-session`): **338 tests PASS**. [Log](evidence/2026-11-01-code-review/shared-tests.log). This is not a fresh whole-workspace run.
- Legacy Python tooling: **26 tests PASS** using the existing IDF 6.1 Python environment and CMake. The initial default-shell attempt lacked CMake/esptool and produced 13 environment errors; it was not treated as a code failure or PASS.
- Documentation links before adding this report: **326 PASS**.
- Final documentation links after recording the review: **339 PASS**; `git diff --check` PASS; the test-only patch passes `git apply --check` and remains unapplied.
- GitHub at this HEAD: [legacy workflow](https://github.com/dvucinozd/Ruzina/actions/runs/41299787601) PASS, including its host job and all **17 IDF build variants**; [host workflow](https://github.com/dvucinozd/Ruzina/actions/runs/41299787599) PASS. No new local full IDF build or native-device run was performed for this review.
- Four review-only regression tests were added to an **isolated copy** of the legacy Rust core and run against unchanged production code. **All four fail**, exposing three defect classes below. [Failure log](evidence/2026-11-01-code-review/review-regressions.log), [test-only patch](evidence/2026-11-01-code-review/review-regressions.patch). The patch is not applied to this checkout; apply it only in the subsequent repair branch or an isolated reproduction copy. Its multithreaded snapshot test is schedule-sensitive; the other three use deterministic state transitions.
- CodeRabbit: **1 issue, minor**, confirming the failed-seek defect in R2. Automated and independent review findings are recorded separately below.

To reproduce the four review regressions, apply the test-only patch in an isolated checkout of the reviewed HEAD, then run:

```sh
cargo +1.95.0 test --locked --manifest-path firmware-idf/legacy-jc1060/rust-core/Cargo.toml --features idf-deck-engine review_ -- --test-threads=1
```

Expected on this baseline: four failing tests (the publication stress case depends on thread scheduling). These are desired-behavior regressions, not passing evidence. The isolated `/tmp` copy was later removed by the WSL restart; the patch and captured failure log in this report preserve the reproduction.

## Findings and concrete repairs

### R1 / P1: LOAD admission is not reserved across asynchronous replacement

Locations: `rust-core/src/deck_engine.rs:162-166, 207-221`, `rust-core/src/deck_play.rs:339-368`, `rust-core/src/controller_midi.rs:448-459` (all under `firmware-idf/legacy-jc1060`).

Trigger: a stopped loaded deck accepts LOAD, then PLAY arrives before the producer finishes. `load_allowed` only checks the current state; no `DeckLoadGate` permit is reserved. The producer closes the old file and replaces the ring. The later `commit_track_load` rejects the new load because the deck now plays, but the adapter calls `unload_track` and clears playback anyway. The original shared commit contract explicitly preserves the previous deck on refusal; this adapter defeats that behavior.

Evidence: `review_play_after_load_admission_must_preserve_loaded_track` fails because the late completion leaves `engine.loaded[0] == false`.

Repair: reuse the shared `DeckLoadGate` transaction. Reserve at admission; carry the permit and load generation with the request; call `begin_replacement` immediately before closing the old source; make PLAY/jog claims cancel a reservation or reject an active replacement; release the permit on every terminal path. A superseded completion must not unload the current source. Publish UI track identity only when the corresponding load commits.

Acceptance: LOAD->PLAY before replacement retains the old source; PLAY during replacement is explicitly refused; rapid A/B loads cannot publish stale A; failure and cancellation release the gate. Test both decks and rejected UI updates.

### R2 / P1: failed and EOF seeks can leave transport permanently waiting

Locations: `deck_engine.rs:185-189, 230-245`; `deck_play.rs:482-485, 501-535`; `idf_media_bridge.rs:803-808`.

The engine clears `seeking` only after observing a different ring epoch. A failed decoder change/read/prime ends the **old** epoch and reports `Seeked { ok: false }` only to the C log. Seeking to the exact end of a valid WAV also takes this path because an empty prime block is classified as failure. The engine then skips position updates and end-of-track handling: its playing flag/LED can remain on after the old PCM drains into silence.

Evidence: both `review_failed_seek_must_not_wait_forever_for_new_epoch` and `review_seek_to_eof_must_publish_a_new_terminal_epoch` fail. The latter uses the existing 250 ms WAV and returns `Seeked { position_ms: 250, ok: false }` without changing epoch.

There is also a race at request publication: `COMMANDS[i].seek(...)` precedes sampling `RINGS[i].epoch()`. The producer can complete between those operations, causing the engine to wait for an epoch after its own completed seek. Two outstanding seeks also cannot be distinguished by merely observing any epoch change.

Repair: add an explicit terminal seek receipt containing request ID, track/load generation, resulting epoch, actual position and success/EOF/failure. Consume only the current request's receipt; clear the hold on every terminal outcome; define a failed seek's stopped/faulted state. Exact/beyond-EOF seeks must produce a coherent terminal source. Capturing the old epoch before publishing is useful but insufficient to solve stale acknowledgements and failure completion.

Acceptance: backward/forward/EOF/beyond-EOF, read failure, canceled and superseded seeks, seek after EOF, and fast producer completion. Position, PCM, LED and load admission must agree. Do not use `ended && empty` alone to infer failure: that is also true before a valid seek away from an already-ended source starts.

### R3 / P1: ring epoch and its metadata are not a coherent snapshot

Locations: `deck_play.rs:136-142, 172-184`.

`restart_at` writes `start/rate/origin`, then releases the new epoch. `Reader::sync` acquires the epoch only once and subsequently reads those fields. A later restart can change the fields between the reads, or while an older published epoch is being consumed. Acquire/release does not prevent observing metadata belonging to another publication. The reader can assign a source position/rate/start that does not belong to its epoch.

Evidence: `review_ring_epoch_metadata_snapshot_is_coherent` calls the real producer/reader methods on two host threads. This run observed **epoch 1 with origin 2**. The shared PCM slot ownership is not alleged to be a proven memory corruption here; the demonstrated defect is mixed publication metadata.

Repair: publish and read the complete source descriptor using a correct single-writer sequence protocol or another bounded snapshot mechanism. Mark writes in progress before mutating fields, validate the snapshot before committing local/shared tail, and retain the previous view when a snapshot is busy. Keep retries bounded on the USB audio task. Couple end-of-source metadata to the same generation protocol.

Acceptance: concurrent restarts with distinct rates/origins, full ring, rapid seeks and counter wrap. Verify coherent tuples and no stale tail publication; use interleaving tests in addition to a stress loop.

### R4 / P1: SD lease loss does not invalidate the audio/engine state

Locations: `idf_media_bridge.rs:754-761, 816-818`; `main/sd_bridge.c` client/owner termination; `deck_engine.rs:230-245`.

When the live media lease is lost, `serve_decks` can return while both PCM rings still contain audio and the engine still regards their tracks as loaded. Its failure exits do not publish deck invalidation or terminal ring state. If a long track's producer was idle because its ring was full, it need not pass through the decode-error path that calls `ring.end()`. The renderer can drain the old data and then accumulate underruns while transport remains playing. The SD owner's correct DMA teardown does not itself revoke the renderer's data.

Evidence: source/control-flow review; no card removal was performed in this review. Previous isolated SD-removal evidence does not qualify this combined playback path.

Repair: carry the media lease/load generation into audio publications, revoke it when media loss is detected, stop affected decks and invalidate cached render/source state through the owning tasks. Ensure every early-return path emits the same terminal notification. Reject old commands and late receipts; expose the fault in UI. Preserve any independent live source according to an explicit policy.

Acceptance: removal while rings are full, while paused, during detail scan, during seek and during second-deck load; both decks lose only their affected lease, never revive old PCM, and stop without endless underflow. Reinsertion needs a fresh owner/mount/catalog generation; current firmware explicitly requires an app restart.

### R5 / P2: a transient USB audio error permanently removes a transfer

Location: `main/usb_host_enum.c:288-303, 333-349`.

Only a transfer completed with `USB_TRANSFER_STATUS_COMPLETED` is submitted again. An error, failed refill or failed submit leaves its slot idle forever; no periodic UAC retry/recovery exists. The pipeline progressively loses its queued transfers, and after all slots fail, `UAC_STREAMING` may remain set with no audio scheduled. This differs from the existing MIDI retry path. Submit failures here also lack a dedicated counter.

Evidence: source/control-flow review; existing successful physical stream logs contain no injected transfer faults.

Repair: model slots and stream health explicitly. On recoverable faults, schedule bounded recovery from the client loop; on loss, stop admitting renders, drain/cancel old transfers and report an unavailable sink. Do not resubmit to a gone device. Handle partial startup and submit failure explicitly; fence callbacks by connection generation before reusing slots. Retain resources if teardown cannot prove all transfers retired.

Acceptance: completion error, submit error, partial allocation/start, unplug during EP0 and streaming, late callback and A->B reconnect. State must report recovery/failure rather than indefinitely claiming a live stream.

### R6 / P2: foreground detail/load/seek work can monopolize the shared producer task

Locations: `idf_media_bridge.rs:755-781`; `deck_play.rs:28-31, 369, 525`.

Alternating which deck polls first does not bound time inside a poll. Detail import runs to completion before either deck is serviced, and a load/seek prime can execute up to 8192 decoder actions including synchronous 512-byte reads. A slow card or large ID3/analysis section on deck B can prevent refilling playing deck A longer than its roughly 1.4-1.5 s ring coverage. This is a structural starvation risk, not a measured underrun on the user's card.

Repair: make metadata scan and prime incremental jobs, prioritize low-water audio refills, and budget work by both bytes/actions and elapsed time. Check cancellation and lease changes between chunks. Retain the filesystem's single owner; do not add unsafe concurrent access to the same volume to work around scheduling.

Acceptance: virtual slow-media tests and target measurements while A plays and B loads/seeks a large-tag file; continuous browsing/detail scans must not exhaust either playing deck's ring. Record worst refill gap and minimum buffered frames.

### R7 / P2: current-state documentation contradicts itself

Locations: `README.md:48-50`; `docs/DEVELOPMENT_STATUS.md:10-18` and its RP table; `docs/HANDOFF.md` current-unit block versus later unqualified "Current image"/"Next" paragraphs.

README still says audible output is pending; status begins with the old exFAT diagnostic as current; handoff correctly records `deck-play` near the top but later calls older diagnostics current and describes already-completed import work as next. The RP table does not distinguish the native backend from the active legacy adapter. An agent following the wrong paragraph can repeat completed work or choose an obsolete firmware baseline.

Repair: one current snapshot (main/head, open PRs, exact device image, next action), separate native and legacy capability columns, and clearly dated historical reports. Keep historical evidence immutable but label it as superseded. Update README, handoff, status and relevant PR checklists together; the CI checks listed as pending in PR descriptions have now passed.

## Maintainability improvements, after the correctness repairs

1. Replace loose per-deck booleans with explicit load/seek/source lifecycle states and tagged receipts. Reuse existing shared ownership contracts rather than implementing a less complete parallel adapter.
2. Pass rings/mailboxes into a testable owner where practical. The current global-state lock and one large scenario test make it difficult to cover reordered producer/control events independently.
3. Forward `SessionDelivery::Connection` into the deck owner explicitly. The legacy drain currently handles only `Control`; do not assume synthetic hold releases fully substitute for the deck engine's connection lifecycle. Add real ingest->disconnect->reconnect tests before defining resume/level policy.
4. Move formatted telemetry from the USB callback to bounded low-priority reporting. Measure deadline margin with logging enabled, not just render time.
5. Make `boxed_copy<T: Clone>`'s contract type-safe. Its bitwise copy is currently used for known non-owning decoder states, but `Clone` alone permits owning pointers. Prefer dedicated constructors/copy-safe types or a narrowly documented unsafe helper.
6. Keep static stack analysis, but call its result an estimate, not an upper bound: `deepest_stack` does not follow indirect calls. Retain target stack high-water/guard measurements under worst-case decoder+control load.
7. Track active transfer count, seek/load failures, buffer low-water marks, source generation and maximum producer service gap. A zero-underrun count without a live-transfer/state check can hide a stopped stream.

## Where development actually stands

| Layer | Verified state | Still open |
|---|---|---|
| JC1060 hardware/backend | Observed ESP32-P4 v1.3/103, separate pinned IDF 6.1 + shared Rust; recovery and backups recorded | Native minimum 302 unchanged; native firmware is not a valid image for this unit |
| Panel/touch | NewPanel display/touch and combined library status screen observed on this unit | Full product UI, scrolling large catalogs, live deck waveforms/artwork and settings integration |
| SD/Rekordbox | Read-only exFAT, shared PDB/ANLZ import, 25-track export, load preflight and detail display observed | Playback lifecycle repairs, hot remount, larger/corrupt/mixed-media cases; writable storage is separate |
| Controller | DDJ-FLX4 HS enumeration, MIDI/LED, combined display/media/controller and UAC1 headphone audio observed | Combined playback reconnect/fault qualification and other controllers |
| Last recorded flashed image | `deck-play`, app SHA `95d45203ec245a9f82455f16f15dd95ed6ad2bb8eaaca9cf85f0816ccd5dbfdd`; source `f8251ab406a49427227a641ce0bbab3446089bd6` | Device was not contacted now; this is the latest repository evidence, not a fresh identity read |
| Physical playback | Deck 1: owner accepted 44.1 kHz FLAC/MP3/WAV, Play/Cue/headphone level; 90 s rendered audio, 15 min USB stream capture, zero recorded underruns/ISO packet errors | 48 kHz source tracks, long sessions, two decks, MAIN/PFL together and fault campaigns |
| PR 71 deck-mix | Shared two-deck render graph, mixer, MAIN/PFL, EQ/filter/faders; host tests and target build CI PASS | Not flashed/qualified in the current records; repair common transport/source boundaries first |
| PR 72 deck-engine | Shared transport, CDJ CUE, seek, pitch/jog bend and local hot cues; host tests and target build CI PASS | Review defects above; physical acceptance pending |
| Advanced performance | Shared host implementations exist | Legacy scratch, loop/pad/beat FX effects are currently counted as unsupported; MT/keylock, analysis-driven Sync/quantize/beat operations, source cues/memory loops and persistence are not fully wired into this device profile |
| Management | Host/simulator settings, Web and signed OTA contracts | Real settings storage, C6/network/TLS, OTA trust/partition/rollback adapters and physical qualification |
| External audio | [Recorded plan](../boards/JC1060_EXTERNAL_AUDIO_PLAN.md): PCM5122 MAIN, optional TLV320DAC3100 headphones, separate I2C because of address collision | No confirmed wiring, driver or physical qualification |
| JC8012 / APTA | Separate board qualification / experimental exclusion retained | No transfer of JC1060 evidence; no original APTA integration-branch merge |

Physical evidence: [deck-play report](2026-11-01-legacy-deck-play.md) and [device index](../boards/readiness/jc1060-s07-deck-play-device-20261101.json). A 15-minute stream capture is not 15 minutes of uninterrupted track playback and does not qualify deck-mix/deck-engine.

## Ordered continuation

1. Repair R1-R4 with regression tests and explicit generation/terminal-state contracts. Include coherent ring snapshots and failure/EOF seek acknowledgements; do not fix only the visible stale LED.
2. Repair UAC recovery and bound producer/detail work (R5-R6). Reconcile status docs (R7). Run host tests/Clippy, feature-matrix checks and fresh package/CI gates on the resulting exact head. Review the repairs before merge. Merge the stacked PRs in dependency order, retargeting PR 72 after PR 71 lands; do not infer physical acceptance from merging.
3. Prepare the exact candidate and recovery chain for an owner-assisted JC1060 campaign: both decks, distinct MAIN/PFL, all CUE modes, rapid seeks/loads, 44.1/48 kHz sources, EOF/failure, SD loss and USB reconnect. Record CPU/render deadline, minimum ring fill, internal/PSRAM/stack and transfer/underrun counters. Preserve accepted output-level policy unless the owner requests a change.
4. Connect source ANLZ/cues/decoded duration and the missing shared performance paths in bounded batches: loops/beat operations, then scratch, MT/keylock and FX, with listening and timing gates for each. Do not advertise unsupported effects as active.
5. Replace the diagnostic library screen with the product UI and durable settings; then integrate C6/Web and OTA with an explicit flash layout/trust/rollback plan. The current roughly 808 kB app in a 1 MiB factory partition is not an implemented OTA slot layout.
6. Implement the external audio plan when modules, physical JP1 mapping/power and two-output timing can be qualified. Keep this independent of repairing the current controller output path.
7. Qualify JC8012 independently when available. Recorder, Pro DJ Link and APTA remain separately scoped later work.

No milestone percentage is assigned: host implementation, target build, observed device behavior and full qualification are different completion levels.

## CodeRabbit

The initial all-changes review was rejected with `too_many_files` (449 versus 150), while the Windows Git worktree had only the pre-existing planning edits. The committed-only retry reviewed the 29 files between main and HEAD. The exact initial error is preserved in [the log](evidence/2026-11-01-code-review/coderabbit-initial.ndjson).

The retry completed successfully and raised **1 issue (minor)**: failed seek leaves the hold set, at `deck_engine.rs:234-236`. This independently confirms R2; this review assigns it P1 because it can leave playback permanently silent with a live playing state. It does not imply CodeRabbit found the other issues in this report.

Evidence: [stored findings](evidence/2026-11-01-code-review/coderabbit-findings.txt), [raw issue](evidence/2026-11-01-code-review/coderabbit-issue.json), [exact reviewed Git state](evidence/2026-11-01-code-review/coderabbit-git.json), [completion marker](evidence/2026-11-01-code-review/coderabbit-complete.json). The CLI process returned zero. WSL restarted after completion and its `/tmp` transcript disappeared; the complete review and matching HEAD/base metadata were recovered from CodeRabbit's persistent local checkpoint. No full stdout transcript for the successful retry is claimed.

The suggested minimal `ended && drained` fix was not applied: it can also clear a valid seek initiated after EOF before its producer runs. Use the request/receipt protocol and regression cases in R2.
