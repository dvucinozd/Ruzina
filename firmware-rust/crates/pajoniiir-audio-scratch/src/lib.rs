#![no_std]
#![forbid(unsafe_code)]

use pajoniiir_audio_dsp::PcmFrame;

pub const DEFAULT_FRAMES_PER_TICK: f32 = 250.0;
pub const DEFAULT_RATE_WINDOW: u32 = 256;
pub const DEFAULT_SLEW_COEF: f32 = 0.18;
pub const DEFAULT_VELOCITY_MAX: f32 = 6.0;
pub const DEFAULT_HOLD_WINDOWS: u32 = 3;
pub const SILENCE_VELOCITY: f32 = 0.001;

const FRAMES_PER_TICK_MAX: f32 = 100_000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScratchConfig {
    pub frames_per_tick: f32,
    pub rate_window_samples: u32,
    pub slew_coef: f32,
    pub velocity_max: f32,
    pub hold_windows: u32,
}

impl Default for ScratchConfig {
    fn default() -> Self {
        Self {
            frames_per_tick: DEFAULT_FRAMES_PER_TICK,
            rate_window_samples: DEFAULT_RATE_WINDOW,
            slew_coef: DEFAULT_SLEW_COEF,
            velocity_max: DEFAULT_VELOCITY_MAX,
            hold_windows: DEFAULT_HOLD_WINDOWS,
        }
    }
}

