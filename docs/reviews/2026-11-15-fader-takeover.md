# JC1060: soft takeover after the Web Remote, and the FLX4 at boot

Date: 2026-11-15. Branch: `ogranak/jc1060-fader-takeover`, from `main` at `59b0f2c`
(after [Wi-Fi in the deck image](2026-11-15-wifi-in-the-deck-image.md)).

## Soft takeover

**Problem:** a fader moved on the phone left the FLX4's fader where it was. The next
touch of the physical fader jumped the level to wherever that fader sat.

**Change:** `Mix` now keeps a `web_remote::Takeover`, and events reach it two ways.

- **From the Web Remote:** these go through `Mix::web_control`. A fader's value there
  becomes a target. This covers channel volume, crossfader, Tempo, EQ low, mid and high,
  filter, master volume and headphone mix.
- **From the controller:** these go through `Mix::controller_control`.
  - Events for a held fader do not count until they come within 2 % of the target
    (328 of 16383), or pass it from one move to the next.
  - From then on the physical fader is in charge again.
  - Every other event counts as before.
- **Limits:** the hold is per deck and control, at most 16 at once, and only in images
  with the Web Remote.

**Review fix (Codex).** A new value from the page cleared where the controller's fader
last sat. So a first move that passed the target without coming near it was missed. In
the example, a fader at 6000, the page set to 8000, and a move to 12000 stayed blocked.
The takeover now keeps each fader's last physical value whether or not it is held, and a
new value from the page leaves it alone.

## The FLX4 at boot

Every capture from s95 to s110 showed the FLX4 enumerate, stream for about 160 ms, drop
and enumerate again. Those captures all start the board with a reset of the P4 alone,
by flash or by the USB Serial/JTAG reset (`reset_reason=11`). The FLX4 keeps its power
through that reset.

**A cold boot** (s110 again, recorded over UART0 on COM8) gives `reset_reason=1`.
- The FLX4 enumerated once and stayed.
- 27 seconds of clean audio followed.

**So the drop belongs to resetting the P4 while the FLX4 keeps running.** A real
power-up does not have it, and nothing is changed for it. A deck that restarted on its
own with the FLX4 powered would be ready about a second later.

## Evidence

- **Host:** a new test.
  - With nothing held, the physical fader counts.
  - After the page sets 4000, moves at 11800 and 9000 do not count, and 4200 takes over.
  - After the page sets 8000, a jump from 2000 to 12000 passes it and takes over.
  - A fader at 6000 whose first move after the page set 8000 lands on 12000 takes over.
  - The other deck's fader is never held.

  The legacy CI matrix, the Slint checks, the Web Remote tests and target clippy pass.
- **Device:** s111 (app `7b73bd5f…`), owner verdict "radi sve ok".
  - Volume was set low on the phone. The FLX4's fader, still high, did not change it.
    Brought down to that level, the fader took over.
  - 97 requests, the longest 36 ms. Underruns 0/0, `packet_errors=0`, longest fill
    6.0 ms.
