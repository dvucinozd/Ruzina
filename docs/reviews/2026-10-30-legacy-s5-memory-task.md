# S5: bounded legacy PSRAM, task/FPU and EQ proof

Date: 2026-10-30. Unit JC1060-20261030-COM3, revision 103. Separate memory/task
profile; native minimum 302 and shared product crates were not edited.
The [S4 scalar baseline](2026-10-30-legacy-s4-runtime.md) is retained as an
immutable recovery image, not replaced in its evidence records.

## Implementation and scope

`sdkconfig.memory.defaults` enables only the opt-in `RUZINA_MEMORY_TASK` profile:
360 MHz CPU, HEX PSRAM 80 MHz, caps allocator, startup memory test, 1000Hz tick,
8192-byte main stack, no PSRAM XiP/instruction/rodata relocation. The scalar
and pure-RAM configurations still reject PSRAM. IDF's dynamic task allocation
uses internal 8-bit memory (`components/freertos/heap_idf.c`); no externally
allocated task stack is supplied. The permissive IDF external-static-stack
option is not used to allocate these stacks.

The probe allocates an exclusive 64 KiB external scratch span plus two canaries,
and an internal 4 KiB comparison buffer. Four passes cover 0x55/0xaa, relative
address-XOR and its inverse. A request larger than detected PSRAM must return
NULL without abort; all owned allocations are freed. Heap-integrity checks run
before/after; no raw-address writes, allocator replacement, DMA or peripheral
owner is introduced.

The optional Rust `memory-task` feature links existing `ruzina-audio-dsp`;
the separate lock pins libm 0.2.16 with its registry checksum. The FP function
accepts a float seed, count and borrowed aligned atomic activity flag. Its
disassembly shows `fmv.s`, atomic flag publication, a live `ft0` accumulator
and `fa1` increment, four million `fadd.s` iterations with no call/yield,
atomic flag clear and float return. Two priority 3 tasks pinned to core 0 use
different exactly representable integer seeds. Reading the other live flag on
the same core proves that task was preempted with a needed FP value live; its
eventual exact result checks restoration.32 calls per worker are followed by
an independent 32-call-per-worker test pinned to cores 0/1. This bounds proof
to these registers/operations; it is not an exhaustive FCSR/all-register test.

C owns distinct input/output buffers. The Rust synchronous pointer/length ABI
checks null, alignment, bounds, overlap and address overflow, never retaining
pointers or sharing Rust layouts. It borrows 256 stereo i16 frames, initializes
the existing EQ at 44100Hz with non-neutral band controls and processes the block.
64 blocks are measured with `esp_timer_get_time`, including EQ initialization;
the deadline was preset to 5804 us (floor of 256/44100 seconds). The benchmark
runs after FP workers complete. It does not establish timing under simultaneous
audio/USB/UI load, persistent EQ-state fidelity, full mixer or two-deck budgets.

## Build, admission and programming

All10 offline test methods pass, including new positive/negative memory-profile
guards. Rust fmt and target release Clippy pass. A first configuration attempt
correctly failed because CONFIG-dependent component requirements hid PSRAM's
Kconfig symbols; PSRAM is now an unconditional dependency with initialization
remaining configuration-gated. Earlier Rust Clippy alignment-style errors were
corrected without suppressing checks. Failed outputs were not qualified.

Accepted build: `D:/BoardEvidence/JC1060-20261030-COM3/s05-memory-task-3`.
App171392 bytes, SHA-256
`d160632d802b6a4c77d3ef47abc88a6bf6cf5df055af5944fbdd0ac5e251f0da`.
The map/header/ELF ABI and source hashes passed the fresh builder. Disassembly
is retained at `D:/BoardEvidence/JC1060-20261030-COM3/s05-fp-disassembly.log`.

The offline write planner overlays verified S4 sector payloads on the matching
factory backup to derive expected live bytes and recovery sectors. It verifies
the predecessor hash, payload bounds and hashes. Before S5 writing, live
identity/security/RDID and every affected preimage matched that S4 baseline.
Three reviewed sector spans were written with no full erase/eFuse programming,
then read back and SHA-compared. The recovery sector set returns to S4;
the original full factory backup remains a separate restore option.

## First physical observation

