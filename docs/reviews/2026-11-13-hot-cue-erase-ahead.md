# JC1060: Hot Cue saves without long erases

Date: 2026-11-13. Branch: `ogranak/hotcue-erase-ahead`, from `main` at `dde8013`.

## Problem

Two open items had one cause:
- Some captures showed one USB audio fill of about 21 ms: s80 had 22.2 ms, and s87
  had 21.6 ms.
- Most captures lost 1 to 5 USB packets in 15 minutes.

Every rise in `packet_errors` on s80, s85, s86 and s87 came right after a Hot Cue journal
save. The long fills came right after saves that took 45 to 50 ms.

- **Why a save stalls the fill:** IDF disables the cache while it programs or erases
  flash. The USB task's fill then waits, because its data lives in PSRAM.
- **The long saves:** these are the ones that erase a 4 KiB sector, which takes tens of
  milliseconds. The ISO queue to the FLX4 holds about 24 ms, so a longer stall loses
  packets.
- **What is heard:** saves already wait until no deck plays, so nothing was heard; the
  lost packets were silence.

## Change

- **`ruzina-hot-cues` journal:**
  - Before erasing a sector, it reads the sector, 256 bytes at a time, and skips the
    erase when the sector already reads erased.
  - A new `erase_spare()` erases, ahead of time, one sector that a later write would
    otherwise have to erase: a bucket's non-active side that still holds data. That data
    is an older generation, or an uncommitted one. A bucket whose two sides are
    inconsistent is left for the writes to report.
- **Board:**
  - The USB audio fill counts its calls.
  - While that count stands still for 200 ms (no controller yet, or it is unplugged), the
    load task erases one spare per turn until none is left. Each such erase is logged as
    event 19.
  - After each save it looks again, because a rotation leaves a new spare.
- **Flash chip:** IDF's flash auto-suspend would avoid the stall entirely. The board's
  Boya flash is not among the chips IDF supports for it, so it stays off.

## Evidence

- **Host:** a new journal test.
  - A fresh bucket's first record, and the first rotation into the blank other side,
    need no erase.
  - `erase_spare` clears the stale side once, and then reports none.
  - The next rotation needs no erase.
  - Without `erase_spare`, a rotation onto a used side still erases it.
  - All 23 journal tests pass. The legacy CI matrix, the Slint checks and target clippy
    pass.
- **Device:** s88 (app `4b43bca9…`), 15 minutes with the soak DJ.
  - At boot, USB audio started before the load task could erase ahead, so nothing was
    erased then.
  - With the FLX4 unplugged for about 30 seconds, 8 stale spares were erased, each in
    24 to 130 ms; then none was left.
  - **18 saves, all 5 to 24 ms**, against up to 50 ms before. There were no erases in the
    run.
  - **Longest fill 5.9 ms**: no 21 ms fill.
  - **One lost USB packet** (against 5 on s87). It followed an 8.9 ms save without an
    erase. A save's many short flash operations can still cost a packet now and then,
    while no deck plays.
  - Underruns 0/0, the USB task's free stack 2832 bytes, no panics.

## Open

- A save can still lose an odd USB packet of silence.
- Spares are only erased while USB audio is off. Unplugging the FLX4 for half a minute
  now and then keeps saves short.
- The Hot Cues could later move to the USB stick, where writing stalls nothing.
