# JC1060: playlists in the Library, and each deck's cover

Date: 2026-11-16. Branch: `ogranak/jc1060-playlists`, from `main` at `e7a5e17`.

## Problem

- **Playlists:** the card's catalog import already read Rekordbox's playlist tree
  (RP-06), but the deck screen's Library listed only the catalog's tracks.
- **Covers:** the catalog already resolved each track's artwork path, and the shared
  crate had a bounded JPEG decoder (RP-07). The board read no cover file, and the deck
  screen showed none.

## Change

### Playlists

- **The tree reaches the screen.** After the first pass, `idf_media_bridge.rs` hands
  the playlist index built on the published catalog to the screen. `deck_view` keeps it
  as a `Shelf`:
  - the top level, then each folder's children and each playlist's tracks;
  - all in Rekordbox's order;
  - a track listed twice in a playlist stays twice.

  The tree is published once and kept, so the engine and the UI task can both read it.
- **Levels:** the Library shows one level at a time:
  - every track;
  - the playlists' top level;
  - a folder's children, or a playlist's tracks.

  The selection is a row of the level shown. Each level above keeps its selection, so
  going back returns to the row you left.
- **Controls, on the controller and the screen:**

  | Control | Effect |
  |---|---|
  | Browse knob press | Opens the selected folder or playlist; nothing on a track. |
  | Shift + browse press | One level up: the track list goes to the top level, the top level back to the track list, and a folder or playlist to its parent. |
  | LOAD on a track | Loads it, as before. |
  | LOAD on a folder or playlist | Opens it and loads nothing, as RP-06 and the donor do. |
  | A second tap on a selected folder or playlist | Opens it, through the same press. |
  | BACK on the page | Goes up, through the same Shift + press. |

  The presses act only while the Library page is open. Taps travel the touch queue as
  `LibraryOpen` and `LibraryBack`, so the engine's task alone moves between levels.
- **The page:**
  - the heading shows where you are (`ALL TRACKS`, or `PLAYLISTS / Clubs / Peak`);
  - folders have an orange mark and playlists a blue one, with their number of entries;
  - track rows are as before;
  - the count reads `3 / 12` on the level shown.
- **Web Remote:** it keeps listing every track by catalog index.

### Covers

- **Reading:** when a deck loads a track, the load task reads its cover after the
  analysis (`read_cover`):
  - first Rekordbox's larger copy, `a1_m.jpg` (240 px), when the export has one;
  - else `a1.jpg` (80 px).

  The cover is decoded to 56 × 56 RGB565 under the RP-07 rules: 64 KiB at most, baseline
  JPEG only, 2048 px at most. The decoder lives on the heap (PSRAM).
- **No cover:** a missing, oversized, refused or undecodable file leaves the deck without
  one.
- **Showing:** the cover travels with the loaded track (`LoadedTrack::cover`). The deck
  column shows it under CUE, outside the lanes and meters Rust draws. An empty deck shows
  none.
- **Decoder:** the shared decoder's largest thumbnail goes from 40 to 64 pixels
  (`JPEG_THUMB_MAX_EDGE`). Its state grows from about 30 KiB to about 70 KiB, and every
  user holds it on the heap. Its tests now also check a 56-pixel cover against an
  independent decode, and refuse 65.

## Evidence

- **Host:**
  - new `deck_view` tests: the shelf keeps the index's tree, order and repeated tracks;
    the Library walks into a folder and a playlist and comes back to each level's row;
    the touch queue carries `LibraryOpen` and `LibraryBack`;
  - a new engine test: Shift + press goes up only on the Library page; LOAD on a
    playlist opens it and loads nothing; the press on a track does nothing; LOAD on the
    playlist's first track asks for that catalog index;
  - the `idf-deck-engine`, `idf-soak-autoplay idf-slint-ui` and
    `idf-soak-autoplay idf-web-remote` tests and clippy;
  - `ruzina-media-catalog` tests and clippy, the desktop simulator's tests;
  - target clippy for every CI feature set.
- **Device:** s125 (app `548d8a2c…`) is flashed and its readback matches. It has not run
  yet: playlists and covers wait for an export that has them. The owner's current export
  has no playlists.

## Test plan

1. Export from Rekordbox, onto the card:
   - a folder with two or three playlists, and a playlist at the top level;
   - one track in two playlists;
   - some tracks with artwork and some without.
2. Open LIBRARY:
   - Shift + browse press shows the playlists;
   - the press, LOAD or a second tap opens a folder or playlist;
   - BACK and Shift + press come back to the row you left.
3. LOAD a playlist's track onto each deck: it plays, and the deck shows its cover.
4. A track without a cover shows none, and the decks still play without underruns.
