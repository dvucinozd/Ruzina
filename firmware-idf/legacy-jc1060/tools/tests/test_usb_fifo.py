import pathlib
import shutil
import subprocess
import tempfile
import unittest


class UsbFifoTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which('cc'), 'Host C compiler required (run on Linux/WSL)')
    def test_the_hs_fifo_split_fits_a_540_byte_isochronous_packet(self):
        app = pathlib.Path(__file__).resolve().parents[2]
        with tempfile.TemporaryDirectory() as directory:
            exe = pathlib.Path(directory) / 'usb-fifo-test'
            subprocess.run(['cc', '-std=c11', '-Wall', '-Wextra', '-Werror', '-I', str(app / 'main'),
                            str(app / 'tools/tests/usb_fifo_test.c'), '-o', str(exe)], check=True)
            subprocess.run([str(exe)], check=True)


if __name__ == '__main__':
    unittest.main()
