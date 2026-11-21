# Rust M2.5 development handoff

**Current state:** the dated entries below, newest last, down to **Images in this log**
and **Open**. Which image the board keeps for play, and the last one measured, is only on
the [status page](DEVELOPMENT_STATUS.md#the-boards-images), with the current summary.

**2026-11-03: [two MP3 decks play clean](reviews/2026-11-03-dual-mp3-opt3.md).**

- **What fixed it:** the decode crates are built at `opt-level = 3` (commit
  `521172a`). On the JC1060 the owner played two MP3 decks together for about
  59 s with 0 underruns and heard it clean.
- **USB audio follow-up (2026-11-04):** the image with the USB audio fix is
  owner-accepted after about 5 minutes with seeks. A lost isochronous packet
  is now only counted, and the recovery budget refills after clean streaming.
- **SD reads (2026-11-04):** [multi-sector reads](reviews/2026-11-04-sd-multisector.md)
  of up to 16 sectors, through a read-ahead in PSRAM. Next to a FLAC deck the
  MP3 frame walk now finishes in 2.2 s instead of 51 s. The worst cold MP3 seek
  is 55 ms instead of 5.4 s. Underruns 0/0, `packet_errors=0`, and the owner
  said the run was fine.
- **Flash QIO 80 MHz (2026-11-04):** [the flash mode change](reviews/2026-11-04-flash-qio80.md)
  from the bring-up DIO 40 MHz cut the longest USB audio fill from about
  5.5 ms to 3.3 ms. Fills over 3 ms went from 967 to 4. Seeks also prime about
  twice as fast. The owner checked by ear: "sve OK".
- **PSRAM 200 MHz (2026-11-04):** [the PSRAM speed change](reviews/2026-11-04-psram-200m.md)
  brought the longest USB audio fill under the 3 ms target: 2.8 ms, with no
  fill over 3 ms in about 5 minutes. The full PSRAM memory test runs at every
  boot. The owner checked by ear: "sve OK".
- **Soak (2026-11-04):** [a soak DJ diagnostic](reviews/2026-11-04-soak-autoplay.md)
  kept both decks loading and playing unattended. The first 40-minute run
  showed underruns only right after an automatic load and Play. A load now
  starts with the seek cushion. The next 35-minute run had 16 automatic
  loads, 0/0 underruns, a longest fill of 2.8 ms and no packet errors.
- **Untagged VBR MP3 seeks (2026-11-05):** [they now resume through the frame index](reviews/2026-11-05-untagged-vbr-seek.md),
  sample-identical to a linear decode on the host. On the board, seeks were
  checked for regressions, and the owner said they were fine ("Sve OK").
- **Loops (2026-11-05):** [seamless loops from a played-audio history](reviews/2026-11-05-loops.md)
  in PSRAM, with a 5.8 ms crossfade at the wrap. On the FLX4 the owner heard the
  section repeat with no click. 4 BEAT/EXIT, IN, OUT, halve, double and Beat Loop
  pads work, and Shift + 4 BEAT brings back the last loop.
- **Scratch (2026-11-05):** [the jog wheel scratches from the same history](reviews/2026-11-05-scratch.md),
  about 24 s back and 1.5 s forward, with 5.8 ms fades. On the FLX4 the owner
  heard no click and the speed follows the hand, also on a paused deck and in a
  loop. 1024 ticks per turn is an estimate.
- **Beat FX and Pad FX (2026-11-05):** [connected to the render graph](reviews/2026-11-05-fx.md),
  with 1.5 MB of PSRAM and short fades so filters do not click. The owner tried
  them on the FLX4 ("Radi"). Turning Beat FX off and Pad FX bank 2 reached the
  device log on 2026-11-19 (s136); Delay and the channel switch have not.
- **Hot cues (2026-11-06):** [Rekordbox cues on load, own cues in a flash journal](reviews/2026-11-06-hot-cues.md)
  on a new 1 MB partition, written only while no deck plays. Writes work on the
  board, longest 11.7 ms, and the owner saw a cue come back after a reset.
- **Controller LEDs (2026-11-06):** [pad modes, Sync, PFL, Smart buttons and Beat FX On](reviews/2026-11-06-controller-leds.md)
  now follow the deck and mixer as on the native firmware. The owner confirmed them on
  the FLX4; the run had no track loaded.
- **Soak with every control (2026-11-06):** [45 min with 626 scripted gestures](reviews/2026-11-06-full-control-soak.md),
  0/0 underruns, longest fill 4466 µs, under the 5 ms limit. Nobody listened to it. Three
  defects from a late Codex review of #94 to #96 are fixed in the same branch.
- **Deck screen (2026-11-10):** [Slint deck screen with directly drawn waveforms](reviews/2026-11-10-deck-screen.md),
  a diagnostic (`--slint-ui`, `--fill-iram`). At 30 frames a second with the soak DJ:
  longest fill 4505 µs, 0/0 underruns, and the owner saw it smooth. Since 2026-11-11 it
  shows the live decks, with the donor's waveform and browse-knob zoom (s56, owner: "Radi,
  sve ok").
- **Deck screen, continued (2026-11-11/12):**
  - channel meters on the screen and on the FLX4 (#100, #101);
  - touch for PLAY, CUE and the tabs (#102);
  - [beat sync](reviews/2026-11-11-beat-sync.md) with Beat Loop and Beat Jump pad LEDs (#103);
  - [quantize on the beat grid](reviews/2026-11-12-quantize.md) (#104).

  All are owner-confirmed on the board ("Radi"). A 15-minute fill profile (s75) showed:
  - up to 6.0 ms during a scratch;
  - 4.2 ms otherwise, with one exception of 5.2 ms right after a SYNC jump back;
  - underruns 0/0.

  The owner accepted both exceptions until the USB stick media path.
- **Lane marks (2026-11-12):** [Hot Cues, the loop and Quantize on the deck screen](reviews/2026-11-12-lane-marks.md):
  - Hot Cue lines and flags;
  - the loop's ends, with shading that grows from Loop In;
  - a Q label for Quantize;
  - Shift + browse zooming three steps.

  Owner: "Radi".
- **Library page (2026-11-12):** [the Library page on the deck screen](reviews/2026-11-12-library.md):
  - the catalog as a list of 12 rows around the selection, with D1/D2 tags;
  - browse (Shift + browse ten rows) moves the selection only while the page is open;
  - a tap selects a row, and LOAD on the controller or the screen loads it.

  Owner: "Radi".
- **Hot Cues page (2026-11-12):** [the Hot Cues page on the deck screen](reviews/2026-11-12-hot-cues-page.md):
  - each deck's eight pads, A to H in the pads' layout and colours, with the cue times;
  - a tap sets or recalls a cue, as the pad does;
  - DELETE then a tap clears a cue.

  Owner: "Radi".
- **Beat FX by touch (2026-11-12):** [Beat FX by touch](reviews/2026-11-12-fx-touch.md):
  - a tap on the FX panel opens a BEAT FX page: effect, beat, channel, a depth bar,
    FX ON and CLOSE;
  - each tap goes through the controller's Beat FX events.

  Owner: "Radi".
- **Master Tempo (2026-11-13):** [Master Tempo](reviews/2026-11-13-master-tempo.md):
  - a tap on MT toggles it (`DeckExtAction::MasterTempo`);
  - a JC1060 WSOLA keylock keeps the key; the shared donor keylock sounded distorted.

  Owner: "Radi". The grain search is spread over each hop. Master Tempo alone peaks at
  about 6.2 ms. With a scratch on the other deck it reached 6.7 ms (s83, 0/0
  underruns), and 44 % of fills ran over 3 ms.
- **Continuous SYNC (2026-11-13):** [continuous SYNC, and the keylock's stack fix](reviews/2026-11-13-continuous-sync.md):
  - a synced deck follows the other deck's tempo fader and tempo range until its own
    moves;
  - the keylock's raised-cosine table no longer passes through the USB task's stack,
    which had fallen to 872 bytes free.

  Owner: "Radi". s85 kept 2832 bytes of free stack for 15 minutes.
- **Master Tempo stops (2026-11-13):** [Master Tempo stops that skip nothing](reviews/2026-11-13-mt-phase.md).
  A keylock stopping on the same source (a scratch, Master Tempo off) used to skip
  about 30 ms ahead. The reader now steps back over its lag. Owner: "Radi". SYNC
  against a Master Tempo deck checks within −12 to +7 ms, the grain search's jitter.
- **Settings page (2026-11-13):** [the Settings page](reviews/2026-11-13-settings-page.md):
  - the decks' sources, buffers and underruns;
  - the mix's played time and limiter;
  - the uptime and the firmware's version and build.

  Owner: "Radi".
  - s87's 15 minutes again showed one 21.6 ms fill, right after a 46 ms Hot Cue journal
    save during the soak DJ's Pause All, as on s80. Underruns were 0/0.
- **Hot Cue saves without long erases (2026-11-13):** [Hot Cue erase-ahead](reviews/2026-11-13-hot-cue-erase-ahead.md):
  - the ~21 ms fills and the lost USB packets both came from journal flash writes;
  - the journal now skips erasing blank sectors, and erases stale spares ahead of time
    while no USB audio flows.

  s88: 18 saves of 5 to 24 ms, longest fill 5.9 ms, one lost packet in 15 minutes
  (5 on s87).
- **CI (2026-11-13):** the legacy workflow now builds the deck screen image:
  - host tests and clippy with `idf-soak-autoplay idf-slint-ui`;
  - target clippy and build for it, with and without `idf-uac-profile`;
  - an IDF job per image, `--soak-autoplay --slint-ui --fill-iram`, plain and with the
    fill profile.

  Turning it on found two things the board images had hidden:
  - with the deck screen, a controller test still expected the old C Library actions;
  - host clippy flagged the screen-only texts as dead code.
- **UART0 service link (2026-11-13):** [JC1060 UART0 service link](boards/JC1060_UART0_SERVICE.md):
  - the console is UART0 on CN2 (connector 10) at 921600 baud, COM8 on the owner's PC;
  - USB Serial/JTAG (COM3) keeps an output-only copy of the app log; the `R` restart
    works only on COM8;
  - flashing over UART0 works: hold BOOT, press Reset, then `uart0-flash.py` at 460800
    baud, checked by the chip's MD5.

  This prepares the USB stick on the FS port.
- **USB stick (2026-11-13, not on main):** branch `ogranak/jc1060-usb-stick` adds
  `--usb-stick`:
  - the host on HS and FS;
  - a SCSI Bulk-Only reader;
  - the media owner reading the stick under the SD contract.

  Both builds compile. On 2026-11-19 it moved behind a hub on the HS port and works on
  the board (s137, see below).
- **Latin letters (2026-11-13):** [Croatian and other Latin letters](reviews/2026-11-13-latin-glyphs.md)
  on the deck screen. Slint embeds only ASCII and literal characters, so an invisible
  text now carries Latin-1, Latin Extended-A and typographic quotes (+641 KB). Owner:
  "Radi".
- **Hot Cue spares erased at boot (2026-11-14):** before the screen and USB audio, the
  journal erases its stale spares ([follow-up](reviews/2026-11-13-hot-cue-erase-ahead.md#follow-up-erase-at-boot-2026-11-14)).
  s93: 5 sectors in 655 ms at boot; then 14 saves of 4 to 23 ms, longest fill 6.1 ms,
  `packet_errors=0` in 15 minutes.
- **Master Tempo on the controller (2026-11-14):** [Master Tempo on the controller](reviews/2026-11-14-mt-on-controller.md).
  Holding Shift + BEAT SYNC for a second turns the deck's Master Tempo over; a short
  press still steps the tempo range, now on release. Owner: "Radi".
- **Smoother Library (2026-11-14):** [a smoother Library page](reviews/2026-11-14-library-smooth.md).
  - The page is made once and keeps one model.
  - The list turns a page at a time.
  - After a Slint frame over 30 ms the screen waits two refreshes, so the decoders get
    their core.
  - Slow frames are logged with their areas (events 23 to 26).

  On s97 a run of 110 ms frames had starved a freshly loaded deck (57 underruns). On s100
  there were 0 underruns, 0 lost packets and a longest fill of 5.7 ms while browsing.
  Owner: "Radi".
- **C6 link probe (2026-11-14):** [the ESP32-C6 link, a first probe](reviews/2026-11-14-c6-link-probe.md).
  - ESP-Hosted 2.11.7 is vendored and only built with `--wifi-probe`.
  - The C6 answers over SDIO slot 1 (GPIO14 to 19, reset 54) and scans Wi-Fi while the
    deck plays.
  - s101: 28 scans in 15 minutes, underruns 0/0, `packet_errors=0`, longest fill 6.1 ms.
  - The C6 runs ESP-Hosted 2.3.0 and needs an upgrade.
- **The deck's own Wi-Fi (2026-11-15):** [the deck's own Wi-Fi and a status page](reviews/2026-11-15-softap-status-page.md).
  - `--wifi-probe` now starts an open access point, `Ruzina-XXXX`, and a page at
    192.168.4.1 that shows both decks.
  - s103 and s104: the owner's phone showed the decks live. Underruns 0/0,
    `packet_errors=0`, longest fill 6.2 ms.
  - The C6 stays on its firmware for now.
- **Web Remote, PLAY and CUE (2026-11-15):** [the Web Remote, PLAY and CUE](reviews/2026-11-15-web-remote-play-cue.md).
  - The deck's network is WPA2 (`CONFIG_RUZINA_WIFI_PASSWORD`, `12345678` by default),
    shown on the Settings page.
  - `ruzina-web` serves the Web Remote. PLAY and CUE take the touch path; other controls
    answer 503.
  - s106: owner "sve je ok". Underruns 0/0, `packet_errors=0`.
- **Web Remote, titles and library (2026-11-15):** [titles and the library](reviews/2026-11-15-web-remote-library.md).
  - The Web Remote shows each deck's title and duration, and lists the catalog from its
    own copy.
  - It loads a track through the same admission as LOAD, without moving the screen's
    selection.
  - s108: owner "sve je uredno".
- **Web Remote, mixer and SYNC (2026-11-15):** [the mixer, SYNC and seeks](reviews/2026-11-15-web-remote-mixer.md).
  - Every control on the page works. Those other than PLAY and CUE go to the mix as
    controller events (`WEB_EVENTS`).
  - The status shows the real mixer and SYNC (`MIX_VIEW`).
  - A seek moves the deck as a beat jump does.
  - s109: owner "sve radi ok". Underruns 0/0, `packet_errors=0`.
- **Wi-Fi in the deck image (2026-11-15):** [Wi-Fi and the Web Remote in the deck image](reviews/2026-11-15-wifi-in-the-deck-image.md).
  - `--slint-ui` builds the deck's Wi-Fi and Web Remote in; `--no-wifi` leaves them out,
    and `--wifi-probe` is gone. The probe names are replaced (`wifi_remote.c`,
    `CONFIG_RUZINA_WIFI_REMOTE`).
  - CI covers `idf-web-remote`.
  - The boot's `Enqueue URB error` is the FLX4's drop and second enumeration, which
    happens on every boot, with or without Wi-Fi.
  - s110: owner "radi".
- **Soft takeover (2026-11-15):** [soft takeover and the FLX4 at boot](reviews/2026-11-15-fader-takeover.md).
  - After the Web Remote moves a fader, the FLX4's fader counts again only once it
    reaches or passes that value.
  - The FLX4's drop at boot belongs to resetting the P4 alone; a cold boot enumerates
    once.
  - s111: owner "radi sve ok".
- **Web Remote connections (2026-11-16):** [four connections at once](reviews/2026-11-16-web-connections.md).
  The server reads up to four connections with `select()` and answers each once its
  request is in. A slow phone no longer holds others back. s113: two tabs, 189 requests,
  the longest 63 ms.
- **SD failure keeps the Wi-Fi (2026-11-16):** [a failed SD card leaves the Wi-Fi's slot alone](reviews/2026-11-16-sd-slot-teardown.md).
  The SD path removes only its own slot on a failure. Before, `sdmmc_host_deinit()` also
  took the C6's slot, and ESP-Hosted asserted. s116: without the card the network
  shows, with it the decks play.
- **Deck name on the page (2026-11-16):** [the Web Remote names its deck](reviews/2026-11-16-deck-name.md).
  The status carries `name` (the network's, `Ruzina-AB2D`), and the page shows it in its
  header and tab title. s114: owner "radi".
- **SYNC with Master Tempo (2026-11-16):** [lines up on the tempo, not on a grain](reviews/2026-11-16-sync-mt-line.md).
  - Behind the keylock, the deck's position follows the tempo's line, not the grain
    search's moves (up to about 6 ms).
  - A SYNC with Master Tempo on waits two hops before its jump.
  - s115: owner "Gotovo, radi", phase checks 0, −1 and +1 ms (were −12 to +7).
  - Open: an SD clock failure at the first boot after a flash now also asserts in
    ESP-Hosted's SDIO bring-up.
- **Screen draw speed (2026-11-16):** [the deck screen draws its big frames faster](reviews/2026-11-16-screen-draw-speed.md).
  - Covered pages are hidden, Slint's draw is built at `opt-level = 3` in its own crate,
    and lines go straight into the frame buffer.
  - The vendored Slint renderer reads glyphs at their own size row by row.
  - s122: Library page 84 → 70 ms, Settings 50 → 45 ms, underruns 0/0.
  - Slow frames now log the preparation, line drawing, copy, write-back and allocations
    (events 27 to 29).
  - Open: Slint's scene preparation is still half of a big frame.
- **Screen task's allocation pool (2026-11-16):** [the deck screen's small allocations come from a pool](reviews/2026-11-16-ui-pool.md).
  - The screen task's allocations up to 256 bytes come from free lists per size class
    in a 1 MB PSRAM arena, not IDF's locked heap.
  - s123: an allocation takes about 0.3 µs, not 2.5 µs. Library page 70 → 61 ms,
    Settings 45 → 41 ms, underruns 0/0. Owner: "Radi".
- **Playlists and covers (2026-11-16/17):** [playlists in the Library, and each deck's cover](reviews/2026-11-16-playlists-covers.md).
  - The browse press opens LIBRARY from any page, and Shift + press opens it at the
    playlists. In LIBRARY the press opens a folder or playlist, and on a track it goes
    back to the Overview. Shift + press or BACK goes up; LOAD loads a track or opens a node.
  - Each deck shows its track's 56 px cover under CUE, from `a1_m.jpg` or `a1.jpg`.
  - The shared JPEG decoder's largest thumbnail is now 64 px.
  - s127 on the owner's export (25 tracks, 1 playlist): playlists, covers and the browse
    press work. Owner: "Radi".
- **Settings (2026-11-17):** [settings the deck keeps](reviews/2026-11-17-settings.md).
  - The Settings page sets LOAD LOCK, each deck's jog (VINYL/CDJ), the brightness
    (10 to 50 %, capped while the board runs from the PC's USB) and the MAIN trim.
  - They live in a new 16 KiB `settings` partition and are saved while no deck plays.
  - The `ruzina-settings` store now appends records within a sector, so a save erases
    nothing. s128's erasing saves had flashed the screen once and lost 2 USB packets.
  - s129: 5 saves, `packet_errors=0`, settings back after a cold boot. Owner: "Radi".
  - Review fixes (s130): the app main task saves, card or not; a save that would erase
    waits for USB audio to stand still. s130: saves of 9 and 2 ms, 30 % back after a boot.
- **SD tried again (2026-11-18):** [the SD card is tried again when it fails to take its clock](reviews/2026-11-18-sd-retry.md).
  Up to three tries, removing and adding back slot 0 alone, the third at 20 MHz. s132:
  `attempts=1` at 40 MHz on a normal boot, owner "Radi"; the retry itself waits for a
  failing boot.
- **Settings page rows (2026-11-19):** [the Settings page keeps its rows](reviews/2026-11-19-settings-model.md).
  Its two columns keep their models and change only the rows that differ. s133: Settings
  frames over 30 ms went from about 3.5 a second to 6 in 46 s; owner "Radi".
- **Read-ahead in the shared crate (2026-11-18):** [the SD read-ahead moves into
  `ruzina-media-io`](reviews/2026-11-18-read-ahead-crate.md), behind its `read-ahead`
  feature. The legacy core uses it from there. s134: both decks and the Library behave
  as before; owner "Radi".
- **Memory cues (2026-11-17, tested 2026-11-18):**
  [Rekordbox memory cues](reviews/2026-11-17-memory-cues.md).
  - Red marks on the lane and overview, a MEMORY list on HOT CUES, and the FLX4's
    SAMPLER pads (read by the board, not the golden profile) call cues 1–16 as a CDJ's
    CALL does.
  - s135, rebased on s134: SAMPLER pads 1–4 and 7 on deck 1 and 1–2 on deck 2 reached
    the board as `PadAction`, `packet_errors=0`; owner "Radi".
  - Review fix: SAMPLER events keep their place among a transfer's other events. s136:
    pads, Play and both jogs, 67 reports with `packet_errors=0`; owner "Radi".
  - s136 short checks (2026-11-19): Shift + pad reached cues 9–12 on both decks, Beat
    FX went on and off, and Pad FX bank 2 pads 1–4 played; `packet_errors=0`; owner
    "Radi".
- **USB stick behind a hub (2026-11-19):** [music from a USB stick behind a hub](reviews/2026-11-19-usb-stick-hub.md).
  - `--usb-stick` now keeps the host on HS with hub support. The FLX4 and the stick share
    a hub, and the PC keeps the FS port for power and COM3.
  - The hub runs without its adapter, because with it the hub drove 5 V back up its cable.
  - s137, with no SD card: the stick mounted, and both decks loaded and played from it.
    9 reports had `packet_errors=0`; owner "Radi".
  - Review fixes: the stick closes only once no command uses it, and a timed-out transfer
    is never reused. s138: pulling the stick out mid-track stopped media access cleanly,
    and the board kept running; owner "Radi".
  - The stick put back: the Library shows MEDIA REMOVED while it is out, and its return
    restarts the app. s139: it mounted again and both decks played; owner "Radi".
- **Stick reads in one transfer (2026-11-19):** [one transfer per high-speed stick read](reviews/2026-11-19-stick-8k-reads.md).
  A high-speed stick takes each 8 KiB read in one data transfer instead of two, and the
  owner's report shows the read times. s140: track 1's index walk took 2.24 s, against
  2.39 to 2.51 s before and 2.04 s from the SD card; owner "Radi".
- **Soak from the stick (2026-11-19):** [a 42-minute soak from the USB stick](reviews/2026-11-19-stick-soak.md).
  s140, soak DJ, no SD card: 0/0 underruns and no USB audio error in 419 reports; 254 MiB
  read from the stick with no recovery. The longest fill was 5950 µs, and 11.2 % of
  fills were over 3 ms, against about 7 % in shorter SD runs. The owner did not want an
  SD soak to compare.
- **The stick image is the board's image (2026-11-19):** the owner chose the
  `--usb-stick` build as the one kept on the board, with the Rekordbox export on the
  stick and no SD card. The SD build stays in CI and for comparisons.
- **Even fills (2026-11-19):** [three even render blocks per USB audio fill](reviews/2026-11-19-even-fills.md).
  The mix renders 96-frame blocks, exactly three per 288-frame fill, instead of 128-frame
  blocks that made every fourth fill render three. s142 against s140 over the same
  159 000 fills: fills over 3 ms fell from 11.3 % to 3.75 %, and the longest from 5950 to
  5235 µs, with 0/0 underruns and no USB audio error; owner "Radi". The rest follow the
  tracks playing, likely PSRAM contention with the other core.
- **Faster render (2026-11-19):** [what Beat FX costs a fill, and a faster render](reviews/2026-11-19-fx-fill-cost.md).
  The profile now sorts fills by class (plain, each Beat FX, FX tail, Pad FX, scratch or
  Master Tempo) and times the render graph at boot. The graph was computing, not waiting
  on PSRAM: cross-crate DSP calls a frame. The per-frame DSP is inlined, per-block values
  are worked out once a block, and the render runs each deck's stages over 16-frame
  chunks; the audio is bit-identical. Graph 1410 → 1008 µs a fill; under the profile,
  fills over 3 ms 8.1 % → 0.61 %, 0/0 underruns (s145, s146, s147). The fill chain's
  stack is now 4016 of its 4096 B budget, through the render.
- **Faster render soak, fill stack (2026-11-20):** [the product image and the fill stack](reviews/2026-11-20-fast-render-soak.md).
  s148 against s142 over the same 159 000 fills: fills over 3 ms 3.75 % → 0.23 %, the
  longest 5235 → 4155 µs, 0/0 underruns, no USB audio error; owner "Radi". The mix now
  builds a graph without effects out of line, so its 1.3 KiB slot is no longer in every
  fill's frame: the product fill chain needs 2704 B, down from 3792 B.
- **Scratch and Master Tempo fills (2026-11-20):** [after the faster render](reviews/2026-11-20-scratch-mt-fills.md).
  The profile splits `scratch_mt` into `scratch`, `master_tempo` and `mt_scratch`. s150, 14
  minutes with the owner switching Master Tempo, 0/0 underruns: a scratch's longest fill
  is 4.2 ms (accepted 6.3 ms before), Master Tempo 5.5 ms with 23 % of fills over 3 ms
  (6.2 ms and over 40 %), both 6.0 ms (7.0 ms). The keylock is the largest cost left.
- **Keylock search spread (2026-11-20):** [Master Tempo's keylock](reviews/2026-11-20-keylock-spread.md).
  A boot bench showed its cost is computing, not PSRAM, and that its grain search ran in
  the first 72 frames of each hop. The search now compares a candidate every 7 frames,
  catching up at the hop's end, so it picks the same start; the audio is bit-identical.
  Render with both decks keylocked: mean 1699 → 1599 µs, longest 2065 → 1770 µs (s153).
  In real play on s153 (10 minutes, 0/0 underruns), Master Tempo fills over 3 ms fell from
  23 % to 18 % and its longest from 5.5 to 5.2 ms; with a scratch, 28 % to 23 % and 6.0 to
  5.6 ms. Master Tempo still needs its exception to the 5 ms rule. The product image s154
  (app `53c2f867…`) peaked at 5.4 ms with Master Tempo on both decks.
- **Keylock search, half the frames (2026-11-20):** [Master Tempo's keylock search](reviews/2026-11-20-keylock-search.md).
  The coarse search compares every 16th frame instead of every 8th, the fine one every
  8th instead of every 4th. On a track-like host signal the joins match as well. Bench:
  keylock mean 642 → 517 µs, render with both decks keylocked 1599 → 1477 µs. Real play
  on s155: Master Tempo fills over 3 ms 18 % → 15 %, but the longest stayed 5.6 ms (6.0 ms
  with a scratch), from a render spike the search does not explain. The owner hears no
  difference. The board runs s155, a profile image.
- **Keylock and profile fixes (2026-11-20):** [from an outside review](reviews/2026-11-20-keylock-profile-fixes.md).
  An outside review of `7fa9b84` had eleven findings; all hold. Taken here: the search no
  longer reaches 3 frames before a run (tempo under 0.51, silent first hop); the profile's
  `master_tempo` and `mt_scratch` go by a keylock actually playing (flag 256), not the
  setting; the Beat FX tail ends for good after 3 s. A bright-tone test showed the fine
  search on every 8th frame joined worse, so it is back on every 4th; the coarse search
  stays on every 16th. Not flashed. Open: Web Remote load/seek outcomes (1, 4, 5), the
  stick's semaphores and reset recovery (2, 3), then 6, 10, 11.
- **Keylock start in the profile (2026-11-21):** [does a keylock start make the longest fills?](reviews/2026-11-21-keylock-start-profile.md)
  Profile builds now flag a fill in which a deck's keylock starts over (flag 512), with its
  own class `mt_start`. s156, 10 minutes, 0/0 underruns: only 18 such fills, with a longest
  of 5.4 ms. Steady Master Tempo reached 5.3 ms, and 5.6 ms with a scratch on the other deck.
  The starts are not the cause. Real play is about twice as slow as the boot bench (plain
  fill 1.9 ms against 1.0 ms), so memory shared with the screen and the stick is the next
  suspect.
- **Web Remote load and seek outcomes (2026-11-21):** [the Web Remote reports what the engine did](reviews/2026-11-21-web-load-seek-outcomes.md).
  Outside review findings 1, 4 and 5. The web task now waits for the engine's answer to a
  load or a seek, and withdraws a request the USB task did not take within 250 ms. Loads
  get the same admission as the FLX4's LOAD, so a playing deck changes track from the page
  when LOAD LOCK is off. Each deck has a load generation in `/status`, which the page sends
  with a seek; a seek for a track the deck no longer holds is refused. Tested on s158
  (app `40688f8c…`) with a phone on the deck's access point:
  - a load onto a stopped deck, and a seek, went through;
  - a load onto a playing deck with LOAD LOCK on, and a seek inside a loop, were refused
    with their reasons;
  - with LOAD LOCK off, the playing deck loaded.
  Underruns were 0/0, and the longest web request took 91 ms.
- **Stick semaphores and Reset Recovery (2026-11-21):** [the stick's semaphores and Reset Recovery](reviews/2026-11-21-msc-lifecycle.md).
  This takes outside review findings 2 and 3.
  - Both semaphores are now static and made before the USB client task starts, with an
    atomic flag the media owner waits on. No bind can go ahead without them.
  - Reset Recovery checks the host's halt, flush and clear, and each control request. It
    returns the first step that failed, not the last step's result. A failed recovery
    gives the stick up instead of letting a read try again.
  - A host test runs the real `usb_msc.inc` against a fake stick, with each failure point.
  - On s159 (app `be416a1f…`, 6 minutes of play) the stick
    bound, opened and read. Its only notice was the usual first-attach UNIT ATTENTION on
    TEST UNIT READY. No Reset Recovery or read retry was needed, and underruns were 0/0.
- **Web server start and deep playlist navigation (2026-11-21):** [the web server's failed start, and Back in a deep playlist tree](reviews/2026-11-21-web-server-and-navigation.md).
  This takes outside review findings 10 and 11.
  - A web server that fails to start now frees its buffers and closes its socket. The
    Wi-Fi report shows the server as starting, listening or failed.
  - Back finds the row it came from in the parent's rows, so it lands right at any depth.
    The old 16-level history is gone.
  - Still open from the review then: 6, slow HTTP sends holding up other clients.
- **Web Remote sends in pieces (2026-11-21):** [the Web Remote sends its answers in pieces](reviews/2026-11-21-web-send-in-pieces.md).
  This takes outside review finding 6, the last one open. Each connection keeps its answer
  until it is sent, and the server sends at most 2920 bytes a pass, without waiting, when
  the socket is ready. A slow client therefore no longer holds back another's controls.
  A connection that has not taken its answer within 5 s is dropped. A host test covers it
  with fake sockets. On s162 (app `4b6044fa…`), tested from a phone, 594 requests came in
  and none was dropped. Underruns were 0/0, and the owner noticed no delay.
- **What slows the fill (2026-11-21):** [what slows the USB audio fill in play](reviews/2026-11-21-fill-contention.md).
  Profile images now record what else ran during each fill: a screen frame, a decoder or a
  stick read. The new `--screen-still` flag stops the screen every other 30 s. On s163
  (app `2ce5c0aa…`, 15 minutes, 0/0 underruns), 88 % of Master Tempo fills over 3 ms
  overlapped a screen frame. With the screen still, 1.7 % went over 3 ms instead of 18.3 %,
  and the longest took 3.9 ms instead of 5.35 ms. A fill with nothing else running costs
  the same with the screen still, so the screen does not leave a slower cache behind. Its
  frames compete for memory while the fill runs. The stick's reads made no difference.
  Next: keep the screen's heavy memory work, and if needed the decoders, out of the fill.
- **Boot screen and screensaver (2026-11-21):** [the boot screen and the screensaver](reviews/2026-11-21-screensaver.md).
  As the donor's: the owner's RUZINA wordmark, its letters lit in turn, for 3 s at boot and
  after an idle wait set on the Settings page (OFF, 2, 5, 10 minutes; 2 by default). A
  playing deck holds it off. A touch or a controller press only wakes it; faders, knobs and
  the Web Remote act too. s172 (app `d0863dcc…`): all of it works by touch and from the Web
  Remote, owner "Sve radi"; the FLX4's wake waits for the controller.
- **The donor's controller profiles (2026-11-21):** [the donor's other controller profiles](reviews/2026-11-21-controller-profiles.md).
  The deck now carries the DDJ-400, Numark DJ2GO2 Touch, Traktor S2 MK3 and Hercules
  Inpulse 500 profiles besides the FLX4's, chosen by VID:PID. It sends a profile's initial
  SysEx and takes its filter flag. The FLX4's SAMPLER trick is limited to the FLX4. The
  parser gained the donor's inverted CC7, latching PFL and native-headphone flag. Audio
  for the Hercules (asynchronous) and the S2 MK3 (UAC2) is still to come. s173 (app
  `fd4d6e1b…`), no controller attached: works, owner "Sve radi". No controller has been
  tested on it yet.
- **Rust palette (2026-11-21):** [the deck screen in the owner's Rust palette](reviews/2026-11-21-rust-palette.md).
  The first step of the screen redesign. It changes:
  - the shell and deck colours;
  - the waveform in three bands (PWV3 whiteness mapped to lows, mids and highs; PWV4
    band mixes blended);
  - the Hot Cues A–H chosen with the owner;
  - the memory cues (now warm white) and the bar markers (now acid green).

  s174 (app `586c7ec6…`) works; owner: "Izgleda dobro".
- **Screen layout (2026-11-21):** [the deck screen's new layout](reviews/2026-11-21-screen-layout.md).
  The second step of the redesign, after the palette:
  - a 48 px icon rail instead of the tabs;
  - lanes of 814×210 with the phase strip's place between them;
  - deck cards at the bottom on every page;
  - the pages narrowed to 952 px.

  s176 (app `72498c59…`): owner "Izgleda dobro"; s175 had a 72 px rail, which the owner
  narrowed. The phase strip followed. It shows both decks' beats between the lanes over
  four master beats, and the PHASE offset in ms. s177 (app `4041be8c…`): owner "Izgleda
  dobro".
- **Images in this log:** which one the board keeps for play is on the
  [status page](DEVELOPMENT_STATUS.md#the-boards-images); this list does not say what is on
  the flash now.
  - s177 (app `4041be8c…`) is the play image flags on the redesign branch: palette,
    layout and phase strip.
  - s176 (app `72498c59…`) is the play image flags on the redesign branch: palette and
    layout.
  - s174 (app `586c7ec6…`) is the play image flags on the Rust palette branch.
  - s173 (app `fd4d6e1b…`) is the play image flags on the controller profiles branch.
  - s172 (app `d0863dcc…`) is the play image flags on the screensaver branch.
  - s162 (app `4b6044fa…`) is the `--usb-stick` image of the Web Remote send branch, with
    no profile. Its firmware is the same as `main` at `830a46b`. It has all 11 outside
    review fixes. In its run it made 42 000 fills with 0/0 underruns, and the longest took
    4.2 ms.
  - s163 (app `2ce5c0aa…`) is the fill contention profile image. It was left on the board
    after its 2026-11-21 run, with s162 to go back for play.
  - s160 (app `efe6975c…`) is `main` at `725488f`. In 8 minutes of play it made 79 000
    fills with 0/0 underruns, and the longest took 5.1 ms (11.5 % over 3 ms).
  - s159 (app `be416a1f…`) and s158 (app `40688f8c…`) are the stick and Web Remote
    branches before they merged.
  - s157 (app `bd390cb0…`) is `main` at `f2a61fd`.
  - s156 (app `d4124d57…`) is the profile image of the keylock start branch.
  - s148 (app `5824136c…`), the kept image before s160, is the `--usb-stick` image with the
    faster render and the stick behind a hub.
  - s154 (app `53c2f867…`) is `main` after the keylock spread.
  - s142 (app `3125b34f…`) is s148 before the faster render, and s140 (app `df3ae93c…`)
    is before even fills.
  - The SD image s136 (app `a666ac6e…`) is from before even fills.
  Each flash keeps its live pre-write backup (`restore-*.bin`) in
  `D:\BoardEvidence\JC1060-20261030-COM3\s<n>-*-flash`.
- **Parked work:** the matched control/opt3 source-compare preparation is on
  `ogranak/jc1060-source-compare-parked`, not for merge.
- **Open:** the current open items are on the [status page](DEVELOPMENT_STATUS.md#open),
  with the latest fill figures (s163, 2026-11-21: Master Tempo fills over 3 ms mostly
  overlap a screen frame; work on that overlap is in progress). The items below are older,
  each with its campaign:
  - 2026-11-05, loops campaign (SD build, [loops](reviews/2026-11-05-loops.md)):
    `fill_max_us` was 3.2 ms in production and
    3.4 ms under profiling while really mixing, over the 3000 µs headroom target, with no
    underruns. The render graph was most of it. The owner stopped there; IRAM was kept for
    when FX add render time;
  - the same period: scratching with both decks playing reached 3.4 ms, the same as
    mixing, with no underruns;
  - the 35 to 40-minute soaks were unattended: no seeks, scratch or EQ
    during them, and nobody listened;
- **Older paragraphs** below are historical checkpoints.

Latest offline implementation: [legacy MP3 in-place reset](reviews/2026-11-03-mp3-in-place-reset.md).
Only legacy dependency resolution uses two pinned local crate copies; three
upstream files change. Safe field resets replace core construction in wrapper
reset/seek; native workspace and normal compiler defaults remain unchanged.
Normal/opt3 builds pass old stack guards, reset frame 16 B. Unit/codec tests
and byte-exact PCM comparisons pass. No COM3 access; s07/history **44** remain.
Review/CI, clean package/archive completion and a fresh explicitly registered
matched-control/opt3 transition chain precede any device work. Existing old
writer bindings/plans cannot authorize these new source/image identities.
Source `a245335` has review 0 issues and three clean source/artifact-verified
packages; the 31466623-byte/405-member archive `fe94bb23...` passes CRC/member/
secondary checks. Exact-source host CI #586 passes; legacy #109 is queued.
Verify that remaining CI gate before preparing any new registered transition.

Latest offline result: [source-budget investigation](reviews/2026-11-03-source-budget.md).
Real producer/decoder shared-time tests and a release benchmark isolate read
pressure and compiler sensitivity; all compared host PCM files match exactly.
Control audit finds 74 extra messages in S03 and reproduces eight unsupported
scratch effects, not media failures. Frozen numerical admissions are retained,
but S03 is not isolated-control acceptance. The opt3 core candidate is rejected
before packaging: constructor frame **39008 > 24576**; independent load-chain
estimate **90544 > 90112**. Normal build passes. Default compiler policy and
native minimum 302 stay unchanged. No device access; s07/history **44** persist.
Next: inspect runtime reset/seek construction and design bounded-stack decoder
initialization; do not register/flash this rejected image or enlarge limits.
Full review raised 1 minor audit-boundary issue; fix `59baed2` has final review
0 issues and four audit tests per OS pass. The 25209794-byte/684-member archive
`b08095f6...` passes CRC/member/secondary checks. New exact-source CI was queued;
do not substitute previous-head CI or local checks for it.

Latest device result: [continuous 42/43 campaign](reviews/2026-11-03-continuous-device.md).
Cold/idle and individual Disturbed/Ben Folds Five MP3 windows were admitted;
both-playing failed after 15.88 seconds, underruns **78/94**, matching owner
crackling. Four admitted checkpoints do not make the failed whole session PASS.
S07 recovery passed all spans and independent 16 MiB equality; the separately
bound single software boot pulse completed, application **NOT_OBSERVED**.
No manual recovery qualification was requested. Tools closed. History **44**,
full `28830827d3e3426bbcd58b9a25d4d2129e28f150bade33fe3c551cd577f93921`.
42/43 plans are consumed; a future transition needs new **44/45** ancestry.
Completed archive `b188a18b...`, 39722727 bytes/119 members, CRC/member/secondary
verification passed. Follow the report's ordered offline source-budget work;
no new flash, native-minimum reduction, APTA/DAC/PFL scope or promotion.
All older current-state/next-step paragraphs below are historical checkpoints.

Latest preparation: [new 42/43 continuous-source registration](reviews/2026-11-03-continuous-registration.md)
and frozen [runbook](reviews/2026-11-03-continuous-campaign.md). Both real-file
offline preflights reconstruct exact current/candidate/return states; 53 tests
pass per OS. Writer review 0 issues, exact source `b92fb23` CI 22/22; all three
separate bindings and dual-copy registration/tools archives pass. No target
access. Next: publish/verify remote and offline bytes, refresh actual topology,
then the registered candidate experiment with automatic s07 exit. Earlier checkpoint details
below retain their historical counts/status; do not reuse old plans.

Current device result: [producer cold/idle and incomplete playback](reviews/2026-11-03-source-supply-device.md).
S01/S02 pass at 1880 us all-fill maximum; S03 ends without load/play and its
initial producer/heartbeat prefix fails frozen analysis. No throughput or
dual-MP3 root cause is established; S04/S05 NOT_RUN. Exact s07 was restored
with full readback, cold boot and owner `sva je ok`; captured playback 9.276 s
has zero errors. All tools closed. Current history **42**, full flash
`28830827d3e3426bbcd58b9a25d4d2129e28f150bade33fe3c551cd577f93921`.
Completed archive 37678594 bytes, `4576676b...`, CRC/members/secondary pass.

Owner decision after recovery: **no more repeated s07 physical cold/listening/
Pause/CUE recovery checks**. Use automatic readback and the
[recovery policy](RECOVERY_POLICY.md); prepare a separately reviewed software
boot action if needed. Preserve old frozen runbooks/archives as historical.
The [continuous adapter/analyzer](reviews/2026-11-03-continuous-adapter.md)
now extends the offline core with immutable checkpoints, whole-journal replay,
owner commands and 39 capture/adapter tests per OS, including one synthetic five-case passive
connection. The separate software boot action is also
[implemented and host-tested](reviews/2026-11-03-software-boot.md)
(46 source-supply tests per OS after the async-admission regressions). Target access is still unregistered. Next:
verify latest CI, then new 42/43 plans, contracts and exact bindings. Combined
CodeRabbit review completed (one minor docs issue fixed, final fix review 0);
exact evidence and dual-copy 64731-byte archive `0a59b8b2...` are retained. No
decoder optimization, extra flash or manual recovery qualification is implied.

The preparation checkpoints below are historical.

Historical registration: [producer campaign registration](reviews/2026-11-03-source-supply-execution.md)
and [frozen runbook](reviews/2026-11-03-source-supply-campaign.md). Candidate/s07
plans are registered against 40/41 predecessors; both offline preflights and
326-member dual-copy archive pass. Current actual s07/history 40/full 7875f650
is unchanged. New producer adapter/analyzer and automatic report-fault stop
have 12 tests passing per OS, with 13 shared-guard WSL tests. Final fix review
has 0 issues and exact `bb96132` CI passes 22/22. Four-file binding `a63241b1...`
and the 48748-byte dual-copy tools archive `949e4831...` are frozen; publication
and both offline preflights precede COM3.
Old normal/profile bindings do not authorize this image. Follow S01-S05 only,
then execute its originally registered exit. That manual s07 exit is superseded
for future campaigns by the owner policy above. Do not edit the frozen runbook.

Active reviewed source: `ogranak/jc1060-source-supply-profile`, draft
[PR #82](https://github.com/dvucinozd/Ruzina/pull/82); read the
[producer/USB measurement contract](reviews/2026-11-03-source-supply-profile.md).
Default-OFF producer profiling separates per-deck service/decode/read/step/push,
owner remainder and USB client/service gaps. 75 diagnostic Rust tests plus
Clippy, 70 normal Rust tests and eight actual C cases pass locally; Windows
Python has 41 PASS/eight explicit C skips, executed separately in WSL. Final
fix review has 0 issues; exact `0c3d86d` CI passed 22/22 with both normal and
diagnostic jobs. Clean normal/diagnostic packages pin `e9ce242`; all 289 source
inputs are byte-identical through the CI/docs-only fixes. Stack estimates are
74640/3120 normal and 75712/3120 diagnostic, with unchanged limits. The
15118202-byte archive `9dce7560...` and every secondary member/hash pass.
Next: freeze a target runbook, register new 40/41 recovery plans, and review/
bind tools that validate producer-profile snapshots before COM3. No new device
access, registration or runtime acceptance. S07/history 40/full 7875f650 remain
the installed baseline. Prior normal-result publication `fd466b3` CI is now
success in both workflows (#564/#87). Device evidence below remains immutable.

Current device state: read the [normal-engine rejection and next ordered work](reviews/2026-11-02-normal-engine-device.md).
Cold/idle max 2397 us passes; each MP3 alone is owner-confirmed with no errors,
but max 3214 us misses the headroom target. Two simultaneous MP3 sources drain
both rings, underruns 4765/5003 and one USB packet error (fill max 4982 us):
registered stop, no PFL acceptance or N03-N08 extension. Exact s07 was restored,
full-readback/cold/owner sound-Pause-CUE confirmed; 14.052 captured playback
seconds have zero errors. Tools closed. Current ancestry **40**, full hash
`7875f650ba7404475e9c26561990851b04c11409b3f309401f6e77a51ddf38a6`.
The 37668270-byte completed archive `500b8398...` has matching CRC/members/secondary.
Next is offline producer/decode/SD/USB-service instrumentation design, then
review/CI and a new 40/41-predecessor diagnostic registration. No new device
work, limit increase, normal promotion or merge follows from this rejection.

Historical preparation (23:21 Zagreb): exclusive-port fix plus real-loop synthetic
tests are published as `08a537b20310347a842a0efdef500c826e4baa59`.
CodeRabbit against `3841b7f` completed with **0 issues**. All 13 normal-monitor
tests pass on Windows/WSL, including four real-loop fake-serial cases; 13 shared
destructive-guard WSL tests pass. Exact CI passed 21/21; binding `a35aa0c4...`
and the 48447-byte dual-copy tools archive `670cdc3d...` are frozen. Publish
this checkpoint before normal device access. Read the latest record rather than the
historical rate-limit paragraphs below. Registered firmware/runbook/plans are
unchanged; native minimum remains 302 and the unit uses rev103 IDF only.

Historical next-campaign preparation: `ogranak/jc1060-normal-engine-campaign`, draft
[PR #81](https://github.com/dvucinozd/Ruzina/pull/81) above #80. Read
[normal execution checkpoint](reviews/2026-11-02-normal-engine-execution.md)
and the byte-frozen [runbook](reviews/2026-11-02-normal-engine-campaign.md).
Source `886127e` has clean normal app `0ba716a4...`, source review 0 issues,
six actual C cases and 21/21 CI. Registration `ec43409` has 21/21 CI; 38/39
candidate/s07-return plans, two offline preflights and dual-copy 312-member
archive pass. Owner confirms RCA MASTER speakers plus controller headphones.
Normal adapter's two minor CLI issues were fixed in `dd245b6`. The next full
review found one major capture-duration issue; `05cff46` fixes it, with four
normal-policy/window tests and 13 shared-guard tests passing. Final fix review
is blocked by CodeRabbit free quota (last error says 34 minutes at 21:34 Zagreb;
retry conservatively after **22:10 on 2026-11-02**). Raw failed reviews/limits
are retained; there is no final 0-issue tool review or execution binding yet.
Latest code CI #557/#80 and publication #558/#81 passed 21/21 each. The 22:10
review found another major issue: connection gaps counted toward capture.
The follow-up records connected seconds and invalidates post-start disconnects,
with seven normal coverage tests passing per OS and 13 shared-guard WSL tests.
Its own review/CI and binding are pending; see the execution record's latest
section. The older rate-limit paragraph above is historical. Superseded
`dd245b6` workflows were cancelled, not passed. Do not modify the frozen runbook.
Next: verify exact latest CI, rerun committed fix review against `f89bb36`,
freeze/archive the new normal binding, publish the handoff, then software ROM
entry and the registered normal-image cases. No normal-image COM3 access or
write has occurred. Accepted s07/current 38/full `1a456836...` remain installed,
all device monitors closed. Do not reuse the old profile binding or treat a
captured-but-short/early-stopped cold/idle/soak window as complete.

Completed physical campaign: read the
[resource-readiness device result](reviews/2026-11-02-resource-readiness-device.md)
and current
[device progress](reviews/evidence/2026-11-02-resource-readiness/device-progress.json).
Candidate and s07-return writes/independent whole-flash readbacks match; current
executed ancestry is 38 and full state is `1a456836...`. Three cold windows each have
7000 callbacks and maxima 2008/1847/1757 us, with zero >3000/>=6000 crossings,
valid calibrations and zero USB/idle errors. Steady adds 9000 callbacks without
new crossings/max. Registered s07 return has independent full readback, cold
boot and owner-confirmed sound/Pause/CUE; 13.566 seconds of played frames have
zero USB/underrun errors. All monitors are closed. Exact full current state:
`1a45683632f9951f1121ad0866fd7b3050d146022bfc6c719be18346848ad4fe`.
Future plans require all 38 executed predecessors. The 37697960-byte completed
archive and every secondary copy/hash pass. Next: independently register the
clean normal engine for bounded MAIN/PFL/two-deck LOAD/seek/CUE/EOF and I02-I13,
including playing-source resource budgets, USB-client runtime stack and
minimum internal/largest heap observations. No normal engine write or product
qualification is implied by idle timing. Keep DAC/APTA/networking out of this
next campaign. Earlier offline-only paragraphs retain their historical gates.

Historical implementation/offline checkpoint: `ogranak/jc1060-resource-readiness`, draft
[PR #80](https://github.com/dvucinozd/Ruzina/pull/80) above #79. Read the
[resource readiness record](reviews/2026-11-02-resource-readiness.md).
Source `d28abd0` prepares both decks' permanent stores fallibly before UAC
activation, preserves SD/USB ownership and leaves media admission separate.
All six host profiles, nine target Clippy configurations, six real C cases,
exact-parent CodeRabbit (0 issues) and clean normal/profile packages pass.
Source/artifact hashes and static RAM/stack estimates are recorded; actual
readiness time, heap/stack high-water and USB timing are NOT_RUN. Both source
`d28abd0` and publication `a2997bd` passed 21/21 CI jobs. Candidate and s07 return
plans are now offline registered against 36/37 predecessors, and both offline
preflights plus a 311-member dual-copy archive pass. Neither plan was executed;
accepted s07/36 predecessors remain unchanged. Registration `79e251a` and
execution-tool source `51c6967` now each pass 21/21 CI. Reviewed guarded
programmer/passive monitor, 13 host tests per OS and the exact-hash binding are
complete; see the record's latest section and tools README. Device access is
explicit and remains unexecuted at this checkpoint. The owner confirms the
song list is visible. The owner subsequently requested agent-performed ROM
entry; one supported native USB software entry succeeded with fresh rev103/MAC/
security identity and no flash write. See the latest record. This substitutes
only the entry method: the chip is currently held in ROM, s07 still installed,
36 executed predecessors. Physical cold boots remain required. The tools
default to offline; raw plan templates are not the
execution path. Next is
the registered three-cold/steady idle resource/timing comparison, followed by
exact s07 return. I02-I13 follow only after
the timing/resource gates permit them. No merge or broader architecture change.

Historical decision checkpoint follows; the implementation above supersedes
its NOT_IMPLEMENTED state while retaining its ownership and acceptance rules.

Current offline decision: `ogranak/jc1060-usb-service-decision`. First read the
[USB service-budget/resource design](reviews/2026-11-02-usb-service-and-resource-decision.md).
The first three next-step audit tasks are complete. USB callbacks serialize;
nominal 24 ms pool depth is not a per-call budget. A 581272-byte per-deck
permanent resource initialization follows catalog PASS while USB already runs;
deck 2 repeats it on first LOAD. Contention is plausible, not proven.
Decision: defer render-ahead ownership/latency migration; implement a fallible,
idempotent both-deck resource preparation gate before UAC prefill/submission
as the next single candidate. Exact states, tests and unchanged gates are in
the design. No firmware implementation or COM3 access in this audit. S07 remains
accepted; recovery ancestry 36. Do not silently combine this with a new task,
queue-size change or broader IRAM placement.

Latest work: `ogranak/jc1060-pack-placement`, PR #78 above #77. Read the
[completed device comparison](reviews/2026-11-02-pack-placement-device.md) first.
Control maxima: 15827/17386/15849 us; IRAM: 14157/13599/15548 us. Both fail;
narrow pack placement is not a qualified fix. Each separate steady window adds
9000 callbacks without new crossings. Exact s07 is restored/readback and cold
boot verified, with owner-confirmed sound/Pause/CUE and 20.112 s captured sound,
zero underruns/USB errors. All monitors are closed. Future plans require **36**
predecessors and reconstructed full state
`7df030bf565c63503a04bfaf2a9cd114632c60e5f357335da9f49d81c2e544e6`.
This is reconstructed from verified predecessor/writes, not an independent
whole post-restore read. The unused control-return is not in the executed chain.
Next is the [service-budget and render/refill design decision](reviews/2026-11-02-post-placement-next-step.md),
PLAN_ONLY. Do not run another placement variant or assume a cache mechanism.
Source/build publication `f0ba8ca` passed 21/21 CI; later evidence CI is separate.
The paragraphs below are historical preparation/results, not current actions.

Historical offline continuation: `ogranak/jc1060-peer-continuity-fixes`. Read the
[peer continuity repair record](reviews/2026-11-02-peer-continuity-fixes.md).
Per-deck revisions preserve the cached mixed suffix and reset only the changed
deck at the next render boundary; media revocation still clears immediately.
The analyzer rejects malformed unsigned record/cause/stage data. These fixes
need fresh exact-image timing; old failed timing evidence is retained below.
No COM3 access; s07 remains the last physically accepted image. Cache-counter
ownership/mapping remains the next offline gate before a bounded experiment.

Latest work: `ogranak/jc1060-uac-cause-profile`, continuing from `62717ce`.
Read the [calibrated device result](reviews/2026-11-02-uac-cause-profile-device.md)
first. [PR #76](https://github.com/dvucinozd/Ruzina/pull/76) contains source
`3e20a8e` (20/20 source CI jobs), clean normal/profile packages and the executed
diagnostic/return plans. Both target calibrations pass. Worst callback 14812 us
has no observed switch delta and 1593–1653 us of covered C-dispatch IRQ time.
The aggregate gate fails; the remainder is not established CPU/cache cost.
Separate steady idle adds 9000 callbacks without new threshold crossings.
Exact s07 was restored/readback/cold-boot verified; the owner confirms normal
sound, Pause and CUE, with zero captured underruns/USB errors. All monitors are
closed. Future plans require **33 predecessors**, reconstructed full SHA-256
`4324fee5c109cc9fe28aebbafa8f5cf2116ee4950afce374be5ef6f81aa2ae68`.
Next is the [bounded cache/compute plan](reviews/2026-11-02-uac-cache-compute-follow-up.md),
PLAN_ONLY; no priority, DSP, gain or native-minimum change.
The paragraphs below retain prior measured checkpoints; their predecessor
counts and not-run states do not supersede the current result above.

Latest continuation: `ogranak/jc1060-uac-callback-profile`, [PR #75](https://github.com/dvucinozd/Ruzina/pull/75).
Read the [callback device record](reviews/2026-11-01-uac-callback-profiling-device.md)
first: source `8a9fe16` has CodeRabbit 0 issues, 20/20 CI jobs and clean normal/
profile packages. Two retained cold callbacks took 13087/13039 us; their valid
exclusive partitions report zero observed scheduler deltas. Wall time still
includes IRQ/cache/observer effects, and runtime hook-liveness calibration is
open. The aggregate >=6000 us gate fails; there is no timing fix yet. A separate
steady window adds 9000 callbacks without new threshold crossings. Exact s07
was restored/readback and cold-boot verified; the owner confirmed normal sound,
Pause and CUE, with zero captured underruns/USB errors. All monitors are closed.
Future plans require **31 predecessors**, reconstructed full SHA-256
`817c75491f1c6f3b8e482b576ce44cec43693458d540685ede6adae392123e8d`.
Next is the [cause experiment plan](reviews/2026-11-01-uac-callback-cause-follow-up.md)
(PLAN_ONLY): calibrate switch counters, then distinguish IRQ/cache/compute
effects before choosing a fix. No priority, gain or native-minimum change.
The older sections below describe prior checkpoints, not new device evidence.

Updated 2026-11-01. Continue from `D:/AI/M1_PCB`, branch
`ogranak/jc1060-review-fixes`. Base is `b35dcd39f4c1ddb9f4b253d6af89110ee68a2335`;
fetched main was `bd52402160af27a3e234f7647d703a847644817b`. Existing planning
and review documents were already uncommitted when repairs began. Inspect Git
status before staging; do not reset or sweep another agent's files.

Current continuation is `ogranak/jc1060-uac-fill-timing`, source `5c579e6`,
[PR #74](https://github.com/dvucinozd/Ruzina/pull/74) above #73. Read the
[phase diagnostic execution](reviews/2026-11-01-uac-timing-execution.md) first:
CodeRabbit 0 issues, 19/19 CI, four clean packages and registered plans are
verified. The later [phase measurement](reviews/2026-11-01-uac-timing-device-results.md)
found 10373 us in callbacks (3/6000 >=6000 us), versus 1844 us initial prefill.
There is no timing fix yet. S07 has been restored/readback and cold-boot verified;
normal sound, Pause and CUE were owner-confirmed with zero captured underruns
and USB errors. All monitors are closed. Future flash plans need all 29
predecessors, including this diagnostic and its return-to-s07 plan.

Read in order:

1. [Repair execution log](reviews/2026-11-01-review-fix-execution.md): actual implementation, commands, logs, package hashes and remaining gates.
2. [Detailed implementation plan](reviews/2026-11-01-review-fix-implementation-plan.md): steps A–H offline; I is later physical acceptance.
3. [Review checkpoint](reviews/2026-11-01-code-review-and-next-steps.md): original defects and failure reproductions.
4. [Current development status](DEVELOPMENT_STATUS.md): separate legacy/native completion levels.

Offline steps A–H are complete: 17/17 IDF packages, host/target tests and final
CodeRabbit (0 issues) pass. PR #73 published the repair source as `b454092` and
passed 19/19 CI jobs. The later [device campaign](reviews/2026-11-01-review-fix-device-results.md)
booted the exact candidate, failed the registered fill timing criterion and
restored s07. Cold boot is verified; the owner confirmed normal sound, Pause
and CUE again. The short capture has zero underruns and USB packet errors;
all serial monitors are closed.

The offline repairs cover descriptor publication, transactional LOAD/PLAY,
identified seek outcomes including EOF/failure, media revocation, USB recovery
and bounded producer work. The execution log is the authority for which gates
passed. Host/build success does not qualify the candidate on hardware.

## Device and publication boundary

Observed hardware: JC1060 NewPanel, ESP32-P4 v1.3/103. Native minimum remains
302. Use only the separate IDF 6.1 backend for this unit. No COM3 access, flash,
RAM execution or gain changes occurred in the repair batch.

Latest accepted playback evidence in the reviewed repository:
[deck-play](reviews/2026-11-01-legacy-deck-play.md), app `95d45203...`, one-deck
44.1 kHz FLAC/MP3/WAV through FLX4 headphones. The [s08/s09 reconciliation](reviews/2026-11-01-review-fix-history-reconciliation.md)
verified all 25 prior plans and a byte-exact 16 MiB live s09 predecessor. The
campaign then wrote the repair candidate and returned to s07 through a new
ancestry-aware plan. Future upgrades must include those two additional plans.

[PR #71](https://github.com/dvucinozd/Ruzina/pull/71) was deck-mix against main;
[PR #72](https://github.com/dvucinozd/Ruzina/pull/72) was deck-engine stacked on
#71. Refresh their state before any publication and merge in dependency order.
The owner subsequently authorized a separate repair PR above #72 and CI on its
published head. See the publication continuation in the execution log and the
[remote repair branch](https://github.com/dvucinozd/Ruzina/tree/ogranak/jc1060-review-fixes).
PR #73 remains unmerged. The subsequent device campaign result is separate from
the published offline validation; passing CI does not close its timing gate.

## Next physical campaign and later development

Start with the [callback profiling plan](reviews/2026-11-01-uac-callback-profiling-plan.md):
the original candidate's aggregate maximum was 12260 us; the instrumented image
has a callback maximum of 10373 us, above the same registered 6000 us bound.
The substage/preemption mechanism is unproven. Initial prefill was below 3000 us
in the instrumented run; do not assume eager initialization will fix it.
After the timing gate passes on a new exact image, validate two-deck MAIN/PFL, CUE/seek/LOAD races,
44.1/48 kHz sources, SD loss and USB reconnect with deadline, transfer, ring,
heap and stack telemetry. Current SD reinsertion requires app restart.

Then wire source cues/analysis and missing performance features in bounded
batches, followed by product UI/settings, C6/networking and OTA. JC8012 needs
its own qualification. APTA remains excluded from the product integration.

[External audio plan](boards/JC1060_EXTERNAL_AUDIO_PLAN.md) records PCM5122 MAIN,
optional TLV320DAC3100 headphones and independent Settings. Separate external
I2C avoids the onboard ES8311/TLV320 address collision at `0x18`. Candidate
pin mapping, wiring, drivers and qualification are still future work.

For earlier legacy campaigns, use their dated `docs/reviews/2026-10-30-*` and
`2026-10-31-*` reports and immutable evidence indexes. Their milestones are
historical observations for named images, not current-image instructions.

## Historical handoff — 2026-10-24 and earlier

The following branch identities and test totals are historical snapshots.
Use the current status and execution log above for continuation.
Host-implemented and merged to `main` on 2026-10-23 (PRs #23-#36, each with
host and ESP32-P4 CI passing on `main`): RP-01/02 (PR #23, `ogranak/m25-rust-parity-plan`),
RP-04 load admission (PR #24, `ogranak/m25-rp04-load-admission`), RP-03 CUE
hold/VINYL-CDJ (PR #25, `ogranak/m25-rp03-cue-jog`), RP-05 memory/loop cues
and source restore (PR #26, `ogranak/m25-rp05-cues`), RP-15/18 reconnect
fencing plus PDB row/EXT cue/loop-range edges (PR #27,
`ogranak/m25-rp15-18-edges`), RP-06 playlist hierarchy (PR #28,
`ogranak/m25-rp06-playlists`), RP-07 part 1: PWV4, memory-cue markers,
artwork references and owner (PR #29, `ogranak/m25-rp07-artwork-pwv4`),
RP-08 MP3 frame index/cold seeks/duration (PR #30, `ogranak/m25-rp08-mp3-seek`),
RP-07 part 2: baseline JPEG decoder and cover/logo display (PR #31,
`ogranak/m25-rp07-cover-decoder`), RP-11 durable settings (PR #32,
`ogranak/m25-rp11-settings`), RP-12 Web/C6 contracts (PR #33,
`ogranak/m25-rp12-web-contracts`), RP-13 signed OTA/readiness contracts
(PR #34, `ogranak/m25-rp13-ota`), the RP-09/10/14/16/17 host contracts
(PR #35, `ogranak/m25-rp09-17-audio-platform`) and the UAC 48 to 44.1 kHz
converter (PR #36, `ogranak/m25-uac-resampler`). See the parity
plan rows for evidence and open items (HTTP adapter for uploads, Web Remote admission/tap
semantics, physical controller/
import/listening acceptance). The sections below still describe the original
baseline.

## Read first and locate the checkout

1. [M2.5 parity audit and acceptance plan](M2_5_PARITY_PLAN.md): 20 open work
   items, pinned sources, implementation evidence, priorities and source links.
2. [Development status](DEVELOPMENT.md) and [product requirements](M1_RUST_PRODUCT_REQUIREMENTS.md).
3. [Architecture](M1_RUST_ARCHITECTURE.md), [validation](VALIDATION.md),
   [controller ADR](adr/ADR-004-controller-profile-platform.md) and
   [OTA ADR](adr/ADR-007-ota-boot-recovery.md).
4. [Board matrix](boards/GUITION_HARDWARE_MATRIX_2026-10-20.md) and
   [first-boot procedure](boards/GUITION_FIRST_BOOT_2026-10-20.md) before hardware work.

Repository: `https://github.com/dvucinozd/Ruzina.git`.
Continuation branch: **`ogranak/m25-rust-parity-plan`**. Audit/planning commit:
`9e1c4344833b808bfaf5305399f09156f59b56a7`; this handoff is a subsequent
documentation-only commit on that branch. Read the branch's current HEAD rather
than treating the earlier audit commit as the complete handoff.

On this Windows machine the active continuation checkout is:

```text
C:\Users\Daniel\.codex\worktrees\m1-guition-bsp\M1_PCB
```

Reuse that worktree after checking its current status. The primary checkout
`D:\AI\M1_PCB` was clean on `main` at
`0f2fb1425ef76f2541a8c76841cfa52178dca589` when this handoff was prepared.
This branch is already checked out in the worktree; do not force it into the
primary checkout or reset either tree. Main was not merged/updated by this task.
Fetch and inspect current refs before relying on these snapshots. In a different
clone, fetch origin and select the continuation branch (create a tracking branch
only if it does not exist locally). Preserve any intervening user changes.

## Frozen donor and target boundaries

- Donor: `https://github.com/dvucinozd/Pajoniiir.git`.
- Original M2.2 foundation: `2c2ec32c253d368765123d7bbf8d37389b790b55`.
- New stable functional reference M2.5:
  `20f1c3f04a615209ae25e9bbdae649d0f5b44e8d`.
- Audited newer donor master: `5f41df1bfec994e325ec1ccdd4b3ebf4ad88061e`.
- Local donor checkout: `D:\Documents\DDJ-FFL4`; use pinned `git show`/diffs
  and preserve its independent changes/branch. Its contents were not modified.
- Stable M2.5, shared-P4 beta images and current master have different scopes.
  The parity plan names which later fixes supply useful regression contracts.
  Do not claim beta features or controller acceptance belong to stable M2.5.
- Targets are Guition **JC1060P470C_I_W_Y** and **JC8012P4A1C_I_W_Y**,
  owner-reported silicon v3.2; configured minimum revision 302. Boards remain
  in transit at handoff. All physical acceptance remains pending.

Native Rust/no_std and Slint remain the architecture. Existing shared host code
already covers two-deck control, Hot Cue journaling, media identity/generations,
sample-exact decoding/seek, loops, scratch, Master Tempo, mixer/FX and independent
MAIN/PFL routing. The audit identifies specific gaps; do not restart these
foundations or copy the C donor tree wholesale.

## Implementation order and acceptance

1. **RP-01/02: parser and profile compatibility.** First correct
   `firmware-rust/crates/ruzina-rekordbox/src/lib.rs` title lookup from index
   18 to 17. Prove it with independent 17/18/19 strings, UTF-16 and fallback/
   malformed fixtures. Preserve existing PQTZ 1..4 -> 0..3 conversion. Then
   complete S3CP v2/v3/v4 through parser, fixture compiler, registry and storage:
   CC7 scaling, NOTE_SELECT, bounded initial output SysEx, LED scaling and filter
   capability. Preserve old bytes and replay behavior, reject malformed/version-
   incompatible records and fence initialization to the current binding. See
   RP-02 for complete bounds; future upload validators must share the contract.
2. **RP-03/04/05 plus RP-15/18: transport/cues.** Implement CUE hold preview,
   release/return and PLAY commit; independent VINYL/CDJ actions; one load-lock
   admission/commit path, including PLAY/LOAD races; full loop cue recall,
   separate memory cues and source restore/local/tombstone precedence. Add
   pending input/LED reconnect fencing and relevant EXT/remote-tap/loop bounds
   regressions. Simulator-only load checks are not shared async admission.
3. **RP-06/07/08: Library/media.** Add ordered nested playlists, bounded artwork
   and fallback, optional PWV4 and predictable cold MP3 seek/duration discovery.
   Preserve sample-exact gapless/reservoir behavior and accepted-output progress;
   decoded duration must remain independent of catalog length/analysis span.
4. **RP-11/12/13: services.** Versioned durable settings, transactional write/
   migration policy, C6 fake-transport/Web contracts and signed A/B OTA with
   strict manifest/identity/TLS/retry/readiness/rollback behavior. Host fault
   injection comes before real networking/flash activation.
5. **RP-09/10/14/16/17: audio/platform.** Host descriptor/packet/clock/teardown
   work can proceed now. Actual UAC/DAC output and product-owner activation
   require exact BSP boot/memory/peripheral/power qualification. Retain later
   donor headphone gain, MT fallback and bounded cache/diagnostics regressions.
6. **RP-19/20 and other P1:** explicitly scoped extra controllers, absolute
   latching PFL, Pro DJ Link, extended UAC and ordinary folder/tag/sidecar work.
   These do not replace unfinished P0 acceptance.

Use separate reviewable commits/PRs for implementation batches. Run targeted
tests and the relevant shared/Slint/target checks before merge. The handoff push
does not merge this branch or authorize a firmware release. The audit defines
acceptance criteria; RP-01..20 remain open until their behavior is implemented
and verified. No test result should be inferred from a requirement table.

## Rename to Ruzina (2026-10-24)

The Rust product is **Ruzina**: crates `ruzina-*`, identifiers `ruzina_*`, the
firmware binary `ruzina-m1-firmware` and the repository `dvucinozd/Ruzina`.
**Pajoniiir** remains the name of the C donor (`dvucinozd/Pajoniiir`, M2.5)
wherever the text means the donor.

Owner decisions on persisted and wire names:

- The media identity domains Rust owns are now `ruzina.track.v1`,
  `ruzina.volume.fat32.v1` and `ruzina.volume.exfat.v1`. No Rust unit had
  stored identities yet, so nothing needs a migration.
- `pajoniiir.hotcue.v2` stays: it is the donor's domain
  (`media_identity.c`), which Rust uses to compute donor Hot Cue identities.
- The Web Remote hostname is `ruzina.local` (only), and the planned media
  sidecar directory is `/RUZINA`. Any donor `/PAJONIIIR` data is never
  overwritten.
- The OTA test bundles are signed for project `ruzina-m1-test`. Production
  projects are board-specific `ruzina-*` names.

Deliberately unchanged: donor URLs, commits and M2.5 quotes, `.ddjota`,
fixture provenance and fixture data (for example the `Pajoniiir test fixture`
vendor strings inside S3CP JSON).

Branding: the owner-supplied artwork is in `firmware-rust/assets/branding`
(emblem, wordmark and a splash composition, with SHA-256 records).

- The emblem is the fallback cover (34x34/40x40 RGB565, generated by a
  standard-library script).
- The M2.5 idle screensaver is ported: `ruzina-ui-model::idle` has the
  timing, wake and input rules (2 min default, playback and recording inhibit
  and count as activity, a wrap-safe clock, a physical wake press consumed,
  a remote mutation delivered).
- The simulator shows the splash on the landscape JC1060 and the emblem over
  the wordmark on the portrait JC8012, with two new reviewed goldens.
- The JC1060 deck screen has had its boot screen and screensaver since 2026-11-21
  ([screensaver](reviews/2026-11-21-screensaver.md)), on the same `IdleScreen` timing.

A case-insensitive search for the old name may show only the donor
references listed above.

## Real Rekordbox export comparison (2026-10-24)

One owner export (Rekordbox 6, 189 tracks, no playlists) was copied off the
stick read-only and compared outside the repository. The Rust side is the
`export_inspect` and `analysis_inspect` examples in `ruzina-rekordbox`. The
M2.5 side is `rekordbox_pdb.c`/`rekordbox_anlz.c` from donor `20f1c3f0`,
built with gcc.

- **Fixed:** every Rust PDB page walk (tracks, names, playlists) followed a
  table past its `last_page`. Rekordbox links the last page to the table's
  empty candidate, then to page 0 and past the end, so the firmware import
  failed with `PageOutOfRange` on this export. `PdbPageHeader::next_in_table`
  now ends the chain at `last_page`, and index pages (`page_flags & 0x40`)
  yield no rows.
- **PDB:** all 189 tracks match M2.5 in title, key, BPM, duration, paths and
  cover. Rust additionally resolves 9 non-ASCII artists and the album table,
  which M2.5 drops.
- **ANLZ:** beat grids, waveforms, color previews and memory cues match for
  all 189 tracks, and hot cues for 188. In the last track, cue D exists only in
  the EXT PCOB; Rust imports it and M2.5 drops it.
- **Still open:** a real export with playlists, and physical USB import on the
  board.

The remaining RP-18 import edges followed the same day:

- A rejected EXT is discarded and the DAT analysis is kept.
- A cut-off trailing PWV4 degrades to PWV3.
- Names are bounded to their own row.

The real export is unchanged by them: no name row out of bounds, and the same
PDB and ANLZ results. A real EXT cut inside PWV4 or PWV3 matches the donor.

## Policies that must survive continuation

- **Headphones/CUE/PFL always use controller UAC.** A ready external stereo DAC
  takes MAIN priority, then compatible controller MAIN, then mute. MAIN switching
  must not reroute headphones. No onboard-codec/external-DAC headphone fallback.
- Keep APTA OFF in Guition/default product graphs. Preserve the original
  `codex/native-apta-consumer` branch; do not merge it. Already selected portable
  adapter/lifecycle work does not close native analysis or accuracy gates.
- `integration-check` only compiles owner combinations. It acquires no board
  resources, registers no heap regions and starts no product tasks. No Guition
  selector currently runs a complete DJ product runtime.
- Board-specific USB instances/roles, SD power/presence, journal partition layout,
  display/touch, DMA/cache and C6 transport need explicit qualified ownership.
  Do not reintroduce retired custom PCB/KiCad artifacts or hard-coded donor pins.
- JC8012 touch-payload distribution rights and exact panel/unit qualification
  remain open. Preserve licence/provenance for imported code, fixtures and assets.
- Recorder stays LATER/OFF; donor `ui_next` is a disabled demo prototype, not
  accepted Slint product scope. No other-board hardware result transfers here.

## Verification at handoff

The audit ran these existing Rust tests successfully at the unchanged firmware
source (`0f2fb1425`); only documentation has changed since:

```sh
cd firmware-rust
cargo test --locked -p ruzina-controller-profile -p ruzina-controller-session \
  -p ruzina-deck -p ruzina-rekordbox -p ruzina-audio-runtime -p ruzina-mixer
```

**315 passed, 0 failed, 0 ignored.** This is not an independent M2.5 differential
suite and does not close the discovered title/profile/transport gaps. Test log
is retained locally outside the repository at
`C:\Users\Daniel\.codex\worktrees\m1-guition-bsp\research\m25-audit-tests.log`.
The audit also verified all eight pinned donor source links against Git objects,
20 RP items, unique requirement IDs, local Markdown targets and whitespace.
For this handoff, the local documentation checker passed all 70 links. No CI
result is claimed for the branch-only documentation push.

Recheck documentation before commit:

```sh
python3 firmware-rust/tools/check_docs.py
git diff --check
git diff --cached --check
```

Use committed Rust 1.95.0 and lockfiles. Full host/Clippy/UI, runtime-owner and
P4 checks are listed in [validation](VALIDATION.md) and the two CI workflows.
They were not rerun for this documentation-only handoff; neither were donor
builds or physical procedures. Hosted CI must be inspected per actual run;
unstarted billing-limited jobs are not PASS.

Physical first boot/recovery, factory backup/readback, flash/PSRAM, display/touch,
USB/SD power and reconnect, UAC/DAC timing, latency, listening, OTA and soak on
both boards remain NOT RUN. M2.5's donor soak waiver never transfers to Rust.
