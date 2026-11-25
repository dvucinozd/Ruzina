# JC1060: a stick put back is read again without an app restart

Date: 2026-11-25. Branch: `ogranak/jc1060-stick-reinsert`, from main `e9d796e`.

Until now, a USB stick put back restarted the app (`usb_msc reinserted restart=YES`, see
[the stick behind a hub](2026-11-19-usb-stick-hub.md#putting-the-stick-back)). The media
session could run only once:

- the media owner stopped for good when the stick went;
- the client's load run returned and its task suspended;
- the deck resource gate went to FAILED and stayed there;
- the decks' rings and producers could be published only once.

## What changed

**The media owner loops over sessions** (`sd_bridge.c`, stick builds only).

1. When the stick goes, the owner invalidates the bridge, as before.
2. It waits for the session's client to stop.
3. It logs `rust_sd_owner=QUIESCED ... reinsert=WAITING` and waits for a stick put back.
4. It opens that stick and activates the bridge again.
5. It starts the client's next load run, which mounts the stick and imports the catalog
   from the start.

The client loops on a semaphore instead of suspending. SD builds stop for good as before.

**The bridge activates again** (`ruzina_legacy_bridge_init`).

- The second time, the same read queue is used. First it drops a stale completion whose
  client is gone (`ReadQueue::forget_stale`); a read still active is never dropped.
- It then activates with a new handle. The media session gives a new lease generation, so
  every lease of the last session is stale.
- The deck engine already unloads both decks when its lease goes stale, and it takes a new
  lease from `deck_transport::MEDIA`. The load run now clears a lease the engine never
  took before it puts the new one.

**The decks prepare again** (`deck_resources::prepare_again`).

- The rings published by the first session stay, because the USB task reads them for
  good.
- A later session takes new producers for them, through the gate going from FAILED back
  to PREPARING and then READY.
- Nothing that was never published can be prepared again.

**USB** (`usb_msc.inc`, `usb_host_enum.c`, `uac_driver.inc`).

- A stick put back binds once the last one is closed, and new devices wait for that close.
- A stick given up with a transfer still in flight cannot reuse its transfers, so that one
  still restarts the app.
- A UAC device that attached while the decks had failed waits for them, rather than
  holding a fault.

**The screen** (`slint_ui.rs`, `display_touch.c`).

- While the stick is out, the Library reads MEDIA REMOVED / PUT IT BACK. The SD card keeps
  RESTART NEEDED.
- **s201p showed a race:** once the stick was put back, waveforms appeared over the
  Library. Slint drew the page before the lanes' model said hidden, so a copy the lanes'
  task began at a refresh in between went over it.
- **The fix:** when a page or cover covers the lanes, the screen first publishes them
  hidden. It then waits for the lanes' task to finish any copy it began
  (`ruzina_lanes_settle`, a mutex the task holds through each refresh's copies), and only
  then does Slint draw.

## Host tests

- `a_stale_completion_whose_client_is_gone_is_dropped_for_the_next_medium`
- `an_active_read_is_never_forgotten`
- `a_later_media_session_takes_new_producers_for_the_published_rings`

## On the board

**s201p** (app `35074ecd…`): a profile image without the soak DJ.

- The owner loaded and played tracks, and pulled the stick out and put it back, twice:
  "Radi".
- Each time:
  - `usb_msc unbound`, `rust_sd_client=STOPPED result=-6` and
    `rust_sd_owner=QUIESCED ... reinsert=WAITING`;
  - after the stick was put back, `usb_msc bound ... again=1`, `S6_RUST_USB_STICK` and
    the catalog of 25 tracks again;
  - 0/0 underruns.
- The owner saw waveforms over the Library after putting the stick back. That is the race
  above.

**s202p** (app `89f53247…`): with the race fixed.

- The owner had the Library open while pulling the stick out and putting it back, then
  switched pages and played: "Radi".

## Known limits

- **A long fill at each reinsertion:** it went up to 6–7 ms on s201p, against about 5 ms
  otherwise. The new producers are allocated and copied into PSRAM while USB audio runs. At
  that moment both decks are empty, and no underrun was seen.
- **SD cards:** an SD card put back still needs an app restart.
