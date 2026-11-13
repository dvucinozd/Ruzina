# JC1060: continuous SYNC, and the keylock's stack fix

Date: 2026-11-13. Branch: `ogranak/jc1060-continuous-sync`, from the
[Master Tempo](2026-11-13-master-tempo.md) branch (merged as `4d3e5b5`).

## Problem

- **SYNC matched only once.** A SYNC press matched a deck's tempo and phase to the other
  deck at that moment. When the other deck's tempo moved later, the synced deck stayed
  where it was, although its SYNC stayed lit.
- **The USB task's stack ran low.** The captures showed its free stack falling with
  Master Tempo:
  - 2832 bytes on s80;
  - 1048 bytes on s83;
  - 872 bytes on s84.

## Change

- **Continuous SYNC** (`ruzina-deck`):
  - When a deck's tempo moves, by its Tempo fader or a new tempo range, the other deck
    follows at once if its SYNC is on. It gets the pitch that matches its BPM to the
    moved deck's, within the beat sync range.
  - The follower keeps following until its own fader or tempo range moves (which turns
    its SYNC off, as before), or SYNC is pressed again.
  - Only a deck's tempo is followed. A jog bend on the reference is not.
- **Stack:**
  - `Keylock::boxed_pair` built each keylock's 2 KiB raised-cosine table on the stack,
    and copied it in. That made a 4192-byte frame on the USB task, once, when the
    keylocks are allocated.
  - The table is now written in place: the frame is 96 bytes.

## Evidence

- **Host:**
  - New `ruzina-deck` test: the follower tracks the reference's fader and its tempo
    range, and stops following once its own fader moves.
  - New legacy engine test: after a SYNC, deck 1's fader at +10 % moves deck 2 to
    +10 % at once. Over the next 500 ms both advance 550 ms, within 2 ms of each other.
  - The legacy CI matrix (tests and clippy for every feature set), target clippy and
    the shared deck tests pass.
- **Device:**
  - s84 (app `e54034a4…`): owner verdict "Radi" on all five points, including with
    Master Tempo.
    - In 15 minutes: underruns 0/0, `packet_errors=0`, longest fill 6.2 ms.
    - The phase checks a second after each SYNC were mostly within ±1 ms, with three
      exceptions:
      - −29 and −30 ms after the soak DJ's SYNC, most likely against a deck in Master
        Tempo. s83, with Master Tempo, already showed up to ±10 ms: the keylock's lag
        on the heard position is an estimate.
      - 211 ms once, during the owner's test, in the second after a SYNC on deck 2. The
        log does not show the cause. A jog bend on deck 1 in that second would do it.
  - s85 (app `319a4618…`), the stack fix: the USB task's free stack is back to 2832
    bytes, as on s80.
    - The first boot after the flash failed to bring up the SD card ("failed to set
      clk", `ESP_ERR_TIMEOUT`, `APP_RESTART_REQUIRED`), so no tracks and no audio.
    - An app restart brought it up normally. The change does not touch the SD path.

## Test plan

1. Both decks playing, SYNC on deck 2.
2. Move deck 1's Tempo fader: deck 2 follows at once, SYNC stays lit, the beats stay
   together.
3. Change deck 1's tempo range: deck 2 still follows.
4. Move deck 2's fader: its SYNC goes off, and it no longer follows.
5. The same with Master Tempo on: the key stays while the tempo follows.

## Open

- SYNC's phase against a deck in Master Tempo is off by up to about 30 ms: the keylock's
  lag estimate should be measured and tightened.
