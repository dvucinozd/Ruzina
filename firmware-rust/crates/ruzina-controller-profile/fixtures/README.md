# Released controller-profile fixtures

Profile JSON sources are copied verbatim from `dvucinozd/Pajoniiir` commit
`e9c41f9202faccdd46e46a70801a600c98cf87a8`:

- `pioneer_ddj_flx4.json` — released FLX4 profile source.
- `hercules_djcontrol_inpulse_500.json` — released Hercules Inpulse 500 profile source.

`compile_profile.py` is copied verbatim from the stable M2.5 reference
`20f1c3f04a615209ae25e9bbdae649d0f5b44e8d`
(`tools/controller_profile/compile_profile.py`, SHA-256
`9c88652aa87462cad3f8c5041e844045385ad29fafd44746e166b26067dc0d6e`). It
accepts the S3CP v2/v3/v4 JSON extensions and emits the oldest version that
represents a profile.

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
