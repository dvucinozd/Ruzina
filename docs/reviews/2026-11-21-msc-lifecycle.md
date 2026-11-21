# JC1060: the stick's semaphores and Reset Recovery

Date: 2026-11-21. Branch: `ogranak/jc1060-msc-lifecycle`, from `main` at `dd845d5`.

## Source

This change takes findings 2 and 3 of the outside review of `main` at `7fa9b84`. Both are
in the USB stick's driver, `main/usb_msc.inc`.

## The semaphores (finding 2)

### Before

The client task made both binary semaphores (`msc_done`, `msc_bound`) on the first bind,
with `xSemaphoreCreateBinary`.

- **Out of memory:** either one could come back NULL, and the bind still went ahead. With
  `msc_done` missing, the first transfer waited on an invalid handle.
- **A race:** the owner task polled the `msc_bound` handle while the client task was
  writing it, with no synchronisation between them.

### Now

- **Made once, statically:** both semaphores use static storage
  (`xSemaphoreCreateBinaryStatic`). `msc_init` makes them in `ruzina_usb_host_enum_start`,
  before the client task starts, so they cannot fail for want of memory.
- **A published flag:** an atomic `msc_ready` says the semaphores exist. The owner waits
  for it instead of for the handle, and a bind refuses until it is set.
- **Failed binds:** a failed transfer allocation or interface claim leaves the stick
  unbound, and the next attach binds it.

## Reset Recovery (finding 3)

### Before

- **Earlier failures were hidden:** recovery returned the result of its last step, the
  OUT endpoint's CLEAR_FEATURE. A class reset that failed (other than the stick going) or
  an IN clear that failed was overwritten by a later success.
- **The host's results were ignored:** the host pipe's halt, flush and clear calls were
  not checked.
- **The outcome was dropped:** the caller ignored the result of the whole recovery, so a
  read could be tried again on a stick that had not recovered.

### Now

- **The pipe operations are checked:** in this host library, halting a pipe that a stall
  already halted returns `ESP_OK`. Flush and clear need a halted pipe, so after the halt
  any error is a real fault, and the endpoint has not cleared.
- **A stalled control request is a failure:** a stalled CLEAR_FEATURE or class reset
  counts as not recovered.
- **Every step counts:** recovery runs the class reset and both clears, and keeps each
  result. The outcome is GONE if the stick went, else the first step that failed, else OK.
- **No retry without recovery:** a read is tried again only after a recovery that
  succeeded. A recovery that fails gives the stick up, as a transfer stuck in flight
  already did (`msc_stuck`). The read returns at once, and the log asks for the stick to
  be put back, which restarts the app.

## Tests

`tools/tests/usb_msc_test.c` (run by `test_usb_msc.py`) compiles the actual
`usb_msc.inc` against a fake Bulk-Only stick.

- **Bind:**
  - nothing binds before `msc_init`;
  - a failed data-transfer allocation, control-transfer allocation or claim leaves the
    stick unbound;
  - the next attach binds, opens (INQUIRY, TEST UNIT READY, READ CAPACITY) and reads.
- **Recovery that succeeds:** an invalid status leads to a recovery in which every step
  succeeds, then the read succeeds on its second attempt.
- **Recovery that fails:** three cases, each one step failing on its own:
  - the class reset stalls;
  - the host's clear of the IN pipe fails;
  - the OUT CLEAR_FEATURE stalls.

  In each, the read returns `ESP_ERR_NOT_FOUND` after one attempt, and nothing more is
  sent to the stick. Before this change, the middle case recovered "OK" and the read went
  on.

## On the board

Image s159 (app `be416a1f…`), the `--usb-stick` product image of this branch, ran for 6
minutes with both decks playing, browsing and jumps.

- **Stick:** it bound (`in=82 out=01 mps=512`), opened at 1982464 sectors and read
  throughout.
- **Notices:** only the usual UNIT ATTENTION after attach (sense key 6, ASC 28) on TEST
  UNIT READY, which the open tries again.
- **No faults:** no Reset Recovery, no read retry, no crash.
- **Audio:** 59 000 fills with 0/0 underruns. The longest took 4.1 ms.
