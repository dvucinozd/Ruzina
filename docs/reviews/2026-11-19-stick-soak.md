# JC1060: a 42-minute soak from the USB stick

Date: 2026-11-19. Image: s140 (app `df3ae93c…`), the `--usb-stick` image of `main` at
`3424541`: the stick behind a hub, with 8 KiB reads.

## Setup

- The board's HS port carried a hub without its adapter, with the FLX4 and a JetFlash
  stick (1 GB, exFAT) behind it. There was no SD card.
- The soak DJ (`--soak-autoplay`) played both decks: deck 1 the even tracks, deck 2 the
  odd ones, with the next track loaded when one played out.
- The board was restarted over COM3 (`--mode reboot`), and the log was captured for 42
  minutes. Nobody touched the FLX4 or the stick.

## Results

| | |
|---|---|
| Duration | 2517 s of uptime, 419 000 USB audio fills |
| Track loads | 5: tracks 0 to 3 and 5, as WAV, MP3 and FLAC |
| Failed loads | none |
| Underruns | 0/0 in all 419 reports |
| `packet_errors` / `iso_errors` / `submit_errors` | 0 / 0 / 0 |
| Stick reads | 32 480 reads, 519 680 sectors (254 MiB) |
| Stick read time | 1.88 ms on average for 8 KiB; at most 8.9 ms after the first report |
| Stick recoveries, retries, stuck transfers | none |
| Faults, resets, brownouts | none |
| Longest fill | 5950 µs; 46 908 fills over 3 ms (11.2 %), none of 6 ms or more |

## Fills over 3 ms

The share is higher than in the shorter runs of the same code base. None of those ran
both decks for so long:

| Run | Media | Fills | Over 3 ms | Longest |
|---|---|---:|---:|---:|
| s136, owner's checks | SD | 205 026 | 6.8 % | 5554 µs |
| s137, first stick run | stick | 18 000 | 8.1 % | 4625 µs |
| s140, owner's loads | stick | 8 000 | 4.0 % | 5294 µs |
| s140, this soak | stick | 419 000 | 11.2 % | 5950 µs |

- The owner accepted fills of up to about 6 to 7 ms during scratches and SYNC jumps,
  with no underrun, until the USB stick path (see the [handoff](../HANDOFF.md)). This
  soak stayed under 6 ms.
- Whether the stick adds to the fill (its bulk transfers share the HS controller with
  the isochronous audio) or a long two-deck run does is not settled. An SD soak of the
  same image would separate the two.

## Verdict

The stick holds a long unattended two-deck run:
- no underrun and no USB audio error;
- no read recovery over 254 MiB;
- no fault.

The longer fills are the open point.
