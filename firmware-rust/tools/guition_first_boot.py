"""Build isolated Guition diagnostic packages and retain read-only observations.

No erase, write-flash, partition selection, port discovery or acceptance command.
Use --help; capture requires optional pyserial, build/ingest use stdlib only.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import zipfile

ROOT = Path(__file__).resolve().parents[2]
MODELS = {"jc1060": "JC1060P470C_I_W_Y", "jc8012": "JC8012P4A1C_I_W_Y"}
TARGET = "riscv32imafc-unknown-none-elf"
MAX_LOG = 1024 * 1024
PROBES = {"none": (), "flash": ("guition-flash-probe",), "psram": ("guition-psram-test",),
          "memory": ("guition-flash-probe", "guition-psram-test")}
SCENARIOS = ("factory", "cold-boot", "warm-reset", "service-reconnect", "rom-recovery")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    result = hashlib.sha256()
    with Path(path).open("rb") as file:
        for chunk in iter(lambda: file.read(65536), b""):
            result.update(chunk)
    return result.hexdigest()


def save(path, value):
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


def git(args, *command):
    return subprocess.check_output([args.git, *command], cwd=ROOT)


def sources(args):
    names = git(args, "ls-files", "--cached", "--others", "--exclude-standard", "-z", "--", "firmware-rust", ":(exclude,glob)firmware-rust/**/target/**").decode().split("\0")
    records = {name: digest(ROOT / name) for name in sorted(set(filter(None, names)))}
    fingerprint = hashlib.sha256(json.dumps(records, sort_keys=True).encode()).hexdigest()
    return records, fingerprint


def build(args):
    probe = getattr(args, "probe", "none")
    require(probe in PROBES, "Unknown diagnostic probe")
    output = args.output.resolve()
    require(not output.exists(), "Output must be a new directory; existing evidence is never overwritten")
    output.mkdir(parents=True)
    records, fingerprint = sources(args)
    commit = git(args, "rev-parse", "HEAD").decode().strip()
    dirty = git(args, "status", "--porcelain", "--untracked-files=all", "--", ".", ":(exclude,glob)**/target/**").decode().splitlines()
    compiler = subprocess.check_output([args.rustc, "+1.95.0", "-vV"], text=True)
    build_environment={key:value for key,value in os.environ.items() if key.startswith(("ESP_HAL_CONFIG_","CARGO_PROFILE_")) or key in ("RUSTFLAGS","CARGO_ENCODED_RUSTFLAGS","CARGO_BUILD_TARGET","SOURCE_DATE_EPOCH")}
    build_inputs_digest=hashlib.sha256(json.dumps([fingerprint,compiler,build_environment],sort_keys=True).encode()).hexdigest()
    summary = {"schema": 1, "status": "BUILDING", "source_commit": commit,
               "source_digest": fingerprint, "source_files_sha256": records,
               "dirty_status": dirty, "rustc": compiler, "build_environment":build_environment, "packages": {}, "hardware": "NOT_RUN"}
    save(output / "build-manifest.json", summary)
    try:
        for board, model in MODELS.items():
            folder = output / board
            folder.mkdir()
            image_id = f"{commit[:12]}-src-{build_inputs_digest[:16]}-{board}-{probe}"
            features = ",".join((f"board-{board}", "board-diagnostics", *PROBES[probe]))
            command = [args.cargo, "+1.95.0", "build", "--locked", "--release", "--target", TARGET,
                       "--manifest-path", str(ROOT / "firmware-rust/apps/m1-firmware/Cargo.toml"),
                       "--target-dir", str(args.target_dir.resolve()), "--features", features]
            env = os.environ.copy()
            env["M1_IMAGE_ID"] = image_id
            with (folder / "build.log").open("wb") as log:
                subprocess.run(command, cwd=ROOT / "firmware-rust/apps/m1-firmware", env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
            require(sources(args) == (records, fingerprint), "Build inputs changed during compilation")
            require(git(args, "rev-parse", "HEAD").decode().strip() == commit, "HEAD changed during compilation")
            elf = folder / "firmware.elf"
            shutil.copyfile(args.target_dir.resolve() / TARGET / "release/pajoniiir-m1-firmware", elf)
            require(image_id.encode() in elf.read_bytes(), "ELF does not contain its expected image identity")
            manifest = {"schema": 1, "board": board, "model": model, "image_id": image_id,
                        "source_commit": commit, "source_digest": fingerprint,
                        "source_files_sha256": records, "dirty_status": dirty, "rustc": compiler, "build_environment":build_environment,
                        "build_command": command, "build_cwd":str(ROOT / "firmware-rust/apps/m1-firmware"), "features": features,
                        "elf": {"file": "firmware.elf", "sha256": digest(elf), "bytes": elf.stat().st_size},
                        "build_log_sha256": digest(folder / "build.log"),
                        "required_min_chip_revision": 302, "documented_flash_bytes": 16777216,
                        "diagnostic_probe": probe, "memory_diagnostic_schema": 1,
                        "documented_psram_bytes": 33554432,
                        "readiness": "NOT_QUALIFIED", "factory_backup": "NOT_RUN",
                        "scenarios": {scenario: "NOT_RUN" for scenario in SCENARIOS}}
            save(folder / "manifest.json", manifest)
            (folder / "README.txt").write_text(
                f"{model}\nImage {image_id}\nELF SHA256 {manifest['elf']['sha256']}\n"
                f"Diagnostic probe: {probe}\nBUILD ONLY / NOT FLASHED / NOT QUALIFIED\n"
                "PSRAM variants initialize external RAM at 80 MHz and overwrite only a reserved 64 KiB CPU scratch span.\n"
                "They do not test cache writeback, DMA, full capacity or production clocks.\n"
                "Preserve factory log and verified backup. Establish ROM recovery, observed flash geometry,\n"
                "and a board-specific programming layout before selecting any flash command.\n"
                "The historical custom-M1 partition table is not a Guition programming instruction.\n"
                "Use guition_first_boot.py ingest/capture for separate unit/scenario records.\n", encoding="utf-8")
            archive = output / f"{board}-diagnostics.zip"
            with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as target:
                for file in sorted(folder.iterdir()):
                    target.write(file, file.relative_to(output))
            (output / (archive.name + ".sha256")).write_text(digest(archive) + "  " + archive.name + "\n")
            summary["packages"][board] = {"archive": archive.name, "sha256": digest(archive),
                                          "elf_sha256": manifest["elf"]["sha256"], "image_id": image_id}
        summary["status"] = "BUILD_ONLY"
    except Exception:
        summary["status"] = "BUILD_FAILED"
        raise
    finally:
        save(output / "build-manifest.json", summary)
    print(json.dumps(summary["packages"], indent=2))


def package(folder):
    manifest = json.loads((folder / "manifest.json").read_text(encoding="utf-8"))
    require(manifest.get("schema") == 1 and MODELS.get(manifest.get("board")) == manifest.get("model"), "Unknown package/board")
    require(manifest["elf"]["file"] == "firmware.elf", "Unexpected ELF path")
    require(digest(folder / "firmware.elf") == manifest["elf"]["sha256"], "ELF hash mismatch")
    require(digest(folder / "build.log") == manifest["build_log_sha256"], "Build log hash mismatch")
    require(manifest.get("required_min_chip_revision") == 302, "Unexpected silicon policy")
    require(manifest.get("readiness", "NOT_QUALIFIED") == "NOT_QUALIFIED", "Package cannot assert hardware qualification")
    require(manifest.get("diagnostic_probe", "none") in PROBES, "Unknown package probe")
    if "memory_diagnostic_schema" in manifest:
        require(manifest["memory_diagnostic_schema"] == 1, "Unknown memory diagnostic schema")
        expected = ",".join((f"board-{manifest['board']}", "board-diagnostics", *PROBES[manifest.get("diagnostic_probe", "none")]))
        require(manifest.get("features") == expected, "Package features/probe mismatch")
    return manifest


def parse_memory_log(raw, manifest):
    keys = {"board_profile", "image_id", "flash_probe", "flash_rdid_raw", "flash_bytes_rdid", "psram_probe",
            "psram_bytes_mapped", "psram_scratch_bytes", "psram_clock_mhz", "psram_failure_word", "psram_expected",
            "psram_actual", "test_scope", "readiness"}
    frames, errors, partial, frame = [], [], 0, None
    for line in raw.decode("utf-8", errors="replace").splitlines():
        if line.startswith("M1_"):
            if frame is not None:
                partial += 1
            frame = {} if line == "M1_MEMORY_DIAGNOSTIC_V1" else None
        elif frame is not None and "=" in line:
            key, value = line.split("=", 1)
            if key not in keys or key in frame:
                errors.append("unknown/duplicate memory field: " + key)
            frame[key] = value
            if key == "readiness":
                if set(frame) == keys:
                    frames.append(frame)
                else:
                    errors.append("incomplete memory diagnostic frame")
                frame = None
    if frame is not None:
        partial += 1
    probe = manifest.get("diagnostic_probe", "none")
    for frame in frames:
        try:
            require(frame["board_profile"] == manifest["model"] and frame["image_id"] == manifest["image_id"], "Memory identity mismatch")
            require(frame["test_scope"] == "CPU_CACHED_SCRATCH_ONLY" and frame["readiness"] == "NOT_QUALIFIED", "Memory scope/readiness mismatch")
            if probe in ("flash", "memory"):
                require(frame["flash_probe"] == "ID_DECODED", "Flash probe did not decode an ID")
                value = frame["flash_rdid_raw"]
                require(len(value) == 6 and all(c in "0123456789abcdef" for c in value), "Invalid RDID")
                rdid = int(value, 16)
                manufacturer, kind, density = rdid & 255, (rdid >> 8) & 255, rdid >> 16
                decoded = (1 << (15 + (kind & 31))) if manufacturer == 0x1f and 4 <= (kind & 31) <= 9 else \
                    (1 << density) if manufacturer not in (0, 0xff, 0x1f) and 0x12 <= density <= 0x1a else None
                require(decoded is not None and frame["flash_bytes_rdid"] == str(decoded), "RDID/capacity mismatch")
                require(decoded == manifest["documented_flash_bytes"], "Observed flash capacity mismatch")
            else:
                require(frame["flash_probe"] == "NOT_REQUESTED" and frame["flash_rdid_raw"] == "NOT_MEASURED"
                        and frame["flash_bytes_rdid"] == "NOT_MEASURED", "Unexpected flash probe")
            if probe in ("psram", "memory"):
                require(frame["psram_probe"] == "CPU_SCRATCH_PASSED", "PSRAM scratch did not pass")
                require(frame["psram_bytes_mapped"] == str(manifest["documented_psram_bytes"])
                        and frame["psram_scratch_bytes"] == "65536" and frame["psram_clock_mhz"] == "80", "PSRAM mapping/test mismatch")
            else:
                require(frame["psram_probe"] == "NOT_REQUESTED" and frame["psram_bytes_mapped"] == "NOT_MEASURED"
                        and frame["psram_scratch_bytes"] == "0" and frame["psram_clock_mhz"] == "0", "Unexpected PSRAM probe")
            require(all(frame[key] == "NONE" for key in ("psram_failure_word", "psram_expected", "psram_actual")), "PSRAM failure evidence present")
        except ValueError as error:
            errors.append(str(error))
    return {"complete_records": len(frames), "partial_records": partial, "errors": errors,
            "frames": frames, "observation_match": bool(frames) and not errors}


def parse_log(raw, manifest):
    require(len(raw) <= MAX_LOG, "Capture exceeds the 1 MiB bound")
    keys = {"board_profile", "image_id", "chip_revision_detected", "chip_minimum_required", "chip_policy_match",
            "flash_bytes_documented", "psram_bytes_documented", "flash_bytes_observed", "psram_bytes_observed", "readiness"}
    frames, errors, partial, reset_reasons = [], [], 0, []
    frame = None
    for line in raw.decode("utf-8", errors="replace").splitlines():
        if line.startswith("reset_reason=") and len(reset_reasons) < 16:
            reset_reasons.append(line[len("reset_reason="):][:128])
        if line == "M1_BOOT_DIAGNOSTIC_V1":
            if frame is not None:
                partial += 1
            frame = {}
        elif frame is not None and "=" in line:
            key, value = line.split("=", 1)
            if key not in keys or key in frame:
                errors.append("unknown/duplicate field: " + key)
            frame[key] = value
            if key == "readiness":
                if set(frame) == keys:
                    frames.append(frame)
                else:
                    errors.append("incomplete diagnostic frame")
                frame = None
    if frame is not None:
        partial += 1
    for frame in frames:
        try:
            require(frame["board_profile"] == manifest["model"], "Board/model mismatch")
            require(frame["image_id"] == manifest["image_id"], "Image identity mismatch")
            minimum = manifest["required_min_chip_revision"]
            require(frame["chip_minimum_required"] == str(minimum), "Chip policy mismatch")
            require(frame["chip_revision_detected"].isascii() and frame["chip_revision_detected"].isdigit(), "Invalid chip revision")
            require(int(frame["chip_revision_detected"]) >= minimum and frame["chip_policy_match"] == "true", "Observed silicon fails policy")
            require(frame["readiness"] == "NOT_QUALIFIED", "Unexpected readiness claim")
            for kind in ("flash", "psram"):
                expected = str(manifest[f"documented_{kind}_bytes"])
                require(frame[f"{kind}_bytes_documented"] == expected, "Documented memory mismatch")
                require(frame[f"{kind}_bytes_observed"] in ("NOT_MEASURED", expected), "Observed memory mismatch")
        except ValueError as error:
            errors.append(str(error))
    memory = parse_memory_log(raw, manifest)
    errors.extend(memory["errors"])
    memory_required = manifest.get("memory_diagnostic_schema") == 1
    if memory_required and frames and not memory["complete_records"]:
        errors.append("Missing memory diagnostic frame")
    # Repeated boot snapshots and probe reports must agree, not merely match
    # profile constants independently.
    if memory["frames"]:
        for boot_key, memory_key in (("flash_bytes_observed", "flash_bytes_rdid"), ("psram_bytes_observed", "psram_bytes_mapped")):
            values = {frame[memory_key] for frame in memory["frames"]}
            if len(values) != 1 or any(frame[boot_key] not in values for frame in frames):
                errors.append("Boot/memory observation mismatch: " + boot_key)
    return {"complete_records": len(frames), "partial_records": partial, "errors": errors,
            "observation_match": bool(frames) and not errors, "memory_diagnostic": memory,
            "frames": frames, "rom_signature_seen": b"ESP-ROM:esp32p4" in raw,
            "reset_reasons": reset_reasons,
            "readiness": "NOT_QUALIFIED"}


def precheck(args):
    require(args.unit and len(args.unit) <= 80 and args.unit.isprintable(), "An explicit printable unit ID is required")
    if args.operator_result != "NOT_RUN":
        require(bool(args.operator_note.strip()), "Operator PASS/FAIL requires an evidence note")
    require(not args.output.resolve().exists(), "Capture output must be a new directory")


def retain(args, raw, manifest, capture_error=None):
    precheck(args)
    output = args.output.resolve()
    output.mkdir(parents=True)
    (output / "raw.log").write_bytes(raw)
    result = parse_log(raw, manifest)
    result.update({"schema": 1, "unit": args.unit, "scenario": args.scenario,
                   "package_manifest_sha256": digest(args.package / "manifest.json"),
                   "expected_elf_sha256": manifest["elf"]["sha256"], "expected_image_id": manifest["image_id"],
                   "raw_log_sha256": digest(output / "raw.log"),
                   "operator_result": args.operator_result, "operator_note": args.operator_note,
                   "scenario_acceptance": "NOT_QUALIFIED", "capture_error":capture_error})
    # A boot log cannot prove a power cycle, reconnect, ROM recovery or exact flash bytes.
    # Operator evidence remains separately attributed, even when a record matches.
    save(output / "result.json", result)
    print(json.dumps({k: result[k] for k in ("unit", "scenario", "observation_match", "errors", "readiness")}, indent=2))
    return 0 if not capture_error and not result["errors"] and (result["observation_match"] or args.scenario in ("factory", "rom-recovery")) else 2


def ingest(args):
    manifest = package(args.package)
    with args.log.open("rb") as file:
        raw = file.read(MAX_LOG + 1)
    require(len(raw) <= MAX_LOG, "Log exceeds 1 MiB")
    return retain(args, raw, manifest)


def capture(args):
    manifest = package(args.package)
    require(0 < args.seconds <= 300, "Capture duration must be 1..300 seconds")
    precheck(args)
    try:
        import serial
    except ImportError as error:
        raise ValueError("Live capture requires pyserial; ingest can process an existing terminal log without it") from error
    port = serial.Serial(port=None, baudrate=args.baud, timeout=0.1)
    port.dtr = False
    port.rts = False
    port.port = args.port
    raw = bytearray()
    # No bytes or reset commands are written. Some serial drivers may still
    # affect reset lines when opening; select the exact service connector first.
    capture_error=None
    try:
        port.open()
        deadline = time.monotonic() + args.seconds
        while time.monotonic() < deadline and len(raw) <= MAX_LOG:
            raw.extend(port.read(min(4096, MAX_LOG + 1 - len(raw))))
    except (OSError,serial.SerialException) as error:
        capture_error=str(error)
    finally:
        port.close()
    if len(raw)>MAX_LOG:
        raw=raw[:MAX_LOG];capture_error="Capture reached the 1 MiB bound"
    return retain(args,bytes(raw),manifest,capture_error)



def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    cmd = commands.add_parser("build", help="Build and hash both diagnostic profiles; never flash")
    cmd.add_argument("--output", type=Path, required=True)
    cmd.add_argument("--target-dir", type=Path, required=True)
    cmd.add_argument("--git", default="git")
    cmd.add_argument("--cargo", default="cargo")
    cmd.add_argument("--rustc", default="rustc")
    cmd.add_argument("--probe", choices=PROBES, default="none", help="Explicit flash RDID/PSRAM scratch image; default activates neither")
    for name in ("ingest", "capture"):
        cmd = commands.add_parser(name, help="Retain a separate unit/scenario observation")
        cmd.add_argument("--package", type=Path, required=True)
        cmd.add_argument("--output", type=Path, required=True)
        cmd.add_argument("--unit", required=True)
        cmd.add_argument("--scenario", choices=SCENARIOS, required=True)
        cmd.add_argument("--operator-result", choices=("NOT_RUN", "PASS", "FAIL"), default="NOT_RUN")
        cmd.add_argument("--operator-note", default="")
        if name == "ingest":
            cmd.add_argument("--log", type=Path, required=True)
        else:
            cmd.add_argument("--port", required=True)
            cmd.add_argument("--seconds", type=int, default=30)
            cmd.add_argument("--baud", type=int, default=115200)
    args = parser.parse_args()
    try:
        return {"build": build, "ingest": ingest, "capture": capture}[args.command](args)
    except (ValueError, OSError, subprocess.CalledProcessError, KeyError, json.JSONDecodeError) as error:
        parser.exit(1, f"ERROR: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
