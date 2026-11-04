# Untagged VBR MP3 seeks resume through the frame index

Date: 2026-11-05. Branch: `ogranak/vbr-untagged-seek`, from `main` at
`e7b1c57`.

## Problem

A far seek in a VBR MP3 without a Xing/Info/VBRI tag decoded from the start
of the file, even after the frame walk had finished.

- **Variable streams:** the codec's own seek (`nanoaudio-mp3`) trusts the
  caller's frame numbering only once it treats the stream as variable.
- **Untagged streams:** it learns that only when decoding meets a frame whose
  bitrate differs from the first. Before that it places frames by
  constant-bitrate byte arithmetic, which is wrong for VBR.
- **Result:** the shared decoder refused that case
  (`indexed_mp3_seek`, `(Plain, variable) => false`) and fell back to a
  linear decode, which is exact but slow.

## Change (`ruzina-audio-runtime/src/decode.rs`, shared with the native firmware)

For an untagged VBR stream whose walk is complete, `indexed_mp3_seek` handles
the seek itself:

1. It picks the indexed frame at or before the target frame minus 16
   (`MP3_RESYNC_LEAD_FRAMES`).
2. It resets the codec (`Mp3Decoder::reset`) and feeds it from that frame's
   walked offset.
3. While the resync lasts, `step` feeds one walked frame per call. Before
   each call it parses the frame header with `Mp3FrameHeader::parse`. It
   checks that the codec consumed exactly that frame, drops the output, and
   advances its own frame count.
4. Once the codec has decoded 2 whole frames (`MP3_RESYNC_SETTLE_FRAMES`),
   the output matches a linear decode. Normal decoding continues from there,
   and `step` skips the remaining frames up to the target.
5. Anything the walk did not promise falls back to decoding from the start,
   which is always exact. That covers:
   - a header that does not parse;
   - a different consumed length;
   - a stall at the end of the file;
   - a settle point past the target.

The codec itself is unchanged, so the native firmware benefits too.

## Evidence

- **Shared crate:** the test
  `untagged_vbr_seeks_through_the_index_and_reports_scanned_duration` on the
  untagged VBR fixture seeks to 7900, 6500, 4999, 3100, 777 and 30 ms.
  - The PCM is identical to a linear decode.
  - Each seek needs fewer than 24 reads, against more than 3 times as many
    linearly.
- **Controls:**
  - With no lead and no settle frames the test fails on the PCM comparison.
  - With one settle frame it still passes, so 2 leaves a margin.
- **Totals:** all `ruzina-audio-runtime` tests and clippy pass.
- **Legacy:** `the_mp3_frame_walk_makes_a_far_seek_cheap` now requires the
  indexed seek to be several times cheaper for untagged VBR too.
  - All 8 legacy feature sets pass, and clippy passes with `-D warnings`.
- **Build:** production `deck-engine`, `BUILD_PACKAGE_PASS`.
  - The app is 827344 B, SHA-256 `8398c731…`.
  - The load chain is 51840 B and the UAC fill chain 3040 B.
- **Device** (`s24-vbr-seek-*`): two MP3 tracks, one on each deck, both
  indexed within 3 s, with 8 hot cue and CUE seeks.
  - Worst seek: 17 ms on deck 1 and 16 ms on deck 2 (`prime_max_us`).
  - Underruns 0/0, `packet_errors=0`.
  - Longest fill: 2781 µs.
  - **Owner verdict:** "Sve OK".
  - The board does not log whether a track is untagged VBR. This run is
    therefore a regression check of the seek path; the untagged case itself
    is proven on the host.
