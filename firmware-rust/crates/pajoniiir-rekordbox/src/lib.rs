#![no_std]
#![forbid(unsafe_code)]

#[cfg(feature = "analysis")]
pub mod anlz_analysis;
pub mod anlz_hot_cues;

#[cfg(test)]
use pajoniiir_media_catalog::CATALOG_KEY_MAX;
use pajoniiir_media_catalog::{CatalogText, CatalogTextError, CatalogTrack};
use pajoniiir_media_identity::{IdentityError, VolumeIdentity, derive_track_id};

pub const REKORDBOX_EXPORT_PDB_PATH: &str = "/PIONEER/rekordbox/export.pdb";

pub const PDB_MAX_TABLES: usize = 32;
pub const PDB_MAX_PAGE_SIZE: u32 = 65_536;
pub const PDB_PAGE_HEAP_OFFSET: usize = 0x28;

pub const PDB_TABLE_TRACKS: u32 = 0x00;
pub const PDB_TABLE_ARTISTS: u32 = 0x02;
pub const PDB_TABLE_ALBUMS: u32 = 0x03;
pub const PDB_TABLE_KEYS: u32 = 0x05;

const PDB_HEADER_MIN: usize = 28;
const PDB_TABLE_DIRECTORY_OFFSET: usize = 0x1c;
const PDB_TABLE_ENTRY_SIZE: usize = 16;
const PDB_PAGE_NROWS_OFFSET: usize = 0x18;
const PDB_PAGE_NEXT_OFFSET: usize = 0x0c;

const TRACK_ROW_SUBTYPE: u16 = 0x0024;
const TRACK_OFF_KEY_ID: usize = 0x20;
const TRACK_OFF_TEMPO: usize = 0x38;
const TRACK_OFF_ALBUM_ID: usize = 0x40;
const TRACK_OFF_ARTIST_ID: usize = 0x44;
const TRACK_OFF_TRACK_ID: usize = 0x48;
const TRACK_OFF_DURATION: usize = 0x54;
const TRACK_OFF_STR_OFFS: usize = 0x5e;
const TRACK_STR_COUNT: usize = 21;
const TRACK_ROW_MIN_SIZE: usize = TRACK_OFF_STR_OFFS + TRACK_STR_COUNT * 2;

// Track string slots follow Deep Symmetry's export.pdb layout and stable M2.5
// `rekordbox_pdb.c` (20f1c3f0): 17 is the title, 18 is an unrelated slot.
const STR_IDX_ANLZ_PATH: usize = 14;
const STR_IDX_TITLE: usize = 17;
const STR_IDX_FILENAME: usize = 19;
const STR_IDX_FILE_PATH: usize = 20;

pub const PDB_TITLE_MAX: usize = 128;
pub const PDB_NAME_MAX: usize = 96;
pub const PDB_PATH_MAX: usize = 256;
pub const PDB_MAX_NAMES: usize = 512;
pub const PDB_TRACK_ROW_HEADER_BYTES: usize = TRACK_ROW_MIN_SIZE;

