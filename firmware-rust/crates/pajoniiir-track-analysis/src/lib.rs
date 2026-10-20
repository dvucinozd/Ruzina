#![no_std]
#![forbid(unsafe_code)]

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Beat {
    pub time_ms: u32,
    pub phase: u16,
    pub bpm_x100: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnalysisProvider {
    RekordboxImport,
    AptaCache,
    AptaNative,
}

#[derive(Clone, Copy, Debug)]
pub struct TrackAnalysis<'a> {
    provider: AnalysisProvider,
    generation: u32,
    bpm_x100: u32,
    beat_grid: Option<BeatGrid<'a>>,
}

impl<'a> TrackAnalysis<'a> {
    pub const fn new(
        provider: AnalysisProvider,
        generation: u32,
        bpm_x100: u32,
        beat_grid: Option<BeatGrid<'a>>,
    ) -> Self {
        Self {
            provider,
            generation,
            bpm_x100,
            beat_grid,
        }
    }

    pub const fn provider(&self) -> AnalysisProvider {
        self.provider
    }

    pub const fn generation(&self) -> u32 {
        self.generation
    }

    pub const fn bpm_x100(&self) -> u32 {
        self.bpm_x100
    }

    pub const fn beat_grid(&self) -> Option<BeatGrid<'a>> {
        self.beat_grid
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BeatGrid<'a> {
    beats: &'a [Beat],
}

impl<'a> BeatGrid<'a> {
    pub const fn new(beats: &'a [Beat]) -> Self {
        Self { beats }
    }

    pub const fn beats(&self) -> &'a [Beat] {
        self.beats
    }

    pub const fn is_empty(&self) -> bool {
        self.beats.is_empty()
    }

    pub fn nearest_index(&self, position_ms: u32) -> Option<usize> {
        let mut closest = None;
        let mut min_diff = u32::MAX;

        for (index, beat) in self.beats.iter().enumerate() {
            let diff = position_ms.abs_diff(beat.time_ms);
            if diff < min_diff {
                min_diff = diff;
                closest = Some(index);
            }
        }

        closest
    }
}

pub fn beat_jump_target_ms(
    position_ms: u32,
    bpm_x100: u32,
    beat_numerator: i32,
    beat_denominator: u16,
    beat_grid: Option<BeatGrid<'_>>,
) -> u32 {
    if beat_numerator == 0 {
        return position_ms;
    }

    let denominator = beat_denominator.max(1) as u64;

    if let Some(grid) = beat_grid.filter(|grid| !grid.is_empty()) {
        let closest_index = grid.nearest_index(position_ms).unwrap_or(0);

        if beat_numerator % denominator as i32 == 0 {
            let shift = beat_numerator / denominator as i32;
            let target = (closest_index as i64 + shift as i64)
                .clamp(0, grid.beats().len().saturating_sub(1) as i64)
                as usize;
            return grid.beats()[target].time_ms;
        }

        let forward = beat_numerator > 0;
        if (forward && closest_index + 1 >= grid.beats().len()) || (!forward && closest_index == 0)
        {
            return grid.beats()[closest_index].time_ms;
        }

        let adjacent_index = if forward {
            closest_index + 1
        } else {
            closest_index - 1
        };
        let closest_ms = grid.beats()[closest_index].time_ms;
        let adjacent_ms = grid.beats()[adjacent_index].time_ms;
        let interval_ms = closest_ms.abs_diff(adjacent_ms) as u64;
        let magnitude = beat_numerator.unsigned_abs() as u64;
        let delta_ms = (interval_ms * magnitude).div_ceil(denominator);

        if forward {
            return (closest_ms as u64 + delta_ms).min(u32::MAX as u64) as u32;
        }
        return if delta_ms >= closest_ms as u64 {
            0
        } else {
            closest_ms - delta_ms as u32
        };
    }

    let safe_bpm_x100 = if bpm_x100 == 0 { 12_000 } else { bpm_x100 };
    let beat_len_ms = 6_000_000u64 / safe_bpm_x100 as u64;
    let magnitude = beat_numerator.unsigned_abs() as u64;
    let delta_ms = (beat_len_ms * magnitude).div_ceil(denominator);

    if beat_numerator < 0 {
        if delta_ms >= position_ms as u64 {
            0
        } else {
            position_ms - delta_ms as u32
        }
    } else {
        (position_ms as u64 + delta_ms).min(u32::MAX as u64) as u32
    }
}

