# Web Remote: the deck's network password from the SETUP page

Date: 2026-11-25. Branch: `ogranak/web-remote-network`, from main `986c1c9`, after
[the SETUP page (#171)](2026-11-25-web-remote-setup.md).

The deck's own network (`Ruzina-XXXX`) had a password fixed at build time
(`CONFIG_RUZINA_WIFI_PASSWORD`). The owner chose to change only that password from the
page. The network's name stays, and joining another network is left for later.

## How it works

**The page** (SETUP, WI-FI PASSWORD):

- A password field with SHOW and SAVE.
- It checks the password as the deck does: 8 to 63 printable characters, or 64 hex
  digits, with no quotes or backslashes.
- It sends the password in the body of `POST /api/network` as `{"password":"…"}`, and
  never keeps it.
- The information now names the network.

**ruzina-web:**

- The password travels in the body only. A query is refused, and so are an empty, escaped
  or cut-short body and a request without the control marker.
- The deck answers:
  - 200: taken;
  - 409: stop both decks first;
  - 400: invalid;
  - 501: no network to change.

**The board:**

- **Hand-over** (`deck_network.rs`): the web task hands a valid WPA2 password to the app
  main task, one at a time. It is wiped once taken.
- **While a deck plays:** the change is refused, because the save writes flash, which
  stalls the audio.
- **Saving** (`ruzina_wifi_change_password`, `wifi_remote.c`): once no deck plays, the app
  main task saves the password in NVS, under namespace `ruzina`, key `ap_password`. The
  running access point then takes it (`esp_wifi_set_config`). Its stations drop and join
  again with the new password.
- **At boot:** the password is read from NVS before the screen and USB audio start, else
  the build's default is used.
- **Why NVS:** the settings store's network record is meant for joining another network,
  and it needs an HTTPS URL. The partition table already had an `nvs` partition that
  nothing used.
- **The deck's Settings page** still shows the password in force. It gains
  **WI-FI PASSWORD: DEFAULT**, which puts the build's password back, so the owner cannot
  be locked out.
- **Logs** say only `password=OWNER` or `DEFAULT` and `password_change=…`, never the
  password.

## Tests

- **ruzina-web:** the body-only rule over HTTP (query, empty, escaped and cut-short bodies,
  the marker) and the deck's four answers. Also the JSON field parsing.
- **Board side:** the WPA2 rule and the one-at-a-time hand-over (`deck_network`).
- **The page:** its checks, and that it sends nothing for an invalid password.
- **Also passing:** `ci_host.sh`, target clippy, and the `ruzina-web` and
  `desktop-simulator` tests and clippy.

## On the board

**s206** (app `ca181185…`, the play image):

- **Change:** the owner set a new password from a phone. The log shows
  `password_change=ESP_OK source=OWNER`, and the phone joined again with it.
- **After a restart** (`--mode reboot`): `wifi_ap nvs=ESP_OK password=OWNER`, so the
  password survived.
- **DEFAULT on the deck:** `password_change=ESP_OK source=DEFAULT`, and the phone joined
  with 12345678.
- 0/0 underruns. The owner: "Sve radi", then "Radi".
- **One long fill:** after the restart with the owner's password, one fill took 7.5 ms at
  boot. The first boot, with nothing in NVS, had 2.4 ms. The Wi-Fi task read NVS once USB
  audio ran.

**s207** (app `6f8be16a…`, `f429f6c`) reads the password at boot, with the deck's
settings.

- `wifi_ap nvs=… password=…` is now logged before `usb_uac stream=STARTED`.
- After a restart with the default password, the longest fill was 1.76 ms, with 0/0
  underruns.
- A restart with an owner's password in NVS was not measured again on s207.

## Not covered

- **The network's name** cannot be changed.
- **Joining another network** (station mode) is not done.
- **A change while a deck plays** is refused, not queued.
