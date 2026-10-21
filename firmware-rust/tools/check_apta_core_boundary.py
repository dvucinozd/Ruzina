"""Verify that the experimental adapter has no product dependency path.

Run from any directory using the workspace's pinned Rust/Cargo toolchain.
Cargo tree resolves normal/build dependencies for the actual product target;
test fixtures are deliberately outside this production boundary.
"""

from pathlib import Path
import subprocess


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    forbidden = {"libapta", "pajoniiir-apta-adapter"}
    profiles = [
        (
            "desktop simulator",
            ["--manifest-path", str(root / "Cargo.toml"), "-p", "desktop-simulator"],
        ),
        (
            "firmware default",
            [
                "--manifest-path",
                str(root / "apps/m1-firmware/Cargo.toml"),
                "--target",
                "riscv32imafc-unknown-none-elf",
            ],
        ),
        (
            "firmware full audio owners",
            [
                "--manifest-path",
                str(root / "apps/m1-firmware/Cargo.toml"),
                "--target",
                "riscv32imafc-unknown-none-elf",
                "--features",
                "board-legacy-m1,render-graph-bringup,sdmmc-profile-storage-bringup",
            ],
        ),
    ]
    for name, arguments in profiles:
        command = [
            "cargo",
            "tree",
            "--locked",
            "--edges",
            "normal,build",
            "--prefix",
            "none",
            "--format",
            "{p}",
            *arguments,
        ]
        result = subprocess.run(command, cwd=root, text=True, capture_output=True)
        if result.returncode:
            raise SystemExit(f"{name}: cargo tree failed:\n{result.stderr}")
        packages = {line.split()[0] for line in result.stdout.splitlines() if line.strip()}
        present = forbidden & packages
        if present:
            raise SystemExit(f"{name}: experimental provider dependency: {sorted(present)}")
        print(f"{name}: APTA dependency exclusion PASS")


if __name__ == "__main__":
    main()
