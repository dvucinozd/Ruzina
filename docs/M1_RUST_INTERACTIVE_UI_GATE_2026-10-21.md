# M1 interactive deck UI and compositor

Date: 2026-10-21

Base: `963e730f614f948830f77ee6c93801e34f6105c7` on the open analysis PR #6, itself
based on audio PR #5. This package is isolated on `ogranak/interactive-deck-ui`.
It adds an interactive host UI; it does not merge the dependent branches into
`main`, merge APTA, or enable a Guition peripheral.

## Implemented behavior

The previous simulator painted one fixed snapshot and waveform placeholders.
The compiled Slint view now has Decks, demo Library, eight-slot Hot Cues and
session Settings. `pajoniiir-ui-model` maps typed deck actions to existing
controller-independent `ControlEvent`s and projects authoritative `DeckState`
plus effective source/local cues into `DeckPresentation`. Adapter indices/codes
are validated; unloaded decks reject mutations. Buttons reflect confirmed
product state rather than UI-only transport toggles.

The host session uses the real deck consumer and the bounded Rekordbox importer
with existing synthetic DAT/EXT fixtures. Analysis is pinned to media lease,
track/catalog identity and load generation. Successful load commits source cues
with retained session-local overrides. Playing destinations reject replacement.
Same-track reload, missing/rejected analysis and detach cannot leave the old
waveform active. Media removal unloads/stops both decks; reconnect requires an
explicit new load. Source cues are merged with local overrides/tombstones, so
the waveform marks the same effective cue positions as the buttons.

Play/Pause, Cue, Sync, Beat Jump, quantized Loop In/Out/Exit and tempo-range
controls enter the shared semantic layer. The host transport acknowledges
playback requests and uses an elapsed-time clock with fractional pitch carry,
loop wrapping and EOF clamping. This is a deterministic simulated transport,
not the embedded PCM/completed-output clock or audio output. Cue edits are held
in process memory per demo track; this does not qualify persistent journal I/O.
Settings expose elapsed/remaining time and real deck quantize/range state;
there is no new on-disk settings schema or fabricated device audio telemetry.

The host compositor passes neutral preview/detail columns to
`pajoniiir-waveform::Rgb565Surface` and draws correctly aligned beat ticks,
playhead, effective cue markers and loop endpoints. Detail uses explicit bounded
time windows (700 columns); beat iteration uses a sorted-grid partition search.
Each 700×90 RGB565 surface is 126,000 bytes and its RGB8 Slint image is 189,000
bytes, with additional transient/previous image storage during refresh. Parsing
does not run in the timer/render path. Host heap allocation, 20 Hz UI refresh and
this image conversion do not establish P4 PSRAM, PPA/DMA or display performance.
No new Slint dependency or compositor is added to the embedded owners here.

The selected board profile supplies documented startup size: JC1060 1024×600,
JC8012 800×1280. Responsive geometry changes two columns to two rows; it is not a
physical panel/touch rotation mapping. Buttons support pointer input, focus and
Space/Return; stable deck/cue models preserve a held click through timer refresh.
All Slint mutation stays on its UI thread. Linux x86_64/DejaVu Sans/scale-1
software-renderer screenshots are reviewed pixel baselines, not photographs of
either board.

## Validation

| Local gate | Result |
|---|---|
| Full workspace tests | PASS, 559 tests |
| Host format, all-target Clippy with warnings denied, desktop compile | PASS |
| Headless Slint callbacks + pointer/keyboard replay | PASS at both board dimensions |
| Reviewed screenshots, independently replayed | PASS, 14 exact RGB pixel goldens |
| P4 workflow / partition validation | PASS, 29 check/link configurations |
| Feature rejection guards | PASS, 20 expected rejections |

The desktop tests cover confirmed transport/load lock/EOF recovery, imported-grid
Beat Jump and quantized looping, cue tombstones after reload, no-analysis/bad-data
waveform invalidation, detach/reconnect and timed cue/playhead alignment. Shared
model tests cover effective pitch/cues, unloaded presentation and button release
semantics. Headless replay exercises actual Slint hit testing and keyboard focus,
including timer/model refresh between pointer press and release for both Play
and cue deletion. This supplements callback tests; it is not physical touch or
operator acceptance. The replay starts a fresh session for each geometry.

[Retained logs, manifests and PNGs](evidence/m1-interactive-ui-2026-10-21/README.md)
include empty, loaded/Sync, cue edit, settings, analysis failure and removed-media
states for both dimensions. The host workflow now renders, checks pixel hashes
and uploads PPM snapshots. Hosted CI still requires the account billing/spending
block to be resolved; local PASS is not green hosted CI. External lock-package
records are unchanged; only local workspace dependencies were added. Existing
18 modified primary-checkout KiCad files remain byte-identical.

## Acceptance boundaries and next work

This is the interactive deck/Library/cue/session-settings part of S6. It supplies
concrete host evidence toward M1R-P0-060/061/062/066; none of those rows is promoted
to complete product/hardware acceptance here. Full M1R-P0-065 parity still needs
mixer/FX, diagnostics/performance views and the remaining M2.2 workflows. Library
uses four explicit synthetic demos, not an arbitrary USB/catalog adapter. Durable
general settings, physical cue journal, real audio/media timing, controller
feedback, Guition display/touch/rotation, combined resource load and operator
acceptance remain open. Newer Rekordbox formats remain outside the classic path.

Next bounded host package: expose mixer/PFL and FX state/actions through the same
authoritative UI boundary and add representative replay/goldens. Board arrival
still starts with exact-unit diagnostics/recovery and memory/peripheral bring-up.
Headphones remain controller-only. APTA remains unmerged.
