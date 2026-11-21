# JC1060: the Web Remote reports what the engine did

Date: 2026-11-21. Branch: `ogranak/jc1060-web-outcomes`, from `main` at `f2a61fd`.

## Source

An outside review of `main` at `7fa9b84` had eleven findings
([earlier fixes](2026-11-20-keylock-profile-fixes.md)). This change takes three of them,
1, 4 and 5. All three are about the Web Remote's loads and seeks.

## Before

The web task answered a load or a seek as soon as it queued it. The USB task applied it
later and dropped the engine's result.

- **Load (finding 1):** the page got `200 OK` once the load was in its slot. The engine
  could still refuse it:
  - a Play on the FLX4 came first;
  - another load was in flight;
  - the media had gone.
  A second request could also find the slot free before the first one's load counted as
  in flight.
- **LOAD LOCK (finding 4):** the web refused every load onto a playing deck, even with
  LOAD LOCK off. With the lock on, it did not see a Play about to start or a scratch,
  which the engine refuses.
- **Seek (finding 5):** the page got `200 OK`, but the engine did nothing during a
  scratch, in a loop or without a track. A seek carried no track identity, so it could
  land on a track loaded in the meantime. `/status` always showed `load_generations`
  `[0,0]`.

## Now

- **One request at a time:** each deck has one exchange for loads and one for seeks,
  from the web task to the USB task. The web task posts a request with a ticket and waits
  for the answer to that ticket.
- **Withdrawn when not taken:** the USB task takes requests at least every 50 ms while
  the controller is attached. If it does not take one within 250 ms, the web task
  withdraws it, so it never runs later, and answers busy.
- **Loads:** the USB task checks the catalog revision the page listed, then asks the
  engine (`try_load`). The engine applies the same admission as the FLX4's LOAD: LOAD
  LOCK with a Play pending and a scratch, a load in flight from any source, and the
  media. The web no longer has a rule of its own. With LOAD LOCK off, a playing deck
  changes track from the page as it does from the controller.
- **Seeks:** the engine answers whether it moved, and why not: no track, or a scratch or
  loop holding the deck.
- **Load generation:** each deck's generation changes whenever its track changes,
  including a reload of the same track and the media going. `/status` publishes it. The
  page sends it with a seek, and a seek for an earlier generation is refused.
- **What `200` means for a load:** the engine took the load. If the file then fails to
  open, the deck shows as empty in the status.

| Request | Engine's answer | HTTP |
|---|---|---|
| Load | taken | 200 `OK` |
| Load | LOAD LOCK | 409 `Deck is playing (load lock)` |
| Load | other load in flight, or not taken in time | 409 `Library changed or load busy` |
| Load | catalog revision changed, or no media | 409 `Library generation changed` |
| Seek | moved | 200 `OK` |
| Seek | no track | 400 `Deck not seekable` |
| Seek | scratch or loop | 409 `Deck is scratching or looping` |
| Seek | another track since the page looked | 409 `Deck track changed` |
| Seek | not taken in time | 503 `Deck busy` |

The protocol only gains things (`ruzina-web`), so older pages still work:

- a seek takes an optional `generation`;
- `SeekOutcome` has three new outcomes;
- the desktop simulator checks the generation too.

## Tests

- **Engine, loads:** a FLX4 Play between the page's request and the USB task's turn is
  refused by LOAD LOCK. A playing deck is refused with the lock on and loads with it off.
  A second load while the first is in flight is busy. A stale catalog revision never
  reaches the engine.
- **Engine, seeks:** without a track, no track. With the right generation, the deck
  moves. With another generation, inside a loop, or after the same track is loaded again,
  the deck does not move.
- **HTTP:** each answer maps to its status. A request that the USB task does not take is
  withdrawn.
- **Page:** a seek carries the deck's generation from the status.

## Cost

While it waits, the web task answers no other request. The wait is a few ms, and at most
250 ms when the USB task is stuck. The web task is not on the audio path. Finding 6, slow
sends delaying other clients, is still open.
