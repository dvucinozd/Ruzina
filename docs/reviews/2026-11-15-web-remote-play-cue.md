# JC1060: the Web Remote, PLAY and CUE

Date: 2026-11-15. Branch: `ogranak/jc1060-web-remote`, from `main` at `93b59ee`
(after [the deck's own Wi-Fi](2026-11-15-softap-status-page.md)).

## Scope

The owner chose the first slice: a password on the deck's network, and the shared Web
Remote (`firmware-rust/crates/ruzina-web`) with PLAY and CUE. The library, loads, seeks,
the mixer and the other buttons come later.

## Change

- **Password:** the access point is WPA2 with `CONFIG_RUZINA_WIFI_PASSWORD`. Its default
  is `12345678`, as the owner asked; a password outside 8 to 63 characters leaves the
  network open.
  - The Settings page shows the network and the password, through
    `ruzina_wifi_credentials`.
  - The log never prints the password.
- **Web Remote:**
  - `main/wifi_probe.c` replaces the probe's status page with a small socket server. It
    takes one connection at a time, reads a bounded request (2 KiB, head and body) and
    hands it to `ruzina_web_handle` in the Rust core.
  - `rust-core/src/web_remote.rs` (feature `idf-web-remote`, which `--wifi-probe` now
    turns on) runs `ruzina-web`'s `WebServer` over a backend for the engine.
  - The page, its status and the guard are `ruzina-web`'s: Host allow-list, and the
    control marker and same-origin check on controls.
- **Controls take the touch path:**
  - PLAY and CUE become touch requests on a second touch queue, `deck_view::WEB`, which
    the USB task drains next to the screen's. They pass the same admission as the deck
    screen's buttons.
  - PLAY acts on the press, CUE on its press and release.
  - Any other control is refused as busy (503), and nothing of that command is queued.
- **Status:** each deck's position, playing, BPM and pitch, from `deck_view::LIVE`.
  - Titles and durations stay empty: the screen owns them.
  - The mixer reports neutral values.
  - Loads, seeks and the library are refused.
- **Build:** `--wifi-probe` now requires `--slint-ui`, which shows the password.

## Evidence

- **Host:** two new tests.
  - PLAY and CUE reach the web queue, PLAY as one press and CUE as a press and a release.
    A volume command is refused with 503, and nothing is queued.
  - The page and `/api/status` answer for 192.168.4.1, and not for a foreign Host.

  The legacy CI matrix, the Slint checks and target clippy, also with `idf-web-remote`,
  pass. The IDF build has no warnings.
- **Device:**
  - s105 had a password made at random on first boot. The owner asked for a fixed one
    instead, so this was dropped, and with it the boot's NVS write.
  - s106 (app `0eaa4502…`): the phone joined with the password, and PLAY and CUE worked.
    Owner: "sve je ok".
  - In the first minutes: underruns 0/0, `packet_errors=0`, longest fill 5.0 ms, USB task
    free stack 2832 bytes. 13 requests, the longest 69 ms.

## Review fixes (Codex)

- **The request reader searched beyond the received head.** The buffer is reused and never
  NUL-terminated, so `strcasestr` could read stale bytes past its 2 KiB end (P1).
  `content_length()` now searches only the received head bytes and caps the value. A host
  harness under AddressSanitizer passes five cases, the buffer exactly as long as each
  request.
- **A CUE tap could leave a held preview** when the web queue had room for its press only
  (P2). The backend now checks that the whole command fits before queueing any of it; a
  new test fills the queue to one free slot.
- Both fixes are host-tested and build into s107 (app `43cb3b6b…`); s107 is not yet run on
  the board.

## Open

- **The mixer, SYNC, PFL, the faders, the library, loads and seeks.** The page offers them,
  and they answer 503 or empty.
- **No titles or durations** in the status.
- **`12345678` is easy to guess.** It is a build setting, so changing it is one line.
- **`Enqueue URB error` at boot** with ESP-Hosted, as before.
