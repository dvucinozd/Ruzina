"""CC0 synthetic cover fixtures (RP-07).

Baseline covers are encoded by ImageMagick 6.9.12 (libjpeg), the standard
layouts Rekordbox exports use; `*.rgb` is an independent full-resolution RGB24
decode of each JPEG by FFmpeg 6.1.1. Two refusal fixtures are kept: a
progressive JPEG and FFmpeg's own MJPEG 4:4:4, whose 1x2 sampling on every
component M2.5's TJpgDec rules (and this decoder) do not accept.
"""
from pathlib import Path
import math
import subprocess

ROOT = Path(__file__).resolve().parent


def run(*args):
    subprocess.run(list(args), cwd=ROOT, check=True)


def ffmpeg(*args):
    run("ffmpeg", "-hide_banner", "-loglevel", "error", "-y", *args)


def image(name, width, height):
    data = bytearray()
    for y in range(height):
        for x in range(width):
            ring = math.hypot(x - width * 0.6, y - height * 0.4) < min(width, height) * 0.3
            r = int(255 * x / max(width - 1, 1))
            g = int(255 * y / max(height - 1, 1))
            b = 230 if ring else int(60 + 40 * math.sin((x + y) * 0.2))
            data.extend((r, g, b))
    (ROOT / (name + ".ppm")).write_bytes(b"P6\n%d %d\n255\n" % (width, height) + bytes(data))


for name, width, height, options in [
    ("cover-420", 96, 72, ["-sampling-factor", "2x2,1x1,1x1"]),
    ("cover-444", 34, 34, ["-sampling-factor", "1x1,1x1,1x1"]),
    ("cover-422", 50, 38, ["-sampling-factor", "2x1,1x1,1x1"]),
    ("cover-gray", 30, 20, ["-colorspace", "Gray"]),
]:
    image(name, width, height)
    run("convert", name + ".ppm", "-strip", "-quality", "90", *options, name + ".jpg")
    ffmpeg("-i", name + ".jpg", "-f", "rawvideo", "-pix_fmt", "rgb24", name + ".rgb")
    (ROOT / (name + ".ppm")).unlink()  # generated intermediate only

image("refused", 40, 30)
run("convert", "refused.ppm", "-strip", "-interlace", "JPEG", "cover-progressive.jpg")
ffmpeg("-i", "refused.ppm", "-pix_fmt", "yuvj444p", "-q:v", "3", "cover-ffmpeg-444.jpg")
(ROOT / "refused.ppm").unlink()  # generated intermediate only
