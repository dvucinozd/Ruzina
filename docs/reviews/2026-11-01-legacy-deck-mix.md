# JC1060 `deck-mix`: both decks through the product mixer

Date: 2026-11-01. Branch: from main `bd52402` (after PR #70). The device
run, with the owner present, is pending. Native minimum 302 and APTA unchanged.

## What it adds

`deck-mix` is the [`deck-play`](2026-11-01-legacy-deck-play.md) profile with
both decks playing and mixed by the product's own render graph and mixer
state, as on the native firmware.

- **Two decks, `deck_play.rs` and `idf_media_bridge.rs`:**
  - The ring and the command slot are now one per deck.
  - Deck 2's producer and ring are allocated at its first Load.
  - The SD load task polls both producers each turn, alternating which one
    goes first.
  - Load and Cue work on both decks.
  - Play events carry the deck in bits 24 and up, and the screen note names
    `D1` or `D2`.
- **Render, `deck_mix.rs` (feature `idf-deck-mix`):**
  - On the USB client task, each packet takes its frames from 128-frame blocks
    of `ruzina-audio-runtime`'s `FixedRateRenderGraph::render_block`, fed from
    both rings.
  - The graph resamples each deck to 48 kHz, applying the Tempo fader as pitch
    (±10 %, no keylock). It then applies trim, 3-band EQ, the channel filter,
    the channel faders, the crossfader, master gain and the MAIN limiter.
  - It also gives the headphone mix of the pre-fader PFL taps and MAIN.
  - The `RenderControl` is built as the native `audio_render_owner` builds it
    from `MixerState`. The one difference is that the source epoch and
    generation are fixed, because this graph has no FX rack.
- **Controls:**
  - Every controller event also goes to `ruzina-mixer`'s
    `MixerState::handle_control`. This covers the channel faders, crossfader,
    trim, EQ, filter, the PFL (headphone CUE) buttons, Headphone Mix,
    Master Cue, Master Volume and Headphone Level.
  - The Tempo faders set each deck's pitch through
    `ruzina_deck::tempo_centipercent_from_raw`.
- **Outputs:**
  - MAIN goes to FLX4 channels 1/2, and the headphone mix to channels 3/4.
  - `controller_audio` now packs both pairs. MAIN stays silent in the tone and
    single-deck profiles.
- **Two diagnostic departures from the product, for safety:**
  - Master Volume and Headphone Level start at a quarter (−12 dB) until each
    knob first moves. The product starts both at full scale.
  - The headphone output keeps a fixed −12 dB ceiling.
- **Telemetry, `usb_host_enum.c`:**
  - `deck_mix_stats` reports rendered frames, underrun blocks per deck,
    buffered frames and rate per deck, and limited MAIN samples.
  - It also reports `fill_max_us`, the longest Rust fill of one transfer (six
    1 ms packets).
- **Stack:**
  - The build now also walks the call chain under `ruzina_legacy_uac_fill`,
    where the render graph runs on the 8 KiB USB client task. That chain is
    3120 bytes against a limit of 4 KiB, written to `uac-fill-stack.log`.
  - The estimate is an upper bound: it includes the FX path, which this graph
    never takes.
  - The load run is now 72912 bytes deep, within the 88 KiB limit.
- **Configuration and tools:**
  - `CONFIG_RUZINA_DECK_MIX` requires deck playback. The guard test gains 2
    cases.
  - New defaults `sdkconfig.deck-mix` and the `deck-mix` variant, with
    mismatch and link checks.
  - The 2 KiB USB-task frame limit now also covers `ruzina_legacy_mix*`.
  - The variant is added to the flash-plan list and the PowerShell wrapper.
  - CI adds `idf-deck-mix` to the test, clippy and target loops, and the
    variant to the matrix.

## Evidence (local)

- **Rust:** 50 host tests pass with `idf-deck-mix`. The new test drives two
  constant sources through the graph:
  - silence before a source is ready;
  - the start levels: MAIN at a quarter, and the headphones at a quarter of
    that under the ceiling;
  - the crossfader centred sums the decks, and each end keeps only its own
    deck;
  - with deck 1's channel fader down and its PFL on and Headphone Mix on CUE,
    MAIN carries deck 2 only while the headphones carry deck 1 pre-fader;
  - full Master Volume gives deck 2 at unity on MAIN;
  - a Tempo fader at +10 % takes 10 % more source frames;
  - paused decks take nothing;
  - a deck that runs dry counts underruns, but not at the end of its track;
  - a device not at 48 kHz gets silence.

  The MIDI test checks deck 2's Cue request.
- **Other checks:**
  - The other feature sets still pass: 49 tests with `idf-deck-play`, 41 with
    `idf-media-load idf-controller-audio` and 2 with `idf-controller-audio`.
  - Host clippy passes, and target clippy passes for every feature set.
  - 26 tooling tests pass.
- **Build:** `build_legacy.py --variant deck-mix` returns `BUILD_PACKAGE_PASS`
  with no warnings. The app is 789536 bytes, 75 % of the factory partition.

## Device acceptance (needs the owner)

Safety:
- Hold the headphones away from the ears first. Both levels start at a
  quarter.
- MAIN now carries audio on the FLX4's MASTER output. With speakers or an
  amplifier connected there, turn them down before the first Play.

Accept the run when:
- **Load and play.** A track loads on each deck: both notes show (`D1 …`,
  `D2 …`) and both decks play together.
- **Crossfader.** It moves MAIN between the decks.
- **Channel controls.** The channel faders, trim, EQ and the Tempo faders act
  on their deck.
- **Headphones.**
  - A deck's headphone CUE (PFL) button puts it in the headphones.
  - Headphone Mix blends between CUE and MASTER.
  - Headphone Level changes the volume.
- **MAIN.** It is checked on the MASTER output, or through the headphones with
  Headphone Mix on MASTER.
- **Telemetry.**
  - `deck_mix_stats` shows `underruns=0/0` while both play.
  - `fill_max_us` stays well below the 6 ms that one transfer covers.
  - `usb_uac_stats` shows no isochronous errors.
- **Everything else.** The screen, browse, detail, LEDs and Cue still work.
