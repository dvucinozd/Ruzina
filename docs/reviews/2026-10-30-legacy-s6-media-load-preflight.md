# Shared Library load preflight for the IDF media owner

Date: 2026-10-30. Branch: `ogranak/jc1060-media-load`, from main
`5adb8b8de57481786863a5ef4976a927e9cd3a4d`. Host and build work only: no COM3
access, flash or new device image. Native minimum 302 and APTA are unchanged.

## Decision

The native Library load lane kept its preflight inside the firmware app
(`library_deck_owner.rs`), tied to the USB0 broker client. The owner chose to
extract it into a shared crate rather than write a second IDF-only copy, so
both platforms run one implementation.

## What changed

- New `ruzina-media-load` (no_std, no unsafe): `validate_audio_source`,
  `load_source_analysis` (DAT, optional EXT, source cues) and the
  `LoadFiles` trait. The logic moved unchanged: a missing or invalid DAT
  yields an empty source bank, a rejected EXT keeps the DAT analysis, and a
  publication withdrawn mid-scan publishes nothing. `FsLoadFiles` adapts any
  `AsyncFileSystem` and checks the live lease around every operation.
- Native firmware: `library_deck_owner.rs` implements `LoadFiles` for its USB0
  client (`Usb0LoadFiles`) and calls the shared preflight. Broker retry and
  invariant-reset handling stay in the app. `library-deck-load` enables the
  crate.
- Legacy IDF core: the `media-load` feature adds `media_load::preflight` over
  the shared exFAT volume. It returns analysis only if the lease and catalog
  publication are still current after the last read. The exFAT test image
  builder is shared through `formatted`/`setup_with`.

## Evidence

Local runs in WSL Ubuntu with Rust 1.95.0:

| Command | Result |
|---|---|
| `cargo fmt --all --check`, firmware and legacy core `cargo fmt --check` | PASS |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | PASS |
| `cargo test --locked --workspace` | 944 passed, 0 failed, 0 ignored |
| `cargo check --locked -p ruzina-media-load --target riscv32imafc-unknown-none-elf` | PASS (no_std) |
| m1-firmware `cargo check --locked --features integration-check,library-deck-load` | PASS |
| Same with `wav-audio-source-bringup`, `pcm-transport-bringup`, `render-graph-bringup`, both `sdmmc-profile-storage-bringup` pairs, `integration-check`, `board-jc1060`, `board-jc8012` | PASS |
| Legacy core `test --features idf-exfat-fixture,media-load` | 35 passed (29 earlier + 6 new) |
| Legacy core host clippy with `media-load`; target clippy and release build with `idf-exfat-fixture` | PASS |

Lockfiles add only the local `ruzina-media-load`, `ruzina-media-catalog` and
`ruzina-rekordbox` crates; no registry package changed. Remote CI results
belong to the PR's head.

Six shared tests cover audio stat rules, DAT+EXT import into source cues, a
missing DAT, a rejected EXT, a publication withdrawn mid-scan and media removed
mid-scan. Six IDF tests use a real exFAT image (audio, DAT and EXT) read through
`ReadQueue`/`ReadClient`: a full import, source errors, a withdrawn
publication and a foreign lease, removal during a paused DAT read, cancellation
followed by drain and a reload under a new lease, and a warm volume cache that
serves no load and performs no IO after removal.

## Limits and next gates

- The [IDF allocator follow-up](#idf-psram-allocator) gives `media-load` a
  target build. No IDF profile selects it yet; a separate opt-in profile is next.
- PDB catalog import is still native-only (`usb0_library.rs`). The IDF path
  needs the same extraction before it can resolve real candidates.
- Deck load gate, persisted Hot Cues, audio-source preparation and the final
  commit remain with each platform's caller.
- No physical load, real export or audio run is part of this batch.

## IDF PSRAM allocator

Follow-up branch `ogranak/jc1060-idf-alloc`, stacked on this batch. The target
feature `idf-media-load` (`idf-exfat-fixture` plus `media-load`) registers
`idf_alloc::IdfPsram` as the staticlib's global allocator. Every allocation
calls `heap_caps_aligned_alloc(layout.align(), layout.size(),
MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT)` and is released with `heap_caps_free`.
The constants and the aligned-free contract were checked against
`esp_heap_caps.h` at the pinned IDF v6.1 commit `fff9895c`.

Policy (ADR-008): library and analysis data go to PSRAM, and internal SRAM
stays for DMA bounce buffers, queues and stacks. There is deliberately no
internal fallback. A profile selecting the feature must initialize PSRAM for
capability allocation, as the S5 memory profile does; the current SD profiles
do not. A failed allocation returns null; the allocation-error panic then
aborts into IDF reset policy. The buffers are CPU-only.

A host test replaces the two IDF functions with shims. It checks the requested
caps and alignment, an aligned pointer, release of every block, and a null
result for a refused allocation. Local results: 36 legacy core tests, host
clippy, and target clippy and release build for both `idf-exfat-fixture` and
`idf-media-load` pass. The archive's only new undefined symbols are
`heap_caps_aligned_alloc` and `heap_caps_free`. The allocator has not run on
the device, and PSRAM capacity under a real load is unmeasured.
