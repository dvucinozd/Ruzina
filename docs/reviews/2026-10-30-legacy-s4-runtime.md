# S4 legacy scalar runtime campaign

Date: 2026-10-30. Unit JC1060-20261030-COM3 / MAC e8:f6:0a:e7:22:1d.
Source and previous results: [S0–S3 log](2026-10-30-legacy-s0-s4-execution.md).
All source changes remain uncommitted on `ogranak/jc1060-c01-c02`.

## Admission and preparation

The owner confirms USB power from the PC only, the socket labelled Full Speed
Type-C, and BOOT/RESET buttons. PCB/module/panel markings cannot be read without
case removal. Case removal was deferred: the scalar experiment uses no external
GPIO/BSP peripherals and is admitted against the repeatedly measured P4
MAC/revision/flash identity. This narrows the earlier photo requirement for
this scalar-only campaign; it does not close physical BSP/model qualification.

The owner removed power, held BOOT, reconnected, then released BOOT. A no-reset
ROM handshake confirmed revision103, the expected MAC, RDID0x184068, Secure Boot
off, flash encryption off, and secure download off. The first helper invocation
had an API-name error after identity/security reads; the corrected invocation
used `esptool.cmds.attach_flash` and passed. Both raw attempts remain external.
The successful evidence is in the [unit record](../boards/readiness/jc1060-20261030-com3.json).

`prepare_flash_plan.py` verifies all package artifact hashes and four whole-flash
backup copies, validates partition MD5/bounds/overlap, and stages complete sectors.
Bytes beyond each new file's end are retained from the factory backup rather
than discarded by sector erase. Original sectors are staged separately for
restoration. The tool has no device operations. Nine offline test methods PASS.

## Exact telemetry package and write evidence

The C probe now reports reset reason and monotonic microsecond heartbeat.
A separate task accepts uppercase R as an operator app-restart request in the
flash-backed variant; it is disabled under PURE_RAM. This adds no display,
PSRAM, SD, USB host, audio or network owner.

Fresh flash-backed build returned BUILD_PACKAGE_PASS, including Rust fmt,
Clippy, linked ELF ABI/header/map checks and unchanged-during-build source hashes.
Package: `D:/BoardEvidence/JC1060-20261030-COM3/s04-reviewed-telemetry-flash`.
App BIN: 149056 bytes, SHA-256
`380aa1ab9c981b0d7c3d401055b26c081ae0df97e52f5ab9d9c9f851fb2a60b0`.
Manifest SHA-256:
`1632b24b189446d3d633ee9974b6e76832afcc8ce5651242c23ab4476b122c9b`.
See [exact sector/recovery plan](../boards/readiness/jc1060-s04-write-recovery-plan-20261030.json).

Before writing, a second live no-reset identity/security check passed and
each affected live sector matched the original backup. A standard esptool5.6.0
programmer stub wrote only the reviewed bootloader/table/app sectors, using
keep/keep/keep header settings, no full erase, no force, no eFuse operations.
Every written sector was read back and SHA-compared to its immutable payload:
[programming result](../boards/evidence/jc1060-20261030-com3/s04-first-flash-result.json).
NVS/PHY/storage were excluded from these write spans. The diagnostic table
temporarily declares a 1 MiB app and does not declare the factory storage area;
those storage bytes have not been erased by the programming operation.

## Initial reset observation

One USB reset from the programmer returned ROM DOWNLOAD mode, not the app:
[result](../boards/evidence/jc1060-20261030-com3/s04-first-boot-result.json).
No HP watchdog loop was observed in that 15-second window. This is a failed
normal-app boot scenario, not a runtime PASS. The owner subsequently confirmed
power removal and reconnection without BOOT; a bounded serial capture was armed
before this action. Its early boot prefix may be missing due to USB enumeration.

## Observed smoke outcomes

| Scenario | Outcome | Evidence |
|---|---|---|
| Physical power removal/reconnect | Owner confirms three cycles; runtime heartbeats after all three; early boot prefix not captured | [first capture](../boards/evidence/jc1060-20261030-com3/s04-cold-boot-result.json), [two repeats](../boards/evidence/jc1060-20261030-com3/s04-power-repeat-result.json) |
| App restart via console R | 3/3 request marker, reset_reason3, scalar match, 11 heartbeats per run, no HP watchdog | [results](../boards/evidence/jc1060-20261030-com3/s04-app-restart-result.json) |
| USB software ROM entry + normal reset after non-BOOT power-on | 3/3 reset_reason11, scalar match, 15 heartbeats per run, no HP watchdog | [run1](../boards/evidence/jc1060-20261030-com3/s04-usb-reset-1-result.json), [run2](../boards/evidence/jc1060-20261030-com3/s04-usb-reset-2-result.json), [run3](../boards/evidence/jc1060-20261030-com3/s04-usb-reset-3-result.json) |
| Host serial close/open without DTR/RTS | 3/3 continuous increasing uptime; four heartbeats per session | [results](../boards/evidence/jc1060-20261030-com3/s04-session-reopen-result.json) |

The repeat-power monitor reached its time limit after three heartbeats following
the last reboot, rather than its desired five-heartbeat early-exit condition.
The subsequent serial-session capture began at uptime17s, above that last
heartbeat, and confirmed continuing operation. This combination supports the
third cold-runtime smoke observation; it is not a captured complete cold boot
log. Because USB is the sole supply, physical USB reconnect is a power cycle;
host close/open is separately labelled and is not a physical cable-reconnect
test under independent power.

The initial reset while exiting manual BOOT download remains recorded as a
failed app boot (stayed in ROM), even though normal USB resets later passed.
No watchdog loop occurred in these flash-backed observations. PURE_RAM reset
remains unsupported for regular bring-up, without a proven root-cause claim.

The first scalar flash-backed boot/reset smoke gate is observed for this exact
image only. Actual factory restoration, independent-power USB reconnect,
physical PCB/module/panel markings, memory/FPU acceptance and long soak remain
open. The native minimum is still302; native C02 remains incompatible with this
unit. The device is left running the legacy scalar diagnostic, with its display
and product peripherals inactive. Next: S5's separate memory/task profile, with
the scalar profile retained as the recovery/debug baseline.

## Preservation and final checks

The [S4 evidence package index](../boards/readiness/jc1060-s04-evidence-package-20261030.json)
records the exact ZIP and secondary copy. The 4,294,400-byte archive contains
source/artifact snapshots, programmer/readback hashes and runtime logs, with
all member SHA-256 values and ZIP CRC verified. It contains no factory dump;
private factory restore-sector payloads remain in the external recovery kit.
ZIP SHA-256: `b0573ac0b99ef01c07aac9e8c29f78e784dafce6cdfca637ad38209083a6f3db`.
The secondary copy under the existing BoardBackups location matches.

Nine offline tests and the unit validator PASS. Documentation links and diff
whitespace checks PASS. Native app/crate sources and revision302 policy were
not edited. Remote CI, commit/push, full product tests and physical product
qualification are not claimed by this campaign.

References used for the tooling review:
[esptool write/erase/header behavior](https://docs.espressif.com/projects/esptool/en/latest/esp32p4/esptool/basic-commands.html)
and [P4 manual boot-mode selection](https://docs.espressif.com/projects/esptool/en/latest/esp32p4/advanced-topics/boot-mode-selection.html),
alongside the installed esptool5.6.0 source. The actual board-specific manual
sequence above is supported by the operator action and no-reset handshake.
