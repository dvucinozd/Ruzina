# Shared Rekordbox catalog import for native and IDF media owners

Date: 2026-10-31. Branch: `ogranak/shared-pdb-catalog`, from main
`53d912eca9a4d7362b66d87649fb878af2dc01bc` (after PR #57). Independent of the
allocator in PR #58. Host and build work only: no COM3 access, flash or new
device image. Native minimum 302 and APTA are unchanged.

## What changed

The native firmware kept its `export.pdb` import inside `usb0_library.rs`,
tied to the USB0 broker client. It now lives in `ruzina_media_load::pdb`, as
the load preflight did in [the previous step](2026-10-30-legacy-s6-media-load-preflight.md):

- **Scanning** (`probe_export_pdb`): header and table directory, the artist,
  album, key and artwork name tables, the track table, and the playlist tree
  and entries. It is moved from the native code by mechanical substitution:
  the same reads, bounds, M2.5 fallbacks (title recovery, a malformed artwork
  table dropped, out-of-row names skipped, invalid playlist rows counted) and
  error precedence. Scanning never allocates.
- **Traits**: `CatalogFiles` (open, positioned read, close under a lease),
  `PdbNameStore` and `CatalogStage` (stage rows, publish catalog and playlists
  together, abort). Errors are `ImportError<E>` with the platform's I/O error
  in `Io(E)` and stage errors in `Catalog(CatalogError)`.
- **`alloc` feature**: `VecNameMap` and `VecCatalogStage`, the native PSRAM
  stage moved unchanged. Every capacity is reserved by `try_new`, and two
  banks and two playlist stores alternate between published and building.
- **`FsCatalogFiles`**: `CatalogFiles` over any `AsyncFileSystem`, with the live
  lease checked around every operation, cache hits included.
- **Crate features**: `preflight` now gates the per-track preflight, whose
  analysis allocates. The PDB core has no allocation, so native firmware
  depends on the crate in every build.

Native firmware: `Usb0LibraryFiles` implements `CatalogFiles` over the existing
broker requests, and `Usb0LibraryError` converts from `ImportError`. With
`p4-psram-storage` the worker uses `VecCatalogStage`. Without it a stub stage
walks the export and keeps no rows, as before. The worker, browser mirroring,
lease statics and command handling stay in the app. The file shrinks from
2455 to about 800 lines. The first PDB page is now read with a seek to 0
instead of a read at the open position, which is one extra broker request.

Legacy IDF core: `media_load::import_catalog` imports `export.pdb` from the
shared exFAT volume through `FsCatalogFiles`.

## Evidence

Local runs in WSL Ubuntu with Rust 1.95.0:

| Command | Result |
|---|---|
| `cargo fmt --all --check`, firmware and legacy core `cargo fmt --check` | PASS |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | PASS |
| `cargo test --locked --workspace` | 951 passed, 0 failed, 0 ignored |
| `ruzina-media-load` riscv `cargo check`: no features, `alloc`, default | PASS (no_std) |
| m1-firmware `cargo check --locked`: `integration-check` alone and with `hotcue-flash-storage`, `usb0-hardware-bringup`, `usb1-controller-bringup`, `library-deck-load`, `wav-audio-source-bringup`, `pcm-transport-bringup`, `render-graph-bringup`, `sdmmc-profile-storage-bringup`, `p4-psram-storage` and the four PSRAM/SD pairs; `board-jc1060`, `board-jc8012`, `board-jc1060,board-diagnostics` | PASS |
| m1-firmware `cargo build --locked --release`, default and `board-jc1060` | PASS |
| Legacy core `test --features idf-exfat-fixture,media-load` | 37 passed (35 earlier + 2 new) |
| Legacy core host clippy with `media-load`; target clippy and release build with `idf-exfat-fixture` | PASS |

The clippy findings of the moved code (argument counts, `Result<_, ()>`) were
resolved with the native allowance and a `StageAllocationError`. The full
workspace test run preceded those type-only changes; the crate's tests,
clippy and the firmware checks were rerun after them. Lockfiles add only local
crates.

Shared tests use a synthetic `export.pdb` laid out like the existing
`ruzina-rekordbox` fixtures. They cover:
- tracks, artist names and playlists published under one publication;
- removal after each of 1 to 7 reads, which publishes nothing and returns all
  banks, so the next generation imports;
- a new lease retiring the old catalog, and invalidation;
- a missing tracks table or foreign lease;
- duplicate artist ids;
- the two first-page probe tests moved from the native firmware.

Two IDF integration tests read a real exFAT image with `PIONEER/rekordbox/export.pdb`
through `ReadQueue`/`ReadClient`. One imports the catalog, takes its track as
a candidate and passes the load preflight with analysis. The other removes the
card during a paused PDB page read and publishes nothing.

## Limits and next gates

- The IDF import is host-tested. On the device it needs the allocator from
  PR #58 and an opt-in profile with PSRAM initialized.
- Volume identity on the IDF path is a fixed test value. A profile must derive
  it from the exFAT volume (`ruzina.volume.exfat.v1`), as the native USB0
  owner does.
- No real export, physical import or audio run is part of this batch. The
  earlier 189-track owner export comparison was made with the unchanged
  parsers and is not repeated here.
