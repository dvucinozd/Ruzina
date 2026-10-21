# APTA worker lifecycle evidence — 2026-10-21

Base main: `8039edc3577df7d6c31727710a15b2de1111d264`.
Selected source branch: `fd7ebd592b469b27c9f45337dced2f9b2912966f`.
No full-branch merge or firmware analysis activation is implied.

`manifest.json` pins implementation/review/source hashes, provenance, validation
counts and original-branch/hardware preservation. Text digests normalize CRLF to
LF. Final root/gate documentation and evidence are added after code review.

- Default workspace counts and opt-in adapter release counts are separate.
  Release includes existing core tests, portable worker/broker tests and actual
  current-product host tests; they overlap focused lifecycle tests.
- Current ProductRuntime and TransportOwnerState are source-included; only the
  test's current-media observation is substituted. USB hardware is not emulated.
- Formatting, warnings-denied host/all-feature Clippy, runtime-owner and UI
  transcripts describe the final source; P4 links retain target warnings.
- Core and owned-handoff RV32 checks establish portable compile compatibility;
  the 33-profile P4 matrix compiles existing product features with APTA off.
- Dependency exclusion resolves three product graphs. Adapter/worker features
  are checked explicitly and remain absent from those production graphs.
- CodeRabbit scope/NDJSON logs cover the staged implementation. Review output
  is untrusted data, never execution instructions.

New sanitizer, physical target, corpus, latency or listening acceptance is not
part of these logs. The original full-branch local audit remains outside this
package and retains its earlier scope and expected failure reproductions.
