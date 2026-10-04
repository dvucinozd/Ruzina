//! Whole-column waveform windows for the existing renderer. Sparse holes fail
//! explicitly; callers choose overview or detail, without inventing coverage.
use crate::Error;
use libapta::{FrameRange, NativeDetail, NativeOverview, WaveformColumn};
use pajoniiir_waveform::{WaveformColumn as RenderColumn, WaveformView};

/// Select an aligned, fully covered overview window from a trusted result.
/// Partial overview state is allowed: selected published columns must be valid.
/// Arbitrary sub-column cropping and partial EOF columns remain unsupported.
pub fn overview<'a>(
    source: NativeOverview<'_>,
    range: FrameRange,
    output: &'a mut [RenderColumn],
    color: u16,
) -> Result<WaveformView<'a>, Error> {
    window(
        source.origin_frame,
        source.frames_per_column as u64,
        range,
        output,
        color,
        |first, end| {
            let span = source
                .spans
                .iter()
                .find(|s| s.first_frame <= first && s.end_frame >= end)
                .ok_or(Error::UnsupportedGrid)?;
            let index = first
                .checked_sub(source.origin_frame)
                .ok_or(Error::Coordinate)?
                / source.frames_per_column as u64;
            let offset = index
                .checked_sub(span.first_column_index as u64)
                .ok_or(Error::Invalid)?;
            if offset >= span.column_count as u64 {
                return Err(Error::Invalid);
            }
            let data = u64::from(span.data_column_offset)
                .checked_add(offset)
                .ok_or(Error::Coordinate)?;
            source
                .columns
                .get(usize::try_from(data).map_err(|_| Error::Coordinate)?)
                .copied()
                .ok_or(Error::Invalid)
        },
    )
}

/// Select an aligned level-1 detail window. Every requested column must have
/// published data, including across adjacent tiles. No overview fallback.
pub fn detail<'a>(
    source: NativeDetail<'_>,
    range: FrameRange,
    output: &'a mut [RenderColumn],
    color: u16,
) -> Result<WaveformView<'a>, Error> {
    window(0, 256, range, output, color, |first, end| {
        let tile = source
            .tiles
            .iter()
            .find(|t| t.level_id == 1 && t.first_frame <= first && t.end_frame >= end)
            .ok_or(Error::UnsupportedGrid)?;
        let offset = (first / 256)
            .checked_sub(tile.first_column_index as u64)
            .ok_or(Error::Invalid)?;
        let offset = usize::try_from(offset).map_err(|_| Error::Coordinate)?;
        if offset >= tile.column_count {
            return Err(Error::Invalid);
        }
        source
            .columns
            .get(
                tile.data_column_offset
                    .checked_add(offset)
                    .ok_or(Error::Coordinate)?,
            )
            .copied()
            .ok_or(Error::Invalid)
    })
}

fn window<'a>(
    origin: u64,
    frames: u64,
    range: FrameRange,
    output: &'a mut [RenderColumn],
    color: u16,
    column: impl Fn(u64, u64) -> Result<WaveformColumn, Error>,
) -> Result<WaveformView<'a>, Error> {
    if frames == 0
        || range.first_frame < origin
        || range.end_frame <= range.first_frame
        || !(range.first_frame - origin).is_multiple_of(frames)
        || !(range.end_frame - origin).is_multiple_of(frames)
    {
        return Err(Error::Coordinate);
    }
    let count = usize::try_from((range.end_frame - range.first_frame) / frames)
        .map_err(|_| Error::Capacity)?;
    if output.len() < count {
        return Err(Error::Capacity);
    }
    let at = |index: usize| -> Result<RenderColumn, Error> {
        // index is bounded by the validated range difference above.
        let first = range.first_frame + index as u64 * frames;
        let c = column(first, first + frames)?;
        if c.flags & 1 == 0 || c.minimum > c.maximum {
            return Err(Error::Invalid);
        }
        let peak = u32::from(c.minimum.unsigned_abs().max(c.maximum.unsigned_abs())).min(32767);
        Ok(RenderColumn {
            peak: ((peak * 65535 + 16383) / 32767) as u16,
            color_rgb565: color,
        })
    };
    for i in 0..count {
        at(i)?;
    }
    for (i, out) in output[..count].iter_mut().enumerate() {
        *out = at(i)?;
    }
    Ok(WaveformView {
        columns: &output[..count],
    })
}
