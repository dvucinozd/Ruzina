# JC1060: the deck's own Wi-Fi and a status page

Date: 2026-11-15. Branch: `ogranak/jc1060-softap`, from `main` at `fb6068a`
(after the [C6 link probe](2026-11-14-c6-link-probe.md)).

## Decision

**The C6 firmware stays as it is for now** (owner's choice). Building ESP-Hosted 2.11.7
for the C6 with IDF 6.1 would need six more registry components (`mqtt`,
`network_provisioning`, `iperf`, `iperf-cmd`, `wifi-cmd`, `ping-cmd`), or changes to
Espressif's build. The C6's current firmware already carries scans, an access point and
HTTP traffic. It reports itself as 2.3.2, although the link's init message says 2.3.0.

**The deck runs its own access point.** A phone joins the deck directly, so the deck
needs no home network and no credentials.

## Change

`build_legacy.py --wifi-probe` now builds this probe instead of the scan:
- **Access point:** `Ruzina-XXXX`, on channel 6, for at most 4 clients.
  - XXXX comes from the C6's access point address: the P4 has no Wi-Fi MAC of its own.
  - It is open, for the probe only.
- **HTTP server**, on core 1 at priority 2, at `192.168.4.1`:
  - `/` serves a page that shows both decks: empty, playing or paused, the track number
    and the position;
  - `/status` returns that state as JSON (`ruzina_legacy_engine_status` and
    `ruzina_legacy_engine_track`), and the page fetches it every second;
  - `/favicon.ico` answers 204.
- **Log:** a report every 30 s with the clients, the requests, the longest request and the
  free internal RAM. No addresses or names of the devices that join.
- **The C6 version** is read once the link is up, which fixes the early `ESP_FAIL` of the
  scan probe.

## Evidence

- **Host:** the IDF build has no warnings.
- **Device, s103** (app `f0098634…`): the phone joined `Ruzina-0000` and the page showed
  both decks playing, with their tracks and times. Owner: "sve je ok".
  - Over 13 minutes: underruns 0/0, `packet_errors=0`, longest fill 6.2 ms, 16 Hot Cue
    saves, USB task free stack 2832 bytes.
  - 91 requests, the longest 30 ms. Internal RAM: 247 KB free.
- **Device, s104** (app `3f3474f5…`, the address fix): the access point is
  `Ruzina-AB2D`, and the page loads (owner).
  - Underruns 0/0, `packet_errors=0`, longest fill 6.2 ms.
  - No icon warning. Once a `recv` error 104: the phone dropped a connection.

## Open

- **The network is open.** Before controls come in, it needs a password: one shown on the
  Settings page, say.
- **The page is a probe.** The Web Remote (`firmware-rust/crates/ruzina-web`: its routes,
  status JSON and controls as `ControlEvent`s) is the next step.
- **`Enqueue URB error: ESP_ERR_INVALID_STATE` at boot.** It came on 2 of the 4 boots with
  ESP-Hosted, at about 2.8 s, while ESP-Hosted resets the C6, and on none without it.
  Audio started normally each time.
- **The C6 firmware is still 2.3.x** against a 2.11 host.
