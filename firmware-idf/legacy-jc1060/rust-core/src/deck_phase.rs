//! The phase strip between the lanes: each deck's beats near its playhead, in real time
//! at a common scale, and how far the other deck's beat is ahead of the master's. Nothing
//! here changes playback.
#![cfg_attr(not(any(test, feature = "slint-ui")), allow(dead_code))]

use crate::deck_engine::beat_at;
use ruzina_track_analysis::Beat;

/// One deck as the phase strip sees it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PhaseDeck<'a> {
    pub(crate) beats: &'a [Beat],
    pub(crate) position_ms: u32,
    /// The tempo it plays at, its pitch included, in hundredths of a BPM.
    pub(crate) bpm_x100: u32,
}

/// How far `other`'s beat is ahead of `master`'s, in milliseconds at the master's tempo,
/// within half a beat: + ahead. `None` when either deck lies outside its beat grid or the
/// master has no tempo.
pub(crate) fn beat_offset_ms(master: PhaseDeck<'_>, other: PhaseDeck<'_>) -> Option<i32> {
    let (_, master_into) = beat_at(master.beats, master.position_ms)?;
    let (_, other_into) = beat_at(other.beats, other.position_ms)?;
    if master.bpm_x100 == 0 {
        return None;
    }
    let ahead = wrap(other_into - master_into);
    Some(round(ahead * 6_000_000.0 / master.bpm_x100 as f32))
}

/// The most beats one deck shows on the strip.
pub(crate) const MAX_TICKS: usize = 8;

/// The columns of the strip where a deck's beats fall, left to right.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Ticks {
    columns: [u16; MAX_TICKS],
    count: u8,
}

impl Ticks {
    pub(crate) fn columns(&self) -> &[u16] {
        &self.columns[..usize::from(self.count)]
    }
}

/// The tempo a deck plays at: its track's `bpm_x100` with its pitch, in hundredths of a
/// BPM (`pitch_centipercent` +800 is +8 %).
pub(crate) fn playing_bpm_x100(bpm_x100: u32, pitch_centipercent: i32) -> u32 {
    let scaled = i64::from(bpm_x100) * (10_000 + i64::from(pitch_centipercent)) / 10_000;
    scaled.clamp(0, i64::from(u32::MAX)) as u32
}

/// The real time the strip spans: `beats` of the master at `bpm_x100`, or 2 s without a
/// tempo.
pub(crate) fn span_ms(bpm_x100: u32, beats: u32) -> u32 {
    if bpm_x100 == 0 {
        return 2_000;
    }
    (u64::from(beats) * 6_000_000 / u64::from(bpm_x100)) as u32
}

/// Where `beats` fall on a strip `width` columns wide that spans `span_ms` of real time
/// with the playhead, `position_ms` into the track, in its middle. A deck pitched up plays
/// its track faster, so its beats come closer in real time.
pub(crate) fn beat_columns(
    beats: &[Beat],
    position_ms: u32,
    pitch_centipercent: i32,
    span_ms: u32,
    width: usize,
) -> Ticks {
    let mut ticks = Ticks::default();
    let rate = 10_000 + i64::from(pitch_centipercent);
    if rate <= 0 || span_ms == 0 || width == 0 {
        return ticks;
    }
    let (span, width) = (i64::from(span_ms), width as i64);
    // Track time from the playhead to either end of the strip.
    let reach = span * rate / 20_000;
    let now = i64::from(position_ms);
    let first = beats.partition_point(|beat| i64::from(beat.time_ms) < now - reach);
    for beat in &beats[first..] {
        let track_ms = i64::from(beat.time_ms) - now;
        if track_ms > reach {
            break;
        }
        let real_ms = track_ms * 10_000 / rate;
        let column = width / 2 + real_ms * width / span;
        if (0..width).contains(&column) && usize::from(ticks.count) < MAX_TICKS {
            ticks.columns[usize::from(ticks.count)] = column as u16;
            ticks.count += 1;
        }
    }
    ticks
}

/// `beats` brought into `[-0.5, 0.5)`.
fn wrap(beats: f32) -> f32 {
    let mut beats = beats % 1.0;
    if beats >= 0.5 {
        beats -= 1.0;
    } else if beats < -0.5 {
        beats += 1.0;
    }
    beats
}

