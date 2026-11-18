# JC1060: the Settings page keeps its rows

Date: 2026-11-19. Branch: `ogranak/jc1060-settings-model`, from the SD retry
(`d5b92d8`, merged as `674dad0`).

## Problem

While the Settings page is open, it refreshes its figures twice a second. Each refresh
gave both columns a new `ModelRc`:
- Slint's repeater then rebuilt every row's items, unchanged rows included;
- on s130 that made a frame over 30 ms 3.3 to 3.7 times a second, for as long as the
  page stayed open;
- those frames took 46.9 ms on average, 62.7 ms at most, with 17.6 ms of preparation
  and about 1931 allocations each.

An outside review pointed at this rebuild; the Library page already kept one model.

## Change

`slint_ui.rs`:
- The two columns get their models once, at start (`SettingsModels`), as the Library
  does.
- A refresh brings each model to the new lines with `sync_rows`: a row whose text
  changed is set, a new row is pushed, and a row no longer there is removed. Rows that
  stay the same are not touched, so Slint keeps their items.

## Evidence

- **Host:** the legacy tests and clippy, and target clippy for every CI feature set.
- **Device:** s133 (app `1ab195d4…`, with the SD retry). The owner left Settings open
  for about 20 seconds: "Radi".

  | | s130 | s133 |
  |---|---|---|
  | Settings frames over 30 ms | 3.3–3.7 a second, on each of four boots | 6 in 46 seconds, mostly when the page opens |
  | A slow Settings frame | 46.9 ms, at most 62.7 ms | 43.3 ms, at most 59.6 ms |
  | Average Slint frame (event 31) | 10.8–11.8 ms | 6.3–6.7 ms |

  `packet_errors=0`, underruns 0/0, and the SD card came up at the first try.
- **Not changed:** the frame that opens the page is still slow, and so is a Library page
  turn. Slint's preparation there is not explained by a model rebuild: the Library
  already kept its model.

## Test plan

1. Open Settings and leave it for 20 seconds: the figures update, and the log shows
   almost no `event=24` lines with `pass=3`.
2. Change a setting: its row follows at once.
