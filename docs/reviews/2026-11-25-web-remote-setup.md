# Web Remote: a SETUP page for the deck's settings

Date: 2026-11-25. Branch: `ogranak/web-remote-settings`, from main `cac3bab`.

The Web Remote had no management pages. This adds the first one, chosen by the owner:
**SETUP**, a fourth view after DECK 1, DUAL / MIX and DECK 2.

## What it offers

The settings are those of the deck's own Settings page:

- LOAD LOCK;
- each deck's jog (VINYL or CDJ);
- the screen's brightness, in its steps of 5 % between 10 and 50 %;
- the MAIN trim (0, −3 or −6 dB);
- the screensaver's wait (off, 2, 5 or 10 minutes).

It also shows what the Settings page tells about the deck: the firmware, its build time,
the controller, the media (ready, reading or removed), the track count and the uptime.

## How it works

**API** (`ruzina-web`, `settings.rs`):

- `GET /api/settings` answers with the settings and the information.
- `POST /api/settings?name=…&value=…` changes one setting. Like every other change, it
  needs the control marker, and it answers with the settings as they then are.
- A wrong name or value is refused with 400 and changes nothing.
- A deck without settings answers 501.

**On the board** (`web_remote.rs`):

- A change goes through the same store as the Settings page (`deck_settings::change`)
  and is saved the same way, once it has been quiet and no deck plays.
- `change` now has two writers: the screen task and the web task. It applies its edit
  by compare-and-swap, so neither undoes the other's change.
- The screen picks up a change made from outside on its next frame: the brightness
  follows, and the Settings page shows the new values.

**The page:**

- It reads the settings every 2 s while SETUP is open.
- Its four tabs sit in one row on a phone.
- On a wide screen the panel is at most 720 px wide, centred.

## Tests

- `ruzina-web`:
  - query parsing, every setting and the refusals;
  - the HTTP exchange: read, change, refusals, the missing marker, and a deck without
    settings.
- The page: its rendering, and the queries it sends, the brightness kept within its range.
- `ci_host.sh` (CI's host list), target clippy, and `ruzina-web` and `desktop-simulator`
  tests and clippy all pass.

## On the board

**s205** (app `2f9cd29e…`): the play image (`--slint-ui --fill-iram --usb-stick`, no
soak DJ, no profile).

The owner used SETUP from a phone on the deck's network (`Ruzina-AB2D`):

- the brightness and LOAD LOCK changed on the deck;
- a change on the deck's Settings page showed on the phone;
- the information read correctly.

The owner's verdict: "Sve radi".

## Not covered

- **Network:** the deck's network name and password are still a build setting.
- **Firmware:** there is no firmware update (OTA) from the page.
- **Controller profiles:** they cannot be managed from the page.
