//! Signed `.ddjota` bundle header (M2.5 `ota_manifest`, byte-compatible).
//!
//! A bundle is a 188-byte header followed by the ESP application image. The
//! first 124 header bytes are signed with ECDSA P-256 over SHA-256; the raw
//! `r || s` signature fills the remaining 64. The header binds target, chip,
//! project, version, image size and image SHA-256, so nothing reaches flash
//! before every field is checked and the signature verified.

use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};

pub const MAGIC: [u8; 8] = *b"DDJOTA1\0";
pub const SCHEMA_VERSION: u16 = 1;
pub const HEADER_SIZE: usize = 188;
pub const SIGNED_SIZE: usize = 124;
pub const PROJECT_SIZE: usize = 32;
pub const VERSION_SIZE: usize = 32;
pub const KEY_ID_SIZE: usize = 8;
pub const RELEASE_KEY_ID: &str = "rel-001";
pub const TARGET_P4: u8 = 1;
pub const ESP32P4_CHIP_ID: u16 = 0x0012;
/// ESP application image header: magic 0xE9, chip id at offset 12.
pub const IMAGE_HEADER_SIZE: usize = 24;
pub const MAX_IMAGE_SIZE: u32 = 0x40_0000;

/// M2.5 release public key (`ddj_ota_release_public.der`, prime256v1),
/// as an uncompressed SEC1 point.
pub const RELEASE_PUBLIC_KEY: [u8; 65] = [
    0x04, 0xd4, 0xac, 0xe4, 0xe9, 0xd3, 0x16, 0x19, 0xde, 0xaa, 0x33, 0x77, 0xe5, 0x15, 0xf0, 0xb7,
    0x4d, 0xff, 0x6a, 0x15, 0x4f, 0xa5, 0x4a, 0x9e, 0xe1, 0xcc, 0x47, 0x46, 0x22, 0x43, 0xcf, 0xc1,
    0xb7, 0x0b, 0x93, 0x32, 0x77, 0xb5, 0x98, 0xdb, 0x8e, 0x84, 0x56, 0xf4, 0x0e, 0x7e, 0x26, 0x87,
    0xaa, 0xee, 0x66, 0x53, 0x8f, 0xb7, 0xc7, 0x51, 0x82, 0x66, 0xc0, 0xe3, 0xe7, 0x0e, 0xdb, 0x92,
    0x98,
];

const OFFSET_SCHEMA: usize = 8;
const OFFSET_HEADER_SIZE: usize = 10;
const OFFSET_TARGET: usize = 12;
const OFFSET_FLAGS: usize = 13;
const OFFSET_CHIP_ID: usize = 14;
const OFFSET_IMAGE_SIZE: usize = 16;
const OFFSET_PROJECT: usize = 20;
const OFFSET_VERSION: usize = 52;
const OFFSET_SHA256: usize = 84;
const OFFSET_KEY_ID: usize = 116;
const OFFSET_SIGNATURE: usize = 124;

/// A NUL-terminated, zero-padded fixed text field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixedText<const N: usize> {
    bytes: [u8; N],
    len: u8,
}

impl<const N: usize> FixedText<N> {
    /// Requires 1..N-1 bytes, one NUL, zero padding and printable ASCII.
    pub fn from_field(field: &[u8]) -> Option<Self> {
        if field.len() != N {
            return None;
        }
        let len = field.iter().position(|byte| *byte == 0)?;
        if len == 0
            || field[len..].iter().any(|byte| *byte != 0)
            || !field[..len].iter().all(|byte| (0x21..=0x7e).contains(byte))
        {
            return None;
        }
        let mut bytes = [0; N];
        bytes[..len].copy_from_slice(&field[..len]);
        Some(Self {
            bytes,
            len: len as u8,
        })
    }

    pub fn new(text: &str) -> Option<Self> {
        let mut field = [0; N];
        let raw = text.as_bytes();
        if raw.len() >= N {
            return None;
        }
        field[..raw.len()].copy_from_slice(raw);
        Self::from_field(&field)
    }

    pub fn as_str(&self) -> &str {
        // Construction admits printable ASCII only.
        core::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or("")
    }
}

pub type Project = FixedText<PROJECT_SIZE>;
pub type Version = FixedText<VERSION_SIZE>;

