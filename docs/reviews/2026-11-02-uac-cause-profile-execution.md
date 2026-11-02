# Calibrated callback cause experiment

Status: REGISTERED_BUILD_ONLY / DEVICE_NOT_RUN. Branch
`ogranak/jc1060-uac-cause-profile`, parent `62717ce6b04b62cd5f1977b1bdfe5a91adcabf4f`.
Continues the [cause plan](2026-11-01-uac-callback-cause-follow-up.md) after the
[prior measurement and verified s07 return](2026-11-01-uac-callback-profiling-device.md).
This is diagnostic attribution, not a timing fix. Native minimum remains 302;
the observed JC1060 v1.3/103 stays on accepted s07 until the new registered run.

The status above is the registration checkpoint. The later
[device result](2026-11-02-uac-cause-profile-device.md) supersedes it: the exact
image was measured, timing failed, and accepted s07 was restored/readback/cold-
boot verified with owner-confirmed normal sound/Pause/CUE. All monitors are
closed. Future plans need 33 predecessors. Continue with the
[cache/compute plan](2026-11-02-uac-cache-compute-follow-up.md); it is PLAN_ONLY.

## Implementation

- Before this connection's first audio submit, the actual USB client binds the
  scheduler hooks, snapshots absolute counters, blocks for one RTOS tick, then
  snapshots again. At least one completed positive-duration switch-out interval
  is required. No deliberate delay is added to a fill/callback. Calibration
  failure disables scheduler attribution; it does not masquerade as zero time.
- IRQ dispatch counters are enabled only by the first USB-task snapshot, after
  timer initialization. The same pre-stream blocking interval must observe an
  actual dispatcher entry. Its absolute before/after counts and validity are
  logged separately. Failed calibration disables IRQ attribution.
- Retained callback records carry absolute scheduler before/after counters,
  wider window durations and calibration results. Worst, latest >=6000 us and
  latest >3000/<6000 us records retain their generation and ordinal.
- Seven inclusive-upper bins (50, 100, 250, 500, 1000, 3000, infinity us) report
  whole-callback distributions and all eight exclusive Rust stages. Only exact
  valid partitions enter stage bins; all callbacks enter the whole bins.
  Counts saturate. Histogram lines include snapshot sample counts to prevent
  mixing successive reports. Scope is boot-cumulative, like the existing phase
  counters; record identities retain their original generations.
- Formatting remains on the logger task; no allocation/printing in the fill or
  IRQ hooks. The explicit profile logger stack is 4096 bytes (normal remains
  3072). Queue/stack/static growth and high-water marks must be evaluated.
- The raw-log analyzer validates exclusive sums, absolute counter arithmetic,
  both calibrations, interval geometry and histogram denominators. Incomplete,
  conflicting, nested or saturated data cannot silently qualify attribution.

## Pinned IRQ coverage audit

Authority: unmodified ESP-IDF `fff9895c82d744c7237be8847347bdd1b07c6643`.
The audit uses local pinned primary sources; web retrieval of this exact commit
was unavailable. No global checkout patch or new trace provider is installed.

`components/riscv/vectors.S` calls `_global_interrupt_handler` after enabling
nested interrupts; context-switch exit occurs after that call. The dispatcher
in `components/riscv/interrupt.c` selects `s_intr_handlers[core][mcause]` and
calls the registered C handler. A diagnostic-only linker `--wrap` brackets that
actual C call. Fresh-build checks require the linked vector to call the wrapper,
the wrapper to call the original dispatcher, and timer/wrapper code in IRAM.
Normal builds must contain neither the wrapper nor its diagnostic symbols.

USB HCD (`third_party/esp-usb-1.6.0/usb/src/hcd_dwc.c`) allocates its handler via
`esp_intr_alloc`. The pinned FreeRTOS `port_systick.c` allocates SYSTIMER tick
handlers the same way; `esp_timer_impl_systimer.c` allocates its target alarm.
These registered C handlers pass through the dispatcher. Generic `traceISR`
macros in `esp_hw_support/intr_alloc.c` do not cover all allocation paths and
are not used as a claim of complete IRQ coverage.

The recorded quantity is **core-0 C-dispatch wall time**, including the selected
handler, not complete ISR latency. Assembly entry/exit, context-switch exit,
exceptions/NMI, core 1, cache/bus stalls outside the dispatcher and measurement
overhead remain uncovered. This cannot establish pure CPU time. Scheduler-away
and IRQ windows may overlap and must not be subtracted together from the fill.

Each dispatcher invocation records an inclusive duration. Atomic depth/epoch
tracking detects nesting and torn snapshots without disabling interrupts. A
nested interval may be counted in both parent and child, so any nested event
in a sampled window makes its IRQ attribution invalid. Raw counts/time remain
visible; the analyzer never treats their sum as an exclusive IRQ union. TEE and
competing trace/SMP configurations are rejected. Early boot forwarding does
not read an uninitialized systimer.

## Validation checkpoint

- Windows pinned Python: 38 tests, 32 PASS and six explicit C skips.
- WSL: all six actual C cases pass with `-Wall -Wextra -Werror`; these cover
  scheduler calibration success/failure, valid/invalid stage distributions,
  saturation/boundaries, nested IRQs, wrong core, counter wrap, torn snapshots,
  coherent records and queue overwrite.
- The first dirty pilot was rejected by `Sources changed during build` because
  calibration was extended while it ran. It is retained as failed build
  evidence and is not a flash package. The initial TEE test also hit an earlier
  dependency guard; its full valid baseline and isolated negatives now pass.
- Rust/DSP source is unchanged. The six Rust feature profiles pass 39/2/56/57/63/68
  tests; format/Clippy and normal/profile RISC-V release build/Clippy pass.
  Source review, clean normal/profile packages, publication/CI and registration
  remain separate gates at this checkpoint.
