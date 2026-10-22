#![no_std]
#![forbid(unsafe_code)]

pub mod journal;

use pajoniiir_core::MediaTrackId;
use pajoniiir_media_identity::PersistentMediaId;

pub const HOT_CUE_SLOT_COUNT: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotCueKind {
    Single,
    Loop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotCueSlot {
    pub pos_ms: u32,
    pub end_ms: u32,
    pub kind: HotCueKind,
}

impl HotCueSlot {
    pub const fn single(pos_ms: u32) -> Self {
        Self {
            pos_ms,
            end_ms: 0,
            kind: HotCueKind::Single,
        }
    }

    pub const fn loop_region(pos_ms: u32, end_ms: u32) -> Option<Self> {
        if end_ms > pos_ms {
            Some(Self {
                pos_ms,
                end_ms,
                kind: HotCueKind::Loop,
            })
        } else {
            None
        }
    }
}

/// Per-track local Hot Cue bank.
///
/// `slots` contains locally stored cue values. `override_mask` states which
/// source slots are replaced by the local bank. An override bit with an empty
/// slot is a durable deletion/tombstone and therefore intentionally differs
/// from "no local override".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotCueBank {
    track_id: MediaTrackId,
    slots: [Option<HotCueSlot>; HOT_CUE_SLOT_COUNT],
    override_mask: u8,
}

impl HotCueBank {
    pub const fn empty(track_id: MediaTrackId) -> Self {
        Self {
            track_id,
            slots: [None; HOT_CUE_SLOT_COUNT],
            override_mask: 0,
        }
    }

    pub const fn track_id(&self) -> MediaTrackId {
        self.track_id
    }

    pub const fn override_mask(&self) -> u8 {
        self.override_mask
    }

    /// Converts a bank into imported/source semantics.
    ///
    /// Source cue presence is carried by the slots themselves; source entries do
    /// not create local overrides.
    pub const fn as_source(mut self) -> Self {
        self.override_mask = 0;
        self
    }

    pub fn slot(&self, index: u8) -> Option<HotCueSlot> {
        self.slots.get(index as usize).copied().flatten()
    }

    pub fn is_overridden(&self, index: u8) -> bool {
        index < HOT_CUE_SLOT_COUNT as u8 && self.override_mask & (1u8 << index) != 0
    }

    pub fn set_single(&mut self, index: u8, pos_ms: u32) -> bool {
        let Some(slot) = self.slots.get_mut(index as usize) else {
            return false;
        };
        *slot = Some(HotCueSlot::single(pos_ms));
        self.override_mask |= 1u8 << index;
        true
    }

    pub fn set_loop(&mut self, index: u8, pos_ms: u32, end_ms: u32) -> bool {
        let Some(cue) = HotCueSlot::loop_region(pos_ms, end_ms) else {
            return false;
        };
        let Some(slot) = self.slots.get_mut(index as usize) else {
            return false;
        };
        *slot = Some(cue);
        self.override_mask |= 1u8 << index;
        true
    }

    /// Creates a durable local deletion override.
    ///
    /// Clearing an empty slot for the first time is meaningful because the
    /// resulting tombstone can hide an imported Rekordbox cue.
    pub fn clear(&mut self, index: u8) -> bool {
        let Some(slot) = self.slots.get_mut(index as usize) else {
            return false;
        };
        let bit = 1u8 << index;
        if slot.is_none() && self.override_mask & bit != 0 {
            return false;
        }
        *slot = None;
        self.override_mask |= bit;
        true
    }

    pub fn exists_mask(&self) -> u8 {
        let mut mask = 0u8;
        for (index, slot) in self.slots.iter().enumerate() {
            if slot.is_some() {
                mask |= 1u8 << index;
            }
        }
        mask
    }

    /// Merges imported/source cues with durable local overrides.
    ///
    /// Local overrides win even when their slot is empty, preserving deleted
    /// source cues across reloads. The returned bank is an effective view, not a
    /// persistence object, so its override mask is intentionally zero.
    pub fn merged(source: Self, local: Self) -> Option<Self> {
        if source.track_id != local.track_id {
            return None;
        }

        let mut merged = Self::empty(source.track_id);
        for index in 0..HOT_CUE_SLOT_COUNT {
            merged.slots[index] = if local.override_mask & (1u8 << index) != 0 {
                local.slots[index]
            } else {
                source.slots[index]
            };
        }
        Some(merged)
    }
}

