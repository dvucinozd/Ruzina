# JC1060: controller LED feedback

Date: 2026-11-06. Branch: `ogranak/jc1060-leds`, from `main` at `2518872`.

## Problem

The legacy firmware drove only the Play, Cue, Loop, Beat FX On and Hot Cue pad LEDs.
Pad mode buttons, Sync, PFL, Master Cue and the Smart buttons stayed dark, and the
Sampler button blinked by itself.

The Beat FX On LED also never lit. It was sent as a global LED in the FX change, but
the FLX4 has one per deck, so the message mapped to nothing.

## Change

- **LEDs:** the legacy firmware now derives them as the native firmware does, in
  `ControllerLedFrame`. It sends what changed, and everything on a new binding. The
  deck engine sets the deck LEDs, and the mix sets PFL and the system buttons.
- **Sampler:** the deck state has no Sampler mode, so the firmware now sends its LED
  off. The owner reported the test as working, so this seems to end the blinking.
  Why it blinked was not established. The guess is that the controller does it while
  no host has set that LED.

- **Lamp test (Codex review):** the startup lamp test clears every output 1.5 s after
  binding, which also cleared the first LED state, and the frame saw nothing to resend.
  The active pad mode and Master Cue stayed dark until they changed. The LED sync now
  waits for the lamp test, and its end sends the whole state again.

## Evidence

- **Host:** 1 new test, which also checks that every LED sent maps to an FLX4 message.
  All 8 legacy feature sets and target clippy on 6 of them pass.
- **Build:** production `deck-engine`, app `b3a1acfd…`, 873392 B. Load chain 51712 B,
  UAC fill chain 3344 B.
- **Device** (`s38-leds-*`), owner verdict "Radi":
  - **In the log:** Hot Cue, Beat Jump and Pad FX 1 mode presses and PFL on both decks,
    Sync on deck 1, Smart CFX, Beat FX On and the channel switch.
  - **Log:** no track load, underruns 0/0, `packet_errors=0`, no faults.

- **Device, with the lamp test fix** (`s39-lamp-leds-*`, app `35872eb5…`, 873616 B): with
  no button pressed, the owner saw Hot Cue on both decks and Master Cue lit after the
  lamp test. The other LEDs from the s38 run were not repeated on this image.

## Not covered

- **LEDs themselves:** the log shows the presses, not the lights, so these rest on the
  owner's verdict.
- **Not in the log:** Master Cue, Smart Fader, Sync on deck 2, the other pad modes and
  a press of the Sampler button.
- **With tracks playing:** no track was loaded in this run, so Sync following a real
  tempo, the Play LED and the LEDs of a loaded track were not exercised here.
- **Beat Jump and Beat Loop pads:** their pads have no LED logic, as on the native
  firmware.
