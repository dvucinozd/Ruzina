use core::char;

use pajoniiir_media_fs::{EntryKind, FS_NAME_MAX};

const EXFAT_FILE_ENTRY: u8 = 0x85;
const EXFAT_STREAM_EXTENSION_ENTRY: u8 = 0xc0;
const EXFAT_FILE_NAME_ENTRY: u8 = 0xc1;
const EXFAT_ENTRY_IN_USE: u8 = 0x80;
const EXFAT_ENTRY_SECONDARY: u8 = 0x40;
const EXFAT_ENTRY_BENIGN: u8 = 0x20;
const EXFAT_ALLOCATION_POSSIBLE: u8 = 0x01;
const EXFAT_NO_FAT_CHAIN: u8 = 0x02;
const EXFAT_DIRECTORY_ATTRIBUTE: u16 = 0x0010;
const EXFAT_NAME_UNITS_PER_ENTRY: usize = 15;
const EXFAT_NAME_MAX_UNITS: usize = 255;
const EXFAT_DIRECTORY_MAX_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExFatEntrySetError {
    NotFileEntry(u8),
    InvalidSecondaryCount(u8),
    TooManySecondaries,
    MissingStreamExtension,
    InvalidStreamExtension,
    InvalidNameLength(u8),
    InvalidFileNameEntry(u8),
    UnexpectedCriticalSecondary(u8),
    InvalidAllocation,
    ValidDataBeyondLength,
    DirectoryTooLarge(u64),
    ChecksumMismatch { expected: u16, observed: u16 },
    InvalidUtf16,
    NameTooLong,
    IncompleteEntrySet { expected: u8, observed: u8 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExFatParsedFileEntry {
    pub kind: EntryKind,
    pub first_cluster: u32,
    pub data_length: u64,
    pub valid_data_length: u64,
    pub no_fat_chain: bool,
    pub file_attributes: u16,
    pub name_len: u16,
}

impl ExFatParsedFileEntry {
    pub fn name<'a>(&self, storage: &'a [u8]) -> Option<&'a [u8]> {
        let len = self.name_len as usize;
        if len <= storage.len() {
            Some(&storage[..len])
        } else {
            None
        }
    }
}

pub struct ExFatFileEntrySetParser {
    secondary_count: u8,
    consumed: u8,
    expected_checksum: u16,
    checksum: u16,
    file_attributes: u16,
    stream_seen: bool,
    name_length: u8,
    expected_name_entries: u8,
    name_entries: u8,
    first_cluster: u32,
    data_length: u64,
    valid_data_length: u64,
    no_fat_chain: bool,
    name_units: [u16; EXFAT_NAME_MAX_UNITS],
}

impl ExFatFileEntrySetParser {
    pub fn begin(primary: &[u8; 32]) -> Result<Self, ExFatEntrySetError> {
        if primary[0] != EXFAT_FILE_ENTRY {
            return Err(ExFatEntrySetError::NotFileEntry(primary[0]));
        }

        let secondary_count = primary[1];
        if secondary_count < 2 {
            return Err(ExFatEntrySetError::InvalidSecondaryCount(secondary_count));
        }

        let expected_checksum = u16::from_le_bytes([primary[2], primary[3]]);
        let file_attributes = u16::from_le_bytes([primary[4], primary[5]]);

        Ok(Self {
            secondary_count,
            consumed: 0,
            expected_checksum,
            checksum: checksum_primary(primary),
            file_attributes,
            stream_seen: false,
            name_length: 0,
            expected_name_entries: 0,
            name_entries: 0,
            first_cluster: 0,
            data_length: 0,
            valid_data_length: 0,
            no_fat_chain: false,
            name_units: [0; EXFAT_NAME_MAX_UNITS],
        })
    }

    pub const fn expected_secondaries(&self) -> u8 {
        self.secondary_count
    }

    pub const fn consumed_secondaries(&self) -> u8 {
        self.consumed
    }

