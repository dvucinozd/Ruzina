# Ruzina Rust Product Requirements

**Status:** Guition / M2.5 requirements baseline; updated 2026-10-22
**Recorded:** 2026-09-24  
**Target:** Guition JC1060P470C_I_W_Y and JC8012P4A1C_I_W_Y, ESP32-P4 v3.2
**Software direction:** native Rust, bare-metal/no_std application architecture

## 1. Purpose and authority

This document defines the product-level acceptance requirements for the Ruzina Rust firmware.

The authorities are intentionally split:

1. **Released Pajoniiir M2.5** (`20f1c3f04a615209ae25e9bbdae649d0f5b44e8d`) supersedes the original M2.2 functional/product reference. M1 Rust must preserve or deliberately supersede its released DJ behavior. The [parity plan](M2_5_PARITY_PLAN.md) separates stable R1 gaps, later shared-P4 repairs and optional features that were disabled/unqualified in the donor.
2. **Exact selected Guition schematics and per-board BSP contracts** define electrical integration. Both models require unit identification, resource ownership and physical qualification.
3. **Pajoniiir-M3** is supporting hardware/integration evidence only; it is not the feature-parity target.
4. **Current Rust ecosystem documentation** defines the initial implementation baseline but does not override product behavior.

M1 Rust R1 is not accepted merely because it boots or compiles on ESP32-P4. It is accepted only after all MUST requirements have equivalent regression evidence and all hardware-dependent MUST gates have been physically qualified.

## 2. Frozen starting baseline

Initial dependency/target assumptions as of 2026-09-24:

- ESP32-P4 silicon v3.2 or newer approved revision; the owner reports v3.2 for both selected Guition models. Exact package, memory and board revision must be recorded per unit.
- Rust target: `riscv32imafc-unknown-none-elf`.
- `esp-hal 1.2.2`, with ESP32-P4 support for chip revision >= v3.x.
- `esp-hal` P4 USB, MIPI DSI, I2S, SDMMC and PSRAM APIs are treated as unstable integration surfaces.
- Slint `1.18.1`.
- embedded-graphics `0.8.2`.
- Rust toolchain baseline `1.95.0`, matching the current esp-hal 1.2.2 minimum Rust version.
- ESP-IDF application runtime / FreeRTOS / esp-idf-sys are not the permanent M1 application architecture.

All exact dependency versions and `Cargo.lock` shall be committed before an executable firmware baseline is accepted. Any esp-hal minor upgrade is a qualified platform change because M1 relies on unstable P4 peripheral APIs.

## 3. Verification vocabulary

Every requirement/evidence item shall use one of these states:

| State | Meaning |
|---|---|
| HOST_VERIFIED | Deterministic host tests pass on supported desktop CI. |
| SIMULATOR_VERIFIED | Behavior is demonstrated in the desktop/UI simulator with golden evidence where applicable. |
| COMPILE_VERIFIED | Correct target build/link succeeds, but physical behavior is not claimed. |
| HARDWARE_PENDING | Requires M1/P4 hardware or attached peripheral hardware. |
| HARDWARE_VERIFIED | Physical acceptance procedure passed and evidence is recorded. |

No requirement may be promoted from COMPILE_VERIFIED to HARDWARE_VERIFIED by inference.

## 4. Priority classes

- **MUST** — required for M1 Rust R1 product release.
- **SHOULD** — intended for R1 if it does not compromise MUST closure.
- **LATER** — explicitly deferred and must not delay R1.

## 5. MUST requirements

The table's evidence column specifies the required verification level, not a
claim that every requirement is implemented. General settings persistence,
OTA/rollback and networking/Web Remote remain unfinished. Current implementation
and open gates are recorded in [development status](DEVELOPMENT.md) and
[validation scope](VALIDATION.md). Host results never establish device acceptance.

### 5.1 Platform and board contract

