import pathlib
import shutil
import subprocess
import tempfile
import unittest


class WebClientTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which('cc'), 'Host C compiler required (run on Linux/WSL)')
    def test_web_server_sends_in_pieces_and_serves_others_meanwhile(self):
        app = pathlib.Path(__file__).resolve().parents[2]
        with tempfile.TemporaryDirectory() as directory:
            exe = pathlib.Path(directory) / 'web-client-test'
            subprocess.run(['cc', '-std=c11', '-Wall', '-Wextra', '-Werror', '-I', str(app / 'main'),
                            str(app / 'tools/tests/web_client_test.c'), '-o', str(exe)], check=True)
            subprocess.run([str(exe)], check=True)


if __name__ == '__main__':
    unittest.main()
