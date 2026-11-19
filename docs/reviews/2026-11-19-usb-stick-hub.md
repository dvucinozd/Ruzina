# JC1060: music from a USB stick behind a hub

Date: 2026-11-19. Branch: `ogranak/jc1060-usb-stick`, rebased on `main` at `175731a`.

## Why

The owner wants tracks to come from a USB stick, not the SD card. The plan from
2026-11-13 put the stick on the FS Type-C port. That port would then become a USB host,
so:
- USB Serial/JTAG (COM3) would be gone;
- the PC could not power the board;
- the board would need its own power on CN2.

For development the owner chose a USB hub on the HS port instead. The FLX4 and the stick
share the hub. The FS port stays the PC's, for power, flashing and the COM3 log. The
CN2 layout is left for the end.

## Change

- **USB host:** `--usb-stick` keeps the host on the HS (UTMI) port only. It turns on the
  vendored USB Host Library's hub support (`CONFIG_USB_HOST_HUBS_SUPPORTED`, esp-usb
  1.6.0); the option needs no experimental features.
- **Devices:** the client skips a hub (`bDeviceClass` 0x09), which belongs to the
  library's hub driver. A mass-storage device binds as the stick, and anything else is
  the controller. They may attach in either order.
- **Stick reads:** the Bulk-Only reader from 2026-11-13 is unchanged. Its 4 KiB pieces
  still fit a full-speed stick, and a high-speed stick takes them as 8 packets of 512
  bytes.
- **Rebase:** the SD path keeps `main`'s three tries and its slot-only teardown.
- **CI:** the legacy CI builds the stick image (`deck-engine`, Slint, `--usb-stick`).

### Review fixes (Codex, two P2)

- **Unplugging:** the client task closed the stick at DEV_GONE, while the media owner could
  be preparing a transfer with the same handle. Now:
  - the owner marks each command busy before it checks that the stick is present;
  - the client marks the stick gone at once, and closes the device only once no command
    is busy;
  - both flags are sequentially consistent, so either the command sees the stick gone or
    the close waits for it;
  - the client loop wakes every 50 ms in the stick build, so a deferred close runs.
- **Timeouts:** this host library cannot cancel EP0, and a bulk transfer may still be in
  flight after its cancel. A transfer that has not ended is now never submitted again.
  The stick is given up (`usb_msc stuck`), until it is unplugged and the app restarts. A
  completion wakes only the wait for its own transfer.
- s138 (app `d42b7986…`) builds with no warnings. On the board (2026-11-19), the owner
  loaded a track from the stick on deck 1 and pulled the stick out while it played:
  - `usb_msc unbound commands=1951 errors=1 stalls=0 resets=0 retries=0`;
  - `rust_sd_owner=QUIESCED error=ESP_ERR_NOT_FOUND reinsert=APP_RESTART_REQUIRED`;
  - the media client stopped, and there was no `stuck` and no fault;
  - the board kept running, and the FLX4 stayed attached;
  - all 6 reports had `packet_errors=0` and underruns 0/0;
  - the owner: "Radi".

### Putting the stick back

On s138, a stick put back was enumerated, but nothing read it. The media owner had
stopped for good, and the Library still listed the old tracks, which no longer loaded.
The owner asked for the quick fix:
- **Library:** while the media are gone (`media_signal::muted()`), the Library heading
  reads MEDIA REMOVED and its count RESTART NEEDED. The rows stay as they were read.
- **Restart:** a stick that attaches after one was lost in this run restarts the app
  (`usb_msc reinserted restart=YES`, then `esp_restart`). The loss is marked at DEV_GONE,
  so even a stick put back before the deferred close restarts it. A restart is no harsher
  than a power cut, which the settings and Hot Cue stores take.
- **s139** (app `c2a28cbb…`, no warnings): the owner loaded a track and pulled the stick
  out mid-track.
  - The read in flight went through 3 Reset Recoveries and 2 retries before the detach,
    then `QUIESCED` and `usb_msc unbound`.
  - The stick put back gave `usb_msc reinserted restart=YES` and a software reset
    (`reset_reason=3`).
  - The stick mounted again, and both decks loaded and played from it.
  - All 6 reports had `packet_errors=0` and underruns 0/0, with no stuck transfer and
    no fault.
  - The owner: "Radi".
- Reading a stick again without a restart (the media session, catalog and decks started
  over) is left for later.

## Hub power

The hub has its own adapter, and with it plugged in the hub drove about 5 V back up its
upstream cable. With the PC also powering the board over FS, that would put two live 5 V
sources on USB5V_IN. The owner chose to run the hub **without its adapter**. The
upstream cable then measured about 0 V, and the PC powers the board, the hub, the FLX4
and the stick.

## Evidence

- **Build:** s137 (app `fbb0b5e1…`), `deck-engine` with Slint and `--usb-stick`. The IDF
  build has no warnings.
- **Device:** the SD card was taken out, and the hub carried the FLX4 and a JetFlash
  stick (1 GB, USB 2.0, exFAT on the whole device with no partition table).
  - Plugging the hub in while the chip waited in the ROM reset the board, which started
    s137 by itself. A restart over COM3 (`--mode reboot`) then logged the boot:
    - `S6_USB_HOST port=HS phy=UTMI hub=YES`;
    - the FLX4 attached at high speed and bound MIDI and UAC. As after any warm reset,
      it detached once and attached again;
    - the stick: `usb_msc bound` (bulk 512), `inquiry` "JetFlash", one unit attention
      after the reset (sense 6/28/00), then `capacity` and
      `S6_RUST_USB_STICK sectors=1982464`.
  - The catalog of 25 tracks was ready about 2 s after the mount, as from the SD card. The
    mount came about 2 s later than the card's, behind the hub's enumeration.
  - The owner loaded and played both decks and browsed the Library (83 steps, 8
    presses): "Radi".
  - 71 680 sectors were read with no stick recovery. All 9 reports had
    `packet_errors=0` and underruns 0/0, with no fault and no brownout.

## Not covered

- **Throughput:** see [one transfer per high-speed stick read](2026-11-19-stick-8k-reads.md).
- **Long run:** no soak from the stick yet.
- **Unplugging:** the stick pulled out while playing stops media access cleanly (s138). Putting it back restarts the app (s139). Pulling the whole hub out is not tried.
- **Hot-plugging:** plugging the hub in reset the board once. Connect it with the board
  off, or before a flash.
- **CN2:** the planned layout (stick on FS, power on CN2, log on UART0) is left for the
  end.

## Test plan

1. Flash a `--usb-stick` image with the hub not yet connected, then connect the hub with
   the FLX4 and the stick, and start the board (`--mode boot`, or `--mode reboot` if it
   already runs).
2. The log shows the FLX4 bound, `usb_msc capacity` and `S6_RUST_USB_STICK sectors=`.
3. Load and play both decks and browse the Library: `packet_errors=0`, underruns 0/0.
