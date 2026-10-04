//! Experimental native provider; not enabled in production firmware.
#![no_std]
#![forbid(unsafe_code)]

pub mod source;
pub mod streaming;
pub mod window;
pub mod worker;

use libapta::{FeatureState, FractionalFrame, GridRepresentation, NativeResultInput, SourceInfo};
use pajoniiir_track_analysis::{AnalysisProvider, Beat, BeatGrid, TrackAnalysis};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Capacity,
    Coordinate,
    Generation,
    UnsupportedGrid,
    Invalid,
    Playing,
    Identity,
}

/// Round nonnegative Q32 sample frames to nearest millisecond, ties upward.
pub fn milliseconds(position: FractionalFrame, rate: u32) -> Result<u32, Error> {
    if rate == 0 {
        return Err(Error::Invalid);
    }
    let ms = position
        .rounded_milliseconds(rate)
        .map_err(|_| Error::Coordinate)?;
    u32::try_from(ms).map_err(|_| Error::Coordinate)
}

/// Millibpm to BPM x100, nearest with ties upward, without u32 addition overflow.
pub fn bpm_x100(millibpm: u32) -> u32 {
    ((millibpm as u64 + 5) / 10) as u32
}

/// Independent immutable converted storage. Native source/lineage are retained
/// outside the small neutral view; generation alone is not a source identity.
#[derive(Clone, Copy, Debug)]
pub struct Analysis<'a> {
    pub source: SourceInfo,
    pub lineage: (u64, u64),
    track: TrackAnalysis<'a>,
}
impl<'a> Analysis<'a> {
    pub fn track(&self) -> TrackAnalysis<'a> {
        self.track
    }
}

/// Convert complete continuous segment/explicit grids with a known single meter. Missing
/// or non-final grid/meter yields tempo only; never invent four-beat phase.
/// Input must come from a trusted session/owned result. For external bytes use
/// `convert_cache`; this is subset conversion, not complete graph validation.
/// Preflight is transactional: errors leave destination untouched. Work is
/// bounded by destination length; run this in the analysis worker, not audio.
///
/// The returned view pins its conversion bank against mutation:
/// ```compile_fail
/// fn overwrite(input: &libapta::NativeResultInput<'_>, out: &mut [pajoniiir_track_analysis::Beat]) {
///     let analysis = pajoniiir_apta_adapter::convert(input, out).unwrap();
///     out[0].time_ms = 0;
///     let _ = analysis.track();
/// }
/// ```
pub fn convert<'a>(
    input: &NativeResultInput<'_>,
    destination: &'a mut [Beat],
) -> Result<Analysis<'a>, Error> {
    let generation = u32::try_from(input.info.generation).map_err(|_| Error::Generation)?;
    if generation == 0 {
        return Err(Error::Generation);
    }
    if input.source.sample_rate == 0 {
        return Err(Error::Invalid);
    }
    let bpm = input
        .tempo
        .map_or(0, |t| bpm_x100(t.selected.tempo_millibpm));
    let grid = input.global_grid.or(input.local_grid);
    let usable = match (grid, input.meter, input.source.total_frames) {
        (Some(g), Some(m), Some(total))
            if g.state == FeatureState::Final
                && m.state == FeatureState::Final
                && g.flags & 1 == 0 =>
        {
            if g.coverage_ranges.len() != 1
                || g.coverage_ranges[0].first_frame != 0
                || g.coverage_ranges[0].end_frame != total
            {
                return Err(Error::UnsupportedGrid);
            }
            if m.segments.len() != 1 || !(1..=32).contains(&m.numerator) || m.denominator != 4 {
                return Err(Error::UnsupportedGrid);
            }
            let segment = m.segments[0];
            if segment.state != FeatureState::Final
                || segment.first_frame != 0
                || segment.end_frame != total
                || segment.numerator != m.numerator
                || segment.denominator != m.denominator
                || segment.downbeat_ordinal != m.downbeat_ordinal
                || segment.downbeat_frame != m.downbeat_frame
            {
                return Err(Error::UnsupportedGrid);
            }
            Some((g, m))
        }
        _ => None,
    };
    let mut count = 0usize;
    if let Some((g, m)) = usable {
        if input.global_grid.is_some()
            && input.revision.is_some_and(|r| {
                r.state != libapta::RevisionState::Applied
                    || g.segments.iter().any(|s| s.revision != r.revision_id)
                    || g.beats.iter().any(|b| b.revision != r.revision_id)
            })
        {
            return Err(Error::UnsupportedGrid);
        }
        count = match g.representation {
            GridRepresentation::Segments => {
                let mut end = 0;
                let mut count = 0usize;
                for s in g.segments {
                    if s.applicability_range.first_frame != end
                        || s.applicability_range.end_frame <= end
                        || s.state != FeatureState::Final
                        || s.flags & 1 != 0
                        || s.beat_count == 0
                    {
                        return Err(Error::UnsupportedGrid);
                    }
                    end = s.applicability_range.end_frame;
                    count = count
                        .checked_add(s.beat_count as usize)
                        .ok_or(Error::Capacity)?;
                }
                if end != input.source.total_frames.unwrap() {
                    return Err(Error::UnsupportedGrid);
                }
                count
            }
            GridRepresentation::Explicit => g.beats.len(),
            // The wire/native model has no explicit override ranges to resolve.
            GridRepresentation::Hybrid => return Err(Error::UnsupportedGrid),
        };
        if count > destination.len() {
            return Err(Error::Capacity);
        }
        let beat = |b: libapta::Beat, tempo| -> Result<Beat, Error> {
            if b.position.whole_frame >= input.source.total_frames.unwrap() {
                return Err(Error::Coordinate);
            }
            Ok(Beat {
                time_ms: milliseconds(b.position, input.source.sample_rate)?,
                phase: ((b.ordinal as i128 - m.downbeat_ordinal as i128)
                    .rem_euclid(m.numerator as i128)) as u16,
                bpm_x100: tempo,
            })
        };
        let mut previous: Option<(u32, i64)> = None;
        let mut matching_downbeat = false;
        visit_beats(g, bpm, |b, tempo| {
            if b.flags & 1 != 0 {
                return Err(Error::UnsupportedGrid);
            }
            let converted = beat(b, tempo)?;
            matching_downbeat |=
                b.ordinal == m.downbeat_ordinal && b.position.whole_frame == m.downbeat_frame;
            if let Some((time, ordinal)) = previous {
                if ordinal.checked_add(1) != Some(b.ordinal) {
                    return Err(Error::UnsupportedGrid);
                }
                if time >= converted.time_ms {
                    return Err(Error::Coordinate);
                }
            }
            previous = Some((converted.time_ms, b.ordinal));
            Ok(())
        })?;
        if !matching_downbeat {
            return Err(Error::UnsupportedGrid);
        }
        let mut index = 0;
        visit_beats(g, bpm, |b, tempo| {
            destination[index] = beat(b, tempo)?;
            index += 1;
            Ok(())
        })?;
    }

    Ok(Analysis {
        source: input.source,
        lineage: (input.info.lineage_id_high, input.info.lineage_id_low),
        track: TrackAnalysis::new(
            AnalysisProvider::AptaNative,
            generation,
            bpm,
            (count != 0).then(|| BeatGrid::new(&destination[..count])),
        ),
    })
}