const NAME_ROW_ID_OFFSET: usize = 4;
const NAME_ROW_TEXT_OFFSET: u16 = 10;
const NAME_ROW_MIN_BYTES: usize = 14;
const KEY_ROW_ID_OFFSET: usize = 0;
const KEY_ROW_TEXT_OFFSET: u16 = 8;
const KEY_ROW_MIN_BYTES: usize = 9;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdbError {
    HeaderTooShort,
    FileTooShort,
    PageSizeOutOfRange(u32),
    PageLargerThanFile,
    TooManyTables(u32),
    TooManyPages,
    PageBufferTooSmall { required: usize, actual: usize },
    TableDirectoryOutOfRange,
    TablePageOutOfRange(u32),
    SlotBatchPlanMismatch,
    DeviceSqlOffsetOutOfRange,
    DeviceSqlTruncated,
    DeviceSqlInvalidLength,
    DeviceSqlInvalidUtf16,
    TrackRowOutOfRange,
    TrackStringOffsetOutOfRange,
    NameRowOutOfRange,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdbHeader {
    pub page_size: u32,
    pub num_tables: u8,
    pub total_pages: u32,
}

impl PdbHeader {
    pub const fn table_directory_bytes(self) -> usize {
        PDB_TABLE_DIRECTORY_OFFSET + self.num_tables as usize * PDB_TABLE_ENTRY_SIZE
    }
}

pub fn parse_pdb_header(header: &[u8], file_len: u64) -> Result<PdbHeader, PdbError> {
    if header.len() < PDB_HEADER_MIN {
        return Err(PdbError::HeaderTooShort);
    }
    if file_len < PDB_HEADER_MIN as u64 {
        return Err(PdbError::FileTooShort);
    }

    let page_size = read_le32(header, 4).ok_or(PdbError::HeaderTooShort)?;
    if !(PDB_PAGE_HEAP_OFFSET as u32..=PDB_MAX_PAGE_SIZE).contains(&page_size) {
        return Err(PdbError::PageSizeOutOfRange(page_size));
    }
    if u64::from(page_size) > file_len {
        return Err(PdbError::PageLargerThanFile);
    }

    let declared_tables = read_le32(header, 8).ok_or(PdbError::HeaderTooShort)?;
    if declared_tables as usize > PDB_MAX_TABLES {
        return Err(PdbError::TooManyTables(declared_tables));
    }

    let total_pages_u64 = file_len / u64::from(page_size);
    let total_pages = u32::try_from(total_pages_u64).map_err(|_| PdbError::TooManyPages)?;
    if total_pages == 0 {
        return Err(PdbError::PageLargerThanFile);
    }

    Ok(PdbHeader {
        page_size,
        num_tables: declared_tables as u8,
        total_pages,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdbTableEntry {
    pub kind: u32,
    pub empty_candidate: u32,
    pub first_page: u32,
    pub last_page: u32,
}

impl PdbTableEntry {
    const EMPTY: Self = Self {
        kind: 0,
        empty_candidate: 0,
        first_page: u32::MAX,
        last_page: u32::MAX,
    };
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdbTableDirectory {
    entries: [PdbTableEntry; PDB_MAX_TABLES],
    len: u8,
}

impl PdbTableDirectory {
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn entries(&self) -> &[PdbTableEntry] {
        &self.entries[..self.len()]
    }

    pub fn find(&self, kind: u32) -> Option<PdbTableEntry> {
        self.entries()
            .iter()
            .copied()
            .find(|entry| entry.kind == kind)
    }
}

pub fn parse_pdb_table_directory(
    first_page_prefix: &[u8],
    header: PdbHeader,
) -> Result<PdbTableDirectory, PdbError> {
    let page_size = header.page_size as usize;
    let required = header.table_directory_bytes();
    if required > page_size {
        return Err(PdbError::TableDirectoryOutOfRange);
    }
    if first_page_prefix.len() < required {
        return Err(PdbError::PageBufferTooSmall {
            required,
            actual: first_page_prefix.len(),
        });
    }

    let mut entries = [PdbTableEntry::EMPTY; PDB_MAX_TABLES];
    for (index, slot) in entries
        .iter_mut()
        .take(header.num_tables as usize)
        .enumerate()
    {
        let offset = PDB_TABLE_DIRECTORY_OFFSET + index * PDB_TABLE_ENTRY_SIZE;
        let entry = PdbTableEntry {
            kind: read_le32(first_page_prefix, offset).ok_or(PdbError::TableDirectoryOutOfRange)?,
            empty_candidate: read_le32(first_page_prefix, offset + 4)
                .ok_or(PdbError::TableDirectoryOutOfRange)?,
            first_page: read_le32(first_page_prefix, offset + 8)
                .ok_or(PdbError::TableDirectoryOutOfRange)?,
            last_page: read_le32(first_page_prefix, offset + 12)
                .ok_or(PdbError::TableDirectoryOutOfRange)?,
        };

        for page in [entry.first_page, entry.last_page] {
            if page != u32::MAX && page >= header.total_pages {
                return Err(PdbError::TablePageOutOfRange(page));
            }
        }
        *slot = entry;
    }

    Ok(PdbTableDirectory {
        entries,
        len: header.num_tables,
    })
}

pub fn pdb_page_next(page: &[u8], page_size: u32) -> Result<u32, PdbError> {
    require_page(page, page_size)?;
    read_le32(page, PDB_PAGE_NEXT_OFFSET).ok_or(PdbError::PageBufferTooSmall {
        required: page_size as usize,
        actual: page.len(),
    })
}

pub const PDB_PAGE_HEADER_BYTES: usize = PDB_PAGE_HEAP_OFFSET;
pub const PDB_ROW_SLOT_BATCH_ROWS: usize = 256;
pub const PDB_ROW_SLOT_BATCH_BYTES_MAX: usize = (PDB_ROW_SLOT_BATCH_ROWS / 16) * (4 + 16 * 2);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdbPageHeader {
    pub next_page: u32,
    pub declared_rows: u32,
}

pub fn parse_pdb_page_header(
    page_prefix: &[u8],
    page_size: u32,
) -> Result<PdbPageHeader, PdbError> {
    if !(PDB_PAGE_HEAP_OFFSET as u32..=PDB_MAX_PAGE_SIZE).contains(&page_size) {
        return Err(PdbError::PageSizeOutOfRange(page_size));
    }
    if page_prefix.len() < PDB_PAGE_HEADER_BYTES {
        return Err(PdbError::PageBufferTooSmall {
            required: PDB_PAGE_HEADER_BYTES,
            actual: page_prefix.len(),
        });
    }

    Ok(PdbPageHeader {
        next_page: read_le32(page_prefix, PDB_PAGE_NEXT_OFFSET)
            .ok_or(PdbError::TrackRowOutOfRange)?,
        declared_rows: read_le32(page_prefix, PDB_PAGE_NROWS_OFFSET)
            .ok_or(PdbError::TrackRowOutOfRange)?
            & 0x1fff,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdbSlotBatchPlan {
    pub offset: u32,
    pub len: u16,
    pub row_count: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdbRowOffsetBatch {
    offsets: [u16; PDB_ROW_SLOT_BATCH_ROWS],
    len: u16,
    truncated: bool,
}

impl PdbRowOffsetBatch {
    const fn new() -> Self {
        Self {
            offsets: [0; PDB_ROW_SLOT_BATCH_ROWS],
            len: 0,
            truncated: false,
        }
    }

    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub const fn was_truncated(&self) -> bool {
        self.truncated
    }

    pub fn offsets(&self) -> &[u16] {
        &self.offsets[..self.len()]
    }

    fn push(&mut self, offset: u16) {
        let index = self.len();
        self.offsets[index] = offset;
        self.len += 1;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdbPageSlotWalker {
    page_size: u32,
    declared_rows: u32,
    processed_rows: u32,
    cursor: u32,
    present_rows: u32,
    truncated: bool,
}

impl PdbPageSlotWalker {
    pub fn new(page_size: u32, declared_rows: u32) -> Result<Self, PdbError> {
        if !(PDB_PAGE_HEAP_OFFSET as u32..=PDB_MAX_PAGE_SIZE).contains(&page_size) {
            return Err(PdbError::PageSizeOutOfRange(page_size));
        }
        if declared_rows > 0x1fff {
            return Err(PdbError::TrackRowOutOfRange);
        }

        Ok(Self {
            page_size,
            declared_rows,
            processed_rows: 0,
            cursor: page_size,
            present_rows: 0,
            truncated: false,
        })
    }

    pub const fn is_complete(&self) -> bool {
        self.processed_rows >= self.declared_rows
    }

    pub const fn processed_rows(&self) -> u32 {
        self.processed_rows
    }

    pub const fn summary(&self) -> PdbPageWalk {
        PdbPageWalk {
            declared_rows: self.declared_rows,
            present_rows: self.present_rows,
            truncated: self.truncated,
        }
    }

    pub fn next_batch_plan(&self) -> Option<PdbSlotBatchPlan> {
        if self.is_complete() {
            return None;
        }

        let row_count =
            (self.declared_rows - self.processed_rows).min(PDB_ROW_SLOT_BATCH_ROWS as u32) as usize;
        let full_groups = row_count / 16;
        let tail_rows = row_count % 16;
        let mut bytes = full_groups * (4 + 16 * 2);
        if tail_rows != 0 {
            bytes += 4 + tail_rows * 2;
        }

        let offset = self.cursor.checked_sub(bytes as u32)?;
        Some(PdbSlotBatchPlan {
            offset,
            len: bytes as u16,
            row_count: row_count as u16,
        })
    }

    pub fn ingest_batch(
        &mut self,
        plan: PdbSlotBatchPlan,
        data: &[u8],
    ) -> Result<PdbRowOffsetBatch, PdbError> {
        if self.next_batch_plan() != Some(plan) {
            return Err(PdbError::SlotBatchPlanMismatch);
        }

        let required = usize::from(plan.len);
        if data.len() < required {
            return Err(PdbError::PageBufferTooSmall {
                required,
                actual: data.len(),
            });
        }

        let mut result = PdbRowOffsetBatch::new();
        let mut cursor = required;
        let mut consumed_rows = 0usize;
        let planned_rows = usize::from(plan.row_count);

        while consumed_rows < planned_rows {
            let group_count = (planned_rows - consumed_rows).min(16);
            let group_bytes = 4 + group_count * 2;
            if cursor < group_bytes {
                return Err(PdbError::SlotBatchPlanMismatch);
            }

            let row_present = read_le16(data, cursor - 4).ok_or(PdbError::SlotBatchPlanMismatch)?;
            for slot_index in 0..group_count {
                if row_present & (1u16 << slot_index) == 0 {
                    continue;
                }

                let slot = cursor - 4 - 2 * (slot_index + 1);
                let heap_offset = read_le16(data, slot).ok_or(PdbError::SlotBatchPlanMismatch)?;
                let absolute = PDB_PAGE_HEAP_OFFSET + usize::from(heap_offset);
                if absolute >= self.page_size as usize {
                    result.truncated = true;
                    self.truncated = true;
                    continue;
                }

                result.push(heap_offset);
                self.present_rows = self.present_rows.saturating_add(1);
            }

            cursor -= group_bytes;
            consumed_rows += group_count;
        }

        if cursor != 0 {
            return Err(PdbError::SlotBatchPlanMismatch);
        }

        self.cursor = plan.offset;
        self.processed_rows += u32::from(plan.row_count);
        Ok(result)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdbPageWalk {
    pub declared_rows: u32,
    pub present_rows: u32,
    pub truncated: bool,
}

pub fn visit_pdb_page_rows(
    page: &[u8],
    page_size: u32,
    mut visitor: impl FnMut(u16) -> bool,
) -> Result<PdbPageWalk, PdbError> {
    require_page(page, page_size)?;
    let size = page_size as usize;
    let declared_rows =
        read_le32(page, PDB_PAGE_NROWS_OFFSET).ok_or(PdbError::PageBufferTooSmall {
            required: size,
            actual: page.len(),
        })? & 0x1fff;

    if declared_rows == 0 {
        return Ok(PdbPageWalk {
            declared_rows: 0,
            present_rows: 0,
            truncated: false,
        });
    }

    let mut cursor = size;
    let mut index = 0u32;
    let mut present_rows = 0u32;
    let mut truncated = false;

    while index < declared_rows {
        let group_count = (declared_rows - index).min(16) as usize;
        let group_bytes = 4usize
            .checked_add(group_count * 2)
            .ok_or(PdbError::TrackRowOutOfRange)?;
        if cursor < group_bytes {
            truncated = true;
            break;
        }

        let row_present = read_le16(page, cursor - 4).ok_or(PdbError::PageBufferTooSmall {
            required: size,
            actual: page.len(),
        })?;

        for slot_index in 0..group_count {
            let slot = cursor - 4 - 2 * (slot_index + 1);
            let heap_offset = read_le16(page, slot).ok_or(PdbError::PageBufferTooSmall {
                required: size,
                actual: page.len(),
            })?;
            if row_present & (1u16 << slot_index) == 0 {
                continue;
            }

            let absolute = PDB_PAGE_HEAP_OFFSET + heap_offset as usize;
            if absolute >= size {
                truncated = true;
                continue;
            }

            present_rows = present_rows.saturating_add(1);
            if !visitor(heap_offset) {
                return Ok(PdbPageWalk {
                    declared_rows,
                    present_rows,
                    truncated,
                });
            }
        }

        cursor -= group_bytes;
        index += group_count as u32;
    }

    Ok(PdbPageWalk {
        declared_rows,
        present_rows,
        truncated,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceSqlDecode {
    pub consumed: usize,
    pub written: usize,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceSqlFieldHeader {
    pub flag: u8,
    pub total_len: u16,
    pub header_len: u8,
    pub wide: bool,
    pub little_endian: bool,
}

impl DeviceSqlFieldHeader {
    pub const fn total_len_usize(self) -> usize {
        self.total_len as usize
    }
}

pub fn parse_devicesql_field_header(
    data: &[u8],
    offset: usize,
) -> Result<DeviceSqlFieldHeader, PdbError> {
    let flag = *data
        .get(offset)
        .ok_or(PdbError::DeviceSqlOffsetOutOfRange)?;

    if flag == 0 || flag == 0x40 {
        return Ok(DeviceSqlFieldHeader {
            flag,
            total_len: 1,
            header_len: 1,
            wide: false,
            little_endian: false,
        });
    }

    if flag & 1 != 0 {
        let total = u16::from(flag >> 1);
        if total == 0 {
            return Err(PdbError::DeviceSqlInvalidLength);
        }
        return Ok(DeviceSqlFieldHeader {
            flag,
            total_len: total,
            header_len: 1,
            wide: false,
            little_endian: false,
        });
    }

    let prefix_end = offset.checked_add(4).ok_or(PdbError::DeviceSqlTruncated)?;
    if prefix_end > data.len() {
        return Err(PdbError::DeviceSqlTruncated);
    }

    let total = read_le16(data, offset + 1).ok_or(PdbError::DeviceSqlTruncated)?;
    if total < 4 {
        return Err(PdbError::DeviceSqlInvalidLength);
    }

    Ok(DeviceSqlFieldHeader {
        flag,
        total_len: total,
        header_len: 4,
        wide: flag & 0x10 != 0,
        little_endian: flag & 0x80 != 0,
    })
}

pub fn decode_devicesql(
    data: &[u8],
    offset: usize,
    output: &mut [u8],
) -> Result<DeviceSqlDecode, PdbError> {
    let flag = *data
        .get(offset)
        .ok_or(PdbError::DeviceSqlOffsetOutOfRange)?;

    if flag == 0 || flag == 0x40 {
        return Ok(DeviceSqlDecode {
            consumed: 1,
            written: 0,
            truncated: false,
        });
    }

    if flag & 1 != 0 {
        let total = usize::from(flag >> 1);
        if total == 0 {
            return Err(PdbError::DeviceSqlInvalidLength);
        }
        let end = offset
            .checked_add(total)
            .ok_or(PdbError::DeviceSqlTruncated)?;
        if end > data.len() {
            return Err(PdbError::DeviceSqlTruncated);
        }

        let source = &data[offset + 1..end];
        let written = source.len().min(output.len());
        output[..written].copy_from_slice(&source[..written]);
        return Ok(DeviceSqlDecode {
            consumed: total,
            written,
            truncated: written < source.len(),
        });
    }

    let length_end = offset.checked_add(3).ok_or(PdbError::DeviceSqlTruncated)?;
    if length_end >= data.len() {
        return Err(PdbError::DeviceSqlTruncated);
    }

    let total = usize::from(read_le16(data, offset + 1).ok_or(PdbError::DeviceSqlTruncated)?);
    if total < 4 {
        return Err(PdbError::DeviceSqlInvalidLength);
    }
    let end = offset
        .checked_add(total)
        .ok_or(PdbError::DeviceSqlTruncated)?;
    if end > data.len() {
        return Err(PdbError::DeviceSqlTruncated);
    }

    let payload = &data[offset + 4..end];
    let wide = flag & 0x10 != 0;
    let little_endian = flag & 0x80 != 0;

    if !wide {
        let written = payload.len().min(output.len());
        output[..written].copy_from_slice(&payload[..written]);
        if written > 0 && output[written - 1] == 0 {
            return Ok(DeviceSqlDecode {
                consumed: total,
                written: output[..written]
                    .iter()
                    .rposition(|byte| *byte != 0)
                    .map_or(0, |index| index + 1),
                truncated: written < payload.len(),
            });
        }
        return Ok(DeviceSqlDecode {
            consumed: total,
            written,
            truncated: written < payload.len(),
        });
    }

    if !payload.len().is_multiple_of(2) {
        return Err(PdbError::DeviceSqlInvalidUtf16);
    }

    let mut input_index = 0usize;
    let mut written = 0usize;
    let mut truncated = false;

    while input_index < payload.len() {
        let first = read_utf16(payload, input_index, little_endian)?;
        input_index += 2;
        if first == 0 {
            break;
        }

        let codepoint = if (0xd800..=0xdbff).contains(&first) {
            if input_index >= payload.len() {
                return Err(PdbError::DeviceSqlInvalidUtf16);
            }
            let low = read_utf16(payload, input_index, little_endian)?;
            if !(0xdc00..=0xdfff).contains(&low) {
                return Err(PdbError::DeviceSqlInvalidUtf16);
            }
            input_index += 2;
            0x1_0000 + (((u32::from(first) - 0xd800) << 10) | (u32::from(low) - 0xdc00))
        } else if (0xdc00..=0xdfff).contains(&first) {
            return Err(PdbError::DeviceSqlInvalidUtf16);
        } else {
            u32::from(first)
        };

        let mut character =
            core::char::from_u32(codepoint).ok_or(PdbError::DeviceSqlInvalidUtf16)?;
        if character == '\\' {
            character = '/';
        }

        let mut encoded = [0u8; 4];
        let text = character.encode_utf8(&mut encoded).as_bytes();
        let Some(next_written) = written.checked_add(text.len()) else {
            return Err(PdbError::DeviceSqlInvalidLength);
        };
        if next_written > output.len() {
            truncated = true;
            break;
        }
        output[written..next_written].copy_from_slice(text);
        written = next_written;
    }

    Ok(DeviceSqlDecode {
        consumed: total,
        written,
        truncated,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdbText<const N: usize> {
    bytes: [u8; N],
    len: u16,
    truncated: bool,
}

impl<const N: usize> PdbText<N> {
    pub const fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
            truncated: false,
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    pub fn as_str(&self) -> Option<&str> {
        core::str::from_utf8(self.as_bytes()).ok()
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub const fn was_truncated(&self) -> bool {
        self.truncated
    }

    pub fn from_devicesql_field(data: &[u8]) -> Result<Self, PdbError> {
        let mut text = Self::new();
        text.decode_from(data, 0)?;
        Ok(text)
    }

    fn decode_from(&mut self, data: &[u8], offset: usize) -> Result<(), PdbError> {
        self.bytes.fill(0);
        let decoded = decode_devicesql(data, offset, &mut self.bytes)?;
        self.len = u16::try_from(decoded.written).map_err(|_| PdbError::DeviceSqlInvalidLength)?;
        self.truncated = decoded.truncated;
        Ok(())
    }

    fn copy_from(&mut self, other: &Self) {
        self.bytes = other.bytes;
        self.len = other.len;
        self.truncated = other.truncated;
    }
}

impl<const N: usize> Default for PdbText<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdbNameRowLayout {
    ArtistOrAlbum,
    Key,
}

impl PdbNameRowLayout {
    pub const fn minimum_bytes(self) -> usize {
        match self {
            Self::ArtistOrAlbum => NAME_ROW_MIN_BYTES,
            Self::Key => KEY_ROW_MIN_BYTES,
        }
    }

    pub const fn id_offset(self) -> usize {
        match self {
            Self::ArtistOrAlbum => NAME_ROW_ID_OFFSET,
            Self::Key => KEY_ROW_ID_OFFSET,
        }
    }

    pub const fn text_relative_offset(self) -> u16 {
        match self {
            Self::ArtistOrAlbum => NAME_ROW_TEXT_OFFSET,
            Self::Key => KEY_ROW_TEXT_OFFSET,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdbNameRowHeader {
    pub id: u32,
    pub text_relative_offset: u16,
}

pub fn parse_pdb_name_row_header(
    row_prefix: &[u8],
    layout: PdbNameRowLayout,
) -> Result<Option<PdbNameRowHeader>, PdbError> {
    if row_prefix.len() < layout.minimum_bytes() {
        return Err(PdbError::NameRowOutOfRange);
    }

    let id = read_le32(row_prefix, layout.id_offset()).ok_or(PdbError::NameRowOutOfRange)?;
    if id == 0 {
        return Ok(None);
    }

    Ok(Some(PdbNameRowHeader {
        id,
        text_relative_offset: layout.text_relative_offset(),
    }))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdbNameEntry {
    pub id: u32,
    pub name: PdbText<PDB_NAME_MAX>,
}

impl PdbNameEntry {
    const EMPTY: Self = Self {
        id: 0,
        name: PdbText::new(),
    };

    pub fn from_header_and_field(
        header: PdbNameRowHeader,
        name: PdbText<PDB_NAME_MAX>,
    ) -> Option<Self> {
        if name.is_empty() {
            None
        } else {
            Some(Self {
                id: header.id,
                name,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdbNameMapError {
    Full { capacity: usize },
    DuplicateId(u32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdbNameMap<const N: usize> {
    entries: [PdbNameEntry; N],
    len: usize,
}

impl<const N: usize> PdbNameMap<N> {
    pub const fn new() -> Self {
        Self {
            entries: [PdbNameEntry::EMPTY; N],
            len: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn capacity(&self) -> usize {
        N
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn entries(&self) -> &[PdbNameEntry] {
        &self.entries[..self.len]
    }

    pub fn push(&mut self, entry: PdbNameEntry) -> Result<usize, PdbNameMapError> {
        if self
            .entries()
            .iter()
            .any(|existing| existing.id == entry.id)
        {
            return Err(PdbNameMapError::DuplicateId(entry.id));
        }
        if self.len == N {
            return Err(PdbNameMapError::Full { capacity: N });
        }

        let index = self.len;
        self.entries[index] = entry;
        self.len += 1;
        Ok(index)
    }

    pub fn lookup(&self, id: u32) -> Option<&PdbText<PDB_NAME_MAX>> {
        if id == 0 {
            return None;
        }
        self.entries()
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| &entry.name)
    }
}

impl<const N: usize> Default for PdbNameMap<N> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait PdbNameLookup {
    fn lookup(&self, id: u32) -> Option<&PdbText<PDB_NAME_MAX>>;
}

impl<const N: usize> PdbNameLookup for PdbNameMap<N> {
    fn lookup(&self, id: u32) -> Option<&PdbText<PDB_NAME_MAX>> {
        PdbNameMap::lookup(self, id)
    }
}

pub const PDB_NAME_MAP_BUDGET_BYTES: usize = 64 * 1_024;
pub const PDB_NAME_STAGE_BUDGET_BYTES: usize = 256 * 1_024;
pub const PDB_NAME_MAP_BYTES: usize = core::mem::size_of::<PdbNameMap<PDB_MAX_NAMES>>();
pub const PDB_NAME_STAGE_BYTES: usize = PDB_NAME_MAP_BYTES * 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdbTrackStringOffsets {
    pub title: u16,
    pub filename: u16,
    pub file_path: u16,
    pub anlz_path: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdbTrackRowHeader {
    pub track_id: u32,
    pub bpm_x100: u32,
    pub bpm: u16,
    pub duration_s: u16,
    pub artist_id: u32,
    pub album_id: u32,
    pub key_id: u32,
    pub strings: PdbTrackStringOffsets,
}

pub fn parse_pdb_track_row_header(
    row_prefix: &[u8],
) -> Result<Option<PdbTrackRowHeader>, PdbError> {
    if row_prefix.len() < PDB_TRACK_ROW_HEADER_BYTES {
        return Err(PdbError::TrackRowOutOfRange);
    }

    let subtype = read_le16(row_prefix, 0).ok_or(PdbError::TrackRowOutOfRange)?;
    if subtype != TRACK_ROW_SUBTYPE {
        return Ok(None);
    }

    let bpm_x100 = read_le32(row_prefix, TRACK_OFF_TEMPO).ok_or(PdbError::TrackRowOutOfRange)?;
    let rounded_bpm = (u64::from(bpm_x100) + 50) / 100;

    let string_offset = |index: usize| {
        read_le16(row_prefix, TRACK_OFF_STR_OFFS + index * 2)
            .ok_or(PdbError::TrackStringOffsetOutOfRange)
    };

    Ok(Some(PdbTrackRowHeader {
        track_id: read_le32(row_prefix, TRACK_OFF_TRACK_ID).ok_or(PdbError::TrackRowOutOfRange)?,
        bpm_x100,
        bpm: u16::try_from(rounded_bpm).unwrap_or(u16::MAX),
        duration_s: read_le16(row_prefix, TRACK_OFF_DURATION)
            .ok_or(PdbError::TrackRowOutOfRange)?,
        artist_id: read_le32(row_prefix, TRACK_OFF_ARTIST_ID)
            .ok_or(PdbError::TrackRowOutOfRange)?,
        album_id: read_le32(row_prefix, TRACK_OFF_ALBUM_ID).ok_or(PdbError::TrackRowOutOfRange)?,
        key_id: read_le32(row_prefix, TRACK_OFF_KEY_ID).ok_or(PdbError::TrackRowOutOfRange)?,
        strings: PdbTrackStringOffsets {
            title: string_offset(STR_IDX_TITLE)?,
            filename: string_offset(STR_IDX_FILENAME)?,
            file_path: string_offset(STR_IDX_FILE_PATH)?,
            anlz_path: string_offset(STR_IDX_ANLZ_PATH)?,
        },
    }))
}

pub fn pdb_track_string_page_offset(
    heap_offset: u16,
    relative_offset: u16,
    page_size: u32,
) -> Result<Option<u32>, PdbError> {
    if relative_offset == 0 {
        return Ok(None);
    }
    if !(PDB_PAGE_HEAP_OFFSET as u32..=PDB_MAX_PAGE_SIZE).contains(&page_size) {
        return Err(PdbError::PageSizeOutOfRange(page_size));
    }

    let absolute = (PDB_PAGE_HEAP_OFFSET as u32)
        .checked_add(u32::from(heap_offset))
        .and_then(|offset| offset.checked_add(u32::from(relative_offset)))
        .ok_or(PdbError::TrackStringOffsetOutOfRange)?;
    if absolute >= page_size {
        return Err(PdbError::TrackStringOffsetOutOfRange);
    }

    Ok(Some(absolute))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdbTrackRow {
    pub track_id: u32,
    pub bpm_x100: u32,
    pub bpm: u16,
    pub duration_s: u16,
    pub artist_id: u32,
    pub album_id: u32,
    pub key_id: u32,
    pub title: PdbText<PDB_TITLE_MAX>,
    pub file_path: PdbText<PDB_PATH_MAX>,
    pub anlz_path: PdbText<PDB_PATH_MAX>,
}

impl PdbTrackRow {
    pub fn from_header_and_fields(
        header: PdbTrackRowHeader,
        mut title: PdbText<PDB_TITLE_MAX>,
        filename: PdbText<PDB_TITLE_MAX>,
        file_path: PdbText<PDB_PATH_MAX>,
        anlz_path: PdbText<PDB_PATH_MAX>,
    ) -> Self {
        if title.is_empty() || title.as_str().is_none() {
            title.copy_from(&filename);
        }

        Self {
            track_id: header.track_id,
            bpm_x100: header.bpm_x100,
            bpm: header.bpm,
            duration_s: header.duration_s,
            artist_id: header.artist_id,
            album_id: header.album_id,
            key_id: header.key_id,
            title,
            file_path,
            anlz_path,
        }
    }
}

/// Title policy shared by the page and streamed readers (M2.5 parity): a
/// title whose offset or DeviceSQL encoding is malformed is treated as missing,
/// so the row falls back to its filename. Path and name fields stay strict.
pub fn recover_pdb_title<const N: usize>(decoded: Result<PdbText<N>, PdbError>) -> PdbText<N> {
    decoded.unwrap_or_else(|_| PdbText::new())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdbResolvedNames<'a> {
    pub artist: Option<&'a PdbText<PDB_NAME_MAX>>,
    pub album: Option<&'a PdbText<PDB_NAME_MAX>>,
    pub key: Option<&'a PdbText<PDB_NAME_MAX>>,
}

pub fn resolve_pdb_track_names<'a, A, B, K>(
    track: &PdbTrackRow,
    artists: &'a A,
    albums: &'a B,
    keys: &'a K,
) -> PdbResolvedNames<'a>
where
    A: PdbNameLookup + ?Sized,
    B: PdbNameLookup + ?Sized,
    K: PdbNameLookup + ?Sized,
{
    PdbResolvedNames {
        artist: artists.lookup(track.artist_id),
        album: albums.lookup(track.album_id),
        key: keys.lookup(track.key_id),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdbCatalogField {
    Title,
    Artist,
    Album,
    Key,
    AudioPath,
    AnalysisPath,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdbCatalogError {
    InvalidUtf8(PdbCatalogField),
    Truncated(PdbCatalogField),
    Text {
        field: PdbCatalogField,
        error: CatalogTextError,
    },
    Identity(IdentityError),
}

pub fn catalog_track_from_pdb(
    volume: VolumeIdentity,
    track: &PdbTrackRow,
    names: PdbResolvedNames<'_>,
) -> Result<CatalogTrack, PdbCatalogError> {
    if track.file_path.was_truncated() {
        return Err(PdbCatalogError::Truncated(PdbCatalogField::AudioPath));
    }
    let audio_path_text = track
        .file_path
        .as_str()
        .ok_or(PdbCatalogError::InvalidUtf8(PdbCatalogField::AudioPath))?;
    let track_id = derive_track_id(volume, audio_path_text).map_err(PdbCatalogError::Identity)?;

    Ok(CatalogTrack {
        track_id,
        source_track_id: track.track_id,
        bpm_x100: track.bpm_x100,
        duration_ms: u32::from(track.duration_s) * 1_000,
        title: catalog_text(PdbCatalogField::Title, &track.title)?,
        artist: optional_catalog_text(PdbCatalogField::Artist, names.artist)?,
        album: optional_catalog_text(PdbCatalogField::Album, names.album)?,
        key: optional_catalog_text(PdbCatalogField::Key, names.key)?,
        audio_path: catalog_text(PdbCatalogField::AudioPath, &track.file_path)?,
        analysis_path: catalog_text(PdbCatalogField::AnalysisPath, &track.anlz_path)?,
    })
}

fn catalog_text<const S: usize, const D: usize>(
    field: PdbCatalogField,
    source: &PdbText<S>,
) -> Result<CatalogText<D>, PdbCatalogError> {
    if source.was_truncated() {
        return Err(PdbCatalogError::Truncated(field));
    }
    let value = source.as_str().ok_or(PdbCatalogError::InvalidUtf8(field))?;
    CatalogText::try_from_str(value).map_err(|error| PdbCatalogError::Text { field, error })
}

fn optional_catalog_text<const D: usize>(
    field: PdbCatalogField,
    source: Option<&PdbText<PDB_NAME_MAX>>,
) -> Result<CatalogText<D>, PdbCatalogError> {
    match source {
        Some(source) => catalog_text(field, source),
        None => Ok(CatalogText::EMPTY),
    }
}

pub fn parse_pdb_track_row(
    page: &[u8],
    page_size: u32,
    heap_offset: u16,
) -> Result<Option<PdbTrackRow>, PdbError> {
    require_page(page, page_size)?;
    let size = page_size as usize;
    let row = PDB_PAGE_HEAP_OFFSET
        .checked_add(heap_offset as usize)
        .ok_or(PdbError::TrackRowOutOfRange)?;
    let minimum_end = row
        .checked_add(TRACK_ROW_MIN_SIZE)
        .ok_or(PdbError::TrackRowOutOfRange)?;
    if minimum_end > size {
        return Err(PdbError::TrackRowOutOfRange);
    }

    let Some(header) = parse_pdb_track_row_header(&page[row..size])? else {
        return Ok(None);
    };

    let mut title = PdbText::new();
    let mut filename = PdbText::new();
    let mut file_path = PdbText::new();
    let mut anlz_path = PdbText::new();

    let decoded_title = decode_track_string(page, size, row, STR_IDX_TITLE, &mut title);
    let title = recover_pdb_title(decoded_title.map(|()| title));
    decode_track_string(page, size, row, STR_IDX_FILENAME, &mut filename)?;
    decode_track_string(page, size, row, STR_IDX_FILE_PATH, &mut file_path)?;
    decode_track_string(page, size, row, STR_IDX_ANLZ_PATH, &mut anlz_path)?;

    Ok(Some(PdbTrackRow::from_header_and_fields(
        header, title, filename, file_path, anlz_path,
    )))
}

fn decode_track_string<const N: usize>(
    page: &[u8],
    page_size: usize,
    row: usize,
    string_index: usize,
    output: &mut PdbText<N>,
) -> Result<(), PdbError> {
    let slot = row
        .checked_add(TRACK_OFF_STR_OFFS + string_index * 2)
        .ok_or(PdbError::TrackStringOffsetOutOfRange)?;
    let relative = read_le16(page, slot).ok_or(PdbError::TrackStringOffsetOutOfRange)?;
    if relative == 0 {
        return Ok(());
    }

    let absolute = row
        .checked_add(relative as usize)
        .ok_or(PdbError::TrackStringOffsetOutOfRange)?;
    if absolute >= page_size {
        return Err(PdbError::TrackStringOffsetOutOfRange);
    }
    output.decode_from(&page[..page_size], absolute)
}

fn require_page(page: &[u8], page_size: u32) -> Result<(), PdbError> {
    let required = page_size as usize;
    if page_size < PDB_PAGE_HEAP_OFFSET as u32 || page_size > PDB_MAX_PAGE_SIZE {
        return Err(PdbError::PageSizeOutOfRange(page_size));
    }
    if page.len() < required {
        return Err(PdbError::PageBufferTooSmall {
            required,
            actual: page.len(),
        });
    }
    Ok(())
}

fn read_utf16(data: &[u8], offset: usize, little_endian: bool) -> Result<u16, PdbError> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or(PdbError::DeviceSqlTruncated)?;
    Ok(if little_endian {
        u16::from_le_bytes([bytes[0], bytes[1]])
    } else {
        u16::from_be_bytes([bytes[0], bytes[1]])
    })
}

fn read_le16(data: &[u8], offset: usize) -> Option<u16> {
    let bytes = data.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_le32(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = data.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_le16(data: &mut [u8], offset: usize, value: u16) {
        data[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_le32(data: &mut [u8], offset: usize, value: u32) {
        data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_short_devicesql(data: &mut [u8], offset: usize, text: &[u8]) {
        let total = text.len() + 1;
        assert!(total <= 0x7f);
        data[offset] = ((total as u8) << 1) | 1;
        data[offset + 1..offset + 1 + text.len()].copy_from_slice(text);
    }

    #[test]
    fn parses_bounded_header_and_table_directory() {
        let mut page = [0u8; 512];
        put_le32(&mut page, 4, 512);
        put_le32(&mut page, 8, 2);

        put_le32(&mut page, 0x1c, PDB_TABLE_TRACKS);
        put_le32(&mut page, 0x24, 1);
        put_le32(&mut page, 0x28, 3);

        put_le32(&mut page, 0x2c, PDB_TABLE_ARTISTS);
        put_le32(&mut page, 0x34, 2);
        put_le32(&mut page, 0x38, 3);

        let header = parse_pdb_header(&page[..28], 512 * 4).unwrap();
        assert_eq!(
            header,
            PdbHeader {
                page_size: 512,
                num_tables: 2,
                total_pages: 4,
            }
        );

        let tables = parse_pdb_table_directory(&page, header).unwrap();
        assert_eq!(tables.len(), 2);
        assert_eq!(tables.find(PDB_TABLE_TRACKS).unwrap().first_page, 1);
        assert_eq!(tables.find(PDB_TABLE_ARTISTS).unwrap().first_page, 2);
    }

    #[test]
    fn table_directory_parses_from_small_prefix_of_large_page() {
        let mut prefix = [0u8; 540];
        put_le32(&mut prefix, 4, PDB_MAX_PAGE_SIZE);
        put_le32(&mut prefix, 8, PDB_MAX_TABLES as u32);

        for index in 0..PDB_MAX_TABLES {
            let offset = PDB_TABLE_DIRECTORY_OFFSET + index * PDB_TABLE_ENTRY_SIZE;
            put_le32(&mut prefix, offset, index as u32);
            put_le32(&mut prefix, offset + 8, 1);
            put_le32(&mut prefix, offset + 12, 1);
        }

        let header =
            parse_pdb_header(&prefix[..PDB_HEADER_MIN], u64::from(PDB_MAX_PAGE_SIZE) * 2).unwrap();
        assert_eq!(header.table_directory_bytes(), prefix.len());

        let tables = parse_pdb_table_directory(&prefix, header).unwrap();
        assert_eq!(tables.len(), PDB_MAX_TABLES);
        assert_eq!(tables.find(31).unwrap().first_page, 1);
    }

    #[test]
    fn rejects_header_limits_before_allocating_page_storage() {
        let mut header = [0u8; 28];
        put_le32(&mut header, 4, PDB_MAX_PAGE_SIZE + 1);
        assert_eq!(
            parse_pdb_header(&header, u64::from(PDB_MAX_PAGE_SIZE + 1)),
            Err(PdbError::PageSizeOutOfRange(PDB_MAX_PAGE_SIZE + 1))
        );

        put_le32(&mut header, 4, 512);
        put_le32(&mut header, 8, PDB_MAX_TABLES as u32 + 1);
        assert_eq!(
            parse_pdb_header(&header, 1024),
            Err(PdbError::TooManyTables(PDB_MAX_TABLES as u32 + 1))
        );
    }

    #[test]
    fn parses_page_header_from_fixed_prefix() {
        let mut prefix = [0u8; PDB_PAGE_HEADER_BYTES];
        put_le32(&mut prefix, PDB_PAGE_NEXT_OFFSET, 9);
        put_le32(&mut prefix, PDB_PAGE_NROWS_OFFSET, 0x6000_0012);

        assert_eq!(
            parse_pdb_page_header(&prefix, 65_536).unwrap(),
            PdbPageHeader {
                next_page: 9,
                declared_rows: 18,
            }
        );
    }

    #[test]
    fn slot_walker_batches_large_pages_without_full_page_buffer() {
        let walker = PdbPageSlotWalker::new(65_536, 300).unwrap();
        let first = walker.next_batch_plan().unwrap();

        assert_eq!(first.row_count, 256);
        assert_eq!(usize::from(first.len), PDB_ROW_SLOT_BATCH_BYTES_MAX);
        assert_eq!(first.offset, 65_536 - PDB_ROW_SLOT_BATCH_BYTES_MAX as u32);
    }

    #[test]
    fn slot_walker_decodes_present_offsets_and_resumes() {
        let mut walker = PdbPageSlotWalker::new(512, 18).unwrap();
        let plan = walker.next_batch_plan().unwrap();
        assert_eq!(plan.row_count, 18);
        assert_eq!(plan.len, 44);

        let mut encoded = [0u8; 44];
        let mut cursor = encoded.len();

        put_le16(&mut encoded, cursor - 4, 0xffff);
        for index in 0..16 {
            put_le16(
                &mut encoded,
                cursor - 4 - 2 * (index + 1),
                100 + index as u16,
            );
        }
        cursor -= 36;

        put_le16(&mut encoded, cursor - 4, 0x0001);
        put_le16(&mut encoded, cursor - 6, 200);
        put_le16(&mut encoded, cursor - 8, 201);

        let batch = walker.ingest_batch(plan, &encoded).unwrap();
        assert_eq!(batch.len(), 17);
        for (index, offset) in batch.offsets()[..16].iter().copied().enumerate() {
            assert_eq!(offset, 100 + index as u16);
        }
        assert_eq!(batch.offsets()[16], 200);
        assert!(!batch.was_truncated());
        assert!(walker.is_complete());
        assert_eq!(
            walker.summary(),
            PdbPageWalk {
                declared_rows: 18,
                present_rows: 17,
                truncated: false,
            }
        );
    }

    #[test]
    fn malformed_page_zero_row_groups_truncate_without_underflow() {
        let mut page = [0u8; 64];
        put_le32(&mut page, PDB_PAGE_NROWS_OFFSET, 0x1fff);
        let mut visited = 0u32;

        let walk = visit_pdb_page_rows(&page, 64, |_| {
            visited += 1;
            true
        })
        .unwrap();

        assert!(walk.truncated);
        assert_eq!(visited, 0);
    }

    #[test]
    fn plans_devicesql_field_length_from_bounded_prefix() {
        let short_flag = ((6u8) << 1) | 1;
        assert_eq!(
            parse_devicesql_field_header(&[short_flag], 0).unwrap(),
            DeviceSqlFieldHeader {
                flag: short_flag,
                total_len: 6,
                header_len: 1,
                wide: false,
                little_endian: false,
            }
        );

        let long_prefix = [0x90, 0x24, 0x01, 0];
        assert_eq!(
            parse_devicesql_field_header(&long_prefix, 0).unwrap(),
            DeviceSqlFieldHeader {
                flag: 0x90,
                total_len: 0x124,
                header_len: 4,
                wide: true,
                little_endian: true,
            }
        );

        assert_eq!(
            parse_devicesql_field_header(&[0x90, 0x24, 0x01], 0),
            Err(PdbError::DeviceSqlTruncated)
        );
    }

    #[test]
    fn fixed_text_decodes_one_devicesql_field_without_allocation() {
        let mut raw = [0u8; 32];
        put_short_devicesql(&mut raw, 0, b"Track Name");

        let text = PdbText::<PDB_TITLE_MAX>::from_devicesql_field(&raw).unwrap();
        assert_eq!(text.as_str(), Some("Track Name"));
        assert!(!text.was_truncated());
    }

    #[test]
    fn released_name_maps_stay_inside_staging_memory_budget() {
        const {
            assert!(PDB_NAME_MAP_BYTES <= PDB_NAME_MAP_BUDGET_BYTES);
            assert!(PDB_NAME_STAGE_BYTES <= PDB_NAME_STAGE_BUDGET_BYTES);
        }
    }

    #[test]
    fn parses_artist_album_and_key_name_row_layouts() {
        let mut artist = [0u8; 32];
        put_le32(&mut artist, NAME_ROW_ID_OFFSET, 42);
        put_short_devicesql(
            &mut artist,
            usize::from(NAME_ROW_TEXT_OFFSET),
            b"Artist Name",
        );
        let artist_header = parse_pdb_name_row_header(&artist, PdbNameRowLayout::ArtistOrAlbum)
            .unwrap()
            .unwrap();
        assert_eq!(artist_header.id, 42);
        assert_eq!(artist_header.text_relative_offset, NAME_ROW_TEXT_OFFSET);
        let artist_name = PdbText::<PDB_NAME_MAX>::from_devicesql_field(
            &artist[usize::from(artist_header.text_relative_offset)..],
        )
        .unwrap();
        assert_eq!(artist_name.as_str(), Some("Artist Name"));

        let mut key = [0u8; 16];
        put_le32(&mut key, KEY_ROW_ID_OFFSET, 7);
        put_short_devicesql(&mut key, usize::from(KEY_ROW_TEXT_OFFSET), b"8A");
        let key_header = parse_pdb_name_row_header(&key, PdbNameRowLayout::Key)
            .unwrap()
            .unwrap();
        assert_eq!(key_header.id, 7);
        assert_eq!(key_header.text_relative_offset, KEY_ROW_TEXT_OFFSET);
        let key_name = PdbText::<PDB_NAME_MAX>::from_devicesql_field(
            &key[usize::from(key_header.text_relative_offset)..],
        )
        .unwrap();
        assert_eq!(key_name.as_str(), Some("8A"));

        put_le32(&mut key, KEY_ROW_ID_OFFSET, 0);
        assert_eq!(
            parse_pdb_name_row_header(&key, PdbNameRowLayout::Key).unwrap(),
            None
        );
    }

    #[test]
    fn bounded_name_map_rejects_duplicates_and_capacity_overflow() {
        let header = PdbNameRowHeader {
            id: 1,
            text_relative_offset: NAME_ROW_TEXT_OFFSET,
        };
        let mut raw = [0u8; 16];
        put_short_devicesql(&mut raw, 0, b"One");
        let one = PdbNameEntry::from_header_and_field(
            header,
            PdbText::<PDB_NAME_MAX>::from_devicesql_field(&raw).unwrap(),
        )
        .unwrap();

        let mut map = PdbNameMap::<1>::new();
        assert_eq!(map.push(one), Ok(0));
        assert_eq!(map.lookup(1).unwrap().as_str(), Some("One"));
        assert_eq!(map.push(one), Err(PdbNameMapError::DuplicateId(1)));

        let header_two = PdbNameRowHeader {
            id: 2,
            text_relative_offset: NAME_ROW_TEXT_OFFSET,
        };
        let two = PdbNameEntry::from_header_and_field(
            header_two,
            PdbText::<PDB_NAME_MAX>::from_devicesql_field(&raw).unwrap(),
        )
        .unwrap();
        assert_eq!(map.push(two), Err(PdbNameMapError::Full { capacity: 1 }));
    }

    #[test]
    fn track_row_assembly_uses_filename_when_title_is_empty() {
        let header = PdbTrackRowHeader {
            track_id: 9,
            bpm_x100: 12_800,
            bpm: 128,
            duration_s: 240,
            artist_id: 3,
            album_id: 4,
            key_id: 5,
            strings: PdbTrackStringOffsets {
                title: 0,
                filename: 0,
                file_path: 0,
                anlz_path: 0,
            },
        };
        let title = PdbText::<PDB_TITLE_MAX>::new();

        let mut filename_raw = [0u8; 32];
        put_short_devicesql(&mut filename_raw, 0, b"fallback.wav");
        let filename = PdbText::<PDB_TITLE_MAX>::from_devicesql_field(&filename_raw).unwrap();

        let row = PdbTrackRow::from_header_and_fields(
            header,
            title,
            filename,
            PdbText::new(),
            PdbText::new(),
        );
        assert_eq!(row.title.as_str(), Some("fallback.wav"));
        assert_eq!(row.track_id, 9);
    }

    #[test]
    fn resolved_pdb_track_converts_to_neutral_catalog_track() {
        fn field<const N: usize>(value: &[u8]) -> PdbText<N> {
            let mut raw = [0u8; 128];
            put_short_devicesql(&mut raw, 0, value);
            PdbText::from_devicesql_field(&raw).unwrap()
        }

        fn name(id: u32, value: &[u8]) -> PdbNameEntry {
            PdbNameEntry::from_header_and_field(
                PdbNameRowHeader {
                    id,
                    text_relative_offset: NAME_ROW_TEXT_OFFSET,
                },
                field(value),
            )
            .unwrap()
        }

        let header = PdbTrackRowHeader {
            track_id: 42,
            bpm_x100: 12_834,
            bpm: 128,
            duration_s: 321,
            artist_id: 7,
            album_id: 8,
            key_id: 9,
            strings: PdbTrackStringOffsets {
                title: 0,
                filename: 0,
                file_path: 0,
                anlz_path: 0,
            },
        };
        let track = PdbTrackRow::from_header_and_fields(
            header,
            field(b"Test Track"),
            PdbText::new(),
            field(b"/Contents/test.wav"),
            field(b"/PIONEER/USBANLZ/P001/00000042/ANLZ0000.DAT"),
        );

        let mut artists = PdbNameMap::<2>::new();
        let mut albums = PdbNameMap::<2>::new();
        let mut keys = PdbNameMap::<2>::new();
        artists.push(name(7, b"Test Artist")).unwrap();
        albums.push(name(8, b"Test Album")).unwrap();
        keys.push(name(9, b"8A")).unwrap();

        let volume = VolumeIdentity([0x31; 32]);
        let resolved = resolve_pdb_track_names(&track, &artists, &albums, &keys);
        let catalog = catalog_track_from_pdb(volume, &track, resolved).unwrap();

        assert_eq!(
            catalog.track_id,
            derive_track_id(volume, "/Contents/test.wav").unwrap()
        );
        assert_eq!(catalog.source_track_id, 42);
        assert_eq!(catalog.bpm_x100, 12_834);
        assert_eq!(catalog.duration_ms, 321_000);
        assert_eq!(catalog.title.as_str(), "Test Track");
        assert_eq!(catalog.artist.as_str(), "Test Artist");
        assert_eq!(catalog.album.as_str(), "Test Album");
        assert_eq!(catalog.key.as_str(), "8A");
        assert_eq!(catalog.audio_path.as_str(), "/Contents/test.wav");
        assert_eq!(
            catalog.analysis_path.as_str(),
            "/PIONEER/USBANLZ/P001/00000042/ANLZ0000.DAT"
        );
    }

    #[test]
    fn catalog_adapter_rejects_provider_side_truncation() {
        let mut raw = [0u8; 16];
        put_short_devicesql(&mut raw, 0, b"hello");
        let truncated = PdbText::<4>::from_devicesql_field(&raw).unwrap();
        assert!(truncated.was_truncated());

        assert_eq!(
            catalog_text::<4, 8>(PdbCatalogField::Title, &truncated),
            Err(PdbCatalogError::Truncated(PdbCatalogField::Title))
        );
    }

    #[test]
    fn catalog_adapter_rejects_metadata_that_exceeds_neutral_contract() {
        fn field<const N: usize>(value: &[u8]) -> PdbText<N> {
            let mut raw = [0u8; 128];
            put_short_devicesql(&mut raw, 0, value);
            PdbText::from_devicesql_field(&raw).unwrap()
        }

        let track = PdbTrackRow::from_header_and_fields(
            PdbTrackRowHeader {
                track_id: 1,
                bpm_x100: 12_000,
                bpm: 120,
                duration_s: 60,
                artist_id: 0,
                album_id: 0,
                key_id: 1,
                strings: PdbTrackStringOffsets {
                    title: 0,
                    filename: 0,
                    file_path: 0,
                    anlz_path: 0,
                },
            },
            field(b"Track"),
            PdbText::new(),
            field(b"/Contents/track.wav"),
            PdbText::new(),
        );
        let key = field::<PDB_NAME_MAX>(b"12345678901234567");
        let names = PdbResolvedNames {
            artist: None,
            album: None,
            key: Some(&key),
        };

        assert_eq!(
            catalog_track_from_pdb(VolumeIdentity([0x44; 32]), &track, names),
            Err(PdbCatalogError::Text {
                field: PdbCatalogField::Key,
                error: CatalogTextError::TooLong {
                    len: 17,
                    max: CATALOG_KEY_MAX,
                },
            })
        );
    }

    #[test]
    fn decodes_devicesql_utf16_to_utf8_and_normalizes_separator() {
        let units = [
            '/', 'C', 'o', 'n', 't', 'e', 'n', 't', 's', '\\', '\u{5468}', '\u{9632}', '\u{7fa9}',
            '\u{548c}', '/', 't', 'e', 's', 't', '.', 'm', 'p', '3',
        ];
        let total = 4 + (units.len() + 1) * 2;
        let mut raw = [0u8; 64];
        raw[0] = 0x90;
        put_le16(&mut raw, 1, total as u16);
        for (index, character) in units.into_iter().enumerate() {
            let value = character as u32 as u16;
            put_le16(&mut raw, 4 + index * 2, value);
        }

        let mut decoded = [0u8; 128];
        let result = decode_devicesql(&raw[..total], 0, &mut decoded).unwrap();
        let text = core::str::from_utf8(&decoded[..result.written]).unwrap();

        assert_eq!(text, "/Contents/周防義和/test.mp3");
        assert!(!result.truncated);
    }

    #[test]
    fn parses_track_header_from_fixed_136_byte_prefix() {
        let mut row = [0u8; PDB_TRACK_ROW_HEADER_BYTES];
        put_le16(&mut row, 0, TRACK_ROW_SUBTYPE);
        put_le32(&mut row, TRACK_OFF_KEY_ID, 7);
        put_le32(&mut row, TRACK_OFF_TEMPO, 12_834);
        put_le32(&mut row, TRACK_OFF_ALBUM_ID, 11);
        put_le32(&mut row, TRACK_OFF_ARTIST_ID, 12);
        put_le32(&mut row, TRACK_OFF_TRACK_ID, 42);
        put_le16(&mut row, TRACK_OFF_DURATION, 321);
        put_le16(&mut row, TRACK_OFF_STR_OFFS + STR_IDX_TITLE * 2, 0x90);
        put_le16(&mut row, TRACK_OFF_STR_OFFS + STR_IDX_FILENAME * 2, 0xa0);
        put_le16(&mut row, TRACK_OFF_STR_OFFS + STR_IDX_FILE_PATH * 2, 0xb0);
        put_le16(&mut row, TRACK_OFF_STR_OFFS + STR_IDX_ANLZ_PATH * 2, 0xc0);

        assert_eq!(
            parse_pdb_track_row_header(&row).unwrap(),
            Some(PdbTrackRowHeader {
                track_id: 42,
                bpm_x100: 12_834,
                bpm: 128,
                duration_s: 321,
                artist_id: 12,
                album_id: 11,
                key_id: 7,
                strings: PdbTrackStringOffsets {
                    title: 0x90,
                    filename: 0xa0,
                    file_path: 0xb0,
                    anlz_path: 0xc0,
                },
            })
        );
        assert_eq!(
            parse_pdb_track_row_header(&row[..PDB_TRACK_ROW_HEADER_BYTES - 1]),
            Err(PdbError::TrackRowOutOfRange)
        );
    }

    #[test]
    fn computes_on_demand_track_string_page_offsets() {
        assert_eq!(
            pdb_track_string_page_offset(0x100, 0x90, 4_096).unwrap(),
            Some(0x1b8)
        );
        assert_eq!(pdb_track_string_page_offset(0x100, 0, 4_096).unwrap(), None);
        assert_eq!(
            pdb_track_string_page_offset(4_000, 100, 4_096),
            Err(PdbError::TrackStringOffsetOutOfRange)
        );
    }

    #[test]
    fn walks_page_slots_and_parses_track_row_without_allocation() {
        let mut page = [0u8; 512];
        put_le32(&mut page, PDB_PAGE_NROWS_OFFSET, 1);
        put_le16(&mut page, 506, 0);
        put_le16(&mut page, 508, 1);

        let row = PDB_PAGE_HEAP_OFFSET;
        put_le16(&mut page, row, TRACK_ROW_SUBTYPE);
        put_le32(&mut page, row + TRACK_OFF_KEY_ID, 7);
        put_le32(&mut page, row + TRACK_OFF_TEMPO, 12_834);
        put_le32(&mut page, row + TRACK_OFF_ALBUM_ID, 11);
        put_le32(&mut page, row + TRACK_OFF_ARTIST_ID, 12);
        put_le32(&mut page, row + TRACK_OFF_TRACK_ID, 42);
        put_le16(&mut page, row + TRACK_OFF_DURATION, 321);

        let title_offset = 0x90u16;
        let file_offset = 0xa0u16;
        let anlz_offset = 0xc0u16;
        put_le16(
            &mut page,
            row + TRACK_OFF_STR_OFFS + STR_IDX_TITLE * 2,
            title_offset,
        );
        put_le16(
            &mut page,
            row + TRACK_OFF_STR_OFFS + STR_IDX_FILE_PATH * 2,
            file_offset,
        );
        put_le16(
            &mut page,
            row + TRACK_OFF_STR_OFFS + STR_IDX_ANLZ_PATH * 2,
            anlz_offset,
        );
        put_short_devicesql(&mut page, row + title_offset as usize, b"Test Track");
        put_short_devicesql(&mut page, row + file_offset as usize, b"/Contents/test.wav");
        put_short_devicesql(
            &mut page,
            row + anlz_offset as usize,
            b"/PIONEER/USBANLZ/P001/00000042/ANLZ0000.DAT",
        );

        let mut parsed = None;
        let walk = visit_pdb_page_rows(&page, 512, |heap_offset| {
            parsed = parse_pdb_track_row(&page, 512, heap_offset).unwrap();
            true
        })
        .unwrap();

        assert_eq!(walk.present_rows, 1);
        let track = parsed.unwrap();
        assert_eq!(track.track_id, 42);
        assert_eq!(track.bpm_x100, 12_834);
        assert_eq!(track.bpm, 128);
        assert_eq!(track.duration_s, 321);
        assert_eq!(track.artist_id, 12);
        assert_eq!(track.album_id, 11);
        assert_eq!(track.key_id, 7);
        assert_eq!(track.title.as_str(), Some("Test Track"));
        assert_eq!(track.file_path.as_str(), Some("/Contents/test.wav"));
        assert_eq!(
            track.anlz_path.as_str(),
            Some("/PIONEER/USBANLZ/P001/00000042/ANLZ0000.DAT")
        );
    }

    #[test]
    fn title_falls_back_to_filename() {
        let mut page = [0u8; 512];
        let row = PDB_PAGE_HEAP_OFFSET;
        put_le16(&mut page, row, TRACK_ROW_SUBTYPE);

        let filename_offset = 0x90u16;
        put_le16(
            &mut page,
            row + TRACK_OFF_STR_OFFS + STR_IDX_FILENAME * 2,
            filename_offset,
        );
        put_short_devicesql(&mut page, row + filename_offset as usize, b"fallback.mp3");

        let track = parse_pdb_track_row(&page, 512, 0).unwrap().unwrap();
        assert_eq!(track.title.as_str(), Some("fallback.mp3"));
    }

    // RP-01 fixtures. Slot positions are written from the documented track row
    // layout (string offsets at 0x5e, title 17, filename 19, path 20), not from
    // the parser's STR_IDX_* constants, so a wrong constant cannot hide here.
    const RAW_TRACK_SUBTYPE: u16 = 0x0024;
    const RAW_STRING_SLOTS: usize = 0x5e;

    fn raw_track_page(strings: &[(usize, u16)]) -> [u8; 512] {
        let mut page = [0u8; 512];
        let row = PDB_PAGE_HEAP_OFFSET;
        put_le16(&mut page, row, RAW_TRACK_SUBTYPE);
        for &(slot, offset) in strings {
            put_le16(&mut page, row + RAW_STRING_SLOTS + slot * 2, offset);
        }
        page
    }

    fn put_utf16le_devicesql(data: &mut [u8], offset: usize, units: &[u16]) {
        data[offset] = 0x90;
        put_le16(data, offset + 1, (4 + units.len() * 2) as u16);
        for (index, unit) in units.iter().copied().enumerate() {
            put_le16(data, offset + 4 + index * 2, unit);
        }
    }

    #[test]
    fn title_comes_from_slot_17_not_adjacent_slots() {
        let row = PDB_PAGE_HEAP_OFFSET;
        let mut page = raw_track_page(&[(17, 0x90), (18, 0xa0), (19, 0xb0), (20, 0xc0)]);
        put_short_devicesql(&mut page, row + 0x90, b"Slot 17 title");
        put_short_devicesql(&mut page, row + 0xa0, b"Slot 18 other");
        put_short_devicesql(&mut page, row + 0xb0, b"slot19.mp3");
        put_short_devicesql(&mut page, row + 0xc0, b"/Contents/a.mp3");

        let header = parse_pdb_track_row_header(&page[row..]).unwrap().unwrap();
        assert_eq!(header.strings.title, 0x90);
        assert_eq!(header.strings.filename, 0xb0);
        assert_eq!(header.strings.file_path, 0xc0);

        let track = parse_pdb_track_row(&page, 512, 0).unwrap().unwrap();
        assert_eq!(track.title.as_str(), Some("Slot 17 title"));
        assert_eq!(track.file_path.as_str(), Some("/Contents/a.mp3"));

        let names = PdbResolvedNames {
            artist: None,
            album: None,
            key: None,
        };
        let catalog = catalog_track_from_pdb(VolumeIdentity([0x17; 32]), &track, names).unwrap();
        assert_eq!(catalog.title.as_str(), "Slot 17 title");
    }

    #[test]
    fn missing_or_empty_slot_17_falls_back_to_filename_not_slot_18() {
        let row = PDB_PAGE_HEAP_OFFSET;
        let mut absent = raw_track_page(&[(18, 0xa0), (19, 0xb0)]);
        put_short_devicesql(&mut absent, row + 0xa0, b"Slot 18 other");
        put_short_devicesql(&mut absent, row + 0xb0, b"slot19.mp3");
        let track = parse_pdb_track_row(&absent, 512, 0).unwrap().unwrap();
        assert_eq!(track.title.as_str(), Some("slot19.mp3"));

        let mut empty = absent;
        put_le16(&mut empty, row + RAW_STRING_SLOTS + 17 * 2, 0x90);
        empty[row + 0x90] = 0x03;
        let track = parse_pdb_track_row(&empty, 512, 0).unwrap().unwrap();
        assert_eq!(track.title.as_str(), Some("slot19.mp3"));

        let mut neither = raw_track_page(&[(18, 0xa0)]);
        put_short_devicesql(&mut neither, row + 0xa0, b"Slot 18 other");
        let track = parse_pdb_track_row(&neither, 512, 0).unwrap().unwrap();
        assert!(track.title.is_empty());
    }

    #[test]
    fn utf16_title_in_slot_17_decodes_with_surrogate_pairs() {
        let row = PDB_PAGE_HEAP_OFFSET;
        let mut page = raw_track_page(&[(17, 0x90), (18, 0xd0), (19, 0xe0)]);
        // "Ünï — 周防 𝄞" as explicit UTF-16 code units.
        put_utf16le_devicesql(
            &mut page,
            row + 0x90,
            &[
                0x00dc, 0x006e, 0x00ef, 0x0020, 0x2014, 0x0020, 0x5468, 0x9632, 0x0020, 0xd834,
                0xdd1e,
            ],
        );
        put_short_devicesql(&mut page, row + 0xd0, b"Slot 18 other");
        put_short_devicesql(&mut page, row + 0xe0, b"slot19.mp3");

        let track = parse_pdb_track_row(&page, 512, 0).unwrap().unwrap();
        assert_eq!(track.title.as_str(), Some("Ünï — 周防 𝄞"));
        assert!(!track.title.was_truncated());
    }

    // M2.5 parity: a malformed title is treated as missing and the row shows
    // its filename. Only the title is lenient; a malformed path still fails.
    #[test]
    fn malformed_slot_17_title_falls_back_to_filename() {
        let row = PDB_PAGE_HEAP_OFFSET;
        let mut lone_surrogate = raw_track_page(&[(17, 0x90), (19, 0xb0), (20, 0xc0)]);
        put_utf16le_devicesql(&mut lone_surrogate, row + 0x90, &[0x0041, 0xdc00]);
        put_short_devicesql(&mut lone_surrogate, row + 0xb0, b"slot19.mp3");
        put_short_devicesql(&mut lone_surrogate, row + 0xc0, b"/Contents/a.mp3");
        let track = parse_pdb_track_row(&lone_surrogate, 512, 0)
            .unwrap()
            .unwrap();
        assert_eq!(track.title.as_str(), Some("slot19.mp3"));
        assert_eq!(track.file_path.as_str(), Some("/Contents/a.mp3"));

        let mut outside_page = raw_track_page(&[(17, 0x1e0), (19, 0xb0)]);
        put_short_devicesql(&mut outside_page, row + 0xb0, b"slot19.mp3");
        let track = parse_pdb_track_row(&outside_page, 512, 0).unwrap().unwrap();
        assert_eq!(track.title.as_str(), Some("slot19.mp3"));
        assert_eq!(
            recover_pdb_title::<PDB_TITLE_MAX>(Err(PdbError::TrackStringOffsetOutOfRange)),
            PdbText::new()
        );

        // Short ASCII with non-UTF-8 bytes decodes but cannot be shown as text.
        let mut not_utf8 = raw_track_page(&[(17, 0x90), (19, 0xb0)]);
        put_short_devicesql(&mut not_utf8, row + 0x90, &[0x54, 0xff, 0xfe]);
        put_short_devicesql(&mut not_utf8, row + 0xb0, b"slot19.mp3");
        let track = parse_pdb_track_row(&not_utf8, 512, 0).unwrap().unwrap();
        assert_eq!(track.title.as_str(), Some("slot19.mp3"));

        let mut bad_path = raw_track_page(&[(17, 0x90), (20, 0xc0)]);
        put_short_devicesql(&mut bad_path, row + 0x90, b"Title");
        put_utf16le_devicesql(&mut bad_path, row + 0xc0, &[0x002f, 0xdc00]);
        assert_eq!(
            parse_pdb_track_row(&bad_path, 512, 0),
            Err(PdbError::DeviceSqlInvalidUtf16)
        );
    }
}
