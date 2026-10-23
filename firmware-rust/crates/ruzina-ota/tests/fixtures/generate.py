#!/usr/bin/env python3
"""Generate signed .ddjota host fixtures for ruzina-ota.

The header layout follows M2.5 tools/ota_signing.py. Bundles are signed with a
throwaway P-256 test key: only its public point (test_key.sec1) is written
here. Without --key a fresh key is generated in memory and discarded, so
regenerating changes every signature and test_key.sec1 together.
"""

import argparse
import hashlib
import struct
from pathlib import Path

from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.asymmetric.utils import decode_dss_signature

# P-256 group order, for the high-S variant of a valid signature.
N = 0xFFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551
PROJECT = "ruzina-m1-test"


def image(size: int, seed: int) -> bytes:
    """Byte pattern behind an ESP header: magic 0xE9, chip id 0x0012 at 12."""
    body = bytearray((i * 31 + seed) & 0xFF for i in range(size))
    body[0] = 0xE9
    body[12:14] = struct.pack("<H", 0x0012)
    return bytes(body)


def fixed(text: str, size: int) -> bytes:
    raw = text.encode()
    return raw + bytes(size - len(raw))


def bundle(key, img: bytes, project: str, version: str, high_s: bool = False) -> bytes:
    header = bytearray(188)
    header[0:8] = b"DDJOTA1\0"
    struct.pack_into("<HHBBHI", header, 8, 1, 188, 1, 0, 0x0012, len(img))
    header[20:52] = fixed(project, 32)
    header[52:84] = fixed(version, 32)
    header[84:116] = hashlib.sha256(img).digest()
    header[116:124] = fixed("rel-001", 8)
    r, s = decode_dss_signature(key.sign(bytes(header[:124]), ec.ECDSA(hashes.SHA256())))
    low = min(s, N - s)
    s = N - low if high_s else low
    header[124:188] = r.to_bytes(32, "big") + s.to_bytes(32, "big")
    return bytes(header) + img


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("out", nargs="?", default=Path(__file__).parent, type=Path)
    parser.add_argument("--key", type=Path, help="reuse a PEM test key")
    args = parser.parse_args()
    if args.key:
        key = serialization.load_pem_private_key(args.key.read_bytes(), password=None)
    else:
        key = ec.generate_private_key(ec.SECP256R1())
    out = args.out
    (out / "test_key.sec1").write_bytes(
        key.public_key().public_bytes(
            serialization.Encoding.X962, serialization.PublicFormat.UncompressedPoint
        )
    )
    newer = image(4096 + 123, 7)
    (out / "newer.ddjota").write_bytes(bundle(key, newer, PROJECT, "M2.5-4-g1a2b3c4d"))
    (out / "newer_high_s.ddjota").write_bytes(
        bundle(key, newer, PROJECT, "M2.5-4-g1a2b3c4d", high_s=True)
    )
    (out / "older.ddjota").write_bytes(
        bundle(key, image(1500, 3), PROJECT, "M2.4-7-g0badc0d")
    )
    (out / "other_project.ddjota").write_bytes(
        bundle(key, image(600, 5), "other-board", "M2.5-9-gfeedbee")
    )


if __name__ == "__main__":
    main()
