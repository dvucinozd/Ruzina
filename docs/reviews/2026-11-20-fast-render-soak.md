# JC1060: the faster render in the product image, and the fill stack

Date: 2026-11-20. Branch: `ogranak/jc1060-fill-stack`, from `main` at `ff467ea`.

## The faster render on the board

s148 (app `5824136c…`) is the `--usb-stick` image of `main` at `ff467ea`, with the
[faster render](2026-11-19-fx-fill-cost.md) and no profile. It has the same flags as s142,
the image the board kept: deck engine, soak DJ, Slint, Wi-Fi, fill code in IRAM. The
soak DJ played both decks for 16 minutes with no touch, the same tracks in the same order
as on s142:

| | s142 (before) | s148 (faster render) |
|---|---:|---:|
| Fills | 159 000 | 159 000 |
| Fills over 3 ms | 5966 (3.75 %) | 362 (0.23 %) |
| Longest fill | 5235 µs | 4155 µs |
| Underruns | 0/0 | 0/0 |
| `packet_errors` / `iso_errors` | 0 / 0 | 0 / 0 |
| Stick reads | 14 320 | 14 320 |
| Stick read, mean / longest | 2118 / 5240 µs | 1598 / 4364 µs |

There was no fault and no fill over 6 ms. The stick reads were faster too, likely
because the render now leaves more of the core to the rest of the USB client task. The
owner listened on the FLX4: "Radi". s148 is now the image the board keeps.

## The fill stack

The UAC fill runs on the USB client task. The build keeps the fill's deepest call chain
within 4096 B of that task's 8 KiB stack. After the faster render it needed 3792 B in
the product image and 4016 B in the profile image, through the render and the Beat FX
filter's coefficient update.

`deck_mix::with`, where the fill enters the mix, had a 1504 B frame. Most of that was a
slot for a whole `FixedRateRenderGraph` (about 1.3 KiB): `render` built one with
`get_or_insert_with(FixedRateRenderGraph::new)` when the mix had none yet, and that slot
was in every fill's frame.

### Change

`render` now builds that graph in `build_graph`, a cold function that is never inlined.
Its frame is taken only on the first block, before the effects are attached. The
behaviour does not change.

The product image's fill chain now needs 2704 B, down from 3792 B, leaving 1392 B under
the budget. Legacy clippy for all target feature sets and the host, and the legacy host
tests, pass. The change was not flashed on its own.
