#![no_std]
#![forbid(unsafe_code)]

pub mod history;
pub mod pcm_stream;
pub mod render;
pub mod routing;
pub mod sink;
pub mod source;
pub mod wav;

use pajoniiir_audio_dsp::{DelayConfig, DelayMode, FlangerConfig, PadFxConfig, PadFxMode};
use pajoniiir_controller_core::{BeatFxTarget, DeckId};
use pajoniiir_deck::{BeatFxEffect, BeatFxState, DeckEffect, PadFxBank};

pub const BEAT_FX_FILTER_CENTER: u16 = 8_192;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BeatFxFilterConfig {
    pub enabled: bool,
    pub raw: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BeatFxDeckConfig {
    pub filter: BeatFxFilterConfig,
    pub time: DelayConfig,
    pub flanger: FlangerConfig,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioEffectCommand {
    BeatFx { decks: [BeatFxDeckConfig; 2] },
    PadFx { deck: DeckId, config: PadFxConfig },
}

pub fn map_deck_effect(effect: DeckEffect) -> Option<AudioEffectCommand> {
    match effect {
        DeckEffect::ApplyBeatFx {
            state,
            delay_ms,
            flanger_period_ms,
        } => Some(AudioEffectCommand::BeatFx {
            decks: map_beat_fx(state, delay_ms, flanger_period_ms),
        }),
        DeckEffect::ApplyPadFx {
            deck,
            bank,
            pad,
            active,
        } => Some(AudioEffectCommand::PadFx {
            deck,
            config: PadFxConfig {
                mode: match bank {
                    PadFxBank::One => PadFxMode::PadFx1,
                    PadFxBank::Two => PadFxMode::PadFx2,
                },
                pad,
                active,
            },
        }),
        _ => None,
    }
}

pub fn map_beat_fx(
    state: BeatFxState,
    delay_ms: u32,
    flanger_period_ms: u32,
) -> [BeatFxDeckConfig; 2] {
    [
        beat_fx_deck_config(state, 0, delay_ms, flanger_period_ms),
        beat_fx_deck_config(state, 1, delay_ms, flanger_period_ms),
    ]
}

pub fn beat_fx_filter_raw_from_depth(depth: u8) -> u16 {
    if depth == 0 {
        return BEAT_FX_FILTER_CENTER;
    }
    let depth = depth.min(127) as u32;
    let sweep = ((BEAT_FX_FILTER_CENTER as u32 * depth) + 63) / 127;
    BEAT_FX_FILTER_CENTER - sweep.min(BEAT_FX_FILTER_CENTER as u32) as u16
}

pub fn beat_fx_time_wet_from_depth(depth: u8) -> u16 {
    let x = depth.min(127) as f32 / 127.0;
    (22_938.0 * libm::sqrtf(x) + 0.5) as u16
}

pub fn beat_fx_echo_feedback_from_depth(depth: u8) -> u16 {
    let x = depth.min(127) as f32 / 127.0;
    (6_554.0 + x * (22_282.0 - 6_554.0) + 0.5) as u16
}

pub fn beat_fx_flanger_depth_q15(depth: u8) -> u16 {
    ((depth.min(127) as u32 * 32_767) / 127) as u16
}

fn beat_fx_deck_config(
    state: BeatFxState,
    deck_index: usize,
    delay_ms: u32,
    flanger_period_ms: u32,
) -> BeatFxDeckConfig {
    let included = target_includes(state.target, deck_index);
    let active = state.enabled && state.depth > 0 && included;
    let filter_enabled = active && state.effect == BeatFxEffect::Filter;
    let time_enabled = active && matches!(state.effect, BeatFxEffect::Echo | BeatFxEffect::Delay);
    let flanger_enabled = active && state.effect == BeatFxEffect::Flanger;
    let delay_mode = if state.effect == BeatFxEffect::Delay {
        DelayMode::Delay
    } else {
        DelayMode::Echo
    };

    BeatFxDeckConfig {
        filter: BeatFxFilterConfig {
            enabled: filter_enabled,
            raw: if filter_enabled {
                beat_fx_filter_raw_from_depth(state.depth)
            } else {
                BEAT_FX_FILTER_CENTER
            },
        },
        time: DelayConfig {
            enabled: time_enabled,
            mode: delay_mode,
            delay_ms: delay_ms.clamp(1, 1_000),
            wet_q15: beat_fx_time_wet_from_depth(state.depth),
            feedback_q15: if delay_mode == DelayMode::Echo {
                beat_fx_echo_feedback_from_depth(state.depth)
            } else {
                0
            },
        },
        flanger: FlangerConfig {
            enabled: flanger_enabled,
            period_ms: flanger_period_ms,
            depth_q15: beat_fx_flanger_depth_q15(state.depth),
        },
    }
}

const fn target_includes(target: BeatFxTarget, deck_index: usize) -> bool {
    match target {
        BeatFxTarget::ChannelOne => deck_index == 0,
        BeatFxTarget::ChannelTwo => deck_index == 1,
        BeatFxTarget::Both => deck_index < 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pajoniiir_deck::BeatFxBeat;

    fn state(effect: BeatFxEffect, target: BeatFxTarget, depth: u8, enabled: bool) -> BeatFxState {
        BeatFxState {
            effect,
            beat: BeatFxBeat::One,
            target,
            depth,
            enabled,
        }
    }

    #[test]
    fn non_audio_deck_effects_are_ignored() {
        assert_eq!(
            map_deck_effect(DeckEffect::SetLoop {
                deck: DeckId::One,
                start_ms: 1_000,
                end_ms: 2_000,
            }),
            None
        );
    }

    #[test]
    fn released_depth_mapping_endpoints_are_exact() {
        assert_eq!(beat_fx_filter_raw_from_depth(0), 8_192);
        assert_eq!(beat_fx_filter_raw_from_depth(127), 0);
        assert_eq!(beat_fx_time_wet_from_depth(0), 0);
        assert_eq!(beat_fx_time_wet_from_depth(127), 22_938);
        assert_eq!(beat_fx_echo_feedback_from_depth(0), 6_554);
        assert_eq!(beat_fx_echo_feedback_from_depth(127), 22_282);
        assert_eq!(beat_fx_flanger_depth_q15(0), 0);
        assert_eq!(beat_fx_flanger_depth_q15(127), 32_767);
    }

    #[test]
    fn released_mid_depth_mapping_matches_c_fixture_math() {
        assert_eq!(beat_fx_filter_raw_from_depth(64), 4_064);
        assert_eq!(beat_fx_time_wet_from_depth(64), 16_283);
        assert_eq!(beat_fx_echo_feedback_from_depth(64), 14_480);
        assert_eq!(beat_fx_flanger_depth_q15(64), 16_512);
    }

    #[test]
    fn filter_target_is_per_deck_and_does_not_touch_time_or_flanger() {
        let configs = map_beat_fx(
            state(BeatFxEffect::Filter, BeatFxTarget::ChannelOne, 127, true),
            500,
            500,
        );

        assert_eq!(
            configs[0].filter,
            BeatFxFilterConfig {
                enabled: true,
                raw: 0,
            }
        );
        assert!(!configs[1].filter.enabled);
        assert!(!configs[0].time.enabled);
        assert!(!configs[1].time.enabled);
        assert!(!configs[0].flanger.enabled);
        assert!(!configs[1].flanger.enabled);
    }

    #[test]
    fn echo_maps_sqrt_wet_linear_feedback_and_both_target() {
        let configs = map_beat_fx(
            state(BeatFxEffect::Echo, BeatFxTarget::Both, 64, true),
            2_000,
            500,
        );

        for config in configs {
            assert!(config.time.enabled);
            assert_eq!(config.time.mode, DelayMode::Echo);
            assert_eq!(config.time.delay_ms, 1_000);
            assert_eq!(config.time.wet_q15, 16_283);
            assert_eq!(config.time.feedback_q15, 14_480);
            assert!(!config.filter.enabled);
            assert!(!config.flanger.enabled);
        }
    }

    #[test]
    fn delay_is_one_shot_and_preserves_zero_feedback_contract() {
        let configs = map_beat_fx(
            state(BeatFxEffect::Delay, BeatFxTarget::ChannelTwo, 127, true),
            0,
            500,
        );

        assert!(!configs[0].time.enabled);
        assert!(configs[1].time.enabled);
        assert_eq!(configs[1].time.mode, DelayMode::Delay);
        assert_eq!(configs[1].time.delay_ms, 1);
        assert_eq!(configs[1].time.wet_q15, 22_938);
        assert_eq!(configs[1].time.feedback_q15, 0);
    }

    #[test]
    fn flanger_maps_period_and_linear_depth() {
        let configs = map_beat_fx(
            state(BeatFxEffect::Flanger, BeatFxTarget::Both, 127, true),
            500,
            2_000,
        );

        for config in configs {
            assert!(config.flanger.enabled);
            assert_eq!(config.flanger.period_ms, 2_000);
            assert_eq!(config.flanger.depth_q15, 32_767);
            assert!(!config.filter.enabled);
            assert!(!config.time.enabled);
        }
    }

    #[test]
    fn disabled_or_zero_depth_state_disables_all_processors() {
        for state in [
            state(BeatFxEffect::Echo, BeatFxTarget::Both, 64, false),
            state(BeatFxEffect::Echo, BeatFxTarget::Both, 0, true),
        ] {
            for config in map_beat_fx(state, 500, 500) {
                assert!(!config.filter.enabled);
                assert!(!config.time.enabled);
                assert!(!config.flanger.enabled);
            }
        }
    }

    #[test]
    fn pad_fx_effect_maps_bank_pad_and_release_without_dsp_knowledge_in_deck() {
        assert_eq!(
            map_deck_effect(DeckEffect::ApplyPadFx {
                deck: DeckId::Two,
                bank: PadFxBank::Two,
                pad: 3,
                active: false,
            }),
            Some(AudioEffectCommand::PadFx {
                deck: DeckId::Two,
                config: PadFxConfig {
                    mode: PadFxMode::PadFx2,
                    pad: 3,
                    active: false,
                },
            })
        );
    }
}
