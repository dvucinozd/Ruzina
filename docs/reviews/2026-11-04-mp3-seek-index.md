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

## Follow-up: seek resume cushion

- **Why:** a completed seek used to resume playback on its first block alone,
  so the next few render blocks underran.
- **Engine (`deck_engine.rs`):** after a seek, a playing deck stays held until
  its new epoch has buffered 4096 frames (about 93 ms at 44.1 kHz) or its
  track has ended. A slow producer can hold the deck for at most 375 render
  blocks (1 s).
- **Host test:** `a_seek_resumes_playback_only_after_a_buffered_cushion`.
  - A Hot Cue jump while playing with only the seek's first block in the ring
    keeps the deck held with no underrun and no movement. Once the producer
    catches up, the deck plays on from the target.
  - The one-second cap releases a starved deck.
  - With the cushion disabled the test fails with 4 underruns, which matches
    the device. All feature sets pass: 58, 59+24, 74+24 and 81+24. Clippy
    passes.
- **Build:** `BUILD_PACKAGE_PASS`.
  - The app is 818320 B (78.0 %), SHA-256
    `09d4f04c2573755465469ab02cb18c553e48e69b0c382f38d22c67e757c45d5d`.
  - The load chain is 50736 B and the UAC fill chain 3040 B.
- **Device** (`s13-seek-cushion-*`): deck 1 played FLAC and deck 2 MP3, both
  playing, with 6 Hot Cue and CUE seeks.
  - **Owner verdict:** "čini se sve uredno".
  - **Underruns:** 0/0.
  - **USB:** `packet_errors=2` with no restart, no faults.
  - **Deck 1 (FLAC) seeks:** at most 125 ms.
  - **Deck 2 (MP3) seeks:** 7.9 s and 11.5 s (`prime_max_us`).
    - That track's walk had not finished about 50 s after its load. The FLAC
      deck was taking most of the card's read time, about 1.8 ms per 512-byte
      read.
    - So those seeks still decoded from the start. In the previous run the
      same track seeked in 78 ms next to another MP3.

## Open

- **Card read throughput.** Every read is one 512-byte sector through the SD
  task, so a whole-file frame walk competes with decoding. Next to a FLAC deck
  it can take more than a minute.
  - Multi-sector reads (for example 8 KiB per request) would speed up decode,
    the walk and cold seeks together.
  - That is the next step.
- **Untagged VBR MP3** seeks still decode from the start.
