use crate::{
    HOT_CUE_RECORD_V4_SIZE, HOT_CUE_STORAGE_KEY_LEN, HotCueRecordBackend, HotCueStorageKey,
};

pub const HOT_CUE_JOURNAL_SECTOR_SIZE: u32 = 4096;
pub const HOT_CUE_JOURNAL_BUCKET_COUNT: u32 = 128;
pub const HOT_CUE_JOURNAL_SECTORS_PER_BUCKET: u32 = 2;
pub const HOT_CUE_JOURNAL_PARTITION_SIZE: u32 =
    HOT_CUE_JOURNAL_SECTOR_SIZE * HOT_CUE_JOURNAL_BUCKET_COUNT * HOT_CUE_JOURNAL_SECTORS_PER_BUCKET;

/// Explicit board-qualified layout. No capacity, partition-table location or
/// writable journal address is selected by the portable journal implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JournalFlashLayout {
    pub flash_bytes: u32,
    pub partition_table_offset: u32,
    pub journal_offset: u32,
}
impl JournalFlashLayout {
    pub fn is_valid(self) -> bool {
        let Some(table_end) = self.partition_table_offset.checked_add(96 * 32) else {
            return false;
        };
        let Some(journal_end) = self
            .journal_offset
            .checked_add(HOT_CUE_JOURNAL_PARTITION_SIZE)
        else {
            return false;
        };
        self.partition_table_offset != 0
            && self.journal_offset != 0
            && self
                .partition_table_offset
                .is_multiple_of(HOT_CUE_JOURNAL_SECTOR_SIZE)
            && self
                .journal_offset
                .is_multiple_of(HOT_CUE_JOURNAL_SECTOR_SIZE)
            && table_end <= self.flash_bytes
            && journal_end <= self.flash_bytes
            && (table_end <= self.journal_offset || journal_end <= self.partition_table_offset)
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    #[test]
    fn explicit_layout_rejects_overflow_overlap_and_unaligned_or_out_of_range_storage() {
        let valid = JournalFlashLayout {
            flash_bytes: 8 * 1024 * 1024,
            partition_table_offset: 0x9000,
            journal_offset: 0x600000,
        };
        assert!(valid.is_valid());
        for invalid in [
            JournalFlashLayout {
                journal_offset: u32::MAX,
                ..valid
            },
            JournalFlashLayout {
                journal_offset: 0x9000,
                ..valid
            },
            JournalFlashLayout {
                partition_table_offset: u32::MAX,
                ..valid
            },
            JournalFlashLayout {
                journal_offset: 0,
                ..valid
            },
            JournalFlashLayout {
                partition_table_offset: 1,
                ..valid
            },
            JournalFlashLayout {
                journal_offset: valid.flash_bytes,
                ..valid
            },
        ] {
            assert!(!invalid.is_valid(), "{invalid:?}");
        }
    }
}

const SECTOR_HEADER_SIZE: u32 = 32;
const ENTRY_SIZE: u32 = 168;
const ENTRIES_PER_SECTOR: usize =
    ((HOT_CUE_JOURNAL_SECTOR_SIZE - SECTOR_HEADER_SIZE) / ENTRY_SIZE) as usize;

const HEADER_MAGIC: u32 = 0x314a_4348; // "HCJ1"
const HEADER_VERSION: u16 = 1;
const HEADER_CRC_OFFSET: usize = 24;
const HEADER_COMMIT_OFFSET: usize = 28;
const HEADER_COMMIT: u32 = 0xc011_17ed;

const ENTRY_MAGIC: u32 = 0x3145_4348; // "HCE1"
const ENTRY_VERSION: u8 = 1;
const ENTRY_KIND_PUT: u8 = 1;
const ENTRY_KIND_DELETE: u8 = 2;
const ENTRY_KEY_OFFSET: usize = 8;
const ENTRY_KEY_END: usize = ENTRY_KEY_OFFSET + HOT_CUE_STORAGE_KEY_LEN;
const ENTRY_PAYLOAD_LEN_OFFSET: usize = 24;
const ENTRY_PAYLOAD_OFFSET: usize = 28;
const ENTRY_CRC_OFFSET: usize = 160;
const ENTRY_COMMIT_OFFSET: usize = 164;
const ENTRY_COMMIT: u32 = 0xe17c_0eed;

pub trait HotCueJournalFlash {
    type Error;

