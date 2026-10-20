# Review: codex/m1-rust-foundation into main

Date: 2026-10-20

Follow-up: the owner approved the proposed foundation repair/integration and explicitly deferred the APTA branch. See [integration validation](M1_RUST_MAIN_INTEGRATION_2026-10-20.md) for the repair and subsequent evidence. Results below remain the historical review of `396b6fd`.

## Recommendation

Use `codex/m1-rust-foundation` as the next `main` development baseline, after correcting the reproduced render-underflow defect and validating the final commit. Preserve the development history through the existing [PR #1](https://github.com/dvucinozd/Pajoniiir-M1/pull/1). This review does not merge, commit or push anything.

The reviewed branch is a substantial software foundation. It is not yet a complete DJ application or a qualified Guition board image. The [Guition platform decision](M1_GUITION_PLATFORM_DECISION_2026-10-20.md) remains the active hardware direction.

## Exact Git scope

| Item | Verified value |
|---|---|
| `main` / `origin/main` | `56175db33f204afd886869df6bd632f3e9f41996` |
| Reviewed branch / remote tip | `396b6fd34b9f273281ffece1ceae58fe392af916` |
| Commits unique to main / foundation | 0 / 384 |
| Changes from main | 118 files, 52,952 inserted lines |
| Tracked `hardware/` difference | None |
| Git merge-tree result | Clean; committed main is an ancestor, so fast-forward is possible |
| Existing PR | #1, open, mergeable; checks currently reported as failed |
| Separate APTA branch | Foundation has 96 unique commits; `codex/native-apta-consumer` has 24 unique commits |

The 18 pre-existing local KiCad modifications were preserved. The local Guition documentation changes also remain uncommitted. A patch of the three tracked documentation edits passes `git apply --check` against the reviewed branch. An actual merge must first preserve/integrate these edits, especially README, which both the branch and local working copy change. The new platform-decision document must be retained too.

The separate APTA branch is not contained in foundation. Keep it and integrate its unique work separately after the foundation baseline is established.

## Finding F1 — P2: starved resampling can replay cached audio

Location: `firmware-rust/crates/pajoniiir-audio-runtime/src/render.rs:209-222`, interacting with `ResamplerState::next` in `pajoniiir-audio-dsp/src/lib.rs`.

`missed` is reset for every output frame and only becomes true when the resampler actually calls the source callback. When the source/output rate ratio is below one, some output frames require no new source frame. After a failed source read, the resampler retains its previous/current samples; on the next interpolation-only frame, `missed` stays false and the renderer emits retained audio again.

Fresh reproduction against the reviewed commit:

- 24 kHz source, 48 kHz render output, neutral pitch;
- supply four frames with left/right samples `1000/-1000`, then return `None` permanently;
- the block reports 60 underflow frames;
- even after output frame 16, every even frame through 126 emits `1000/-1000` instead of silence;
- the independent assertion fails: `source starvation repeated cached audio`.

This also matters for other effective ratios below one, including reduced pitch. The existing starvation test uses a 1:1 ratio and passes without exercising the failing case. Hardware output is not connected yet, so the observed result is a software-render failure, not a listening-test result.

Before merge, make underflow state persist across interpolation-only frames and define recovery when fresh source data returns. Add regression coverage for sub-unity ratios, complete starvation and recovery while preserving generation/seek handling. Verify both MAIN and PFL behavior. Do not merely reset the resampler in a way that prevents sub-unity phase accumulation and recovery.

## CI state and evidence

Latest checks on `396b6fd`:

- [Host run 37448076015](https://github.com/dvucinozd/Pajoniiir-M1/actions/runs/37448076015): failed before any steps.
- [P4 run 37448076142](https://github.com/dvucinozd/Pajoniiir-M1/actions/runs/37448076142): failed before any steps.
- Both job annotations report failed recent account payments or a spending limit that needs increasing. These are infrastructure failures; they provide no source-code failure result.

The previous functional checkpoint `bbf269656fb4f75f60f81a16465dd281c6d2ffa8` passed [host CI](https://github.com/dvucinozd/Pajoniiir-M1/actions/runs/37446784558) and the [P4 compile/link matrix](https://github.com/dvucinozd/Pajoniiir-M1/actions/runs/37446784516) on 2026-10-06. The host log contains 468 passing tests including the simulator. Between that checkpoint and the reviewed tip, changes are one documentation file and removal of one unused embedded import. These historical successes do not make the current GitHub checks green.

Fresh local checks on the exact reviewed tip, using isolated Rust 1.95.0 in Ubuntu/WSL:

| Check | Result |
|---|---|
| Host `cargo fmt --all --check` | PASS |
| `cargo test --locked --workspace --exclude desktop-simulator --quiet` | PASS, 465 tests |
| Host Clippy, same exclusion, all targets, warnings denied | PASS |
| P4 `cargo build --locked --release` | PASS; 6 compiler warnings |
| P4 release link with `render-graph-bringup,sdmmc-profile-storage-bringup` | PASS; 3 compiler warnings |
| Independent underflow regression | FAIL, reproduced F1 |
| Fresh desktop simulator build/runtime | Not run; required native Linux dependencies absent |
| Physical Guition board tests | Not run |

Host format/Clippy cover the host workspace; the nested embedded workspace is checked by the P4 release builds above. The fresh P4 checks cover default and combined render/USB0/USB1/PSRAM/SDMMC ownership, not every separate CI feature configuration.

Logs and the standalone failing test are retained in `docs/evidence/m1-rust-foundation-review-2026-10-20/`. The clean review checkout is `C:/Users/Daniel/.codex/worktrees/m1-rust-review/M1_PCB`; the runnable probe and local runner are in its sibling `review-evidence/` directory.

Both Rust workflows currently trigger only for pull requests and manual dispatch. Add path-filtered `push` checks for `main` when adopting it as the active baseline, so direct updates and the merged main commit receive Rust checks too. Also keep or strengthen the pre-merge PR checks; changing triggers does not resolve the billing condition.

## Implemented and remaining work

| Area | Current foundation | Remaining boundary |
|---|---|---|
| Product/control | Deck/Mixer state, semantic controller events, bounded scheduling, profile storage and LED feedback | Physical controller qualification; product completion |
| Media | USB MSC ownership, partition discovery, native async FAT32/exFAT, Rekordbox catalog and load paths | Real storage interoperability, throughput and detach/recovery evidence |
| Audio | PCM16 WAV source, bounded PCM queues, resampler/EQ/filter/mixer, fixed-rate reference render, Hot Cue journal | F1; live Master Tempo/scratch arbitration, live FX, MP3/FLAC, I2S/DMA and UAC output |
| UI | Slint desktop fixture and waveform/UI model crates | Product-connected UI, embedded display/touch/BSP integration |
| Analysis | Neutral track-analysis contracts | Separate native APTA integration branch |
| Board | P4 no_std target with minimum revision 302 | Explicit BSPs for both selected Guition boards and exact-unit bring-up |

The existing embedded target still contains the custom M1 pin map (including microSD GPIO39..46 and USB FS GPIO27/26), 16 MiB flash configuration and custom-board references. Do not enable those features on Guition units by model-name inference. Reconcile pins, memory, peripherals and partition bounds per board as described in the platform decision. Missing board qualification does not prevent merging the software foundation as a development baseline.

## Proposed merge sequence

1. Correct F1 on the foundation/integration branch and retain a failing-then-passing regression for starvation and recovery.
2. Carry forward the Guition decision and update the Rust README/architecture/requirements so custom-PCB assumptions are explicitly historical. Preserve the local KiCad edits separately.
3. Add `main` push triggers for both Rust workflows. Validate the final commit with host checks and the P4 matrix; restore current GitHub CI execution when the account condition is resolved.
4. Merge through PR #1 preserving history, then verify the local and remote main SHAs. No firmware flashing is part of this merge.
5. Continue Guition BSP work from main and integrate the separate APTA changes through their own review.
