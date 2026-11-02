# Calibrated callback cause experiment

Status: IMPLEMENTED_OFFLINE / DEVICE_NOT_RUN. Branch
`ogranak/jc1060-uac-cause-profile`, parent `62717ce6b04b62cd5f1977b1bdfe5a91adcabf4f`.
Continues the [cause plan](2026-11-01-uac-callback-cause-follow-up.md) after the
[prior measurement and verified s07 return](2026-11-01-uac-callback-profiling-device.md).
This is diagnostic attribution, not a timing fix. Native minimum remains 302;
the observed JC1060 v1.3/103 stays on accepted s07 until the new registered run.

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
