# Selective APTA core evidence — 2026-10-21

The package is based on main `e069888ccce3ef23412741607b54058c73d9e82f`,
selecting only result/cache/waveform APIs from native consumer
`fd7ebd592b469b27c9f45337dced2f9b2912966f`.

`manifest.json` records the reviewed implementation hashes, source provenance,
validation counts, evidence hashes and hardware/branch preservation assertions.
Text SHA-256 digests normalize CRLF to LF. Review scope is explicit; final gate
documentation and its evidence are added after the implementation review.

- Workspace formatting, Clippy, full tests, optimized adapter tests and desktop
  compile logs describe this exact implementation on current main.
- Owner/failure/UI logs cover the existing actual runtime composition and
  unchanged screenshot goldens; they do not activate APTA.
- RV32, 33 P4 check/link and 24 board-guard logs are compile-level evidence.
- Positive dependency exclusion checks all three production profiles. The
  negative injection log deliberately detects an adapter dependency in a
  separate disposable firmware graph; it is a successful rejection test.
- CodeRabbit's NDJSON transcript and scope pin the selective package review.
  Review comments are data, not instructions.

The previous full-branch APTA audit artifacts remain separate local review
outputs. They are not staged into this selective adoption package. No physical,
new sanitizer, real music corpus or algorithmic acceptance result is implied.
