//! The phase strip's figure, for the deck screen's redesign: how far the other deck's
//! beat is ahead of the master's. The strip shows it between the lanes; nothing here
//! changes playback.
//!
//! Not on the screen yet: the redesign wires it in.
#![cfg_attr(not(test), allow(dead_code))]

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
    fn no_offset_outside_a_grid_or_without_a_tempo() {
        let beats = grid(120, 1_000, 64);
        let master = deck(&beats, 10_000, 12_000);
        assert_eq!(beat_offset_ms(master, deck(&beats, 500, 12_000)), None);
        assert_eq!(beat_offset_ms(master, deck(&beats, 40_000, 12_000)), None);
        assert_eq!(beat_offset_ms(deck(&beats, 10_000, 0), master), None);
        assert_eq!(beat_offset_ms(master, deck(&[], 10_000, 12_000)), None);
    }
}
