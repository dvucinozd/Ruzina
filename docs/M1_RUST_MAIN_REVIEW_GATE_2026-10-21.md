# M1 development stack integration review

Date: 2026-10-21. Review corrections are based on
`e1891a93e332d7d0b9e5d9d5eb7346c8a622ea60`, carried on the final PR #11 branch.
Scope: review and consolidate PRs #5 through #11 into main, in dependency order.
APTA PR #2 remains separate; no hardware activation or acceptance is implied.

## Reviewed dependency chain

| PR | Package | Original reviewed head |
| --- | --- | --- |
| #5 | PCM history/arbitration, output delivery/completion, routing and boot diagnostics | `6ea7b977aa2ea0390bcbd578e99118affd78c234` |
| #6 | Pinned Rekordbox analysis and deck/waveform consumers | `963e730f614f948830f77ee6c93801e34f6105c7` |
| #7 | Interactive deck/Library/cue UI | `df1524b9a98ba52a740577f718017626619732da` |
| #8 | Mixer/PFL and Beat/Pad FX controls | `eac8481e2a91f7eaf8971c36bfa8188ba42a3f3e` |
| #9 | Service counters, event history and diagnostics UI | `4252d6d48eaaec33eb5b0f03403591cb579387d5` |
| #10 | Publication generation/freshness and counter adapters | `f1786a96fcbee17a3f0a25c142deff9f17582010` |
| #11 | Composed product/controller/media owner health | `e1891a93e332d7d0b9e5d9d5eb7346c8a622ea60`, plus these corrections |

The original heads form one ancestor chain. Nine retained source manifests
(including the three stages inside #5) were checked against their exact
historical commits: all **191 file hashes** match. External lockfile records
remain identical to pre-integration main (503 host / 207 target records).
The stack has no hardware-file diff, and all 18 pre-existing modified KiCad files
in the primary checkout retain their original recorded hashes.

Review focused on generation/lease invalidation, completed-output transport,
controller-only headphone routing, bounded analysis publication, semantic UI
state, cue retention and runtime-health recovery. It found three actionable
regressions, matching the existing inline findings on #7, #9 and #11.

## Corrections before integration

1. **Shared track cue edits:** local cue banks on two decks could diverge and a
   later edit could replace the session bank with a stale snapshot, resurrecting
   a deleted source cue. Accepted edits now refresh local banks on decks sharing
   the same track. The firmware ProductRuntime also checks the media lease.
   The refresh validates track identity and changes neither source cues nor
   transport state. Rejected/backpressured product transactions do not publish
   the staged edit. Tests cover interleaved deletion/store, reload, playback
   preservation, different-track isolation and the actual ProductRuntime path.
2. **EOF counter:** repeatedly seeking beyond the duration could count the same
   stopped endpoint many times and evict useful events. Host control handling
   now captures the pre-control endpoint state, because the semantic handler can
   already change position before returning Seek. It records EndOfTrack only
   on entry. Moving away and returning, or a new playback ending, still counts.
3. **Media setup cleanup:** class construction, LUN selection or capacity probe
   failure could invalidate the media producer without sending the terminal
   event needed to claim its replacement. All three cleanup paths now notify
   the media worker through one helper after releasing USB resources. The
   explicit SetupFailed reason yields a current Failed observation and retained
   failure counter. A later attachment can recover to Idle. The host test
   covers the pre-Ready coordinator/worker token protocol, not physical USB
   failure injection; target compilation/linking covers the actual cleanup paths.

The two new desktop regressions were first run against the untouched original
`e1891a9` session source with the new tests retained. Both failed: a shared cue
deletion was lost, and the EOF counter reached 41 instead of 1. The corrected
source passes. The original component was restored to the candidate after the
controlled reproduction; no primary-checkout files were used for it.

## Verification and integration procedure

Final corrected source passes **591 host tests**, workspace formatting and
warnings-denied host clippy, desktop compile, **33 P4 check/link configurations**,
**24 expected board/feature rejections**, and all **38 unchanged UI pixel goldens**.
The composed owner test suite now has nine passing integration cases. Evidence,
original regression failures, CI annotations and source hashes are retained in
[the review evidence](evidence/m1-main-review-2026-10-21/README.md).

The merge procedure preserves history and checks the exact selected head of
each PR. After each merge, main's Git tree must equal that PR head's tree before
the next dependent PR is retargeted to main. The last merge must therefore
retain the exact corrected/validated source. No administrator bypass, force
push, branch deletion, APTA retarget or hardware write is part of this procedure.
The primary main checkout is advanced only by fast-forward after verifying that
incoming paths do not intersect its retained local KiCad modifications.

All 14 original PR CI jobs were confirmed to have failed **before starting**
because of the account payment/spending-limit restriction. These are not code
test failures, and local PASS is not hosted CI PASS. Current merge/run identities
are recorded by GitHub; integration does not declare this account gate solved.

## Still open

The Guition profiles remain scheduler/opt-in boot-diagnostic skeletons. Legacy
runtime owner wiring does not select Guition pins. Exact-unit recovery, silicon
and memory observations, display/touch, storage, USB power/cancellation/timing,
physical output, listening, durable-write failure policy, remaining decoder/FX
execution and full product UI acceptance retain their existing gates.
MAIN preference remains ready external DAC -> compatible controller -> mute;
headphones/CUE/PFL remain controller-only. APTA remains unmerged.
