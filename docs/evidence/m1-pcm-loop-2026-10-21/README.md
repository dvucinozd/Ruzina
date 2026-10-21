# Local PCM loop evidence

See [the implementation, control semantics and limits](../../M1_RUST_PCM_LOOP_GATE_2026-10-21.md).
The baseline is integrated main `d81a300784f81da4e3c7288135c6504e8d6f8623`.
`manifest.json` records the exact changed source/document hashes, lockfiles,
retained logs and preservation results. SHA-256 text hashes normalize CRLF to LF.

| Transcript | Verified result |
| --- | --- |
| `host-fmt.log`, `host-clippy.log`, `host-tests.log`, `desktop-check.log` | Workspace formatting, warnings-denied Clippy, 614 host tests and desktop compilation pass. A successful formatting log can be empty. |
| `audio-focused.log` | Includes the twelve new `loop_playback` cases and existing audio/deck/board tests. Counts overlap the workspace run. |
| `composed-clippy.log`, `composed-failures.log` | Explicit runtime-owner test Clippy and eleven composed cases pass, including the new real product Loop IN/OUT/ClearLoop scenario. |
| `runtime-owners.log` | Nine owner-health scenarios pass. |
| `ui-owner-replay.log` | Both UI geometries pass interaction replay and all 38 reviewed pixel goldens remain unchanged. |
| `p4-matrix-summary.log`, `p4-01.log` through `p4-33.log`, `board-guards.log` | All 33 workflow check/link commands and all 24 expected feature rejections pass. Existing target warnings remain visible. |
| `loop-carry-before.log` | **Expected baseline failure:** the original `d81a300` WAV decoder, with the new stream/test code retained, fails the full-mono-block split-read regression. |
| `loop-carry-after.log` | The exact restored corrected decoder passes the same filtered test. This does not add another test to the 614 count. |
| `integration.json` | PR #12 merge identity, selected-head/main tree equality and the verified CI account restriction. |

The WAV stream and canonical source tests execute actual no_std project code,
using memory-backed source bytes in place of filesystem reads/seeks. They check
exact PCM order at steady wraps, arbitrary streamed loop length, bounded short-loop
caching, partial bytes, edits/EOF, generation and receipt rejection, confirmed
position, two decks and source-mode interaction. Driver scheduling, physical I/O
latency and listening are not simulated or qualified by these results.

The capture verifies both unchanged lockfiles, no historical hardware diff, all
18 original primary KiCad modification hashes preserved, and APTA unmerged at
`fd7ebd592b469b27c9f45337dced2f9b2912966f`. Guition peripheral ownership and physical
audio remain pending; headphones/CUE/PFL retain controller-only routing.
