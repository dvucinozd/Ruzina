# Producer-profile execution registration

Date: 2026-11-03. Continues the [reviewed source/packages](2026-11-03-source-supply-profile.md)
and byte-frozen [campaign runbook](2026-11-03-source-supply-campaign.md).
Current status: **EXECUTED / COLD_IDLE_PASS / PLAYBACK_NOT_CAPTURED / S07_RESTORED**.
Read the [actual device result](2026-11-03-source-supply-device.md) and
[new owner recovery policy](../RECOVERY_POLICY.md). Current history 42/full
`28830827...`; all tools closed. The preparation sequence below is historical;
its NOT_FLASHED/history 40 statements do not describe the current device.
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

Full committed tool review at `d713276` found one minor capture-boundary issue:
an unterminated final serial line could be parsed as missing fields. The fix
preserves/hash-checks all raw bytes, counts the final fragment separately, and
parses only complete lines. A complete producer header with missing stages
still fails. Regression cases cover both outcomes; all 12 tests pass per OS.
Raw review and updated logs are retained. Final fix review/CI/binding remain
pending; no source package, frozen runbook, plan or device state changed.

## Execution tools verified

Final fix review at `bb9613277ecebb1017622d4042f01a4b0fe420c2` against
`d713276` completed with **0 issues**. Exact CI is **22/22 PASS**:
[host #572](https://github.com/dvucinozd/Ruzina/actions/runs/41601593545),
[legacy #95](https://github.com/dvucinozd/Ruzina/actions/runs/41601593562).
The full/fix reviews and all 12 per-OS tests are retained separately; the shared
destructive guard remains SHA-identical with all 13 WSL fault tests passing.

[Execution binding](evidence/2026-11-03-source-supply/execution-binding.json)
SHA-256: `a63241b1c2c64f33a1c0d127e0489fdc1a86064a9c78526dd95c462ea18608dc`.
It pins campaign/program/monitor/analyzer and the unchanged write boundary.
The [tools archive](evidence/2026-11-03-source-supply/execution-tools-archive.json)
is **48748 bytes**, SHA-256
`949e48318a3538f1909d4ce72c4eb65fbdf5c4184927049b04c9fe896bb42aa8`.
CRC, every member and secondary copy match. Publish this checkpoint and rerun
both actual offline preflights before the already authorized software ROM entry.
No normal/profile binding is reused. S01 admission precedes every source case;
always restore/readback/physical cold/listen s07 before campaign exit.
