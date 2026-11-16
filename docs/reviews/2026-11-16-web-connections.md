# JC1060: the Web Remote reads four connections at once

Date: 2026-11-16. Branch: `ogranak/jc1060-web-connections`, from `main` at `d4010fc`
(after [soft takeover](2026-11-15-fader-takeover.md)).

## Problem

The Web Remote's server took one connection at a time. It waited up to 2 s for that
connection's request before it took the next. A phone that opened a connection and sent
late held every other request back: one request on s107 took 1.4 s.

## Change

`main/wifi_remote.c`:
- **One task reads up to four connections at once,** with `select()`.
  - Each connection has its own 2 KiB request buffer in PSRAM.
  - A connection is answered as soon as its request (head, then a body up to
    Content-Length) is complete.
  - `ruzina-web` still sees one request at a time, from one task, so `ruzina_web_handle`
    needs no lock.
- **Full:** with all four busy, new connections wait in the listen backlog.
- **Dropped:** a connection that has not sent its request within 2 s, or sent one too
  large, is closed. The 30-second report counts these as `dropped`.

## Evidence

- **Host:** the IDF build has no warnings. The Rust core is unchanged.
- **Device:** s113 (app `9b7576a7…`).
  - First run: at boot it showed `web_remote listen=ok clients=4`.
  - Later run, after a cold power-up: the owner used the page in two tabs at once. The
    log shows 189 requests in about a minute, the longest 63 ms, and `dropped=0`.
  - Underruns 0/0. No USB packet was lost during the test. One was lost earlier in the
    23 minutes since power-up, uncaptured.
