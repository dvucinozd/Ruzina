import pathlib
import shutil
import subprocess
import tempfile
import unittest

class UacRecoveryTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which('cc'), 'Host C compiler required (run on Linux/WSL)')
    def test_production_recovery_policy(self):
        app = pathlib.Path(__file__).resolve().parents[2]
        with tempfile.TemporaryDirectory() as directory:
            exe = pathlib.Path(directory) / 'uac-recovery-test'
            subprocess.run(['cc', '-std=c11', '-Wall', '-Wextra', '-Werror',
                            '-I', str(app / 'main'), str(app / 'tools/tests/uac_recovery_test.c'),
                            '-o', str(exe)], check=True)
            subprocess.run([str(exe)], check=True)

    @unittest.skipUnless(shutil.which('cc'), 'Host C compiler required (run on Linux/WSL)')
    def test_actual_driver_fault_paths(self):
        self._run_driver(timing=False)

    @unittest.skipUnless(shutil.which('cc'), 'Host C compiler required (run on Linux/WSL)')
    def test_actual_driver_timing_phases(self):
        self._run_driver(timing=True)

    @unittest.skipUnless(shutil.which('cc'), 'Host C compiler required (run on Linux/WSL)')
    def test_actual_driver_profile_context(self):
        self._run_driver(timing=True, profile=True)

    @unittest.skipUnless(shutil.which('cc'), 'Host C compiler required (run on Linux/WSL)')
    def test_actual_scheduler_hooks(self):
        app = pathlib.Path(__file__).resolve().parents[2]
        with tempfile.TemporaryDirectory() as directory:
            exe = pathlib.Path(directory) / 'uac-scheduler-test'
            subprocess.run(['cc', '-std=c11', '-Wall', '-Wextra', '-Werror', '-DUAC_PROFILE_TEST=1',
                            '-I', str(app / 'main'), str(app / 'main/uac_profile_sched.c'),
                            str(app / 'tools/tests/uac_profile_sched_test.c'), '-o', str(exe)], check=True)
            subprocess.run([str(exe)], check=True)

    def _run_driver(self, timing, profile=False):
        app = pathlib.Path(__file__).resolve().parents[2]
        with tempfile.TemporaryDirectory() as directory:
            exe = pathlib.Path(directory) / 'uac-driver-test'
            (pathlib.Path(directory) / 'esp_timer.h').write_text(
                '#pragma once\n#include <stdint.h>\nint64_t esp_timer_get_time(void);\n')
            subprocess.run(['cc', '-std=c11', '-Wall', '-Wextra', '-Werror', '-Wno-unused-function',
                            f'-DUAC_TEST_TIMING={int(timing)}', f'-DUAC_TEST_PROFILE={int(profile)}', '-I', directory,
                            '-I', str(app / 'main'), str(app / 'tools/tests/uac_driver_test.c'),
                            *([str(app / 'main/uac_profile_sched.c'), '-DUAC_PROFILE_TEST=1'] if profile else []),
                            '-o', str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