| ID | Requirement | Required evidence |
|---|---|---|
| M1R-P0-001 | Native Rust application architecture on ESP32-P4; no permanent ESP-IDF app runtime. | COMPILE_VERIFIED |
| M1R-P0-002 | M1 BSP owns all board-specific GPIO, rail, USB VBUS, display, touch, audio, SD and C6 definitions. Product crates must not contain raw M1 GPIO numbers. | HOST_VERIFIED |
| M1R-P0-003 | Enforce P4 minimum production revision policy consistent with M1 v3.2+ hardware target. | HOST_VERIFIED / HARDWARE_PENDING |
| M1R-P0-004 | Preserve safe boot state: audio muted, host power enabled only by the board-qualified policy, display brightness initially off. | HOST_VERIFIED / HARDWARE_PENDING |
| M1R-P0-005 | Preserve UART0 and USB Serial/JTAG service/recovery paths. | COMPILE_VERIFIED / HARDWARE_PENDING |
| M1R-P0-006 | Hardware abstraction must isolate unstable esp-hal APIs behind platform/BSP crates. | HOST_VERIFIED |

### 5.2 Product behavior parity

| ID | Requirement | Required evidence |
|---|---|---|
| M1R-P0-010 | Two independent decks with deterministic load/play/pause/cue lifecycle. | HOST_VERIFIED |
| M1R-P0-011 | MP3, WAV and FLAC playback behavior equivalent to released M2.5, with explicit unsupported-format errors and decoded duration/EOF independent of catalog length and analysis span. | HOST_VERIFIED |
| M1R-P0-012 | Bounded, seekable compressed-media cache; no unbounded per-track allocation. Cold MP3 seeking and complete-file duration discovery have explicit I/O/time/memory budgets and cancellation. | HOST_VERIFIED |
| M1R-P0-013 | Hot Cues, loop-cue recall, separate memory cues, source restore/local overrides, loops/reloop, Beat Jump and Sync preserve M2.5 semantics unless a documented successor policy intentionally changes them. | HOST_VERIFIED |
| M1R-P0-014 | Jog, explicit per-deck VINYL/CDJ, CUE hold-preview/release/PLAY commit, tempo and Master Tempo preserve released semantics. | HOST_VERIFIED |
| M1R-P0-015 | Mixer path preserves trim, 3-band EQ, filter/Pad FX/Beat FX, channel fader, crossfader, master volume/trim and MAIN limiter behavior. | HOST_VERIFIED |
| M1R-P0-016 | PFL/cue branch occurs before channel fader/crossfader and MAIN-only limiter behavior remains separate. | HOST_VERIFIED |
| M1R-P0-017 | Beat FX set includes FILTER, ECHO, FLANGER and DELAY with beat-time derivation behavior covered by tests. | HOST_VERIFIED |
| M1R-P0-018 | Smart CFX and Smart Fader remain available. | HOST_VERIFIED |
| M1R-P0-019 | Scratch source has explicit priority over normal source progression and cannot corrupt the canonical timeline. | HOST_VERIFIED |
| M1R-P0-020 | Internal timing/frame counters use wrap-safe/64-bit-safe semantics appropriate for long sets. | HOST_VERIFIED |
| M1R-P0-021 | Default-on load lock is enforced through shared touch/controller/Web admission and asynchronous source publication; a refused/raced LOAD preserves the authoritative session. | HOST_VERIFIED |

### 5.3 Controller platform