// Two identical linear passes: validate before writing, then populate the bank.
fn visit_beats(
    grid: libapta::NativeGrid<'_>,
    bpm: u32,
    mut visit: impl FnMut(libapta::Beat, u32) -> Result<(), Error>,
) -> Result<(), Error> {
    match grid.representation {
        GridRepresentation::Segments => {
            for segment in grid.segments {
                for index in 0..segment.beat_count {
                    let beat = segment
                        .beat_at(index)
                        .map_err(|_| Error::Coordinate)?
                        .ok_or(Error::Coordinate)?;
                    visit(beat, bpm_x100(segment.nominal_tempo_millibpm))?;
                }
            }
        }
        GridRepresentation::Explicit => {
            for &beat in grid.beats {
                visit(beat, bpm)?;
            }
        }
        GridRepresentation::Hybrid => return Err(Error::UnsupportedGrid),
    }
    Ok(())
}

impl<'a> Analysis<'a> {
    /// Explicit stopped-deck boundary. Caller supplies the actual deck's playing
    /// state. Keep both backing banks alive; errors preserve the pinned view.
    pub fn upgrade(&mut self, next: Self, playing: bool) -> Result<(), Error> {
        if playing {
            return Err(Error::Playing);
        }
        if self.source.fingerprint_kind == 0
            || self.source != next.source
            || self.lineage != next.lineage
            || self.track.provider() != next.track.provider()
        {
            return Err(Error::Identity);
        }
        if next.track.generation() <= self.track.generation() {
            return Err(Error::Generation);
        }
        *self = next;
        Ok(())
    }
}

/// Read a validated local-grid cache without global scratch. Cache files do not persist session
/// generations: the host assigns a nonzero generation in its cache lineage.
/// Identity equality checks supplied bytes, not the source object's hash.
/// Global caches use `convert_cache_with_scratch`; hybrid authority is unsupported.
pub fn convert_cache<'a>(
    bytes: &[u8],
    expected: libapta::session::SourceIdentity,
    generation: u32,
    limits: libapta::result::Limits,
    destination: &'a mut [Beat],
) -> Result<Analysis<'a>, Error> {
    convert_cache_with_scratch(
        bytes,
        expected,
        generation,
        limits,
        &mut [],
        &mut [],
        destination,
    )
}