pub const HOT_CUE_RECORD_V4_SIZE: usize = 132;
pub const HOT_CUE_RECORD_V3_SIZE: usize = 128;
pub const HOT_CUE_RECORD_V2_SIZE: usize = 144;

const HOT_CUE_RECORD_V4_MAGIC: u32 = 0x3443_5648;
const HOT_CUE_RECORD_V3_MAGIC: u32 = 0x3343_5648;
const HOT_CUE_RECORD_V2_MAGIC: u32 = 0x3243_5648;
const HOT_CUE_RECORD_V4_VERSION: u16 = 4;
const HOT_CUE_RECORD_V3_VERSION: u16 = 3;
const HOT_CUE_RECORD_V2_VERSION: u16 = 2;
const HOT_CUE_RECORD_SLOT_SIZE: usize = 12;
const HOT_CUE_RECORD_V4_HEADER_SIZE: usize = 32;
const HOT_CUE_RECORD_V3_HEADER_SIZE: usize = 28;
const HOT_CUE_RECORD_V2_HEADER_SIZE: usize = 44;
const HOT_CUE_RECORD_V4_PAYLOAD_LEN: u16 =
    (8 + HOT_CUE_SLOT_COUNT * HOT_CUE_RECORD_SLOT_SIZE) as u16;
const HOT_CUE_RECORD_V3_PAYLOAD_LEN: u16 =
    (4 + HOT_CUE_SLOT_COUNT * HOT_CUE_RECORD_SLOT_SIZE) as u16;
const HOT_CUE_RECORD_V4_CRC_OFFSET: usize =
    HOT_CUE_RECORD_V4_HEADER_SIZE + HOT_CUE_SLOT_COUNT * HOT_CUE_RECORD_SLOT_SIZE;
const HOT_CUE_RECORD_V3_CRC_OFFSET: usize =
    HOT_CUE_RECORD_V3_HEADER_SIZE + HOT_CUE_SLOT_COUNT * HOT_CUE_RECORD_SLOT_SIZE;
const HOT_CUE_RECORD_V2_CRC_OFFSET: usize =
    HOT_CUE_RECORD_V2_HEADER_SIZE + HOT_CUE_SLOT_COUNT * HOT_CUE_RECORD_SLOT_SIZE;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotCueRecordError {
    InvalidLength,
    InvalidMagic,
    UnsupportedVersion,
    InvalidPayloadLength,
    TrackMismatch,
    InvalidCrc,
    InvalidMask,
    InvalidSlot,
}

pub fn encode_record_v4(bank: &HotCueBank) -> [u8; HOT_CUE_RECORD_V4_SIZE] {
    let mut record = [0u8; HOT_CUE_RECORD_V4_SIZE];
    put_u32(&mut record[0..4], HOT_CUE_RECORD_V4_MAGIC);
    put_u16(&mut record[4..6], HOT_CUE_RECORD_V4_VERSION);
    put_u16(&mut record[6..8], HOT_CUE_RECORD_V4_PAYLOAD_LEN);
    record[8..24].copy_from_slice(&bank.track_id().0);
    put_u32(&mut record[24..28], bank.exists_mask() as u32);
    put_u32(&mut record[28..32], bank.override_mask() as u32);
    encode_slots(
        bank,
        &mut record[HOT_CUE_RECORD_V4_HEADER_SIZE..HOT_CUE_RECORD_V4_CRC_OFFSET],
    );

    let crc = crc32_iso(&record[..HOT_CUE_RECORD_V4_CRC_OFFSET]);
    put_u32(
        &mut record[HOT_CUE_RECORD_V4_CRC_OFFSET..HOT_CUE_RECORD_V4_SIZE],
        crc,
    );
    record
}

/// Legacy native-Rust V3 encoder retained for migration fixtures.
pub fn encode_record_v3(bank: &HotCueBank) -> [u8; HOT_CUE_RECORD_V3_SIZE] {
    let mut record = [0u8; HOT_CUE_RECORD_V3_SIZE];
    put_u32(&mut record[0..4], HOT_CUE_RECORD_V3_MAGIC);
    put_u16(&mut record[4..6], HOT_CUE_RECORD_V3_VERSION);
    put_u16(&mut record[6..8], HOT_CUE_RECORD_V3_PAYLOAD_LEN);
    record[8..24].copy_from_slice(&bank.track_id().0);
    put_u32(&mut record[24..28], bank.exists_mask() as u32);
    encode_slots(
        bank,
        &mut record[HOT_CUE_RECORD_V3_HEADER_SIZE..HOT_CUE_RECORD_V3_CRC_OFFSET],
    );

    let crc = crc32_iso(&record[..HOT_CUE_RECORD_V3_CRC_OFFSET]);
    put_u32(
        &mut record[HOT_CUE_RECORD_V3_CRC_OFFSET..HOT_CUE_RECORD_V3_SIZE],
        crc,
    );
    record
}

