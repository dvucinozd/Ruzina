import pathlib
import shutil
import subprocess
import tempfile
import unittest


class UsbMscTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which('cc'), 'Host C compiler required (run on Linux/WSL)')
    def test_actual_stick_driver_bind_and_recovery_faults(self):
        app = pathlib.Path(__file__).resolve().parents[2]
        tests = app / 'tools/tests'
        with tempfile.TemporaryDirectory() as directory:
            exe = pathlib.Path(directory) / 'usb-msc-test'
            subprocess.run(['cc', '-std=c11', '-Wall', '-Wextra', '-Werror',
                            '-I', str(tests / 'msc_stubs'), '-I', str(app / 'main'),
                            str(tests / 'usb_msc_test.c'), '-o', str(exe)], check=True)
            subprocess.run([str(exe)], check=True)


if __name__ == '__main__':
    unittest.main()