| ID | Requirement | Required evidence |
|---|---|---|
| M1R-P0-030 | Controller handling is semantic and data-driven, not hard-coded to one controller. | HOST_VERIFIED |
| M1R-P0-031 | S3CP v2/v3/v4 are readable end to end, preserving legacy bytes and adding center-preserving CC7 scaling, NOTE_SELECT, bounded initial output SysEx, LED scaling and profile-owned filter policy. The compiler emits the oldest representable format; incompatible semantic actions fail closed. | HOST_VERIFIED |
| M1R-P0-032 | Profile validation includes magic/version/length/bounds/CRC checks and fail-closed activation. | HOST_VERIFIED |
| M1R-P0-033 | Preserve FLX4 built-in/profile parity fixtures. | HOST_VERIFIED |
| M1R-P0-034 | Preserve Hercules DJControl Inpulse 500 and Generic MIDI profile compatibility at host-test level. | HOST_VERIFIED |
| M1R-P0-035 | Reconnect/rebind releases held state, reconciles authoritative state and replays only explicitly replayable absolute controls. | HOST_VERIFIED |
| M1R-P0-036 | Semantic LED state is separate from controller-specific MIDI output mapping and is fully resynchronized after reconnect. | HOST_VERIFIED |
| M1R-P0-037 | USB MIDI transport and profile runtime are separate crates/interfaces. | HOST_VERIFIED |
| M1R-P0-038 | Controller support claims, including FLX4 on the new boards, require exact board/controller/image MIDI/LED/UAC qualification. Donor acceptance and software fixtures alone do not establish physical support. | Documentation gate |
| M1R-P0-039 | USB audio capability must become profile/capability-driven instead of remaining FLX4-hardcoded. | HOST_VERIFIED / HARDWARE_PENDING |

The generic controller-audio capability model shall be able to express at least: USB interface, alternate setting, OUT endpoint, optional feedback endpoint, PCM format/bit depth, channel count/map, supported sample rates, headphone L/R mapping, synchronization mode and device quirks.

### 5.4 Storage, media and library

| ID | Requirement | Required evidence |
|---|---|---|
| M1R-P0-040 | The qualified High-Speed USB path is primary mass storage. Each BSP owns root/PHY/controller roles, power and FIFO budgets; no fixed USB0 numbering or donor port swap is assumed. | COMPILE_VERIFIED / HARDWARE_PENDING |
| M1R-P0-041 | FAT32 and exFAT media are product requirements. | HOST_VERIFIED / HARDWARE_PENDING |
| M1R-P0-042 | Superfloppy, MBR and GPT partition discovery remain supported. | HOST_VERIFIED |
| M1R-P0-043 | Disconnect/reinsert and failed-media recovery are generation-safe and cannot leave stale deck/library pointers active. | HOST_VERIFIED / HARDWARE_PENDING |
| M1R-P0-044 | Stable media identity distinguishes identical Rekordbox IDs or paths on different media. | HOST_VERIFIED |
| M1R-P0-045 | Track identity is derived from stable volume identity plus normalized relative path, with content SHA-256 available for cache invalidation. | HOST_VERIFIED |
| M1R-P0-046 | Audio/deck logic consumes a media abstraction and does not depend directly on USB/FAT implementation details. | HOST_VERIFIED |
| M1R-P0-047 | microSD is a secondary/background medium and shall not violate the primary audio deadlines. | HOST_VERIFIED / HARDWARE_PENDING |
| M1R-P0-048 | Rekordbox Library preserves ordered nested playlist/folder membership and bounded artwork references; malformed hierarchy and stale selection/load work fail safely. | HOST_VERIFIED / SIMULATOR_VERIFIED |

### 5.5 Rekordbox and neutral analysis

| ID | Requirement | Required evidence |
|---|---|---|
| M1R-P0-050 | Existing Rekordbox PDB and ANLZ behavior remains compatible. | HOST_VERIFIED |
| M1R-P0-051 | Deck/UI/Beat Jump/Sync consume a provider-neutral immutable `TrackAnalysis` model, not Rekordbox-specific structs. | HOST_VERIFIED |
| M1R-P0-052 | Rekordbox titles, waveform/PWV3 timing/optional PWV4, BPM, downbeat numbering, separate memory cues and source cues have independent golden parity fixtures against released behavior. | HOST_VERIFIED |
| M1R-P0-053 | Analysis generations are immutable; a playing deck pins the generation it consumes until a safe upgrade boundary. | HOST_VERIFIED |

