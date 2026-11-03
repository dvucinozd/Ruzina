# Matched source comparison: exact-return layout proposal

This continues the [in-place MP3 reset work](2026-11-03-mp3-in-place-reset.md).
The new preparer has no device/execute path. It checks real retained files,
four factory copies, the complete **44 executed** plans and last two actual
execution/readback captures, then proposes four conditional transitions.
Native minimum stays **302**. No COM3 access or new device evidence is claimed.

## Whole-flash recovery finding

The new diagnostic apps are larger than accepted s07. Ordinary replanning of
the smaller s07 image would preserve current diagnostic bytes beyond its erase
end, producing a different whole-flash hash even though the s07 app matches.
This is confirmed on both actual proposed layouts, not only a synthetic test.

The proposal instead restores all baseline bytes of each candidate erase span.
This includes old bytes beyond the s07 file/erase end. A full sector snapshot
is explicitly marked `BASELINE_SECTOR_SNAPSHOT`; `bytes` denotes its full span,
not an artifact length. The original 42/43 image-based writer remains unchanged
and cannot authorize this new payload kind. No writer is implemented here.

| Proposed transition | Actual predecessors required | Resulting full-flash hash |
|---|---:|---|
| control | 44 | `626a75b9b09ed4377777c4d6db7c64c8e9d08092639cbd9f7a52ef06ba531570` |
| exact return after control | 45 | `28830827d3e3426bbcd58b9a25d4d2129e28f150bade33fe3c551cd577f93921` |
| opt3 | 46 | `17a4ae5e96677a9e31f309bd7423458a2d557c6e3bab2bd199cf3e7aa8a50eed` |
| exact return after opt3 | 47 | `28830827d3e3426bbcd58b9a25d4d2129e28f150bade33fe3c551cd577f93921` |

Return snapshot totals are **856064 B** after control and **847872 B** after
opt3. Both are bounded to the original candidate's three erase ranges. The
expected factory-to-state replay proves exact 16-MiB equality after each return.
It is offline reconstruction, not live full-before/full-after readback.

## Prepared evidence and limits

[Proposal](evidence/2026-11-03-source-compare/proposal.json),
[control](evidence/2026-11-03-source-compare/control-plan.json),
[control return](evidence/2026-11-03-source-compare/control-return-s07-plan.json),
[opt3](evidence/2026-11-03-source-compare/opt3-plan.json) and
[opt3 return](evidence/2026-11-03-source-compare/opt3-return-s07-plan.json)
retain exact identities/ancestry. All are **LAYOUT_ONLY_UNBOUND_NOT_EXECUTABLE**,
NOT_RUN, without runnable command templates. There are still **44 executed**
plans; 48 is only the conditional final count if every proposed transition
actually completes. The same s07 hash repeating does not satisfy an ancestry
gate or prove a device state.

The [runbook](2026-11-03-source-compare-runbook.md) binds bounded cases, actual
readiness, independent control audit, unchanged stop/resource gates and
automatic exact return. It requires new source/image/capture/boot identities
and reviewed bindings. Owner policy still removes repeated manual s07 cold/
listening/Pause/CUE recovery qualification. No automatic retry or continuation
of a failed write/session is authorized by this proposal.

Five tests pass per OS: smaller-image stale-sector regression and exact return,
tampered saved bytes with a refreshed digest, escaped payloads, command/link/
range drift, missing spans, unknown writes outside candidate ranges, and CLI
rejection of `--execute` before creating output. The first four historical
plans predate embedded prefix lists; their original hash links are verified,
and every present embedded chain must match. Original historical files are
unchanged. Real-file preparation verifies all 313 inputs per matched image,
all artifacts, original s07 bytes, four factory copies and all 44 saved restores.

Remaining firmware legacy CI was still queued when preparation began. That
check, exact new tool review/CI, dual-copy archive/publication and new verified
execution/capture/boot adapters remain separate gates. No local layout/test
result grants hardware, playback, PFL, reconnect or soak acceptance.

## Next implementation

Implement a new adapter that explicitly proves full sector-snapshot rollback,
complete actual ordered ancestry and exact source/image/package/override bytes
before constructing immutable input for the unchanged shared no-reset writer.
Require fresh live chip/MAC/security/RDID/full-before equality and existing
span/full-after checks. Do not generalize old frozen bindings to new plans.

Then add new per-leg continuous capture and return-boot contracts using the
new app/ELF/counters, preserving framing/readiness/freshness/stops. Apply control
audit to isolated acceptance, retain failed cases, and validate the complete
session. Only reviewed, tested, CI-verified and separately bound tools may be
considered for the physical matched comparison.
