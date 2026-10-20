#![no_std]
#![forbid(unsafe_code)]

use pajoniiir_audio_dsp::smart_cfx_curve_raw;
use pajoniiir_controller_core::{ControlEvent, ControlValue, DeckId, SemanticControl};

pub const MIXER_CONTROL_MAX: u16 = 16_383;
pub const MIXER_CONTROL_CENTER: u16 = 8_192;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixerStage {
    Trim,
    ChannelDsp,
    PflTap,
    ChannelFader,
    Crossfader,
    DeckSum,
    MasterGain,
    MainLimiter,
}

pub const MAIN_SIGNAL_PATH: [MixerStage; 8] = [
    MixerStage::Trim,
    MixerStage::ChannelDsp,
    MixerStage::PflTap,
    MixerStage::ChannelFader,
    MixerStage::Crossfader,
    MixerStage::DeckSum,
    MixerStage::MasterGain,
    MixerStage::MainLimiter,
];

pub const PFL_SIGNAL_PATH: [MixerStage; 3] =
    [MixerStage::Trim, MixerStage::ChannelDsp, MixerStage::PflTap];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MixerDeckState {
    pub channel_volume: u16,
    pub trim: u16,
    pub eq_high: u16,
    pub eq_mid: u16,
    pub eq_low: u16,
    pub filter: u16,
    pub pfl_enabled: bool,
}

impl MixerDeckState {
    pub const fn new() -> Self {
        Self {
            channel_volume: MIXER_CONTROL_MAX,
            trim: MIXER_CONTROL_CENTER,
            eq_high: MIXER_CONTROL_CENTER,
            eq_mid: MIXER_CONTROL_CENTER,
            eq_low: MIXER_CONTROL_CENTER,
            filter: MIXER_CONTROL_CENTER,
            pfl_enabled: false,
        }
    }
}

impl Default for MixerDeckState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixerState {
    decks: [MixerDeckState; 2],
    crossfader: u16,
    master_volume: u16,
    master_trim_gain: f32,
    headphone_mix: u16,
    headphone_level: u16,
    master_cue_enabled: bool,
    smart_cfx_enabled: bool,
    smart_fader_enabled: bool,
}

impl MixerState {
    pub const fn new() -> Self {
        Self {
            decks: [MixerDeckState::new(), MixerDeckState::new()],
            crossfader: MIXER_CONTROL_CENTER,
            master_volume: MIXER_CONTROL_MAX,
            master_trim_gain: 1.0,
            headphone_mix: MIXER_CONTROL_MAX,
            headphone_level: MIXER_CONTROL_MAX,
            master_cue_enabled: true,
            smart_cfx_enabled: false,
            smart_fader_enabled: false,
        }
    }

    pub const fn deck(&self, deck: DeckId) -> MixerDeckState {
        self.decks[deck_index(deck)]
    }

    pub const fn crossfader(&self) -> u16 {
        self.crossfader
    }

    pub const fn master_volume(&self) -> u16 {
        self.master_volume
    }

    pub const fn master_trim_gain(&self) -> f32 {
        self.master_trim_gain
    }

    pub const fn headphone_mix(&self) -> u16 {
        self.headphone_mix
    }

    pub const fn headphone_level(&self) -> u16 {
        self.headphone_level
    }

    pub const fn master_cue_enabled(&self) -> bool {
        self.master_cue_enabled
    }

    pub const fn smart_cfx_enabled(&self) -> bool {
        self.smart_cfx_enabled
    }

    pub const fn smart_fader_enabled(&self) -> bool {
        self.smart_fader_enabled
    }

    pub fn effective_filter_raw(&self, deck: DeckId) -> u16 {
        let raw = self.decks[deck_index(deck)].filter;
        if self.smart_cfx_enabled {
            smart_cfx_curve_raw(raw)
        } else {
            raw
        }
    }

    pub fn set_master_trim_gain(&mut self, gain: f32) {
        self.master_trim_gain = if gain.is_nan() || gain < 0.0 {
            0.0
        } else if gain > 1.0 {
            1.0
        } else {
            gain
        };
    }