### 5.6 UI and waveform

| ID | Requirement | Required evidence |
|---|---|---|
| M1R-P0-060 | Slint is the M1 primary UI framework. | SIMULATOR_VERIFIED |
| M1R-P0-061 | Desktop simulator uses the same UI model and Slint views intended for M1. | SIMULATOR_VERIFIED |
| M1R-P0-062 | Main/detail waveform rendering is owned by a separate Rust waveform crate, not expressed as thousands of Slint UI elements. | HOST_VERIFIED |
| M1R-P0-063 | Waveform composition targets RGB565 first; custom span rasterization is allowed for hot waveform paths, embedded-graphics is used where useful for primitives/markers/grid. | HOST_VERIFIED |
| M1R-P0-064 | Slint and waveform rendering compose through an explicit framebuffer/compositor ownership model. | HOST_VERIFIED |
| M1R-P0-065 | Overview, Library hierarchy/artwork with safe fallback, Hot Cues/memory cues, Settings, deck status, mixer/FX state, beat indication, diagnostics and performance views reach functional parity with M2.5 UI. | SIMULATOR_VERIFIED |
| M1R-P0-066 | UI screenshot/golden tests cover stable representative states. | SIMULATOR_VERIFIED |
| M1R-P0-067 | Only the UI task/context may call Slint; other subsystems publish bounded messages/snapshots. | HOST_VERIFIED |

### 5.7 Audio real-time contract

| ID | Requirement | Required evidence |
|---|---|---|
| M1R-P0-070 | Deadline-critical audio path performs no filesystem I/O, network I/O, OTA work, controller-profile parsing or unbounded allocation. | HOST_VERIFIED |
| M1R-P0-071 | Audio queues/rings are bounded and expose underflow/overflow/late metrics. | HOST_VERIFIED |
| M1R-P0-072 | DSP has deterministic host fixtures/golden comparisons before hardware output is trusted. | HOST_VERIFIED |
| M1R-P0-073 | MAIN and headphone/PFL are distinct sinks. MAIN may use an external stereo DAC or compatible controller audio; headphones/PFL must always use the controller UAC headphone channels. No headphone fallback to an external DAC or onboard codec is permitted. | HOST_VERIFIED / HARDWARE_PENDING |
| M1R-P0-074 | Controller UAC health distinguishes idle underflow, temporary starvation, real active data loss, re-enumeration and clock drift. | HOST_VERIFIED / HARDWARE_PENDING |
| M1R-P0-075 | M1 hardware qualification includes mixed sample-rate dual-deck playback, Master Tempo, scratch, FX, MAIN and cue under sustained load. | HARDWARE_PENDING |
| M1R-P0-076 | UAC applies dedicated headphone gain exactly once, packetizes supported formats with bounded clock/backpressure/teardown ownership, and never credits stale output after MAIN sink changes. Unsupported audio preserves usable MIDI. | HOST_VERIFIED / HARDWARE_PENDING |

### 5.8 Settings, diagnostics and health

| ID | Requirement | Required evidence |
|---|---|---|
| M1R-P0-080 | Settings/state storage is versioned and migrated; raw Rust struct layout is never the persistent format. | HOST_VERIFIED |
| M1R-P0-081 | Persist at least released settings semantics: backlight, elapsed/remaining mode, cue mode, master trim preset and Wi-Fi remote enable state as applicable. Network/OTA settings update/clear is atomic and RAM publication follows durable success; credentials are redacted. | HOST_VERIFIED |
| M1R-P0-082 | Persistent Hot Cue state survives normal reboot and rejects corrupt/incompatible records safely. | HOST_VERIFIED / COMPILE_VERIFIED / HARDWARE_PENDING |
| M1R-P0-083 | Telemetry/service logging uses fixed-size/bounded producer paths and cannot block the real-time path. | HOST_VERIFIED |
| M1R-P0-084 | Telemetry reports board-observable power/USB state, memory reserves, recovery counts, audio/UAC metrics, storage faults, C6 link, OTA state and reset reason. | HOST_VERIFIED / HARDWARE_PENDING |
| M1R-P0-085 | Firmware health tracks running image identity, candidate/rollback state and explicit readiness. | HOST_VERIFIED |