pub fn beat_loop_duration_ms(
    position_ms: u32,
    bpm_x100: u32,
    beat_numerator: u16,
    beat_denominator: u16,
    beat_grid: Option<BeatGrid<'_>>,
) -> u32 {
    let mut beat_len_ms = 0u32;

    if let Some(grid) = beat_grid.filter(|grid| grid.beats().len() >= 2) {
        let closest_index = grid.nearest_index(position_ms).unwrap_or(0);
        beat_len_ms = if closest_index + 1 < grid.beats().len() {
            grid.beats()[closest_index + 1]
                .time_ms
                .saturating_sub(grid.beats()[closest_index].time_ms)
        } else {
            grid.beats()[closest_index]
                .time_ms
                .saturating_sub(grid.beats()[closest_index - 1].time_ms)
        };
    }

    if beat_len_ms == 0 {
        let safe_bpm_x100 = if bpm_x100 == 0 { 12_000 } else { bpm_x100 };
        beat_len_ms = (6_000_000u64 / safe_bpm_x100 as u64) as u32;
    }

    let numerator = beat_numerator.max(1) as u64;
    let denominator = beat_denominator.max(1) as u64;
    let duration = (beat_len_ms as u64 * numerator).div_ceil(denominator);
    duration.max(1).min(u32::MAX as u64) as u32
}

