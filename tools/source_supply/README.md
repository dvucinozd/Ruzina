# Registered producer-profile campaign tools

Exact registration `83e4a7fb...`, source `e9ce242`, candidate/return history
40/41. Default program/monitor commands perform offline preflight only. The
destructive writer, USB identity and esptool backend are imported from the
unchanged SHA-pinned resource-readiness guard. Existing bindings do not
authorize these adapter/analyzer files. Freeze/archive/publish a new exact
binding after review and complete CI before device access.

```powershell
python tools/source_supply/program.py --root D:/BoardEvidence/JC1060-20261030-COM3/source-supply-20261103 --mode candidate
python tools/source_supply/program.py --root D:/BoardEvidence/JC1060-20261030-COM3/source-supply-20261103 --mode return-s07
python -m unittest discover -s tools/source_supply/tests -v
```

Explicit execution needs `--execute --output NEW --binding FILE
--binding-sha256 HASH` with all four campaign/program/monitor/analyzer hashes.
Passive capture windows are cold 45 seconds after expected ELF (overall <=600),
idle 60 connected seconds, or functional 30-180 seconds; S05 uses 60 total. No
soak is registered. Port wait never counts as capture; post-start connection
loss invalidates evidence. Before expected cold boot, only a genuinely absent
port can retry opening. DTR/RTS are false before open; no serial writes.

The candidate monitor automatically stops at the first complete report of a
registered runtime fault. It preserves raw bytes and `stopped_for_fault`.
A short fault window stays incomplete; never keep playing to meet 30 seconds.
Stop/duration/capture status is not product acceptance. Source integrity is
checked offline; missing or invalid evidence stops extension, not recovery.

```powershell
python tools/source_supply/analyze.py --root D:/BoardEvidence/JC1060-20261030-COM3/source-supply-20261103 --capture cold-1 --kind cold --playing 0,0
```

Use `idle`/`functional` and the actual requested playing state for later cases.
They revalidate the exact cold anchor. The analyzer checks complete stage
groups, same-window partitions, advancing/fresh timestamps/sequences, counter
monotonicity, resources, callback bounds, drops and stable-source intervals.
It reports wall time and actual intervals; no CPU-time, hardware IRQ-latency,
simultaneous cross-core or full-case throughput inference. Incomplete evidence
raises rather than granting a continuation result. Preserve failed/partial
raw captures and restore exact s07 even when quantitative analysis cannot pass.

Follow the byte-frozen [runbook](../../docs/reviews/2026-11-03-source-supply-campaign.md).
Always return/full-readback/cold/listen s07 at campaign exit. Keep native
minimum 302 and current accepted gain/ownership unchanged.
