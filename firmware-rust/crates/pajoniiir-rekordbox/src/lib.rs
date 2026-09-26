#![no_std]
#![forbid(unsafe_code)]

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

const STR_IDX_ANLZ_PATH: usize = 14;
const STR_IDX_TITLE: usize = 18;
const STR_IDX_FILENAME: usize = 19;
const STR_IDX_FILE_PATH: usize = 20;

pub const PDB_TITLE_MAX: usize = 128;
pub const PDB_PATH_MAX: usize = 256;

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
    DeviceSqlOffsetOutOfRange,
    DeviceSqlTruncated,
    DeviceSqlInvalidLength,
    DeviceSqlInvalidUtf16,
    TrackRowOutOfRange,
    TrackStringOffsetOutOfRange,
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

#[derive(Clone, Debug, Eq, PartialEq)]
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

    let subtype = read_le16(page, row).ok_or(PdbError::TrackRowOutOfRange)?;
    if subtype != TRACK_ROW_SUBTYPE {
        return Ok(None);
    }

    let bpm_x100 = read_le32(page, row + TRACK_OFF_TEMPO).ok_or(PdbError::TrackRowOutOfRange)?;
    let rounded_bpm = (u64::from(bpm_x100) + 50) / 100;

    let mut title = PdbText::new();
    let mut filename = PdbText::new();
    let mut file_path = PdbText::new();
    let mut anlz_path = PdbText::new();

    decode_track_string(page, size, row, STR_IDX_TITLE, &mut title)?;
    decode_track_string(page, size, row, STR_IDX_FILENAME, &mut filename)?;
    decode_track_string(page, size, row, STR_IDX_FILE_PATH, &mut file_path)?;
    decode_track_string(page, size, row, STR_IDX_ANLZ_PATH, &mut anlz_path)?;

    if title.is_empty() {
        title.copy_from(&filename);
    }

    Ok(Some(PdbTrackRow {
        track_id: read_le32(page, row + TRACK_OFF_TRACK_ID).ok_or(PdbError::TrackRowOutOfRange)?,
        bpm_x100,
        bpm: u16::try_from(rounded_bpm).unwrap_or(u16::MAX),
        duration_s: read_le16(page, row + TRACK_OFF_DURATION)
            .ok_or(PdbError::TrackRowOutOfRange)?,
        artist_id: read_le32(page, row + TRACK_OFF_ARTIST_ID)
            .ok_or(PdbError::TrackRowOutOfRange)?,
        album_id: read_le32(page, row + TRACK_OFF_ALBUM_ID).ok_or(PdbError::TrackRowOutOfRange)?,
        key_id: read_le32(page, row + TRACK_OFF_KEY_ID).ok_or(PdbError::TrackRowOutOfRange)?,
        title,
        file_path,
        anlz_path,
    }))
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
}