    pub fn handle_control(&mut self, event: ControlEvent) -> bool {
        match event.control {
            SemanticControl::Pfl => {
                let Some(deck) = event.deck else {
                    return false;
                };
                if event.value != ControlValue::Pressed(true) {
                    return false;
                }
                let index = deck_index(deck);
                self.decks[index].pfl_enabled = !self.decks[index].pfl_enabled;
                true
            }
            SemanticControl::MasterCue => {
                if event.value != ControlValue::Pressed(true) {
                    return false;
                }
                self.master_cue_enabled = !self.master_cue_enabled;
                true
            }
            SemanticControl::SmartCfx => {
                if event.value != ControlValue::Pressed(true) {
                    return false;
                }
                self.smart_cfx_enabled = !self.smart_cfx_enabled;
                true
            }
            SemanticControl::SmartFader => {
                if event.value != ControlValue::Pressed(true) {
                    return false;
                }
                self.smart_fader_enabled = !self.smart_fader_enabled;
                true
            }
            SemanticControl::SmartCfxShift | SemanticControl::SmartFaderShift => false,
            SemanticControl::ChannelVolume
            | SemanticControl::Trim
            | SemanticControl::EqHigh
            | SemanticControl::EqMid
            | SemanticControl::EqLow
            | SemanticControl::Filter => {
                let Some(deck) = event.deck else {
                    return false;
                };
                let Some(raw) = normalized_absolute(event.value) else {
                    return false;
                };
                let deck_state = &mut self.decks[deck_index(deck)];
                let target = match event.control {
                    SemanticControl::ChannelVolume => &mut deck_state.channel_volume,
                    SemanticControl::Trim => &mut deck_state.trim,
                    SemanticControl::EqHigh => &mut deck_state.eq_high,
                    SemanticControl::EqMid => &mut deck_state.eq_mid,
                    SemanticControl::EqLow => &mut deck_state.eq_low,
                    SemanticControl::Filter => &mut deck_state.filter,
                    _ => unreachable!(),
                };
                update_raw(target, raw)
            }
            SemanticControl::Crossfader
            | SemanticControl::MasterVolume
            | SemanticControl::HeadphoneMix
            | SemanticControl::HeadphoneLevel => {
                let Some(raw) = normalized_absolute(event.value) else {
                    return false;
                };
                let target = match event.control {
                    SemanticControl::Crossfader => &mut self.crossfader,
                    SemanticControl::MasterVolume => &mut self.master_volume,
                    SemanticControl::HeadphoneMix => &mut self.headphone_mix,
                    SemanticControl::HeadphoneLevel => &mut self.headphone_level,
                    _ => unreachable!(),
                };
                update_raw(target, raw)
            }
            _ => false,
        }
    }

    pub fn stage_gains(&self) -> MixerStageGains {
        let (mut xf_one, mut xf_two) = crossfader_gains(self.crossfader);
        if self.smart_fader_enabled {
            if self.crossfader < MIXER_CONTROL_CENTER {
                xf_two *= xf_two;
            } else if self.crossfader > MIXER_CONTROL_CENTER {
                xf_one *= xf_one;
            }
        }
        MixerStageGains {
            pre: [trim_gain(self.decks[0].trim), trim_gain(self.decks[1].trim)],
            post: [
                fader_gain(self.decks[0].channel_volume) * xf_one,
                fader_gain(self.decks[1].channel_volume) * xf_two,
            ],
            master: fader_gain(self.master_volume) * self.master_trim_gain,
        }
    }
}