/// What this firmware accepts. `project` is board-specific, so a bundle for
/// one Guition model cannot be installed on the other.
#[derive(Clone, Copy, Debug)]
pub struct Expected<'a> {
    pub target: u8,
    pub chip_id: u16,
    pub project: &'a str,
    pub max_image_size: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Manifest {
    pub image_size: u32,
    pub project: Project,
    pub version: Version,
    pub image_sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestError {
    HeaderSize,
    Magic,
    Schema,
    Target,
    Flags,
    Chip,
    ImageSize,
    Project,
    Version,
    Key,
    Signature,
}

impl ManifestError {
    pub const fn name(self) -> &'static str {
        match self {
            Self::HeaderSize => "bad manifest header size",
            Self::Magic => "bad bundle magic",
            Self::Schema => "unsupported manifest schema",
            Self::Target => "wrong manifest target",
            Self::Flags => "unsupported manifest flags",
            Self::Chip => "wrong manifest chip",
            Self::ImageSize => "bad manifest image size",
            Self::Project => "wrong manifest project",
            Self::Version => "bad manifest version",
            Self::Key => "unknown manifest signing key",
            Self::Signature => "invalid manifest signature",
        }
    }
}

fn le16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

/// Field checks in the M2.5 order; the signature is a separate step so the
/// caller can report which one failed.
pub fn parse_header(header: &[u8], expected: &Expected<'_>) -> Result<Manifest, ManifestError> {
    if header.len() != HEADER_SIZE {
        return Err(ManifestError::HeaderSize);
    }
    if header[..8] != MAGIC {
        return Err(ManifestError::Magic);
    }
    if le16(header, OFFSET_SCHEMA) != SCHEMA_VERSION {
        return Err(ManifestError::Schema);
    }
    if usize::from(le16(header, OFFSET_HEADER_SIZE)) != HEADER_SIZE {
        return Err(ManifestError::HeaderSize);
    }
    if header[OFFSET_TARGET] != expected.target {
        return Err(ManifestError::Target);
    }
    if header[OFFSET_FLAGS] != 0 {
        return Err(ManifestError::Flags);
    }
    if le16(header, OFFSET_CHIP_ID) != expected.chip_id {
        return Err(ManifestError::Chip);
    }
    let image_size = u32::from_le_bytes(
        header[OFFSET_IMAGE_SIZE..OFFSET_IMAGE_SIZE + 4]
            .try_into()
            .unwrap_or([0; 4]),
    );
    if (image_size as usize) < IMAGE_HEADER_SIZE || image_size > expected.max_image_size {
        return Err(ManifestError::ImageSize);
    }
    let project = Project::from_field(&header[OFFSET_PROJECT..OFFSET_PROJECT + PROJECT_SIZE])
        .filter(|project| project.as_str() == expected.project)
        .ok_or(ManifestError::Project)?;
    let version = Version::from_field(&header[OFFSET_VERSION..OFFSET_VERSION + VERSION_SIZE])
        .ok_or(ManifestError::Version)?;
    FixedText::<KEY_ID_SIZE>::from_field(&header[OFFSET_KEY_ID..OFFSET_KEY_ID + KEY_ID_SIZE])
        .filter(|key| key.as_str() == RELEASE_KEY_ID)
        .ok_or(ManifestError::Key)?;
    let mut image_sha256 = [0; 32];
    image_sha256.copy_from_slice(&header[OFFSET_SHA256..OFFSET_SHA256 + 32]);
    Ok(Manifest {
        image_size,
        project,
        version,
        image_sha256,
    })
}

/// ECDSA P-256/SHA-256 over the signed prefix. Like mbedTLS in M2.5, both
/// low- and high-S encodings of a valid signature verify.
pub fn verify_signature(header: &[u8], public_key: &[u8; 65]) -> Result<(), ManifestError> {
    if header.len() != HEADER_SIZE {
        return Err(ManifestError::HeaderSize);
    }
    let key = VerifyingKey::from_sec1_bytes(public_key).map_err(|_| ManifestError::Key)?;
    let signature = Signature::from_slice(&header[OFFSET_SIGNATURE..HEADER_SIZE])
        .map_err(|_| ManifestError::Signature)?;
    let signature = signature.normalize_s().unwrap_or(signature);
    key.verify(&header[..SIGNED_SIZE], &signature)
        .map_err(|_| ManifestError::Signature)
}

