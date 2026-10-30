# Guition C01/C02 operator package

Prepared 2026-10-27; arrival update 2026-10-30. Each model/unit needs independent
observations. The first JC1060 reports v1.3 on COM3 and cannot run these native
v3.2 packages. Read the [unit-specific assessment](JC1060_ARRIVAL_2026-10-30.md).
No board has been flashed or physically qualified by this preparation.

Use the [hardware matrix](GUITION_HARDWARE_MATRIX_2026-10-20.md),
[first-boot capture procedure](GUITION_FIRST_BOOT_2026-10-20.md) and
[ownership cards](GUITION_PERIPHERAL_OWNERSHIP.md). Begin with a copy of the
matching `readiness/jc1060-unit-template.json` or `readiness/jc8012-unit-template.json`.
Never fill an observed field with a documented candidate value.

## C01: identity, factory preservation and recovery

1. Assign a unique unit ID. Photograph model, PCB/module/panel markings and
   connectors. Record power source, cable, host OS and all initially attached
   hardware. Keep externally powered USB peripherals disconnected until their
   VBUS/backfeed ownership is established.
2. Run the factory image first and retain raw logs/photos. Record the exact
   service connector and transport: native USB Serial/JTAG and CH340/UART are
   different paths. JC8012's UART connector does not establish native USB logs.
3. Establish ROM entry from the actual buttons/reset/connector, and document
   the observed sequence. Record read-only chip identity, silicon revision,
   flash ID/geometry and the programmer tool/version used. If ROM entry or
   geometry cannot be established, stop before selecting any write command.
4. Read the entire observed flash twice as independent operations into two
   new files outside Git. Preserve both logs, commands, timestamps and start
   address/length. Neither file may be produced by copying the other.
5. Check both lengths against the observed geometry and compare SHA-256. The
   following Windows commands only inspect existing files; replace paths and
   observed byte count with the actual captured evidence:

```powershell
$backupA = 'D:\BoardEvidence\UNIT_ID\factory-read-1.bin'
$backupB = 'D:\BoardEvidence\UNIT_ID\factory-read-2.bin'
$observedFlashBytes = (Read-Host 'Observed whole-flash byte count') -as [long]
if ($null -eq $observedFlashBytes -or $observedFlashBytes -le 0) { throw 'Missing observed geometry' }
if ((Resolve-Path -LiteralPath $backupA).Path -eq (Resolve-Path -LiteralPath $backupB).Path) { throw 'Two distinct readback files required' }
if ((Get-Item -LiteralPath $backupA).Length -ne $observedFlashBytes) { throw 'First read length mismatch' }
if ((Get-Item -LiteralPath $backupB).Length -ne $observedFlashBytes) { throw 'Second read length mismatch' }
$hashA = (Get-FileHash -LiteralPath $backupA -Algorithm SHA256).Hash
$hashB = (Get-FileHash -LiteralPath $backupB -Algorithm SHA256).Hash
if ($hashA -ne $hashB) { throw 'Factory readbacks differ; preserve both and investigate' }
$hashA
```

6. Repeat ROM entry after a normal factory boot and retain the read-only
   handshake. This proves repeatable ROM entry; it does not prove restoration.
   Copy the two backups and logs to a second storage location and verify hashes.
7. Record a board-specific restoration/programming procedure from the observed
   geometry, connector and actual image layout. Review bootloader/partition/
   application offsets before any erase/write. This package does not infer
   offsets from an ELF and does not provide a universal flash command.
8. Gate C02 on completed identity, matching independent full readbacks and
   repeatable recovery entry. Restoration stays NOT_RUN until actually tested;
   a successful handshake or matching backup hashes must not be labelled a
   factory-restore PASS. Keep factory images outside the source repository.

## C02: exact diagnostic image and boot evidence

1. Select only the matching board and explicit probe package. Verify its ZIP,
   `firmware.elf`, manifest and build-log hashes against the package index.
   Check source commit, image_id, compiler, required minimum revision 302,
   model and feature list. Do not interchange models or probe variants.
2. Use the separately reviewed programming layout and recovery path established
   in C01. Retain the exact programmed file(s), offsets, commands and readback
   evidence. An application image_id alone is not a byte-for-byte flash check.
3. Start with `basic` (`probe=none`): HAL/scheduler/native serial diagnostics.
   Capture cold-boot, warm-reset and service-reconnect separately with
   `guition_first_boot.py capture` or `ingest`; name unit/scenario explicitly.
4. Next use `flash`: bounded read-only RDID. Unknown/busy/timeout stays explicit;
   no capacity is filled in from the board profile.
5. Only then select `psram` or combined `memory`. These release images initialize
   external RAM at 80 MHz and overwrite an exclusive 64 KiB CPU-cached scratch
   span. They do not register a product allocator or qualify full capacity,
   DMA/cache coherence, retention or production clocks.
6. Retain raw logs plus result.json per image/unit/scenario. Test spontaneous
   reset and reconnect observations without reclassifying repeated boot reports
   as runtime heartbeats. Keep all scenario_acceptance values NOT_QUALIFIED
   until an operator reviews the actual physical evidence.

## Prepared assets and remaining boundaries

The existing package builder produces both models for each of `none`, `flash`,
`psram` and `memory` and refuses to overwrite output evidence. Local package
hashes/source identities will be recorded in the readiness package index.
The prepared local images are now recorded in the
[package index](readiness/2026-10-27-package-index.json); see the
[final preparation report](../reviews/2026-10-27-publication-and-bringup.md).
Remote CI also retains `guition-firstboot-diagnostics` and
`guition-memory-diagnostics` artifacts for its own exact commit/build inputs.
Do not substitute CI/local archives merely because their source code matches:
each image/ZIP is identified by its own manifest and hashes.

C01/C02 do not activate display/touch, SD, USB host power, audio or C6. Those
owners and the controller-only headphone policy remain governed by C03–C07.