fn round(value: f32) -> i32 {
    if value >= 0.0 {
        (value + 0.5) as i32
    } else {
        (value - 0.5) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    /// A grid at `bpm`, from `first_ms`.
    fn grid(bpm: u32, first_ms: u32, beats: usize) -> Vec<Beat> {
        let length = 60_000 / bpm;
        (0..beats)
            .map(|i| Beat {
                time_ms: first_ms + i as u32 * length,
                phase: i as u16 % 4 + 1,
                bpm_x100: bpm * 100,
            })
            .collect()
    }

    fn deck(beats: &[Beat], position_ms: u32, bpm_x100: u32) -> PhaseDeck<'_> {
        PhaseDeck {
            beats,
            position_ms,
            bpm_x100,
        }
    }

    #[test]
    fn beat_offset_is_within_half_a_beat_at_the_master_tempo() {
        // 120 BPM: a beat is 500 ms.
        let beats = grid(120, 1_000, 64);
        let master = deck(&beats, 10_000, 12_000);
        // 60 ms later in the same grid: 60 ms ahead.
        assert_eq!(
            beat_offset_ms(master, deck(&beats, 10_060, 12_000)),
            Some(60)
        );
        assert_eq!(
            beat_offset_ms(master, deck(&beats, 9_940, 12_000)),
            Some(-60)
        );
        // 440 ms ahead is 60 ms behind the next beat.
        assert_eq!(
            beat_offset_ms(master, deck(&beats, 10_440, 12_000)),
            Some(-60)
        );
        // A whole beat apart: in phase.
        assert_eq!(
            beat_offset_ms(master, deck(&beats, 10_500, 12_000)),
            Some(0)
        );
    }

    #[test]
    fn beat_offset_is_in_milliseconds_at_the_master_tempo() {
        // The other track at 100 BPM (600 ms a beat), a tenth into its beat; the master at
        // 120 BPM, on its beat: a tenth of the master's 500 ms beat ahead.
        let master_beats = grid(120, 0, 64);
        let other_beats = grid(100, 0, 64);
        let master = deck(&master_beats, 5_000, 12_000);
        assert_eq!(
            beat_offset_ms(master, deck(&other_beats, 6_060, 12_000)),
            Some(50)
        );
    }

    #[test]
    fn beats_fall_on_the_strip_in_real_time() {
        // 120 BPM from 0: a beat each 500 ms. Four beats span 2 s on 800 columns, so a
        // beat is 200 columns; the playhead 100 ms past a beat puts it 40 columns left of
        // the middle.
        let beats = grid(120, 0, 64);
        let span = span_ms(12_000, 4);
        assert_eq!(span, 2_000);
        let ticks = beat_columns(&beats, 10_100, 0, span, 800);
        assert_eq!(ticks.columns(), &[160, 360, 560, 760]);
        // Pitched up 25 %, the same beats come 400 ms apart in real time: 160 columns.
        let fast = beat_columns(&beats, 10_100, 2_500, span, 800);
        assert_eq!(fast.columns(), &[48, 208, 368, 528, 688]);
        // Nothing to show without beats or a span.
        assert!(beat_columns(&[], 10_100, 0, span, 800).columns().is_empty());
        assert!(beat_columns(&beats, 10_100, 0, 0, 800).columns().is_empty());
        assert_eq!(playing_bpm_x100(12_000, 2_500), 15_000);
        assert_eq!(playing_bpm_x100(12_000, -800), 11_040);
        assert_eq!(span_ms(0, 4), 2_000);
    }

    #[test]
    fn no_offset_outside_a_grid_or_without_a_tempo() {
        let beats = grid(120, 1_000, 64);
        let master = deck(&beats, 10_000, 12_000);
        assert_eq!(beat_offset_ms(master, deck(&beats, 500, 12_000)), None);
        assert_eq!(beat_offset_ms(master, deck(&beats, 40_000, 12_000)), None);
        assert_eq!(beat_offset_ms(deck(&beats, 10_000, 0), master), None);
        assert_eq!(beat_offset_ms(master, deck(&[], 10_000, 12_000)), None);
    }
}
