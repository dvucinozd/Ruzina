#![no_std]
#![forbid(unsafe_code)]

use core::str;
use ruzina_core::MediaTrackId;

pub const MEDIA_PATH_MAX: usize = 256;
pub const TRACK_ID_DOMAIN: &[u8] = b"ruzina.track.v1";
pub const FAT32_VOLUME_ID_DOMAIN: &[u8] = b"ruzina.volume.fat32.v1";
pub const EXFAT_VOLUME_ID_DOMAIN: &[u8] = b"ruzina.volume.exfat.v1";
pub const LEGACY_HOTCUE_V2_DOMAIN: &[u8] = b"pajoniiir.hotcue.v2";

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Sha256Digest(pub [u8; 32]);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct VolumeIdentity(pub [u8; 32]);

impl From<Sha256Digest> for VolumeIdentity {
    fn from(value: Sha256Digest) -> Self {
        Self(value.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PersistentMediaId(pub [u8; 32]);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityError {
    EmptyPath,
    ParentTraversal,
    EmbeddedNul,
    PathTooLong,
}

pub struct Sha256 {
    state: [u32; 8],
    total_len: u64,
    buffer: [u8; 64],
    buffer_len: usize,
}

impl Sha256 {
    pub const fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            total_len: 0,
            buffer: [0; 64],
            buffer_len: 0,
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total_len = self.total_len.wrapping_add(data.len() as u64);

        while !data.is_empty() {
            let take = (64 - self.buffer_len).min(data.len());
            self.buffer[self.buffer_len..self.buffer_len + take].copy_from_slice(&data[..take]);
            self.buffer_len += take;
            data = &data[take..];

            if self.buffer_len == 64 {
                let block = self.buffer;
                self.transform(&block);
                self.buffer_len = 0;
            }
        }
    }

    pub fn finalize(mut self) -> Sha256Digest {
        let bits = self.total_len.wrapping_mul(8);
        let mut pad = [0u8; 128];
        pad[0] = 0x80;
        let pad_len = if self.buffer_len < 56 {
            56 - self.buffer_len
        } else {
            120 - self.buffer_len
        };
        self.update(&pad[..pad_len]);
        self.update(&bits.to_be_bytes());

        let mut digest = [0u8; 32];
        for (index, word) in self.state.iter().enumerate() {
            digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        Sha256Digest(digest)
    }

    fn transform(&mut self, block: &[u8; 64]) {
        let mut words = [0u32; 64];
        for (index, chunk) in block.chunks_exact(4).take(16).enumerate() {
            words[index] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }

        let mut a = self.state[0];
        let mut b = self.state[1];
        let mut c = self.state[2];
        let mut d = self.state[3];
        let mut e = self.state[4];
        let mut f = self.state[5];
        let mut g = self.state[6];
        let mut h = self.state[7];

        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ ((!e) & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(choose)
                .wrapping_add(SHA256_K[index])
                .wrapping_add(words[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }

        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

pub fn sha256(data: &[u8]) -> Sha256Digest {
    let mut hash = Sha256::new();
    hash.update(data);
    hash.finalize()
}

pub fn derive_fat32_volume_identity(
    volume_id: u32,
    bytes_per_sector: u16,
    total_sectors: u32,
    partition_first_block: u64,
) -> VolumeIdentity {
    let mut hash = Sha256::new();
    hash.update(FAT32_VOLUME_ID_DOMAIN);
    hash.update(&volume_id.to_le_bytes());
    hash.update(&bytes_per_sector.to_le_bytes());
    hash.update(&total_sectors.to_le_bytes());
    hash.update(&partition_first_block.to_le_bytes());
    VolumeIdentity(hash.finalize().0)
}

pub fn derive_exfat_volume_identity(
    volume_serial: u32,
    bytes_per_sector: u32,
    sectors_per_cluster: u32,
    volume_length: u64,
    partition_first_block: u64,
) -> VolumeIdentity {
    let mut hash = Sha256::new();
    hash.update(EXFAT_VOLUME_ID_DOMAIN);
    hash.update(&volume_serial.to_le_bytes());
    hash.update(&bytes_per_sector.to_le_bytes());
    hash.update(&sectors_per_cluster.to_le_bytes());
    hash.update(&volume_length.to_le_bytes());
    hash.update(&partition_first_block.to_le_bytes());
    VolumeIdentity(hash.finalize().0)
}

pub fn normalize_relative_path<'a>(
    input: &str,
    output: &'a mut [u8],
) -> Result<&'a str, IdentityError> {
    let bytes = input.as_bytes();
    let mut input_pos = 0usize;
    let mut output_len = 0usize;

    while input_pos < bytes.len() {
        while input_pos < bytes.len() && is_separator(bytes[input_pos]) {
            input_pos += 1;
        }
        if input_pos == bytes.len() {
            break;
        }

        let start = input_pos;
        while input_pos < bytes.len() && !is_separator(bytes[input_pos]) {
            input_pos += 1;
        }
        let segment = &bytes[start..input_pos];

        if segment == b"." {
            continue;
        }
        if segment == b".." {
            return Err(IdentityError::ParentTraversal);
        }
        if segment.contains(&0) {
            return Err(IdentityError::EmbeddedNul);
        }

        let separator_len = usize::from(output_len != 0);
        let next_len = output_len
            .checked_add(separator_len)
            .and_then(|value| value.checked_add(segment.len()))
            .ok_or(IdentityError::PathTooLong)?;
        if next_len > MEDIA_PATH_MAX || next_len > output.len() {
            return Err(IdentityError::PathTooLong);
        }

        if separator_len != 0 {
            output[output_len] = b'/';
            output_len += 1;
        }
        output[output_len..output_len + segment.len()].copy_from_slice(segment);
        output_len += segment.len();
    }

    if output_len == 0 {
        return Err(IdentityError::EmptyPath);
    }

    str::from_utf8(&output[..output_len]).map_err(|_| IdentityError::EmbeddedNul)
}

pub fn derive_persistent_track_id(
    volume: VolumeIdentity,
    relative_path: &str,
) -> Result<PersistentMediaId, IdentityError> {
    let mut normalized_storage = [0u8; MEDIA_PATH_MAX];
    let normalized = normalize_relative_path(relative_path, &mut normalized_storage)?;

    let mut hash = Sha256::new();
    hash.update(TRACK_ID_DOMAIN);
    hash.update(&volume.0);
    hash.update(&(normalized.len() as u16).to_le_bytes());
    hash.update(normalized.as_bytes());
    Ok(PersistentMediaId(hash.finalize().0))
}

pub fn derive_track_id(
    volume: VolumeIdentity,
    relative_path: &str,
) -> Result<MediaTrackId, IdentityError> {
    let persistent = derive_persistent_track_id(volume, relative_path)?;
    let mut compact = [0u8; 16];
    compact.copy_from_slice(&persistent.0[..16]);
    Ok(MediaTrackId(compact))
}

pub fn derive_legacy_hotcue_v2(
    export_digest: Sha256Digest,
    usb_relative_path: &str,
    file_size: u64,
    file_mtime: i64,
) -> Result<PersistentMediaId, IdentityError> {
    let path = usb_relative_path.as_bytes();
    if path.is_empty() {
        return Err(IdentityError::EmptyPath);
    }
    if path.len() > u16::MAX as usize {
        return Err(IdentityError::PathTooLong);
    }
    if path.contains(&0) {
        return Err(IdentityError::EmbeddedNul);
    }

    let mut hash = Sha256::new();
    hash.update(LEGACY_HOTCUE_V2_DOMAIN);
    hash.update(&export_digest.0);
    hash.update(&(path.len() as u16).to_le_bytes());
    hash.update(path);
    hash.update(&file_size.to_le_bytes());
    hash.update(&file_mtime.to_le_bytes());
    Ok(PersistentMediaId(hash.finalize().0))
}

const fn is_separator(value: u8) -> bool {
    value == b'/' || value == b'\\'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(value: Sha256Digest) -> [u8; 64] {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut output = [0u8; 64];
        for (index, byte) in value.0.iter().copied().enumerate() {
            output[index * 2] = DIGITS[(byte >> 4) as usize];
            output[index * 2 + 1] = DIGITS[(byte & 0x0f) as usize];
        }
        output
    }

    #[test]
    fn sha256_matches_released_abc_vector() {
        assert_eq!(
            &hex(sha256(b"abc")),
            b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn streaming_sha256_matches_one_shot_digest() {
        let mut streaming = Sha256::new();
        streaming.update(b"a");
        streaming.update(b"b");
        streaming.update(b"c");
        assert_eq!(streaming.finalize(), sha256(b"abc"));
    }

    #[test]
    fn legacy_hotcue_v2_matches_released_byte_contract() {
        let digest = sha256(b"abc");
        let identity = derive_legacy_hotcue_v2(digest, "/Contents/song.wav", 1_234, 5_678).unwrap();
        assert_eq!(
            identity.0,
            [
                0x61, 0xb1, 0x6b, 0xd8, 0xee, 0x8f, 0xed, 0xca, 0xd1, 0xef, 0x4a, 0x91, 0x45, 0x89,
                0x4d, 0xb8, 0xf2, 0x60, 0xbe, 0xcc, 0x68, 0x4f, 0xb0, 0xd8, 0x6b, 0xab, 0x97, 0x65,
                0x8d, 0x66, 0xcb, 0x22,
            ]
        );
    }

    #[test]
    fn fat32_volume_identity_is_stable_and_geometry_bound() {
        let first = derive_fat32_volume_identity(0x1234_5678, 512, 262_144, 2_048);
        let same = derive_fat32_volume_identity(0x1234_5678, 512, 262_144, 2_048);
        let moved = derive_fat32_volume_identity(0x1234_5678, 512, 262_144, 4_096);
        let resized = derive_fat32_volume_identity(0x1234_5678, 512, 524_288, 2_048);

        assert_eq!(first, same);
        assert_ne!(first, moved);
        assert_ne!(first, resized);
    }

    #[test]
    fn exfat_volume_identity_is_stable_and_geometry_bound() {
        let first = derive_exfat_volume_identity(0x1234_5678, 512, 8, 1_000_000, 2_048);
        let same = derive_exfat_volume_identity(0x1234_5678, 512, 8, 1_000_000, 2_048);
        let moved = derive_exfat_volume_identity(0x1234_5678, 512, 8, 1_000_000, 4_096);
        let resized = derive_exfat_volume_identity(0x1234_5678, 512, 8, 2_000_000, 2_048);

        assert_eq!(first, same);
        assert_ne!(first, moved);
        assert_ne!(first, resized);
    }

    #[test]
    fn path_normalization_is_separator_stable_and_bounded() {
        let mut output = [0u8; MEDIA_PATH_MAX];
        assert_eq!(
            normalize_relative_path("/Contents//./Artist\\Track.wav", &mut output).unwrap(),
            "Contents/Artist/Track.wav"
        );
        assert_eq!(
            normalize_relative_path("Contents/Artist/../Track.wav", &mut output),
            Err(IdentityError::ParentTraversal)
        );
        assert_eq!(
            normalize_relative_path("////./", &mut output),
            Err(IdentityError::EmptyPath)
        );
    }

    #[test]
    fn stable_track_id_uses_volume_and_normalized_relative_path() {
        let volume = VolumeIdentity([0x11; 32]);
        let canonical = derive_track_id(volume, "Contents/Artist/Track.wav").unwrap();
        let equivalent = derive_track_id(volume, "/Contents//Artist\\./Track.wav").unwrap();
        assert_eq!(canonical, equivalent);
        assert_eq!(
            canonical,
            MediaTrackId([
                0x03, 0x31, 0x2a, 0x3d, 0x92, 0x29, 0x01, 0xea, 0x83, 0x26, 0x58, 0xcd, 0x04, 0x5a,
                0x83, 0xf2,
            ])
        );
    }

    #[test]
    fn identical_paths_on_different_media_never_alias() {
        let a = derive_track_id(VolumeIdentity([0x11; 32]), "Music/track.flac").unwrap();
        let b = derive_track_id(VolumeIdentity([0x22; 32]), "Music/track.flac").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn different_paths_on_same_media_never_alias_in_fixture() {
        let volume = VolumeIdentity([0x33; 32]);
        let a = derive_track_id(volume, "Music/a.flac").unwrap();
        let b = derive_track_id(volume, "Music/b.flac").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn content_digest_is_separate_from_stable_track_identity() {
        let volume = VolumeIdentity([0x44; 32]);
        let track_id = derive_track_id(volume, "Music/live.wav").unwrap();
        let old_content = sha256(b"old content");
        let new_content = sha256(b"new content");

        assert_ne!(old_content, new_content);
        assert_eq!(track_id, derive_track_id(volume, "Music/live.wav").unwrap());
    }

    #[test]
    fn path_limit_is_fail_closed() {
        let long = [b'a'; MEDIA_PATH_MAX + 1];
        let input = str::from_utf8(&long).unwrap();
        let mut output = [0u8; MEDIA_PATH_MAX + 1];
        assert_eq!(
            normalize_relative_path(input, &mut output),
            Err(IdentityError::PathTooLong)
        );
    }
}
