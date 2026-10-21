# M1 pinned Rekordbox analysis

Date: 2026-10-21

Base: `6ea7b977aa2ea0390bcbd578e99118affd78c234` on the open audio branch/PR #5. Analysis is isolated on `ogranak/rekordbox-analysis`, based on that branch. This implements the host-testable S5 import/publication path and its legacy target integration. It enables no Guition peripherals and merges no APTA code.

## Import and ownership

[`AnlzImporter`](../firmware-rust/crates/pajoniiir-rekordbox/src/anlz_analysis.rs) is an independent streaming implementation of the [Deep Symmetry ANLZ format](https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/anlz.html), cross-checked against its [Kaitai schema](https://github.com/Deep-Symmetry/crate-digger/blob/main/src/main/kaitai/rekordbox_anlz.ksy). DAT supplies `PQTZ` beatgrid, `PWAV` monochrome preview and eight-slot `PCOB/PCPT` Hot Cues/loops; optional EXT supplies `PWV3` detail. Wire phases 1..4 become neutral phases 0..3. Detail timing is exposed neutrally at 150 columns/second; the UI does not decode ANLZ.

Reads are at most 512 bytes. File size is capped at 8 MiB, section count at 256, beat count at 65,535, preview at 400 columns and detail at 131,072 columns. Known section sizes/counts, child records, grid ordering/phase/tempo, cue uniqueness and loop bounds are checked before publication. Invalid/short/wrong-offset input latches failure; a failed session cannot publish its partially built DAT state. Unknown sections are bounded and skipped. No parser or allocation runs inside audio render or semantic control handling.

[`AnalysisBuilder`, `AnalysisPublication` and `AnalysisPins`](../firmware-rust/crates/pajoniiir-track-analysis/src/owned.rs) own bounded vectors and expose immutable borrowed `TrackAnalysis` views. Reserve failures propagate as errors. Publications move through the existing Library completion channel rather than copying arrays or taking mutable shared references. At maximum capacities, retained beat/column payload is about 1.3 MiB per publication; two loaded decks plus a staging load can approach 4 MiB before catalog/UI/audio storage and allocator overhead. Actual PSRAM/resource/timing acceptance is still unrun.

Owned storage is opt-in. Host Rekordbox tests enable `analysis`; the embedded dependency disables default features and enables owned analysis only with `library-deck-load`, which already requires the PSRAM owner. Default/Guition diagnostic images do not acquire an allocator or analysis owner through this change. Both locks change only the local Rust graph, not external package records.

## Runtime and consumers

Each publication is bound to media lease, stable track ID, catalog revision and a nonzero load generation. `ProductRuntime` validates all identities before committing deck state and pinning analysis. A same-track reload requires a new token. Analysis generation is independent of PCM seek generation, so seek preserves the same song's immutable analysis.

The Library owner scans DAT and optional EXT through the existing async file broker, checks publication identity during reads and before commit, and closes handles on success/failure. A missing EXT preserves valid DAT preview/grid. A malformed present DAT/EXT rejects the combined analysis instead of promoting partial data; audio may still load with existing BPM fallback and persisted local Hot Cues. It does not reuse the previous track's analysis. The existing file-open API returns an optional handle, so an EXT open failure currently degrades like a missing EXT; it never claims a detail waveform is present.

Fresh media leases are rechecked at analysis view access as well as Library dispatch. Committed replacement, product source invalidation and media detach clear pins; already borrowed views cannot outlive their owner in safe Rust. An unrelated deck retains its own pin. Playing-track analysis remains tied to its loaded catalog revision rather than silently adopting a refreshed catalog's data.

Deck controls now call `handle_control_with_analysis`: Sync phase/tempo, Beat Jump and quantized loop operations receive the actual loaded analysis. Missing/rejected analysis retains the existing fallback behavior. Imported source Hot Cues feed the existing merge path, preserving local overrides and deletion tombstones. `TrackAnalysis` now supplies neutral preview/detail/cue views and a bounded detail window. `pajoniiir-waveform` reexports the neutral column type and renders these views directly into RGB565. Interactive Slint screen/compositor integration remains the next UI task.

## Validation and provenance

| Local gate | Result |
|---|---|
| Full workspace host tests | PASS, 554 tests including 18 import/lifecycle/consumer cases |
| Host format, all-target Clippy with warnings denied, desktop compile | PASS |
| P4 workflow matrix and partition validation | PASS, 29 check/link configurations |
| Invalid board/legacy/diagnostic combinations | PASS, 20 expected rejections |
| Public DAT/EXT corpus | PASS, three pinned pairs against independent decoding and released M2.2 |

Synthetic fixtures and their generator/hashes are in [`rekordbox/tests/fixtures`](../firmware-rust/crates/pajoniiir-rekordbox/tests/fixtures/). They exercise complete headers, multi-read detail, unknown sections, memory-container skipping and source Hot Cues. Corruption cases cover malformed container/count/stride/phase/tempo/ordering/cue loop, duplicates, incomplete reads and resource caps. Consumer fixtures run import → pin → Sync/Beat Jump/quantized loop and waveform rasterization, plus source cue/local override merging.

Three public DAT/EXT pairs from [Holzhaus/rekordcrate at `14d54ed`](https://github.com/Holzhaus/rekordcrate/tree/14d54eded7b0d2f6fe67c7871533dca35a73e5fa/data) yield 257/368/257 beats, 400 preview columns each and 22,500/25,866/19,208 detail columns. Counts, BPM and normalized full beat/waveform hashes match an independent Python wire decoder and the released C parser from [M2.2 commit `2c2ec32`](https://github.com/dvucinozd/Pajoniiir/tree/2c2ec32c253d368765123d7bbf8d37389b790b55/firmware/main-deck-p4/components/library). C phase values are normalized to the neutral schema for comparison. These pairs contain no imported source Hot Cues; nonzero cue semantics use synthetic fixtures with the released test's point/loop coordinates. M2.2's compact synthetic wire headers are not treated as the actual ANLZ specification.

No third-party parser implementation or public corpus binaries are copied into the product tree. Raw corpus/C reference sources remain in external research storage; immutable URLs, file hashes, comparison harness/results and local logs are retained in [gate evidence](evidence/m1-rekordbox-analysis-2026-10-21/). Existing released deck semantic tests remain green. This is not complete export-version, physical media, UI screenshot or device acceptance.

Supported scope is the classic monochrome/8-Hot-Cue path. PCO2 extended cues/comments/colors, memory-cue UI, PWV4/PWV5/PWV6/PWV7 color/3-band waveforms, PQT2, phrase analysis and VBR seek import are not implemented by this gate. Public files containing those tags pass because unknown tags are skipped, not because those capabilities are supported. The UI, large-grid controller timing, real media removal/seek, physical output and combined-load tests remain open. APTA stays unmerged and all pre-existing custom-PCB edits remain unchanged.
