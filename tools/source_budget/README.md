# Offline source-service investigation

These tools never access COM3. Frozen continuous campaign files and admission
results remain unchanged. See [the investigation](../../docs/reviews/2026-11-03-source-budget.md)
for measured scope, compiler controls and remaining target gates.

`audit_capture.py --capture <retained-capture-directory> --output <fresh-json>`
verifies raw/case digests and reports additional MIDI controls and unsupported
counter transitions. It is not admission or a protocol-order validator. It
does not infer who generated controller messages. Run its tests with:

```sh
python3 -m unittest discover -s tools/source_budget/tests -v
```

The Rust `deck_play::tests::service_model` module uses the real producer,
decoder, ring and scheduling function with the CC0 eight-second 44.1-kHz VBR
fixture. One shared virtual timeline consumes both rings during reads and
between polls. Decode compute/preemption costs zero virtual time; owner
overhead is explicitly 100 us per turn. Read latency is a sensitivity input,
not target calibration. Prime/load occurs before playback. The test carries
fractional sample debt and stops on the first missing source frame. It does
not model UAC/resampling, different formats, seeks or hardware IRQs.

```sh
cargo +1.95.0 test --locked --manifest-path firmware-idf/legacy-jc1060/rust-core/Cargo.toml --features idf-producer-profile shared_read_timeline -- --nocapture
```

The ignored host benchmark decodes the complete fixture using memory and
requested 200-us sleeping reads, five repetitions each. Host scheduler sleep
may exceed 200 us; actual read wall time is reported. Exact PCM equality is
checked between repetitions/readers. Optional `RUZINA_BENCH_PCM_OUT` writes one
fresh PCM16 stereo file after measurement, enabling cross-build byte comparison.
Frame counts, FNV diagnostic hash, read counts and measured wall partitions
are printed. Elapsed time includes consumer/checksum work; stage times do not.
No host duration threshold qualifies an ESP32-P4 image.

```sh
cargo +1.95.0 test --locked --release --manifest-path firmware-idf/legacy-jc1060/rust-core/Cargo.toml --features idf-producer-profile host_source_service_benchmark -- --ignored --nocapture
cargo +1.95.0 --config profile.release.package.nanomp3-core.opt-level=3 test --locked --release --manifest-path firmware-idf/legacy-jc1060/rust-core/Cargo.toml --features idf-producer-profile host_source_service_benchmark -- --ignored --nocapture
```

The opt3 target build is **rejected**: its MP3 constructor has a 39008-byte
frame, above the unchanged 24576-byte guard. Do not flash or register it.
The release profile stays `s`. The separate builder option `--mp3-core-opt3`
requires `--variant deck-engine --producer-profile`, inserts only the
`nanomp3-core` package override for Cargo build/Clippy, and records it in the
manifest. It does not change buffers, scheduling, gain, thresholds or normal
defaults. Other compiler experiments in the host matrix are not this candidate.
No current writer binding registers or authorizes the new image.
