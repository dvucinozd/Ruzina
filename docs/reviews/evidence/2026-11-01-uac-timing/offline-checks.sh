#!/bin/bash
set -eu
cd /mnt/d/AI/M1_PCB
manifest=firmware-idf/legacy-jc1060/rust-core/Cargo.toml
cargo +1.95.0 fmt --manifest-path "$manifest" -- --check
for feature in idf-media-load idf-controller-audio idf-deck-play idf-deck-mix idf-deck-engine; do
  echo "HOST_FEATURE=$feature"
  cargo +1.95.0 test --locked --manifest-path "$manifest" --features "$feature"
  cargo +1.95.0 clippy --locked --manifest-path "$manifest" --features "$feature" --tests -- -D warnings
done
python3 -m unittest discover -s firmware-idf/legacy-jc1060/tools/tests -p test_uac_recovery.py -v
