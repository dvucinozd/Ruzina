# Controller profiles in SETUP, and three fixes from the day

Date: 2026-11-25. Branch: `ogranak/profiles-and-polish`, from main `4dccc9b`.

The owner asked for both of these in one PR:

- the Web Remote's controller profiles;
- the small fixes left from the day's work.

## Controller profiles in SETUP

- **On the page:** SETUP lists the five profiles the deck carries: the FLX4, DDJ-400,
  DJ2GO2 Touch, S2 MK3 and Inpulse 500. Each row shows the profile's name and its USB
  VID:PID.
- **Marks:**
  - ACTIVE marks the attached controller's profile;
  - TESTED marks the profiles tried on this hardware, which is the FLX4 only.
- **API:** `GET /api/settings` carries them as `profiles`, as `{name, id, tested, active}`.
- **Read-only:** profiles cannot be uploaded or chosen from the page. The deck picks one by
  VID:PID when a controller attaches.

## The fixes

**A long fill when the stick is put back** (`deck_resources.rs`).

- The next session's producers are written in 16 KiB steps, with one tick between them,
  while USB audio runs.
- Written in one burst, they held a fill to 6 to 7 ms on s201p.

**Lane misses** (`LaneCache::update`, `slint_ui.rs`).

- **Forward moves:** a move forward past what the cache holds is now drawn whole into the
  spare cache, as a jump is. Topped up instead, the lanes' task was told the new place
  before its columns were in, and missed refreshes: about once in 80 s on s199p.
- **Moves back:** while the columns behind are drawn, the place last published stays in
  force.

**Text tearing.** It was looked at and left alone. The owner has not seen it, and a
change of text could tear for one refresh at most.

## Tests

- **`ruzina-web`:** the HTTP test checks the profiles in the settings JSON.
- **The page:** a test checks the list and its marks.
- **Pacing:** the producers' pacing runs inside the existing test of a later session
  (`a_later_media_session_takes_new_producers_for_the_published_rings`).
- **Also passing:** `ci_host.sh`, target clippy, and the `ruzina-web` and
  `desktop-simulator` tests.

## On the board

**s208p** (app `50175f51…`) is a profile image without the soak DJ. The run lasted 10
minutes, with Master Tempo on for the last part. The owner checked SETUP and the
profiles, jumped with hot cues and loops at the largest zoom, and pulled the stick out
and put it back: "Sve radi".

| Measure | s208p |
|---|---|
| Lane misses | 0 in 16 reports (2 copies from the spare cache: the loop) |
| Longest fill after the stick was put back | 3.30 ms (6 to 7 ms on s201p) |
| Master Tempo fills | 13.1 % over 3 ms, longest 5.14 ms |
| Plain fills | 0.1 % over 3 ms, longest 3.94 ms |
| Underruns | 0/0 |

**s209** (app `91b83e13…`) is the play image: the same firmware without the profile. The
owner played on it: "Sve radi".
