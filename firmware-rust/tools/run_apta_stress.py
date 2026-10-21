"""Repeat actual service/product cancellation and detach regressions on the host.

Records exact binary/source hashes and finite iteration logs. No hardware or
musical-accuracy/performance acceptance is inferred from this lifecycle run.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import time

from guition_first_boot import ROOT, digest, require, save, sources


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iterations", type=int, default=25)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--git", default="git")
    args = parser.parse_args()
    require(1 <= args.iterations <= 1000, "Iterations must be 1..1000")
    require(not args.output.exists(), "Output must be a new directory")
    args.output.mkdir(parents=True)
    records, fingerprint = sources(args)
    command = ["cargo", "test", "--locked", "-p", "pajoniiir-apta-adapter", "--all-features",
               "--release", "--test", "usb0", "--no-run", "--message-format=json"]
    with (args.output / "build.log").open("wb") as log:
        built = subprocess.run(command, cwd=ROOT / "firmware-rust", stdout=subprocess.PIPE, stderr=log, check=True)
    (args.output / "build.jsonl").write_bytes(built.stdout)
    artifacts = [json.loads(line) for line in built.stdout.splitlines() if line.startswith(b"{")]
    binaries = [Path(row["executable"]) for row in artifacts if row.get("reason") == "compiler-artifact"
                and row.get("target", {}).get("name") == "usb0" and row.get("executable")]
    require(len(binaries) == 1, "Expected one actual USB0 regression executable")
    binary = binaries[0]
    summary = {"schema": 1, "scope": "HOST_LIFECYCLE_ONLY", "hardware": "NOT_RUN", "status": "RUNNING",
               "source_digest": fingerprint, "source_files_sha256": records, "build_command": command,
               "binary_sha256": digest(binary), "iterations_requested": args.iterations,
               "iterations_completed": 0, "cancel_reload_cycles": 0, "detach_client_lifetimes": 0}
    start = time.monotonic()
    try:
        for index in range(args.iterations):
            env=os.environ.copy()
            env["M1_APTA_STRESS_RECORD"]=str((args.output/f"native-observation-{index + 1:03}.txt").resolve())
            with (args.output / f"iteration-{index + 1:03}.log").open("wb") as log:
                result = subprocess.run([str(binary), "repeated_", "--nocapture", "--test-threads=1"],
                                        stdout=log, stderr=subprocess.STDOUT, timeout=120,env=env)
            text = (args.output / f"iteration-{index + 1:03}.log").read_text()
            require(result.returncode == 0 and "test result: ok. 2 passed;" in text, "Lifecycle regression failed or was not run")
            cancel = re.search(r"stress cancel/reload: (\d+) cycles", text)
            detach = re.search(r"stress detach: (\d+) independent", text)
            require(cancel is not None and detach is not None, "Missing actual cycle evidence")
            summary["cancel_reload_cycles"] += int(cancel[1])
            summary["detach_client_lifetimes"] += int(detach[1])
            summary["iterations_completed"] += 1
        require(sources(args) == (records, fingerprint), "Source inputs changed during stress run")
        require(digest(binary) == summary["binary_sha256"], "Test executable changed during stress run")
        summary["status"] = "PASS"
    finally:
        if summary["status"] != "PASS":
            summary["status"] = "FAIL"
        summary["host_elapsed_seconds"] = round(time.monotonic() - start, 3)
        summary["logs_sha256"] = {path.name: digest(path) for path in sorted(args.output.iterdir()) if path.is_file()}
        save(args.output / "summary.json", summary)
    print(json.dumps({key: summary[key] for key in ("status", "iterations_completed", "cancel_reload_cycles", "detach_client_lifetimes", "hardware")}, indent=2))


if __name__ == "__main__":
    main()
