//! Streaming importer. Format reference (independent implementation):
//! https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/anlz.html
//! Reads are <=512 bytes, allocations and total file/record counts are capped.
use crate::anlz_hot_cues::{
    AnlzHotCueError, AnlzSectionHeader, RekordboxCueKind, parse_memory_pcpt, parse_pcob_header,
    parse_pcpt, parse_section_header,
};
use ruzina_track_analysis::{
    AnalysisCue, AnalysisMemoryCue, AnalysisProvider, Beat, ColorPreviewEntry,
    MAX_COLOR_PREVIEW_COLUMNS, WaveformColumn,
    owned::{
        AnalysisBuilder, AnalysisDataError, AnalysisIdentity, AnalysisPublication,
        MAX_ANALYSIS_BEATS, MAX_DETAIL_COLUMNS,
    },
};

pub const ANLZ_READ_BYTES: usize = 512;
pub const ANLZ_MAX_FILE_BYTES: u32 = 8 * 1024 * 1024;
pub const ANLZ_MAX_SECTIONS: u16 = 256;
const PQTZ: u32 = u32::from_be_bytes(*b"PQTZ");
const PWAV: u32 = u32::from_be_bytes(*b"PWAV");
const PWV3: u32 = u32::from_be_bytes(*b"PWV3");
const PWV4: u32 = u32::from_be_bytes(*b"PWV4");
const PWV4_ENTRY_BYTES: u32 = 6;
const PCOB: u32 = u32::from_be_bytes(*b"PCOB");
const PMAI: u32 = u32::from_be_bytes(*b"PMAI");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnalysisFileKind {
    Dat,
    Ext,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportError {
    Container,
    Bounds,
    Duplicate,
    ShortRead,
    UnexpectedRead,
    Incomplete,
    Data(AnalysisDataError),
    Cue(AnlzHotCueError),
}
impl From<AnalysisDataError> for ImportError {
    fn from(value: AnalysisDataError) -> Self {
        Self::Data(value)
    }
}
impl From<AnlzHotCueError> for ImportError {
    fn from(value: AnlzHotCueError) -> Self {
        Self::Cue(value)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnalysisRead {
    pub offset: u32,
    pub len: usize,
}
#[derive(Clone, Copy)]
enum Payload {
    Beats,
    Preview,
    Detail,
    Cues(u32),
    /// Retained PWV4 columns end at `end`; the rest of the section is skipped.
    ColorPreview {
        end: u32,
    },
}
#[derive(Clone, Copy)]
enum Stage {
    Idle,
    Root,
    Section(u32),
    Header(AnlzSectionHeader),
    Body {
        section: AnlzSectionHeader,
        offset: u32,
        kind: Payload,
    },
    Done,
    Failed,
}

pub struct AnlzImporter {
    data: AnalysisBuilder,
    stage: Stage,
    file_len: u32,
    kind: AnalysisFileKind,
    dat_started: bool,
    dat_done: bool,
    ext_started: bool,
    ext_done: bool,
    sections: u16,
    seen_beats: bool,
    seen_preview: bool,
    seen_detail: bool,
    seen_cues: bool,
    seen_memory: bool,
    seen_color_preview: bool,
    color_preview_rejected: bool,
}
impl AnlzImporter {
    pub const fn new() -> Self {
        Self {
            data: AnalysisBuilder::new(),
            stage: Stage::Idle,
            file_len: 0,
            kind: AnalysisFileKind::Dat,
            dat_started: false,
            dat_done: false,
            ext_started: false,
            ext_done: false,
            sections: 0,
            seen_beats: false,
            seen_preview: false,
            seen_detail: false,
            seen_cues: false,
            seen_memory: false,
            seen_color_preview: false,
            color_preview_rejected: false,
        }
    }
    /// A PWV4 section was present but malformed or repeated and was skipped;
    /// PWAV/PWV3 stay authoritative (M2.5 "PWV4 ignored").
    pub const fn color_preview_rejected(&self) -> bool {
        self.color_preview_rejected
    }
    pub fn begin_file(&mut self, kind: AnalysisFileKind, len: u64) -> Result<(), ImportError> {
        if !matches!(self.stage, Stage::Idle | Stage::Done)
            || !(12..=u64::from(ANLZ_MAX_FILE_BYTES)).contains(&len)
            || (kind == AnalysisFileKind::Dat && self.dat_started)
            || (kind == AnalysisFileKind::Ext && (!self.dat_done || self.ext_started))
        {
            self.stage = Stage::Failed;
            return Err(ImportError::Container);
        }
        self.kind = kind;
        self.file_len = len as u32;
        self.sections = 0;
        // One cue list of each type per file; DAT/EXT overlap is merged.
        self.seen_cues = false;
        self.seen_memory = false;
        self.stage = Stage::Root;
        match kind {
            AnalysisFileKind::Dat => self.dat_started = true,
            AnalysisFileKind::Ext => self.ext_started = true,
        }
        Ok(())
    }
    pub fn read_plan(&self) -> Option<AnalysisRead> {
        match self.stage {
            Stage::Root => Some(AnalysisRead { offset: 0, len: 12 }),
            Stage::Section(offset) => Some(AnalysisRead { offset, len: 12 }),
            Stage::Header(section) => Some(AnalysisRead {
                offset: section.position,
                len: if section.tag == PWAV { 20 } else { 24 },
            }),
            Stage::Body {
                section,
                offset,
                kind,
            } => {
                let unit = match kind {
                    Payload::Beats => 8,
                    Payload::Cues(_) => 56,
                    Payload::ColorPreview { .. } => PWV4_ENTRY_BYTES as usize,
                    _ => 1,
                };
                let end = match kind {
                    Payload::ColorPreview { end } => end,
                    _ => section.next_position,
                };
                let len = (end - offset).min((ANLZ_READ_BYTES / unit * unit) as u32) as usize;
                Some(AnalysisRead { offset, len })
            }
            _ => None,
        }
    }
    pub fn ingest(&mut self, offset: u32, bytes: &[u8]) -> Result<(), ImportError> {
        let result = self.ingest_inner(offset, bytes);
        if result.is_err() {
            self.stage = Stage::Failed;
        }
        result
    }
    fn ingest_inner(&mut self, offset: u32, bytes: &[u8]) -> Result<(), ImportError> {
        let plan = self.read_plan().ok_or(ImportError::UnexpectedRead)?;
        if offset != plan.offset {
            return Err(ImportError::UnexpectedRead);
        }
        if bytes.len() != plan.len {
            return Err(ImportError::ShortRead);
        }
        match self.stage {
            Stage::Root => {
                if be32(bytes, 0) != PMAI || be32(bytes, 8) != self.file_len {
                    return Err(ImportError::Container);
                }
                let header = be32(bytes, 4);
                if header < 12 || header > self.file_len {
                    return Err(ImportError::Bounds);
                }
                self.next_section(header)?;
            }
            Stage::Section(position) => {
                self.sections = self
                    .sections
                    .checked_add(1)
                    .filter(|n| *n <= ANLZ_MAX_SECTIONS)
                    .ok_or(ImportError::Bounds)?;
                let section = parse_section_header(position, self.file_len, bytes)?;
                if section.tag == PMAI {
                    return Err(ImportError::Container);
                }
                let known = match self.kind {
                    AnalysisFileKind::Dat => matches!(section.tag, PQTZ | PWAV | PCOB),
                    // EXT may carry hot cues D-H and memory cues (M2.5 082d2c0a).
                    AnalysisFileKind::Ext => matches!(section.tag, PWV3 | PWV4 | PCOB),
                };
                if section.tag == PWV4 && (section.header_size < 24 || self.seen_color_preview) {
                    // Optional color data never fails the PWV3/cue import.
                    self.color_preview_rejected = true;
                    self.next_section(section.next_position)?;
                } else if known {
                    let minimum = if section.tag == PWAV { 20 } else { 24 };
                    if section.header_size < minimum || section.segment_size < minimum {
                        return Err(ImportError::Bounds);
                    }
                    self.stage = Stage::Header(section);
                } else {
                    self.next_section(section.next_position)?;
                }
            }
            Stage::Header(section) => {
                let payload_len = section.segment_size - section.header_size;
                let kind = match section.tag {
                    PQTZ => {
                        if self.seen_beats {
                            return Err(ImportError::Duplicate);
                        }
                        self.seen_beats = true;
                        let count = be32(bytes, 20) as usize;
                        if count > MAX_ANALYSIS_BEATS || count as u64 * 8 != u64::from(payload_len)
                        {
                            return Err(ImportError::Bounds);
                        }
                        self.data.reserve_beats(count)?;
                        Payload::Beats
                    }
                    PWAV => {
                        if self.seen_preview {
                            return Err(ImportError::Duplicate);
                        }
                        self.seen_preview = true;
                        if be32(bytes, 12) != 400 || payload_len != 400 {
                            return Err(ImportError::Bounds);
                        }
                        self.data.reserve_waveform(false, 400)?;
                        Payload::Preview
                    }
                    PWV3 => {
                        if self.seen_detail {
                            return Err(ImportError::Duplicate);
                        }
                        self.seen_detail = true;
                        let count = be32(bytes, 16);
                        if be32(bytes, 12) != 1
                            || count > MAX_DETAIL_COLUMNS as u32
                            || payload_len != count
                        {
                            return Err(ImportError::Bounds);
                        }
                        self.data.reserve_waveform(true, count as usize)?;
                        self.data.set_detail_rate(150);
                        Payload::Detail
                    }
                    PWV4 => {
                        self.seen_color_preview = true;
                        let entry_size = be32(bytes, 12);
                        let count = be32(bytes, 16);
                        if entry_size != PWV4_ENTRY_BYTES
                            || count == 0
                            || u64::from(payload_len) < u64::from(count) * 6
                        {
                            self.color_preview_rejected = true;
                            return self.next_section(section.next_position);
                        }
                        let retained = count.min(MAX_COLOR_PREVIEW_COLUMNS as u32);
                        self.data.reserve_color_preview(
                            retained as usize,
                            count > MAX_COLOR_PREVIEW_COLUMNS as u32,
                        )?;
                        Payload::ColorPreview {
                            end: section.position + section.header_size + retained * 6,
                        }
                    }
                    PCOB => {
                        let plan = parse_pcob_header(section.position, section, bytes)?;
                        let seen = match plan.list_type {
                            0 => Some(&mut self.seen_memory),
                            1 => Some(&mut self.seen_cues),
                            _ => None,
                        };
                        if let Some(seen) = seen {
                            if *seen {
                                return Err(ImportError::Duplicate);
                            }
                            *seen = true;
                        }
                        Payload::Cues(plan.list_type)
                    }
                    _ => return Err(ImportError::Container),
                };
                if payload_len == 0 {
                    self.next_section(section.next_position)?;
                } else {
                    self.stage = Stage::Body {
                        section,
                        offset: section.position + section.header_size,
                        kind,
                    };
                }
            }
            Stage::Body {
                section,
                offset,
                kind,
            } => {
                match kind {
                    Payload::Beats => {
                        for beat in bytes.chunks_exact(8) {
                            let phase = u16::from_be_bytes([beat[0], beat[1]]);
                            if !(1..=4).contains(&phase) {
                                return Err(ImportError::Data(AnalysisDataError::InvalidBeat));
                            }
                            self.data.push_beat(Beat {
                                time_ms: be32(beat, 4),
                                phase: phase - 1,
                                bpm_x100: u32::from(u16::from_be_bytes([beat[2], beat[3]])),
                            })?;
                        }
                    }
                    Payload::Preview | Payload::Detail => {
                        for byte in bytes {
                            self.data.push_waveform(
                                matches!(kind, Payload::Detail),
                                mono_column(*byte),
                            )?;
                        }
                    }
                    Payload::ColorPreview { .. } => {
                        for entry in bytes.chunks_exact(PWV4_ENTRY_BYTES as usize) {
                            let entry = entry.try_into().expect("six-byte chunk");
                            self.data.push_color_preview(ColorPreviewEntry(entry))?;
                        }
                    }
                    Payload::Cues(0) => {
                        for record in bytes.chunks_exact(56) {
                            let cue = parse_memory_pcpt(record)?;
                            let cue = AnalysisMemoryCue {
                                start_ms: cue.start_ms,
                                end_ms: (cue.kind == RekordboxCueKind::Loop).then_some(cue.end_ms),
                            };
                            match self.kind {
                                AnalysisFileKind::Dat => self.data.push_memory_cue(cue)?,
                                AnalysisFileKind::Ext => self.data.merge_memory_cue(cue)?,
                            }
                        }
                    }
                    Payload::Cues(list) => {
                        for record in bytes.chunks_exact(56) {
                            if let Some(cue) = parse_pcpt(list, record)? {
                                let cue = AnalysisCue {
                                    slot: cue.index,
                                    start_ms: cue.start_ms,
                                    end_ms: (cue.kind == RekordboxCueKind::Loop)
                                        .then_some(cue.end_ms),
                                };
                                match self.kind {
                                    AnalysisFileKind::Dat => self.data.push_cue(cue)?,
                                    AnalysisFileKind::Ext => self.data.merge_source_cue(cue)?,
                                }
                            }
                        }
                    }
                }
                let end = offset + bytes.len() as u32;
                let payload_end = match kind {
                    Payload::ColorPreview { end } => end,
                    _ => section.next_position,
                };
                if end == payload_end {
                    self.next_section(section.next_position)?;
                } else {
                    self.stage = Stage::Body {
                        section,
                        offset: end,
                        kind,
                    };
                }
            }
            _ => return Err(ImportError::UnexpectedRead),
        }
        Ok(())
    }
    fn next_section(&mut self, offset: u32) -> Result<(), ImportError> {
        if offset == self.file_len {
            self.stage = Stage::Done;
            match self.kind {
                AnalysisFileKind::Dat => self.dat_done = true,
                AnalysisFileKind::Ext => self.ext_done = true,
            }
        } else {
            if offset > self.file_len || self.file_len - offset < 12 {
                return Err(ImportError::Bounds);
            }
            self.stage = Stage::Section(offset);
        }
        Ok(())
    }
    pub fn file_complete(&self) -> bool {
        matches!(self.stage, Stage::Done)
    }
    pub fn finish(
        self,
        identity: AnalysisIdentity,
        fallback_bpm_x100: u32,
    ) -> Result<AnalysisPublication, ImportError> {
        if !self.dat_done
            || (self.ext_started && !self.ext_done)
            || !matches!(self.stage, Stage::Done)
        {
            return Err(ImportError::Incomplete);
        }
        Ok(self.data.finish(
            identity,
            AnalysisProvider::RekordboxImport,
            fallback_bpm_x100,
        )?)
    }
}
impl Default for AnlzImporter {
    fn default() -> Self {
        Self::new()
    }
}
fn be32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().expect("planned field"))
}
fn mono_column(byte: u8) -> WaveformColumn {
    let white = u16::from(byte >> 5);
    WaveformColumn {
        peak: (u32::from(byte & 31) * 65_535 / 31) as u16,
        color_rgb565: ((white * 31 / 7) << 11) | ((white * 63 / 7) << 5) | 31,
    }
}

pub fn extended_path<'a>(dat: &str, storage: &'a mut [u8]) -> Option<&'a str> {
    if dat.len() < 4
        || dat.len() > storage.len()
        || !dat.as_bytes()[dat.len() - 4..].eq_ignore_ascii_case(b".dat")
    {
        return None;
    }
    storage[..dat.len()].copy_from_slice(dat.as_bytes());
    storage[dat.len() - 3..dat.len()].copy_from_slice(b"EXT");
    core::str::from_utf8(&storage[..dat.len()]).ok()
}
