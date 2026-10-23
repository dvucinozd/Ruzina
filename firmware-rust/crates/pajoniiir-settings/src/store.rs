//! A/B record slots on NOR flash. Each record kind owns two erase sectors;
//! a write erases the slot not holding the newest committed record, writes
//! header and payload, and only then the commit word. A torn write or a
//! power cut therefore leaves the previous committed record authoritative.

use crate::crc32;

pub const SETTINGS_SECTOR_SIZE: u32 = 4096;
/// Settings slots A/B, then network slots A/B.
pub const SETTINGS_PARTITION_SIZE: u32 = 4 * SETTINGS_SECTOR_SIZE;
pub const RECORD_MAX_PAYLOAD: usize = 512;

const HEADER_BYTES: usize = 24;
const MAGIC: u32 = 0x3154_5350; // "PST1"
const COMMIT_OFFSET: usize = 20;
const COMMIT: u32 = 0x5e77_1ed0;

/// Partition-relative NOR access (bits only clear between erases).
pub trait SettingsFlash {
    type Error;
    fn read(&mut self, offset: u32, output: &mut [u8]) -> Result<(), Self::Error>;
    fn write_nor(&mut self, offset: u32, data: &[u8]) -> Result<(), Self::Error>;
    fn erase_sector(&mut self, offset: u32) -> Result<(), Self::Error>;
}

/// Board-qualified placement; nothing here picks a flash address.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SettingsFlashLayout {
    pub flash_bytes: u32,
    pub partition_table_offset: u32,
    pub settings_offset: u32,
}