impl ScratchConfig {
    pub fn sanitized(
        frames_per_tick: f32,
        rate_window_samples: u32,
        slew_coef: f32,
        velocity_max: f32,
        hold_windows: u32,
    ) -> Self {
        let frames_per_tick = if !frames_per_tick.is_finite() || frames_per_tick <= 0.0 {
            DEFAULT_FRAMES_PER_TICK
        } else {
            frames_per_tick.min(FRAMES_PER_TICK_MAX)
        };
        let slew_coef = if !slew_coef.is_finite() {
            DEFAULT_SLEW_COEF
        } else {
            slew_coef.clamp(0.0, 1.0)
        };
        let velocity_max = if !velocity_max.is_finite() || velocity_max <= 0.0 {
            DEFAULT_VELOCITY_MAX
        } else {
            velocity_max
        };

        Self {
            frames_per_tick,
            rate_window_samples: rate_window_samples.max(1),
            slew_coef,
            velocity_max,
            hold_windows,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScratchState {
    head_back: f32,
    velocity: f32,
    velocity_target: f32,
    pending_ticks: i32,
    window_pos: u32,
    empty_windows: u32,
    config: ScratchConfig,
    edge_latch: i8,
    edge_hits: u32,
    active: bool,
}

impl ScratchState {
    pub fn new() -> Self {
        Self {
            head_back: 0.0,
            velocity: 0.0,
            velocity_target: 0.0,
            pending_ticks: 0,
            window_pos: 0,
            empty_windows: 0,
            config: ScratchConfig::default(),
            edge_latch: 0,
            edge_hits: 0,
            active: false,
        }
    }

    pub fn configure(&mut self, config: ScratchConfig) {
        self.config = ScratchConfig::sanitized(
            config.frames_per_tick,
            config.rate_window_samples,
            config.slew_coef,
            config.velocity_max,
            config.hold_windows,
        );
    }

    pub fn seed(&mut self, head_back: f32) {
        self.head_back = if head_back.is_finite() {
            head_back.max(0.0)
        } else {
            0.0
        };
        self.velocity = 0.0;
        self.velocity_target = 0.0;
        self.pending_ticks = 0;
        self.window_pos = 0;
        self.empty_windows = 0;
        self.edge_latch = 0;
        self.active = true;
    }

    pub fn end(&mut self) {
        self.active = false;
        self.velocity = 0.0;
        self.velocity_target = 0.0;
        self.pending_ticks = 0;
        self.edge_latch = 0;
    }

    pub fn jog(&mut self, ticks: i16) {
        if ticks == 0 {
            return;
        }
        self.pending_ticks = self.pending_ticks.saturating_add(ticks as i32);
    }

    pub const fn head_back(&self) -> f32 {
        self.head_back
    }

    pub const fn velocity(&self) -> f32 {
        self.velocity
    }

    pub const fn velocity_target(&self) -> f32 {
        self.velocity_target
    }

    pub const fn edge_latch(&self) -> i8 {
        self.edge_latch
    }

    pub const fn edge_hits(&self) -> u32 {
        self.edge_hits
    }

    pub const fn is_active(&self) -> bool {
        self.active
    }

    pub fn render<F>(&mut self, filled: u32, mut read_frame_back: F) -> Option<PcmFrame>
    where
        F: FnMut(u32) -> Option<PcmFrame>,
    {
        if !self.active {
            return None;
        }

        self.update_velocity();

        if self.edge_latch > 0 {
            if self.velocity < -SILENCE_VELOCITY || self.velocity_target < -SILENCE_VELOCITY {
                self.edge_latch = 0;
            } else {
                self.velocity = 0.0;
                return None;
            }
        } else if self.edge_latch < 0 {
            if self.velocity > SILENCE_VELOCITY || self.velocity_target > SILENCE_VELOCITY {
                self.edge_latch = 0;
            } else {
                self.velocity = 0.0;
                return None;
            }
        }

        if filled < 2 {
            return None;
        }
        let max_back = (filled - 1) as f32;

        if self.velocity > -SILENCE_VELOCITY && self.velocity < SILENCE_VELOCITY {
            return None;
        }

        self.head_back = self.head_back.clamp(0.0, max_back);

        let past_new_edge = self.head_back <= 0.0 && self.velocity > 0.0;
        let past_old_edge = self.head_back >= max_back && self.velocity < 0.0;
        if past_new_edge || past_old_edge {
            self.edge_latch = if past_new_edge { 1 } else { -1 };
            self.edge_hits = self.edge_hits.saturating_add(1);
            self.velocity = 0.0;
            return None;
        }

        let mut k0 = self.head_back as u32;
        let mut fraction = self.head_back - k0 as f32;
        if k0 + 1 > filled - 1 {
            k0 = filled - 2;
            fraction = 1.0;
        }

        let newer = read_frame_back(k0)?;
        let older = read_frame_back(k0 + 1)?;
        let frame = PcmFrame {
            left: ((1.0 - fraction) * newer.left as f32 + fraction * older.left as f32) as i16,
            right: ((1.0 - fraction) * newer.right as f32 + fraction * older.right as f32) as i16,
        };

        self.head_back -= self.velocity;
        self.head_back = self.head_back.clamp(0.0, max_back);
        Some(frame)
    }

    fn update_velocity(&mut self) {
        self.window_pos = self.window_pos.saturating_add(1);
        if self.window_pos >= self.config.rate_window_samples {
            self.window_pos = 0;
            let ticks = core::mem::take(&mut self.pending_ticks);
            if ticks != 0 {
                self.empty_windows = 0;
                let velocity = ticks as f32 * self.config.frames_per_tick
                    / self.config.rate_window_samples as f32;
                self.velocity_target =
                    velocity.clamp(-self.config.velocity_max, self.config.velocity_max);
            } else {
                self.empty_windows = self.empty_windows.saturating_add(1);
                if self.empty_windows >= self.config.hold_windows {
                    self.velocity_target = 0.0;
                }
            }
        }

        self.velocity += (self.velocity_target - self.velocity) * self.config.slew_coef;
    }
}

impl Default for ScratchState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn track_position_ms(
    newest_pos_ms: u32,
    head_back_frames: f32,
    sample_rate: u32,
    loop_region: Option<(u32, u32)>,
) -> u32 {
    if sample_rate == 0 {
        return newest_pos_ms;
    }

    let back_ms = if head_back_frames.is_finite() && head_back_frames > 0.0 {
        let calculated = head_back_frames as f64 * 1_000.0 / sample_rate as f64;
        calculated.min(u32::MAX as f64) as u32
    } else {
        0
    };

    if let Some((loop_start_ms, loop_end_ms)) = loop_region
        && loop_end_ms > loop_start_ms
        && newest_pos_ms >= loop_start_ms
        && newest_pos_ms <= loop_end_ms
    {
        let loop_ms = loop_end_ms - loop_start_ms;
        let newest_offset = (newest_pos_ms - loop_start_ms) % loop_ms;
        let back_offset = back_ms % loop_ms;
        let target_offset = (newest_offset + loop_ms - back_offset) % loop_ms;
        return loop_start_ms + target_offset;
    }

    newest_pos_ms.saturating_sub(back_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: [PcmFrame; 10] = [
        PcmFrame {
            left: 900,
            right: -900,
        },
        PcmFrame {
            left: 800,
            right: -800,
        },
        PcmFrame {
            left: 700,
            right: -700,
        },
        PcmFrame {
            left: 600,
            right: -600,
        },
        PcmFrame {
            left: 500,
            right: -500,
        },
        PcmFrame {
            left: 400,
            right: -400,
        },
        PcmFrame {
            left: 300,
            right: -300,
        },
        PcmFrame {
            left: 200,
            right: -200,
        },
        PcmFrame {
            left: 100,
            right: -100,
        },
        PcmFrame { left: 0, right: 0 },
    ];

    fn read(back: u32) -> Option<PcmFrame> {
        WINDOW.get(back as usize).copied()
    }

    fn instant(frames_per_tick: f32, velocity_max: f32, hold_windows: u32) -> ScratchConfig {
        ScratchConfig::sanitized(frames_per_tick, 1, 1.0, velocity_max, hold_windows)
    }

    #[test]
    fn inactive_and_stopped_are_silent() {
        let mut state = ScratchState::new();
        assert_eq!(state.render(WINDOW.len() as u32, read), None);

        state.seed(4.0);
        assert!(state.is_active());
        assert_eq!(state.render(WINDOW.len() as u32, read), None);
    }

    #[test]
    fn invalid_configuration_is_sanitized() {
        let config = ScratchConfig::sanitized(f32::NAN, 0, f32::INFINITY, -1.0, 3);
        assert_eq!(config.frames_per_tick, DEFAULT_FRAMES_PER_TICK);
        assert_eq!(config.rate_window_samples, 1);
        assert_eq!(config.slew_coef, DEFAULT_SLEW_COEF);
        assert_eq!(config.velocity_max, DEFAULT_VELOCITY_MAX);

        let mut state = ScratchState::new();
        state.seed(f32::NAN);
        assert_eq!(state.head_back(), 0.0);
        assert_eq!(track_position_ms(1_234, f32::INFINITY, 44_100, None), 1_234);
    }

    #[test]
    fn forward_and_reverse_follow_released_head_direction() {
        let mut forward = ScratchState::new();
        forward.configure(instant(1.0, 100.0, 1_000_000));
        forward.seed(5.0);
        forward.jog(1);
        assert_eq!(forward.render(10, read).unwrap().left, 400);
        assert_eq!(forward.render(10, read).unwrap().left, 500);
        assert_eq!(forward.render(10, read).unwrap().left, 600);

        let mut reverse = ScratchState::new();
        reverse.configure(instant(1.0, 100.0, 1_000_000));
        reverse.seed(4.0);
        reverse.jog(-1);
        assert_eq!(reverse.render(10, read).unwrap().left, 500);
        assert_eq!(reverse.render(10, read).unwrap().left, 400);
        assert_eq!(reverse.render(10, read).unwrap().left, 300);
    }

    #[test]
    fn fractional_head_linearly_interpolates() {
        let mut state = ScratchState::new();
        state.configure(instant(0.5, 100.0, 1_000_000));
        state.seed(3.5);
        state.jog(1);

        assert_eq!(state.render(10, read).unwrap().left, 550);
    }

    #[test]
    fn edge_latch_is_silent_until_inward_reversal() {
        let mut state = ScratchState::new();
        state.configure(instant(1.0, 100.0, 1_000_000));
        state.seed(0.0);
        state.jog(4);

        assert_eq!(state.render(10, read), None);
        assert_eq!(state.edge_latch(), 1);
        assert_eq!(state.edge_hits(), 1);
        assert_eq!(state.velocity(), 0.0);

        for _ in 0..8 {
            state.jog(1);
            assert_eq!(state.render(10, read), None);
            assert_eq!(state.head_back(), 0.0);
            assert_eq!(state.velocity(), 0.0);
            assert_eq!(state.edge_hits(), 1);
        }

        state.jog(-2);
        assert_eq!(state.render(10, read).unwrap().left, 900);
        assert_eq!(state.edge_latch(), 0);
        assert_eq!(state.head_back(), 2.0);
    }

    #[test]
    fn velocity_rate_estimate_clamps_and_seed_resets() {
        let mut state = ScratchState::new();
        state.configure(instant(1.0, 2.5, 1_000_000));
        state.seed(5.0);

        state.jog(1);
        state.jog(1);
        let _ = state.render(10, read);
        assert_eq!(state.velocity(), 2.0);

        state.jog(5);
        let _ = state.render(10, read);
        assert_eq!(state.velocity(), 2.5);

        state.jog(-100);
        let _ = state.render(10, read);
        assert_eq!(state.velocity(), -2.5);

        state.seed(3.0);
        assert_eq!(state.velocity(), 0.0);
    }

    #[test]
    fn velocity_holds_between_ticks_then_stops() {
        let mut state = ScratchState::new();
        state.configure(instant(1.0, 100.0, 3));
        state.seed(5.0);
        state.jog(1);

        assert_eq!(state.render(10, read).unwrap().left, 400);
        assert_eq!(state.render(10, read).unwrap().left, 500);
        assert_eq!(state.render(10, read).unwrap().left, 600);
        assert_eq!(state.render(10, read), None);
        assert_eq!(state.velocity(), 0.0);
    }

    #[test]
    fn dense_ticks_average_over_fixed_window() {
        let mut state = ScratchState::new();
        state.configure(ScratchConfig::sanitized(2.0, 4, 1.0, 100.0, 1_000_000));
        state.seed(5.0);
        for _ in 0..4 {
            state.jog(1);
        }

        for _ in 0..3 {
            let _ = state.render(10, read);
        }
        assert_eq!(state.velocity(), 0.0);

        let _ = state.render(10, read);
        assert_eq!(state.velocity(), 2.0);
    }

    #[test]
    fn loop_aware_track_position_wraps_backward() {
        assert_eq!(
            track_position_ms(10_200, 500.0, 1_000, Some((10_000, 20_000))),
            19_700
        );
        assert_eq!(
            track_position_ms(10_200, 20_500.0, 1_000, Some((10_000, 20_000))),
            19_700
        );
        assert_eq!(track_position_ms(1_200, 500.0, 1_000, None), 700);
        assert_eq!(track_position_ms(200, 500.0, 1_000, None), 0);
    }
}