impl Default for MixerState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixerStageGains {
    pub pre: [f32; 2],
    pub post: [f32; 2],
    pub master: f32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LimiterStats {
    pub limited_samples: u32,
    pub positive_overloads: u32,
    pub negative_overloads: u32,
    pub peak_input_abs: i32,
}

pub fn fader_gain(raw: u16) -> f32 {
    if raw >= MIXER_CONTROL_MAX {
        1.0
    } else {
        raw as f32 / MIXER_CONTROL_MAX as f32
    }
}

pub fn trim_gain(raw: u16) -> f32 {
    let raw = raw.min(MIXER_CONTROL_MAX);
    if raw <= MIXER_CONTROL_CENTER {
        let t = raw as f32 / MIXER_CONTROL_CENTER as f32;
        0.25 + (0.75 * t)
    } else {
        let t =
            (raw - MIXER_CONTROL_CENTER) as f32 / (MIXER_CONTROL_MAX - MIXER_CONTROL_CENTER) as f32;
        1.0 + t
    }
}

pub fn crossfader_gains(raw: u16) -> (f32, f32) {
    let raw = raw.min(MIXER_CONTROL_MAX);
    if raw <= MIXER_CONTROL_CENTER {
        (1.0, raw as f32 / MIXER_CONTROL_CENTER as f32)
    } else {
        (
            (MIXER_CONTROL_MAX - raw) as f32 / (MIXER_CONTROL_MAX - MIXER_CONTROL_CENTER) as f32,
            1.0,
        )
    }
}

pub fn limit_main_sample(mixed: f32, stats: &mut LimiterStats) -> i16 {
    let mixed = if mixed.is_nan() { 0.0 } else { mixed };
    let abs_float = if mixed < 0.0 { -mixed } else { mixed };
    let abs_mixed = if abs_float >= i32::MAX as f32 {
        i32::MAX
    } else {
        (abs_float + 0.5) as i32
    };
    if abs_mixed > stats.peak_input_abs {
        stats.peak_input_abs = abs_mixed;
    }

    if mixed > 30_000.0 {
        stats.limited_samples = stats.limited_samples.saturating_add(1);
        stats.positive_overloads = stats.positive_overloads.saturating_add(1);
        let wide = if mixed >= i32::MAX as f32 {
            i32::MAX
        } else {
            (mixed + 0.5) as i32
        };
        limit_positive_sample(wide)
    } else if mixed < -30_000.0 {
        stats.limited_samples = stats.limited_samples.saturating_add(1);
        stats.negative_overloads = stats.negative_overloads.saturating_add(1);
        let wide = if mixed <= i32::MIN as f32 {
            i32::MIN
        } else {
            (mixed - 0.5) as i32
        };
        limit_negative_sample(wide)
    } else {
        (if mixed >= 0.0 {
            mixed + 0.5
        } else {
            mixed - 0.5
        }) as i16
    }
}

fn normalized_absolute(value: ControlValue) -> Option<u16> {
    let ControlValue::Absolute { value, max } = value else {
        return None;
    };
    if max == 0 {
        return None;
    }
    let bounded = value.min(max) as u32;
    let scaled = ((bounded * MIXER_CONTROL_MAX as u32) + (max as u32 / 2)) / max as u32;
    Some(scaled.min(MIXER_CONTROL_MAX as u32) as u16)
}

fn update_raw(target: &mut u16, value: u16) -> bool {
    if *target == value {
        false
    } else {
        *target = value;
        true
    }
}

const fn deck_index(deck: DeckId) -> usize {
    match deck {
        DeckId::One => 0,
        DeckId::Two => 1,
    }
}

fn soft_limit_abs_sample(abs_sample: i64, knee: i32, ceiling: i32) -> i32 {
    if abs_sample <= knee as i64 {
        return abs_sample as i32;
    }

    let range = (ceiling - knee) as f32;
    let excess = (abs_sample - knee as i64) as f32;
    let shaped = knee as f32 + ((range * excess) / (excess + range));
    if shaped >= ceiling as f32 {
        ceiling
    } else {
        (shaped + 0.5) as i32
    }
}

fn limit_positive_sample(sample: i32) -> i16 {
    soft_limit_abs_sample(sample as i64, 30_000, 32_767) as i16
}

fn limit_negative_sample(sample: i32) -> i16 {
    let magnitude = soft_limit_abs_sample(-(sample as i64), 30_000, 32_768);
    if magnitude >= 32_768 {
        i16::MIN
    } else {
        -(magnitude as i16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn absolute(
        deck: Option<DeckId>,
        control: SemanticControl,
        value: u16,
        max: u16,
    ) -> ControlEvent {
        ControlEvent {
            deck,
            control,
            value: ControlValue::Absolute { value, max },
        }
    }

    fn pressed(deck: Option<DeckId>, control: SemanticControl, down: bool) -> ControlEvent {
        ControlEvent {
            deck,
            control,
            value: ControlValue::Pressed(down),
        }
    }

    #[test]
    fn defaults_match_released_mixer_baseline() {
        let state = MixerState::new();
        assert_eq!(state.deck(DeckId::One), MixerDeckState::new());
        assert_eq!(state.deck(DeckId::Two), MixerDeckState::new());
        assert_eq!(state.crossfader(), MIXER_CONTROL_CENTER);
        assert_eq!(state.master_volume(), MIXER_CONTROL_MAX);
        assert_eq!(state.master_trim_gain(), 1.0);
        assert_eq!(state.headphone_mix(), MIXER_CONTROL_MAX);
        assert_eq!(state.headphone_level(), MIXER_CONTROL_MAX);
        assert!(state.master_cue_enabled());
        assert!(!state.smart_cfx_enabled());
        assert!(!state.smart_fader_enabled());
    }

    #[test]
    fn seven_bit_controls_scale_to_internal_fourteen_bit_domain() {
        let mut state = MixerState::new();
        assert!(state.handle_control(absolute(
            Some(DeckId::One),
            SemanticControl::ChannelVolume,
            64,
            127,
        )));
        let expected = ((64u32 * MIXER_CONTROL_MAX as u32) + 63) / 127;
        assert_eq!(state.deck(DeckId::One).channel_volume, expected as u16);

        assert!(state.handle_control(absolute(None, SemanticControl::Crossfader, 127, 127,)));
        assert_eq!(state.crossfader(), MIXER_CONTROL_MAX);
    }

    #[test]
    fn deck_absolute_controls_require_deck_identity() {
        let mut state = MixerState::new();
        assert!(!state.handle_control(absolute(None, SemanticControl::Trim, 0, 127,)));
        assert_eq!(state.deck(DeckId::One).trim, MIXER_CONTROL_CENTER);
    }

    #[test]
    fn pfl_and_master_cue_toggle_only_on_press_edge() {
        let mut state = MixerState::new();

        assert!(state.handle_control(pressed(Some(DeckId::Two), SemanticControl::Pfl, true,)));
        assert!(state.deck(DeckId::Two).pfl_enabled);
        assert!(!state.handle_control(pressed(Some(DeckId::Two), SemanticControl::Pfl, false,)));
        assert!(state.deck(DeckId::Two).pfl_enabled);

        assert!(state.handle_control(pressed(None, SemanticControl::MasterCue, true)));
        assert!(!state.master_cue_enabled());
        assert!(!state.handle_control(pressed(None, SemanticControl::MasterCue, false)));
        assert!(!state.master_cue_enabled());
    }

    #[test]
    fn smart_buttons_toggle_only_on_press_and_shift_variants_are_noops() {
        let mut state = MixerState::new();

        assert!(state.handle_control(pressed(None, SemanticControl::SmartCfx, true)));
        assert!(state.smart_cfx_enabled());
        assert!(!state.handle_control(pressed(None, SemanticControl::SmartCfx, false)));
        assert!(state.smart_cfx_enabled());

        assert!(state.handle_control(pressed(None, SemanticControl::SmartFader, true)));
        assert!(state.smart_fader_enabled());
        assert!(!state.handle_control(pressed(None, SemanticControl::SmartFader, false)));
        assert!(state.smart_fader_enabled());

        assert!(!state.handle_control(pressed(None, SemanticControl::SmartCfxShift, true,)));
        assert!(!state.handle_control(pressed(None, SemanticControl::SmartFaderShift, true,)));
        assert!(state.smart_cfx_enabled());
        assert!(state.smart_fader_enabled());
    }

    #[test]
    fn smart_cfx_only_changes_effective_filter_mapping() {
        let mut state = MixerState::new();
        state.handle_control(absolute(
            Some(DeckId::One),
            SemanticControl::Filter,
            MIXER_CONTROL_CENTER - 512,
            MIXER_CONTROL_MAX,
        ));
        let raw = state.deck(DeckId::One).filter;
        assert_eq!(state.effective_filter_raw(DeckId::One), raw);

        state.handle_control(pressed(None, SemanticControl::SmartCfx, true));
        let effective = state.effective_filter_raw(DeckId::One);
        assert!(effective < MIXER_CONTROL_CENTER);
        assert!(effective > raw);
        assert_eq!(state.deck(DeckId::One).filter, raw);
    }

    #[test]
    fn smart_fader_squares_only_the_fading_out_crossfader_side() {
        let mut state = MixerState::new();
        state.handle_control(absolute(
            None,
            SemanticControl::Crossfader,
            MIXER_CONTROL_CENTER / 2,
            MIXER_CONTROL_MAX,
        ));
        let normal_left = state.stage_gains();
        assert_eq!(normal_left.post[0], 1.0);
        assert!(normal_left.post[1] > 0.0);

        state.handle_control(pressed(None, SemanticControl::SmartFader, true));
        let smart_left = state.stage_gains();
        assert_eq!(smart_left.post[0], normal_left.post[0]);
        assert_eq!(
            smart_left.post[1],
            normal_left.post[1] * normal_left.post[1]
        );

        state.handle_control(absolute(
            None,
            SemanticControl::Crossfader,
            MIXER_CONTROL_CENTER + (MIXER_CONTROL_MAX - MIXER_CONTROL_CENTER) / 2,
            MIXER_CONTROL_MAX,
        ));
        let smart_right = state.stage_gains();
        let (xf_one, xf_two) = crossfader_gains(state.crossfader());
        assert_eq!(smart_right.post[0], xf_one * xf_one);
        assert_eq!(smart_right.post[1], xf_two);
    }

    #[test]
    fn released_gain_curves_are_preserved() {
        assert_eq!(fader_gain(0), 0.0);
        assert_eq!(fader_gain(MIXER_CONTROL_MAX), 1.0);

        assert_eq!(trim_gain(0), 0.25);
        assert_eq!(trim_gain(MIXER_CONTROL_CENTER), 1.0);
        assert_eq!(trim_gain(MIXER_CONTROL_MAX), 2.0);

        assert_eq!(crossfader_gains(0), (1.0, 0.0));
        assert_eq!(crossfader_gains(MIXER_CONTROL_CENTER), (1.0, 1.0));
        assert_eq!(crossfader_gains(MIXER_CONTROL_MAX), (0.0, 1.0));
    }

    #[test]
    fn pfl_path_ends_before_fader_crossfader_master_and_limiter() {
        assert_eq!(
            PFL_SIGNAL_PATH,
            [MixerStage::Trim, MixerStage::ChannelDsp, MixerStage::PflTap,]
        );
        assert_eq!(MAIN_SIGNAL_PATH[2], MixerStage::PflTap);
        assert_eq!(MAIN_SIGNAL_PATH[3], MixerStage::ChannelFader);
        assert_eq!(MAIN_SIGNAL_PATH[7], MixerStage::MainLimiter);
    }

    #[test]
    fn stage_gains_keep_trim_separate_from_post_fader_path() {
        let mut state = MixerState::new();
        state.handle_control(absolute(
            Some(DeckId::One),
            SemanticControl::Trim,
            MIXER_CONTROL_MAX,
            MIXER_CONTROL_MAX,
        ));
        state.handle_control(absolute(
            Some(DeckId::One),
            SemanticControl::ChannelVolume,
            0,
            MIXER_CONTROL_MAX,
        ));

        let gains = state.stage_gains();
        assert_eq!(gains.pre[0], 2.0);
        assert_eq!(gains.post[0], 0.0);
        assert_eq!(gains.master, 1.0);
    }

    #[test]
    fn master_trim_is_clamped_and_rejects_nan_to_silence() {
        let mut state = MixerState::new();
        state.set_master_trim_gain(2.0);
        assert_eq!(state.master_trim_gain(), 1.0);
        state.set_master_trim_gain(-1.0);
        assert_eq!(state.master_trim_gain(), 0.0);
        state.set_master_trim_gain(f32::NAN);
        assert_eq!(state.master_trim_gain(), 0.0);
    }

    #[test]
    fn main_limiter_preserves_normal_level_and_shapes_overload() {
        let mut stats = LimiterStats::default();
        assert_eq!(limit_main_sample(22_000.0, &mut stats), 22_000);
        assert_eq!(stats.limited_samples, 0);

        let positive = limit_main_sample(60_000.0, &mut stats);
        let negative = limit_main_sample(-60_000.0, &mut stats);
        assert!(positive > 30_000);
        assert!(negative < -30_000);
        assert_eq!(stats.limited_samples, 2);
        assert_eq!(stats.positive_overloads, 1);
        assert_eq!(stats.negative_overloads, 1);
        assert_eq!(stats.peak_input_abs, 60_000);
    }

    #[test]
    fn limiter_maps_nan_to_zero_without_polluting_stats() {
        let mut stats = LimiterStats::default();
        assert_eq!(limit_main_sample(f32::NAN, &mut stats), 0);
        assert_eq!(stats, LimiterStats::default());
    }
}
