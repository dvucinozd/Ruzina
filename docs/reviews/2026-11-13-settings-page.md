# JC1060: the Settings page

Date: 2026-11-13. Branch: `ogranak/jc1060-settings`, from `main` at `6978bcf` (after
[Master Tempo stops that skip nothing](2026-11-13-mt-phase.md)).

## Problem

The deck screen's SETTINGS tab only marked itself. Whatever the deck knew about itself
was only in the serial log:
- the decks' sources, buffers and underruns;
- the mix's figures;
- which firmware was running.

## Change

- **Page:** the SETTINGS tab now opens a status page over the lanes and meters, as the
  other pages do. It refreshes twice a second while open. It has two columns:
  - **DECKS:** the Library's track count, then each deck's source rate (or "no track"),
    buffered audio in seconds, and underrun blocks.
  - **MIX AND SYSTEM:**
    - the output (48 kHz to the FLX4);
    - the time played, from the rendered frames;
    - the limiter's samples;
    - the uptime;
    - the firmware's version, build date and time, and the start of its ELF SHA-256, from
      IDF's app description;
    - whether the soak DJ is built in.
- **Figures:** the USB task publishes `Mix::stats` to the screen on each refresh outside
  the fill (`deck_view::STATS`, eight relaxed atomics). A page may show two refreshes
  mixed, which does not matter for a status page.
- It shows only what the deck reports. There are no settings to change yet.

## Evidence

- **Host:** 1 new test: the published figures reach the screen.

  The legacy CI matrix, target clippy and the profile build pass.
- **Device:** s87 (app `ee29e6be…`), owner verdict "Radi".
  - The firmware shows as `0.1.0-legacy-eval`, built Nov 13 2026 02:39:38.
  - In the first minutes: underruns 0/0, `packet_errors=0`, the USB task's free stack
    2832 bytes, longest fill 4.7 ms.
  - The worst Slint frame in a 10 s report was 54 ms, while the average stayed at
    5.4 ms. s86 showed the same right after boot; the page was not singled out.

## Test plan

1. SETTINGS opens two columns; the lanes and meters are hidden.
2. While a deck plays, its buffer, the time played and the uptime move.
3. The firmware and the build date match the image.
4. Back to OVERVIEW: the lanes and meters return.
