# JC1060: the Web Remote sends its answers in pieces

Date: 2026-11-21. Branch: `ogranak/jc1060-web-send`, from `main` at `34261eb`.

## Source

This change takes finding 6 of the outside review of `main` at `7fa9b84`, the last one
still open.

## Before

The web server read up to four connections at once, but sent each answer whole, in a
blocking loop. A client that took its answer slowly held the one server task for up to the
socket's 2 s send timeout. Partial sends went back into the loop, so the wait could run
longer still. Meanwhile every other client's controls and status waited.

The audio was never affected: the server runs on the screen's core at priority 2. Only the
Web Remote was held up.

## Now

- **Shared logic:** the connections' code is in `main/web_client.h`, which
  `wifi_remote.c` and a host test both include.
- **Own answer buffer:** each connection keeps its answer in its own buffer until it is
  all sent. Four of the largest asset come to about 50 KiB of PSRAM.
- **Bounded sends:** a send never waits (`MSG_DONTWAIT`) and hands over at most
  `WEB_SEND_SLICE`, 2920 bytes or two full TCP segments. A large answer therefore goes
  out one piece per pass, and the other connections are served between the pieces.
- **One select:** the server selects on the connections still sending their requests for
  reading, and on those with an answer for writing. A complete request is answered at
  once, and its first piece goes out in the same pass.
- **Timeouts:** a connection that has not sent its request within 2 s is dropped, as
  before. A connection that has not taken its whole answer within 5 s is now dropped too.
  This replaces the socket's 2 s send timeout.

## Tests

`tools/tests/web_client_test.c`, run by `test_web_client.py`, includes `web_client.h` with
fake sockets.

- **A slow and a quick client:** the slow client's socket takes 1000 bytes a pass. While
  its 12 000-byte asset goes out over 12 passes, one send call each, a control request from
  a second client is answered whole in the first pass.
- **A fast socket:** a socket that would take everything still gets the answer in
  2920-byte slices.
- **Stuck connections:** a full socket never makes a step wait. A client that stops taking
  its answer is dropped after 5 s, a failed send drops at once, and an unfinished request
  is dropped after 2 s.
