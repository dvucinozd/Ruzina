# Final A+B review and C01/C02 preparation

Date: 2026-10-27. Branch: `fix/astra-review-phase-a`.
Implementation commit: `cb70a425d691d0fd10ec097b263567fdefe70070`.
The later package-index/documentation commit does not alter firmware sources.

## Review and validation

CodeRabbit CLI 0.9.1 reviewed all 44 A+B changed/new files against main
`4a8c660183910b0950ae7ddabce9e8b395a2f10d` in an isolated native Git checkout.
**CodeRabbit raised 0 issues.** The completed event, reviewed-file list and raw
log SHA-256 are retained in [review result](2026-10-27-final-review-result.json).
The scope includes runtime, HTTP uploads, profile/settings owners, OTA streaming,
tests and the A+B documentation. Subsequent C01/C02 additions are documentation,
templates and package indexes; no runtime code changed after that review.

Local phase B validation remains: 938 Rust tests passed, 6 JS tests passed,
Clippy/format passed, 64 UI goldens passed and changed no_std contracts passed
P4 checking. Details are in the [phase B log](2026-10-27-phase-b-log.md).
The nine existing package/capture Python tests were rerun and passed. The new
backup command snippet passed PowerShell AST parsing and both unit templates
passed JSON parsing with physical_acceptance=NOT_RUN.

Remote full host and P4 feature/build/package workflow results are maintained by
GitHub against the exact published HEAD. Consult the branch's Actions runs;
a local result or previous-main CI run is not a substitute for that SHA's run.
No merge to main or release publication is part of this task.

## Ready-to-use local operator kits

Eight release diagnostic ELFs were built from the clean implementation commit:
`basic`, `flash`, `psram` and `memory` independently for jc1060 and jc8012.
No source inputs/HEAD changed during compilation, and all eight manifests record
empty dirty_status. Build source/command/compiler/environment and exact image_id
are recorded by the existing guition_first_boot.py tool.

The [package index](../boards/readiness/2026-10-27-package-index.json) records
ELF, per-variant ZIP and final per-board kit SHA-256 values, sizes and local paths.
Final archives are under `firmware-rust/target/guition-c01c02-20261027/kits-final/`:

- `jc1060-c01c02-ready.zip`
- `jc8012-c01c02-ready.zip`

Each kit includes the matching four diagnostic packages, build logs/manifests,
source documentation snapshot, matching unit template, capture/ingest helper,
kit-manifest.json and SHA256SUMS. Every inner ELF/build-log/ZIP hash was checked;
final archives passed ZIP CRC checks and byte/hash comparison of every member
against the assembled kit. Original intermediate packages remain retained.

The ZIPs/ELFs remain generated local artifacts, outside Git history. Remote P4 CI
retains its own basic/memory diagnostic artifacts built against its exact HEAD;
their image IDs and hashes must be read from those manifests. Do not treat a
remote CI archive as the same binary as a local cb70a42 kit.

## C01/C02 operator starting point

Read [backup/recovery and diagnostic procedure](../boards/GUITION_C01_C02_READINESS.md)
and [peripheral ownership cards](../boards/GUITION_PERIPHERAL_OWNERSHIP.md).
Templates are separate for
[JC1060](../boards/readiness/jc1060-unit-template.json) and
[JC8012](../boards/readiness/jc8012-unit-template.json).

Record actual unit/panel/PCB/chip/geometry/connector first. Preserve two
independent whole-flash readbacks, compare observed sizes and SHA-256, retain a
second storage copy, and establish repeatable ROM entry before selecting any
programming layout. No universal offsets, ROM button sequence or GPIO activation
were inferred. PSRAM diagnostics are only an exclusive 64 KiB CPU-cached scratch
test at 80 MHz, not full memory/DMA qualification.

All packages remain **BUILD_ONLY / NOT_FLASHED / NOT_QUALIFIED**. Hardware and
factory restoration remain NOT_RUN. APTA remains OFF; headphones remain on the
controller during every MAIN route. Actual OTA trust/project/slot adapters,
permitted real fixtures and C03–C07 physical qualification remain open.
