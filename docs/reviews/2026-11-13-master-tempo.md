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
  - s82 (this keylock, app `1d223d1d…`, fill profile): owner verdict "Radi".
    - In the first minutes: underruns 0/0, `packet_errors=0`, no panics.
    - Longest fill **8.4 ms**, with deck 1 in Master Tempo and deck 2 replaying a loop.
    - **Fills over 3 ms rose to 35 %** (about 9 % before), and 421 fills reached 6 ms,
      against a handful without Master Tempo. The graph stage carries it: the
      per-frame crossfade, plus the grain search once every 512 frames per deck.
  - This is over the accepted ~6.3 ms scratch exception. The ring's cushion has covered
    it so far, but the search should be made cheaper (or spread over the hop) before
    Master Tempo is relied on.

## Test plan

1. Play a track, tap MT and move the tempo: the key stays, without a metallic or
   rough sound.
2. Larger tempo moves (±8 % and more) still sound clean.
3. MT off: the tempo moves the key again.
4. MT on both decks, then SYNC: the decks line up.
5. A scratch with MT on, then play continues.