### 5.9 OTA and recovery

| ID | Requirement | Required evidence |
|---|---|---|
| M1R-P0-090 | Signed application update bundles remain mandatory; preserve `.ddjota` compatibility where technically practical. | HOST_VERIFIED |
| M1R-P0-091 | A/B update, interrupted-transfer safety, rollback and wired recovery are MUST behavior. | HOST_VERIFIED / HARDWARE_PENDING |
| M1R-P0-092 | A new candidate is marked READY only after critical startup self-tests complete. | HOST_VERIFIED / HARDWARE_PENDING |
| M1R-P0-093 | OTA reboot path has a dedicated regression for the M2.2 post-update reboot panic class. | HOST_VERIFIED / HARDWARE_PENDING |
| M1R-P0-094 | Pull OTA strictly binds complete manifest/signature/app/board/project/version/size/hash, comparable newer-only versions and offer expiry, with verified TLS, bounded retries and phase diagnostics. Requested networking is part of candidate readiness; local signed service rollback has a distinct policy. | HOST_VERIFIED / HARDWARE_PENDING |

### 5.10 Web / Wi-Fi Remote

| ID | Requirement | Required evidence |
|---|---|---|
| M1R-P0-100 | Web, touch and physical controllers all produce the same semantic `ControlEvent` path. | HOST_VERIFIED |
| M1R-P0-101 | Wi-Fi Remote preserves released transport, waveform seek, library/status/diagnostics, controller-profile update and signed OTA capability. | HOST_VERIFIED / SIMULATOR_VERIFIED |
| M1R-P0-102 | Web API contract tests prevent browser state from diverging from authoritative device state. | HOST_VERIFIED |
| M1R-P0-103 | ESP32-C6 networking transport is isolated behind a platform/network interface so host tests do not require C6 hardware. | HOST_VERIFIED |

## 6. SHOULD requirements

| ID | Requirement |
|---|---|
| M1R-P1-001 | Integrate libapta behind the neutral `TrackAnalysis` provider boundary after its upstream embedded/release gates are satisfied. |
| M1R-P1-002 | Support ordinary MP3/WAV/FLAC folder libraries in addition to Rekordbox media. |
| M1R-P1-003 | Implement versioned `/RUZINA` sidecars/catalog/playlists with transactional writes and stale-part recovery. |
| M1R-P1-004 | Add bounded tag readers for ID3v2.3/v2.4, FLAC Vorbis comments and WAV RIFF/INFO. |
| M1R-P1-005 | Add factory firmware target for board bring-up and manufacturing/service tests. |
| M1R-P1-006 | Add property tests, fuzzing and fault injection for parsers, profile format, OTA manifest, media identity and storage write boundaries. |
| M1R-P1-007 | Qualify production Secure Boot v2 + Flash Encryption release provisioning on dedicated pilot hardware before batch enablement. |
| M1R-P1-008 | Add per-board power/current health thresholds where a qualified measurement path exists. |
| M1R-P1-009 | Select additional controller fixtures/capabilities explicitly, including idempotent latching PFL with controller-owned disconnect cleanup; qualify each real MIDI/LED/UAC path before support claims. |
| M1R-P1-010 | Evaluate Pro DJ Link discovery/browse/download/cache/sync after local playback/networking, behind peer/media epochs and qualified Ethernet; no assumption of an equivalent JC8012 network path. |
| M1R-P1-011 | Extend UAC to feedback/HS/UAC2/native mixer-volume capabilities only for explicitly selected controllers with descriptor, FIFO/clock and physical evidence. |

