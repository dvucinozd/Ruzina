# Phase B execution log

Base remote main unchanged at `4a8c660183910b0950ae7ddabce9e8b395a2f10d`.
Phase A working changes preserved. No commit/push/flash performed.

| Task | Host result | Evidence / limit |
|---|---|---|
| B01 | Implemented | [HTTP contract](../WEB_INTEGRATION_CONTRACT.md) was written before API implementation; strict config/schema, sizes, status codes and cancellation policy |
| B02 | Implemented | no_std server leases, <=4096-byte chunks, exact lengths; nonblocking simulator body state, absolute 30 s deadline, EOF/error cancellation, one active upload |
| B03 | Implemented | HTTP config→SettingsOwner→RAM NOR→reload; HTTP S3CP→ProfileScanBuffers::install→RAM FS→rescan→activation validation→ProfileRuntime→semantic event |
| B04 | Host composition verified | Incremental signed local Push owner→SlotFlash→A/B boot model through real WebServer admission/stream/finish. Demo Session upload deliberately 503 until explicit trust/project/slot adapter is selected |
| B05 | Snapshots and bounded fixture tests implemented; real captures OPEN | Resources/diagnostics are owner snapshots with unavailable heap values and explicit simulated scope. Existing fixtures inventoried; no new real exports/controller captures available |

## Behavior and relevant files

- `ruzina-web/src/upload.rs`, `server.rs`, `http.rs`: duplicated profile headers
  refused; query parameters refused on management routes; leases cannot be
  reused after finish/cancel. Old cancellation cannot terminate a new lease.
- `ruzina-web/src/config.rs`: complete small JSON, no duplicate/unknown fields,
  escapes or trailing garbage; Debug is redacted. Empty/invalid body cannot clear.
- Simulator `management.rs`: public network fields omit the password. 200 means
  owner durable commit succeeded; newer network format returns 409 without writes.
- Profile installs preserve the shared swap/recovery algorithm; bad CRC/version/
  unsupported semantic mapping cannot replace the existing active profile.
  Profile initialization/activation in progress refuses new profile installs.
  Registry limit stays 16, profiles stay <=16 KiB. RAM futures are explicitly
  immediate; `profile_store::ready` must never wrap real device I/O.
- `ruzina-ota/src/push.rs`: constant 212-byte prefix storage and SHA state;
  signed header, project/chip, declared size and image header checked before
  erase; hash/descriptor checked before boot selection. Any failure/cancel after
  suspension aborts and resumes. Successful finish returns the existing core-0
  RebootPermit. Local signed rollback remains allowed; pull stays newer-only via
  the existing owner/transport tests. No production trust/key was changed.
- Resources reports byte units, unavailable heaps as null and actual owner
  elapsed time. Diagnostic log returns at most 16 typed events, sequence/time,
  overwritten count and truncation indicator; no raw request, password or URL
  is added to logs. Tiny response buffers fail closed via existing 500 handling.

## Regression scenarios

`desktop-simulator/src/tests.rs` and `web.rs` now cover:

- settings POST→GET→reload, Keep/clear policy, malformed request preserving data;
- write interruption at each of five record mutation budgets and future-format
  byte preservation; existing shared settings tests retain byte-cut coverage;
- second upload Busy, one-byte chunks, truncated finish, oversized chunk,
  stale write/cancel after a new lease, EOF and absolute deadline before commit;
- profile install/rescan and a real parsed controller event entering Session;
- invalid CRC preserves blob/registry; 20 HTTP/storage mutation budgets followed
  by recover_profile leave the old or new valid blob;
- real socket config upload/GET beside an idle client, demonstrating that the
  transport calls the management owner (not merely direct backend methods);
- log overflow, bounded fields and explicit simulation/missing measurements.

`ruzina-ota/tests/ab_lifecycle.rs` additionally covers HTTP one-byte signed
uploads, local older versions, candidate unpublished until finish, pending boot
and unconfirmed rollback, wrong trust/project/hash, partial cancellation and 15
flash operation budgets. It reuses the existing A/B device, keys and fixtures;
the synthetic ESP image pattern is not bootable firmware or descriptor evidence.

`ruzina-web::http::tests::bounded_mutation_seed_5eedb001_never_panics_or_allocates`
performs 2048 deterministic mutations plus bounded config parsing. This is a
bounded mutation test, not a full fuzz campaign. Fixture provenance/size/hash
inventory: [existing fixture manifest](2026-10-27-phase-b-fixtures.json).
Original controller/OTA README provenance is linked by each manifest record;
OTA bundles are CC0 synthetic data, controller goldens retain their original
donor/tool pins. No private music or new controller capture was introduced.

## Validation

WSL Ubuntu, Rust 1.95.0, Node 22.23.3. Logs:
`/home/daniel/.cache/ruzina-review-20261027/phase-b-*.log`.
All commands run from `firmware-rust`.

| Command | Result |
|---|---|
| `cargo fmt --all --check` | PASS |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | PASS |
| `cargo test --locked --workspace` | 938 passed, 0 failed, 0 ignored (first complete phase B run) |
| `node --test crates/ruzina-web/web/tests/app.test.cjs` | 6 passed |
| `cargo run --locked -p desktop-simulator -- --smoke target/ui-smoke` | PASS both geometries |
| `python3 tools/check_ui_goldens.py target/ui-smoke` | 64 passed, baselines unchanged |
| `cargo check --locked -p ruzina-web -p ruzina-ota -p ruzina-settings -p ruzina-uac --target riscv32imafc-unknown-none-elf` | PASS |

Final repeat after preserving the existing RebootPermit return contract also
passed: **938 passed / 0 failed / 0 ignored**, Clippy and P4 contract check PASS.
Clippy and target checks were additionally executed directly with confirmed
exit code 0. Documentation checker: **96 links passed**. Cargo.lock changes contain only
new local workspace dependency edges, no registry package/version upgrades.

## Remaining gates

- Configure the actual product OTA trust/project/inactive-slot/descriptor adapter
  before enabling uploads in the demo or board runtime. The test key is never
  selected implicitly and no new production key/project identity is invented.
- C6/TLS certificate/hostname verification, real persistent flash/SD, MIDI/LED
  activation and physical boot/rollback/audio stop/resume remain NOT RUN.
- Permitted real Rekordbox exports (nested/multiple media/ID collisions) and
  actual controller captures require source data/provenance; synthetic tests do
  not close these acceptance gates. No full external fuzz campaign was run.
- No Web management settings/profile/firmware browser pages were added. The
  routes are callable APIs; the existing playback Web page remains unchanged.
- No remote CI/build/package matrix or browser automation was run for these
  uncommitted changes. Phase A runtime-owner/P4 board results remain a separate
  earlier snapshot; phase B rechecks the changed no_std contract crates.
- B04 does not claim the demo Session has a working production OTA backend:
  it exposes firmware metadata and returns 503 for OTA uploads. HTTP→actual OTA
  owner→fake slot→boot/rollback is executable in the integration suite.

Next: review and persist this local phase A+B batch when authorized, then select
board OTA composition and acquire the missing physical/provenance evidence.
Boards still need the ordered phase C backup/recovery/bring-up procedure.
