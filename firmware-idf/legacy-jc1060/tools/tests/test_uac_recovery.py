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
        app = pathlib.Path(__file__).resolve().parents[2]
        with tempfile.TemporaryDirectory() as directory:
            exe = pathlib.Path(directory) / 'uac-driver-test'
            subprocess.run(['cc', '-std=c11', '-Wall', '-Wextra', '-Werror', '-Wno-unused-function',
                            '-I', str(app / 'main'), str(app / 'tools/tests/uac_driver_test.c'),
                            '-o', str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
