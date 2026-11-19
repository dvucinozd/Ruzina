# JC1060: one transfer per high-speed stick read

Date: 2026-11-19. Branch: `ogranak/jc1060-stick-speed`, from `main` at `0edba20`.

## Problem

The stick behind the hub ([review](2026-11-19-usb-stick-hub.md)) was slower than the SD
card. Reading the whole of track 1 on deck 2 (the MP3 index walk, load event 16) took:
- 2.04 s from the SD card (s136);
- 2.39 to 2.51 s from the stick (s137 to s139).

Each media read of 16 sectors (8 KiB) took four USB transfers: the command, two 4 KiB
data pieces and the status. Each transfer is a round trip through the USB client task.
The 4 KiB pieces exist for a full-speed stick, whose OTG 1.1 transfer carries at most
127 packets of 64 bytes (8128 bytes).

## Change

- `usb_msc.inc`: a high-speed stick (512-byte bulk packets) takes the whole data stage
  of up to 8 KiB in one transfer. A full-speed stick keeps the 4 KiB pieces. The
  transfer buffer is 8 KiB. The host library's limit per transfer at 512-byte packets
  is far above that.
- `sd_bridge.c`: every 80 reads, the owner's report adds the reads' average and longest
  time (`read_avg_us`, `read_max_us`), for the SD card and the stick alike.

## Evidence

- **Build:** s140 (app `df3ae93c…`), the `--usb-stick` image. The IDF build has no
  warnings.
- **Device:** booted over COM3 from the held ROM, with the hub, the FLX4 and the stick.
  - The owner loaded track 1 on deck 2 (MP3) and a WAV on deck 1, then other tracks:
    "Radi".
  - Track 1's index walk took **2.24 s**, against 2.39 to 2.51 s before (6 to 11 % less).
    The SD card's 2.04 s is still about 10 % faster.
  - Over 26 reports, an 8 KiB read took 1.65 ms on average (about 5 MB/s). The longest
    was 5.4 ms after the first report, and 89 ms in the first one, as the stick woke up.
  - All 8 audio reports had `packet_errors=0` and underruns 0/0. There was no recovery,
    retry or stuck transfer.

## Next

What is left per read is the command and the status around the data, each a round trip.
Larger reads would spread that cost:
- the read-ahead lines in `ruzina-media-io` (16 sectors);
- the bridge's `BRIDGE_MAX_SECTORS`;
- the bounce buffer.

The SD card would get larger reads too. That change touches internal RAM, so it is left
for its own measurement.
