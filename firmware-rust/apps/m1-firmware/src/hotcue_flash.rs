use esp_storage::{FlashStorage, FlashStorageError};
use pajoniiir_hot_cues::journal::{
    HOT_CUE_JOURNAL_PARTITION_SIZE, HOT_CUE_JOURNAL_SECTOR_SIZE, HotCueJournalFlash,
};

pub(crate) const M1_FLASH_CAPACITY: u32 = 0x0100_0000;
pub(crate) const HOT_CUE_PARTITION_OFFSET: u32 = 0x00f0_0000;
pub(crate) const HOT_CUE_PARTITION_SIZE: u32 = HOT_CUE_JOURNAL_PARTITION_SIZE;

const PARTITION_TABLE_OFFSET: u32 = 0x8000;
const PARTITION_ENTRY_SIZE: u32 = 32;
const PARTITION_ENTRY_COUNT: u32 = 96;
const PARTITION_MAGIC: u16 = 0x50aa;
const PARTITION_MAGIC_MD5: u16 = 0xebeb;
const PARTITION_TYPE_DATA: u8 = 0x01;
const HOT_CUE_PARTITION_SUBTYPE: u8 = 0x40;
const HOT_CUE_LABEL: &[u8] = b"hotcue";
const MAX_TRANSFER_SIZE: usize = 168;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HotCueFlashError {
    Storage(FlashStorageError),
    FlashTooSmall,
    InvalidPartitionTable,
    MissingPartition,
    PartitionMismatch,
    InvalidTransfer,
}

#[repr(C, align(4))]
struct AlignedTransfer([u8; MAX_TRANSFER_SIZE]);

pub(crate) struct EspHotCueFlash {
    storage: FlashStorage<'static>,
}

impl EspHotCueFlash {
    pub(crate) fn open(
        flash: esp_hal::peripherals::FLASH<'static>,
    ) -> Result<Self, HotCueFlashError> {
        let mut storage = FlashStorage::new(flash).multicore_auto_park();
        if storage.capacity() < M1_FLASH_CAPACITY as usize {
            return Err(HotCueFlashError::FlashTooSmall);
        }
        verify_hot_cue_partition(&mut storage)?;
        Ok(Self { storage })
    }

    fn absolute_range(
        offset: u32,
        len: usize,
    ) -> Result<(u32, u32), HotCueFlashError> {
        let len = u32::try_from(len).map_err(|_| HotCueFlashError::InvalidTransfer)?;
        let end = offset
            .checked_add(len)
            .ok_or(HotCueFlashError::InvalidTransfer)?;
        if end > HOT_CUE_PARTITION_SIZE {
            return Err(HotCueFlashError::InvalidTransfer);
        }
        let start = HOT_CUE_PARTITION_OFFSET
            .checked_add(offset)
            .ok_or(HotCueFlashError::InvalidTransfer)?;
        let absolute_end = start
            .checked_add(len)
            .ok_or(HotCueFlashError::InvalidTransfer)?;
        Ok((start, absolute_end))
    }
}

impl HotCueJournalFlash for EspHotCueFlash {
    type Error = HotCueFlashError;

    fn read(&mut self, offset: u32, output: &mut [u8]) -> Result<(), Self::Error> {
        if output.len() > MAX_TRANSFER_SIZE
            || !offset.is_multiple_of(FlashStorage::READ_SIZE as u32)
            || !output.len().is_multiple_of(FlashStorage::READ_SIZE)
        {
            return Err(HotCueFlashError::InvalidTransfer);
        }
        let (absolute, _) = Self::absolute_range(offset, output.len())?;
        let mut aligned = AlignedTransfer([0u8; MAX_TRANSFER_SIZE]);
        self.storage
            .read_nor(absolute, &mut aligned.0[..output.len()])
            .map_err(HotCueFlashError::Storage)?;
        output.copy_from_slice(&aligned.0[..output.len()]);
        Ok(())
    }

    fn write_nor(&mut self, offset: u32, data: &[u8]) -> Result<(), Self::Error> {
        if data.len() > MAX_TRANSFER_SIZE
            || !offset.is_multiple_of(FlashStorage::WRITE_SIZE as u32)
            || !data.len().is_multiple_of(FlashStorage::WRITE_SIZE)
        {
            return Err(HotCueFlashError::InvalidTransfer);
        }
        let (absolute, _) = Self::absolute_range(offset, data.len())?;
        let mut aligned = AlignedTransfer([0u8; MAX_TRANSFER_SIZE]);
        aligned.0[..data.len()].copy_from_slice(data);
        self.storage
            .write_nor(absolute, &aligned.0[..data.len()])
            .map_err(HotCueFlashError::Storage)
    }

    fn erase_sector(&mut self, offset: u32) -> Result<(), Self::Error> {
        if !offset.is_multiple_of(HOT_CUE_JOURNAL_SECTOR_SIZE) {
            return Err(HotCueFlashError::InvalidTransfer);
        }
        let (absolute, end) =
            Self::absolute_range(offset, HOT_CUE_JOURNAL_SECTOR_SIZE as usize)?;
        self.storage
            .erase(absolute, end)
            .map_err(HotCueFlashError::Storage)
    }
}

fn verify_hot_cue_partition(
    storage: &mut FlashStorage<'static>,
) -> Result<(), HotCueFlashError> {
    let mut aligned = AlignedTransfer([0u8; MAX_TRANSFER_SIZE]);
    let mut found_hot_cue = false;
    let hot_cue_end = HOT_CUE_PARTITION_OFFSET + HOT_CUE_PARTITION_SIZE;

    for index in 0..PARTITION_ENTRY_COUNT {
        let table_offset = PARTITION_TABLE_OFFSET + index * PARTITION_ENTRY_SIZE;
        let entry = &mut aligned.0[..PARTITION_ENTRY_SIZE as usize];
        storage
            .read_nor(table_offset, entry)
            .map_err(HotCueFlashError::Storage)?;

        let magic = u16::from_le_bytes([entry[0], entry[1]]);
        if magic == u16::MAX || magic == PARTITION_MAGIC_MD5 {
            break;
        }
        if magic != PARTITION_MAGIC {
            return Err(HotCueFlashError::InvalidPartitionTable);
        }

        let offset = u32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]);
        let size = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]);
        let end = offset
            .checked_add(size)
            .ok_or(HotCueFlashError::InvalidPartitionTable)?;
        let flags = u32::from_le_bytes([entry[28], entry[29], entry[30], entry[31]]);
        let is_hot_cue = label_matches(&entry[12..28], HOT_CUE_LABEL);

        if is_hot_cue {
            if found_hot_cue
                || entry[2] != PARTITION_TYPE_DATA
                || entry[3] != HOT_CUE_PARTITION_SUBTYPE
                || offset != HOT_CUE_PARTITION_OFFSET
                || size != HOT_CUE_PARTITION_SIZE
                || flags != 0
            {
                return Err(HotCueFlashError::PartitionMismatch);
            }
            found_hot_cue = true;
            continue;
        }

        if offset < hot_cue_end && end > HOT_CUE_PARTITION_OFFSET {
            return Err(HotCueFlashError::PartitionMismatch);
        }
    }

    if found_hot_cue {
        Ok(())
    } else {
        Err(HotCueFlashError::MissingPartition)
    }
}

fn label_matches(field: &[u8], expected: &[u8]) -> bool {
    if field.len() != 16 || expected.len() >= field.len() {
        return false;
    }
    &field[..expected.len()] == expected
        && field[expected.len()] == 0
        && field[expected.len() + 1..].iter().all(|byte| *byte == 0)
}
