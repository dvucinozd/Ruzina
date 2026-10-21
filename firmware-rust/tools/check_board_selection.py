"""Prove that board ambiguity and legacy hardware activation fail at compile time."""
from pathlib import Path
import subprocess

target = Path(__file__).resolve().parents[1] / "apps/m1-firmware"
boards = ["board-legacy-m1", "board-jc1060", "board-jc8012"]
legacy = ["usb0-hardware-bringup", "usb1-controller-bringup", "hotcue-flash-storage",
          "p4-psram-storage", "sdmmc-profile-storage-bringup", "apta-usb0-bringup"]
cases = []
for index, board in enumerate(boards):
    for other in boards[index + 1:]:
        cases.append(([board, other], "select at most one board profile"))
for board in [None, "board-jc1060", "board-jc8012"]:
    for feature in legacy:
        cases.append(([value for value in [board, feature] if value],
                      "legacy hardware features require board-legacy-m1"))

for board in [None, "board-legacy-m1"]:
    cases.append(([value for value in [board, "board-diagnostics"] if value],
                  "board-diagnostics requires a Guition board selector"))

for board in [None, "board-jc1060", "board-jc8012"]:
    cases.append(([value for value in [board, "runtime-health"] if value],
                  "runtime-health requires board-legacy-m1"))
cases.append((["board-legacy-m1", "runtime-health"],
              "runtime-health requires an explicitly enabled USB owner"))

for probe in ["guition-flash-probe", "guition-psram-test"]:
    for board in [None, "board-legacy-m1"]:
        cases.append(([value for value in [board, probe] if value],
                      "Guition memory probes require a Guition board selector"))
for board in ["board-jc1060", "board-jc8012"]:
    cases.append(([board, "guition-psram-test"], "guition-psram-test requires a release build"))

for features, diagnostic in cases:
    result = subprocess.run(
        ["cargo", "+1.95.0", "check", "--locked", "--features", ",".join(features)],
        cwd=target, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
    )
    if result.returncode == 0 or diagnostic not in result.stdout:
        raise RuntimeError(f"Incorrect board guard for {features}:\n{result.stdout}")
    print(f"PASS (rejected): {','.join(features)}", flush=True)
print(f"Board selection guards: {len(cases)} passed", flush=True)