pub fn decode_record_v4(
    expected_track: MediaTrackId,
    record: &[u8],
) -> Result<HotCueBank, HotCueRecordError> {
    if record.len() != HOT_CUE_RECORD_V4_SIZE {
        return Err(HotCueRecordError::InvalidLength);
    }
    if get_u32(&record[0..4]) != HOT_CUE_RECORD_V4_MAGIC {
        return Err(HotCueRecordError::InvalidMagic);
    }
    if get_u16(&record[4..6]) != HOT_CUE_RECORD_V4_VERSION {
        return Err(HotCueRecordError::UnsupportedVersion);
    }
    if get_u16(&record[6..8]) != HOT_CUE_RECORD_V4_PAYLOAD_LEN {
        return Err(HotCueRecordError::InvalidPayloadLength);
    }
    if record[8..24] != expected_track.0 {
        return Err(HotCueRecordError::TrackMismatch);
    }
    let expected_crc = get_u32(&record[HOT_CUE_RECORD_V4_CRC_OFFSET..HOT_CUE_RECORD_V4_SIZE]);
    if expected_crc != crc32_iso(&record[..HOT_CUE_RECORD_V4_CRC_OFFSET]) {
        return Err(HotCueRecordError::InvalidCrc);
    }

    decode_slots(
        expected_track,
        get_u32(&record[24..28]),
        get_u32(&record[28..32]),
        &record[HOT_CUE_RECORD_V4_HEADER_SIZE..HOT_CUE_RECORD_V4_CRC_OFFSET],
        true,
    )
}

pub fn decode_record_v3(
    expected_track: MediaTrackId,
    record: &[u8],
) -> Result<HotCueBank, HotCueRecordError> {
    if record.len() != HOT_CUE_RECORD_V3_SIZE {
        return Err(HotCueRecordError::InvalidLength);
    }
    if get_u32(&record[0..4]) != HOT_CUE_RECORD_V3_MAGIC {
        return Err(HotCueRecordError::InvalidMagic);
    }
    if get_u16(&record[4..6]) != HOT_CUE_RECORD_V3_VERSION {
        return Err(HotCueRecordError::UnsupportedVersion);
    }
    if get_u16(&record[6..8]) != HOT_CUE_RECORD_V3_PAYLOAD_LEN {
        return Err(HotCueRecordError::InvalidPayloadLength);
    }
    if record[8..24] != expected_track.0 {
        return Err(HotCueRecordError::TrackMismatch);
    }
    let expected_crc = get_u32(&record[HOT_CUE_RECORD_V3_CRC_OFFSET..HOT_CUE_RECORD_V3_SIZE]);
    if expected_crc != crc32_iso(&record[..HOT_CUE_RECORD_V3_CRC_OFFSET]) {
        return Err(HotCueRecordError::InvalidCrc);
    }

    // V3 had no override mask. Treat it as an authoritative local bank so
    // source cues cannot unexpectedly reappear after migration.
    decode_slots(
        expected_track,
        get_u32(&record[24..28]),
        0xff,
        &record[HOT_CUE_RECORD_V3_HEADER_SIZE..HOT_CUE_RECORD_V3_CRC_OFFSET],
        true,
    )
}

