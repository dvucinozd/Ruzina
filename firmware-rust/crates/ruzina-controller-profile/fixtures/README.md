# Released controller-profile fixtures

Profile JSON sources are copied verbatim from `dvucinozd/Pajoniiir` commit
`e9c41f9202faccdd46e46a70801a600c98cf87a8`:

- `pioneer_ddj_flx4.json` — released FLX4 profile source.
- `hercules_djcontrol_inpulse_500.json` — released Hercules Inpulse 500 profile source.

`compile_profile.py` is copied verbatim from the donor (Pajoniiir shared P4 core)
`0cde41815c7a4cb8f7b534f990073a629628d951`
(`tools/controller_profile/compile_profile.py`, SHA-256
`d0b3e28a15496b888b2bfd7d042c0e6830d23f077d1275fd400cd56c06a1ac20`). It
accepts the S3CP v2/v3/v4 JSON extensions and emits the oldest version that
represents a profile. Over the M2.5 reference (`20f1c3f0`, SHA-256
`9c88652a…`), which it replaces, it adds `cc7_to14_inverted` inputs, the
`deckN.pfl_set` event, `note_velocity` outputs and the `native_headphone_controls`
capability. It still reproduces every golden binary below byte for byte.

Frozen binaries (RP-02) — produced by tooling, never by the Rust parser:

| File | Producer | SHA-256 |
|---|---|---|
| `pioneer_ddj_flx4.v2-golden.s3bin` | pre-M2.5 fixture compiler (e9c41f92) | `b9f8a786eb4659280801a5e7435f8ab945d3700640b0fdbb2ddfbaff06de82db` |
| `hercules_djcontrol_inpulse_500.v2-golden.s3bin` | pre-M2.5 fixture compiler (e9c41f92) | `4eb737e7dd6c96432878e273b1d0466c19ff07b295fc372940c42e3c0db5aa70` |
| `s3cp_v3_scaled_cc.golden.s3bin` | M2.5 compiler from `s3cp_v3_scaled_cc.json` | `07db154a7a7bf77551a52a3d68929df268fd7d2ea0dd9e95defb13fe86634438` |
| `s3cp_v4_extended.golden.s3bin` | M2.5 compiler from `s3cp_v4_extended.json` | `b3c88648e992cbe4a550af4d6a72f61ea86232451f7574130858636157304dae` |

The M2.5 compiler reproduces both v2 golden binaries byte for byte. The v3/v4
JSON sources are synthetic Pajoniiir test fixtures (VID `0x1209`), not real
controller mappings; expected runtime values in the tests come from the M2.5
schema (`0/64/127 -> 0/8192/16383`, `min(127, value * scale / 127)`).

The Rust integration test compiles every JSON source during the host test,
compares the output with the frozen bytes and parses it with
`ruzina-controller-profile`. Passing this test is host evidence only. It
does not upgrade Hercules or any other non-FLX4 controller to physically
qualified hardware support.

## The donor's other controllers

The deck carries these besides the FLX4's, chosen by the controller's USB VID:PID. Their
JSON sources are copied verbatim from the donor at `0cde4181`
(`controllers/<name>/profile.json`); each `.s3bin` is the compiler's output, the same
bytes as the donor's own `profile.s3bin`. The donor's README for each controller
records where its mapping comes from and what was tested.

| JSON | VID:PID | Source (from the donor's README) | Donor's hardware status | SHA-256 of `.s3bin` |
|---|---|---|---|---|
| `pioneer_ddj_400.json` | `2B73:0026` | kayrozen's Pajoniiir fork `428b97dd` | not run | `2fc922ec2986249f4af7c7a077367e4bb5b9e9f22ced5b3d137e853d21f74408` |
| `numark_dj2go2_touch.json` | `15E4:003C` | Mixxx mapping (addresses and roles only), `0fffd3d0` | controls and audio run by an operator; LEDs partial | `6e899de3f34a910fbc7df38d2d81cfd54b4cf47e49650f05d96d937fd3960578` |
| `ni_traktor_s2_mk3_full.json` | `17CC:1710` | Algoriddim djay Pro 2.2.9 factory mapping (not redistributed) | experimental candidate | `ab9259c51c2b18cf5b67e2e279e3a0fbf9ee167a489f345b813a9ef37f40725f` |
| `hercules_djcontrol_inpulse_500.v4.json` | `06F8:B12B` | the donor's `docs/HERCULES_INPULSE_500_MIDI_MAP.md` | 60-minute soak on the donor's M3 bench; control and LED coverage partial | `570b59ecca6baa7319ef85ca92d7d3a11fbe11ea1f9c9c580a8779031a40f549` |

None is tested on this project's hardware. The Hercules (asynchronous UAC1 with a
feedback endpoint) and the S2 MK3 (UAC2) have no audio route here yet; their MIDI works
without it.
