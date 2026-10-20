#![no_std]
#![forbid(unsafe_code)]

use core::num::NonZeroU64;

pub const MAX_PARTITION_CANDIDATES: usize = 8;
pub const MIN_SECTOR_SIZE: usize = 512;
pub const MAX_GPT_ENTRY_SIZE: usize = 512;
pub const MAX_GPT_ENTRY_COUNT: u32 = 1_024;

const MICROSOFT_BASIC_DATA_GUID: [u8; 16] = [
    0xa2, 0xa0, 0xd0, 0xeb, 0xe5, 0xb9, 0x33, 0x44, 0x87, 0xc0, 0x68, 0xb6, 0xb7, 0x26, 0x99, 0xc7,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VolumeKind {
    Unknown,
    Fat,
    ExFat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PartitionCandidate {
    pub first_lba: u64,
    pub sector_count: Option<NonZeroU64>,
    pub kind: VolumeKind,
}

impl PartitionCandidate {
    const EMPTY: Self = Self {
        first_lba: 0,
        sector_count: None,
        kind: VolumeKind::Unknown,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PartitionLayout {
    candidates: [PartitionCandidate; MAX_PARTITION_CANDIDATES],
    count: usize,
    protective_mbr: bool,
}

impl PartitionLayout {
    pub const fn new() -> Self {
        Self {
            candidates: [PartitionCandidate::EMPTY; MAX_PARTITION_CANDIDATES],
            count: 0,
            protective_mbr: false,
        }
    }

    pub fn clear(&mut self) {
        *self = Self::new();
    }

    pub fn candidates(&self) -> &[PartitionCandidate] {
        &self.candidates[..self.count]
    }

    pub const fn count(&self) -> usize {
        self.count
    }

    pub const fn has_protective_mbr(&self) -> bool {
        self.protective_mbr
    }

    pub fn append_candidate(
        &mut self,
        first_lba: u64,
        sector_count: Option<NonZeroU64>,
        kind: VolumeKind,
    ) -> bool {
        if let Some(existing) = self.candidates[..self.count]
            .iter_mut()
            .find(|candidate| candidate.first_lba == first_lba)
        {
            if existing.kind == VolumeKind::Unknown && kind != VolumeKind::Unknown {
                existing.kind = kind;
            }
            if existing.sector_count.is_none() {
                existing.sector_count = sector_count;
            }
            return true;
        }

        if self.count >= MAX_PARTITION_CANDIDATES {
            return false;
        }

        self.candidates[self.count] = PartitionCandidate {
            first_lba,
            sector_count,
            kind,
        };
        self.count += 1;
        true
    }
}

impl Default for PartitionLayout {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PartitionScanResult {
    Ok,
    Invalid,
    NeedsGpt,
    NoCandidate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GptTableInfo {
    pub entries_lba: u64,
    pub entry_count: u32,
    pub entry_size: u32,
    pub entry_array_crc32: u32,
}

pub fn parse_gpt_header(header_sector: &[u8]) -> Option<GptTableInfo> {
    if header_sector.len() < MIN_SECTOR_SIZE
        || header_sector.get(..8) != Some(b"EFI PART".as_slice())
    {
        return None;
    }

    let header_size = read_u32_le(&header_sector[12..16]) as usize;
    if !(92..=MIN_SECTOR_SIZE).contains(&header_size) {
        return None;
    }

    let expected_header_crc = read_u32_le(&header_sector[16..20]);
    if gpt_header_crc32(header_sector, header_size)? != expected_header_crc {
        return None;
    }

    let entries_lba = read_u64_le(&header_sector[72..80]);
    let entry_count = read_u32_le(&header_sector[80..84]);
    let entry_size = read_u32_le(&header_sector[84..88]);
    let entry_array_crc32 = read_u32_le(&header_sector[88..92]);
    if entries_lba == 0
        || entry_count == 0
        || entry_count > MAX_GPT_ENTRY_COUNT
        || !(128..=MAX_GPT_ENTRY_SIZE).contains(&(entry_size as usize))
    {
        return None;
    }

    Some(GptTableInfo {
        entries_lba,
        entry_count,
        entry_size,
        entry_array_crc32,
    })
}

pub fn append_gpt_entries(
    entries: &[u8],
    entry_size: u32,
    max_entries: u32,
    layout: &mut PartitionLayout,
) -> u32 {
    let entry_size = entry_size as usize;
    if !(128..=512).contains(&entry_size) || max_entries == 0 {
        return 0;
    }

    let complete_entries = entries.len() / entry_size;
    let entries_to_scan = complete_entries.min(max_entries as usize);
    let mut scanned = 0u32;

    for index in 0..entries_to_scan {
        let entry = &entries[index * entry_size..(index + 1) * entry_size];
        scanned += 1;
        if entry.get(..16) != Some(MICROSOFT_BASIC_DATA_GUID.as_slice()) {
            continue;
        }

        let first_lba = read_u64_le(&entry[32..40]);
        let last_lba = read_u64_le(&entry[40..48]);
        if first_lba == 0 || last_lba < first_lba {
            continue;
        }
        let Some(sector_count) = last_lba
            .checked_sub(first_lba)
            .and_then(|distance| distance.checked_add(1))
            .and_then(NonZeroU64::new)
        else {
            continue;
        };

        if !layout.append_candidate(first_lba, Some(sector_count), VolumeKind::Unknown) {
            break;
        }
    }

    scanned
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GptEntryStream {
    entry_size: u16,
    remaining_entries: u32,
    partial: [u8; MAX_GPT_ENTRY_SIZE],
    partial_len: u16,
    crc32_state: u32,
    expected_crc32: u32,
}

impl GptEntryStream {
    pub fn new(info: GptTableInfo) -> Option<Self> {
        let entry_size = info.entry_size as usize;
        if !(128..=MAX_GPT_ENTRY_SIZE).contains(&entry_size)
            || info.entry_count == 0
            || info.entry_count > MAX_GPT_ENTRY_COUNT
        {
            return None;
        }

        Some(Self {
            entry_size: info.entry_size as u16,
            remaining_entries: info.entry_count,
            partial: [0u8; MAX_GPT_ENTRY_SIZE],
            partial_len: 0,
            crc32_state: u32::MAX,
            expected_crc32: info.entry_array_crc32,
        })
    }

    pub const fn entry_size(&self) -> usize {
        self.entry_size as usize
    }

    pub const fn remaining_entries(&self) -> u32 {
        self.remaining_entries
    }

    pub const fn pending_bytes(&self) -> usize {
        self.partial_len as usize
    }

    pub const fn is_complete(&self) -> bool {
        self.remaining_entries == 0
    }

    pub const fn is_valid_complete(&self) -> bool {
        self.remaining_entries == 0
            && self.partial_len == 0
            && !self.crc32_state == self.expected_crc32
    }

    /// Feed an arbitrary raw GPT entry-table byte chunk.
    ///
    /// The chunk does not need to align to GPT entry boundaries. At most one
    /// partial entry is retained internally; complete entries are forwarded to
    /// the existing bounded candidate parser immediately.
    pub fn push(&mut self, mut bytes: &[u8], layout: &mut PartitionLayout) -> u32 {
        let entry_size = self.entry_size();
        let mut scanned = 0u32;

        while !bytes.is_empty() && self.remaining_entries != 0 {
            if self.partial_len == 0 && bytes.len() >= entry_size {
                let entry = &bytes[..entry_size];
                self.crc32_state = crc32_update(self.crc32_state, entry);
                let _ = append_gpt_entries(entry, self.entry_size as u32, 1, layout);
                bytes = &bytes[entry_size..];
                self.remaining_entries -= 1;
                scanned += 1;
                continue;
            }

            let partial_len = self.partial_len as usize;
            let needed = entry_size - partial_len;
            let take = needed.min(bytes.len());
            let chunk = &bytes[..take];
            self.crc32_state = crc32_update(self.crc32_state, chunk);
            self.partial[partial_len..partial_len + take].copy_from_slice(chunk);
            self.partial_len += take as u16;
            bytes = &bytes[take..];

            if self.partial_len as usize == entry_size {
                let entry = &self.partial[..entry_size];
                let _ = append_gpt_entries(entry, self.entry_size as u32, 1, layout);
                self.partial_len = 0;
                self.remaining_entries -= 1;
                scanned += 1;
            }
        }

        scanned
    }
}

pub fn classify_boot_sector(sector: &[u8]) -> VolumeKind {
    if !has_boot_signature(sector) || !has_valid_boot_jump(sector) {
        return VolumeKind::Unknown;
    }

    if sector.get(3..11) == Some(b"EXFAT   ".as_slice()) {
        return VolumeKind::ExFat;
    }

    if sector.get(54..57) == Some(b"FAT".as_slice())
        || sector.get(82..87) == Some(b"FAT32".as_slice())
    {
        return VolumeKind::Fat;
    }

    VolumeKind::Unknown
}

pub fn scan_mbr_or_superfloppy(sector: &[u8], layout: &mut PartitionLayout) -> PartitionScanResult {
    layout.clear();
    if !has_boot_signature(sector) {
        return PartitionScanResult::Invalid;
    }

    let superfloppy_kind = classify_boot_sector(sector);
    if superfloppy_kind != VolumeKind::Unknown {
        return if layout.append_candidate(0, None, superfloppy_kind) {
            PartitionScanResult::Ok
        } else {
            PartitionScanResult::Invalid
        };
    }

    for index in 0..4usize {
        let offset = 446 + index * 16;
        let entry = &sector[offset..offset + 16];
        let partition_type = entry[4];
        let first_lba = read_u32_le(&entry[8..12]);
        let sector_count = read_u32_le(&entry[12..16]);

        if partition_type == 0 {
            continue;
        }
        if partition_type == 0xee {
            layout.protective_mbr = true;
            continue;
        }
        if first_lba == 0 || sector_count == 0 || !mbr_range_fits(first_lba, sector_count) {
            continue;
        }

        let kind = if is_mbr_fat_type(partition_type) {
            Some(VolumeKind::Fat)
        } else if partition_type == 0x07 {
            Some(VolumeKind::Unknown)
        } else {
            None
        };

        if let Some(kind) = kind {
            let sectors = NonZeroU64::new(sector_count as u64);
            let _ = layout.append_candidate(first_lba as u64, sectors, kind);
        }
    }

    if layout.protective_mbr {
        PartitionScanResult::NeedsGpt
    } else if layout.count != 0 {
        PartitionScanResult::Ok
    } else {
        PartitionScanResult::NoCandidate
    }
}

pub fn scan_gpt(
    header_sector: &[u8],
    entries: &[u8],
    layout: &mut PartitionLayout,
) -> PartitionScanResult {
    layout.clear();
    let Some(info) = parse_gpt_header(header_sector) else {
        return PartitionScanResult::Invalid;
    };
    let Some(table_len) = (info.entry_count as usize).checked_mul(info.entry_size as usize) else {
        return PartitionScanResult::Invalid;
    };
    let Some(table) = entries.get(..table_len) else {
        return PartitionScanResult::Invalid;
    };
    if crc32(table) != info.entry_array_crc32 {
        return PartitionScanResult::Invalid;
    }

    let _ = append_gpt_entries(table, info.entry_size, info.entry_count, layout);
    if layout.count != 0 {
        PartitionScanResult::Ok
    } else {
        PartitionScanResult::NoCandidate
    }
}

fn has_boot_signature(sector: &[u8]) -> bool {
    sector.len() >= MIN_SECTOR_SIZE && read_u16_le(&sector[510..512]) == 0xaa55
}

fn has_valid_boot_jump(sector: &[u8]) -> bool {
    matches!(sector.first().copied(), Some(0xeb | 0xe9))
}

fn is_mbr_fat_type(partition_type: u8) -> bool {
    matches!(partition_type, 0x01 | 0x04 | 0x06 | 0x0b | 0x0c | 0x0e)
}

fn mbr_range_fits(first_lba: u32, sector_count: u32) -> bool {
    (first_lba as u64)
        .checked_add(sector_count as u64)
        .and_then(|end| end.checked_sub(1))
        .is_some_and(|end| end <= u32::MAX as u64)
}

fn gpt_header_crc32(header_sector: &[u8], header_size: usize) -> Option<u32> {
    let header = header_sector.get(..header_size)?;
    if header.len() < 20 {
        return None;
    }

    let mut state = u32::MAX;
    state = crc32_update(state, &header[..16]);
    state = crc32_update(state, &[0u8; 4]);
    state = crc32_update(state, &header[20..]);
    Some(!state)
}

fn crc32(input: &[u8]) -> u32 {
    !crc32_update(u32::MAX, input)
}

fn crc32_update(mut crc: u32, input: &[u8]) -> u32 {
    for &byte in input {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    crc
}

fn read_u16_le(input: &[u8]) -> u16 {
    u16::from_le_bytes([input[0], input[1]])
}

fn read_u32_le(input: &[u8]) -> u32 {
    u32::from_le_bytes([input[0], input[1], input[2], input[3]])
}

fn read_u64_le(input: &[u8]) -> u64 {
    u64::from_le_bytes([
        input[0], input[1], input[2], input[3], input[4], input[5], input[6], input[7],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_u16_le(output: &mut [u8], value: u16) {
        output.copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32_le(output: &mut [u8], value: u32) {
        output.copy_from_slice(&value.to_le_bytes());
    }

    fn put_u64_le(output: &mut [u8], value: u64) {
        output.copy_from_slice(&value.to_le_bytes());
    }

    fn exfat_boot() -> [u8; 512] {
        let mut sector = [0u8; 512];
        sector[0] = 0xeb;
        sector[1] = 0x76;
        sector[2] = 0x90;
        sector[3..11].copy_from_slice(b"EXFAT   ");
        put_u16_le(&mut sector[510..512], 0xaa55);
        sector
    }

    fn fat32_boot() -> [u8; 512] {
        let mut sector = [0u8; 512];
        sector[0] = 0xeb;
        sector[1] = 0x58;
        sector[2] = 0x90;
        sector[82..90].copy_from_slice(b"FAT32   ");
        put_u16_le(&mut sector[510..512], 0xaa55);
        sector
    }

    fn mbr(partition_type: u8, first_lba: u32, sectors: u32) -> [u8; 512] {
        let mut sector = [0u8; 512];
        let entry = &mut sector[446..462];
        entry[4] = partition_type;
        put_u32_le(&mut entry[8..12], first_lba);
        put_u32_le(&mut entry[12..16], sectors);
        put_u16_le(&mut sector[510..512], 0xaa55);
        sector
    }

    fn gpt_header(entry_count: u32, entry_size: u32, entry_array_crc32: u32) -> [u8; 512] {
        let mut sector = [0u8; 512];
        sector[..8].copy_from_slice(b"EFI PART");
        put_u32_le(&mut sector[12..16], 92);
        put_u64_le(&mut sector[72..80], 2);
        put_u32_le(&mut sector[80..84], entry_count);
        put_u32_le(&mut sector[84..88], entry_size);
        put_u32_le(&mut sector[88..92], entry_array_crc32);
        refresh_gpt_header_crc(&mut sector);
        sector
    }

    fn refresh_gpt_header_crc(sector: &mut [u8; 512]) {
        put_u32_le(&mut sector[16..20], 0);
        let crc = gpt_header_crc32(sector, 92).unwrap();
        put_u32_le(&mut sector[16..20], crc);
    }

    fn write_basic_data_entry(entry: &mut [u8], first_lba: u64, last_lba: u64) {
        entry[..16].copy_from_slice(&MICROSOFT_BASIC_DATA_GUID);
        put_u64_le(&mut entry[32..40], first_lba);
        put_u64_le(&mut entry[40..48], last_lba);
    }

    #[test]
    fn classifies_released_fat_and_exfat_boot_sectors() {
        assert_eq!(classify_boot_sector(&exfat_boot()), VolumeKind::ExFat);
        assert_eq!(classify_boot_sector(&fat32_boot()), VolumeKind::Fat);
        assert_eq!(classify_boot_sector(&[0u8; 512]), VolumeKind::Unknown);
        assert_eq!(classify_boot_sector(&[0u8; 511]), VolumeKind::Unknown);
    }

    #[test]
    fn exfat_superfloppy_is_candidate_at_lba_zero_with_unknown_extent() {
        let mut layout = PartitionLayout::new();
        assert_eq!(
            scan_mbr_or_superfloppy(&exfat_boot(), &mut layout),
            PartitionScanResult::Ok
        );
        assert_eq!(
            layout.candidates(),
            &[PartitionCandidate {
                first_lba: 0,
                sector_count: None,
                kind: VolumeKind::ExFat,
            }]
        );
    }

    #[test]
    fn fat32_and_type_07_mbr_candidates_match_released_behavior() {
        let mut layout = PartitionLayout::new();
        assert_eq!(
            scan_mbr_or_superfloppy(&mbr(0x0c, 2_048, 65_536), &mut layout),
            PartitionScanResult::Ok
        );
        assert_eq!(layout.candidates()[0].first_lba, 2_048);
        assert_eq!(layout.candidates()[0].sector_count, NonZeroU64::new(65_536));
        assert_eq!(layout.candidates()[0].kind, VolumeKind::Fat);

        assert_eq!(
            scan_mbr_or_superfloppy(&mbr(0x07, 4_096, 131_072), &mut layout),
            PartitionScanResult::Ok
        );
        assert_eq!(layout.candidates()[0].kind, VolumeKind::Unknown);
    }

    #[test]
    fn invalid_mbr_extent_past_u32_address_space_is_rejected() {
        let mut layout = PartitionLayout::new();
        assert_eq!(
            scan_mbr_or_superfloppy(&mbr(0x0c, u32::MAX - 8, 16), &mut layout),
            PartitionScanResult::NoCandidate
        );
        assert_eq!(layout.count(), 0);
    }

    #[test]
    fn protective_mbr_requires_gpt_scan() {
        let mut layout = PartitionLayout::new();
        assert_eq!(
            scan_mbr_or_superfloppy(&mbr(0xee, 1, u32::MAX), &mut layout),
            PartitionScanResult::NeedsGpt
        );
        assert!(layout.has_protective_mbr());
    }

    #[test]
    fn gpt_basic_data_candidate_matches_released_fixture() {
        let mut entries = [0u8; 512];
        write_basic_data_entry(&mut entries[..128], 32_768, 98_303);
        let header = gpt_header(4, 128, crc32(&entries));
        let mut layout = PartitionLayout::new();

        assert_eq!(
            scan_gpt(&header, &entries, &mut layout),
            PartitionScanResult::Ok
        );
        assert_eq!(
            layout.candidates()[0],
            PartitionCandidate {
                first_lba: 32_768,
                sector_count: NonZeroU64::new(65_536),
                kind: VolumeKind::Unknown,
            }
        );
    }

    #[test]
    fn gpt_header_exposes_bounded_entry_table_geometry() {
        let header = gpt_header(128, 128, 0);
        assert_eq!(
            parse_gpt_header(&header),
            Some(GptTableInfo {
                entries_lba: 2,
                entry_count: 128,
                entry_size: 128,
                entry_array_crc32: 0,
            })
        );

        let mut invalid = header;
        put_u64_le(&mut invalid[72..80], 0);
        refresh_gpt_header_crc(&mut invalid);
        assert_eq!(parse_gpt_header(&invalid), None);
    }

    #[test]
    fn gpt_stream_accepts_entry_split_across_arbitrary_chunks() {
        let mut bytes = [0u8; 384];
        write_basic_data_entry(&mut bytes[128..256], 32_768, 98_303);
        write_basic_data_entry(&mut bytes[256..384], 131_072, 196_607);
        let info = GptTableInfo {
            entries_lba: 2,
            entry_count: 3,
            entry_size: 128,
            entry_array_crc32: crc32(&bytes),
        };

        let mut stream = GptEntryStream::new(info).unwrap();
        let mut layout = PartitionLayout::new();

        assert_eq!(stream.push(&bytes[..17], &mut layout), 0);
        assert_eq!(stream.pending_bytes(), 17);
        assert_eq!(stream.push(&bytes[17..143], &mut layout), 1);
        assert_eq!(stream.remaining_entries(), 2);
        assert_eq!(stream.push(&bytes[143..301], &mut layout), 1);
        assert_eq!(stream.remaining_entries(), 1);
        assert_eq!(stream.push(&bytes[301..], &mut layout), 1);

        assert!(stream.is_complete());
        assert!(stream.is_valid_complete());
        assert_eq!(stream.pending_bytes(), 0);
        assert_eq!(layout.count(), 2);
        assert_eq!(layout.candidates()[0].first_lba, 32_768);
        assert_eq!(layout.candidates()[1].first_lba, 131_072);
    }

    #[test]
    fn gpt_stream_ignores_bytes_after_declared_entry_count() {
        let mut bytes = [0u8; 256];
        write_basic_data_entry(&mut bytes[..128], 2_048, 4_095);
        write_basic_data_entry(&mut bytes[128..], 8_192, 16_383);
        let info = GptTableInfo {
            entries_lba: 2,
            entry_count: 1,
            entry_size: 128,
            entry_array_crc32: crc32(&bytes[..128]),
        };

        let mut stream = GptEntryStream::new(info).unwrap();
        let mut layout = PartitionLayout::new();
        assert_eq!(stream.push(&bytes, &mut layout), 1);
        assert!(stream.is_complete());
        assert!(stream.is_valid_complete());
        assert_eq!(layout.count(), 1);
        assert_eq!(layout.candidates()[0].first_lba, 2_048);
    }

    #[test]
    fn gpt_header_and_stream_reject_unbounded_entry_counts() {
        let mut header = gpt_header(MAX_GPT_ENTRY_COUNT + 1, 128, 0);
        assert_eq!(parse_gpt_header(&header), None);

        put_u32_le(&mut header[80..84], MAX_GPT_ENTRY_COUNT);
        refresh_gpt_header_crc(&mut header);
        let info = parse_gpt_header(&header).unwrap();
        assert_eq!(info.entry_count, MAX_GPT_ENTRY_COUNT);
        assert!(GptEntryStream::new(info).is_some());
    }

    #[test]
    fn gpt_entries_can_be_accumulated_across_bounded_chunks() {
        let mut first_chunk = [0u8; 256];
        let mut second_chunk = [0u8; 256];
        write_basic_data_entry(&mut first_chunk[128..256], 32_768, 98_303);
        write_basic_data_entry(&mut second_chunk[..128], 131_072, 196_607);

        let mut layout = PartitionLayout::new();
        assert_eq!(append_gpt_entries(&first_chunk, 128, 2, &mut layout), 2);
        assert_eq!(layout.count(), 1);

        assert_eq!(append_gpt_entries(&second_chunk, 128, 2, &mut layout), 2);
        assert_eq!(layout.count(), 2);
        assert_eq!(layout.candidates()[0].first_lba, 32_768);
        assert_eq!(layout.candidates()[1].first_lba, 131_072);
    }

    #[test]
    fn gpt_chunk_scanner_ignores_partial_or_invalid_entry_sizes() {
        let mut layout = PartitionLayout::new();
        let entries = [0u8; 127];
        assert_eq!(append_gpt_entries(&entries, 128, 1, &mut layout), 0);
        assert_eq!(append_gpt_entries(&[0u8; 512], 64, 8, &mut layout), 0);
        assert_eq!(layout.count(), 0);
    }

    #[test]
    fn gpt_supports_valid_extents_beyond_u32_lba() {
        let mut entries = [0u8; 128];
        let first = u32::MAX as u64 + 4_096;
        write_basic_data_entry(&mut entries, first, first + 65_535);
        let header = gpt_header(1, 128, crc32(&entries));
        let mut layout = PartitionLayout::new();

        assert_eq!(
            scan_gpt(&header, &entries, &mut layout),
            PartitionScanResult::Ok
        );
        assert_eq!(layout.candidates()[0].first_lba, first);
        assert_eq!(layout.candidates()[0].sector_count, NonZeroU64::new(65_536));
    }

    #[test]
    fn gpt_header_crc_mismatch_fails_closed() {
        let mut header = gpt_header(1, 128, 0);
        header[24] ^= 0x01;
        assert_eq!(parse_gpt_header(&header), None);
    }

    #[test]
    fn gpt_entry_array_crc_mismatch_fails_closed() {
        let mut entries = [0u8; 128];
        write_basic_data_entry(&mut entries, 2_048, 4_095);
        let header = gpt_header(1, 128, crc32(&entries));
        entries[64] ^= 0x80;

        let mut layout = PartitionLayout::new();
        assert_eq!(
            scan_gpt(&header, &entries, &mut layout),
            PartitionScanResult::Invalid
        );
        assert_eq!(layout.count(), 0);
    }

    #[test]
    fn gpt_stream_crc_detects_corruption_across_chunk_boundaries() {
        let mut bytes = [0u8; 256];
        write_basic_data_entry(&mut bytes[128..], 8_192, 16_383);
        let info = GptTableInfo {
            entries_lba: 2,
            entry_count: 2,
            entry_size: 128,
            entry_array_crc32: crc32(&bytes),
        };
        let mut stream = GptEntryStream::new(info).unwrap();
        let mut layout = PartitionLayout::new();

        assert_eq!(stream.push(&bytes[..97], &mut layout), 0);
        let mut remainder = [0u8; 159];
        remainder.copy_from_slice(&bytes[97..]);
        remainder[31] ^= 0x01;
        assert_eq!(stream.push(&remainder, &mut layout), 2);
        assert!(stream.is_complete());
        assert!(!stream.is_valid_complete());
    }

    #[test]
    fn malformed_gpt_and_short_mbr_fail_closed_without_panicking() {
        let mut layout = PartitionLayout::new();
        assert_eq!(
            scan_gpt(&[0u8; 512], &[0u8; 512], &mut layout),
            PartitionScanResult::Invalid
        );
        assert_eq!(
            scan_mbr_or_superfloppy(&[0u8; 511], &mut layout),
            PartitionScanResult::Invalid
        );
    }

    #[test]
    fn duplicate_candidate_upgrades_kind_without_consuming_slot() {
        let mut layout = PartitionLayout::new();
        assert!(layout.append_candidate(8_192, NonZeroU64::new(262_144), VolumeKind::Unknown));
        assert!(layout.append_candidate(8_192, NonZeroU64::new(262_144), VolumeKind::ExFat));
        assert_eq!(layout.count(), 1);
        assert_eq!(layout.candidates()[0].kind, VolumeKind::ExFat);
    }
}
