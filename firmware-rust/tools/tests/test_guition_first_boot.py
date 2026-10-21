import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import guition_first_boot as boot


def manifest():
    return {"schema": 1, "board": "jc1060", "model": boot.MODELS["jc1060"], "image_id": "exact-source-image",
            "required_min_chip_revision": 302, "documented_flash_bytes": 16777216,
            "documented_psram_bytes": 33554432}


def record(**changes):
    values = {"board_profile": boot.MODELS["jc1060"], "image_id": "exact-source-image", "chip_revision_detected": "302",
              "chip_minimum_required": "302", "chip_policy_match": "true", "flash_bytes_documented": "16777216",
              "psram_bytes_documented": "33554432", "flash_bytes_observed": "NOT_MEASURED",
              "psram_bytes_observed": "NOT_MEASURED", "readiness": "NOT_QUALIFIED"}
    values.update(changes)
    return ("M1_BOOT_DIAGNOSTIC_V1\n" + "".join(f"{key}={value}\n" for key, value in values.items())).encode()


def memory_record(probe="memory", **changes):
    flash = probe in ("flash", "memory")
    psram = probe in ("psram", "memory")
    values = {"board_profile": boot.MODELS["jc1060"], "image_id": "exact-source-image",
              "flash_probe": "ID_DECODED" if flash else "NOT_REQUESTED",
              "flash_rdid_raw": "1840ef" if flash else "NOT_MEASURED",
              "flash_bytes_rdid": "16777216" if flash else "NOT_MEASURED",
              "psram_probe": "CPU_SCRATCH_PASSED" if psram else "NOT_REQUESTED",
              "psram_bytes_mapped": "33554432" if psram else "NOT_MEASURED",
              "psram_scratch_bytes": "65536" if psram else "0", "psram_clock_mhz": "80" if psram else "0",
              "psram_failure_word": "NONE", "psram_expected": "NONE", "psram_actual": "NONE",
              "test_scope": "CPU_CACHED_SCRATCH_ONLY", "readiness": "NOT_QUALIFIED"}
    values.update(changes)
    return ("M1_MEMORY_DIAGNOSTIC_V1\n" + "".join(f"{key}={value}\n" for key, value in values.items())).encode()


