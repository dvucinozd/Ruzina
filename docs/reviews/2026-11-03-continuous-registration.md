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

Still required before device work: successful review of new writer/registration
code; exact source CI; verified dual-copy registration archive; exact independent
write/capture/boot bindings and tool archive; publication and final offline
preflight. Then refresh actual topology and run only the registered candidate
cases, always restoring/readback s07 with the new automatic exit. Do not
reintroduce manual s07 recovery checks or lower the native minimum 302.
