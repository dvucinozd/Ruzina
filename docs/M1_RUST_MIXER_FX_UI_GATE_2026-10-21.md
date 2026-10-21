# M1 mixer/PFL and FX UI

Date: 2026-10-21

Base: `df1524b9a98ba52a740577f718017626619732da`, UI PR #7 above Rekordbox PR #6
and audio PR #5. Work is isolated on `ogranak/mixer-pfl-fx-ui`. No dependency
branch is merged into main and APTA remains unmerged.

## Behavior and ownership

The host simulator now exposes Mixer and FX pages in addition to Decks, Library,
Hot Cues and session Settings. The shared no_std UI model supplies typed/range-
checked `MixerParameter`, `MixerToggle`, `BeatFxAction` and pad events plus small
authoritative `MixerPresentation`/`FxPresentation` projections. Wrong channel vs
global scopes, negative/oversize values, invalid bank/slot and unknown action
codes are rejected before dispatch. No MIDI numbers, DSP implementation or
physical pin assignments are placed in the UI.

Mixer controls operate on `MixerState`: each channel has Trim, three EQ bands,
Filter, channel level and PFL; global controls expose crossfader, master level,
headphone CUE/MAIN mix, headphone level, Master Cue, Smart CFX and Smart Fader.
PFL/headphone configuration is independent of MAIN/crossfader settings. The
controller-only headphone policy is visible, without fabricated DAC/UAC device
health or signal-level meters. Readouts show normalized control percentages;
centered Trim/EQ/Filter and crossfader use zero at neutral, not invented dB values.
Mixer configuration remains adjustable with no loaded track and is retained on
demo media removal, like a mixer configuration rather than per-track state.

Beat FX uses the real global deck FX state: effect previous/next, beat decrease/
increase, channel 1/2/both target, depth, enabled and Clear. Pad FX exposes both
banks of eight momentary pads per loaded deck. Pointer down and Space/Return
activate; release, pointer cancel/exit, focus loss and page departure deactivate.
The held button captures its bank token, so a later bank selection cannot change
the identity of its release. Changing a bank first releases that deck's active
pad; the other deck remains independent. Stable models preserve held input across
refresh. Media removal releases pads, unloads both decks and resets Beat FX via
the semantic Clear action. A late release cannot replace the media recovery
message or recreate old state. Reconnect still requires explicit track loading.

Native Slint sliders support drag, Home/End and arrow keys. After a manual edit,
later authoritative state is explicitly reapplied to the same slider item;
otherwise the widget could retain an old local value. The replay verifies a
second-source value publication followed by an arrow-key edit, as well as a drag
spanning a timer/model refresh. Build configuration pins Fluent widget styling;
Slint calls remain confined to the UI thread.

This is host control/state evidence. `ApplyBeatFx`/`ApplyPadFx` are still not
connected to an audible DSP owner here. No PCM, limiter/VU telemetry, real USB
controller or I2S/UAC output is generated. Main/headphone routing, PFL placement
and FX sound/lifecycle qualification remain separate runtime/physical gates.
Guition resource acquisition, display/touch rotation and production defaults
are unchanged; a linked no_std UI model is not a running board UI.

## Validation and retained evidence

| Local gate | Result |
|---|---|
| Full workspace tests | PASS, 564 tests (+3 shared model, +2 host session) |
| Format / all-target warnings-denied Clippy / desktop compile | PASS |
| Compiled Slint pointer, keyboard and callback replay | PASS at 1024×600 and 800×1280 |
| Reviewed pixel goldens, independent replay | PASS, 24 snapshots |
| P4 workflow, partition validation | PASS, 29 check/link configurations |
| Invalid-feature guards | PASS, 20 expected rejections |

The existing 14 scenes have identical pixels below navigation compared with the
prior reviewed PNGs. Their navigation changed to six equal destinations. Ten
new snapshots cover default/edited Mixer, default/active FX and disabled pads
after media removal at both sizes. Mixer places channels beside Master/Phones
in landscape and above it in portrait; Beat FX and Pad FX similarly reflow.
Linux x86_64, Slint 1.18.1 software renderer, Fluent, DejaVu Sans and scale 1 define
the pixel environment. A separate process replays and compares all 24 raw RGB
SHA-256 hashes. No automatic baseline replacement is part of the checker.

The existing host workflow already executes this expanded replay/checker and
retains snapshots. GitHub jobs remain subject to the account billing/spending
block; local PASS does not establish hosted CI PASS. Logs, reviewed PNGs, source
and font identities are in [retained evidence](evidence/m1-mixer-fx-ui-2026-10-21/README.md).
External package records in both locks remain unchanged; only a local mixer
dependency is added. All 18 pre-existing primary-checkout KiCad file hashes
remain unchanged. No firmware was flashed or published.

## Remaining product work

This closes the mixer/PFL/FX host-control slice of S6; it does not close full
M1R-P0-065 parity or M2.2 audio acceptance. Diagnostics/performance views need
real service/health publications. Arbitrary media/catalog and product-runtime
adapters, durable general settings, device controller feedback, actual FX DSP
execution and physical display/touch/audio/resource qualification remain open.
The next bounded host package is the service/health model and diagnostics view,
with faults and absent hardware shown as actual states rather than fabricated
measurements. Board arrival still starts with exact-unit recovery/diagnostics.
