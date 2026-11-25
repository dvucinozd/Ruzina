"""Signed firmware bundles (.ddjota) for the Web Remote's firmware update.

A bundle is a 188-byte header and the app image (build/ruzina_legacy_probe.bin). The
header's first 124 bytes are signed with ECDSA P-256 over SHA-256 (raw r || s), in the
layout ruzina-ota's `bundle` module reads (byte-compatible with the M2.5 donor's
tools/ota_signing.py). The project and version come from the image's own app description,
so the deck's check of the written slot's description matches them.

    generate --private KEY.pem --public KEY.sec1   new key pair; prints the Rust constant
    bundle --private-key KEY.pem --input APP.bin --output X.ddjota [--key-id ruz-001]
    inspect --public KEY.sec1 --input X.ddjota      checks a bundle, prints its fields

The private key stays outside the repository (D:\\BoardEvidence\\keys); the deck carries
only the public key (rust-core/src/deck_ota.rs).
"""
import argparse
import hashlib
import struct
import sys
from pathlib import Path

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.asymmetric.utils import decode_dss_signature, encode_dss_signature

MAGIC = b'DDJOTA1\0'
SCHEMA = 1
HEADER_SIZE = 188
SIGNED_SIZE = 124
TARGET_P4 = 1
ESP32P4_CHIP_ID = 0x0012
KEY_ID = 'ruz-001'
# The deck's A/B slots (partitions-ota.csv).
SLOT_SIZE = 0x600000
# esp_app_desc_t follows the image header (24 bytes) and the first segment's header (8).
DESC_OFFSET = 32
DESC_MAGIC = 0xABCD5432


def fixed(text, size, label):
    raw = text.encode('ascii')
    if not raw or len(raw) >= size or any(not 0x21 <= b <= 0x7e for b in raw):
        raise ValueError(f'{label} must be 1..{size - 1} printable ASCII bytes: {text!r}')
    return raw + bytes(size - len(raw))


def c_text(field):
    return field.split(b'\0', 1)[0].decode('ascii')


def app_description(image):
    if len(image) < DESC_OFFSET + 256 or image[0] != 0xE9:
        raise ValueError('not an ESP app image')
    if struct.unpack_from('<H', image, 12)[0] != ESP32P4_CHIP_ID:
        raise ValueError('not an ESP32-P4 image')
    if struct.unpack_from('<I', image, DESC_OFFSET)[0] != DESC_MAGIC:
        raise ValueError('no app description')
    version = c_text(image[DESC_OFFSET + 16:DESC_OFFSET + 48])
    project = c_text(image[DESC_OFFSET + 48:DESC_OFFSET + 80])
    return project, version


def generate(args):
    for path in (args.private, args.public):
        if path.exists():
            raise SystemExit(f'{path} exists: not overwritten')
    key = ec.generate_private_key(ec.SECP256R1())
    args.private.parent.mkdir(parents=True, exist_ok=True)
    args.private.write_bytes(key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8,
                                               serialization.NoEncryption()))
    point = key.public_key().public_bytes(serialization.Encoding.X962, serialization.PublicFormat.UncompressedPoint)
    args.public.write_bytes(point)
    print(f'public key sha256={hashlib.sha256(point).hexdigest()}')
    print('pub(crate) const PUBLIC_KEY: [u8; 65] = [')
    for row in range(0, 65, 16):
        print('    ' + ' '.join(f'0x{b:02x},' for b in point[row:row + 16]))
    print('];')


def bundle(args):
    key = serialization.load_pem_private_key(args.private_key.read_bytes(), None)
    if not isinstance(key, ec.EllipticCurvePrivateKey) or key.curve.name != 'secp256r1':
        raise SystemExit('the key must be P-256')
    image = args.input.read_bytes()
    project, version = app_description(image)
    if len(image) > SLOT_SIZE:
        raise SystemExit(f'image {len(image)} bytes is larger than a slot ({SLOT_SIZE})')
    header = bytearray(HEADER_SIZE)
    header[0:8] = MAGIC
    struct.pack_into('<HHBBHI', header, 8, SCHEMA, HEADER_SIZE, TARGET_P4, 0, ESP32P4_CHIP_ID, len(image))
    header[20:52] = fixed(project, 32, 'project')
    header[52:84] = fixed(version, 32, 'version')
    header[84:116] = hashlib.sha256(image).digest()
    header[116:124] = fixed(args.key_id, 8, 'key id')
    r, s = decode_dss_signature(key.sign(bytes(header[:SIGNED_SIZE]), ec.ECDSA(hashes.SHA256())))
    header[124:188] = r.to_bytes(32, 'big') + s.to_bytes(32, 'big')
    if args.output.exists():
        raise SystemExit(f'{args.output} exists: not overwritten')
    args.output.write_bytes(bytes(header) + image)
    print(f'bundle={args.output} project={project} version={version} image_bytes={len(image)} '
          f'key_id={args.key_id} sha256={hashlib.sha256(bytes(header) + image).hexdigest()}')


def inspect(args):
    data = args.input.read_bytes()
    header, image = data[:HEADER_SIZE], data[HEADER_SIZE:]
    if header[:8] != MAGIC:
        raise SystemExit('not a bundle')
    schema, size, target, flags, chip, image_size = struct.unpack_from('<HHBBHI', header, 8)
    point = args.public.read_bytes()
    public = ec.EllipticCurvePublicKey.from_encoded_point(ec.SECP256R1(), point)
    signature = encode_dss_signature(int.from_bytes(header[124:156], 'big'), int.from_bytes(header[156:188], 'big'))
    try:
        public.verify(signature, header[:SIGNED_SIZE], ec.ECDSA(hashes.SHA256()))
        signed = 'ok'
    except InvalidSignature:
        signed = 'BAD'
    hashed = 'ok' if hashlib.sha256(image).digest() == header[84:116] and len(image) == image_size else 'BAD'
    print(f'schema={schema} header={size} target={target} flags={flags} chip=0x{chip:04x} image_bytes={image_size} '
          f'project={c_text(header[20:52])} version={c_text(header[52:84])} key_id={c_text(header[116:124])} '
          f'signature={signed} image={hashed}')
    return 0 if signed == hashed == 'ok' else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest='command', required=True)
    g = commands.add_parser('generate')
    g.add_argument('--private', type=Path, required=True)
    g.add_argument('--public', type=Path, required=True)
    b = commands.add_parser('bundle')
    b.add_argument('--private-key', type=Path, required=True)
    b.add_argument('--input', type=Path, required=True)
    b.add_argument('--output', type=Path, required=True)
    b.add_argument('--key-id', default=KEY_ID)
    i = commands.add_parser('inspect')
    i.add_argument('--public', type=Path, required=True)
    i.add_argument('--input', type=Path, required=True)
    args = parser.parse_args()
    return {'generate': generate, 'bundle': bundle, 'inspect': inspect}[args.command](args) or 0


if __name__ == '__main__':
    sys.exit(main())
