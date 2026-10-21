# APTA observations, repeated lifecycle checks and Guition first-boot packages

Date: 2026-10-21. Base: `db99f8b9b2467813672edec0c22248feb78e3199` (PR #19).
Branch: `ogranak/apta-observability-firstboot`.

This completes the three approved software tasks: visible analysis observations,
repeatable service stress checks, and independent hashed diagnostic packages for
JC1060P470C_I_W_Y / JC8012P4A1C_I_W_Y. It does not activate Guition peripherals,
flash any device, change audio routing or merge the original APTA branch.

## 1. State and failure observations

`pajoniiir-service::analysis` provides an allocator-free, provider-neutral scalar
Sample/Observation/Monitor. The actual product supervisor emits Waiting,
Analyzing, Paused, Deferred, Published, Rejected, Failed, RecoveryBlocked and
Quarantined, with Disabled as the default. Deck/load generation are diagnostic
correlation only, never a replacement for media/request authority. No worker
bank, backend error object or mutable catalog borrow crosses the observation.

The portable service classifies source failures into storage, format, fixed
capacity, configured work limit, contract, stale, analysis and generation
exhaustion categories. The concrete port and host service harness share the same
mapping. Startup allocation and endpoint claim failures remain distinct. A
Published observation follows acceptance by today's ProductRuntime, not merely
a queued worker output. Pending completed results are Deferred; cancellation
and busy transport are Paused. Full recycle ownership remains visible.

The combined runtime-health view includes `M1_ANALYSIS_OBSERVATION_V1`; reading
or re-observing an unchanged sample never renews its transition timestamp. It is
not a heartbeat, percentage, ETA, real-time deadline or hardware acceptance. The
bounded parser rejects missing/duplicate/unknown fields, unsupported labels,
invalid deck/load pairs, zero load identities, oversized and truncated records.
Existing owner publication/freshness contracts remain separate.

Health → Analysis in the desktop simulator displays a captured record, with
state-specific severity and a reason/recovery explanation. It does not start or
depend on the native worker. For example, after a host stress capture:

```sh
cargo run --locked -p desktop-simulator -- --board jc8012 --analysis-observation /absolute/stress/native-observation-001.txt
```

This option expects a standalone bounded analysis frame, not an entire mixed
serial log. Captured state remains captured when the UI refreshes. The ordinary
host media summary does not turn a captured fault green. Source-clock transition
time is labelled explicitly. Default provider exclusion remains enforced.

The actual compiled Slint UI smoke exercises eight captured states at 1024x600
and 800x1280, including pointer navigation to the new tab. Fourteen existing
Health screenshots change because of the added tab; 24 other screenshots remain
byte-identical. Sixteen new analysis views make 54 reviewed goldens. Focus is
restored after the isolated replay so other product pages keep their baseline.

## 2. Repeated real-service lifecycle checks

The USB0 host harness source-includes today's ProductRuntime, transport state,
supervisor, broker priority/read-cap and reservation guard. New tests run:

- 64 pending-read cancel/reload cycles on the same owner, client and fixed work/
  output banks, then successful publication and 64 turns of refused recycling;
- 32 independent detach-during-I/O client/media lifetimes with no stale result;
- malformed source, configured object limit and a just-over-capacity WAV, with
  the actual failure category visible and no hot-loop retry.

Each fixture explicitly shuts down its test service, drains/closes the client,
retires the original 512-byte broker allocation and verifies pointer identity
before freeing it. Open/close counts and bounded queues are checked. This is
host ownership evidence, not proof of physical buffer/DMA behavior.

Run the reproducible release campaign with a NEW output directory:

```sh
python3 tools/run_apta_stress.py --iterations 25 --output /absolute/new/stress
```

It compiles the actual integration-test executable, records its SHA-256, all
firmware source hashes and the pinned build command, then runs both lifecycle
regressions in each iteration with a finite timeout. The default campaign totals
1600 cancel/reload cycles and 800 detach lifetimes. It requires actual cycle
markers and two passed tests per iteration. Every log and the actual final
supervisor observation are retained and hashed. Source/binary changes during the
campaign fail the run. Duration is host elapsed time only; CPU scheduling,
latency, high-water and musical-accuracy acceptance on ESP32-P4 remain NOT RUN.

## 3. Separate Guition first-boot packages

`tools/guition_first_boot.py build` builds both board-diagnostic configurations
separately, copies each ELF immediately, verifies its embedded image ID, and
creates per-model manifests, build logs, ZIPs and raw SHA-256 sidecars. It captures
commit, dirty state, exact input hashes, compiler, relevant build environment,
command and working directory. Cargo runs under the firmware's `.cargo/config`
so target linker flags and the silicon policy are applied. Changed inputs or
build failure retain BUILD_FAILED evidence. Existing output cannot be replaced.

`ingest` retains an existing raw log; optional `capture` reads an explicitly
selected port with deasserted DTR/RTS, no writes/reset commands, and finite time/
data bounds. A serial failure preserves partial bytes and its explicit error.
Only live capture needs pyserial. No port scan, flash command, erase, partition
selection or reuse of custom-M1 flash layout is included.

Results belong to one named unit and scenario: factory, cold boot, warm reset,
service reconnect or ROM recovery. A complete application record must match the
package's model, image label, minimum silicon policy and documented capacities.
Unmeasured memory stays unknown. Duplicate/mixed/inconsistent records fail
closed. Partial repeated frames and raw reset reasons are retained. ROM signature
presence is recorded separately. Optional operator PASS/FAIL requires a note and
is attributed to the operator; it does not turn a repeated log into automatic
power-cycle/recovery acceptance. Readiness/scenario acceptance stay NOT_QUALIFIED.
A matching image label is not proof of exact flashed bytes; programming/readback
and factory-backup evidence remain separate requirements.

See [the updated unit procedure](boards/GUITION_FIRST_BOOT_2026-10-20.md) for exact
commands. Packages and captures are outside the tracked source tree. CI prepares
and retains build-only packages; account-side CI failures are separate from local
build/test evidence.

## Evidence and remaining gates

The evidence manifest pins source hashes, review scope, tests, stress outputs,
UI review and per-model package manifests/ELF hashes. Actual local packages are
retained separately; no binary release or device flash is performed.

Required local checks cover workspace/default tests, optional adapter release
and Clippy, 15 supervisor/policy tests, 20 composed owner tests, seven Python
package/capture regressions, 54 UI goldens, both portable RV32 profiles, all 36
P4 check/link workflow profiles, 27 invalid board guards and dependency exclusion
with explicit opt-in presence. The stress campaign and both real diagnostic
builds are independent evidence. Test fixtures are explicitly not unit records.

Still NOT RUN: physical first boot/ROM recovery, observed flash/PSRAM capacity,
factory readback, Guition panel/touch/USB/audio activation, allocation high-water,
physical I/O timing and backfeed, reconnect stress on devices, concurrent
analysis/playback, corpus accuracy, sanitizers and listening/soak. Native analysis
remains an explicit legacy-only feature and cancels during playback. Headphones
remain controller-only. The original APTA branch and historical KiCad edits stay
preserved.