/// Align a target playhead to the reference deck's beat phase while preserving
/// the reference deck's signed intra-beat offset.
///
/// This is the provider-neutral equivalent of the released M2.2 phase-align
/// behavior. Rekordbox and future APTA providers only supply immutable BeatGrid
/// values.
pub fn phase_align_target_ms(
    target_position_ms: u32,
    target: BeatGrid<'_>,
    reference_position_ms: u32,
    reference: BeatGrid<'_>,
) -> Option<u32> {
    let reference_index = reference.nearest_index(reference_position_ms)?;
    let reference_beat = reference.beats()[reference_index];
    let target_phase = reference_beat.phase;
    let reference_offset_ms = reference_position_ms as i64 - reference_beat.time_ms as i64;

    let mut best = None;
    let mut min_diff = u32::MAX;

    for beat in target.beats() {
        if beat.phase != target_phase {
            continue;
        }

        let candidate = (beat.time_ms as i64 + reference_offset_ms).max(0);
        let candidate_ms = candidate.min(u32::MAX as i64) as u32;
        let diff = target_position_ms.abs_diff(candidate_ms);

        if best.is_none() || diff < min_diff {
            best = Some(candidate_ms);
            min_diff = diff;
        }
    }

    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_track_analysis_keeps_provider_and_generation_explicit() {
        let beats = [Beat {
            time_ms: 1000,
            phase: 0,
            bpm_x100: 12_000,
        }];
        let analysis = TrackAnalysis::new(
            AnalysisProvider::RekordboxImport,
            7,
            12_000,
            Some(BeatGrid::new(&beats)),
        );

        assert_eq!(analysis.provider(), AnalysisProvider::RekordboxImport);
        assert_eq!(analysis.generation(), 7);
        assert_eq!(analysis.bpm_x100(), 12_000);
        assert_eq!(analysis.beat_grid().unwrap().beats(), &beats);
    }

    #[test]
    fn nearest_beat_prefers_first_entry_on_equal_distance() {
        let beats = [
            Beat {
                time_ms: 1000,
                phase: 0,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 2000,
                phase: 1,
                bpm_x100: 12_000,
            },
        ];

        let grid = BeatGrid::new(&beats);
        assert_eq!(grid.nearest_index(1500), Some(0));
    }

    #[test]
    fn released_integer_jump_uses_nearest_grid_entry() {
        let beats = [
            Beat {
                time_ms: 1000,
                phase: 0,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 2000,
                phase: 1,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 3000,
                phase: 2,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 4000,
                phase: 3,
                bpm_x100: 12_000,
            },
        ];
        let grid = BeatGrid::new(&beats);

        assert_eq!(beat_jump_target_ms(2200, 12_000, 1, 1, Some(grid)), 3000);
        assert_eq!(beat_jump_target_ms(2800, 12_000, -1, 1, Some(grid)), 2000);
        assert_eq!(beat_jump_target_ms(900, 12_000, -32, 1, Some(grid)), 1000);
        assert_eq!(beat_jump_target_ms(3800, 12_000, 32, 1, Some(grid)), 4000);
    }

    #[test]
    fn released_fractional_jump_uses_local_grid_spacing() {
        let beats = [
            Beat {
                time_ms: 1000,
                phase: 0,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 1501,
                phase: 1,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 2000,
                phase: 2,
                bpm_x100: 12_000,
            },
        ];
        let grid = BeatGrid::new(&beats);

        assert_eq!(beat_jump_target_ms(1010, 12_000, 1, 16, Some(grid)), 1032);
        assert_eq!(beat_jump_target_ms(1490, 12_000, -1, 16, Some(grid)), 1469);
        assert_eq!(beat_jump_target_ms(1490, 12_000, 1, 2, Some(grid)), 1751);
        assert_eq!(beat_jump_target_ms(900, 12_000, -1, 16, Some(grid)), 1000);
        assert_eq!(beat_jump_target_ms(2100, 12_000, 1, 16, Some(grid)), 2000);
    }

    #[test]
    fn released_jump_falls_back_to_bpm_and_clamps() {
        assert_eq!(beat_jump_target_ms(1000, 12_000, 4, 1, None), 3000);
        assert_eq!(beat_jump_target_ms(1000, 12_000, -8, 1, None), 0);
        assert_eq!(beat_jump_target_ms(1000, 0, 1, 1, None), 1500);
        assert_eq!(beat_jump_target_ms(1000, 12_000, 1, 16, None), 1032);
        assert_eq!(beat_jump_target_ms(10, 12_000, -1, 16, None), 0);
        assert_eq!(
            beat_jump_target_ms(u32::MAX - 10, 12_000, 1, 16, None),
            u32::MAX
        );
    }

    #[test]
    fn released_loop_duration_uses_local_grid_or_bpm_fallback() {
        let beats = [
            Beat {
                time_ms: 1000,
                phase: 0,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 1500,
                phase: 1,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 2000,
                phase: 2,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 2500,
                phase: 3,
                bpm_x100: 12_000,
            },
        ];
        let grid = BeatGrid::new(&beats);

        assert_eq!(beat_loop_duration_ms(1750, 12_000, 1, 1, Some(grid)), 500);
        assert_eq!(beat_loop_duration_ms(1750, 12_000, 4, 1, Some(grid)), 2000);
        assert_eq!(beat_loop_duration_ms(1000, 12_000, 1, 2, None), 250);
        assert_eq!(beat_loop_duration_ms(1000, 12_000, 1, 4, None), 125);
        assert_eq!(beat_loop_duration_ms(1000, 12_000, 1, 32, None), 16);
        assert_eq!(beat_loop_duration_ms(1000, 0, 2, 1, None), 1000);
    }

    #[test]
    fn released_phase_align_fixture_returns_1962_ms() {
        let target_beats = [
            Beat {
                time_ms: 1000,
                phase: 0,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 1500,
                phase: 1,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 2000,
                phase: 2,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 2500,
                phase: 3,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 3000,
                phase: 0,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 3500,
                phase: 1,
                bpm_x100: 12_000,
            },
        ];
        let reference_beats = [
            Beat {
                time_ms: 8000,
                phase: 0,
                bpm_x100: 12_800,
            },
            Beat {
                time_ms: 8469,
                phase: 1,
                bpm_x100: 12_800,
            },
            Beat {
                time_ms: 8938,
                phase: 2,
                bpm_x100: 12_800,
            },
            Beat {
                time_ms: 9407,
                phase: 3,
                bpm_x100: 12_800,
            },
        ];

        assert_eq!(
            phase_align_target_ms(
                2600,
                BeatGrid::new(&target_beats),
                8900,
                BeatGrid::new(&reference_beats),
            ),
            Some(1962)
        );
    }

    #[test]
    fn phase_align_requires_both_nonempty_grids() {
        let target_beats = [Beat {
            time_ms: 1000,
            phase: 0,
            bpm_x100: 12_000,
        }];
        let empty: [Beat; 0] = [];

        assert_eq!(
            phase_align_target_ms(
                1000,
                BeatGrid::new(&target_beats),
                1000,
                BeatGrid::new(&empty),
            ),
            None
        );
        assert_eq!(
            phase_align_target_ms(
                1000,
                BeatGrid::new(&empty),
                1000,
                BeatGrid::new(&target_beats),
            ),
            None
        );
    }

    #[test]
    fn negative_reference_offset_clamps_candidate_to_zero() {
        let target_beats = [Beat {
            time_ms: 100,
            phase: 0,
            bpm_x100: 12_000,
        }];
        let reference_beats = [Beat {
            time_ms: 1000,
            phase: 0,
            bpm_x100: 12_000,
        }];

        assert_eq!(
            phase_align_target_ms(
                0,
                BeatGrid::new(&target_beats),
                0,
                BeatGrid::new(&reference_beats),
            ),
            Some(0)
        );
    }

    #[test]
    fn missing_matching_phase_returns_none() {
        let target_beats = [Beat {
            time_ms: 1000,
            phase: 0,
            bpm_x100: 12_000,
        }];
        let reference_beats = [Beat {
            time_ms: 1000,
            phase: 2,
            bpm_x100: 12_000,
        }];

        assert_eq!(
            phase_align_target_ms(
                1000,
                BeatGrid::new(&target_beats),
                1000,
                BeatGrid::new(&reference_beats),
            ),
            None
        );
    }
}