## 7. LATER requirements

| ID | Requirement |
|---|---|
| M1R-P2-001 | Audio recorder: retain design/test knowledge but do not make R1 release depend on recorder enablement until SD latency and power-loss behavior are qualified. |
| M1R-P2-002 | General input SysEx matching and MIDI-CI extensions beyond the note/CC profile model. The bounded initial output SysEx required by S3CP v4 is P0-031, not deferred here. |
| M1R-P2-003 | Excluded by the 2026-10-20 owner decision: controller-independent headphone DAC/codec/amplifier output. Headphones must always use the controller. |

## 8. Controller-only headphone policy

The owner clarified the mandatory headphone policy on 2026-10-20:

- Headphones/CUE/PFL always use a compatible controller's UAC headphone channels, regardless of the active MAIN sink.
- In automatic MAIN selection, a present and initialized external stereo DAC has priority; compatible controller MAIN audio is the fallback. If neither MAIN sink is ready, MAIN is muted.
- Switching MAIN must not reroute PFL or duplicate it into MAIN. Switching to controller MAIN requires distinct, verified MAIN/headphone channel mappings.
- Without a compatible controller headphone-audio capability, headphone output is unavailable and reported as such. MIDI-only control can remain available, and an external DAC can continue MAIN playback.
- Controller detach immediately invalidates its headphone route; reconnect restores it only after capability validation, stream initialization and fresh route binding. An external DAC or onboard speaker is never a headphone fallback.

The external MAIN DAC module, its detection/wiring, controller-specific channel mapping and physical qualification remain open. These are required routing semantics, not a claim that the current reference-sink firmware implements physical audio output.

## 9. Host-first development sequence

The pre-hardware sequence is:

1. lock this requirements baseline and architecture ADRs;
2. create Rust workspace and desktop simulator;
3. implement waveform renderer and golden images;
4. port semantic controller core and complete S3CP v2/v3/v4 compatibility;
5. port reconnect/state reconciliation/LED replay;
6. port deck/mixer/hot-cue/loop/Beat Jump/Sync logic;
7. introduce media identity and neutral TrackAnalysis;
8. port Rekordbox parsers/fixtures;
9. port audio/DSP as deterministic host-testable code;
10. add storage, OTA, Web API, settings and telemetry state machines;
11. add P4 BSP/driver adapters and compile gates;
12. implement each selected Guition BSP and execute exact-board bring-up.

This is the original dependency sequence, not a list of unfinished steps.
The [M2.5 parity plan](M2_5_PARITY_PLAN.md#execution-order) defines the current
implementation batches and distinguishes completed foundations from open gaps.

## 10. Hardware bring-up order

For each selected Guition model, qualify its actual peripherals and record an explicit disposition for unavailable hardware:

`unit identity / backup / ROM recovery -> boot/service -> flash/PSRAM -> DSI/backlight/touch -> SD -> USB power/data -> controller MIDI/UAC and external MAIN DAC -> C6 networking -> integrated stress -> OTA/rollback -> listening and release soak`

Each step creates recorded HARDWARE_VERIFIED evidence; later steps do not retroactively waive earlier failures.

## 11. R1 release gate

M1 Rust R1 is releasable only when:

- every MUST requirement is closed with the required evidence class;
- the M2.5-equivalent host regression corpus and selected later reliability regressions are green or superseded by documented stronger tests, with optional donor features explicitly scoped;
- desktop UI/waveform golden tests are green;
- exact Rust dependencies and toolchain are locked;
- unsafe code is confined to reviewed hardware/FFI boundary crates;
- no product crate depends directly on unstable esp-hal APIs;
- all required M1 hardware gates are HARDWARE_VERIFIED;
- OTA interruption, rollback and candidate-readiness tests pass;
- controller support claims match actual physical qualification evidence;
- release documentation lists remaining accepted limitations explicitly.
