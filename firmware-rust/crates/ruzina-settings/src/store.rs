//! Records on NOR flash. Each record kind owns two erase sectors, and a write appends its
//! record after the newest one: header and payload, and only then the commit word. A torn
//! write or a power cut therefore leaves the previous committed record authoritative.
//!
//! A sector holds many records, so writes seldom erase: only a full sector moves on to
//! the other one, which [`RecordStore::erase_spare`] can erase ahead of time.

use crate::crc32;

pub const SETTINGS_SECTOR_SIZE: u32 = 4096;
/// Settings slots A/B, then network slots A/B.
pub const SETTINGS_PARTITION_SIZE: u32 = 4 * SETTINGS_SECTOR_SIZE;
pub const RECORD_MAX_PAYLOAD: usize = 512;

const HEADER_BYTES: usize = 24;
const MAGIC: u32 = 0x3154_5350; // "PST1"
const COMMIT_OFFSET: usize = 20;
const COMMIT: u32 = 0x5e77_1ed0;
/// Records start on this alignment within their sector.
const RECORD_ALIGN: u32 = 32;

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
enum Entry {
    /// Erased: nothing was written here, and the sector's records end.
    Empty,
    /// Written without its commit word: a cut write. What follows cannot be trusted.
    Torn,
    /// Committed but failing its checks; `len` when the header still gives one.
    Corrupt { len: Option<usize> },
    Valid {
        version: u8,
        len: usize,
        sequence: u32,
    },
}

