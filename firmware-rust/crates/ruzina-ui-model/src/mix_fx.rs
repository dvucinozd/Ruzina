//! Typed UI adapter codes; scope and ranges are checked before semantic dispatch.
use ruzina_controller_core::{
    BeatFxTarget, ControlEvent, ControlValue, DeckId, PadAction, PadMode, SemanticControl,
};
use ruzina_deck::{ActivePadFx, BeatFxState, DeckProductState, PadFxBank};
use ruzina_mixer::{MIXER_CONTROL_MAX, MixerDeckState, MixerState};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixerParameter {
    ChannelVolume,
    Trim,
    EqHigh,
    EqMid,
    EqLow,
    Filter,
    Crossfader,
    MasterVolume,
    HeadphoneMix,
    HeadphoneLevel,
}
impl MixerParameter {
    pub const fn from_code(code: i32) -> Option<Self> {
        Some(match code {
            0 => Self::ChannelVolume,
            1 => Self::Trim,
            2 => Self::EqHigh,
            3 => Self::EqMid,
            4 => Self::EqLow,
            5 => Self::Filter,
            6 => Self::Crossfader,
            7 => Self::MasterVolume,
            8 => Self::HeadphoneMix,
            9 => Self::HeadphoneLevel,
            _ => return None,
        })
    }
    pub const fn event(self, deck: Option<DeckId>, raw: i32) -> Option<ControlEvent> {
        if raw < 0 || raw > MIXER_CONTROL_MAX as i32 {
            return None;
        }
        let global = matches!(
            self,
            Self::Crossfader | Self::MasterVolume | Self::HeadphoneMix | Self::HeadphoneLevel
        );
        if global == deck.is_some() {
            return None;
        }
        let control = match self {
            Self::ChannelVolume => SemanticControl::ChannelVolume,
            Self::Trim => SemanticControl::Trim,
            Self::EqHigh => SemanticControl::EqHigh,
            Self::EqMid => SemanticControl::EqMid,
            Self::EqLow => SemanticControl::EqLow,
            Self::Filter => SemanticControl::Filter,
            Self::Crossfader => SemanticControl::Crossfader,
            Self::MasterVolume => SemanticControl::MasterVolume,
            Self::HeadphoneMix => SemanticControl::HeadphoneMix,
            Self::HeadphoneLevel => SemanticControl::HeadphoneLevel,
        };
        Some(ControlEvent {
            deck,
            control,
            value: ControlValue::Absolute {
                value: raw as u16,
                max: MIXER_CONTROL_MAX,
            },
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixerToggle {
    Pfl,
    MasterCue,
    SmartCfx,
    SmartFader,
}
impl MixerToggle {
    pub const fn from_code(code: i32) -> Option<Self> {
        Some(match code {
            0 => Self::Pfl,
            1 => Self::MasterCue,
            2 => Self::SmartCfx,
            3 => Self::SmartFader,
            _ => return None,
        })
    }
    pub const fn event(self, deck: Option<DeckId>, pressed: bool) -> Option<ControlEvent> {
        if matches!(self, Self::Pfl) != deck.is_some() {
            return None;
        }
        Some(ControlEvent {
            deck,
            control: match self {
                Self::Pfl => SemanticControl::Pfl,
                Self::MasterCue => SemanticControl::MasterCue,
                Self::SmartCfx => SemanticControl::SmartCfx,
                Self::SmartFader => SemanticControl::SmartFader,
            },
            value: ControlValue::Pressed(pressed),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MixerPresentation {
    pub channels: [MixerDeckState; 2],
    pub crossfader: u16,
    pub master_volume: u16,
    pub headphone_mix: u16,
    pub headphone_level: u16,
    pub master_cue: bool,
    pub smart_cfx: bool,
    pub smart_fader: bool,
}
pub const fn present_mixer(mixer: &MixerState) -> MixerPresentation {
    MixerPresentation {
        channels: [mixer.deck(DeckId::One), mixer.deck(DeckId::Two)],
        crossfader: mixer.crossfader(),
        master_volume: mixer.master_volume(),
        headphone_mix: mixer.headphone_mix(),
        headphone_level: mixer.headphone_level(),
        master_cue: mixer.master_cue_enabled(),
        smart_cfx: mixer.smart_cfx_enabled(),
        smart_fader: mixer.smart_fader_enabled(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BeatFxAction {
    Previous,
    Next,
    BeatDecrease,
    BeatIncrease,
    Toggle,
    Clear,
}
impl BeatFxAction {
    pub const fn from_code(code: i32) -> Option<Self> {
        Some(match code {
            0 => Self::Previous,
            1 => Self::Next,
            2 => Self::BeatDecrease,
            3 => Self::BeatIncrease,
            4 => Self::Toggle,
            5 => Self::Clear,
            _ => return None,
        })
    }
    pub const fn event(self, pressed: bool) -> ControlEvent {
        ControlEvent {
            deck: None,
            control: match self {
                Self::Previous => SemanticControl::BeatFxSelectPrev,
                Self::Next => SemanticControl::BeatFxSelectNext,
                Self::BeatDecrease => SemanticControl::BeatFxBeatDec,
                Self::BeatIncrease => SemanticControl::BeatFxBeatInc,
                Self::Toggle => SemanticControl::BeatFxOn,
                Self::Clear => SemanticControl::BeatFxClear,
            },
            value: ControlValue::Pressed(pressed),
        }
    }
}
pub const fn fx_target_event(code: i32) -> Option<ControlEvent> {
    let target = match code {
        0 => BeatFxTarget::ChannelOne,
        1 => BeatFxTarget::ChannelTwo,
        2 => BeatFxTarget::Both,
        _ => return None,
    };
    Some(ControlEvent {
        deck: None,
        control: SemanticControl::BeatFxTarget,
        value: ControlValue::BeatFxTarget(target),
    })
}
pub const fn fx_depth_event(raw: i32) -> Option<ControlEvent> {
    if raw < 0 || raw > 127 {
        return None;
    }
    Some(ControlEvent {
        deck: None,
        control: SemanticControl::BeatFxDepth,
        value: ControlValue::Absolute {
            value: raw as u16,
            max: 127,
        },
    })
}
pub const fn pad_bank_event(deck: DeckId, bank: i32) -> Option<ControlEvent> {
    let control = match bank {
        0 => SemanticControl::PadModePadFx1,
        1 => SemanticControl::PadModePadFx2,
        _ => return None,
    };
    Some(ControlEvent {
        deck: Some(deck),
        control,
        value: ControlValue::Pressed(true),
    })
}
pub const fn pad_fx_event(
    deck: DeckId,
    bank: i32,
    pad: i32,
    pressed: bool,
) -> Option<ControlEvent> {
    if pad < 0 || pad > 7 {
        return None;
    }
    let mode = match bank {
        0 => PadMode::PadFx1,
        1 => PadMode::PadFx2,
        _ => return None,
    };
    Some(ControlEvent {
        deck: Some(deck),
        control: SemanticControl::PadAction,
        value: ControlValue::PadAction(PadAction {
            pad: pad as u8,
            mode,
            shifted: false,
            pressed,
        }),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PadFxPresentation {
    pub bank: PadFxBank,
    pub active: Option<ActivePadFx>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FxPresentation {
    pub beat: BeatFxState,
    pub pads: [PadFxPresentation; 2],
}
pub fn present_fx(decks: &DeckProductState) -> FxPresentation {
    FxPresentation {
        beat: decks.beat_fx(),
        pads: core::array::from_fn(|i| {
            let state = decks.deck(if i == 0 { DeckId::One } else { DeckId::Two });
            PadFxPresentation {
                bank: if state.pad_mode == PadMode::PadFx2 {
                    PadFxBank::Two
                } else {
                    PadFxBank::One
                },
                active: state.active_pad_fx,
            }
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixer_scope_and_raw_bounds_reject_cross_domain_input() {
        for code in 0..10 {
            let parameter = MixerParameter::from_code(code).unwrap();
            let target = if code < 6 { Some(DeckId::One) } else { None };
            for raw in [0, 8192, 16383] {
                assert!(parameter.event(target, raw).is_some());
            }
            assert!(parameter.event(target, -1).is_none());
            assert!(parameter.event(target, 16384).is_none());
            assert!(
                parameter
                    .event(
                        if target.is_some() {
                            None
                        } else {
                            Some(DeckId::Two)
                        },
                        8192
                    )
                    .is_none()
            );
        }
        assert!(MixerParameter::from_code(10).is_none());
        assert!(MixerToggle::Pfl.event(None, true).is_none());
        assert!(
            MixerToggle::MasterCue
                .event(Some(DeckId::One), true)
                .is_none()
        );
    }
    #[test]
    fn pfl_and_headphone_controls_preserve_independent_main_state() {
        let mut mixer = MixerState::new();
        mixer.handle_control(MixerParameter::Crossfader.event(None, 0).unwrap());
        mixer.handle_control(MixerToggle::Pfl.event(Some(DeckId::Two), true).unwrap());
        mixer.handle_control(MixerParameter::HeadphoneMix.event(None, 0).unwrap());
        let p = present_mixer(&mixer);
        assert!(p.channels[1].pfl_enabled && !p.channels[0].pfl_enabled);
        assert_eq!(p.crossfader, 0);
        assert_eq!(p.master_volume, 16383);
        assert_eq!(p.headphone_mix, 0);
        assert_eq!(mixer.stage_gains().post[1], 0.0);
    }
    #[test]
    fn fx_bounds_release_and_wrong_bank_cannot_clear_another_pad() {
        let mut decks = DeckProductState::new();
        assert!(fx_target_event(3).is_none());
        assert!(fx_depth_event(128).is_none());
        assert!(pad_fx_event(DeckId::One, 0, 8, true).is_none());
        assert!(pad_bank_event(DeckId::One, 2).is_none());
        decks.handle_control(BeatFxAction::Next.event(false));
        assert_eq!(decks.beat_fx(), BeatFxState::new());
        decks.handle_control(pad_fx_event(DeckId::One, 1, 3, true).unwrap());
        decks.handle_control(pad_fx_event(DeckId::One, 0, 3, false).unwrap());
        assert_eq!(
            present_fx(&decks).pads[0].active,
            Some(ActivePadFx {
                bank: PadFxBank::Two,
                pad: 3
            })
        );
        decks.handle_control(pad_fx_event(DeckId::One, 1, 3, false).unwrap());
        assert!(present_fx(&decks).pads[0].active.is_none());
    }
}
