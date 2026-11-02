# Normal-engine source/package/registration checkpoint

Date: 2026-11-02. Continues the frozen
[functional runbook](2026-11-02-normal-engine-campaign.md). Do not edit that
runbook under an active registration; its byte hash is part of preflight.
Status: OFFLINE_REGISTERED / CONNECTION_COVERAGE_FIX_REVIEW_PENDING / NOT_FLASHED.
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

## Review fixes and exact continuation

Full adapter/registration review at `ec43409` reported two minor argparse help
issues; both are fixed in `dd245b6`. Registration `ec43409` independently passed
21/21 CI ([host #554](https://github.com/dvucinozd/Ruzina/actions/runs/41518199082),
[legacy #77](https://github.com/dvucinozd/Ruzina/actions/runs/41518198972)).
Intermediate `dd245b6` workflows were deliberately cancelled after a newer
bounded-window adapter was published; cancelled jobs are not counted as PASS.

Full review of `f89bb36` then reported one major monitor issue: an ELF arriving
near the overall timeout could receive CAPTURED_NOT_QUALIFIED before completing
45 post-boot seconds. `05cff46bb38ff1ba365a89ceaedf08543bdf48a3` now records
post-boot duration/completion explicitly, classifies short windows as
CAPTURE_INCOMPLETE, and permits an early functional stop only after 30 seconds.
Late/no boot, shortened idle/soak and early functional boundaries are tested.
Four normal-policy/window tests pass on Windows/WSL; all 13 unchanged shared
destructive-guard tests pass separately. The shared write/reset boundary and
firmware package bytes remain unchanged by these Python fixes.

Final fix review against parent `f89bb363286aec396cc6c7454eb833b663725c07`
could not start: the retained CodeRabbit error reports **rate_limit / 34 minutes**
at about **21:34 Europe/Zagreb on 2026-11-02**. Earlier six-minute and five-second
quota errors are retained too. These are unavailable review attempts, never
passed reviews. The initial full review and duration finding are preserved beside
the [latest limiter result](evidence/2026-11-02-normal-engine/coderabbit-adapter-rate-limit-3.raw.log).
No final tool 0-issue result/binding has been issued, no device is opened, and
accepted s07 remains installed. This is a review-service limit, separate from
the raised GitHub Actions quota.

Continue after **22:10 Europe/Zagreb on 2026-11-02**, or after the repository is
connected to a suitable CodeRabbit organization plan. Do not replace the final
review with a claimed manual PASS. The exact command is:

```powershell
wsl -d Ubuntu bash -lc 'cd /mnt/d/AI/M1_PCB && coderabbit review --agent -t committed --base-commit f89bb363286aec396cc6c7454eb833b663725c07 > /mnt/d/BoardEvidence/JC1060-20261030-COM3/normal-engine-20261102/coderabbit-adapter-final.raw.log 2>&1'
```

Verify exact latest code CI:
[host #557](https://github.com/dvucinozd/Ruzina/actions/runs/41520394199) and
[legacy #80](https://github.com/dvucinozd/Ruzina/actions/runs/41520394178).
They are pending at this checkpoint; older source/registration passes are not
their results. Then create `adapter-ci-verified.json` bound to the reviewed
tool commit and freeze with the external `freeze_execution_tools.py` (source
currently `05cff46`, fix-review base `f89bb36`, four policy tests). Retain prior
full review findings and their fixing commits beside the final fix review.
Archive and publish binding/tool hashes before COM3. Rerun both normal offline
preflights. The owner-authorized software ROM method remains available; only
cold/soak evidence still needs the corresponding real physical/elapsed event.

## Connection-coverage follow-up (22:13 Zagreb)

The scheduled 22:10 review completed and found one major additional issue:
waiting for COM3 and serial-disconnection time could count toward a completed
window. The [raw finding](evidence/2026-11-02-normal-engine/coderabbit-adapter-connection-finding.raw.log)
is retained. The monitor now starts non-cold duration only after opening the
port, permits at most ten seconds of initial waiting, and invalidates any
disconnect after the measurement begins. Cold windows permit the expected
pre-ELF USB power cycle, then require uninterrupted post-ELF capture within
the original overall bound. Actual connected seconds/interruption are recorded.
Seven normal policy/coverage tests pass on Windows and WSL; the unchanged
shared guard still passes all 13 WSL tests. Source `05cff46` and publication
`b1afa1b` have now each completed their own 21/21 CI. The connection fix needs
its own review/CI and a fresh binding before hardware access; source, package,
38/39 plans and the installed s07 remain unchanged. The owner reconfirmed the
same COM3/topology, connected MASTER/headphones and visible library.
