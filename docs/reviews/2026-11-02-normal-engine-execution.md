# Normal-engine source/package/registration checkpoint

Date: 2026-11-02. Continues the frozen
[functional runbook](2026-11-02-normal-engine-campaign.md). Do not edit that
runbook under an active registration; its byte hash is part of preflight.
Status: OFFLINE_REGISTERED / EXECUTION_TOOLS_REVIEW_PENDING / NOT_FLASHED.
Draft [PR #81](https://github.com/dvucinozd/Ruzina/pull/81) is stacked above #80.

Firmware source `886127e0714c15cb096cc3b86798fbacba1773f7` passed exact-parent
CodeRabbit with 0 issues, all six actual C cases (including distinct USB/logger
watermarks and no heap calls on the client report), and 21/21 CI:
[host #553](https://github.com/dvucinozd/Ruzina/actions/runs/41516491107),
[legacy #76](https://github.com/dvucinozd/Ruzina/actions/runs/41516491176).
Pinned Windows Python has 40 passing tests plus six C skips; the six actual
C cases pass separately in WSL. The final prior evidence head `098a8c5` also
completed its own 21/21 CI before this registration.

The clean normal engine package has profile=false and pack-IRAM=false:
819744-byte app SHA-256
`0ba716a48d615681da7d58d3dd85976cb14cd886a30c9f785ef3686818b0f011`,
ELF `02b053052ba10e7d7d889028cd56644d128b3b4aa2057873a0e3b83cdb042df2`.
All 285 source hashes and eight artifacts match; static direct-call estimates
remain 74640 load / 3120 fill bytes under unchanged limits. Compared with the
prior clean normal, allocated flash sections add 140 text and 176 rodata bytes;
static RAM sections are unchanged. Runtime queue payload grows by one u32;
actual heap/watermark/timing behavior requires this exact image's campaign.

The [registration](evidence/2026-11-02-normal-engine/campaign-registration.json)
and [offline preflights](evidence/2026-11-02-normal-engine/offline-preflight.json)
bind two unexecuted transitions:

| Path | Predecessors | Expected whole flash afterward |
|---|---:|---|
| [normal candidate](../boards/readiness/jc1060-normal-engine-candidate-write-recovery-plan-20261102.json) | 38 | `155ab6b5db69ae73f3a4498e97c658fc50031d0a69c60275d59fc80a1692b5b1` |
| [s07 return](../boards/readiness/jc1060-normal-engine-return-s07-write-recovery-plan-20261102.json) | 39 | `7875f650ba7404475e9c26561990851b04c11409b3f309401f6e77a51ddf38a6` |

Neither is executed. Current s07 remains installed, current history is **38**
and independent full state remains
`1a45683632f9951f1121ad0866fd7b3050d146022bfc6c719be18346848ad4fe`.
Return tails preserve bytes from the larger candidate; do not mistake a new
prospective full hash for a changed s07 app. Every historical write/restore/tail
and all four factory copies were checked. No prior alternative plan is inserted.

The [registration archive](evidence/2026-11-02-normal-engine/registration-archive.json)
contains 312 files, **35202842 bytes**, SHA-256
`6ded13437ca6165fd8b5301869e5a68e08c79b44ac1d5042c1333620b505457b`.
ZIP CRC, all members and secondary copies match. Primary:
`D:/BoardEvidence/JC1060-20261030-COM3/normal-engine-20261102/`;
secondary: `C:/Users/Daniel/Documents/BoardBackups/JC1060-20261030-COM3/normal-engine-20261102/`.
The [package checks](evidence/2026-11-02-normal-engine/checks.json) and review/test
raw logs retain source/artifact identity, stack estimates and static deltas.

The [normal adapter](../../tools/normal_engine/README.md) accepts this source,
normal flags, runbook and 38/39 ancestry. It imports the unchanged SHA-pinned
destructive boundary from the reviewed resource campaign. Old profile bindings
are not reused. Both actual offline preflights and new normal-policy tests pass;
exact-parent tool review, its publication/CI and a new frozen binding remain
required before device execution. No COM3 access occurred in this preparation.
