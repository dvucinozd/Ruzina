# JC1060: Master Tempo

Date: 2026-11-13. Branch: `ogranak/jc1060-master-tempo`, from `main` at `7d9d8dd` (after
[Beat FX by touch](2026-11-12-fx-touch.md)).

## Problem

The deck screen showed an MT box, but nothing could switch Master Tempo on:
- No control set the deck state's `master_tempo`.
- The legacy render graph refuses Master Tempo: its pop-only path renders silence for it.

So the tempo fader always moved the key with the tempo.

## Change

- **Switch:**
  - `DeckExtAction::MasterTempo` (in `ruzina-controller-core`) toggles a deck's Master
    Tempo in `ruzina-deck` on press. No controller profile maps it yet; there is a new
    test for it.
  - A tap on a deck's MT box (with a few pixels of slack around it) sends it, through
    the touch queue as the other screen taps do. MT lights while it is on.
- **JC1060 keylock (`deck_keylock.rs`):** a WSOLA time stretch at the track's own rate.
  - Grains are 1024 frames (about 23 ms) and overlap by half, joined with a raised
    cosine.
  - Each grain's start moves up to 256 frames (about 6 ms) either way, to where its
    frames best match the grain it takes over from:
    - a coarse search on every 4th frame, by normalised cross-correlation;
    - then frame by frame around the best match.
  - The search runs during the hop before the grain starts, one candidate an output
    frame, so the fill's cost stays even:
    - the coarse search tries every 8th start, on a mono copy of every 4th frame kept as
      frames arrive;
    - the fine search tries ±3 frames around the best.
  - A search the deck cannot feed yet (just after a seek) leaves the grain where the
    tempo puts it.
  - At the track's own tempo the output is the input, frame for frame.
- **Why not the shared keylock:** `ruzina-audio-keylock` was tried first (s81) and kept
  the key, but sounded distorted on music. It is the donor's low-budget design: 5 ms
  grains, a ±1 ms search and a match on 2 samples, kept that way by its parity tests.
  It stays as it is.
- **In the mix:**
  - The keylock reads the deck's frames in play order, so loops, phase jumps and the
    history replay reach it unchanged.
  - The render graph then only resamples: pitch 0 at the track's rate.
  - A scratch bypasses the keylock, and a new source (load, seek) restarts it.
  - The frames the keylock has taken but not yet played are left out of the deck's
    position, so SYNC and the display follow what is heard.
- **Memory and placement:**
  - Both keylocks (about 36 KiB each) are allocated once, zeroed on the heap, outside
    the fill.
  - The fill placement moves `deck_keylock` into IRAM with the rest of the fill path.
- **Profile:** fill profile flag 128 marks a deck with Master Tempo on.

## Evidence

- **Host:**
  - The keylock's own tests:
    - unity tempo is sample-exact;
    - a sine keeps its pitch at tempo 0.8 to 1.6 while the track advances at the tempo;
    - a 120 Hz harmonic tone stays periodic across grain joins (residual under 0.5 % at
      0.87, 1.13 and 1.4);
    - a starved keylock waits and loses no frame;
    - the lead stays within two grains.
  - Also: the MT touch round-trip, and the engine test that a tap toggles each deck.

  The legacy feature sets and target clippy (including `idf-slint-ui` and the profile
  build) pass, as do the shared `ruzina-deck` tests.
- **Device:**
  - s81 (shared keylock, app `5a980fc3…`): owner heard distortion.
    - In 15 minutes, 157/9 underrun blocks, most likely after the soak DJ's jumps: the
      shared keylock needed frames ahead that a fresh seek did not yet have.
    - Longest fill 7.3 ms.
  - s82 (first version of this keylock, app `1d223d1d…`, fill profile): owner verdict
    "Radi", but costly.
    - In 15 minutes: underruns 0/0, `packet_errors=0`, no panics.
    - Longest fill 8.7 ms; 54 % of fills over 3 ms (about 9 % without Master Tempo);
      6095 fills of 6 ms or more.
    - The whole grain search fell into one render block every 512 frames per deck.
  - s83 (the search spread over the hop, app `b77f945a…`, fill profile): owner verdict
    "Radi".
    - In 15 minutes: underruns 0/0, `packet_errors=0`, no panics.
    - Longest fill 6.7 ms: deck 1 playing in Master Tempo while deck 2 scratched, also
      in Master Tempo. With Master Tempo alone, and Beat FX, fills peaked at about
      6.2 ms.
    - 44 % of fills over 3 ms (Master Tempo was on for most of the run), and 18 fills
      of 6 ms or more.
  - On the host, the stretch costs 18 ns a frame off the track's tempo (59 ns before
    the change) and 8 ns at it (15 ns).
  - Master Tempo with a scratch on the other deck runs 0.4 ms over the accepted
    ~6.3 ms scratch exception. It also lifts the share of fills over 3 ms, from about
    9 % to over 40 %.

## Test plan

1. Play a track, tap MT and move the tempo: the key stays, without a metallic or
   rough sound.
2. Larger tempo moves (±8 % and more) still sound clean.
3. MT off: the tempo moves the key again.
4. MT on both decks, then SYNC: the decks line up.
5. A scratch with MT on, then play continues.
