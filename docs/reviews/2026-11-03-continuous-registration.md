# New continuous-source 42/43 registration

Preparation only: **no COM3 access, flash write, reset or device result**.
Installed s07 remains history 42/full `28830827...`. The immutable
[new runbook](2026-11-03-continuous-campaign.md) governs only this candidate
experiment. The old 40/41 plans are consumed historical evidence.

[`register_continuous.py`](../../tools/source_supply/register_continuous.py)
rehashes the previous frozen registration, both executed programmer proofs
and their complete before/after captures. It reconstructs all 42 executed
plans from the factory backup, verifies four distinct factory copies and
all 289 candidate source inputs, then calls the unchanged SHA-pinned planner
for new candidate/return plans. It produces the write registration, capture
contract and separate s07 software boot policy. It never imports serial or
executes a programmer. Fresh output is mandatory; partial failures remain
for inspection rather than overwriting or deleting old results.

[`continuous_write.py`](../../tools/source_supply/continuous_write.py) defaults
to offline preflight. It binds the runbook and owner recovery policy, checks
exact package/app/ELF and source hashes, all ancestry links/counts/full states,
and independently validates each historical restore payload and preserved
sector tail. Each new write must equal its actual bound package image plus
the unchanged predecessor sector tail; rehashed arbitrary staging bytes are
rejected. It also enforces exact erase bounds, all three offsets and unchanged
no-reset/keep/no-force command policy. Return ancestry must end in this exact
registered candidate. No old execution binding authorizes it.

The destructive operation is still the unchanged shared `program` function
and `EspBackend`: fresh chip revision/MAC/security/RDID, full 16 MiB before
equality, span readbacks and full 16 MiB after equality, no reset/eFuse or
erase-all. The new writer has its own separate reviewed binding requirement.
Capture and boot retain their already implemented independent bindings; no
capture admission or owner playback reply is required to invoke recovery.

## Prepared artifacts

Primary root: `D:/BoardEvidence/JC1060-20261030-COM3/continuous-source-20261103/`.
`campaign-registration.json` SHA-256:
`163d3ecd3e6809710c627b264aa57f5ec924aa76c5e3f74686f10ce1fe192a47`.
`capture-contract.json` SHA-256:
`4eb8e011166eb133b13c0296062a1279f6e97bfc673156707833937390a6a3ce`.
`boot-policy.json` SHA-256:
`f4a76f15dd1b303a110d568c88f41accb85c656e613c0fac1b520ebc6c0342ff`.

| Transition | Predecessors | Plan SHA-256 | Full hash afterward |
|---|---:|---|---|
| Producer candidate | 42 | `baa23c1709125d34751b2802ea6df621d21579fb841588f12dd5c48a15d56c95` | `95e069e0ebf04452f983f192520789e30d09449d3853f57bd0cb0eaa50a048e9` |
| Exact s07 return | 43 | `c39e6f0e3de6783fedbda1d4a37b423cd822ee7fb57d4a0ef825745e4d3a6a56` | `28830827d3e3426bbcd58b9a25d4d2129e28f150bade33fe3c551cd577f93921` |

Both real-file offline preflights passed with the pinned IDF Python. This is
reconstruction, not live flash equality or authorization to execute before
review/CI/archive/publication. Physical execution is NOT_RUN. Automatic s07
readback and separately recorded boot follow the [owner policy](../RECOVERY_POLICY.md),
without repeated manual cold/listening/Pause/CUE qualification.

## Host checks and remaining gates

Seven new preflight tests cover a complete synthetic 42-plan chain, exact
image/tail matching, damaged historical restore data, reordered/truncated
executed prefixes, old/native/command drift, fresh-output registration and
old-binding rejection. All 53 source-supply tests pass per OS: 39 capture/
adapter, 7 boot and 7 writer/registration tests. Real-file integration verifies
the actual retained packages and all factory/history bytes separately; no
synthetic fixture is registered for target use. The historical four tools and
shared writer remain unchanged.

## Completed review, CI, archives and bindings

Exact source `b92fb2364db94d23fb7745391953597ba51a03b5` passed **22/22 CI**,
including separate normal-engine/producer builds, and CodeRabbit raised
**0 issues** in the complete writer/registration/runbook review. The initial
rate-limit failure remains a separate retained attempt; the owner explicitly
chose to wait for the free-limit reset. The successful retry is retained as
`coderabbit-registration-after-reset.raw.log`. Capture/boot source `0ac84ab`
also passed 22/22, and its exact byte equivalence with its earlier reviewed
tools was checked before creating those two bindings. No failed/partial review
or old source CI substitutes for these verified results.

[Final binding record](evidence/2026-11-03-continuous-registration/execution-bindings.json)
SHA-256 `08b433671476a8de2399ebd2250acb395612382a09ac44167dbfaecd2f1e1815`:

| Separate binding | SHA-256 |
|---|---|
| write-binding.json | `59e6a05a9ef75ca22749e3260344f91193869904ba205d19c2b2f6610523cc83` |
| capture-binding.json | `b971f685caecd7b810db638e9be28470a40b8131927b229c58c8694b248d5542` |
| boot-binding.json | `53175f800c99aa5dfcd290e4b44c6a2b0cb967ad2ac537ff9b90e2cb5bf7edf8` |

All three runtime binding validators pass offline, as do candidate/return
preflights. They pin different actions; a capture binding cannot authorize
writing or booting. All original firmware inputs, historical four execution
files and shared writer remain unchanged. Nothing here is a live equality
check or a physical result.

[Registration archive](evidence/2026-11-03-continuous-registration/registration-archive.json):
`registered-continuous-source.zip`, **86523695 bytes**, 656 members,
SHA-256 `99d41efcccc3aa4674786d037e07787c616ae3e3e57efc15d287cec0977128c7`.
It retains plans/payloads, all history, firmware source/packages and executed
readback evidence. Its earlier review/CI-pending label records preparation
time; this final binding record records the subsequent completed gates.

[Reviewed tools archive](evidence/2026-11-03-continuous-registration/tools-archive.json):
`reviewed-continuous-source-tools.zip`, **91834 bytes**, 42 members,
SHA-256 `8344bfe621a318f5bfe8a8a1c71de8d361a2d28b10bf2df99d289f2731d93fbc`.
Both archives passed CRC and every member hash; every secondary file/archive
matches under `C:/Users/Daniel/Documents/BoardBackups/JC1060-20261030-COM3/continuous-source-20261103/`.
The previous archives and runbooks remain immutable.

The new exported [candidate plan](../boards/readiness/jc1060-continuous-candidate-write-recovery-plan-20261103.json)
and [s07 return plan](../boards/readiness/jc1060-continuous-return-s07-write-recovery-plan-20261103.json)
are preparation artifacts, both **NOT_RUN**. Publish and verify the remote
head, recheck all offline bytes, then refresh actual topology before any COM3
access. Only then run the registered candidate cases and automatic s07 exit.
Do not reintroduce manual s07 recovery checks or lower native minimum 302.
