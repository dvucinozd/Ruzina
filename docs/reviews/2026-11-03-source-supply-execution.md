# Producer-profile execution registration

Date: 2026-11-03. Continues the [reviewed source/packages](2026-11-03-source-supply-profile.md)
and byte-frozen [campaign runbook](2026-11-03-source-supply-campaign.md).
Status: **OFFLINE_REGISTERED / TOOLS_REVIEW_CI_PENDING / NOT_FLASHED**.
No COM3 access has occurred in this registration. The owner reconfirmed the
same COM3/full-speed USB-C, USB-only power, FLX4, MASTER/headphones and library.
Accepted s07/history 40/full `7875f650...` remain installed. Native minimum 302
and all gain/task/resource/timing limits stay unchanged.

The source package is `e9ce242` with producer profile=true, UAC profile=false,
pack-IRAM=false, app `173933e100e2de34257beb89013556b0937e8b0e94bd36bf797eb755d505d1cd`,
ELF `54dbdc424791bfc3f3516dd194487bd0a09e009bf7215e380c7a57b10d58af11`.
Its final CI/docs fixes and publication `84df18b` both have 22/22 CI; firmware
package source hashes are byte-identical through those metadata changes.

[Registration](evidence/2026-11-03-source-supply/campaign-registration.json)
SHA-256: `83e4a7fb8cf47c14d773f1fa06c46e683b88dd788d04bdd7cbcba936099d2818`.
Both [offline preflights](evidence/2026-11-03-source-supply/offline-preflight.json)
replay every restore/staged/tail and all four distinct factory copies.

| Unexecuted transition | Predecessors | Expected full flash afterward |
|---|---:|---|
| [Producer candidate](../boards/readiness/jc1060-source-supply-candidate-write-recovery-plan-20261103.json) | 40 | `95e069e0ebf04452f983f192520789e30d09449d3853f57bd0cb0eaa50a048e9` |
| [Exact s07 return](../boards/readiness/jc1060-source-supply-return-s07-write-recovery-plan-20261103.json) | 41 | `28830827d3e3426bbcd58b9a25d4d2129e28f150bade33fe3c551cd577f93921` |

These prospective hashes are not executed history. Current actual full flash
is `7875f650ba7404475e9c26561990851b04c11409b3f309401f6e77a51ddf38a6`.
Return tails preserve the longer candidate; the accepted s07 app is unchanged.

The [registration archive](evidence/2026-11-03-source-supply/registration-archive.json)
has 326 members, **36784762 bytes**, SHA-256
`ba070b5c79301bc2cca231be17e69eee26ad7b915b71c9378217ca3fa63721e4`.
CRC, members and all secondary copies match. Primary/secondary paths are the
same source-supply roots in the source record. The original registration
recipe exported repo metadata to the earlier normal evidence directory; export
copies were moved to this campaign and the older files restored exactly from
Git before publication. Frozen registration/plan/payload/archive bytes were
unchanged; no prior evidence is modified.

The [new adapter/analyzer](../../tools/source_supply/README.md) requires exact
producer mode and 40/41 ancestry, reuses the unchanged hash-pinned destructive
guard, and copies connected-duration/port-race protections into its passive
monitor. Candidate report faults stop capture immediately, even before the
minimum positive-evidence duration. Partial failure remains incomplete data.
Analyzer checks same-window partitions, complete/fresh per-deck/owner/service
observations, stable source states, resources, expected cold identity and raw
hashes. No stale/invalid/short window becomes PASS. It reports actual source
intervals separately from case duration, and cannot infer CPU time or listening.

Twelve analyzer/policy/real-loop synthetic serial cases pass on Windows and
WSL, plus all 13 shared-guard WSL fault cases. Documentation links pass.
Review/CI and a new published exact-hash binding must precede execution.
The prior normal binding is not reused. Keep the runbook byte-identical.
After the reviewed binding is published: software ROM entry, guarded candidate
write, S01/S02 admission, separate single sources, then bounded both-playing.
Always exact s07 return/full readback/physical cold/listening at campaign exit.