    fn read(&mut self, offset: u32, output: &mut [u8]) -> Result<(), Self::Error>;
    fn write_nor(&mut self, offset: u32, data: &[u8]) -> Result<(), Self::Error>;
    fn erase_sector(&mut self, offset: u32) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotCueJournalError<E> {
    Flash(E),
    Corrupt,
    BucketFull,
    OutOfBounds,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HeaderState {
    Erased,
    Uncommitted,
    Committed { generation: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EntryKind {
    Put,
    Delete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EntryState {
    Erased,
    Torn,
    Valid {
        kind: EntryKind,
        key: [u8; HOT_CUE_STORAGE_KEY_LEN],
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActiveSector {
    side: u32,
    generation: u32,
}

pub struct HotCueJournalBackend<F> {
    flash: F,
}

impl<F> HotCueJournalBackend<F> {
    pub const fn new(flash: F) -> Self {
        Self { flash }
    }

    pub const fn flash(&self) -> &F {
        &self.flash
    }

    pub fn flash_mut(&mut self) -> &mut F {
        &mut self.flash
    }

    pub fn into_flash(self) -> F {
        self.flash
    }
}

impl<F> HotCueRecordBackend for HotCueJournalBackend<F>
where
    F: HotCueJournalFlash,
{
    type Error = HotCueJournalError<F::Error>;

    fn read_record(
        &mut self,
        key: HotCueStorageKey,
        output: &mut [u8; HOT_CUE_RECORD_V4_SIZE],
    ) -> Result<bool, Self::Error> {
        let bucket = bucket_for(key);
        let Some(active) = self.active_sector(bucket)? else {
            return Ok(false);
        };

        let base = sector_base(bucket, active.side)?;
        let mut latest = None;
        for slot in 0..ENTRIES_PER_SECTOR {
            let offset = entry_offset(base, slot)?;
            match self.read_entry_state(offset)? {
                EntryState::Erased | EntryState::Torn => {}
                EntryState::Valid {
                    kind,
                    key: entry_key,
                } if entry_key == *key.as_bytes() => latest = Some((slot, kind)),
                EntryState::Valid { .. } => {}
            }
        }

        let Some((slot, kind)) = latest else {
            return Ok(false);
        };
        if kind == EntryKind::Delete {
            return Ok(false);
        }

        let offset = entry_offset(base, slot)?;
        let mut raw = [0u8; ENTRY_SIZE as usize];
        self.read(offset, &mut raw)?;
        output.copy_from_slice(
            &raw[ENTRY_PAYLOAD_OFFSET..ENTRY_PAYLOAD_OFFSET + HOT_CUE_RECORD_V4_SIZE],
        );
        Ok(true)
    }

    fn write_record_atomic(
        &mut self,
        key: HotCueStorageKey,
        record: &[u8; HOT_CUE_RECORD_V4_SIZE],
    ) -> Result<(), Self::Error> {
        self.append_operation(key, Some(record))
    }

    fn remove_record(&mut self, key: HotCueStorageKey) -> Result<(), Self::Error> {
        let bucket = bucket_for(key);
        if self.active_sector(bucket)?.is_none() {
            return Ok(());
        }
        self.append_operation(key, None)
    }
}

impl<F> HotCueJournalBackend<F>
where
    F: HotCueJournalFlash,
{
    fn append_operation(
        &mut self,
        key: HotCueStorageKey,
        record: Option<&[u8; HOT_CUE_RECORD_V4_SIZE]>,
    ) -> Result<(), HotCueJournalError<F::Error>> {
        let bucket = bucket_for(key);
        let active = self.active_sector(bucket)?;

        let Some(active) = active else {
            if record.is_none() {
                return Ok(());
            }
            let base = sector_base(bucket, 0)?;
            self.flash
                .erase_sector(base)
                .map_err(HotCueJournalError::Flash)?;
            self.write_header_uncommitted(base, bucket, 1)?;
            self.write_entry(base, 0, key, record)?;
            self.commit_header(base)?;
            return Ok(());
        };

        let active_base = sector_base(bucket, active.side)?;
        if let Some(slot) = self.first_erased_slot(active_base)? {
            self.write_entry(active_base, slot, key, record)?;
            return Ok(());
        }

        self.rotate(bucket, active, key, record)
    }

    fn rotate(
        &mut self,
        bucket: u32,
        active: ActiveSector,
        incoming_key: HotCueStorageKey,
        incoming_record: Option<&[u8; HOT_CUE_RECORD_V4_SIZE]>,
    ) -> Result<(), HotCueJournalError<F::Error>> {
        let old_base = sector_base(bucket, active.side)?;
        let new_side = active.side ^ 1;
        let new_base = sector_base(bucket, new_side)?;

        let mut selected = [None; ENTRIES_PER_SECTOR];
        let mut selected_count = 0usize;
        let mut seen = [[0u8; HOT_CUE_STORAGE_KEY_LEN]; ENTRIES_PER_SECTOR];
        let mut seen_count = 0usize;

        for slot in (0..ENTRIES_PER_SECTOR).rev() {
            let offset = entry_offset(old_base, slot)?;
            let state = self.read_entry_state(offset)?;
            let EntryState::Valid {
                kind,
                key: entry_key,
            } = state
            else {
                continue;
            };

            if entry_key == *incoming_key.as_bytes() || seen[..seen_count].contains(&entry_key) {
                continue;
            }

            seen[seen_count] = entry_key;
            seen_count += 1;

            if kind == EntryKind::Put {
                selected[selected_count] = Some(slot);
                selected_count += 1;
            }
        }

        let incoming_slots = usize::from(incoming_record.is_some());
        if selected_count + incoming_slots > ENTRIES_PER_SECTOR {
            return Err(HotCueJournalError::BucketFull);
        }

        self.flash
            .erase_sector(new_base)
            .map_err(HotCueJournalError::Flash)?;
        self.write_header_uncommitted(new_base, bucket, active.generation.wrapping_add(1))?;

        let mut next_slot = 0usize;
        for old_slot in selected[..selected_count].iter().flatten().rev() {
            let old_offset = entry_offset(old_base, *old_slot)?;
            let mut raw = [0u8; ENTRY_SIZE as usize];
            self.read(old_offset, &mut raw)?;
            self.write_raw_entry(new_base, next_slot, &raw)?;
            next_slot += 1;
        }

        if incoming_record.is_some() {
            self.write_entry(new_base, next_slot, incoming_key, incoming_record)?;
        }

        // The generation does not become authoritative until every compacted
        // entry and the incoming operation are durably committed.
        self.commit_header(new_base)?;
        Ok(())
    }

    fn active_sector(
        &mut self,
        bucket: u32,
    ) -> Result<Option<ActiveSector>, HotCueJournalError<F::Error>> {
        let left = self.read_header(sector_base(bucket, 0)?, bucket)?;
        let right = self.read_header(sector_base(bucket, 1)?, bucket)?;

        let left = match left {
            HeaderState::Committed { generation } => Some(ActiveSector {
                side: 0,
                generation,
            }),
            HeaderState::Erased | HeaderState::Uncommitted => None,
        };
        let right = match right {
            HeaderState::Committed { generation } => Some(ActiveSector {
                side: 1,
                generation,
            }),
            HeaderState::Erased | HeaderState::Uncommitted => None,
        };

        match (left, right) {
            (None, None) => Ok(None),
            (Some(active), None) | (None, Some(active)) => Ok(Some(active)),
            (Some(left), Some(right)) => {
                if left.generation == right.generation {
                    return Err(HotCueJournalError::Corrupt);
                }
                if generation_newer(right.generation, left.generation) {
                    Ok(Some(right))
                } else {
                    Ok(Some(left))
                }
            }
        }
    }

    fn read_header(
        &mut self,
        base: u32,
        bucket: u32,
    ) -> Result<HeaderState, HotCueJournalError<F::Error>> {
        let mut raw = [0u8; SECTOR_HEADER_SIZE as usize];
        self.read(base, &mut raw)?;
        if is_erased(&raw) {
            return Ok(HeaderState::Erased);
        }

        if get_u32(&raw[HEADER_COMMIT_OFFSET..HEADER_COMMIT_OFFSET + 4]) != HEADER_COMMIT {
            return Ok(HeaderState::Uncommitted);
        }

        if get_u32(&raw[0..4]) != HEADER_MAGIC
            || get_u16(&raw[4..6]) != HEADER_VERSION
            || get_u16(&raw[6..8]) as u32 != bucket
            || raw[12..HEADER_CRC_OFFSET].iter().any(|byte| *byte != 0)
        {
            return Err(HotCueJournalError::Corrupt);
        }
        let expected_crc = get_u32(&raw[HEADER_CRC_OFFSET..HEADER_CRC_OFFSET + 4]);
        if expected_crc != crc32_iso(&raw[..HEADER_CRC_OFFSET]) {
            return Err(HotCueJournalError::Corrupt);
        }

        Ok(HeaderState::Committed {
            generation: get_u32(&raw[8..12]),
        })
    }

    fn write_header_uncommitted(
        &mut self,
        base: u32,
        bucket: u32,
        generation: u32,
    ) -> Result<(), HotCueJournalError<F::Error>> {
        let mut raw = [0u8; HEADER_COMMIT_OFFSET];
        put_u32(&mut raw[0..4], HEADER_MAGIC);
        put_u16(&mut raw[4..6], HEADER_VERSION);
        put_u16(&mut raw[6..8], bucket as u16);
        put_u32(&mut raw[8..12], generation);
        let crc = crc32_iso(&raw[..HEADER_CRC_OFFSET]);
        put_u32(&mut raw[HEADER_CRC_OFFSET..HEADER_CRC_OFFSET + 4], crc);
        self.flash
            .write_nor(base, &raw)
            .map_err(HotCueJournalError::Flash)
    }

    fn commit_header(&mut self, base: u32) -> Result<(), HotCueJournalError<F::Error>> {
        self.flash
            .write_nor(
                base + HEADER_COMMIT_OFFSET as u32,
                &HEADER_COMMIT.to_le_bytes(),
            )
            .map_err(HotCueJournalError::Flash)
    }

    fn first_erased_slot(
        &mut self,
        base: u32,
    ) -> Result<Option<usize>, HotCueJournalError<F::Error>> {
        for slot in 0..ENTRIES_PER_SECTOR {
            let offset = entry_offset(base, slot)?;
            if self.read_entry_state(offset)? == EntryState::Erased {
                return Ok(Some(slot));
            }
        }
        Ok(None)
    }

    fn read_entry_state(
        &mut self,
        offset: u32,
    ) -> Result<EntryState, HotCueJournalError<F::Error>> {
        let mut raw = [0u8; ENTRY_SIZE as usize];
        self.read(offset, &mut raw)?;
        if is_erased(&raw) {
            return Ok(EntryState::Erased);
        }

        if get_u32(&raw[ENTRY_COMMIT_OFFSET..ENTRY_COMMIT_OFFSET + 4]) != ENTRY_COMMIT {
            return Ok(EntryState::Torn);
        }

        if get_u32(&raw[0..4]) != ENTRY_MAGIC
            || raw[4] != ENTRY_VERSION
            || raw[6] != 0
            || raw[7] != 0
            || raw[ENTRY_KEY_END] != 0
        {
            return Err(HotCueJournalError::Corrupt);
        }

        let kind = match raw[5] {
            ENTRY_KIND_PUT
                if get_u32(&raw[ENTRY_PAYLOAD_LEN_OFFSET..ENTRY_PAYLOAD_LEN_OFFSET + 4])
                    == HOT_CUE_RECORD_V4_SIZE as u32 =>
            {
                EntryKind::Put
            }
            ENTRY_KIND_DELETE
                if get_u32(&raw[ENTRY_PAYLOAD_LEN_OFFSET..ENTRY_PAYLOAD_LEN_OFFSET + 4]) == 0
                    && raw[ENTRY_PAYLOAD_OFFSET..ENTRY_CRC_OFFSET]
                        .iter()
                        .all(|byte| *byte == 0) =>
            {
                EntryKind::Delete
            }
            _ => return Err(HotCueJournalError::Corrupt),
        };

        let expected_crc = get_u32(&raw[ENTRY_CRC_OFFSET..ENTRY_CRC_OFFSET + 4]);
        if expected_crc != crc32_iso(&raw[..ENTRY_CRC_OFFSET]) {
            return Err(HotCueJournalError::Corrupt);
        }

        let mut key = [0u8; HOT_CUE_STORAGE_KEY_LEN];
        key.copy_from_slice(&raw[ENTRY_KEY_OFFSET..ENTRY_KEY_END]);
        Ok(EntryState::Valid { kind, key })
    }

    fn write_entry(
        &mut self,
        base: u32,
        slot: usize,
        key: HotCueStorageKey,
        record: Option<&[u8; HOT_CUE_RECORD_V4_SIZE]>,
    ) -> Result<(), HotCueJournalError<F::Error>> {
        let mut raw = [0u8; ENTRY_SIZE as usize];
        put_u32(&mut raw[0..4], ENTRY_MAGIC);
        raw[4] = ENTRY_VERSION;
        raw[5] = if record.is_some() {
            ENTRY_KIND_PUT
        } else {
            ENTRY_KIND_DELETE
        };
        raw[ENTRY_KEY_OFFSET..ENTRY_KEY_END].copy_from_slice(key.as_bytes());
        if let Some(record) = record {
            put_u32(
                &mut raw[ENTRY_PAYLOAD_LEN_OFFSET..ENTRY_PAYLOAD_LEN_OFFSET + 4],
                HOT_CUE_RECORD_V4_SIZE as u32,
            );
            raw[ENTRY_PAYLOAD_OFFSET..ENTRY_PAYLOAD_OFFSET + HOT_CUE_RECORD_V4_SIZE]
                .copy_from_slice(record);
        }
        let crc = crc32_iso(&raw[..ENTRY_CRC_OFFSET]);
        put_u32(&mut raw[ENTRY_CRC_OFFSET..ENTRY_CRC_OFFSET + 4], crc);
        put_u32(
            &mut raw[ENTRY_COMMIT_OFFSET..ENTRY_COMMIT_OFFSET + 4],
            ENTRY_COMMIT,
        );
        self.write_raw_entry(base, slot, &raw)
    }

    fn write_raw_entry(
        &mut self,
        base: u32,
        slot: usize,
        raw: &[u8; ENTRY_SIZE as usize],
    ) -> Result<(), HotCueJournalError<F::Error>> {
        let offset = entry_offset(base, slot)?;
        self.flash
            .write_nor(offset, &raw[..ENTRY_COMMIT_OFFSET])
            .map_err(HotCueJournalError::Flash)?;
        self.flash
            .write_nor(
                offset + ENTRY_COMMIT_OFFSET as u32,
                &raw[ENTRY_COMMIT_OFFSET..ENTRY_COMMIT_OFFSET + 4],
            )
            .map_err(HotCueJournalError::Flash)
    }

    fn read(&mut self, offset: u32, output: &mut [u8]) -> Result<(), HotCueJournalError<F::Error>> {
        let end = offset
            .checked_add(output.len() as u32)
            .ok_or(HotCueJournalError::OutOfBounds)?;
        if end > HOT_CUE_JOURNAL_PARTITION_SIZE {
            return Err(HotCueJournalError::OutOfBounds);
        }
        self.flash
            .read(offset, output)
            .map_err(HotCueJournalError::Flash)
    }
}

fn bucket_for(key: HotCueStorageKey) -> u32 {
    crc32_iso(key.as_bytes()) & (HOT_CUE_JOURNAL_BUCKET_COUNT - 1)
}

fn sector_base<E>(bucket: u32, side: u32) -> Result<u32, HotCueJournalError<E>> {
    if bucket >= HOT_CUE_JOURNAL_BUCKET_COUNT || side >= HOT_CUE_JOURNAL_SECTORS_PER_BUCKET {
        return Err(HotCueJournalError::OutOfBounds);
    }
    Ok((bucket * HOT_CUE_JOURNAL_SECTORS_PER_BUCKET + side) * HOT_CUE_JOURNAL_SECTOR_SIZE)
}

fn entry_offset<E>(base: u32, slot: usize) -> Result<u32, HotCueJournalError<E>> {
    if slot >= ENTRIES_PER_SECTOR {
        return Err(HotCueJournalError::OutOfBounds);
    }
    Ok(base + SECTOR_HEADER_SIZE + slot as u32 * ENTRY_SIZE)
}

fn generation_newer(candidate: u32, current: u32) -> bool {
    candidate != current && candidate.wrapping_sub(current) < 0x8000_0000
}

fn is_erased(bytes: &[u8]) -> bool {
    bytes.iter().all(|byte| *byte == 0xff)
}

fn put_u16(output: &mut [u8], value: u16) {
    output.copy_from_slice(&value.to_le_bytes());
}

fn put_u32(output: &mut [u8], value: u32) {
    output.copy_from_slice(&value.to_le_bytes());
}

fn get_u16(input: &[u8]) -> u16 {
    u16::from_le_bytes([input[0], input[1]])
}

fn get_u32(input: &[u8]) -> u32 {
    u32::from_le_bytes([input[0], input[1], input[2], input[3]])
}

fn crc32_iso(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::vec;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum FakeError {
        NotNor,
        Injected,
        OutOfBounds,
    }

    struct FakeFlash {
        bytes: std::vec::Vec<u8>,
        partial_next_write: Option<usize>,
    }

    impl FakeFlash {
        fn new() -> Self {
            Self {
                bytes: vec![0xff; HOT_CUE_JOURNAL_PARTITION_SIZE as usize],
                partial_next_write: None,
            }
        }

        fn cut_next_write_after(&mut self, bytes: usize) {
            self.partial_next_write = Some(bytes);
        }
    }

    impl HotCueJournalFlash for FakeFlash {
        type Error = FakeError;

        fn read(&mut self, offset: u32, output: &mut [u8]) -> Result<(), Self::Error> {
            let start = offset as usize;
            let end = start
                .checked_add(output.len())
                .ok_or(FakeError::OutOfBounds)?;
            let source = self.bytes.get(start..end).ok_or(FakeError::OutOfBounds)?;
            output.copy_from_slice(source);
            Ok(())
        }

        fn write_nor(&mut self, offset: u32, data: &[u8]) -> Result<(), Self::Error> {
            let start = offset as usize;
            let end = start
                .checked_add(data.len())
                .ok_or(FakeError::OutOfBounds)?;
            let target = self
                .bytes
                .get_mut(start..end)
                .ok_or(FakeError::OutOfBounds)?;

            let write_len = self
                .partial_next_write
                .take()
                .unwrap_or(data.len())
                .min(data.len());
            for (old, new) in target[..write_len].iter_mut().zip(&data[..write_len]) {
                if *old & *new != *new {
                    return Err(FakeError::NotNor);
                }
                *old = *new;
            }
            if write_len != data.len() {
                return Err(FakeError::Injected);
            }
            Ok(())
        }

        fn erase_sector(&mut self, offset: u32) -> Result<(), Self::Error> {
            if !offset.is_multiple_of(HOT_CUE_JOURNAL_SECTOR_SIZE) {
                return Err(FakeError::OutOfBounds);
            }
            let start = offset as usize;
            let end = start + HOT_CUE_JOURNAL_SECTOR_SIZE as usize;
            let target = self
                .bytes
                .get_mut(start..end)
                .ok_or(FakeError::OutOfBounds)?;
            target.fill(0xff);
            Ok(())
        }
    }

    fn key(seed: u8) -> HotCueStorageKey {
        let mut id = [0u8; 16];
        for (index, byte) in id.iter_mut().enumerate() {
            *byte = seed.wrapping_add(index as u8);
        }
        HotCueStorageKey::from_track_id(pajoniiir_core::MediaTrackId(id))
    }

    fn record(seed: u8) -> [u8; HOT_CUE_RECORD_V4_SIZE] {
        let mut record = [0u8; HOT_CUE_RECORD_V4_SIZE];
        for (index, byte) in record.iter_mut().enumerate() {
            *byte = seed.wrapping_add(index as u8);
        }
        record
    }

    #[test]
    fn journal_partition_geometry_is_exactly_one_mibibyte() {
        assert_eq!(ENTRY_SIZE as usize, 168);
        assert_eq!(ENTRIES_PER_SECTOR, 24);
        assert_eq!(HOT_CUE_JOURNAL_PARTITION_SIZE, 1024 * 1024);
    }

    #[test]
    fn journal_put_read_update_and_delete_roundtrip() {
        let flash = FakeFlash::new();
        let mut journal = HotCueJournalBackend::new(flash);
        let key = key(1);
        let first = record(0x10);
        let second = record(0x20);
        let mut output = [0u8; HOT_CUE_RECORD_V4_SIZE];

        assert!(!journal.read_record(key, &mut output).unwrap());
        journal.write_record_atomic(key, &first).unwrap();
        assert!(journal.read_record(key, &mut output).unwrap());
        assert_eq!(output, first);

        journal.write_record_atomic(key, &second).unwrap();
        assert!(journal.read_record(key, &mut output).unwrap());
        assert_eq!(output, second);

        journal.remove_record(key).unwrap();
        assert!(!journal.read_record(key, &mut output).unwrap());
    }

    #[test]
    fn torn_append_preserves_previous_committed_record() {
        let flash = FakeFlash::new();
        let mut journal = HotCueJournalBackend::new(flash);
        let key = key(2);
        let first = record(0x31);
        let second = record(0x42);
        journal.write_record_atomic(key, &first).unwrap();

        journal.flash_mut().cut_next_write_after(40);
        assert_eq!(
            journal.write_record_atomic(key, &second),
            Err(HotCueJournalError::Flash(FakeError::Injected))
        );

        let flash = journal.into_flash();
        let mut rebooted = HotCueJournalBackend::new(flash);
        let mut output = [0u8; HOT_CUE_RECORD_V4_SIZE];
        assert!(rebooted.read_record(key, &mut output).unwrap());
        assert_eq!(output, first);
    }

    #[test]
    fn full_sector_rotates_and_keeps_latest_value() {
        let flash = FakeFlash::new();
        let mut journal = HotCueJournalBackend::new(flash);
        let key = key(3);

        for seed in 0..=ENTRIES_PER_SECTOR as u8 {
            journal.write_record_atomic(key, &record(seed)).unwrap();
        }

        let mut output = [0u8; HOT_CUE_RECORD_V4_SIZE];
        assert!(journal.read_record(key, &mut output).unwrap());
        assert_eq!(output, record(ENTRIES_PER_SECTOR as u8));
    }

    #[test]
    fn torn_rotation_header_never_supersedes_old_generation() {
        let flash = FakeFlash::new();
        let mut journal = HotCueJournalBackend::new(flash);
        let key = key(4);

        for seed in 0..ENTRIES_PER_SECTOR as u8 {
            journal.write_record_atomic(key, &record(seed)).unwrap();
        }

        journal.flash_mut().cut_next_write_after(12);
        assert_eq!(
            journal.write_record_atomic(key, &record(0xee)),
            Err(HotCueJournalError::Flash(FakeError::Injected))
        );

        let flash = journal.into_flash();
        let mut rebooted = HotCueJournalBackend::new(flash);
        let mut output = [0u8; HOT_CUE_RECORD_V4_SIZE];
        assert!(rebooted.read_record(key, &mut output).unwrap());
        assert_eq!(output, record(ENTRIES_PER_SECTOR as u8 - 1));
    }

    #[test]
    fn committed_entry_crc_corruption_fails_closed() {
        let flash = FakeFlash::new();
        let mut journal = HotCueJournalBackend::new(flash);
        let key = key(5);
        journal.write_record_atomic(key, &record(0x55)).unwrap();

        let bucket = bucket_for(key);
        let base = sector_base::<FakeError>(bucket, 0).unwrap();
        let payload_byte = (base + SECTOR_HEADER_SIZE + ENTRY_PAYLOAD_OFFSET as u32) as usize;
        journal.flash_mut().bytes[payload_byte] ^= 1;

        let mut output = [0u8; HOT_CUE_RECORD_V4_SIZE];
        assert_eq!(
            journal.read_record(key, &mut output),
            Err(HotCueJournalError::Corrupt)
        );
    }

    #[test]
    fn generation_comparison_is_wrap_safe() {
        assert!(generation_newer(0, u32::MAX));
        assert!(generation_newer(10, 9));
        assert!(!generation_newer(9, 10));
        assert!(!generation_newer(7, 7));
    }
}
