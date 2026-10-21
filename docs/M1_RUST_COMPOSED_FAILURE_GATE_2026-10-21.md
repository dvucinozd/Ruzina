# Composed Library, source, transport and cue-write failures

Date: 2026-10-21. Based on integrated main `d1013291cf189eb550ccf902689b791eef78287a`.
Implementation branch: `ogranak/composed-failure-policy`.

This follow-up closes ordinary seek-failure reset and persistence-worker hang
paths in the existing legacy integration. It adds source lifecycle fencing and
composed fault injection using the actual firmware product/transport/store
transition code. It does not activate Guition peripherals.

## Failure policy

| Boundary | Result and retained state | Recovery |
| --- | --- | --- |
| Library preflight/stat or persisted-cue read fails before Prepare | Fail the load; preserve the old loaded source. Missing/rejected ANLZ retains the existing analysis fallback policy. | Retry a later load if its dependencies are available. |
| WAV Prepare fails after revoking the old source | Invalidate only that deck's product source, analysis and source-scoped queued commands. Never advertise its old track as playable. | Load a valid source again. |
| Ordinary WAV seek fails | Source owner revokes PCM and releases that deck's handle. Transport reports SourceFailed instead of resetting the chip. Product unloads only the matching deck. | Load a valid source again; the other deck continues. |
| PCM read/decode fault reaches transport/render | Stop only that deck at confirmed terminal position; retain track metadata for a subsequent seek/reload. Render progress remains gated by completed output receipts. | Seek or reload; physical I/O cancellation and timing still require hardware. |
| Media lease disappears or changes | Invalidate every loaded deck belonging to the obsolete lease. Existing source owners revoke their PCM on lease change. | Mount/publish a new lease and load again. |
| Product/transport command queue is full | Preserve the held transaction and staged product state until space is available. On source invalidation, discard only that source's transport/Pad FX broker entries; preserve survivor order and cue snapshots. | Retry after owner progress. Draining obsolete target commands also signals freed space. |
| EOF/fault/playback completion arrives from an old load | Reject by per-deck source epoch. PCM terminal results also require the current stream generation. | No state change to the replacement source. |
| A queued seek is overtaken by a source Prepare | Validate the expected active generation before revoking or seeking the source. | Reject the obsolete seek without touching the replacement. |
| Cue store open/read/write fails | Latch persistence unavailable for this boot. Keep servicing requests and return Failed, including reads queued before a write fault. Do not retry uncertain writes automatically. | Storage diagnosis/repair and a later boot; no automatic in-session requalification. |
| Cue write is not confirmed | Retain its exact pending bank. Do not complete its broker entry as successful. | A failed write remains unconfirmed; power-loss durability is not claimed. |
| New cue edit after persistence fault | Consume the controller delivery as a rejected operation, without committing its staged bank or LED changes. Increment deferred-controls diagnostics; leave transport controls usable. | No cue-write acknowledgement. Persistence remains faulted. |
| Same-track load races with accepted cue edits | Prefer the latest accepted pending/live local bank over an older completed store read, including when its write has just completed. | Preserve accepted cue edits and tombstones across load. |

The persistence worker no longer parks forever after opening, reading or writing
fails. A Library request already queued at the time of a flash fault therefore
receives a terminal failure. Failed cue storage is never treated as an empty bank.
The persistence latch is available in ProductRuntime and the owner dispatch report;
this change does not add an operator-facing error dialog or a new service format.

Source epochs change on successful load and invalidation, independently for each
deck. Already dispatched transport commands retain their original epoch. Product
broker cancellation intentionally preserves durable cue writes and global Beat FX.
The extracted pure transport transitions are used by both the P4 owner and host tests.

## Verification

- **601 host tests**, including **10 new composed failure scenarios** and the
  existing **9 composed owner-health scenarios**. No counts are added twice.
- Workspace formatting, warnings-denied workspace Clippy, explicit warnings-denied
  service test Clippy with `runtime-owners`, and desktop compilation pass.
- **33 P4 check/link configurations** and **24 expected board/feature rejections** pass.
- Interactive UI replay passes both geometries and **38 unchanged pixel goldens**.
- External dependencies and both lockfiles are unchanged. APTA remains separate.

Reproduce the focused host gate from `firmware-rust`:

```sh
cargo clippy --locked -p pajoniiir-service --features runtime-owners --tests -- -D warnings
cargo test --locked -p pajoniiir-service --features runtime-owners --test composed_failures
cargo test --locked -p pajoniiir-service --features runtime-owners --test runtime_owners
```

The host workflow now runs and retains the new composed-failure transcript.
[Retained local evidence](evidence/m1-composed-failure-2026-10-21/README.md)
records source hashes and check output. Hosted CI was not run for this local branch;
the previously confirmed account billing restriction is not marked resolved.

## Remaining gates

These tests inject operation outcomes at owner boundaries; they do not run the
ESP USB/flash drivers, executor scheduling, timeouts or real sink callbacks. The
Library-feature product variant is cross-compiled/linked; its host tests exercise
the common product transitions without a host filesystem or ANLZ worker emulation.

Unexpected ticket/completion shapes that lose owned buffers/handles, mismatched
Library protocol results and panic invariants still retain their reset policy.
Removing those resets requires a proved cancellation/resource-reclamation path;
ordinary filesystem I/O failure must not be confused with a broken ownership
protocol. A stuck driver/operation still needs a separately qualified watchdog
and cancellation design. This package is not a complete executor recovery claim.

Physical flash power-cut/journal recovery, exact-board memory and recovery images,
USB power/cancellation/throughput, display/touch, I2S/UAC output/listening and full
operator UI acceptance remain unrun. Guition profiles stay inactive for these
peripherals. Actual PCM loop execution, audible Beat/Pad FX and MP3/FLAC remain
subsequent implementation work. The DSP target owner still records commands;
its future audible implementation must also fence already-dispatched FX by source.

MAIN preference remains ready external DAC -> compatible controller -> mute.
Headphones/CUE/PFL remain controller-only. No APTA merge, board flash, partition
change or production feature activation is part of this package.
