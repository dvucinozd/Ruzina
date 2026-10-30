# JC1060 legacy: S0–S3 implementation and S4 preparation

Date: 2026-10-30. Worktree `C:/Users/Daniel/.codex/worktrees/m1-guition-bsp/M1_PCB`,
branch `ogranak/jc1060-c01-c02`, base/remote-main
`4fe18e986e0a97d7f5eec0e68303f96481928af4`. Remote SHA was read with
`git ls-remote`; no remote mutation. Existing tracked/untracked changes were
preserved. New changes remain uncommitted. No serial port was opened.

## S0 — current authority

The unit aggregate links the immutable historical run using `probe_run`, and
records later owner-assisted power-disconnect recovery separately. Historical
`physical_power_cycle=NOT_RUN` is scoped to run-2; the current factory display
status is owner-confirmed. Actual factory restore remains NOT_RUN. The validator
allows historical failure followed by recovery and rejects missing links or
contradictory display/current recovery status. No old evidence was regenerated.

## S1 — fresh builds and manifests

`firmware-idf/legacy-jc1060/tools/build_legacy.py` and the PowerShell wrapper
accept explicit IDF/output/variant parameters. Windows can select WSL Rust.
They check the IDF commit and recursive submodule cleanliness, GCC/Rust/esptool
versions, run fmt/target Clippy, build Rust into the new output, then build IDF.
The manifest binds shared dependency source hashes, locks, archive, ELF/BIN/map,
config, Git state and available bootloader/table/flasher-args artifacts.
Existing output directories are rejected. Build tools have no device operations.

The source/ABI guards retain single-float checks and verify the selected matching
runtime. The native Rust revision minimum and product firmware were untouched.
The Python builder incorporates packaging rather than introducing a second
package command that could accept an arbitrary stale archive.

## S2 — guarded loader

`tools/run_ram_probe.py` defaults to offline verification; it compares approved
hash/length before transport access, validates chip/header/revision, segment
overlaps/ranges and entrypoint, and passes immutable bytes to esptool5.6.0.
It requires an explicit execution/reset policy plus unit/backup inputs for
hardware mode. No assert-based guards, flash/eFuse operations or unconditional
cleanup resets remain. Port closing and terminal JSON recording include failure
paths. Scalar success is distinct from factory recovery observation.

The frozen old RAM BIN passed offline verification with an explicit local
manifest in `D:/BoardEvidence/JC1060-20261030-COM3/s03-frozen-ram-manifest.json`.
No execution mode was invoked. Old helpers and ZIP remain historical evidence.

## S3 — validation and CI

Dedicated `.github/workflows/legacy-idf.yml` covers IDF-only changes, shared
crate/lock changes, offline tests, fmt/Clippy and both fresh build variants.
It pins the IDF commit and uploads BUILD_ONLY artifacts. The shared CMake guard
has a positive baseline and negative cases for target/revision/CPU/PSRAM/IDF.
The documentation checker now includes IDF README files and excludes build dirs.

Executed locally in the exported IDF environment:

```text
python -m unittest discover -s firmware-idf/legacy-jc1060/tools/tests -v
python firmware-idf/legacy-jc1060/tools/validate_unit.py docs/boards/readiness/jc1060-20261030-com3.json
python firmware-idf/legacy-jc1060/tools/run_ram_probe.py --image <frozen old RAM BIN> --manifest <local approved manifest>
```

Results: 7 test methods PASS (multiple negative cases within methods), including
`python -O`, wrong identity, load/capture/reset/close failure and output reuse.
UNIT_RECORD_PASS and OFFLINE_VERIFY_PASS for both the frozen old RAM image and
the new RAM image. Documentation: 139 links PASS after final report links;
diff whitespace check PASS. Fresh builds both returned BUILD_PACKAGE_PASS:

| Variant | Bytes | BIN SHA-256 | Local output |
|---|---|---|---|
| flash-build-only | 148432 | `5ea577d9ae2bd53c340d816f1eff911a076bb3fe319122c616e20f1c5718e3f9` | `D:/BoardEvidence/JC1060-20261030-COM3/s03-verified-flash` |
| pure-ram | 113312 | `071184123d67599e88f22b11f0df5cbac47ea00556172b647840d219219d77ce` | `D:/BoardEvidence/JC1060-20261030-COM3/s03-verified-ram` |

Both builds ran fmt and target release Clippy, then rebuilt the Rust archive and
IDF in new output directories. Every manifest artifact hash was independently
rechecked after build. See the [offline package index](../boards/readiness/jc1060-s03-offline-builds-20261030.json)
for manifest hashes, tool versions and exact potential flash-sector ranges.
The new RAM hash differs from the historical run: it inherits no physical PASS.
Both frozen historical ZIP copies still match their original SHA-256.

During tool development, fresh attempts caught a WSL path-conversion issue,
an unavailable CMake version variable and an overly broad archive-name check
that also matched IDF's own C HAL libraries. These were corrected before the
two final builds; failed output directories were retained, never reused as
accepted packages. A test invocation without the exported CMake PATH failed;
the full final suite passed in the documented exported environment.

Remote workflow execution is NOT_RUN; it has not been
pushed. Workflow YAML parsed locally and IDF-only path triggers checked; final
manifest source hashes still match the current tree. The full product host/native matrix was not rerun because its sources
were unchanged. Native CI continues independently.

## S4 — offline preparation

[Recovery/layout analysis](../boards/JC1060_LEGACY_RECOVERY.md) records verified
backup/table/header observations, proposed changed ranges, recovery source,
manual-entry/operator gaps and scenario/stop criteria. Factory table MD5 matches;
P4 headers at 0x2000 and 0x10000 declare 100..199. No NVS values were exported.
Actual manual ROM entry, connector/power/photo evidence, approved write plan,
programming/readback, restore and physical reset qualification remain open.

Next action: complete the physical C01 observations with the operator, bind the
exact new flash-backed package to reviewed sector ranges and recovery procedure,
then perform separately authorized S4 hardware scenarios. No peripheral work
or native gate reduction is part of this batch.