On the first USB-reset boot of this exact image:

| Check | Observed |
|---|---|
| PSRAM identity/capacity |33554432 bytes,80 MHz configured/reported |
| Scratch/canaries |Four passes, zero mismatches, canaries match |
| Allocation failure |Expected NULL |
| Same-core FP |Both workers core 0; zero mismatches; live-other observations 31/11 |
| Dual-core FP |Workers core 0/1; zero mismatches; live-other observations 30/30 |
| Worker stack remaining |Same-core 3620/3624 bytes; dual 3760/3764 bytes |
| Internal free/minimum |604096 before /595584 minimum bytes |
| External free/minimum |33551868 before /33484280 minimum bytes |
| Main stack remaining |7912 bytes |
| EQ benchmark |64 blocks, worst 353 us <5804 us; checksum 2527781824 |
| Buffer invalid inputs |Null, oversized, aliased and misaligned calls rejected |
| Runtime continuation |S5 match marker and 39 heartbeats; no HP watchdog |

This is bounded CPU-cached scratch/ABI/FP/EQ smoke, not full PSRAM capacity,
DMA/cache coherence, retention,200 MHz operation or physical audio acceptance.
Full factory restore remains NOT_RUN; product readiness remains
NOT_QUALIFIED.

## Repetition, regressions and handoff

Three console-triggered app restarts each reran the complete probe and passed:
[restart results](../boards/evidence/jc1060-20261030-com3/s05-app-restart-result.json).
Across first boot/restarts, zero FP/memory mismatches occurred, checksum stayed
2527781824 and the worst observed EQ block was 354 us. Same-core live-other
counts were nonzero on every run, with worker stacks at least 3620 bytes free.

The owner then disconnected the sole USB supply for 5–10 seconds and reconnected
without BOOT. The armed capture observed scalar match, S5 match and four
heartbeats: [cold result](../boards/evidence/jc1060-20261030-com3/s05-cold-boot-result.json).
The raw USB startup prefix may be missing. A previous monitor expired before
this owner action and recorded only ongoing heartbeats; that external log is
not classified as cold-boot evidence. The final cold run observed EQ353 us,
zero mismatch and no HP watchdog. The application did capture reset_reason1
(power-on); the ROM's earliest prefix is missing, so this is not a complete
raw power-on transition log.

Fresh scalar and pure-RAM regression builds both returned BUILD_PACKAGE_PASS
at `D:/BoardEvidence/JC1060-20261030-COM3/s05-scalar-regression` and
`.../s05-pure-ram-regression`. Neither regression image was loaded/flashed.
This confirms build isolation after adding the memory feature; the historical
S4 binary/evidence remain immutable. No full product suite/remote CI is claimed.

The [unit record](../boards/readiness/jc1060-20261030-com3.json) names the S5
current firmware, exact image hash and scenario evidence. S4 recovery payloads
remain available through the [S5 upgrade plan](../boards/readiness/jc1060-s05-write-recovery-plan-20261030.json).
The current pointer ABI is an experimental synchronous boundary, not yet a
product allocator/streaming API. Measurement under concurrent product load,
all-register/FCSR FP testing, DMA/cache coherence and soak remain separate work.
Next planned area is S6 BSP display/touch source/pin/identity review before
peripheral activation. PCB/module/panel markings are still unavailable.

Reference: [IDF FreeRTOS scheduler/affinity](https://docs.espressif.com/projects/esp-idf/en/v6.1/esp32p4/api-reference/system/freertos_idf.html)
and the exact local pinned Kconfig/memory/FreeRTOS sources, rather than importing
vendor 200 MHz/XiP settings. No vendor BSP/table or new peripheral code was added.

Final preservation: [S5 evidence index](../boards/readiness/jc1060-s05-evidence-package-20261030.json)
records a4,731,143-byte ZIP with 75 entries, all member hashes/CRC verified and
an independently hash-verified secondary copy. ZIP SHA-256:
`09d35077536fc72565de30bb0664b620e4bdde368b908c5718306eac75b9c765`.
It contains no factory dump. Final checks: 10 offline test methods PASS,
UNIT_RECORD_PASS,166 documentation links PASS, diff whitespace PASS.
No commit/push or remote CI was performed; current changes remain local.