- The initial CLI uncommitted review selected an overly broad main diff and
  rejected 471 files at its 150-file limit. It is not a passed review; the
  committed source must be reviewed against the exact `62717ce` parent instead.

Evidence root: `D:/BoardEvidence/JC1060-20261030-COM3/uac-cause-20261102/`.

### Source review correction

Source `7fc8bdb` passed fresh normal and profile IDF builds, including the actual
vector/wrapper/IRAM checks. CodeRabbit's exact-parent review found one minor
but valid analyzer defect: a truncated IRQ calibration line could omit time
and nesting fields yet grant attribution. The correction requires all scalar
calibration fields, unsigned bounds and interval geometry, and logs the IRQ
calibration window. Regression tests cover truncation at every added field,
negative/oversized/malformed words and impossible durations. Corrected-source
clean packages and a repeated review are required; the first packages are not
registered for flashing.

## Exact next physical gates

No COM3 access in this implementation batch. Before device entry: publish and
freeze reviewed source, clean-build both variants, verify all source/artifact
hashes, replay **31** predecessors, verify four factory backup copies and
register exact diagnostic/return plans with secondary copies. Expected current
reconstructed full state is
`817c75491f1c6f3b8e482b576ce44cec43693458d540685ede6adae392123e8d`.

Require fresh owner-assisted manual ROM entry and identity/security/RDID/full
predecessor equality before writing. Cold boot and a separate steady idle window
only: no controls, track loading or playback in the diagnostic. Keep the
aggregate >=6000 us rejection and <=3000 us headroom target; calibrations,
validity and distributions supplement them. If attribution is still partial,
register a later single-variable cache/compute control; do not guess a DSP or
priority fix. Restore exact s07, verify readback/cold boot and normal sound,
Pause/CUE before stopping. I02–I13 and target recovery timing remain NOT_RUN.

## Clean corrected-source registration

Published source `3e20a8e64ef7f2ff855bf18706712e289c6cc747`, [PR #76](https://github.com/dvucinozd/Ruzina/pull/76)
above #75. Both corrected-source clean packages pass all link/isolation/IRAM
and stack gates, with empty dirty status, 282 source hashes each and 16 artifact
hashes verified. The correction passes 33 Python tests; all six actual C cases
cover the six explicit Windows skips. Rust feature/target evidence above is
unchanged. No new target result is claimed.

| Package | App bytes | App SHA-256 |
|---|---:|---|
| clean-normal-r2 | 817344 | `22b9af7c4816b3270226e9ef0185758763604c73b687bf2af6c34ab3340ffe58` |
| clean-profile-r2 | 823072 | `562788ee5fd8de05d6bad98af44f1e8b4e299c2eed2fd056f6b2c1430e65db35` |

Normal/profile ELF text: 791924/797440 bytes; data: 25290/25506; BSS:
544092/545084. The profile adds 5516 text bytes and 1208 static RAM bytes.
The snapshot is 1172 versus normal 208 bytes (964 extra queue payload bytes);
the profile logger stack adds 1024 bytes. Same RTOS control blocks. Direct Rust
fill stack estimates are 3120 normal / 3104 profile bytes, load 73616 bytes;
these exclude indirect/C/kernel frames. Runtime stack high-water remains open.

The full-source repeated review of `3e20a8e` found only an evidence-format issue:
a CLI text prefix made its `.ndjson` name misleading. Both affected raw streams
are now preserved byte-for-byte as `.raw.log`, with separate strict
`.events.ndjson` files and a [hash index](evidence/2026-11-02-uac-cause/review-stream-index.json).
The reviewed analyzer defect is fixed; no raw evidence bytes were stripped.

Offline ancestry replay verifies all 31 predecessors, preserved tails and four
factory backup copies. [Diagnostic plan](../boards/readiness/jc1060-uac-cause-profile-write-recovery-plan-20261102.json)
SHA-256 `fae4606a5e23b7d1bb6ec88f5045057d41184c97f90b119608ce1bf672d56528`;
[s07 return plan](../boards/readiness/jc1060-uac-cause-profile-return-s07-write-recovery-plan-20261102.json)
SHA-256 `45f4386dc026aeeaf83e16c8e788c613a4d0179f39339cf6bc19f13f5e5771bb`.
Both plans, full packages and artifacts have matching secondary copies.

The [registration](evidence/2026-11-02-uac-cause/diagnostic-registration.json)
and [archive index](evidence/2026-11-02-uac-cause/registration-archive.json)
record the current 79-file, 15142719-byte v3 ZIP with verified CRC, all member SHA-256 values
and equal secondary SHA-256
`960865fa94b559abbedc6c4b6bfaf6d9702d88270d9339fd1cd95d214b10f357`.
Secondary root:
`C:/Users/Daniel/Documents/BoardBackups/JC1060-20261030-COM3/uac-cause-20261102/`.
Final evidence-format review, CI and publication are tracked independently of
clean-build success. Device work stays disarmed until those gates close and the
owner performs fresh manual ROM entry.

The registration review identified the redundant mixed-format `.ndjson` members
inside the first archive as well. The v3 archive retains raw logs and strict
event files, excludes only those redundant mislabeled members, and parses every
remaining NDJSON line during verification. The original v1 archive is retained
at its recorded hash; v2 stopped on a revised-tooling secondary-copy mismatch
and is not registered. The earlier secondary tooling version is preserved.
The reviewed analyzer and both evidence-format findings are resolved; no
firmware source changed after the corrected clean builds. Archive CRC/content,
strict NDJSON and secondary equality pass independently of CI.
