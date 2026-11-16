# JC1060: SYNC with Master Tempo lines up on the tempo, not on a grain

Date: 2026-11-16. Branch: `ogranak/jc1060-sync-mt-phase`, from `main` at `fa9c83b`
(after [the deck's name](2026-11-16-deck-name.md)).

## Problem

SYNC against, or from, a deck in Master Tempo checked within −12 to +7 ms a second
later (s86), against ±1 ms without Master Tempo
([Master Tempo stops](2026-11-13-mt-phase.md)).

The cause:
- **The position was a grain's, not the tempo's.** The deck's position behind the
  keylock was where the grain fading in reads. The grain search moves each grain's start
  up to `SEEK` frames (256, about 6 ms) either way from where the tempo puts it, grain by
  grain.
- **That move stayed in the phase.** SYNC's jump took one grain's move with it, and so
  did the check a second later.
- A host test at +8 % measured the grain's place up to 300 frames (6.8 ms) off the
  tempo's line.

## Change

- **`Keylock::heard_lag`.** The keylock keeps where the grain fading in would have
  started at exactly the tempo, and how fast that line moves through the hop. The
  deck's position (`Reader::set_lag`) follows that line. The beats a listener hears
  are spread around it, grain by grain.
- **`Keylock::lag`** (the frames taken but not played) still steps the reader back when
  a scratch takes the deck or Master Tempo goes off. The deck goes on from the very
  frames it played; its position moves by at most the grain's move then.
- **A wait before the jump.** At exactly the track's own tempo the stretch plays its
  input as it is, and keeps its last grain's move from then on. A SYNC that lands a
  Master Tempo deck there (two tracks of one BPM) must see that move. While either deck
  is in Master Tempo, the phase jump waits 12 render blocks (about 32 ms, two hops), so
  that the stretch plays at the new tempo first.

## Evidence

- **Host:**
  - New keylock test: at tempos 0.92, 1.08 and 1.5 the position stays within half a
    frame of the tempo's line. The grain's place wanders 258, 300 and 512 frames.
  - The legacy engine test, SYNC against and from a Master Tempo deck at +8 % on two
    120 BPM tracks: 0 ms and 0 ms. Without the wait it was 5 ms from the Master Tempo
    deck.
  - Master Tempo going off still moves the deck 108 ms in 100 ms at +8 %, now within
    10 ms instead of 4. The position moves from the line to the grain's place; the
    audio skips nothing. The old fault was 135 ms.
  - The legacy CI matrix, the `idf-web-remote` tests (166), target clippy and fmt pass.
- **Device:** s115 (app `30a25375…`), owner verdict "Gotovo, radi".
  - **SYNC:** three checks a second after SYNC read 0, −1 and +1 ms.
  - **Health:** underruns 0/0 and `packet_errors=0`. The longest fill was 6.9 ms, within
    the 7.0 ms accepted for Master Tempo with a scratch. The USB task had 2896 bytes of
    stack free.
  - **The first boot after the flash failed.**
    - The SD card did not take its clock (`failed to set clk`, `ESP_ERR_TIMEOUT`), as on
      s85.
    - ESP-Hosted then asserted in `sdmmc_host_get_slot_width`: the C6 shares the SDMMC
      host.
    - COM3 then timed out on writes. A cold power-up brought the board up normally.

## Test plan

1. A deck in Master Tempo with its fader moved, SYNC on it: the beats stay together.
2. A deck in Master Tempo at about +8 %, SYNC on the other: the beats stay together.
3. Two tracks of one BPM, Master Tempo on, SYNC: the beats stay together.
4. Master Tempo off while playing: no jump.

## Open

- **An SD failure at boot takes the Wi-Fi down with it.** The SD card can fail to take
  its clock on the first boot after a flash. With Wi-Fi, ESP-Hosted's SDIO bring-up then
  asserts on the shared SDMMC host. It should skip the Wi-Fi, or bring up the host
  itself, when the SD card failed.
- **The once-seen 211 ms phase error (s84)** is not explained by this either.