pub fn decode_legacy_record_v2(
    expected_legacy_id: PersistentMediaId,
    migrated_track_id: MediaTrackId,
    record: &[u8],
) -> Result<HotCueBank, HotCueRecordError> {
    if record.len() != HOT_CUE_RECORD_V2_SIZE {
        return Err(HotCueRecordError::InvalidLength);
    }
    if get_u32(&record[0..4]) != HOT_CUE_RECORD_V2_MAGIC {
        return Err(HotCueRecordError::InvalidMagic);
    }
    if get_u16(&record[4..6]) != HOT_CUE_RECORD_V2_VERSION {
        return Err(HotCueRecordError::UnsupportedVersion);
    }
    if get_u16(&record[6..8]) != HOT_CUE_RECORD_V3_PAYLOAD_LEN {
        return Err(HotCueRecordError::InvalidPayloadLength);
    }
    if record[8..40] != expected_legacy_id.0 {
        return Err(HotCueRecordError::TrackMismatch);
    }
    let expected_crc = get_u32(&record[HOT_CUE_RECORD_V2_CRC_OFFSET..HOT_CUE_RECORD_V2_SIZE]);
    if expected_crc != crc32_iso(&record[..HOT_CUE_RECORD_V2_CRC_OFFSET]) {
        return Err(HotCueRecordError::InvalidCrc);
    }

    decode_slots(
        migrated_track_id,
        get_u32(&record[40..44]),
        0xff,
        &record[HOT_CUE_RECORD_V2_HEADER_SIZE..HOT_CUE_RECORD_V2_CRC_OFFSET],
        false,
    )
}

fn encode_slots(bank: &HotCueBank, output: &mut [u8]) {
    for index in 0..HOT_CUE_SLOT_COUNT {
        let offset = index * HOT_CUE_RECORD_SLOT_SIZE;
        if let Some(slot) = bank.slot(index as u8) {
            put_u32(&mut output[offset..offset + 4], slot.pos_ms);
            put_u32(&mut output[offset + 4..offset + 8], slot.end_ms);
            output[offset + 8] = match slot.kind {
                HotCueKind::Single => 1,
                HotCueKind::Loop => 2,
            };
        }
    }
}

