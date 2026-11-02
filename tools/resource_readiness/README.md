# JC1060 registered resource-readiness execution tools

These tools serve only the frozen 2026-11-02 JC1060 rev103 resource-readiness
candidate and its s07 return. Firmware packages and executed flash history
remain unchanged. They are outside the firmware package's hashed source tree.
Read the [campaign record](../../docs/reviews/2026-11-02-resource-readiness.md)
and its immutable registration before use.

`program.py` and `monitor.py` default to offline preflight. For example, in the
pinned Python environment, with no COM port opened:

```powershell
python tools/resource_readiness/program.py --root D:/BoardEvidence/JC1060-20261030-COM3/resource-readiness-20261102 --mode candidate
python tools/resource_readiness/program.py --root D:/BoardEvidence/JC1060-20261030-COM3/resource-readiness-20261102 --mode return-s07
python tools/resource_readiness/monitor.py --root D:/BoardEvidence/JC1060-20261030-COM3/resource-readiness-20261102 --mode candidate
python -m unittest discover -s tools/resource_readiness/tests -v
```

All safety checks use exceptions, including under Python `-O`. Preflight pins
the registration/planner hashes, packages, all candidate source files, four
factory copies, complete ordered ancestry, staged payloads, preserved tails
and reconstructed before/after images. Verified write bytes stay in memory;
execution does not reopen staged payload files. Host tests inject a fake
backend, including corruption outside written spans. They never open COM3.

Device operations additionally require `--execute`, a fresh `--output`, and
`--binding` plus its independently recorded `--binding-sha256`. The reviewed
binding must identify the campaign and all three current Python tool files.
Create it only after exact-parent review; record/archive it and successful
registration/tool-source CI before device work. Do not compute a new binding
hash on the fly to bypass drift. No executable device command is implied by
the offline examples above. Owner-assisted ROM entry/topology refresh remain
separate physical gates.

The programmer permits COM3 only, with the registered VID/PID/USB serial. Pinned
esptool 5.6.0 connects with `no-reset`; rev103, MAC, security and RDID must match.
Every byte of the 16 MiB predecessor must match before writing. Writes keep
flash mode/frequency/size, use no force/erase-all, and are followed by span and
whole-flash readback. Exceptions, interruptions and mismatches produce failure
evidence and close the device; they do not auto-reset or try another recovery
path. Inspect failures before another action. The prewrite/after files and
result remain outside Git in the fresh output directory.

The passive monitor uses no serial writes and disables DTR/RTS before opening.
Cold capture runs for 45 seconds after the expected ELF identity, bounded by
`--seconds` (default 300, maximum 600). The raw capture is retained; a separate
boot window excludes preceding-image telemetry. A second boot invalidates that
window. `--after-boot 0 --seconds 60` is the separate steady capture; its image
identity must be associated with the preceding verified cold capture. Capturing
bytes never grants timing/calibration/resource or listening acceptance.

Always return to the exact s07 plan after the candidate, then verify readback,
cold boot and owner listening. Normal engine playback/I02-I13 need a later,
independently registered image. Native minimum 302 is unchanged.
