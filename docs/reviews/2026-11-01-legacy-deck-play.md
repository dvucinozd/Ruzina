# JC1060 `deck-play`: deck 1 plays card tracks to the FLX4 headphones

Date: 2026-11-01. Branch: from main `1fe34ce` (after PR #69). The device
run, with the owner present, is pending. Native minimum 302 and APTA unchanged.

## What it adds

`deck-play` is the [`deck-audio`](2026-11-01-legacy-deck-audio.md) profile
with the diagnostic tone replaced by deck 1's track. It is decoded from the
card with the shared `ruzina-audio-runtime` decoder.

- **Rust, `deck_play.rs`** (feature `idf-deck-play`, which adds
  `ruzina-audio-runtime`):
  - **Producer, on the SD load task.**
    - On a Load, the task opens the track's `audio_path` on the exFAT volume.
    - It chooses the codec by its magic bytes: `RIFF` gives WAV, `fLaC` gives
      FLAC, and `ID3` or an MPEG sync gives MP3. For WAV it walks the RIFF
      chunks to `fmt ` and `data`.
    - It runs `SourceDecoder`, answering each 512-byte read through the
      volume.
    - It pushes each 256-frame block, at the source rate, into a PSRAM ring of
      65536 frames (about 1.5 s at 44.1 kHz).
    - The decoder state lives in the PSRAM heap: `DecoderStorage`, three MP3
      checkpoints and the 96 KiB encoded window.
  - **Ring.**
    - The ring is single-producer, single-consumer and lock-free.
    - A Load or Cue opens a new epoch: the producer records where the new
      frames start and the rate, and the consumer jumps there when it sees
      the new epoch. Old frames are never played and the producer never waits
      for them to drain.
    - When the consumer still holds the old epoch, the new track's first block
      waits in `pending`.
  - **Consumer, on the USB client task.**
    - `controller_audio` now fills packets from a PCM source.
    - With `deck-play`, that source pulls each packet through the product's
      `ResamplerState` from the source rate to the device's 48 kHz, so the
      44.1 kHz tracks play at the right speed.
    - Deck 1 plays on both headphone channels (3/4) while its Play latch is
      on. MAIN stays silent.
    - When the ring runs dry while playing, it outputs silence and counts one
      underrun per packet. The end of a track does not count as an underrun.
  - **Gain.**
    - The Headphone Level knob sets the gain on an audio taper, up to
      −12 dBFS at full turn.
    - Until the knob first moves, the gain is a quarter turn: −36 dBFS.
  - **Controls.**
    - Play toggles the latch, as before.
    - Cue pauses deck 1 and returns its track to the start.
    - Load on a playing deck pauses it.
- **Load task, `idf_media_bridge.rs`.**
  - With deck playback, the first pass is the only one. After it, the task
    serves detail selections and the producer (`serve_decks`) instead of
    re-reading the card.
  - Events 7–10 report ready (codec), the rate, a load failure (stage) and the
    end of a track.
- **C.**
  - `load_display.c` passes deck 1's Load to `ruzina_legacy_play_load`.
  - `sd_bridge.c` turns the play events into the screen's status note (for
    example `D1 MP3 44100 Hz - press Play`) and logs the internal heap at
    start.
  - `usb_host_enum.c` adds `deck_play_stats` to the UAC statistics.
- **SD client stack: 96 KiB, up from 48.**
  - nanomp3 builds its 24 KiB `Mp3Dec` by value, and the runtime's reset,
    checkpoint and seek paths copy similar temporaries.
  - The worst direct-call chain under `ruzina_legacy_load_run` is 71664 bytes:
    the load run, `serve_decks`, `poll`, `decode`, `step`, the MP3 seek and
    `Mp3Dec::new`.
  - The new `deepest_stack` guard in `stack_frames.py` computes that chain at
    build, writes `load-run-stack.log` and rejects more than 88 KiB.
  - The per-frame guard caught an earlier draft that built `DecoderStorage` as
    a 175 KiB stack value. The heap slots are now filled by copying
    compile-time statics.
- **Configuration and tools.**
  - The new option `CONFIG_RUZINA_DECK_PLAY` requires UAC and the deck UI.
    The guard test gains 3 cases.
  - New defaults `sdkconfig.deck-play` and the `deck-play` variant, with
    mismatch and link checks. The 2 KiB USB-task frame limit now also covers
    `ruzina_legacy_play*`.
  - The variant is added to the flash-plan list and the PowerShell wrapper.
  - CI adds `idf-deck-play` to the test, clippy and target loops, and the
    variant to the matrix.

## Evidence (local)

- **Rust: 49 host tests pass with `idf-deck-play`.** The new tests use the
  shared runtime's synthetic CC0 codec fixtures.
  - The WAV and FLAC tracks reach the ring bit-exact against the WAV PCM.
  - The 48 kHz and 44.1 kHz MP3 tracks decode 12000 frames at their own rate.
  - An 8-second MP3 behind an ID3v2 tag fills the ring, then refills it as the
    consumer drains. It ends once, within 2000 frames of 8 × 44100.
  - Cue replays the same first 1000 frames. Load onto a full ring waits, then
    delivers the new track whole.
  - Each failure stage is reported (path, open, format) and silences the deck.
    A read failure mid-track ends it as faulted.
  - Output: silent before a source is ready and while paused. 0.2 s at 48 kHz
    consumes 8820 ± 2 source frames at 44.1 kHz. Running out at the end of a
    track is not an underrun; starving mid-track is.
  - Gain values, and the ring's attach rules.
  - The MIDI test checks that Cue pauses deck 1 and requests the restart, that
    Load pauses a playing deck, and that the Headphone Level reaches the
    gain.
- **Other checks.**
  - The other feature sets still pass: 2 tests with `idf-controller-audio`,
    41 with `idf-media-load idf-controller-audio`.
  - Host clippy passes, and target clippy passes for every feature set.
  - 26 tooling tests pass.
- **Build:** `build_legacy.py --variant deck-play` returns
  `BUILD_PACKAGE_PASS` with no warnings.
  - The app is 768176 bytes, 73% of the 1 MiB factory partition. The decoders
    and their initial states account for the growth from 508976.
  - `load-run-stack.log` gives 71664 bytes.

## Device acceptance (needs the owner)

Safety:
- Hold the headphones away from the ears first.
- The start gain is −36 dBFS. Turn Headphone Level down before the first Play
  and raise it slowly; full turn is −12 dBFS, louder than the −24 dBFS tone.

Accept the run when:
- `S6_RUST_LOAD … internal_free=` is logged, and the load pass completes
  25/25 with the 96 KiB client task running.
- **Load and playback.**
  - Loading a track on deck 1 logs events 7 and 8 and shows
    `D1 <codec> <rate> Hz - press Play`.
  - Play gives the track at the right pitch and speed in both headphone ears.
    Play again pauses it, and Cue returns to the start.
  - At least one MP3 at 44.1 kHz, one at 48 kHz, the FLAC and the WAV play.
- **Telemetry.**
  - `deck_play_stats` shows `underruns=0` while playing and a `buffered`
    count that stays well above zero.
  - `usb_uac_stats` shows no isochronous errors.
  - The load events' `stack_free` stays above 8 KiB.
- **Headphone Level** changes the volume.
- The screen, browse, detail and LEDs still work.