fn decode_slots(
    track_id: MediaTrackId,
    valid_mask: u32,
    override_mask: u32,
    slots: &[u8],
    strict_reserved: bool,
) -> Result<HotCueBank, HotCueRecordError> {
    if (valid_mask | override_mask) & !0xff != 0 || valid_mask & !override_mask != 0 {
        return Err(HotCueRecordError::InvalidMask);
    }

    let mut bank = HotCueBank::empty(track_id);
    for index in 0..HOT_CUE_SLOT_COUNT {
        let offset = index * HOT_CUE_RECORD_SLOT_SIZE;
        let raw = &slots[offset..offset + HOT_CUE_RECORD_SLOT_SIZE];
        let pos_ms = get_u32(&raw[0..4]);
        let end_ms = get_u32(&raw[4..8]);
        let kind = raw[8];
        let reserved_clean = raw[9..12] == [0, 0, 0];
        let valid = valid_mask & (1u32 << index) != 0;

        if strict_reserved && !reserved_clean {
            return Err(HotCueRecordError::InvalidSlot);
        }

        if !valid {
            if pos_ms != 0 || end_ms != 0 || kind != 0 {
                return Err(HotCueRecordError::InvalidSlot);
            }
            continue;
        }

        bank.slots[index] = Some(match kind {
            1 if end_ms == 0 => HotCueSlot::single(pos_ms),
            2 => HotCueSlot::loop_region(pos_ms, end_ms).ok_or(HotCueRecordError::InvalidSlot)?,
            _ => return Err(HotCueRecordError::InvalidSlot),
        });
    }

    bank.override_mask = override_mask as u8;
    Ok(bank)
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

pub const HOT_CUE_STORAGE_KEY_LEN: usize = 15;

/// Bounded backend key matching the released product's NVS key-size constraint.
///
/// The key intentionally carries only a prefix of the provider-neutral track ID.
/// Every stored V4 record still carries the complete 16-byte ID, so a truncated
/// key collision is detected as `TrackMismatch` instead of silently returning,
/// replacing or deleting another track's cues.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotCueStorageKey([u8; HOT_CUE_STORAGE_KEY_LEN]);

impl HotCueStorageKey {
    pub fn from_track_id(track_id: MediaTrackId) -> Self {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut key = [0u8; HOT_CUE_STORAGE_KEY_LEN];
        key[0] = b'h';
        for (index, byte) in track_id.0[..7].iter().copied().enumerate() {
            key[1 + index * 2] = HEX[(byte >> 4) as usize];
            key[2 + index * 2] = HEX[(byte & 0x0f) as usize];
        }
        Self(key)
    }

    pub const fn as_bytes(&self) -> &[u8; HOT_CUE_STORAGE_KEY_LEN] {
        &self.0
    }
}

/// Minimal key/value persistence boundary used by the native Rust product.
///
/// `write_record_atomic` is deliberately stronger than a raw flash write: after
/// any reset or power loss, a subsequent read for the same key must return either
/// the complete previous record or the complete new record, never a torn record.
pub trait HotCueRecordBackend {
    type Error;

    fn read_record(
        &mut self,
        key: HotCueStorageKey,
        output: &mut [u8; HOT_CUE_RECORD_V4_SIZE],
    ) -> Result<bool, Self::Error>;

    fn write_record_atomic(
        &mut self,
        key: HotCueStorageKey,
        record: &[u8; HOT_CUE_RECORD_V4_SIZE],
    ) -> Result<(), Self::Error>;

    fn remove_record(&mut self, key: HotCueStorageKey) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordBackedHotCueStoreError<E> {
    Backend(E),
    Record(HotCueRecordError),
}

pub struct RecordBackedHotCueStore<B> {
    backend: B,
}

impl<B> RecordBackedHotCueStore<B> {
    pub const fn new(backend: B) -> Self {
        Self { backend }
    }

    pub const fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    pub fn into_backend(self) -> B {
        self.backend
    }
}

pub trait HotCueStore {
    type Error;

    fn load(&mut self, track_id: MediaTrackId) -> Result<Option<HotCueBank>, Self::Error>;
    fn save(&mut self, bank: &HotCueBank) -> Result<(), Self::Error>;
    fn clear(&mut self, track_id: MediaTrackId) -> Result<(), Self::Error>;
}

impl<B> RecordBackedHotCueStore<B>
where
    B: HotCueRecordBackend,
{
    fn validate_existing_key(
        &mut self,
        key: HotCueStorageKey,
        track_id: MediaTrackId,
    ) -> Result<(), RecordBackedHotCueStoreError<B::Error>> {
        let mut record = [0u8; HOT_CUE_RECORD_V4_SIZE];
        if self
            .backend
            .read_record(key, &mut record)
            .map_err(RecordBackedHotCueStoreError::Backend)?
        {
            decode_record_v4(track_id, &record)
                .map(|_| ())
                .map_err(RecordBackedHotCueStoreError::Record)?;
        }
        Ok(())
    }
}

impl<B> HotCueStore for RecordBackedHotCueStore<B>
where
    B: HotCueRecordBackend,
{
    type Error = RecordBackedHotCueStoreError<B::Error>;

    fn load(&mut self, track_id: MediaTrackId) -> Result<Option<HotCueBank>, Self::Error> {
        let key = HotCueStorageKey::from_track_id(track_id);
        let mut record = [0u8; HOT_CUE_RECORD_V4_SIZE];
        if !self
            .backend
            .read_record(key, &mut record)
            .map_err(RecordBackedHotCueStoreError::Backend)?
        {
            return Ok(None);
        }

        decode_record_v4(track_id, &record)
            .map(Some)
            .map_err(RecordBackedHotCueStoreError::Record)
    }

    fn save(&mut self, bank: &HotCueBank) -> Result<(), Self::Error> {
        let key = HotCueStorageKey::from_track_id(bank.track_id());
        self.validate_existing_key(key, bank.track_id())?;
        let record = encode_record_v4(bank);
        self.backend
            .write_record_atomic(key, &record)
            .map_err(RecordBackedHotCueStoreError::Backend)
    }

    fn clear(&mut self, track_id: MediaTrackId) -> Result<(), Self::Error> {
        let key = HotCueStorageKey::from_track_id(track_id);
        self.validate_existing_key(key, track_id)?;
        self.backend
            .remove_record(key)
            .map_err(RecordBackedHotCueStoreError::Backend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(seed: u8) -> MediaTrackId {
        derived_id(seed, "Music/test.wav")
    }

    fn derived_id(seed: u8, path: &str) -> MediaTrackId {
        pajoniiir_media_identity::derive_track_id(
            pajoniiir_media_identity::VolumeIdentity([seed; 32]),
            path,
        )
        .unwrap()
    }

    fn legacy_record(id: PersistentMediaId, bank: &HotCueBank) -> [u8; HOT_CUE_RECORD_V2_SIZE] {
        let mut record = [0u8; HOT_CUE_RECORD_V2_SIZE];
        put_u32(&mut record[0..4], HOT_CUE_RECORD_V2_MAGIC);
        put_u16(&mut record[4..6], HOT_CUE_RECORD_V2_VERSION);
        put_u16(&mut record[6..8], HOT_CUE_RECORD_V3_PAYLOAD_LEN);
        record[8..40].copy_from_slice(&id.0);
        put_u32(&mut record[40..44], bank.exists_mask() as u32);
        encode_slots(
            bank,
            &mut record[HOT_CUE_RECORD_V2_HEADER_SIZE..HOT_CUE_RECORD_V2_CRC_OFFSET],
        );

        let crc = crc32_iso(&record[..HOT_CUE_RECORD_V2_CRC_OFFSET]);
        put_u32(
            &mut record[HOT_CUE_RECORD_V2_CRC_OFFSET..HOT_CUE_RECORD_V2_SIZE],
            crc,
        );
        record
    }

    #[test]
    fn crc32_matches_iso_hdlc_reference_vector() {
        assert_eq!(crc32_iso(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn v4_record_roundtrip_preserves_track_slots_and_tombstones() {
        let id = derived_id(0x21, "Music/roundtrip.flac");
        let mut bank = HotCueBank::empty(id);
        assert!(bank.set_single(0, 1_000));
        assert!(bank.set_loop(2, 2_000, 4_000));
        assert!(bank.clear(6));

        let record = encode_record_v4(&bank);
        assert_eq!(record.len(), HOT_CUE_RECORD_V4_SIZE);
        let decoded = decode_record_v4(id, &record).unwrap();
        assert_eq!(decoded, bank);
        assert_eq!(decoded.override_mask(), 0x45);
        assert!(decoded.is_overridden(6));
        assert_eq!(decoded.slot(6), None);
    }

    #[test]
    fn restored_bank_record_round_trips_without_overrides_and_shadows_older_edits() {
        // RP-05 source restore persists an override-free bank. It must decode
        // as "no local overrides" so reboot shows the imported source again.
        let id = derived_id(0x22, "Music/restore.flac");
        let mut source = HotCueBank::empty(id);
        assert!(source.set_single(0, 1_000));
        let source = source.as_source();
        let restored = HotCueBank::empty(id);
        let decoded = decode_record_v4(id, &encode_record_v4(&restored)).unwrap();
        assert_eq!(decoded, restored);
        assert_eq!(decoded.override_mask(), 0);
        assert_eq!(
            HotCueBank::merged(source, decoded).unwrap().slot(0),
            Some(HotCueSlot::single(1_000))
        );
    }

    #[test]
    fn v4_record_rejects_wrong_track_crc_masks_and_invalid_slot() {
        let id = derived_id(0x31, "Music/a.wav");
        let other = derived_id(0x31, "Music/b.wav");
        let mut bank = HotCueBank::empty(id);
        assert!(bank.set_single(0, 500));
        let record = encode_record_v4(&bank);

        assert_eq!(
            decode_record_v4(other, &record),
            Err(HotCueRecordError::TrackMismatch)
        );

        let mut corrupt_crc = record;
        corrupt_crc[40] ^= 0x80;
        assert_eq!(
            decode_record_v4(id, &corrupt_crc),
            Err(HotCueRecordError::InvalidCrc)
        );

        let mut invalid_mask = record;
        put_u32(&mut invalid_mask[28..32], 0);
        let crc = crc32_iso(&invalid_mask[..HOT_CUE_RECORD_V4_CRC_OFFSET]);
        put_u32(
            &mut invalid_mask[HOT_CUE_RECORD_V4_CRC_OFFSET..HOT_CUE_RECORD_V4_SIZE],
            crc,
        );
        assert_eq!(
            decode_record_v4(id, &invalid_mask),
            Err(HotCueRecordError::InvalidMask)
        );

        let mut invalid_slot = record;
        let slot_offset = HOT_CUE_RECORD_V4_HEADER_SIZE;
        invalid_slot[slot_offset + 4] = 1;
        let crc = crc32_iso(&invalid_slot[..HOT_CUE_RECORD_V4_CRC_OFFSET]);
        put_u32(
            &mut invalid_slot[HOT_CUE_RECORD_V4_CRC_OFFSET..HOT_CUE_RECORD_V4_SIZE],
            crc,
        );
        assert_eq!(
            decode_record_v4(id, &invalid_slot),
            Err(HotCueRecordError::InvalidSlot)
        );
    }

    #[test]
    fn v3_record_migrates_as_authoritative_override_bank() {
        let id = derived_id(0x22, "Music/v3.flac");
        let mut bank = HotCueBank::empty(id);
        assert!(bank.set_single(1, 1_500));

        let record = encode_record_v3(&bank);
        let decoded = decode_record_v3(id, &record).unwrap();
        assert_eq!(decoded.slot(1), bank.slot(1));
        assert_eq!(decoded.override_mask(), 0xff);
    }

    #[test]
    fn legacy_v2_record_migrates_into_new_track_id() {
        let export_digest = pajoniiir_media_identity::sha256(b"legacy export");
        let legacy_id = pajoniiir_media_identity::derive_legacy_hotcue_v2(
            export_digest,
            "/Contents/song.wav",
            1_234,
            5_678,
        )
        .unwrap();
        let migrated_id = derived_id(0x44, "Contents/song.wav");
        let mut legacy_bank = HotCueBank::empty(migrated_id);
        assert!(legacy_bank.set_single(1, 9_000));
        assert!(legacy_bank.set_loop(6, 12_000, 16_000));

        let record = legacy_record(legacy_id, &legacy_bank);
        let migrated = decode_legacy_record_v2(legacy_id, migrated_id, &record).unwrap();

        assert_eq!(migrated.slot(1), legacy_bank.slot(1));
        assert_eq!(migrated.slot(6), legacy_bank.slot(6));
        assert_eq!(migrated.override_mask(), 0xff);
    }

    #[derive(Clone, Copy)]
    struct OneRecordBackend {
        key: Option<HotCueStorageKey>,
        record: [u8; HOT_CUE_RECORD_V4_SIZE],
    }

    impl OneRecordBackend {
        const fn new() -> Self {
            Self {
                key: None,
                record: [0u8; HOT_CUE_RECORD_V4_SIZE],
            }
        }
    }

    impl HotCueRecordBackend for OneRecordBackend {
        type Error = ();

        fn read_record(
            &mut self,
            key: HotCueStorageKey,
            output: &mut [u8; HOT_CUE_RECORD_V4_SIZE],
        ) -> Result<bool, Self::Error> {
            if self.key != Some(key) {
                return Ok(false);
            }
            *output = self.record;
            Ok(true)
        }

        fn write_record_atomic(
            &mut self,
            key: HotCueStorageKey,
            record: &[u8; HOT_CUE_RECORD_V4_SIZE],
        ) -> Result<(), Self::Error> {
            self.record = *record;
            self.key = Some(key);
            Ok(())
        }

        fn remove_record(&mut self, key: HotCueStorageKey) -> Result<(), Self::Error> {
            if self.key == Some(key) {
                self.key = None;
                self.record = [0u8; HOT_CUE_RECORD_V4_SIZE];
            }
            Ok(())
        }
    }

    #[test]
    fn bounded_storage_key_matches_nvs_limit_and_is_deterministic() {
        let track = MediaTrackId([
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x10, 0x32, 0x54, 0x76, 0x98, 0xba,
            0xdc, 0xfe,
        ]);
        let key = HotCueStorageKey::from_track_id(track);
        assert_eq!(key.as_bytes(), b"h0123456789abcd");
        assert_eq!(key.as_bytes().len(), HOT_CUE_STORAGE_KEY_LEN);
    }

    #[test]
    fn record_backed_store_roundtrips_and_clears_v4_bank() {
        let track = id(0x51);
        let mut bank = HotCueBank::empty(track);
        assert!(bank.set_single(0, 1_250));
        assert!(bank.set_loop(4, 5_000, 9_000));
        assert!(bank.clear(7));

        let mut store = RecordBackedHotCueStore::new(OneRecordBackend::new());
        assert_eq!(store.load(track), Ok(None));
        assert_eq!(store.save(&bank), Ok(()));
        assert_eq!(store.load(track), Ok(Some(bank)));
        assert_eq!(store.clear(track), Ok(()));
        assert_eq!(store.load(track), Ok(None));
    }

    #[test]
    fn truncated_backend_key_collision_fails_closed_on_load() {
        let prefix = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77];
        let first = MediaTrackId([
            prefix[0], prefix[1], prefix[2], prefix[3], prefix[4], prefix[5], prefix[6], 0x01, 0,
            0, 0, 0, 0, 0, 0, 1,
        ]);
        let second = MediaTrackId([
            prefix[0], prefix[1], prefix[2], prefix[3], prefix[4], prefix[5], prefix[6], 0x02, 0,
            0, 0, 0, 0, 0, 0, 2,
        ]);
        assert_eq!(
            HotCueStorageKey::from_track_id(first),
            HotCueStorageKey::from_track_id(second)
        );

        let mut bank = HotCueBank::empty(first);
        assert!(bank.set_single(2, 7_000));
        let mut store = RecordBackedHotCueStore::new(OneRecordBackend::new());
        assert_eq!(store.save(&bank), Ok(()));

        assert_eq!(
            store.load(second),
            Err(RecordBackedHotCueStoreError::Record(
                HotCueRecordError::TrackMismatch
            ))
        );
    }

    #[test]
    fn short_key_collision_cannot_overwrite_or_delete_foreign_record() {
        let prefix = [0xa1, 0xb2, 0xc3, 0xd4, 0xe5, 0xf6, 0x07];
        let first = MediaTrackId([
            prefix[0], prefix[1], prefix[2], prefix[3], prefix[4], prefix[5], prefix[6], 1, 0, 0,
            0, 0, 0, 0, 0, 1,
        ]);
        let second = MediaTrackId([
            prefix[0], prefix[1], prefix[2], prefix[3], prefix[4], prefix[5], prefix[6], 2, 0, 0,
            0, 0, 0, 0, 0, 2,
        ]);

        let mut first_bank = HotCueBank::empty(first);
        assert!(first_bank.set_single(0, 1_000));
        let mut second_bank = HotCueBank::empty(second);
        assert!(second_bank.set_single(0, 9_000));

        let mut store = RecordBackedHotCueStore::new(OneRecordBackend::new());
        assert_eq!(store.save(&first_bank), Ok(()));
        assert_eq!(
            store.save(&second_bank),
            Err(RecordBackedHotCueStoreError::Record(
                HotCueRecordError::TrackMismatch
            ))
        );
        assert_eq!(
            store.clear(second),
            Err(RecordBackedHotCueStoreError::Record(
                HotCueRecordError::TrackMismatch
            ))
        );
        assert_eq!(store.load(first), Ok(Some(first_bank)));
    }

    #[test]
    fn merge_uses_local_overrides_and_preserves_durable_deletion() {
        let track = id(0x61);
        let mut source = HotCueBank::empty(track);
        assert!(source.set_single(0, 1_000));
        assert!(source.set_single(2, 3_000));
        source = source.as_source();

        let mut local = HotCueBank::empty(track);
        assert!(local.set_single(2, 3_500));
        assert!(local.clear(0));

        let effective = HotCueBank::merged(source, local).unwrap();
        assert_eq!(effective.slot(0), None);
        assert_eq!(effective.slot(2), Some(HotCueSlot::single(3_500)));
        assert_eq!(effective.override_mask(), 0);
    }

    #[test]
    fn empty_bank_has_no_slots_or_overrides() {
        let bank = HotCueBank::empty(id(1));
        assert_eq!(bank.track_id(), id(1));
        assert_eq!(bank.exists_mask(), 0);
        assert_eq!(bank.override_mask(), 0);
        for slot in 0..HOT_CUE_SLOT_COUNT as u8 {
            assert_eq!(bank.slot(slot), None);
        }
    }

    #[test]
    fn single_and_loop_slots_are_bounded() {
        let mut bank = HotCueBank::empty(id(2));
        assert!(bank.set_single(0, 1_000));
        assert!(bank.set_loop(7, 2_000, 4_000));
        assert!(!bank.set_single(8, 5_000));
        assert!(!bank.set_loop(1, 5_000, 5_000));

        assert_eq!(bank.slot(0), Some(HotCueSlot::single(1_000)));
        assert_eq!(
            bank.slot(7),
            Some(HotCueSlot {
                pos_ms: 2_000,
                end_ms: 4_000,
                kind: HotCueKind::Loop,
            })
        );
        assert_eq!(bank.exists_mask(), 0x81);
        assert_eq!(bank.override_mask(), 0x81);
    }

    #[test]
    fn clear_is_idempotent_and_reports_first_tombstone() {
        let mut bank = HotCueBank::empty(id(3));
        assert!(bank.clear(3));
        assert!(!bank.clear(3));
        assert!(bank.set_single(3, 9_000));
        assert!(bank.clear(3));
        assert!(!bank.clear(3));
        assert_eq!(bank.exists_mask(), 0);
        assert_eq!(bank.override_mask(), 1 << 3);
    }
}
