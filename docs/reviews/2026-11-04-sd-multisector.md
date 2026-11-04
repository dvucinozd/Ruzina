# JC1060: multi-sector SD reads make cold MP3 seeks fast

Date: 2026-11-04. Branch: `ogranak/jc1060-sd-multisector`, stacked on
`ogranak/jc1060-mp3-seek-index` (`702f8c1`).

## Problem

In the [seek-index run](2026-11-04-mp3-seek-index.md) an MP3 deck next to a
FLAC deck seeked in 7.9–11.5 s. Its frame walk had not finished about 50 s
after the load, because every card read was one 512-byte sector through the SD
task. That cost about 1.8 ms per read.

## Step 1: 40 MHz and an index-done event (s14)

- **SD clock:** `sd_bridge.c` now asks for `SDMMC_FREQ_HIGHSPEED` (40 MHz), the
  same as the vendor BSP. The boot line prints the clock it actually got.
- **New event:** `deck_play.rs` reports `INDEXED` (event 16) once a deck's MP3
  frame walk completes. The low bits carry the milliseconds since the load.
- **Device** (`s14-sd-40mhz-*`, app `b6ad3aaf…`, about 30 min run):
  - The clock reached 40 MHz, but throughput stayed at about 840 reads/s, as
    before. The cost is fixed per request (handoff and command overhead), not
    the transfer itself.
  - **Walk:** finished 51.1 s after the load.
  - **Seeks:** the cold MP3 seek took 5.4 s and the FLAC seek 138 ms
    (`prime_max_us`). Once the walk had finished, MP3 seeks were fast.
  - **Audio:** underruns 0/0. `packet_errors=5`, with no restart.
    `fill_max_us=5140`.

## Step 2: up to 16 sectors per request (s15)

- **C SD task (`sd_bridge.c`):**
  - The DMA bounce buffer is now 8 KiB, allocated with
    `MALLOC_CAP_CACHE_ALIGNED`. The driver therefore DMAs straight into it,
    without allocating a temporary buffer for every read and splitting it.
  - `check_buffer_alignment` is checked once at start, and the task does not
    start if it fails.
  - Each request is validated: at most 16 sectors, and inside the card.
  - The boot line prints `dma_direct=1 max_sectors=16`. The periodic line
    prints both requests and sectors.
- **Read-ahead (`read_ahead.rs`, new, no `unsafe`):**
  - **Position:** it sits under the exFAT volume. The volume still reads one
    sector at a time.
  - **Miss:** the layer fetches the sector's aligned 16-sector line in one
    request. The last line stops at the card's end.
  - **Capacity:** 8 lines of 8 KiB in PSRAM, evicting the least recently
    used. That covers two decks, each decoding and walking its index at
    different positions, plus directory and FAT reads.
  - **Errors:** a failed request keeps its line and is retried on the next
    miss.
  - **Staleness:** the card is read-only and one lease owns the transport, so
    lines never go stale. Like the volume's own sector cache, a hit does no
    lease check.
- **Bridge (`idf_media_bridge.rs`):**
  - **Buffer type:** the transport carries heap lines of up to 8 KiB
    (`MAX_READ_BYTES`) under `media-load`. The diagnostics without a heap
    keep a 512-byte buffer.
  - **Copy:** the DMA bytes are copied outside the critical section, which
    masks interrupts on one core and spins the client on the other. A
    `copying` flag keeps `take` and deinit away from the request in the
    meantime.
  - A completion whose length differs from the request is refused.
- **Host tests:**
  - **Five read-ahead tests:**
    - a sequential read costs one request per line;
    - a miss fetches its aligned line;
    - the last line stops at the card's end;
    - eviction is least recently used across 8 interleaved streams;
    - a failed read is retried.
  - **exFAT fixture:** verification (SHA-256 of 8 MiB plus 10 seeks) passes
    through the read-ahead with more than 15× fewer requests.
  - **Feature sets:** tests pass for `idf-media-bridge` 20,
    `idf-exfat-fixture` 29, `idf-media-load` 44, `idf-deck-play` 63,
    `idf-deck-mix` 64+24, `idf-deck-engine` 79+24 and `idf-producer-profile`
    86+24. Clippy passes with `-D warnings` for all of them.
- **Build:** `BUILD_PACKAGE_PASS`.
  - The app is 825920 B (78.8 %), SHA-256
    `baaa8de9ae31c9638a063410fe4573a57de7751629c886840bcc134bf96b72ef`.
  - The load chain is 51792 B (limit 90112) and the UAC fill chain 3040 B
    (limit 4096).
- **Device** (`s15-sd-multisector-*`, about 3 min of streaming):
  - **Test:** deck 1 played FLAC and deck 2 MP3, both playing. Hot cue seeks
    started right after the MP3 load and repeated on both decks: 5 seeks per
    deck.
  - **Owner verdict:** the owner said the run was fine.
  - **Boot:** `real_khz=40000 bounce_bytes=8192 dma_direct=1
    max_sectors=16`.
  - **Requests:** every one carried a full line (`reads=3280 sectors=52480`).
    `denied_commands=0`.
  - **Walk and seeks:**

    | | s14 | s15 |
    |---|---:|---:|
    | MP3 walk finished after load | 51.1 s | **2.2 s** |
    | MP3 seek, worst (`prime_max_us`) | 5448 ms | **55 ms** |
    | FLAC seek, worst (`prime_max_us`) | 138 ms | **87 ms** |

  - **Audio:** underruns 0/0, `packet_errors=0`, no faults.
    `fill_max_us=5483`, with 967 of 30027 callbacks above 3 ms.

## Open

- `fill_max_us` is about 5.5 ms, above the 3000 µs headroom target. It did
  not change with the SD work.
- Untagged VBR MP3 seeks still decode from the start.
- There has been no soak longer than about 30 minutes, and none at all with
  multi-sector reads.
