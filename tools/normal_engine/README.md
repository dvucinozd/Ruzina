# Normal-engine campaign adapter

This adapter binds the [normal campaign](../../docs/reviews/2026-11-02-normal-engine-campaign.md)
to its own source/package/runbook/registration and 38/39-predecessor plans.
It rejects the earlier profile-only or 36/37-predecessor registration.
The existing destructive programmer/port/esptool boundary is imported from the
unchanged, SHA-pinned `tools/resource_readiness/campaign.py`. No new write policy
or automatic reset is introduced. Prior tool bindings do not authorize these
new adapter files; freeze a new binding after their review.

Default commands perform offline verification only:

```powershell
python tools/normal_engine/program.py --root D:/BoardEvidence/JC1060-20261030-COM3/normal-engine-20261102 --mode candidate
python tools/normal_engine/program.py --root D:/BoardEvidence/JC1060-20261030-COM3/normal-engine-20261102 --mode return-s07
python -m unittest discover -s tools/normal_engine/tests -v
python -m unittest discover -s tools/resource_readiness/tests -v
```

Execution requires the new exact binding, explicit execute and a fresh output
directory. Keep the registered runbook byte-identical until the campaign ends.
Normal mode has no scheduler/IRQ substage profile; use whole/phase fill counters
and distinct engine resource rows, not the diagnostic calibration analyzer.
Observe reserves and all-fill limits before advancing cases. Missing telemetry
or an unavailable target fault injector is never PASS. Always return/readback/
cold/listen s07 at campaign exit; preserve current full state and executed history.
