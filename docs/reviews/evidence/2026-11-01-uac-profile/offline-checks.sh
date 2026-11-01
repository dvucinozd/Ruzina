#!/bin/bash
set -eu
cd /mnt/d/AI/M1_PCB
manifest=firmware-idf/legacy-jc1060/rust-core/Cargo.toml
cargo +1.95.0 fmt --manifest-path "$manifest" -- --check
for feature in idf-media-load idf-controller-audio idf-deck-play idf-deck-mix idf-deck-engine idf-uac-profile; do
 echo "HOST_FEATURE=$feature"
 cargo +1.95.0 test --locked --manifest-path "$manifest" --features "$feature"
 cargo +1.95.0 clippy --locked --manifest-path "$manifest" --features "$feature" --tests -- -D warnings
done
for feature in idf-deck-engine idf-uac-profile; do
 echo "TARGET_FEATURE=$feature"
 cargo +1.95.0 clippy --locked --release --manifest-path "$manifest" --features "$feature" --target riscv32imafc-unknown-none-elf -- -D warnings
 cargo +1.95.0 build --locked --release --manifest-path "$manifest" --features "$feature" --target riscv32imafc-unknown-none-elf
done
python3 -m unittest discover -s firmware-idf/legacy-jc1060/tools/tests -p test_uac_recovery.py -v
