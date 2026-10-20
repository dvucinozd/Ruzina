//! Bounded owned data, built outside render/control and immutable after publish.
use crate::{AnalysisCue, AnalysisProvider, Beat, BeatGrid, TrackAnalysis, WaveformColumn};
use alloc::vec::Vec;
use pajoniiir_core::MediaTrackId;
use pajoniiir_media_session::MediaLease;

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
    InvalidIdentity,
}

#[derive(Debug)]
pub struct AnalysisBuilder {
    beats: Vec<Beat>,
    preview: Vec<WaveformColumn>,
    detail: Vec<WaveformColumn>,
    cues: Vec<AnalysisCue>,
    detail_rate_hz: u16,
}
impl AnalysisBuilder {
    pub const fn new() -> Self {
        Self {
            beats: Vec::new(),
            preview: Vec::new(),
            detail: Vec::new(),
            cues: Vec::new(),
            detail_rate_hz: 0,
        }
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
        .with_detail_rate(self.data.detail_rate_hz)
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
