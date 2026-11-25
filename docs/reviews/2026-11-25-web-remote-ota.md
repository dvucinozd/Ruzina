# Web Remote: firmware updates from the browser

Date: 2026-11-25. Branch: `ogranak/web-remote-ota`, from main `2a68ea8`, after
[controller profiles in SETUP (#173)](2026-11-25-profiles-and-polish.md).

The owner asked for firmware updates from the browser ("OTA iz preglednika"), signed with
a new Ruzina key.

## The key and the bundle

- **The key:** a new P-256 key, `ruz-001`, made with `tools/ota_bundle.py generate`.
  - The private half is kept outside the repository, in `D:\BoardEvidence\keys`.
  - The deck carries only the public key (`rust-core/src/deck_ota.rs`). Its SHA-256 is
    `bd94eba9…a76a72`.
- **The bundle:** `tools/ota_bundle.py bundle` wraps a build's app image in a `.ddjota`
  bundle.
  - The bundle is a 188-byte header followed by the image, in the layout `ruzina-ota` reads
    (the same layout as M2.5).
  - The header's first 124 bytes are signed.
  - The tool takes the project and version from the image's own app description, so they
    match what the deck reads back from the written slot.
  - `inspect` checks a bundle's signature and hash.
- **The version:** each image now carries a version, set by `build_legacy.py --version`
  (for example `s210`). The page shows it.
- **ruzina-ota:**
  - `Expected` names the key id it accepts. It was fixed to M2.5's `rel-001`.
  - `Push::new` takes the firmware's image limit, at most a 6 MiB slot. The old fixed
    limit was 4 MiB, and s209's image is already 3.43 MB.

## Two app slots and rollback

**`partitions-ota.csv`**, used by every image with Wi-Fi:

| Partition | Offset | Size |
|---|---|---|
| ota_0 | 0x10000 | 6 MiB |
| ota_1 | 0x610000 | 6 MiB |
| hotcues | 0xC10000 | 1 MiB |
| settings | 0xD10000 | 16 KiB |
| otadata | 0xD14000 | 8 KiB |

- The Hot Cue journal, the settings and the NVS partition (which holds the network's
  password) stay where they were.

**Rollback:**

- `CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE` is on.
- A new image boots *pending*. If the deck restarts before the image is kept, the
  bootloader goes back to the previous image.

**Keeping a new image** (`ota_slot.c`, `ruzina_ota_settle`):

- The Wi-Fi task keeps a pending image once it has run 30 s with the Web Remote's server
  listening, because the Web Remote is how the next image arrives.
- It rolls back at once if:
  - the access point cannot start;
  - the server fails;
  - the server has not started within three reports.

**The build:** `build_legacy.py` refuses an image with Wi-Fi that lacks the rollback
bootloader, or that does not fit a slot.

## The upload

**The page** (SETUP, FIRMWARE UPDATE):

- **Choosing a file:** a `.ddjota` picker and INSTALL.
- **Checks:** the page checks the file's name and size.
- **Sending:** it sends the bundle whole as the body of `POST /api/ota/p4`. The request
  carries `X-DDJ-Control: 1` and `application/octet-stream`.
- **While sending:** it shows the upload's progress.
- **Refusals:**
  - 409: stop both decks first;
  - 400: not a bundle signed for this deck;
  - 500: the deck keeps its firmware;
  - 503: another update is in progress.
- **The info list** now names the slot the firmware runs from.

**The server** (`web_client.h`):

- A body too long for the 2 KiB request buffer streams instead, when it is no larger than
  the header and a slot.
- Its bytes go on as they arrive, at most 2 KiB at a time.
- The connection's timeout runs from the last bytes that arrived (10 s).
- A refused body is still read to its end, so the answer reaches the browser.
- A dropped connection cancels the upload.

**ruzina-web:** the existing upload path (`begin_body`, `body_chunk`, `finish_body`) runs
the guard: the host, the control marker and the origin.

**The deck** (`deck_ota.rs`, `ota_slot.c`):

- **Before flash:** nothing reaches flash until the header's fields, key id and signature
  are checked, and the image's first bytes are confirmed to be an ESP32-P4 app.
- **Decks playing:** the upload is refused while either deck plays, because writing flash
  stalls the USB audio fills.
- **Writing:** the image goes to the slot the deck is not running. The slot is erased as
  it is written, so no single call holds the web task for the whole erase.
- **Switching the boot slot:** this happens only once all of these match:
  - the image's SHA-256;
  - IDF's check of the written image;
  - the written app description's project and version.
- **Restart:** the deck restarts into the new image 1 s after the answer has gone.
- **Logs:** `ota begin/end/boot/keep/rollback slot=…`, and `ota running=… version=…
  pending=…` at start.

## The screen during an update

**UPDATING FIRMWARE** (`screen_idle.rs`, `slint_ui.rs`, `overview.slint`):

- **When it shows:** once a bundle's header passes and both decks are stopped, a cover over
  everything says UPDATING FIRMWARE, and that the deck restarts when the update is in.
- **While it shows:** the lanes are hidden, the wordmark stands still, and once the cover
  is drawn the screen draws nothing more until the restart.
- **Touch:** the cover takes every tap.
- **If the update ends unfinished,** the screen comes back.

**The panel's interrupt while flash is written** (`CONFIG_LCD_DSI_ISR_CACHE_SAFE`, in
`sdkconfig.slint-ui.defaults`):

- **The cause of the flicker:** writing flash turns the cache off. The DSI driver's
  interrupt was held off until it came back, so the panel missed frames and the screen
  flickered. It did so on s212 even under the still cover.
- **The fix:** the interrupt now runs with the cache off.
  - Our `frame_done` callback is already in IRAM.
  - It calls only `FromISR` functions, which IDF keeps in IRAM.
- **Every screen image** has it, so the Hot Cue journal's and the settings' writes no
  longer hold the panel either.

## Tests

- **ruzina-ota:** a header naming another key id is refused.
- **Board side** (host):
  - a bundle the tool made with the Ruzina key passes the header checks;
  - one for another key, or a tampered one, is refused before the slot;
  - a bundle larger than a slot is refused at once;
  - the streamed path refuses a request without the marker (403) or one too long (413),
    answers what ended an upload, and starts a new one after a cancelled one.
- **`web_client_test.c`:**
  - a 10 000-byte body streams in pieces of at most 2 KiB;
  - a refused body is read to its end before its answer;
  - a stalled one is dropped after 10 s and cancelled;
  - one over the limit is not taken.
- **The screen:** UPDATING FIRMWARE is set once an upload stops the decks, refused while a
  deck plays, and cleared when the update ends unfinished.
- **The page:** its file checks, the request it sends, its progress and a refusal.
- **Also passing:** `ci_host.sh`, target clippy, and the `ruzina-web`, `ruzina-ota` and
  `desktop-simulator` tests.

## On the board

**s210** (app `0d976bb1…`, version `s210`), flashed over USB:

- **Flash:** the new table, the rollback bootloader and the app went to ota_0, and
  otadata was erased. Each written span was read back and matched. The live spans were
  saved first, to `s210-ota-flash/restore-*.bin`.
- **Boot:** the bootloader logged `No factory image, trying OTA 0`. The app logged
  `ota running=ota_0 version=s210 pending=0`, and the access point and the server
  started.
- **Refusals,** from the PC on the deck's network:
  - an upload without the control marker: 403;
  - a bundle with one signature byte changed: 400, after its 3.4 MB had been read, and
    without touching the slot.

**s211** (app `a1f398fa…`, the same source, version `s211`), sent as `s211.ddjota`:

| Step | Seen |
|---|---|
| Upload | 200 `Installed; restarting` after 72 s |
| Slot | `ota begin` / `ota end` / `ota boot slot=ota_1`, all `ESP_OK`, then `ota restart` |
| Boot | `Loaded app from partition at offset 0x610000`, `ota running=ota_1 version=s211 pending=1` |
| Kept | `ota keep slot=ota_1 result=ESP_OK`, 30 s later |

**Rollback:**

- **What was done:** `s210.ddjota` was uploaded (200 after 102 s). The board was then
  reset over USB (`--mode reboot`) 10 s after the answer, before the image could be kept.
- **What happened:** the bootloader loaded ota_1 again, with
  `ota running=ota_1 version=s211 pending=0`.
- **After the rollback:** 0/0 underruns, and the longest fill was 1.76 ms.

**During an upload:**

- **On s211 the screen flickered,** as the owner saw.
- **USB audio stalls.**
  - Its fills took up to 34 ms, and about 1 700 packets were lost over the 72 s, with
    both decks stopped.
  - While flash is written, code cannot run from it: the screen's tasks and the USB
    audio fill wait.
- **After the restart,** all of it is as before.

**The UPDATING FIRMWARE screen and the panel's interrupt:**

- **s212** (app `28728a57…`, the UPDATING FIRMWARE screen):
  - sent from s211, it went to ota_0;
  - sent again from s212, it went to ota_1.
  - The owner saw UPDATING FIRMWARE, but the screen still flickered.
- **s213** (app `95f00c31…`, with the panel's interrupt running while flash is written):
  - sent from s212, it went to ota_0, in 102 s;
  - sent again from s213, it went to ota_1, in 73 s.
  - On the second upload the owner saw it hold still: "sad je dobro. ne blica!"

**Boot errors:** the two `Enqueue URB error` lines at boot are the FLX4 boot drop seen
since s200 (parked).

## Not covered

- **USB audio during an update** still stalls, with both decks stopped. Running code from
  PSRAM (XIP) would keep it going, but that changes the whole image.
- **Playing while updating:** the upload is refused while a deck plays. A deck started
  during the upload is not stopped.
- **The ESP32-C6's firmware** is not updated.
- **Pulling a bundle from a server** (`ruzina-ota`'s pull channel) is not done.
