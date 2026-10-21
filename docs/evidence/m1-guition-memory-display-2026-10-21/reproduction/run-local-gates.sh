#!/bin/bash
set -euo pipefail
export CARGO_HOME=/home/daniel/.cache/m1-rust-review/cargo
export RUSTUP_HOME=/home/daniel/.cache/m1-rust-review/rustup
export PATH="$CARGO_HOME/bin:/usr/local/bin:/usr/bin:/bin"
export CARGO_BUILD_JOBS=2
root=/mnt/c/Users/Daniel/.codex/worktrees/m1-guition-bsp
cd "$root/M1_PCB/firmware-rust"
case "$1" in
host)
 export CARGO_TARGET_DIR=/home/daniel/.cache/m1-rust-review/host-target
 export PKG_CONFIG_SYSROOT_DIR=/home/daniel/.cache/m1-rust-review/native/sysroot
 export PKG_CONFIG_PATH="$PKG_CONFIG_SYSROOT_DIR/usr/lib/x86_64-linux-gnu/pkgconfig"
 export LD_LIBRARY_PATH="$PKG_CONFIG_SYSROOT_DIR/usr/lib/x86_64-linux-gnu"
 cargo fmt --all --check > "$root/research/guition-fmt.log" 2>&1
 cargo clippy --locked --workspace --all-targets -- -D warnings > "$root/research/guition-host-clippy.log" 2>&1
 cargo test --locked --workspace > "$root/research/guition-workspace.log" 2>&1
 python3 -m unittest discover -s tools/tests -v > "$root/research/guition-python.log" 2>&1
 python3 tools/check_apta_core_boundary.py > "$root/research/guition-exclusion.log" 2>&1
 echo 'Host workspace, Clippy, Python and exclusion PASS'
 ;;
p4)
 export CARGO_TARGET_DIR=/home/daniel/.cache/m1-rust-review/p4-target
 export M1_IMAGE_ID=guition-memory-display-preparation
 cargo check --locked -p pajoniiir-board-profile --target riscv32imafc-unknown-none-elf > "$root/research/guition-profile-rv32.log" 2>&1
 cd apps/m1-firmware
 rustfmt +1.95.0 --edition 2024 --config skip_children=true --check src/platform/mod.rs src/platform/diagnostics.rs src/platform/memory.rs
 python3 "$root/research/guition-prep-matrix.py" > "$root/research/guition-matrix.log" 2>&1
 python3 ../../tools/check_board_selection.py > "$root/research/guition-board-guards.log" 2>&1
 for probe in none flash psram memory; do
   python3 ../../tools/guition_first_boot.py build --probe "$probe" --output "$root/research/guition-memory-prep-20261021/$probe" --target-dir "$CARGO_TARGET_DIR" --git '/mnt/c/Program Files/Git/cmd/git.exe' > "$root/research/guition-package-$probe.log" 2>&1
 done
 echo 'P4 matrix, guards, four variants for each board PASS'
 ;;
esac
