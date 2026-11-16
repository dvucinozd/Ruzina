# JC1060: the deck screen draws its big frames faster

Date: 2026-11-16. Branch: `ogranak/jc1060-library-open`, from `main` at `4a57626`.

## Problem

Opening the Library, turning its pages and leaving Settings open drew slow frames
(s100):
- **Opening the Library:** 102 ms, then 88 ms for the second buffer.
- **A page turn:** about 84 ms on average.
- **Settings:** about 50 ms per buffer, twice a second.

## Change

- **Covered pages are not drawn:** `ui/overview.slint` hides the decks, the waveforms
  and the FX panel while another page covers them whole.
- **Slint's draw is built for speed:** the new crate `rust-core/screen-lines`
  (`ruzina-screen-lines`) holds the one `render_by_line` call. Slint's renderer is
  generic, so its code is made in the crate that calls it. The firmware is built for
  size; this crate is built at `opt-level = 3`.
- **Lines are drawn straight into the frame buffer:**
  - Slint fills every pixel of each range it draws, background first, and the buffer
    drawn is the one the panel is not showing.
  - A line no Rust-drawn rectangle crosses is therefore drawn straight into the frame.
  - Only a line one does cross still goes through the line buffer in internal RAM. Its
    parts outside the rectangles are copied out as whole blocks.
- **Glyphs at their own size:** the Slint software renderer 1.18.1 is vendored in
  `third_party/` with one fast path. A glyph drawn at its own size is read row by row.
  See [the renderer's notice](../../firmware-idf/legacy-jc1060/third_party/slint-renderer-NOTICE.md).
- **The slow-frame log says where the time went.** For each frame over 30 ms:
  - **Event 27:** the scene's preparation in µs, then Slint's line drawing.
  - **Event 28:** the copy out of the line buffer, then the cache write-back for the
    panel.
  - **Event 29:** the screen task's allocations and frees, then the µs they took.
    `idf_alloc` counts them for the task that registers itself (`count_task`), with
    the CPU's cycle counter.
- **CI** runs clippy on the new crate for the host and the target.

## Tried and dropped

Both were tried on the board and reverted:
- **Code and constants from PSRAM** (`CONFIG_SPIRAM_XIP_FROM_PSRAM`, s120):
  - The preparation stayed the same, so flash was not what slowed it.
  - It took 2.4 MB of PSRAM and 10 KB of internal RAM.
- **The screen's small allocations in a 64 KB internal-RAM budget** (s121):
  - The preparation stayed the same.
  - The budget filled within seconds, and the free internal RAM's largest block fell
    from 176 KB to 98 KB.

## Evidence

- **Host:**
  - fmt; the `idf-soak-autoplay idf-slint-ui` tests (156 + 25) and clippy;
  - target builds and clippy for both deck screen images;
  - clippy for `ruzina-screen-lines` on the host and the target;
  - the allocator test for `idf-media-load`.
- **On the host bench:** the Overview, Library, Hot Cues and Settings pages drew the
  same pixels (frame checksums) with the glyph fast path and the speed-built crate.
- **Device:** s122 (app `35e2d787…`), from this branch. The owner opened the Library,
  turned its pages and opened Settings.

  | Frame (rows drawn) | s100 | s122 |
  |---|---|---|
  | Library page (395) | 84 ms average, 92 ms longest | 70 ms average, 78 ms longest |
  | Library open (472) | 75 ms average, 102 ms longest | 74 ms average, 87 ms longest |
  | Settings (338) | about 50 ms | 45 ms average |
  - Underruns 0/0, the longest fill 6.2 ms, `packet_errors=0`.
  - Internal RAM: 265 KB free, largest block 176 KB.
- **Where a big frame's time goes now** (s122, a Library page turn, 395 rows):
  - the scene's preparation: 36 ms;
  - drawing the lines: 31 ms;
  - the cache write-back: 0.4 ms.

  Before the lines were drawn straight into the frame (s119), the same frame also
  copied for 15 ms.

## Open

- **The preparation is half of a big frame** and about 100 times slower than on the
  host:
  - **Allocations:** 1000 to 3500 per frame, about 2.5 µs each, through IDF's heap.
    That is 3 to 9 ms. A fast allocator for the screen task alone would win most of
    it back.
  - **The rest** (14 to 27 ms) is not yet explained. It would need timing inside
    Slint's preparation: text, the item tree and sorting.

## Test plan

1. Open the Library several times and turn the browse knob across pages.
2. Open Settings and leave it open for ten seconds.
3. Go back to the Overview: it looks as before, and the decks play without underruns.
