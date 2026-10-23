//! Bounded owned data, built outside render/control and immutable after publish.
use crate::{
    AnalysisCue, AnalysisMemoryCue, AnalysisProvider, Beat, BeatGrid, ColorPreviewEntry,
    MAX_COLOR_PREVIEW_COLUMNS, MAX_MEMORY_CUES, TrackAnalysis, WaveformColumn,
};
use alloc::vec::Vec;
use ruzina_core::MediaTrackId;
use ruzina_media_session::MediaLease;

pub const MAX_ANALYSIS_BEATS: usize = 65_535;
pub const MAX_PREVIEW_COLUMNS: usize = 400;
pub const MAX_DETAIL_COLUMNS: usize = 131_072;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnalysisIdentity {
    pub track: MediaTrackId,
    pub lease: MediaLease,
    pub catalog_revision: u32,
    pub load_generation: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnalysisDataError {
    Capacity,
    Allocation,
    InvalidBeat,
    InvalidCue,
    DuplicateCue,
    /// EXT names a hot-cue slot that DAT already holds with different bounds.
    ConflictingCue,
    InvalidIdentity,
}

#[derive(Debug)]
pub struct AnalysisBuilder {
    beats: Vec<Beat>,
    preview: Vec<WaveformColumn>,
    detail: Vec<WaveformColumn>,
    cues: Vec<AnalysisCue>,
    memory_cues: Vec<AnalysisMemoryCue>,
    memory_cues_truncated: bool,
    detail_rate_hz: u16,
    color_preview: Vec<ColorPreviewEntry>,
    color_preview_truncated: bool,
}
impl AnalysisBuilder {
    pub const fn new() -> Self {
        Self {
            beats: Vec::new(),
            preview: Vec::new(),
            detail: Vec::new(),
            cues: Vec::new(),
            memory_cues: Vec::new(),
            memory_cues_truncated: false,
            detail_rate_hz: 0,
            color_preview: Vec::new(),
            color_preview_truncated: false,
        }
    }
    /// Reserves the retained PWV4 columns; `truncated` records that the
    /// export carried more than [`MAX_COLOR_PREVIEW_COLUMNS`].
    pub fn reserve_color_preview(
        &mut self,
        count: usize,
        truncated: bool,
    ) -> Result<(), AnalysisDataError> {
        if count > MAX_COLOR_PREVIEW_COLUMNS || !self.color_preview.is_empty() {
            return Err(AnalysisDataError::Capacity);
        }
        self.color_preview
            .try_reserve_exact(count)
            .map_err(|_| AnalysisDataError::Allocation)?;
        self.color_preview_truncated = truncated;
        Ok(())
    }
    pub fn push_color_preview(
        &mut self,
        entry: ColorPreviewEntry,
    ) -> Result<(), AnalysisDataError> {
        push_bounded(&mut self.color_preview, entry, MAX_COLOR_PREVIEW_COLUMNS)
    }
    pub fn reserve_beats(&mut self, count: usize) -> Result<(), AnalysisDataError> {
        if count > MAX_ANALYSIS_BEATS || !self.beats.is_empty() {
            return Err(AnalysisDataError::Capacity);
        }
        self.beats
            .try_reserve_exact(count)
            .map_err(|_| AnalysisDataError::Allocation)
    }
    pub fn push_beat(&mut self, beat: Beat) -> Result<(), AnalysisDataError> {
        if beat.phase > 3
            || beat.bpm_x100 == 0
            || self
                .beats
                .last()
                .is_some_and(|old| old.time_ms >= beat.time_ms)
        {
            return Err(AnalysisDataError::InvalidBeat);
        }
        push_bounded(&mut self.beats, beat, MAX_ANALYSIS_BEATS)
    }
    pub fn reserve_waveform(
        &mut self,
        detail: bool,
        count: usize,
    ) -> Result<(), AnalysisDataError> {
        let (values, max) = if detail {
            (&mut self.detail, MAX_DETAIL_COLUMNS)
        } else {
            (&mut self.preview, MAX_PREVIEW_COLUMNS)
        };
        if count > max || !values.is_empty() {
            return Err(AnalysisDataError::Capacity);
        }
        values
            .try_reserve_exact(count)
            .map_err(|_| AnalysisDataError::Allocation)
    }
    pub fn push_waveform(
        &mut self,
        detail: bool,
        column: WaveformColumn,
    ) -> Result<(), AnalysisDataError> {
        if detail {
            push_bounded(&mut self.detail, column, MAX_DETAIL_COLUMNS)
        } else {
            push_bounded(&mut self.preview, column, MAX_PREVIEW_COLUMNS)
        }
    }
    pub fn set_detail_rate(&mut self, rate_hz: u16) {
        self.detail_rate_hz = rate_hz;
    }
    pub fn push_cue(&mut self, cue: AnalysisCue) -> Result<(), AnalysisDataError> {
        if cue.slot >= 8 || cue.end_ms.is_some_and(|end| end <= cue.start_ms) {
            return Err(AnalysisDataError::InvalidCue);
        }
        if self.cues.iter().any(|old| old.slot == cue.slot) {
            return Err(AnalysisDataError::DuplicateCue);
        }
        push_bounded(&mut self.cues, cue, 8)
    }
    /// Keeps export order; entries beyond [`MAX_MEMORY_CUES`] set an explicit
    /// truncation flag instead of failing the import. A malformed loop fails.
    pub fn push_memory_cue(&mut self, cue: AnalysisMemoryCue) -> Result<(), AnalysisDataError> {
        if cue.end_ms.is_some_and(|end| end <= cue.start_ms) {
            return Err(AnalysisDataError::InvalidCue);
        }
        if self.memory_cues.len() >= MAX_MEMORY_CUES {
            self.memory_cues_truncated = true;
            return Ok(());
        }
        push_bounded(&mut self.memory_cues, cue, MAX_MEMORY_CUES)
    }
    /// EXT enrichment (M2.5 082d2c0a): EXT may carry hot cues D-H that DAT
    /// lacks. Repeating an identical DAT slot is a no-op; a different cue in an
    /// occupied slot is a conflict that rejects the import.
    pub fn merge_source_cue(&mut self, cue: AnalysisCue) -> Result<(), AnalysisDataError> {
        match self.cues.iter().find(|old| old.slot == cue.slot) {
            Some(old) if *old == cue => Ok(()),
            Some(_) => Err(AnalysisDataError::ConflictingCue),
            None => self.push_cue(cue),
        }
    }
    /// EXT memory cues already present from DAT are skipped; new ones append
    /// in export order under the same bound and truncation flag.
    pub fn merge_memory_cue(&mut self, cue: AnalysisMemoryCue) -> Result<(), AnalysisDataError> {
        if self.memory_cues.contains(&cue) {
            return Ok(());
        }
        self.push_memory_cue(cue)
    }
    pub fn finish(
        self,
        identity: AnalysisIdentity,
        provider: AnalysisProvider,
        fallback_bpm_x100: u32,
    ) -> Result<AnalysisPublication, AnalysisDataError> {
        if identity.load_generation == 0 {
            return Err(AnalysisDataError::InvalidIdentity);
        }
        let bpm_x100 = self
            .beats
            .first()
            .map(|beat| beat.bpm_x100)
            .unwrap_or(fallback_bpm_x100);
        Ok(AnalysisPublication {
            identity,
            provider,
            bpm_x100,
            data: self,
        })
    }
}
impl Default for AnalysisBuilder {
    fn default() -> Self {
        Self::new()
    }
}

fn push_bounded<T>(values: &mut Vec<T>, value: T, max: usize) -> Result<(), AnalysisDataError> {
    if values.len() >= max {
        return Err(AnalysisDataError::Capacity);
    }
    if values.len() == values.capacity() {
        values
            .try_reserve_exact(1)
            .map_err(|_| AnalysisDataError::Allocation)?;
    }
    values.push(value);
    Ok(())
}

#[derive(Debug)]
pub struct AnalysisPublication {
    identity: AnalysisIdentity,
    provider: AnalysisProvider,
    bpm_x100: u32,
    data: AnalysisBuilder,
}
impl AnalysisPublication {
    pub const fn identity(&self) -> AnalysisIdentity {
        self.identity
    }
    pub fn view(&self) -> TrackAnalysis<'_> {
        TrackAnalysis::new(
            self.provider,
            self.identity.load_generation,
            self.bpm_x100,
            (!self.data.beats.is_empty()).then(|| BeatGrid::new(&self.data.beats)),
        )
        .with_visuals(&self.data.preview, &self.data.detail, &self.data.cues)
        .with_memory_cues(&self.data.memory_cues, self.data.memory_cues_truncated)
        .with_detail_rate(self.data.detail_rate_hz)
        .with_color_preview(&self.data.color_preview, self.data.color_preview_truncated)
    }
}

pub struct AnalysisPins {
    slots: [Option<AnalysisPublication>; 2],
}
impl AnalysisPins {
    pub const fn new() -> Self {
        Self {
            slots: [None, None],
        }
    }
    pub fn publish(
        &mut self,
        deck: usize,
        expected: AnalysisIdentity,
        publication: AnalysisPublication,
    ) -> bool {
        if deck >= 2 || expected != publication.identity() || expected.load_generation == 0 {
            return false;
        }
        self.slots[deck] = Some(publication);
        true
    }
    pub fn clear(&mut self, deck: usize) {
        if let Some(slot) = self.slots.get_mut(deck) {
            *slot = None;
        }
    }
    pub fn view(&self, deck: usize, expected: AnalysisIdentity) -> Option<TrackAnalysis<'_>> {
        self.slots
            .get(deck)?
            .as_ref()
            .filter(|data| data.identity() == expected)
            .map(|data| data.view())
    }
}
impl Default for AnalysisPins {
    fn default() -> Self {
        Self::new()
    }
}
