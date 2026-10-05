//! Caller-array preflight for the experimental streaming local-tempo profile.
//! Control, filesystem, futures, executor, stack, catalog and other live banks
//! are additional. This is not a total task budget or a placement recommendation.
use crate::{
    Error,
    source::{PCM_SAMPLES, READ_BYTES},
};
use core::mem::size_of;
use libapta::{
    WaveformColumn,
    analysis::{BIN_CAPACITY, OnsetBin},
    waveform::NormalizedSample,
};
use pajoniiir_track_analysis::Beat;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LocalTempoStorage {
    pub overview_columns: usize,
    pub queue_frames: usize,
    pub beats_per_bank: usize,
    pub onset_bytes: usize,
    pub queue_bytes: usize,
    pub overview_bytes: usize,
    pub rendered_bytes: usize,
    pub beat_banks_bytes: usize,
    pub source_scratch_bytes: usize,
    pub total_bytes: usize,
}
impl LocalTempoStorage {
    /// Size the existing onset/flux, normalized queue, native/rendered overview,
    /// TWO retained beat banks, byte scratch and PCM scratch. All arithmetic is
    /// checked for the compiling target's usize. Inclusive byte ceiling applies
    /// only to these arrays. Beat capacity is caller policy, not a guarantee that
    /// conversion will fit. Additional features require additional storage.
    pub fn plan(
        total_frames: u64,
        frames_per_column: u32,
        queue_frames: usize,
        beats_per_bank: usize,
        maximum_bytes: usize,
    ) -> Result<Self, Error> {
        if total_frames == 0
            || total_frames == u64::MAX
            || queue_frames == 0
            || !(64..=65536).contains(&frames_per_column)
            || !frames_per_column.is_power_of_two()
        {
            return Err(Error::Invalid);
        }
        let overview_columns = usize::try_from(total_frames.div_ceil(u64::from(frames_per_column)))
            .map_err(|_| Error::Capacity)?;
        let bytes = |n: usize, size: usize| n.checked_mul(size).ok_or(Error::Capacity);
        let onset_bytes = bytes(BIN_CAPACITY, size_of::<OnsetBin>() + size_of::<f32>())?;
        let queue_bytes = bytes(queue_frames, size_of::<NormalizedSample>())?;
        let overview_bytes = bytes(overview_columns, size_of::<WaveformColumn>())?;
        let rendered_bytes = bytes(
            overview_columns,
            size_of::<pajoniiir_waveform::WaveformColumn>(),
        )?;
        let beat_banks_bytes = bytes(beats_per_bank, 2 * size_of::<Beat>())?;
        let source_scratch_bytes = READ_BYTES + PCM_SAMPLES * size_of::<f32>();
        let total_bytes = [
            onset_bytes,
            queue_bytes,
            overview_bytes,
            rendered_bytes,
            beat_banks_bytes,
            source_scratch_bytes,
        ]
        .into_iter()
        .try_fold(0usize, |sum, n| sum.checked_add(n).ok_or(Error::Capacity))?;
        if total_bytes > maximum_bytes {
            return Err(Error::Capacity);
        }
        Ok(Self {
            overview_columns,
            queue_frames,
            beats_per_bank,
            onset_bytes,
            queue_bytes,
            overview_bytes,
            rendered_bytes,
            beat_banks_bytes,
            source_scratch_bytes,
            total_bytes,
        })
    }
}

/// Borrowed working arrays for the existing local tempo + meter profile.
/// Retained output banks and source/PCM scratch remain independently owned.
pub struct LocalTempoBuffers<'a> {
    pub queue: &'a mut [NormalizedSample],
    pub columns: &'a mut [WaveformColumn],
    pub bins: &'a mut [OnsetBin],
    pub flux: &'a mut [f32],
}
