# JC1060: the MP3 frame walk makes seeks fast

Date: 2026-11-04. Branch: `ogranak/jc1060-mp3-seek-index`, from `main` at
`150b486`.

## Problem

In the [two-deck run](2026-11-03-dual-mp3-opt3.md) a hot cue jump on an MP3
deck stalled for about 10 s, and the deck then underran 11 times.

- **Cause:** without an index or a nearby checkpoint, an MP3 seek decodes from
  the start of the file to the target. It reads the card 512 bytes at a time,
  and each read takes about 2 ms through the SD task.
- **Unused support:** the shared decoder can resume at an exact indexed frame
  (`index_read`/`provide_index`, `Mp3SeekIndex`). The legacy producer never
  drove the frame walk.

## Change (`deck_play.rs`)

- When the producer has nothing to decode (the ring is full or the track has
  ended), each turn spends one card read on the MP3 frame walk.
- A walk read counts as work. The owner then polls again without waiting a
  10 ms tick, so the walk runs at card speed, one read per deck per turn.
  Decoding always comes first.
- `IndexScanPolicy` stops the walk after three failed reads in a row, until
  the next load.

## Evidence

- **Host:**
  - New test `the_mp3_frame_walk_makes_a_far_seek_cheap`, for a 7 s seek on
    8 s fixtures:

    | Fixture | Card reads, cold | Card reads, indexed |
    |---|---:|---:|
    | CBR | 187 | 12 |
    | Xing-tagged VBR | 101 | 15 |

  - **Untagged VBR** has no exact indexed resume in the shared decoder yet,
    so it still decodes from the start. That limitation is recorded in the
    test.
  - **PCM:** in all three cases the audio after the seek is identical with
    and without the index.
  - **Totals:** tests pass for `idf-deck-play` 58, `idf-deck-mix` 59+24,
    `idf-deck-engine` 73+24 and `idf-producer-profile` 80+24. Clippy passes
    with `-D warnings`.
  - **Regression check:** the existing "a slow prime never starves the playing
    deck" test (at most 2 reads per turn) still passes.
- **Build:** `BUILD_PACKAGE_PASS`.
  - The app is 817952 B (78.0 %), SHA-256
    `28fd7e9757863b6826b5a7229c38e59d7e35a79de0f112fea7a0d415d7e9e569`.
  - The load chain is 50736 B and the UAC fill chain 3056 B.
- **Device** (`s12-mp3-seek-index-*` in `D:\BoardEvidence`):
  - **Owner test and verdict ("sve je uredno"):** both MP3 decks playing,
    with hot cue jumps on both decks.
  - **Seek time:** a hot cue seek now takes at most 78 ms on deck 1 and 40 ms
    on deck 2 (`prime_max_us`), against about 10 s before.
  - **USB:** `packet_errors=0`, no faults.
  - **Underruns:** 8/6 in total. They come in small bursts, a few render
    blocks right after each seek, and none at other times.

## Open

- **Short underrun bursts after a seek.**
  - When a seek completes, the deck resumes as soon as its first block
    arrives. The new epoch then holds about one block, so the next few render
    blocks underrun.
  - The fix is a resume cushion: keep the deck held after a seek until its
    new epoch has buffered, for example, about 100 ms, or the track has
    ended.
- **Untagged VBR MP3** seeks still decode from the start.
