//! Host stand-in for the settings partition: NOR semantics in RAM (bits only
//! clear until a sector erase), so the simulator commits settings through the
//! product `SettingsOwner` record store. It lasts for one simulator process.
use ruzina_settings::store::{SETTINGS_PARTITION_SIZE, SETTINGS_SECTOR_SIZE, SettingsFlash};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RamFlashError {
    OutOfBounds,
    NotErased,
}

#[derive(Clone, Debug)]
pub struct RamFlash {
    bytes: Vec<u8>,
}

impl RamFlash {
    pub fn new() -> Self {
        Self {
            bytes: vec![0xff; SETTINGS_PARTITION_SIZE as usize],
        }
    }

    fn range(&self, offset: u32, len: usize) -> Result<core::ops::Range<usize>, RamFlashError> {
        let start = offset as usize;
        let end = start.checked_add(len).ok_or(RamFlashError::OutOfBounds)?;
        if end > self.bytes.len() {
            return Err(RamFlashError::OutOfBounds);
        }
        Ok(start..end)
    }
}

impl SettingsFlash for RamFlash {
    type Error = RamFlashError;

    fn read(&mut self, offset: u32, output: &mut [u8]) -> Result<(), RamFlashError> {
        let range = self.range(offset, output.len())?;
        output.copy_from_slice(&self.bytes[range]);
        Ok(())
    }

    fn write_nor(&mut self, offset: u32, data: &[u8]) -> Result<(), RamFlashError> {
        let range = self.range(offset, data.len())?;
        if self.bytes[range.clone()]
            .iter()
            .zip(data)
            .any(|(old, new)| old & new != *new)
        {
            return Err(RamFlashError::NotErased);
        }
        for (old, new) in self.bytes[range].iter_mut().zip(data) {
            *old &= new;
        }
        Ok(())
    }

    fn erase_sector(&mut self, offset: u32) -> Result<(), RamFlashError> {
        if !offset.is_multiple_of(SETTINGS_SECTOR_SIZE) {
            return Err(RamFlashError::OutOfBounds);
        }
        let range = self.range(offset, SETTINGS_SECTOR_SIZE as usize)?;
        self.bytes[range].fill(0xff);
        Ok(())
    }
}
