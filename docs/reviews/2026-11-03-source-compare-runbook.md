# Matched producer control / opt3 proposal, executed prefix 44

This is an offline layout proposal for the clean source `a245335`. It does
not authorize COM3 access. Firmware legacy CI, reviewed execution/capture/boot
contracts, exact tool bindings, archives and publication still precede a device
experiment. Native minimum remains **302**; the physical unit is rev103 on its
separate pinned IDF path. No APTA, DAC or peripheral work is included.

Use the clean producer control and opt3 packages from the
[in-place reset implementation](2026-11-03-mp3-in-place-reset.md). Both contain
the identical reset/source patch; only core compiler opt-level differs.
Do not substitute the older unpatched image as a compiler-only control.

## Four conditional transitions, no executed-history inflation

| Transition | Required actual predecessors | Target |
|---|---:|---|
| control diagnostic | 44 | matched producer, core `s` |
| control return | 45 | exact original full s07 state |
| opt3 diagnostic | 46 | matched producer, core `3` |
| opt3 return | 47 | exact original full s07 state |

There are **44 executed** plans, not 48. The last four are proposed and NOT_RUN.
The opt3 path requires successful actual execution/readback of the control
transition and its return; a repeated full-flash hash is not a substitute for
the correct complete ordered ancestry. Each diagnostic leg is a separate
bounded passive session. A fault ends its session and triggers the registered
return. Inspect and record that result before deciding to proceed to another
leg; never automatically continue a failed session to manufacture a pass.
An incomplete/failed write needs inspected recovery, not an assumed valid next
full-state predecessor.

## Exact sector-snapshot return

The diagnostic apps are larger than accepted s07. Rewriting only s07's shorter
image/erase span leaves diagnostic bytes in following sectors. A valid s07
app hash alone therefore does not restore the original whole-flash state.

For each leg, retain the known pre-diagnostic bytes of **every candidate erase
span** at 0x2000, 0x8000 and 0x10000. The return writes those full sector snapshots,
including bytes beyond s07's file/erase end. Its payload `bytes` describes the
whole saved span, not an image length. `payload_kind=BASELINE_SECTOR_SNAPSHOT`
must be validated explicitly by a new reviewed adapter. The old image-based
writer's tail checks/42-43 constants do not authorize this proposal.

Return metadata binds the exact diagnostic plan, original full hash, saved
span bytes, current expected bytes and complete proposed ancestry. Independent
factory-to-state replay verifies both returns restore all **16 MiB** to
`28830827d3e3426bbcd58b9a25d4d2129e28f150bade33fe3c551cd577f93921`.
No erase-all, flash-mode/frequency change, native-revision relaxation, new
partition layout or whole-flash factory restore is part of this proposal.

## Measurement contract to implement and review before use

Reuse the continuous capture core's proven framing/readiness structure, with
new explicit source/app/ELF/plan identities for each leg. Do not alter frozen
old contracts or broaden their bindings. Keep cold 45 s, idle 60 s, deck1 180 s,
deck2 180 s and both 60 s, including selection/load setup. Require actual
readiness before each functional timer; retain exact source names/formats and
actual listening replies separately. Preserve full-stream hashes, receipt
events, immutable offsets, checkpoint analysis and final whole-session result.

Maintain existing error/resource/freshness gates, separate 3000-us target and
6000-us hard stop, 20 actual source-time seconds per playing source/overlap,
and no timer extension or repaired/repeated invalid case. Stop at the first
source underrun, USB error, missing/invalid telemetry, reserve failure, reset
or disconnect. No source/dual/PFL/soak acceptance follows from preparation.

The earlier S03 included extra controller messages. Explain the permitted
setup controls before starting; record and independently audit all controls.
An extra-control or boundary-incomplete interval cannot become isolated
acceptance just because the numerical analyzer admitted it. Do not infer who
generated those messages. During stable observation, no PFL, transport CUE,
jog, MASTER changes or extra browse actions. Keep the known controller output
configuration and existing listening level.

## Mandatory recovery without repeated owner qualification

Keep the existing shared programmer unchanged: fresh chip/MAC/security/RDID,
all-16-MiB before equality, exact written-span readbacks, independent all-16-MiB
after equality, and no reset/eFuse/erase-all. A new adapter must separately
prove baseline-snapshot semantics and image/sector/header/proposed ancestry
before it can construct any bytes for that shared writer. An offline layout
plan alone is insufficient authority.

After a verified return, close the writer, then issue only a separately reviewed
and bound one-shot software boot action with the correct new predecessor/count
and result proof. Its successful pulse means APPLICATION_NOT_OBSERVED unless
an independent observation actually exists. Do not retry an unknown pulse.
Follow the [owner recovery policy](../RECOVERY_POLICY.md): no repeated manual
s07 cold boot, listening, Pause/CUE or recovery confirmation requests.

Archive each real result on both drives with CRC/member verification. Keep
NOT_RUN, interrupted, invalid and conditional transitions explicit. No merge
or product promotion is included.