/// The first image bytes must be an ESP32-P4 application before the inactive
/// slot is erased (M2.5 `p4_ota_policy_header_valid`).
pub fn image_header_valid(image: &[u8], chip_id: u16) -> bool {
    image.len() >= IMAGE_HEADER_SIZE && image[0] == 0xe9 && le16(image, 12) == chip_id
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEWER: &[u8] = include_bytes!("../tests/fixtures/newer.ddjota");
    const NEWER_HIGH_S: &[u8] = include_bytes!("../tests/fixtures/newer_high_s.ddjota");
    const OTHER_PROJECT: &[u8] = include_bytes!("../tests/fixtures/other_project.ddjota");
    const TEST_KEY: &[u8; 65] = include_bytes!("../tests/fixtures/test_key.sec1");
    const EXPECTED: Expected<'static> = Expected {
        target: TARGET_P4,
        chip_id: ESP32P4_CHIP_ID,
        project: "ruzina-m1-test",
        max_image_size: MAX_IMAGE_SIZE,
    };

    fn header(bundle: &[u8]) -> [u8; HEADER_SIZE] {
        bundle[..HEADER_SIZE].try_into().unwrap()
    }

    #[test]
    fn signed_fixture_binds_every_field_and_verifies() {
        let parsed = parse_header(&header(NEWER), &EXPECTED).unwrap();
        assert_eq!(parsed.image_size as usize, NEWER.len() - HEADER_SIZE);
        assert_eq!(parsed.version.as_str(), "M2.5-4-g1a2b3c4d");
        assert_eq!(parsed.project.as_str(), "ruzina-m1-test");
        assert_eq!(verify_signature(&header(NEWER), TEST_KEY), Ok(()));
        assert_eq!(verify_signature(&header(NEWER_HIGH_S), TEST_KEY), Ok(()));
        assert!(image_header_valid(&NEWER[HEADER_SIZE..], ESP32P4_CHIP_ID));
        assert_eq!(
            verify_signature(&header(NEWER), &RELEASE_PUBLIC_KEY),
            Err(ManifestError::Signature),
            "a test-key bundle is not a release bundle"
        );
        assert!(VerifyingKey::from_sec1_bytes(&RELEASE_PUBLIC_KEY).is_ok());
    }

    #[test]
    fn a_valid_signature_never_overrides_identity() {
        assert_eq!(
            parse_header(&header(OTHER_PROJECT), &EXPECTED),
            Err(ManifestError::Project)
        );
        assert_eq!(verify_signature(&header(OTHER_PROJECT), TEST_KEY), Ok(()));
    }

    #[test]
    fn every_header_field_is_checked() {
        let good = header(NEWER);
        let mutate = |offset: usize, value: u8| {
            let mut bad = good;
            bad[offset] = value;
            parse_header(&bad, &EXPECTED)
        };
        assert_eq!(mutate(0, b'X'), Err(ManifestError::Magic));
        assert_eq!(mutate(OFFSET_SCHEMA, 2), Err(ManifestError::Schema));
        assert_eq!(
            mutate(OFFSET_HEADER_SIZE, 189),
            Err(ManifestError::HeaderSize)
        );
        assert_eq!(mutate(OFFSET_TARGET, 2), Err(ManifestError::Target));
        assert_eq!(mutate(OFFSET_FLAGS, 1), Err(ManifestError::Flags));
        assert_eq!(mutate(OFFSET_CHIP_ID, 0x13), Err(ManifestError::Chip));
        assert_eq!(
            mutate(OFFSET_IMAGE_SIZE + 3, 0x01),
            Err(ManifestError::ImageSize)
        );
        assert_eq!(
            mutate(OFFSET_PROJECT + 31, b'x'),
            Err(ManifestError::Project)
        );
        assert_eq!(mutate(OFFSET_VERSION, 0), Err(ManifestError::Version));
        assert_eq!(
            mutate(OFFSET_VERSION + 31, b'1'),
            Err(ManifestError::Version)
        );
        assert_eq!(mutate(OFFSET_KEY_ID + 6, b'2'), Err(ManifestError::Key));
        let mut tiny = good;
        tiny[OFFSET_IMAGE_SIZE..OFFSET_IMAGE_SIZE + 4].copy_from_slice(&23u32.to_le_bytes());
        assert_eq!(
            parse_header(&tiny, &EXPECTED),
            Err(ManifestError::ImageSize)
        );
        assert_eq!(
            parse_header(&good[..187], &EXPECTED),
            Err(ManifestError::HeaderSize)
        );
        let mut tampered = good;
        tampered[OFFSET_SHA256] ^= 1;
        assert!(parse_header(&tampered, &EXPECTED).is_ok());
        assert_eq!(
            verify_signature(&tampered, TEST_KEY),
            Err(ManifestError::Signature)
        );
        let mut zero_sig = good;
        zero_sig[OFFSET_SIGNATURE..].fill(0);
        assert_eq!(
            verify_signature(&zero_sig, TEST_KEY),
            Err(ManifestError::Signature)
        );
    }

    #[test]
    fn image_header_must_be_an_esp32p4_application() {
        let image = &NEWER[HEADER_SIZE..];
        assert!(!image_header_valid(&image[..23], ESP32P4_CHIP_ID));
        assert!(!image_header_valid(image, 0x0005));
        let mut wrong_magic: [u8; 24] = image[..24].try_into().unwrap();
        wrong_magic[0] = 0xe8;
        assert!(!image_header_valid(&wrong_magic, ESP32P4_CHIP_ID));
    }
}
