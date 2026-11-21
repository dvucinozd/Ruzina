import pathlib
import shutil
import subprocess
import tempfile
import unittest


class FillGapTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which('cc'), 'Host C compiler required (run on Linux/WSL)')
    def test_the_screen_starts_its_work_between_fills(self):
        app = pathlib.Path(__file__).resolve().parents[2]
        with tempfile.TemporaryDirectory() as directory:
            exe = pathlib.Path(directory) / 'fill-gap-test'
            subprocess.run(['cc', '-std=c11', '-Wall', '-Wextra', '-Werror', '-I', str(app / 'main'),
                            str(app / 'tools/tests/fill_gap_test.c'), '-o', str(exe)], check=True)
            subprocess.run([str(exe)], check=True)


if __name__ == '__main__':
    unittest.main()