/// What one sector holds: its newest valid record, and where the next record goes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Scan {
    /// Offset, format, length and sequence.
    newest: Option<(u32, u8, usize, u32)>,
    /// The first free offset; `None` once a cut write or damage leaves no trusted end.
    end: Option<u32>,
    corrupt: bool,
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
        let scans = [self.scan(kind, 0)?, self.scan(kind, 1)?];
        let corrupt_slot = scans.iter().any(|scan| scan.corrupt);
        let Some((side, _)) = newest(&scans) else {
            return Ok(LoadOutcome {
                loaded: Loaded::Missing,
                corrupt_slot,
            });
        };
        let (offset, version, len, sequence) = scans[side].newest.expect("newest side");
        let loaded = if version > supported {
            Loaded::Newer { version, sequence }
        } else if len > payload.len() {
            return Err(StoreError::PayloadTooLarge);
        } else {
            self.read(
                sector_base(kind, side) + offset + HEADER_BYTES as u32,
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
    ///
    /// The record goes after the newest one, in the same sector, while it fits there;
    /// only a full (or damaged) sector moves on to the other, which is erased then
    /// unless [`erase_spare`](Self::erase_spare) already did.
    pub fn write(
        &mut self,
        kind: RecordKind,
        version: u8,
        payload: &[u8],
    ) -> Result<u32, StoreError<F::Error>> {
        if payload.len() > RECORD_MAX_PAYLOAD {
            return Err(StoreError::PayloadTooLarge);
        }
        let size = record_size(payload.len());
        let scans = [self.scan(kind, 0)?, self.scan(kind, 1)?];
        let (current, sequence) = match newest(&scans) {
            Some((side, sequence)) => (side, sequence.wrapping_add(1)),
            None => (0, 1),
        };
        let mut at = None;
        if let Some(end) = scans[current].end
            && end + size <= SETTINGS_SECTOR_SIZE
            && self.erased(sector_base(kind, current) + end, size)?
        {
            at = Some(sector_base(kind, current) + end);
        }
        let at = match at {
            Some(at) => at,
            None => {
                // Move on to the other sector; with no record yet, start this one over.
                let side = if newest(&scans).is_some() {
                    current ^ 1
                } else {
                    current
                };
                let base = sector_base(kind, side);
                if !self.erased(base, SETTINGS_SECTOR_SIZE)? {
                    self.flash.erase_sector(base).map_err(StoreError::Flash)?;
                }
                base
            }
        };
        let mut header = [0u8; COMMIT_OFFSET];
        header[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        header[4] = kind as u8;
        header[5] = version;
        header[6..8].copy_from_slice(&(payload.len() as u16).to_le_bytes());
        header[8..12].copy_from_slice(&sequence.to_le_bytes());
        let crc = crc32(&[&header[..12], payload]);
        header[12..16].copy_from_slice(&crc.to_le_bytes());
        self.flash
            .write_nor(at, &header)
            .map_err(StoreError::Flash)?;
        self.flash
            .write_nor(at + HEADER_BYTES as u32, payload)
            .map_err(StoreError::Flash)?;
        self.flash
            .write_nor(at + COMMIT_OFFSET as u32, &COMMIT.to_le_bytes())
            .map_err(StoreError::Flash)?;
        Ok(sequence)
    }

    /// Erases, ahead of time, the sector the next writes of `kind` will move on to once
    /// the current one is full, so that no write erases (an erase stalls a board running
    /// from flash). Returns whether a sector was erased. The sector holding the newest
    /// committed record is never touched.
    pub fn erase_spare(&mut self, kind: RecordKind) -> Result<bool, StoreError<F::Error>> {
        let scans = [self.scan(kind, 0)?, self.scan(kind, 1)?];
        let Some((current, _)) = newest(&scans) else {
            return Ok(false);
        };
        let base = sector_base(kind, current ^ 1);
        if self.erased(base, SETTINGS_SECTOR_SIZE)? {
            return Ok(false);
        }
        self.flash.erase_sector(base).map_err(StoreError::Flash)?;
        Ok(true)
    }

    /// Walks one sector's records from its start.
    fn scan(&mut self, kind: RecordKind, side: usize) -> Result<Scan, StoreError<F::Error>> {
        let base = sector_base(kind, side);
        let mut scan = Scan {
            newest: None,
            end: None,
            corrupt: false,
        };
        let mut at = 0;
        while at + HEADER_BYTES as u32 <= SETTINGS_SECTOR_SIZE {
            match self.entry(kind, base + at)? {
                Entry::Empty => {
                    scan.end = Some(at);
                    break;
                }
                // A cut write is no damage: the records before it stand.
                Entry::Torn => break,
                Entry::Corrupt { len: None } => {
                    scan.corrupt = true;
                    break;
                }
                Entry::Corrupt { len: Some(len) } => {
                    scan.corrupt = true;
                    at += record_size(len);
                }
                Entry::Valid {
                    version,
                    len,
                    sequence,
                } => {
                    if scan
                        .newest
                        .is_none_or(|(_, _, _, seen)| newer(sequence, seen))
                    {
                        scan.newest = Some((at, version, len, sequence));
                    }
                    at += record_size(len);
                }
            }
        }
        Ok(scan)
    }

    fn entry(&mut self, kind: RecordKind, at: u32) -> Result<Entry, StoreError<F::Error>> {
        let mut header = [0u8; HEADER_BYTES];
        self.read(at, &mut header)?;
        if header.iter().all(|byte| *byte == 0xff) {
            return Ok(Entry::Empty);
        }
        if header[COMMIT_OFFSET..] != COMMIT.to_le_bytes() {
            return Ok(Entry::Torn);
        }
        let len = usize::from(u16::from_le_bytes([header[6], header[7]]));
        let fits = len <= RECORD_MAX_PAYLOAD
            && (at % SETTINGS_SECTOR_SIZE) + record_size(len) <= SETTINGS_SECTOR_SIZE;
        if header[0..4] != MAGIC.to_le_bytes()
            || header[4] != kind as u8
            || header[16..20].iter().any(|byte| *byte != 0)
            || !fits
        {
            return Ok(Entry::Corrupt {
                len: fits.then_some(len),
            });
        }
        let mut payload = [0u8; RECORD_MAX_PAYLOAD];
        self.read(at + HEADER_BYTES as u32, &mut payload[..len])?;
        let stored = u32::from_le_bytes(header[12..16].try_into().expect("four bytes"));
        if stored != crc32(&[&header[..12], &payload[..len]]) {
            return Ok(Entry::Corrupt { len: Some(len) });
        }
        Ok(Entry::Valid {
            version: header[5],
            len,
            sequence: u32::from_le_bytes(header[8..12].try_into().expect("four bytes")),
        })
    }

    /// Whether `len` bytes at `at` read erased, checked 256 bytes at a time.
    fn erased(&mut self, at: u32, len: u32) -> Result<bool, StoreError<F::Error>> {
        let mut chunk = [0u8; 256];
        let mut offset = 0;
        while offset < len {
            let take = (len - offset).min(chunk.len() as u32) as usize;
            self.read(at + offset, &mut chunk[..take])?;
            if chunk[..take].iter().any(|byte| *byte != 0xff) {
                return Ok(false);
            }
            offset += take as u32;
        }
        Ok(true)
    }

    fn read(&mut self, offset: u32, output: &mut [u8]) -> Result<(), StoreError<F::Error>> {
        self.flash.read(offset, output).map_err(StoreError::Flash)
    }
}

/// The side holding the newest valid record of the two sectors, and its sequence.
fn newest(scans: &[Scan; 2]) -> Option<(usize, u32)> {
    (0..2)
        .filter_map(|side| {
            scans[side]
                .newest
                .map(|(_, _, _, sequence)| (side, sequence))
        })
        .reduce(|a, b| if newer(b.1, a.1) { b } else { a })
}

/// A record's room in its sector: header and payload, rounded up to [`RECORD_ALIGN`].
const fn record_size(len: usize) -> u32 {
    (HEADER_BYTES as u32 + len as u32).div_ceil(RECORD_ALIGN) * RECORD_ALIGN
}

const fn sector_base(kind: RecordKind, side: usize) -> u32 {
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
        /// Sectors erased so far.
        pub erases: usize,
    }

    impl FakeFlash {
        pub fn new() -> Self {
            Self {
                bytes: vec![0xff; SETTINGS_PARTITION_SIZE as usize],
                budget: None,
                erases: 0,
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
            self.erases += 1;
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
    fn records_append_in_a_sector_and_the_newest_commit_wins() {
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
        // One after another in the first sector, 32 bytes each, with no erase.
        assert_eq!(record_size(8), 32);
        assert_eq!(store.flash_mut().bytes[4 * 32 + HEADER_BYTES], 5);
        assert_eq!(store.flash_mut().erases, 0);
        // Kinds are independent.
        assert_eq!(
            load(&mut store, RecordKind::Network).0.loaded,
            Loaded::Missing
        );
    }

    #[test]
    fn a_full_sector_moves_on_and_the_spare_is_erased_ahead() {
        let mut store = RecordStore::new(FakeFlash::new());
        let per_sector = SETTINGS_SECTOR_SIZE / record_size(8);
        let mut sequence = 0u32;
        let mut write = |store: &mut RecordStore<FakeFlash>| {
            sequence += 1;
            assert_eq!(
                store.write(RecordKind::Settings, 1, &[sequence as u8; 8]),
                Ok(sequence)
            );
        };
        // A whole sector, then on into the blank second one: no erase.
        for _ in 0..=per_sector {
            write(&mut store);
        }
        assert_eq!(store.flash_mut().erases, 0);
        assert_eq!(
            store.flash_mut().bytes[SETTINGS_SECTOR_SIZE as usize + HEADER_BYTES],
            (per_sector + 1) as u8
        );
        // The first sector holds only older records now; erasing it ahead spares the
        // write that next moves back to it its erase.
        assert_eq!(store.erase_spare(RecordKind::Settings), Ok(true));
        assert_eq!(store.erase_spare(RecordKind::Settings), Ok(false));
        assert_eq!(store.flash_mut().erases, 1);
        assert_eq!(
            load(&mut store, RecordKind::Settings).1,
            [(per_sector + 1) as u8; 8]
        );
        for _ in 0..per_sector {
            write(&mut store);
        }
        assert_eq!(store.flash_mut().erases, 1);
        // Without an erase ahead, moving back to the second sector erases it.
        for _ in 0..per_sector {
            write(&mut store);
        }
        assert_eq!(store.flash_mut().erases, 2);
        assert_eq!(
            load(&mut store, RecordKind::Settings).1,
            [sequence as u8; 8]
        );
        // A blank store has no spare to erase; the other kind is untouched.
        let mut blank = RecordStore::new(FakeFlash::new());
        assert_eq!(blank.erase_spare(RecordKind::Settings), Ok(false));
        assert_eq!(
            load(&mut store, RecordKind::Network).0.loaded,
            Loaded::Missing
        );
    }

    #[test]
    fn a_power_cut_at_any_byte_keeps_the_previous_record() {
        // Header (20) + payload (8) + commit (4), appended with no erase; and the same
        // for a write that moves on to a used sector, which erases first (1).
        for (before, bytes) in [(2usize, 32usize), (2 * 128, 33)] {
            for cut in 0..bytes {
                let mut store = RecordStore::new(FakeFlash::new());
                for value in 1..=before {
                    store
                        .write(RecordKind::Settings, 1, &[value as u8; 8])
                        .unwrap();
                }
                store.flash_mut().budget = Some(cut);
                assert!(store.write(RecordKind::Settings, 1, &[0xaa; 8]).is_err());
                let mut flash = store.into_flash();
                flash.budget = None;
                let mut rebooted = RecordStore::new(flash);
                let (outcome, payload) = load(&mut rebooted, RecordKind::Settings);
                assert_eq!(payload, [before as u8; 8], "cut after {cut} bytes");
                assert!(!outcome.corrupt_slot);
                // The next write succeeds and supersedes it.
                rebooted.write(RecordKind::Settings, 1, &[0xbb; 8]).unwrap();
                assert_eq!(load(&mut rebooted, RecordKind::Settings).1, [0xbb; 8]);
                // And so does the one after it.
                rebooted.write(RecordKind::Settings, 1, &[0xcc; 8]).unwrap();
                assert_eq!(load(&mut rebooted, RecordKind::Settings).1, [0xcc; 8]);
            }
        }
    }

    #[test]
    fn records_written_by_the_two_slot_store_still_load_and_append() {
        // The earlier store wrote each record at the start of a sector, erasing it first:
        // records 1 and 3 in the first sector (3 over 1), 2 in the second.
        let mut store = RecordStore::new(FakeFlash::new());
        for value in 1..=3 {
            store.write(RecordKind::Settings, 1, &[value; 8]).unwrap();
        }
        let mut flash = store.into_flash();
        let (mut second, mut third) = ([0u8; 32], [0u8; 32]);
        second.copy_from_slice(&flash.bytes[32..64]);
        third.copy_from_slice(&flash.bytes[64..96]);
        flash.bytes[..SETTINGS_SECTOR_SIZE as usize].fill(0xff);
        flash.bytes[..32].copy_from_slice(&third);
        flash.bytes[SETTINGS_SECTOR_SIZE as usize..][..32].copy_from_slice(&second);
        let mut store = RecordStore::new(flash);
        assert_eq!(load(&mut store, RecordKind::Settings).1, [3; 8]);
        // The next goes after record 3, in its sector, with no erase.
        store.write(RecordKind::Settings, 1, &[4; 8]).unwrap();
        assert_eq!(store.flash_mut().bytes[32 + HEADER_BYTES], 4);
        assert_eq!(store.flash_mut().erases, 0);
        assert_eq!(load(&mut store, RecordKind::Settings).1, [4; 8]);
    }

    #[test]
    fn corrupt_newer_record_falls_back_and_newer_formats_are_not_decoded() {
        let mut store = RecordStore::new(FakeFlash::new());
        store.write(RecordKind::Settings, 1, &[1; 8]).unwrap();
        store.write(RecordKind::Settings, 1, &[2; 8]).unwrap(); // after record 1
        let payload_byte = record_size(8) as usize + HEADER_BYTES;
        store.flash_mut().bytes[payload_byte] ^= 0x40;
        let (outcome, payload) = load(&mut store, RecordKind::Settings);
        assert!(outcome.corrupt_slot);
        assert_eq!(payload, [1; 8]);

        // A damaged record is stepped over; the next goes after it.
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
        // A header that gives no length ends the sector's records; writes move on.
        let mut store = RecordStore::new(FakeFlash::new());
        store.write(RecordKind::Settings, 1, &[1; 8]).unwrap();
        store.write(RecordKind::Settings, 1, &[2; 8]).unwrap();
        store.flash_mut().bytes[record_size(8) as usize + 7] = 0xfe; // length 0xfe08
        let (outcome, payload) = load(&mut store, RecordKind::Settings);
        assert!(outcome.corrupt_slot);
        assert_eq!(payload, [1; 8]);
        store.write(RecordKind::Settings, 1, &[3; 8]).unwrap();
        assert_eq!(
            store.flash_mut().bytes[SETTINGS_SECTOR_SIZE as usize + HEADER_BYTES],
            3
        );
        assert_eq!(load(&mut store, RecordKind::Settings).1, [3; 8]);
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
