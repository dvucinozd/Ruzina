# Legacy MP3 in-place reset and bounded-stack opt3 build

This implements the next offline step from the [source-budget investigation](2026-11-03-source-budget.md).
No COM3 access, reset or flash occurred. Accepted s07/history **44** remain the
latest device readback state; application execution remains **NOT_OBSERVED**.
Native minimum **302**, native dependency resolution and normal compiler
defaults remain unchanged. These artifacts are **BUILD_ONLY_NOT_QUALIFIED**.

## Cause and change

The pinned MP3 wrapper previously assigned `nanomp3_core::Decoder::new()` in
both reset and seek. At core opt-level 3, the compiler materialized a **39008-B**
temporary. The existing const/static allocation path did not protect these
later runtime calls. The prior candidate failed the 24576-B frame guard and
its independent load-chain estimate exceeded 90112 B.

Legacy-only Cargo patches now use local copies of `nanomp3-core 0.2.0` and
`nanoaudio-mp3 0.1.1`, preserving versions, production source and licenses.
Only three upstream files change. The core's new `reset(&mut self)` restores
the constructor's values directly in caller-owned storage: overlap/filterbank,
reservoir/header/free-format state, scratch arrays and main-data buffer. It
also restores the constructor's default compatibility flag, matching the old
replacement expression. Exhaustive struct patterns ensure that adding a field
requires an explicit reset decision. All clearing uses safe field assignments
and slice fills; the core retains `forbid(unsafe_code)`. No zeroed object,
pointer cast, layout assumption, heap allocation or enlarged stack is used.

The wrapper replaces exactly its two core-construction assignments with the
in-place reset. Seek arithmetic, reservoir preroll, gapless trim, input-window
policy, wrapper state transitions, decoder algorithm, rings and scheduling do
not change. Initial constructors remain const for static initialization.
Runtime callers should use reset, not assume every by-value constructor has a
bounded stack under every optimization policy.

## Provenance and scope

Baseline commit `15f46c6` adds the copied upstream files before applying the
three-file reset patch. [Upstream file hashes](../../firmware-idf/legacy-jc1060/third_party/upstream-files.json)
pin the copied bytes. [Patch policy](../../tools/source_budget/vendor-patch.json)
pins original package checksums and exact edited bytes; `verify_vendor.py`
checks all **22 copied files**, exactly **3 edited files**, missing/extra files
and source/manifest drift. Original crate archives match the pre-patch lock:

- `nanomp3-core 0.2.0`: `63fa2238e662d3d3c1a71168b3278f23422cf9bf76f6943ee6b1c2581d74eb5c`.
- `nanoaudio-mp3 0.1.1`: `c35cae35635af5c7008c732f885ec9c3be6246b64ac508420cdab8ca801a2214`.

The copies retain published source/test text, manifests and license files;
registry cache markers, package lockfiles and upstream MP3 fixture assets are
not copied. Upstream fixture-dependent tests are **not claimed as run**. The
existing project CC0 codec corpus is instead run as an explicit legacy
integration target against the patched dependency graph. The native workspace
manifests/lock and registry dependencies are unchanged. No unrelated dependency
version update is included.

## Local validation

The new reset regression dirties a reusable decoder with real MP3 data, resets
it twice, and compares its PCM exactly with a fresh decoder across CBR, VBR,
mono, long VBR and source reuse. It also starts with the compatibility-mode
constructor to check restoration of default rounding. Both the default and
opt3 benchmark PCM files match the earlier unpatched reference byte for byte:
**352800 stereo frames**, SHA-256
`2fb60aa3d5dc7243d38208d215fa40c13083d062b51c5eeebca8a88c6fd3ddac`.
These host measurements do not establish target speed or playback acceptance.

- Producer diagnostic: **79 unit tests pass**, one ordinary-suite ignored
  manual benchmark; **24 shared codec integration tests pass**.
- Normal engine: **72 unit tests and 24 codec tests pass**.
- Opt3 release: **79 unit and 24 codec tests pass**; manual memory/delayed-read
  benchmark runs separately, five repetitions per reader and compiler policy.
- The shared corpus covers independent PCM references, short reads, exact CBR/
  VBR seek, gapless/preroll, EOF, dirty source reuse, index fallback and loops.
  Existing legacy tests retain cancellation/removal/transport coverage.
- Vendor/audit Python tests: **5 per OS**; host Clippy and formatting pass.

