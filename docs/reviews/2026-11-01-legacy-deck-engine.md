# JC1060 `deck-engine`: the product deck engine owns transport

Date: 2026-11-01. Branch: `ogranak/jc1060-deck-engine`, on top of
[`deck-mix`](2026-11-01-legacy-deck-mix.md) (PR #71, open, device run
pending). Not yet on the device. Native minimum 302 and APTA unchanged.

## What it adds

`deck-engine` is `deck-mix` with the diagnostic Play latch and the "Cue goes
to the start" shortcut replaced. In their place runs the product deck engine,
`ruzina-deck`'s `DeckProductState`, which the native firmware runs.

- **Engine, `deck_engine.rs`** (feature `idf-deck-engine`):
  - Every controller event reaches `DeckProductState::handle_control` on the
    USB client task.
  - The module is the transport owner for the engine's effects, as the native
    `transport_owner_state` is:
    - `PlaybackRequest` is accepted only for a loaded deck and resolved at
      once with `resolve_playback_request`, and `Pause` stops rendering;
    - `Seek` goes to the deck's producer (below);
    - `SetPitchCentipercent` is the render pitch;
    - `JogNudge` feeds the product's `TempoNudge`: 0.02 per tick, ±0.30,
      decaying as in M2.5.
  - Scratch, loop, pad and beat FX effects are counted as `unsupported` and
    not executed yet. The pads in Hot Cue mode still set and recall local hot
    cues, through the engine's own seeks.
- **CUE, as the engine defines it (CDJ, M2.5 B12):**
  - While playing, CUE returns to the cue point and pauses.
  - Stopped away from the cue point, CUE stores a new cue point there.
  - Stopped at the cue point, CUE previews while held and returns on release.
  - The jog scrubs while paused (a seek) and bends while playing.
- **Positions:**
  - Each render block reports each deck's source position with
    `set_position_ms`, so CUE works on the real position. That position is
    the epoch's origin frame plus the frames taken since.
  - A seek holds these reports until its new epoch arrives, so a stale
    position never overwrites the target.
  - A deck whose track has played out stops with `finish_playback`.
- **Seek, `deck_play.rs`:**
  - `Commands::seek` carries the target, and a newer seek replaces an older
    one.
  - The producer calls `SourceDecoder::change(StreamChange::SeekMilliseconds)`,
    decodes the first block and opens a new ring epoch there. The epoch
    records its origin frame (`Ring::restart_at`).
  - The old epoch plays on until then, and a full ring holds the first block
    back as on a load.
- **Loads:**
  - The SD load task publishes each deck's ready or failed load (catalog
    index, BPM, track id) through a sequence-locked mailbox, `LOADED`.
  - The engine commits a ready load with `commit_track_load`, with an empty
    source hot cue bank, and unloads a failed one.
  - The controller's Load is passed to the display only when the product's
    load admission allows it, so it is refused while that deck plays.
  - Load event 11 reports seeks.
- **LEDs:** the Play LEDs follow the engine's confirmed playback. The C
  client loop calls `ruzina_legacy_midi_refresh` every 50 ms, so a track
  that plays out turns its LED off without a controller event.
- **Screen:** the header shows each deck's state and position, `> m:ss`
  playing or `| m:ss` paused, from `ruzina_legacy_engine_status`. The values
  are published atomically for the display task, and the header redraws once
  a second while a deck plays.
- **Telemetry:** `deck_engine_stats` reports each deck's position and flags,
  unexecuted effects and refused loads.
- **Configuration and tools:**
  - `CONFIG_RUZINA_DECK_ENGINE` requires the deck mix. The guard test gains 2
    cases.
  - New defaults `sdkconfig.deck-engine` and the `deck-engine` variant, with
    mismatch and link checks.
  - The 2 KiB USB-task frame limit covers `ruzina_legacy_engine*`.
  - The variant is added to the flash-plan list and the PowerShell wrapper.
  - CI adds `idf-deck-engine` to the test, clippy and target loops, and the
    variant to the matrix.
  - The tests that use the process-wide rings, commands and controller
    session now share a lock.

## Evidence (local)

- **Rust:** 52 host tests pass with `idf-deck-engine`.
  - **Engine test** (two-deck `Mix` with a simulated producer):
    - Play on an empty deck is refused.
    - A published load commits with its BPM.
    - Play is audible and the position follows the source to within 3 ms.
      Load is refused while playing.
    - CUE while playing pauses and seeks to 0, and the stale position is held
      until the new epoch.
    - The CUE preview plays while held and returns on release.
    - Pausing away from the cue and pressing CUE stores the cue there without
      a seek.
    - The jog scrubs when paused, and a bend of 5 ticks gives +10 % that
      decays back to 0.
    - The Tempo fader at its end gives +10 % through the engine, measured in
      position.
    - A Hot Cue pad stores the current position in the local bank, and its
      next press seeks back there.
    - The end of a track stops the deck.
    - The display status carries the position and flags.
    - A failed load unloads the deck.
  - **Mailbox test:** a post is read whole, once, and never mid-write.
  - **Producer seek test** (real fixtures):
    - WAV is frame-exact at 100 ms, including the position after draining.
    - A seek after the end revives the track.
    - MP3 behind an ID3 tag seeks to 6.0 s, 1.0 s and 7.9 s, and each epoch
      starts at its target.
    - The newer of two seeks wins, and a deck without a track ignores a seek.
  - **MIDI test:** under the engine, Play on empty decks leaves the LEDs off.
- **Other checks:**
  - The other feature sets still pass: 51 tests with `idf-deck-mix` and 50
    with `idf-deck-play`, both now including the seek test.
  - Host clippy passes, and target clippy passes for every feature set.
  - 26 tooling tests pass.
- **Build:** `build_legacy.py --variant deck-engine` returns
  `BUILD_PACKAGE_PASS` with no warnings.
  - The app is 807776 bytes, 77 % of the factory partition.
  - The load run chain is 73008 bytes (limit 88 KiB), and the UAC fill chain
    3104 bytes (limit 4 KiB).
  - The MIDI ingest chain through the engine is about 2.3 KiB, not counting
    the assert-abort path.

## Device acceptance (needs the owner)

Safety is as for `deck-mix`: both levels start at a quarter, and the headphones
should be held away from the ears first.

Accept the run when:
- **Load and play.** Load works on both decks. Load is refused on a playing
  deck, and Play on an empty deck does nothing.
- **CUE.**
  - While playing, CUE returns to the cue point and pauses.
  - Paused elsewhere, CUE sets a new cue point.
  - At the cue point, holding CUE previews and releasing returns.
- **Jog.** Paused, the jog scrubs; playing, it bends and springs back.
- **Pads.** In Hot Cue mode a pad sets a hot cue, and pressing it again jumps
  there.
- **Screen.** The header shows `>`/`|` and the time for each deck, and the
  Play LEDs follow.
- **Telemetry.** `deck_mix_stats` shows no underruns and `usb_uac_stats` no
  isochronous errors. `deck_engine_stats` counts any unsupported effects that
  were tried.
