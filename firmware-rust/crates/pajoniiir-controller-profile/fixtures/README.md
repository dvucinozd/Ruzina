# Released controller-profile fixtures

These host-test fixtures are copied verbatim from `dvucinozd/Pajoniiir` commit
`e9c41f9202faccdd46e46a70801a600c98cf87a8`.

Included sources:

- `compile_profile.py` — released S3CP v2 compiler.
- `pioneer_ddj_flx4.json` — released FLX4 profile source.
- `hercules_djcontrol_inpulse_500.json` — released Hercules Inpulse 500 profile source.

The Rust integration test compiles these JSON sources during the host test and
parses the resulting binary with `pajoniiir-controller-profile`. This checks
binary-format compatibility against the released product tooling without
embedding a second hand-maintained binary representation.

Passing this test is host evidence only. It does not upgrade Hercules or any
other non-FLX4 controller to physically qualified hardware support.