Normal and opt3 producer target builds both pass the unchanged frame/chain,
config, readonly-symbol and packaging guards. In both, the core reset's emitted
frame is **16 B**. The opt3 build's largest Rust frame is **16048 B**, below
24576 B; the normal build's maximum is 23776 B. Initial load-chain estimates
are 50704 B normal / 51728 B opt3, below 90112 B. USB-fill estimates are
3120 B / 3104 B, below 4096 B. These are static direct-call estimates with
the existing analyzer's indirect-call/recursion limits, not measured stack use.
Exact package/source hashes and the additional matched producer-control build
are retained with the final validation evidence. Dirty-checkpoint artifacts
and later clean-source packages must be identified separately.

## Clean package, review and archive completion

Source **`a2453350624bd6c207a921850354c5df4dd23316`** has completed CodeRabbit
review with **0 issues**, scoped from the separately verified upstream-copy
baseline `15f46c648837bedf47492dd9eb60a1be459fa680`. The [review record](evidence/2026-11-03-mp3-reset/review.json)
retains exact scope/raw digest. This is review of the actual reset/dependency/
test/provenance changes; original copied bytes are independently checked
against both locked crate archives.

[Qualification](evidence/2026-11-03-mp3-reset/qualification.json) verifies three
fresh packages from that exact commit, each with **empty dirty_status**, all
**313 source inputs** and all **8 artifacts** rehashed. Profiles are matched:

| Clean package | Producer profile | Core opt3 | App bytes | Load-chain estimate | Fill-chain estimate |
|---|---|---|---:|---:|---:|
| normal | OFF | OFF | 819328 | 50704 B | 3120 B |
| producer control | ON | OFF | 824320 | 51728 B | 3120 B |
| producer opt3 | ON | ON | 818784 | 51728 B | 3104 B |

Full app/ELF/manifest digests are in the qualification and retained manifests.
The initial fresh control attempt failed compiling IDF `pmu_sleep.c` without
compiler diagnostics; its cause is **UNKNOWN**. Its failed exit/log remain
separate. A new serial build directory, with identical committed source and
unchanged guards, produced the accepted clean control package. No failed
attempt was resumed/relabelled as successful. Earlier dirty checkpoints are
retained separately and are not the clean packages above.

[Archive](evidence/2026-11-03-mp3-reset/archive.json): `completed-reset-evidence.zip`,
**31466623 bytes**, **405 members**, SHA-256
`fe94bb235ffe7fb86fa8074af1c9e7933c0e590361e51f20d0689bf14db533dc`.
CRC, every member hash and all secondary copies match. Primary:
`D:/BoardEvidence/JC1060-20261030-COM3/reset-inplace-20261103/`; secondary:
`C:/Users/Daniel/Documents/BoardBackups/JC1060-20261030-COM3/reset-inplace-20261103/`.
The archive retains the earlier report checkpoint, source snapshots, three
clean packages, original crates, tests/PCM/review and failed attempts; this
subsequent publication records the completed archive and source checks.

[CI checkpoint](evidence/2026-11-03-mp3-reset/ci-checkpoint.json): exact source
Rust host workflow **#586 passed**; legacy workflow **#109**, including its new
opt3 job, is still **queued**. Local packages do not turn queued remote jobs
into PASS. New publication CI remains separate. No image is registered/flashed.

## Next gates

1. Complete exact-source review/CI and clean normal, producer-control and opt3
   package checks, source hashes and dual-copy archives. Any new stack/config
   failure stops preparation; unchanged limits remain mandatory.
2. Compare opt3 with a producer control containing the **same reset patch**.
   The old unpatched producer is not a compiler-only control. Do not attribute
   a difference to opt3 while changing reset code between those images.
3. Register fresh transitions from the exact executed **44-plan** prefix and
   accepted full s07 hash. The new image/source/patched-dependency/override
   identities need new plans/contracts and reviewed bindings; consumed 42/43
   plans and old byte-frozen writers do not authorize these packages. A paired
   control/opt3 campaign needs its full transition/return chain registered,
   not an assumed two-write ancestry.
4. Only after those gates, conduct bounded physical cold/idle/source cases,
   retaining actual readiness and an independent control audit. Stop on the
   unchanged first fault and automatically restore/readback accepted s07 with
   separately bound software boot. Do not repeat manual s07 recovery tests.

No improved P4 throughput, normal dual-deck/PFL isolation, reconnect or soak
acceptance, peripheral expansion, merge or product promotion is claimed.