    pub fn push_secondary(&mut self, entry: &[u8; 32]) -> Result<(), ExFatEntrySetError> {
        if self.consumed >= self.secondary_count {
            return Err(ExFatEntrySetError::TooManySecondaries);
        }

        self.checksum = checksum_bytes(self.checksum, entry);
        let ordinal = self.consumed;
        self.consumed = self.consumed.saturating_add(1);

        if ordinal == 0 {
            return self.parse_stream_extension(entry);
        }

        if self.name_entries < self.expected_name_entries {
            return self.parse_file_name(entry);
        }

        let entry_type = entry[0];
        if entry_type & (EXFAT_ENTRY_IN_USE | EXFAT_ENTRY_SECONDARY)
            != (EXFAT_ENTRY_IN_USE | EXFAT_ENTRY_SECONDARY)
            || entry_type & EXFAT_ENTRY_BENIGN == 0
        {
            return Err(ExFatEntrySetError::UnexpectedCriticalSecondary(entry_type));
        }

        Ok(())
    }

    pub fn finish(
        self,
        name_storage: &mut [u8; FS_NAME_MAX],
    ) -> Result<ExFatParsedFileEntry, ExFatEntrySetError> {
        if self.consumed != self.secondary_count {
            return Err(ExFatEntrySetError::IncompleteEntrySet {
                expected: self.secondary_count,
                observed: self.consumed,
            });
        }
        if !self.stream_seen {
            return Err(ExFatEntrySetError::MissingStreamExtension);
        }
        if self.name_entries != self.expected_name_entries {
            return Err(ExFatEntrySetError::IncompleteEntrySet {
                expected: self.expected_name_entries,
                observed: self.name_entries,
            });
        }
        if self.checksum != self.expected_checksum {
            return Err(ExFatEntrySetError::ChecksumMismatch {
                expected: self.expected_checksum,
                observed: self.checksum,
            });
        }
        if self.valid_data_length > self.data_length {
            return Err(ExFatEntrySetError::ValidDataBeyondLength);
        }
        if self.data_length == 0 {
            if self.first_cluster != 0 || self.no_fat_chain {
                return Err(ExFatEntrySetError::InvalidAllocation);
            }
        } else if self.first_cluster < 2 {
            return Err(ExFatEntrySetError::InvalidAllocation);
        }

        let kind = if self.file_attributes & EXFAT_DIRECTORY_ATTRIBUTE != 0 {
            if self.data_length > EXFAT_DIRECTORY_MAX_BYTES {
                return Err(ExFatEntrySetError::DirectoryTooLarge(self.data_length));
            }
            EntryKind::Directory
        } else {
            EntryKind::File
        };

        let name_len = encode_utf16_name(
            &self.name_units[..usize::from(self.name_length)],
            name_storage,
        )?;

        Ok(ExFatParsedFileEntry {
            kind,
            first_cluster: self.first_cluster,
            data_length: self.data_length,
            valid_data_length: self.valid_data_length,
            no_fat_chain: self.no_fat_chain,
            file_attributes: self.file_attributes,
            name_len: name_len as u16,
        })
    }

    fn parse_stream_extension(&mut self, entry: &[u8; 32]) -> Result<(), ExFatEntrySetError> {
        if entry[0] != EXFAT_STREAM_EXTENSION_ENTRY {
            return Err(ExFatEntrySetError::MissingStreamExtension);
        }

        let flags = entry[1];
        if flags & EXFAT_ALLOCATION_POSSIBLE == 0 || flags & !0x03 != 0 {
            return Err(ExFatEntrySetError::InvalidStreamExtension);
        }

        let name_length = entry[3];
        if name_length == 0 {
            return Err(ExFatEntrySetError::InvalidNameLength(name_length));
        }
        let expected_name_entries = usize::from(name_length).div_ceil(EXFAT_NAME_UNITS_PER_ENTRY);
        let expected_name_entries = u8::try_from(expected_name_entries)
            .map_err(|_| ExFatEntrySetError::InvalidNameLength(name_length))?;
        if usize::from(name_length) > EXFAT_NAME_MAX_UNITS
            || 1usize + usize::from(expected_name_entries) > usize::from(self.secondary_count)
        {
            return Err(ExFatEntrySetError::InvalidNameLength(name_length));
        }

        self.stream_seen = true;
        self.name_length = name_length;
        self.expected_name_entries = expected_name_entries;
        self.valid_data_length = read_u64(entry, 8);
        self.first_cluster = read_u32(entry, 20);
        self.data_length = read_u64(entry, 24);
        self.no_fat_chain = flags & EXFAT_NO_FAT_CHAIN != 0;
        Ok(())
    }

