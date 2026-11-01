#!/bin/bash
set -eu
cd /mnt/d/AI/M1_PCB
manifest=firmware-idf/legacy-jc1060/rust-core/Cargo.toml
for feature in idf-deck-play idf-deck-mix idf-deck-engine; do
  echo "FEATURE=$feature"
  cargo +1.95.0 test --locked --manifest-path "$manifest" --features "$feature"
  cargo +1.95.0 clippy --locked --manifest-path "$manifest" --features "$feature" --tests -- -D warnings
  cargo +1.95.0 clippy --locked --release --manifest-path "$manifest" --features "$feature" --target riscv32imafc-unknown-none-elf -- -D warnings
done