/// Global cache conversion with caller-owned native segment/beat scratch.
/// Scratch can change on error; the converted destination remains transactional.
/// No returned view borrows the bytes or scratch. Hybrid remains unsupported.
pub fn convert_cache_with_scratch<'a>(
    bytes: &[u8],
    expected: libapta::session::SourceIdentity,
    generation: u32,
    limits: libapta::result::Limits,
    segments: &mut [libapta::GridSegment],
    beats: &mut [libapta::Beat],
    destination: &'a mut [Beat],
) -> Result<Analysis<'a>, Error> {
    let cache = libapta::result::parse(bytes, limits).map_err(|_| Error::Invalid)?;
    if expected.kind() == 0
        || cache.source.fingerprint_kind != expected.kind()
        || cache.source.fingerprint != expected.bytes()
    {
        return Err(Error::Identity);
    }
    let global = cache
        .global_grid
        .map(|g| g.copy_into(segments, beats))
        .transpose()
        .map_err(|_| Error::Capacity)?;
    let global_coverage = global.map(|g| [g.coverage_range]);
    let global_grid = global.map(|g| libapta::NativeGrid {
        state: g.state,
        confidence: g.confidence,
        flags: g.flags,
        representation: g.representation,
        requested_range: g.requested_range,
        evidence_range: g.evidence_range,
        applicability_range: g.applicability_range,
        coverage_ranges: global_coverage.as_ref().unwrap(),
        segments: g.segments,
        beats: g.beats,
    });
    let local = cache.local_grid;
    let coverage = local.map(|g| [g.coverage]);
    let segments = local.map(|g| [g.segment]);
    let local_grid = local.map(|g| libapta::NativeGrid {
        state: g.state,
        confidence: g.confidence,
        flags: g.flags,
        representation: GridRepresentation::Segments,
        requested_range: g.requested_range,
        evidence_range: g.evidence_range,
        applicability_range: g.applicability_range,
        coverage_ranges: coverage.as_ref().unwrap(),
        segments: segments.as_ref().unwrap(),
        beats: &[],
    });
    let meter_segment = libapta::MeterSegment {
        first_frame: 0,
        end_frame: 0,
        downbeat_frame: 0,
        downbeat_ordinal: 0,
        numerator: 4,
        denominator: 4,
        state: FeatureState::Final,
        confidence: 0,
        segment_id: 0,
    };
    let mut meter_storage = [meter_segment];
    let meter = match cache.meter {
        Some(m) => Some(
            m.copy_into(&mut meter_storage)
                .map_err(|_| Error::UnsupportedGrid)?,
        ),
        None => None,
    };
    let input = NativeResultInput {
        source: cache.source,
        info: libapta::NativeResultInfo {
            generation: generation as u64,
            ..Default::default()
        },
        provenance: libapta::Provenance {
            origin: libapta::ProvenanceOrigin::ExternalImport,
            source_name: "APTA cache",
            source_version: "1",
        },
        overview: None,
        detail: None,
        metadata: None,
        tempo: cache.tempo.map(|t| libapta::TempoView {
            selected: t.selected(),
            candidates: &[],
        }),
        local_grid,
        global_grid,
        revision: cache.revision,
        key: None,
        meter,
        quality: &[],
    };
    let mut converted = convert(&input, destination)?;
    converted.track = TrackAnalysis::new(
        AnalysisProvider::AptaCache,
        generation,
        converted.track.bpm_x100(),
        converted.track.beat_grid(),
    );
    Ok(converted)
}

/// Full continuous overview only. The renderer stretches this whole-track view;
/// sparse/detail viewport selection needs a separate coordinate-aware contract.
/// Peak conversion maps signed Q15 magnitude to u16 with nearest rounding;
/// -32768 saturates to full scale. Color is a product choice supplied by caller.
pub fn waveform<'a>(
    overview: libapta::NativeOverview<'_>,
    destination: &'a mut [pajoniiir_waveform::WaveformColumn],
    color_rgb565: u16,
) -> Result<pajoniiir_waveform::WaveformView<'a>, Error> {
    if overview.state != FeatureState::Final
        || overview.origin_frame != 0
        || overview.spans.len() != 1
    {
        return Err(Error::UnsupportedGrid);
    }
    let span = overview.spans[0];
    if span.first_frame != 0
        || span.first_column_index != 0
        || span.data_column_offset != 0
        || span.column_count as usize != overview.columns.len()
    {
        return Err(Error::Invalid);
    }
    if destination.len() < overview.columns.len() {
        return Err(Error::Capacity);
    }
    if overview
        .columns
        .iter()
        .any(|c| c.minimum > c.maximum || c.flags & 1 == 0)
    {
        return Err(Error::Invalid);
    }
    for (out, c) in destination.iter_mut().zip(overview.columns) {
        let peak = u32::from(c.minimum.unsigned_abs().max(c.maximum.unsigned_abs())).min(32767);
        *out = pajoniiir_waveform::WaveformColumn {
            peak: ((peak * 65535 + 16383) / 32767) as u16,
            color_rgb565,
        };
    }
    Ok(pajoniiir_waveform::WaveformView {
        columns: &destination[..overview.columns.len()],
    })
}
