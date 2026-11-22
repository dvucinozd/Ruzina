# JC1060: the Library's sort, BPM filter and key colours

Date: 2026-11-21. Branch: `ogranak/jc1060-redesign`, after [the phase strip](2026-11-21-screen-layout.md#the-phase-strip).

## Source

The owner asked for filters on the Library page, modelled on the donor firmware. The donor's
ui-next Browse page (`ui_next_view.c`, `ui_next_key.c`, `ui_next_browse.c`; spec LB-7, LB-16,
LB-18) has these:

- a match strip with "BPM ±4%" and "KEY FITS";
- column headings that sort when tapped, and reverse on a second tap;
- key colours that show how each key sits with a deck's key;
- genre and tag filters, which our catalog has no data for.

The owner chose the BPM filter, the sort and the key colours (not KEY FITS). The reference
deck is the one that plays; with both or neither playing, it is the master.

## Now

- **Sort:** a tap on a heading sorts the list on that column, ascending; a second tap
  reverses it.
  - The columns are the title (its heading is the level's name), ARTIST, BPM, KEY and TIME.
  - Text sorts case-insensitively. The title column sorts by the title shown, which is the
    artist when a track has no title.
  - Keys sort in Camelot order, and an unknown key goes last whichever way the list runs.
  - Equal rows keep the level's order.
- **BPM ±4%:** a button under BACK. When it is on, the list shows only tracks within 4 % of
  the reference deck's playing tempo (pitch included); tracks without a tempo drop out.
  - The line under the button names the reference, for example "D1 128.0".
  - With no track on the reference deck, the filter shows every track.
  - The tempo counts to a tenth of a BPM, so a moving pitch does not rebuild the list at
    every frame.
- **Key colours:** a row's key turns green when it fits, yellow for +2 and blue for +7, and
  grey otherwise; it stays orange with no reference.
  - "Fits" means the same number, one step either way, or the relative major or minor.
  - Keys parse as Camelot ("8A") or classical names ("Am", "F#", "Bbmaj").
- **Headings:** at 16 px, like the titles, on a 34 px band in Rust `#B7410E`, in warm white.
  The sorted column is neon yellow with ▲ or ▼.
  - The owner asked for the larger font on s178 and for a more visible band on s179.
- **Where it applies:** the catalog's track list and a playlist's tracks; folders and the
  playlist tree are left as they are. The sort and the filter are not saved.
- **Browse knob and LOAD:** they follow the list as shown.
  - The screen task builds the view (`library_view.rs`, host-tested) and publishes it to
    `deck_view`: up to 1024 row positions, guarded by a sequence count.
  - The engine reads the view through `level_len` and `row_at` on the other core, without a
    lock. If it catches the view mid-write, it reads again. After 64 tries it gives up: the
    knob and LOAD do nothing that turn, rather than read the catalog's own order (Codex on
    #166).
  - When the view changes, the selection stays on the row it was on. The screen finds that
    row in the new view first and publishes the selection with the view, and LOAD reads
    the two together (`selected_row`), so it never pairs a new view with the old
    selection (Codex on #166).

## Tests

- `library_view` (5 tests):
  - a tap sorts and a second tap reverses;
  - each column sorts with case aside, and unknown keys go last;
  - the BPM filter keeps tracks within 4 %, in the catalog and in a playlist;
  - keys parse in both notations;
  - keys fit as the donor colours them.
- `deck_view`: the browse knob and LOAD follow a published view, and a view of another
  level leaves this one as it is.
- The legacy host tests (172) and clippy, with the Slint, Web Remote, profile and
  screen-still feature sets on the target.

## On the board

- **s178** (app `196952a2…`): the owner asked for larger headings.
- **s179** (app `89d9c3fb…`): headings at 16 px; the owner asked for a more visible band.
- **s180** (app `68235527…`): the rust band; the owner says "Sve radi". The log has no
  errors.
