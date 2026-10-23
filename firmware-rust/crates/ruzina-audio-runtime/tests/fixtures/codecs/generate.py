"""CC0 synthetic audio fixtures. Run with Python 3 and FFmpeg (baseline 6.1.1)."""
from pathlib import Path
import math
import subprocess
import wave

ROOT = Path(__file__).resolve().parent


def ffmpeg(*args):
    subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", *args], cwd=ROOT, check=True)


def crc(data, width, polynomial):
    value = 0
    for byte in data:
        value ^= byte << (width - 8)
        for _ in range(8):
            value = (value << 1) ^ (polynomial if value & (1 << (width - 1)) else 0)
            value &= (1 << width) - 1
    return value


for name, rate, channels in [("stereo", 48000, 2), ("vbr", 44100, 2), ("mono", 22050, 1)]:
    data = bytearray()
    for i in range(12000):
        for channel in range(channels):
            value = int(11000 * math.sin(i * i / 700000 + i * (0.023 + channel * 0.019)) + 2100 * math.sin(i * 0.0037))
            data.extend(value.to_bytes(2, "little", signed=True))
    with wave.open(str(ROOT / (name + ".wav")), "wb") as out:
        out.setnchannels(channels)
        out.setsampwidth(2)
        out.setframerate(rate)
        out.writeframes(data)

ffmpeg("-i", "stereo.wav", "-c:a", "libmp3lame", "-b:a", "128k", "cbr.mp3")
ffmpeg("-i", "vbr.wav", "-c:a", "libmp3lame", "-q:a", "1", "vbr.mp3")
ffmpeg("-i", "mono.wav", "-c:a", "libmp3lame", "-b:a", "64k", "mono.mp3")
ffmpeg("-i", "stereo.wav", "-c:a", "libmp3lame", "-b:a", "128k", "-write_xing", "0", "untagged.mp3")
ffmpeg("-i", "stereo.wav", "-c:a", "flac", "-compression_level", "8", "stereo.flac")
ffmpeg("-i", "mono.wav", "-c:a", "flac", "mono.flac")
ffmpeg("-i", "stereo.wav", "-ar", "96000", "-sample_fmt", "s32", "-c:a", "flac", "deep.flac")
for name in ["cbr", "vbr"]:
    ffmpeg("-i", name + ".mp3", "-ac", "2", "-f", "s16le", name + "-reference.pcm")
# The product duplicates mono at unity; FFmpeg's default upmix attenuates by 3 dB.
ffmpeg("-i", "mono.mp3", "-af", "pan=stereo|c0=c0|c1=c0", "-f", "s16le", "mono-reference.pcm")
ffmpeg("-i", "deep.flac", "-ac", "2", "-f", "s16le", "deep-reference.pcm")

# A native FLAC independent-stereo VERBATIM frame, 256 samples, 32-bit PCM.
# STREAMINFO MD5 is zero (not supplied); frame/header CRCs are supplied.
values = [0, 2147483647, -2147483648, 65536, -65536, 32768, -32768, 123456789]
packed = (48000 << 44) | (1 << 41) | (31 << 36) | 256
info = b"\x01\x00\x01\x00" + bytes(6) + packed.to_bytes(8, "big") + bytes(16)
header = b"\xff\xf8\x8a\x10\x00"
frame = bytearray(header + bytes([crc(header, 8, 0x07)]))
for channel in range(2):
    frame.append(2)  # VERBATIM subframe, no wasted bits
    for i in range(256):
        frame.extend(values[(i + channel) % len(values)].to_bytes(4, "big", signed=True))
frame.extend(crc(frame, 16, 0x8005).to_bytes(2, "big"))
(ROOT / "wide32.flac").write_bytes(b"fLaC\x80\x00\x00\x22" + info + frame)
ffmpeg("-i", "wide32.flac", "-f", "s16le", "wide32-reference.pcm")

with wave.open(str(ROOT / "long-input.wav"), "wb") as out:
    out.setnchannels(2)
    out.setsampwidth(2)
    out.setframerate(48000)
    data = bytearray()
    for i in range(8 * 48000):
        for c in range(2):
            value = int(10000 * math.sin(i * (0.023 + c * 0.017)))
            data.extend(value.to_bytes(2, "little", signed=True))
    out.writeframes(data)
ffmpeg("-i", "long-input.wav", "-c:a", "libmp3lame", "-b:a", "128k", "long.mp3")
(ROOT / "long-input.wav").unlink()  # generated intermediate only


def long_vbr():
    """RP-08: eight seconds whose tone and noise sections force VBR frame sizes."""
    seed = 12345
    with wave.open(str(ROOT / "long-vbr-input.wav"), "wb") as out:
        out.setnchannels(2)
        out.setsampwidth(2)
        out.setframerate(44100)
        data = bytearray()
        for i in range(8 * 44100):
            noisy = (i // 22050) % 3 == 1
            for c in range(2):
                seed = (seed * 1103515245 + 12345) & 0x7FFFFFFF
                noise = (seed >> 8) % 12000 - 6000 if noisy else 0
                value = int(7000 * math.sin(i * (0.011 + c * 0.013))) + noise
                data.extend(value.to_bytes(2, "little", signed=True))
        out.writeframes(data)
    ffmpeg("-i", "long-vbr-input.wav", "-c:a", "libmp3lame", "-q:a", "5", "long-vbr.mp3")
    ffmpeg("-i", "long-vbr-input.wav", "-c:a", "libmp3lame", "-q:a", "5", "-write_xing", "0",
           "long-vbr-untagged.mp3")
    (ROOT / "long-vbr-input.wav").unlink()  # generated intermediate only


long_vbr()