impl SettingsFlashLayout {
    pub fn is_valid(self) -> bool {
        let (Some(table_end), Some(end)) = (
            self.partition_table_offset.checked_add(96 * 32),
            self.settings_offset.checked_add(SETTINGS_PARTITION_SIZE),
        ) else {
            return false;
        };
        self.partition_table_offset != 0
            && self.settings_offset != 0
            && self
                .partition_table_offset
                .is_multiple_of(SETTINGS_SECTOR_SIZE)
            && self.settings_offset.is_multiple_of(SETTINGS_SECTOR_SIZE)
            && table_end <= self.flash_bytes
            && end <= self.flash_bytes
            && (table_end <= self.settings_offset || end <= self.partition_table_offset)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordKind {
    Settings = 1,
    Network = 2,
}

impl RecordKind {
    const fn base(self) -> u32 {
        match self {
            Self::Settings => 0,
            Self::Network => 2 * SETTINGS_SECTOR_SIZE,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreError<E> {
    Flash(E),
    PayloadTooLarge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Loaded {
    /// No committed record in either slot.
    Missing,
    /// The newest committed record; its payload is in the caller's buffer.
    Record {
        version: u8,
        len: usize,
        sequence: u32,
    },
    /// The newest committed record has a format this firmware does not know.
    Newer { version: u8, sequence: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoadOutcome {
    pub loaded: Loaded,
    /// A committed slot failed its checks and was ignored (bit rot or a
    /// foreign writer); the other slot, if valid, was used.
    pub corrupt_slot: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Slot {
    Empty,
    Torn,
    Corrupt,
    Valid {
        version: u8,
        len: usize,
        sequence: u32,
    },
}

pub struct RecordStore<F> {
    flash: F,
}

impl<F: SettingsFlash> RecordStore<F> {
    pub const fn new(flash: F) -> Self {
        Self { flash }
    }

    pub fn flash_mut(&mut self) -> &mut F {
        &mut self.flash
    }

    pub fn into_flash(self) -> F {
        self.flash
    }

    /// Loads the newest committed record of `kind` whose format is at most
    /// `supported`; a newer format is reported, never decoded.
    pub fn load(
        &mut self,
        kind: RecordKind,
        supported: u8,
        payload: &mut [u8],
    ) -> Result<LoadOutcome, StoreError<F::Error>> {
        let slots = [self.slot(kind, 0)?, self.slot(kind, 1)?];
        let corrupt_slot = slots.contains(&Slot::Corrupt);
        let newest = (0..2)
            .filter_map(|side| match slots[side] {
                Slot::Valid { sequence, .. } => Some((side, sequence)),
                _ => None,
            })
            .reduce(|a, b| if newer(b.1, a.1) { b } else { a });
        let Some((side, _)) = newest else {
            return Ok(LoadOutcome {
                loaded: Loaded::Missing,
                corrupt_slot,
            });
        };
        let Slot::Valid {
            version,
            len,
            sequence,
        } = slots[side]
        else {
            unreachable!("selected a valid slot");
        };
        let loaded = if version > supported {
            Loaded::Newer { version, sequence }
        } else if len > payload.len() {
            return Err(StoreError::PayloadTooLarge);
        } else {
            self.read(
                slot_base(kind, side) + HEADER_BYTES as u32,
                &mut payload[..len],
            )?;
            Loaded::Record {
                version,
                len,
                sequence,
            }
        };
        Ok(LoadOutcome {
            loaded,
            corrupt_slot,
        })
    }

    /// Commits `payload` as the next record of `kind` and returns its
    /// sequence. The newest committed record stays intact until the commit
    /// word of the new one is written.
    pub fn write(
        &mut self,
        kind: RecordKind,
        version: u8,
        payload: &[u8],
    ) -> Result<u32, StoreError<F::Error>> {
        if payload.len() > RECORD_MAX_PAYLOAD {
            return Err(StoreError::PayloadTooLarge);
        }
        let slots = [self.slot(kind, 0)?, self.slot(kind, 1)?];
        let newest = (0..2)
            .filter_map(|side| match slots[side] {
                Slot::Valid { sequence, .. } => Some((side, sequence)),
                _ => None,
            })
            .reduce(|a, b| if newer(b.1, a.1) { b } else { a });
        let (side, sequence) = match newest {
            Some((side, sequence)) => (side ^ 1, sequence.wrapping_add(1)),
            None => (0, 1),
        };
        let base = slot_base(kind, side);
        self.flash.erase_sector(base).map_err(StoreError::Flash)?;
        let mut header = [0u8; COMMIT_OFFSET];
        header[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        header[4] = kind as u8;
        header[5] = version;
        header[6..8].copy_from_slice(&(payload.len() as u16).to_le_bytes());
        header[8..12].copy_from_slice(&sequence.to_le_bytes());
        let crc = crc32(&[&header[..12], payload]);
        header[12..16].copy_from_slice(&crc.to_le_bytes());
        self.flash
            .write_nor(base, &header)
            .map_err(StoreError::Flash)?;
        self.flash
            .write_nor(base + HEADER_BYTES as u32, payload)
            .map_err(StoreError::Flash)?;
        self.flash
            .write_nor(base + COMMIT_OFFSET as u32, &COMMIT.to_le_bytes())
            .map_err(StoreError::Flash)?;
        Ok(sequence)
    }

    fn slot(&mut self, kind: RecordKind, side: usize) -> Result<Slot, StoreError<F::Error>> {
        let base = slot_base(kind, side);
        let mut header = [0u8; HEADER_BYTES];
        self.read(base, &mut header)?;
        if header.iter().all(|byte| *byte == 0xff) {
            return Ok(Slot::Empty);
        }
        if header[COMMIT_OFFSET..] != COMMIT.to_le_bytes() {
            return Ok(Slot::Torn);
        }
        let len = usize::from(u16::from_le_bytes([header[6], header[7]]));
        if header[0..4] != MAGIC.to_le_bytes()
            || header[4] != kind as u8
            || header[16..20].iter().any(|byte| *byte != 0)
            || len > RECORD_MAX_PAYLOAD
        {
            return Ok(Slot::Corrupt);
        }
        let mut payload = [0u8; RECORD_MAX_PAYLOAD];
        self.read(base + HEADER_BYTES as u32, &mut payload[..len])?;
        let stored = u32::from_le_bytes(header[12..16].try_into().expect("four bytes"));
        if stored != crc32(&[&header[..12], &payload[..len]]) {
            return Ok(Slot::Corrupt);
        }
        Ok(Slot::Valid {
            version: header[5],
            len,
            sequence: u32::from_le_bytes(header[8..12].try_into().expect("four bytes")),
        })
    }

    fn read(&mut self, offset: u32, output: &mut [u8]) -> Result<(), StoreError<F::Error>> {
        self.flash.read(offset, output).map_err(StoreError::Flash)
    }
}

const fn slot_base(kind: RecordKind, side: usize) -> u32 {
    kind.base() + side as u32 * SETTINGS_SECTOR_SIZE
}

/// Wrap-safe: `candidate` is newer if it is less than half the space ahead.
fn newer(candidate: u32, current: u32) -> bool {
    candidate != current && candidate.wrapping_sub(current) < 0x8000_0000
}

#[cfg(test)]
pub(crate) mod fake {
    extern crate std;
    use super::*;
    use std::{vec, vec::Vec};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum FakeError {
        NotNor,
        Injected,
        OutOfBounds,
    }

    /// NOR semantics plus a write budget: after `budget` bytes the next
    /// write tears (power cut) and every later operation fails.
    pub struct FakeFlash {
        pub bytes: Vec<u8>,
        pub budget: Option<usize>,
    }

    impl FakeFlash {
        pub fn new() -> Self {
            Self {
                bytes: vec![0xff; SETTINGS_PARTITION_SIZE as usize],
                budget: None,
            }
        }

        fn spend(&mut self, len: usize) -> Result<usize, FakeError> {
            match self.budget.as_mut() {
                None => Ok(len),
                Some(0) => Err(FakeError::Injected),
                Some(left) => {
                    let take = len.min(*left);
                    *left -= take;
                    Ok(take)
                }
            }
        }
    }

    impl SettingsFlash for FakeFlash {
        type Error = FakeError;

        fn read(&mut self, offset: u32, output: &mut [u8]) -> Result<(), FakeError> {
            let start = offset as usize;
            let source = self
                .bytes
                .get(start..start + output.len())
                .ok_or(FakeError::OutOfBounds)?;
            output.copy_from_slice(source);
            Ok(())
        }

        fn write_nor(&mut self, offset: u32, data: &[u8]) -> Result<(), FakeError> {
            let start = offset as usize;
            if self.bytes.get(start..start + data.len()).is_none() {
                return Err(FakeError::OutOfBounds);
            }
            let take = self.spend(data.len())?;
            for (old, new) in self.bytes[start..start + take].iter_mut().zip(data) {
                if *old & *new != *new {
                    return Err(FakeError::NotNor);
                }
                *old = *new;
            }
            if take != data.len() {
                return Err(FakeError::Injected);
            }
            Ok(())
        }

        fn erase_sector(&mut self, offset: u32) -> Result<(), FakeError> {
            if !offset.is_multiple_of(SETTINGS_SECTOR_SIZE) {
                return Err(FakeError::OutOfBounds);
            }
            self.spend(1)?;
            let start = offset as usize;
            self.bytes
                .get_mut(start..start + SETTINGS_SECTOR_SIZE as usize)
                .ok_or(FakeError::OutOfBounds)?
                .fill(0xff);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeFlash;
    use super::*;

    fn load(store: &mut RecordStore<FakeFlash>, kind: RecordKind) -> (LoadOutcome, [u8; 8]) {
        let mut payload = [0u8; 8];
        (store.load(kind, 1, &mut payload).unwrap(), payload)
    }

    #[test]
    fn writes_alternate_slots_and_the_newest_commit_wins() {
        let mut store = RecordStore::new(FakeFlash::new());
        assert_eq!(
            load(&mut store, RecordKind::Settings).0.loaded,
            Loaded::Missing
        );
        for value in 1..=5u8 {
            assert_eq!(
                store.write(RecordKind::Settings, 1, &[value; 8]),
                Ok(u32::from(value))
            );
            let (outcome, payload) = load(&mut store, RecordKind::Settings);
            assert_eq!(
                outcome.loaded,
                Loaded::Record {
                    version: 1,
                    len: 8,
                    sequence: u32::from(value)
                }
            );
            assert_eq!(payload, [value; 8]);
        }
        // Kinds are independent.
        assert_eq!(
            load(&mut store, RecordKind::Network).0.loaded,
            Loaded::Missing
        );
    }

    #[test]
    fn a_power_cut_at_any_byte_keeps_the_previous_record() {
        let mut probe = RecordStore::new(FakeFlash::new());
        probe.write(RecordKind::Settings, 1, &[1; 8]).unwrap();
        // Erase (1) + header (20) + payload (8) + commit (4).
        for cut in 0..33 {
            let mut store = RecordStore::new(FakeFlash::new());
            store.write(RecordKind::Settings, 1, &[1; 8]).unwrap();
            store.write(RecordKind::Settings, 1, &[2; 8]).unwrap();
            store.flash_mut().budget = Some(cut);
            assert!(store.write(RecordKind::Settings, 1, &[3; 8]).is_err());
            let mut flash = store.into_flash();
            flash.budget = None;
            let mut rebooted = RecordStore::new(flash);
            let (outcome, payload) = load(&mut rebooted, RecordKind::Settings);
            assert_eq!(payload, [2; 8], "cut after {cut} bytes");
            assert!(!outcome.corrupt_slot);
            // The next write succeeds and supersedes it.
            rebooted.write(RecordKind::Settings, 1, &[4; 8]).unwrap();
            assert_eq!(load(&mut rebooted, RecordKind::Settings).1, [4; 8]);
        }
    }

    #[test]
    fn corrupt_newer_slot_falls_back_and_newer_formats_are_not_decoded() {
        let mut store = RecordStore::new(FakeFlash::new());
        store.write(RecordKind::Settings, 1, &[1; 8]).unwrap();
        store.write(RecordKind::Settings, 1, &[2; 8]).unwrap(); // slot B
        let payload_byte = (SETTINGS_SECTOR_SIZE as usize) + HEADER_BYTES;
        store.flash_mut().bytes[payload_byte] ^= 0x40;
        let (outcome, payload) = load(&mut store, RecordKind::Settings);
        assert!(outcome.corrupt_slot);
        assert_eq!(payload, [1; 8]);

        store.write(RecordKind::Settings, 2, &[9; 8]).unwrap();
        let (outcome, _) = load(&mut store, RecordKind::Settings);
        assert_eq!(
            outcome.loaded,
            Loaded::Newer {
                version: 2,
                sequence: 2
            }
        );
        assert_eq!(
            store.write(RecordKind::Settings, 1, &[0; RECORD_MAX_PAYLOAD + 1]),
            Err(StoreError::PayloadTooLarge)
        );
    }

    #[test]
    fn sequence_comparison_wraps_and_layout_is_checked() {
        assert!(newer(0, u32::MAX));
        assert!(!newer(5, 6));
        let layout = SettingsFlashLayout {
            flash_bytes: 16 * 1024 * 1024,
            partition_table_offset: 0x8000,
            settings_offset: 0x7f_0000,
        };
        assert!(layout.is_valid());
        for bad in [
            SettingsFlashLayout {
                settings_offset: 0x8000,
                ..layout
            },
            SettingsFlashLayout {
                settings_offset: 0x7f_0001,
                ..layout
            },
            SettingsFlashLayout {
                settings_offset: layout.flash_bytes - 0x1000,
                ..layout
            },
            SettingsFlashLayout {
                settings_offset: 0,
                ..layout
            },
        ] {
            assert!(!bad.is_valid(), "{bad:?}");
        }
    }
}