    fn parse_file_name(&mut self, entry: &[u8; 32]) -> Result<(), ExFatEntrySetError> {
        if entry[0] != EXFAT_FILE_NAME_ENTRY {
            return Err(ExFatEntrySetError::InvalidFileNameEntry(entry[0]));
        }

        let start = usize::from(self.name_entries) * EXFAT_NAME_UNITS_PER_ENTRY;
        let remaining = usize::from(self.name_length).saturating_sub(start);
        let take = remaining.min(EXFAT_NAME_UNITS_PER_ENTRY);

        for index in 0..take {
            let offset = 2 + index * 2;
            self.name_units[start + index] = u16::from_le_bytes([entry[offset], entry[offset + 1]]);
        }
        self.name_entries = self.name_entries.saturating_add(1);
        Ok(())
    }
}

fn checksum_primary(primary: &[u8; 32]) -> u16 {
    let mut checksum = 0u16;
    for (index, byte) in primary.iter().copied().enumerate() {
        if matches!(index, 2 | 3) {
            continue;
        }
        checksum = checksum.rotate_right(1).wrapping_add(u16::from(byte));
    }
    checksum
}

fn checksum_bytes(mut checksum: u16, bytes: &[u8]) -> u16 {
    for byte in bytes.iter().copied() {
        checksum = checksum.rotate_right(1).wrapping_add(u16::from(byte));
    }
    checksum
}

fn encode_utf16_name(
    units: &[u16],
    output: &mut [u8; FS_NAME_MAX],
) -> Result<usize, ExFatEntrySetError> {
    let mut written = 0usize;
    for decoded in char::decode_utf16(units.iter().copied()) {
        let character = decoded.map_err(|_| ExFatEntrySetError::InvalidUtf16)?;
        let mut encoded = [0u8; 4];
        let utf8 = character.encode_utf8(&mut encoded).as_bytes();
        let end = written
            .checked_add(utf8.len())
            .ok_or(ExFatEntrySetError::NameTooLong)?;
        if end > output.len() {
            return Err(ExFatEntrySetError::NameTooLong);
        }
        output[written..end].copy_from_slice(utf8);
        written = end;
    }
    Ok(written)
}

