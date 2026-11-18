# JC1060: Rekordbox memory cues

Date: 2026-11-17. Branch: `ogranak/jc1060-memory-cues`, from `main` at `f1a4894`.

## Problem

- **Imported, but unused.** The card's analysis import already read Rekordbox's memory
  points and loops (RP-05: PCOB list 0, up to 16 per track, in export order). The board
  only counted them.
- **The donor shows them only.** M2.5 lists memory cues on its HOT CUES page, with no
  recall and no LED.
- **No controller mapping.** The FLX4's SAMPLER button and pads are not in the golden
  profile, so they did nothing.

## Change

### Calling a memory cue

- **`ruzina-deck` gains `recall_memory_cue`.** A call works as a CDJ's CALL does:
  - the deck moves to the cue as a Hot Cue recall does, and its play state stays;
  - a memory loop runs, and a memory point leaves an active loop;
  - CUE then returns to the cue.
- **The engine** gets each track's memory cues with its beat grid (`deck_view::Grid`).
  `Engine::call_memory` calls one. It does nothing for a cue the loaded track does not
  have, and nothing during a scratch.

### The FLX4's SAMPLER pads

- **The board reads the messages itself.** The golden S3CP v2 profile stays as it is.
  `controller_midi::sampler_event` reads the SAMPLER messages before they reach the
  profile. They follow the other pad modes' layout:
  - the button is note 0x22 on deck 1's or deck 2's channel;
  - the pads are notes 0x30 to 0x37 on the pad channels (0x97, 0x99), and with Shift on
    the next channels (0x98, 0x9A).
- **Pads:** in SAMPLER mode, pads 1–8 call memory cues 1–8, and Shift + pad calls 9–16.
- **SAMPLER mode:** the deck state keeps no Sampler mode, as its released scope
  says. The engine keeps it, so the SAMPLER LED lights and the other pad modes' LEDs go
  off.
- **Pad lights:** a SAMPLER pad lights where its memory cue (1 to 8) exists. Those notes
  go out as they are, since the profile has no output map for them.

### The deck screen

- **Lane:**
  - each memory cue is a red line through the lane, and a memory loop's end is too;
  - at its start, a red triangle sits at the Hot Cue flags' edge, pointing into the
    lane;
  - a Hot Cue or a loop end drawn at the same pixel wins.
- **Overview:** red marks at the top and bottom where each memory cue starts.
- **HOT CUES page:**
  - each deck has a MEMORY button, which swaps its pads for a list of its memory cues:
    two columns of eight, with number, time and LOOP;
  - a tap calls one (`TouchControl::Memory`, through the same SAMPLER pad event);
  - with none, the list says NO MEMORY CUES.

### Review fix: packet order

Codex (P2) found that a transfer's SAMPLER events were delivered after all of its
profile events. A memory pad followed by Jog Touch in one Bulk-IN would then be
refused, because the jog already counted as touched. Now `ingest` cuts the transfer
after each SAMPLER packet:
- the session first delivers the profile events before it;
- then comes the SAMPLER event;
- then the rest of the transfer.

A transfer without a SAMPLER packet stays one batch, as before. The session test checks
the order Play, pad, Play, SAMPLER in one transfer.

## Evidence

- **Host:**
  - **`ruzina-deck`:** a memory point moves the deck and its CUE; a memory loop
    runs; a point leaves the loop; an end before the start is a point. All 93 crate tests
    pass.
  - **The engine:**
    - the SAMPLER pads call the loaded track's cues, and not another track's;
    - Shift reaches cue 10, and a missing cue does nothing;
    - the SAMPLER button and the other pad modes switch the mode;
    - the LED test now expects the SAMPLER LED.
  - **Controller:** the SAMPLER button and pads, with and without Shift, and note-offs
    become semantic events; other notes stay the profile's.
  - **Touch:** the touch queue carries `Memory(0..15)`.
  - **Other checks:**
    - the legacy tests and clippy;
    - target clippy for every CI feature set;
    - the desktop simulator's and media catalog's tests.
- **Device:**
  - **s131** (app `2b6a970a…`): the SAMPLER button reached the board
    (`PadModeSampler`, deck 1). The owner did not know which track has memory cues, and
    the log had no pad press, so the test was left for later.
  - **s135** (app `b9b0c6cb…`, 2026-11-18): the branch rebased on `main` (s134) and
    started over COM3 from the held ROM.
    - The SD card came up at the first try.
    - The owner loaded tracks on both decks and pressed SAMPLER.
    - The log has `control=PadAction` for pads 1–4 and 7 on deck 1, and pads 1–2 on
      deck 2, each with its release.
    - The owner: "Radi".
    - All 15 reports had `packet_errors=0` and underruns 0/0.
    - **Not tried:** Shift + pad (cues 9–16).

## Test plan

1. Export tracks with memory cues, and one memory loop. In Rekordbox, MEMORY in the CUE
   and LOOP panel stores one.
2. Load such a track:
   - the lane shows red lines with triangles;
   - the overview shows red marks;
   - HOT CUES → MEMORY lists the cues.
3. A tap in the list calls the cue, and CUE then returns there.
4. SAMPLER on the FLX4:
   - the button lights;
   - the pads light where cues exist;
   - pad 1 and pad 2 call cues 1 and 2, and Shift + pad calls cue 9.
   The log shows `control=PadAction` for each press.
5. A memory loop runs when called.