class FirstBootTests(unittest.TestCase):
    def test_probe_variants_require_real_consistent_memory_records_without_promoting_acceptance(self):
        for probe in boot.PROBES:
            value = {**manifest(), "diagnostic_probe": probe, "memory_diagnostic_schema": 1}
            boot_record = record(flash_bytes_observed="16777216" if probe in ("flash", "memory") else "NOT_MEASURED",
                                 psram_bytes_observed="33554432" if probe in ("psram", "memory") else "NOT_MEASURED")
            parsed = boot.parse_log(boot_record + memory_record(probe), value)
            self.assertTrue(parsed["observation_match"], parsed)
            self.assertEqual(parsed["readiness"], "NOT_QUALIFIED")
            self.assertFalse(boot.parse_log(boot_record, value)["observation_match"])
            self.assertFalse(boot.parse_log(boot_record + memory_record(probe, image_id="wrong"), value)["observation_match"])
        self.assertFalse(boot.parse_log(record() + memory_record(), value)["observation_match"])

    def test_probe_timeouts_corruption_partial_and_mismatched_measurements_are_retained_as_failure(self):
        value = {**manifest(), "diagnostic_probe": "memory", "memory_diagnostic_schema": 1}
        raw_boot = record(flash_bytes_observed="16777216", psram_bytes_observed="33554432")
        for change in [{"flash_probe":"TIMEOUT"}, {"flash_rdid_raw":"1740ef"}, {"flash_rdid_raw":"000000"},
                       {"flash_bytes_rdid":"8388608"}, {"psram_probe":"CPU_SCRATCH_FAILED"},
                       {"psram_scratch_bytes":"33554432"}, {"psram_clock_mhz":"250"},
                       {"psram_failure_word":"12"}, {"readiness":"PASS"}]:
            with self.subTest(change=change):
                self.assertFalse(boot.parse_log(raw_boot + memory_record(**change), value)["observation_match"])
        for frame in [memory_record().replace(b"image_id=", b"image_id=duplicate\nimage_id="),
                      memory_record().replace(b"readiness=", b"unknown=x\nreadiness="), memory_record()[:100]]:
            self.assertFalse(boot.parse_log(raw_boot + frame, value)["observation_match"])
        parsed = boot.parse_log(raw_boot + memory_record() + b"M1_MEMORY_DIAGNOSTIC_V1\nimage_id=partial", value)
        self.assertTrue(parsed["observation_match"])
        self.assertEqual(parsed["memory_diagnostic"]["partial_records"], 1)

    def test_capture_rejects_bad_arguments_before_constructing_or_opening_serial(self):
        with tempfile.TemporaryDirectory() as tmp:
            folder=Path(tmp)
            for change in [{"unit":""},{"operator_result":"PASS"},{"output":folder}]:
                args=types.SimpleNamespace(package=folder,output=folder/"new",unit="fixture",scenario="cold-boot",
                                           operator_result="NOT_RUN",operator_note="",seconds=1,baud=115200,port="TEST-ONLY")
                for key,value in change.items():setattr(args,key,value)
                serial=types.SimpleNamespace(Serial=unittest.mock.Mock(side_effect=AssertionError("serial must not be touched")),SerialException=OSError)
                with patch.dict(sys.modules,{"serial":serial}),patch.object(boot,"package",return_value=manifest()):
                    with self.assertRaises(ValueError):boot.capture(args)
                    serial.Serial.assert_not_called()

    def test_serial_failure_retains_partial_log_without_writes_or_reset_commands(self):
        with tempfile.TemporaryDirectory() as tmp:
            folder=Path(tmp);package=folder/"package";package.mkdir()
            (package/"manifest.json").write_text("{}")
            args=types.SimpleNamespace(package=package,output=folder/"capture",unit="fixture",scenario="cold-boot",
                                       operator_result="NOT_RUN",operator_note="",seconds=1,baud=115200,port="TEST-ONLY")
            class Serial:
                dtr=True
                rts=True
                def __init__(self,**_kwargs):self.calls=0
                def open(self):
                    self.assert_lines=(self.dtr,self.rts)
                    if self.assert_lines!=(False,False):raise AssertionError("reset lines were not deasserted")
                def read(self,_size):
                    self.calls+=1
                    if self.calls==1:return b"partial captured evidence\n"
                    raise OSError("fixture unplug")
                def write(self,_value):raise AssertionError("capture must not write")
                def close(self):pass
            with patch.dict(sys.modules,{"serial":types.SimpleNamespace(Serial=Serial,SerialException=OSError)}), \
                 patch.object(boot,"package",return_value={**manifest(),"elf":{"sha256":"expected"}}),contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(boot.capture(args),2)
            self.assertEqual((args.output/"raw.log").read_bytes(),b"partial captured evidence\n")
            result=json.loads((args.output/"result.json").read_text())
            self.assertEqual(result["capture_error"],"fixture unplug")
            self.assertEqual(result["scenario_acceptance"],"NOT_QUALIFIED")

    def test_matching_record_keeps_unmeasured_memory_and_unqualified_readiness(self):
        result = boot.parse_log(record(), manifest())
        self.assertTrue(result["observation_match"])
        self.assertEqual(result["readiness"], "NOT_QUALIFIED")
        self.assertEqual(result["frames"][0]["flash_bytes_observed"], "NOT_MEASURED")

    def test_wrong_board_image_silicon_memory_and_readiness_fail_closed(self):
        for change in [{"board_profile": boot.MODELS["jc8012"]}, {"image_id": "different-image"},
                       {"chip_revision_detected": "301"}, {"chip_minimum_required": "301"},
                       {"chip_revision_detected": "garbage"}, {"chip_policy_match": "false"},
                       {"flash_bytes_observed": "8388608"}, {"psram_bytes_documented": "123"}, {"readiness": "PASS"}]:
            with self.subTest(change=change):
                self.assertFalse(boot.parse_log(record(**change), manifest())["observation_match"])

    def test_duplicates_unknown_fields_partial_and_mixed_images_are_not_accepted(self):
        for raw in [record().replace(b"image_id=", b"image_id=duplicate\nimage_id="),
                    record().replace(b"readiness=", b"extra=x\nreadiness="), record()[:100],
                    record() + record(image_id="wrong")]:
            self.assertFalse(boot.parse_log(raw, manifest())["observation_match"])
        result = boot.parse_log(record() + record() + b"M1_BOOT_DIAGNOSTIC_V1\nimage_id=partial", manifest())
        self.assertEqual((result["complete_records"], result["partial_records"]), (2, 1))
        self.assertEqual(result["readiness"], "NOT_QUALIFIED")
        with self.assertRaises(ValueError):
            boot.parse_log(b"x" * (boot.MAX_LOG + 1), manifest())

    def test_retains_raw_failure_and_never_reuses_an_existing_capture_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            folder = Path(tmp)
            package = folder / "package"
            package.mkdir()
            (package / "firmware.elf").write_bytes(b"known-elf")
            (package / "build.log").write_bytes(b"successful local build")
            value = manifest()
            value["elf"] = {"file": "firmware.elf", "sha256": boot.digest(package / "firmware.elf")}
            value["build_log_sha256"] = boot.digest(package / "build.log")
            boot.save(package / "manifest.json", value)
            source = folder / "input.log"
            source.write_bytes(record(image_id="wrong"))
            args = types.SimpleNamespace(package=package, output=folder / "capture", log=source,
                                         unit="test-unit", scenario="cold-boot", operator_result="NOT_RUN", operator_note="")
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(boot.ingest(args), 2)
            result = json.loads((args.output / "result.json").read_text())
            self.assertFalse(result["observation_match"])
            self.assertEqual(result["scenario_acceptance"], "NOT_QUALIFIED")
            self.assertEqual(boot.digest(args.output / "raw.log"), boot.digest(source))
            with self.assertRaises(ValueError):
                boot.ingest(args)
            (package / "firmware.elf").write_bytes(b"tampered")
            args.output = folder / "tampered"
            with self.assertRaises(ValueError):
                boot.ingest(args)
            self.assertFalse(args.output.exists())

    def test_build_isolates_board_images_hashes_and_rejects_source_mutation(self):
        for probe, mutate in [(probe, False) for probe in boot.PROBES] + [("memory", True)]:
            with self.subTest(probe=probe, mutate=mutate), tempfile.TemporaryDirectory() as tmp:
                folder = Path(tmp)
                (folder / "firmware-rust").mkdir()
                source = folder / "firmware-rust/source.rs"
                source.write_text("pinned source")
                args = types.SimpleNamespace(output=folder / "package", target_dir=folder / "target",
                                             git="git", cargo="cargo", rustc="rustc", probe=probe)
                def fake_git(_args, *command):
                    return {"ls-files": b"firmware-rust/source.rs\0", "rev-parse": b"a" * 40,
                            "status": b" M firmware-rust/source.rs"}[command[0]]
                def compile_image(command, **kwargs):
                    self.assertNotIn("flash", command)
                    features = command[command.index("--features") + 1]
                    self.assertEqual([name for name in features.split(",") if name.startswith("guition-")], list(boot.PROBES[probe]))
                    image = args.target_dir / boot.TARGET / "release/pajoniiir-m1-firmware"
                    image.parent.mkdir(parents=True, exist_ok=True)
                    image.write_bytes(b"ELF" + kwargs["env"]["M1_IMAGE_ID"].encode())
                    kwargs["stdout"].write(b"mock build; no device")
                    if mutate:
                        source.write_text("changed during build")
                with patch.object(boot, "ROOT", folder), patch.object(boot, "git", fake_git), \
                     patch.object(boot.subprocess, "check_output", return_value="pinned compiler"), \
                     patch.object(boot.subprocess, "run", side_effect=compile_image), contextlib.redirect_stdout(io.StringIO()):
                    if mutate:
                        with self.assertRaises(ValueError):
                            boot.build(args)
                        self.assertEqual(json.loads((args.output / "build-manifest.json").read_text())["status"], "BUILD_FAILED")
                    else:
                        boot.build(args)
                        first = boot.package(args.output / "jc1060")
                        second = boot.package(args.output / "jc8012")
                        self.assertNotEqual(first["elf"]["sha256"], second["elf"]["sha256"])
                        self.assertNotEqual(first["image_id"], second["image_id"])
                        self.assertEqual(first["readiness"], "NOT_QUALIFIED")
                        self.assertEqual(first["diagnostic_probe"], probe)
                        self.assertTrue(first["image_id"].endswith("-" + probe))
                        with self.assertRaises(ValueError):
                            boot.build(args)


if __name__ == "__main__":
    unittest.main()