fn read_u32(bytes: &[u8; 32], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64(bytes: &[u8; 32], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_u64(entry: &mut [u8; 32], offset: usize, value: u64) {
        entry[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn file_entry_set(
        name: &[u16],
        directory: bool,
        no_fat_chain: bool,
    ) -> std::vec::Vec<[u8; 32]> {
        let name_entries = name.len().div_ceil(EXFAT_NAME_UNITS_PER_ENTRY);
        let secondary_count = 1 + name_entries;
        let mut entries = std::vec::Vec::with_capacity(1 + secondary_count);

        let mut primary = [0u8; 32];
        primary[0] = EXFAT_FILE_ENTRY;
        primary[1] = secondary_count as u8;
        if directory {
            primary[4..6].copy_from_slice(&EXFAT_DIRECTORY_ATTRIBUTE.to_le_bytes());
        }
        entries.push(primary);

        let mut stream = [0u8; 32];
        stream[0] = EXFAT_STREAM_EXTENSION_ENTRY;
        stream[1] = EXFAT_ALLOCATION_POSSIBLE | if no_fat_chain { EXFAT_NO_FAT_CHAIN } else { 0 };
        stream[3] = name.len() as u8;
        write_u64(&mut stream, 8, 4_096);
        stream[20..24].copy_from_slice(&7u32.to_le_bytes());
        write_u64(&mut stream, 24, 4_096);
        entries.push(stream);

        for chunk in name.chunks(EXFAT_NAME_UNITS_PER_ENTRY) {
            let mut name_entry = [0u8; 32];
            name_entry[0] = EXFAT_FILE_NAME_ENTRY;
            for (index, unit) in chunk.iter().copied().enumerate() {
                let offset = 2 + index * 2;
                name_entry[offset..offset + 2].copy_from_slice(&unit.to_le_bytes());
            }
            entries.push(name_entry);
        }

        let mut checksum = checksum_primary(&entries[0]);
        for entry in &entries[1..] {
            checksum = checksum_bytes(checksum, entry);
        }
        entries[0][2..4].copy_from_slice(&checksum.to_le_bytes());
        entries
    }

    fn parse(
        entries: &[[u8; 32]],
    ) -> Result<(ExFatParsedFileEntry, [u8; FS_NAME_MAX]), ExFatEntrySetError> {
        let mut parser = ExFatFileEntrySetParser::begin(&entries[0])?;
        for entry in &entries[1..] {
            parser.push_secondary(entry)?;
        }
        let mut storage = [0u8; FS_NAME_MAX];
        let parsed = parser.finish(&mut storage)?;
        Ok((parsed, storage))
    }

    #[test]
    fn parses_multi_entry_utf16_name_and_contiguous_stream() {
        let name: std::vec::Vec<u16> = "PIONEER rekordbox export database".encode_utf16().collect();
        let entries = file_entry_set(&name, false, true);
        let (parsed, storage) = parse(&entries).unwrap();

        assert_eq!(parsed.kind, EntryKind::File);
        assert_eq!(parsed.first_cluster, 7);
        assert_eq!(parsed.data_length, 4_096);
        assert_eq!(parsed.valid_data_length, 4_096);
        assert!(parsed.no_fat_chain);
        assert_eq!(
            parsed.name(&storage),
            Some("PIONEER rekordbox export database".as_bytes())
        );
    }

    #[test]
    fn preserves_directory_kind_and_fat_chain_flag() {
        let name: std::vec::Vec<u16> = "PIONEER".encode_utf16().collect();
        let entries = file_entry_set(&name, true, false);
        let (parsed, storage) = parse(&entries).unwrap();

        assert_eq!(parsed.kind, EntryKind::Directory);
        assert!(!parsed.no_fat_chain);
        assert_eq!(parsed.name(&storage), Some(b"PIONEER".as_slice()));
    }

    #[test]
    fn rejects_entry_set_checksum_corruption() {
        let name: std::vec::Vec<u16> = "track.wav".encode_utf16().collect();
        let mut entries = file_entry_set(&name, false, true);
        entries[2][6] ^= 0x01;

        assert!(matches!(
            parse(&entries),
            Err(ExFatEntrySetError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn rejects_missing_stream_extension_before_name_entries() {
        let mut primary = [0u8; 32];
        primary[0] = EXFAT_FILE_ENTRY;
        primary[1] = 2;
        let mut parser = ExFatFileEntrySetParser::begin(&primary).unwrap();
        let mut wrong = [0u8; 32];
        wrong[0] = EXFAT_FILE_NAME_ENTRY;

        assert_eq!(
            parser.push_secondary(&wrong),
            Err(ExFatEntrySetError::MissingStreamExtension)
        );
    }

    #[test]
    fn rejects_invalid_utf16_without_replacement_characters() {
        let mut entries = file_entry_set(&[0xd800], false, true);
        let mut checksum = checksum_primary(&entries[0]);
        for entry in &entries[1..] {
            checksum = checksum_bytes(checksum, entry);
        }
        entries[0][2..4].copy_from_slice(&checksum.to_le_bytes());

        assert_eq!(parse(&entries), Err(ExFatEntrySetError::InvalidUtf16));
    }

    #[test]
    fn accepts_benign_secondary_after_required_file_name_entries() {
        let name: std::vec::Vec<u16> = "track.wav".encode_utf16().collect();
        let mut entries = file_entry_set(&name, false, true);
        entries[0][1] += 1;
        let mut vendor = [0u8; 32];
        vendor[0] = EXFAT_ENTRY_IN_USE | EXFAT_ENTRY_SECONDARY | EXFAT_ENTRY_BENIGN;
        entries.push(vendor);

        let mut checksum = checksum_primary(&entries[0]);
        for entry in &entries[1..] {
            checksum = checksum_bytes(checksum, entry);
        }
        entries[0][2..4].copy_from_slice(&checksum.to_le_bytes());

        assert!(parse(&entries).is_ok());
    }
}
