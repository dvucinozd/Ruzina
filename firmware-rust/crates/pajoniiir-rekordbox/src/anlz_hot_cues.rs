pub const ANLZ_SECTION_HEADER_BYTES: usize = 12;
pub const ANLZ_PCOB_HEADER_BYTES: usize = 24;
pub const ANLZ_PCPT_BYTES: usize = 56;
pub const REKORDBOX_HOT_CUE_COUNT: usize = 8;

pub const ANLZ_TAG_PMAI: u32 = u32::from_be_bytes(*b"PMAI");
pub const ANLZ_TAG_PCOB: u32 = u32::from_be_bytes(*b"PCOB");
pub const ANLZ_TAG_PCPT: u32 = u32::from_be_bytes(*b"PCPT");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RekordboxCueKind {
    Single,
    Loop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RekordboxHotCue {
    pub index: u8,
    pub kind: RekordboxCueKind,
    pub start_ms: u32,
    pub end_ms: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RekordboxHotCueBank {
    slots: [Option<RekordboxHotCue>; REKORDBOX_HOT_CUE_COUNT],
}

impl RekordboxHotCueBank {
    pub const fn new() -> Self {
        Self {
            slots: [None; REKORDBOX_HOT_CUE_COUNT],
        }
    }

    pub fn slot(&self, index: u8) -> Option<RekordboxHotCue> {
        self.slots.get(index as usize).copied().flatten()
    }

    pub fn insert(&mut self, cue: RekordboxHotCue) -> Result<(), AnlzHotCueError> {
        let Some(slot) = self.slots.get_mut(cue.index as usize) else {
            return Err(AnlzHotCueError::HotCueOutOfRange);
        };
        if slot.is_some() {
            return Err(AnlzHotCueError::DuplicateHotCue(cue.index));
        }
        *slot = Some(cue);
        Ok(())
    }

    pub fn exists_mask(&self) -> u8 {
        self.slots
            .iter()
            .enumerate()
            .fold(0u8, |mask, (index, slot)| {
                if slot.is_some() {
                    mask | (1u8 << index)
                } else {
                    mask
                }
            })
    }
}

impl Default for RekordboxHotCueBank {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnlzHotCueError {
    FileTooShort,
    HeaderTooShort,
    InvalidSectionSize,
    SectionOutOfRange,
    InvalidPcob,
    InvalidPcpt,
    HotCueOutOfRange,
    InvalidLoop,
    DuplicateHotCue(u8),
    TrailingBytes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnlzSectionHeader {
    pub position: u32,
    pub tag: u32,
    pub header_size: u32,
    pub segment_size: u32,
    pub next_position: u32,
}

pub fn parse_section_header(
    position: u32,
    file_len: u32,
    bytes: &[u8],
) -> Result<AnlzSectionHeader, AnlzHotCueError> {
    if file_len < ANLZ_SECTION_HEADER_BYTES as u32 {
        return Err(AnlzHotCueError::FileTooShort);
    }
    if bytes.len() < ANLZ_SECTION_HEADER_BYTES {
        return Err(AnlzHotCueError::HeaderTooShort);
    }

    let tag = be32(&bytes[0..4]);
    let header_size = be32(&bytes[4..8]);
    let segment_size = be32(&bytes[8..12]);
    if header_size < ANLZ_SECTION_HEADER_BYTES as u32 || segment_size < header_size {
        return Err(AnlzHotCueError::InvalidSectionSize);
    }

    let advance = if tag == ANLZ_TAG_PMAI {
        header_size
    } else {
        segment_size
    };
    if advance < ANLZ_SECTION_HEADER_BYTES as u32 {
        return Err(AnlzHotCueError::InvalidSectionSize);
    }
    let next_position = position
        .checked_add(advance)
        .ok_or(AnlzHotCueError::SectionOutOfRange)?;
    if next_position > file_len {
        return Err(AnlzHotCueError::SectionOutOfRange);
    }

    Ok(AnlzSectionHeader {
        position,
        tag,
        header_size,
        segment_size,
        next_position,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcobPlan {
    pub list_type: u32,
    pub declared_count: u16,
    pub records_position: u32,
}

pub fn parse_pcob_header(
    position: u32,
    section: AnlzSectionHeader,
    bytes: &[u8],
) -> Result<PcobPlan, AnlzHotCueError> {
    if section.tag != ANLZ_TAG_PCOB
        || section.position != position
        || bytes.len() < ANLZ_PCOB_HEADER_BYTES
        || &bytes[0..4] != b"PCOB"
    {
        return Err(AnlzHotCueError::InvalidPcob);
    }
    if section.header_size < ANLZ_PCOB_HEADER_BYTES as u32
        || section.segment_size < section.header_size
    {
        return Err(AnlzHotCueError::InvalidPcob);
    }

    let list_type = be32(&bytes[12..16]);
    let declared_count = be16(&bytes[18..20]);
    let data_len = section.segment_size - section.header_size;
    if !data_len.is_multiple_of(ANLZ_PCPT_BYTES as u32)
        || u32::from(declared_count) != data_len / ANLZ_PCPT_BYTES as u32
    {
        return Err(AnlzHotCueError::InvalidPcob);
    }

    let records_position = position
        .checked_add(section.header_size)
        .ok_or(AnlzHotCueError::SectionOutOfRange)?;
    Ok(PcobPlan {
        list_type,
        declared_count,
        records_position,
    })
}

pub fn parse_pcpt(
    list_type: u32,
    bytes: &[u8],
) -> Result<Option<RekordboxHotCue>, AnlzHotCueError> {
    if bytes.len() < ANLZ_PCPT_BYTES
        || &bytes[0..4] != b"PCPT"
        || be32(&bytes[4..8]) < 28
        || be32(&bytes[8..12]) != ANLZ_PCPT_BYTES as u32
    {
        return Err(AnlzHotCueError::InvalidPcpt);
    }

    if list_type == 0 {
        return Ok(None);
    }
    if list_type != 1 {
        return Ok(None);
    }

    let hot_cue = be32(&bytes[12..16]);
    if hot_cue == 0 || hot_cue > REKORDBOX_HOT_CUE_COUNT as u32 {
        return Ok(None);
    }

    let kind = match bytes[28] {
        1 => RekordboxCueKind::Single,
        2 => RekordboxCueKind::Loop,
        _ => return Err(AnlzHotCueError::InvalidPcpt),
    };
    let start_ms = be32(&bytes[32..36]);
    let raw_end_ms = be32(&bytes[36..40]);
    let end_ms = match kind {
        RekordboxCueKind::Single => 0,
        RekordboxCueKind::Loop if raw_end_ms > start_ms => raw_end_ms,
        RekordboxCueKind::Loop => return Err(AnlzHotCueError::InvalidLoop),
    };

    Ok(Some(RekordboxHotCue {
        index: (hot_cue - 1) as u8,
        kind,
        start_ms,
        end_ms,
    }))
}

pub fn parse_hot_cues(data: &[u8]) -> Result<RekordboxHotCueBank, AnlzHotCueError> {
    let file_len = u32::try_from(data.len()).map_err(|_| AnlzHotCueError::SectionOutOfRange)?;
    if file_len < ANLZ_SECTION_HEADER_BYTES as u32 {
        return Err(AnlzHotCueError::FileTooShort);
    }

    let mut bank = RekordboxHotCueBank::new();
    let mut position = 0u32;
    while position + ANLZ_SECTION_HEADER_BYTES as u32 <= file_len {
        let start = position as usize;
        let section = parse_section_header(position, file_len, &data[start..start + 12])?;
        if section.tag == ANLZ_TAG_PCOB {
            let header_end = start
                .checked_add(ANLZ_PCOB_HEADER_BYTES)
                .ok_or(AnlzHotCueError::SectionOutOfRange)?;
            if header_end > data.len() {
                return Err(AnlzHotCueError::InvalidPcob);
            }
            let plan = parse_pcob_header(position, section, &data[start..header_end])?;
            for record in 0..u32::from(plan.declared_count) {
                let record_position = plan
                    .records_position
                    .checked_add(record * ANLZ_PCPT_BYTES as u32)
                    .ok_or(AnlzHotCueError::SectionOutOfRange)?;
                let record_start = record_position as usize;
                let record_end = record_start
                    .checked_add(ANLZ_PCPT_BYTES)
                    .ok_or(AnlzHotCueError::SectionOutOfRange)?;
                let bytes = data
                    .get(record_start..record_end)
                    .ok_or(AnlzHotCueError::SectionOutOfRange)?;
                if let Some(cue) = parse_pcpt(plan.list_type, bytes)? {
                    bank.insert(cue)?;
                }
            }
        }
        position = section.next_position;
    }

    if position != file_len {
        return Err(AnlzHotCueError::TrailingBytes);
    }
    Ok(bank)
}

fn be16(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[0], bytes[1]])
}

fn be32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    fn put_be16(bytes: &mut [u8], value: u16) {
        bytes.copy_from_slice(&value.to_be_bytes());
    }

    fn put_be32(bytes: &mut [u8], value: u32) {
        bytes.copy_from_slice(&value.to_be_bytes());
    }

    fn pcpt(index: u32, kind: u8, start_ms: u32, end_ms: u32) -> [u8; 56] {
        let mut raw = [0u8; 56];
        raw[0..4].copy_from_slice(b"PCPT");
        put_be32(&mut raw[4..8], 28);
        put_be32(&mut raw[8..12], 56);
        put_be32(&mut raw[12..16], index);
        raw[28] = kind;
        put_be32(&mut raw[32..36], start_ms);
        put_be32(&mut raw[36..40], end_ms);
        raw
    }

    fn pcob(list_type: u32, records: &[[u8; 56]]) -> std::vec::Vec<u8> {
        let header_size = 24u32;
        let segment_size = header_size + records.len() as u32 * 56;
        let mut raw = std::vec![0u8; segment_size as usize];
        raw[0..4].copy_from_slice(b"PCOB");
        put_be32(&mut raw[4..8], header_size);
        put_be32(&mut raw[8..12], segment_size);
        put_be32(&mut raw[12..16], list_type);
        put_be16(&mut raw[18..20], records.len() as u16);
        for (index, record) in records.iter().enumerate() {
            let start = 24 + index * 56;
            raw[start..start + 56].copy_from_slice(record);
        }
        raw
    }

    #[test]
    fn parses_hot_cue_and_loop_with_released_offsets() {
        let records = [pcpt(1, 1, 1_250, 0), pcpt(8, 2, 8_000, 12_000)];
        let data = pcob(1, &records);
        let bank = parse_hot_cues(&data).unwrap();

        assert_eq!(bank.exists_mask(), 0x81);
        assert_eq!(
            bank.slot(0),
            Some(RekordboxHotCue {
                index: 0,
                kind: RekordboxCueKind::Single,
                start_ms: 1_250,
                end_ms: 0,
            })
        );
        assert_eq!(
            bank.slot(7),
            Some(RekordboxHotCue {
                index: 7,
                kind: RekordboxCueKind::Loop,
                start_ms: 8_000,
                end_ms: 12_000,
            })
        );
    }

    #[test]
    fn ignores_memory_cue_list_for_performance_bank() {
        let data = pcob(0, &[pcpt(0, 1, 500, 0)]);
        let bank = parse_hot_cues(&data).unwrap();
        assert_eq!(bank.exists_mask(), 0);
    }

    #[test]
    fn pmai_advances_by_header_size_then_walks_child_sections() {
        let hot = pcob(1, &[pcpt(3, 1, 3_000, 0)]);
        let mut file = std::vec![0u8; 28];
        file[0..4].copy_from_slice(b"PMAI");
        put_be32(&mut file[4..8], 28);
        put_be32(&mut file[8..12], (28 + hot.len()) as u32);
        file.extend_from_slice(&hot);

        let bank = parse_hot_cues(&file).unwrap();
        assert_eq!(bank.slot(2).unwrap().start_ms, 3_000);
    }

    #[test]
    fn rejects_duplicate_hot_cue_across_pcob_sections() {
        let first = pcob(1, &[pcpt(2, 1, 1_000, 0)]);
        let second = pcob(1, &[pcpt(2, 1, 2_000, 0)]);
        let mut data = first;
        data.extend_from_slice(&second);

        assert_eq!(
            parse_hot_cues(&data),
            Err(AnlzHotCueError::DuplicateHotCue(1))
        );
    }

    #[test]
    fn rejects_malformed_count_and_loop() {
        let mut bad_count = pcob(1, &[pcpt(1, 1, 1_000, 0)]);
        put_be16(&mut bad_count[18..20], 2);
        assert_eq!(
            parse_hot_cues(&bad_count),
            Err(AnlzHotCueError::InvalidPcob)
        );

        let bad_loop = pcob(1, &[pcpt(1, 2, 5_000, 4_000)]);
        assert_eq!(parse_hot_cues(&bad_loop), Err(AnlzHotCueError::InvalidLoop));
    }

    #[test]
    fn rejects_tag_like_payload_and_trailing_garbage() {
        let mut unknown = std::vec![0u8; 24];
        unknown[0..4].copy_from_slice(b"XXXX");
        put_be32(&mut unknown[4..8], 12);
        put_be32(&mut unknown[8..12], 24);
        unknown[12..16].copy_from_slice(b"PCOB");
        let bank = parse_hot_cues(&unknown).unwrap();
        assert_eq!(bank.exists_mask(), 0);

        let mut trailing = pcob(1, &[]);
        trailing.push(0xff);
        assert_eq!(
            parse_hot_cues(&trailing),
            Err(AnlzHotCueError::TrailingBytes)
        );
    }
}
