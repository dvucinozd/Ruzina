use pajoniiir_controller_core::{
    BeatFxTarget, ControlEvent, ControlValue, DeckExtAction, DeckExtActionValue, DeckId, PadAction,
    PadMode, SemanticControl,
};

use crate::ProfileEvent;

const TYPE_BUTTON: u8 = 0x01;
const TYPE_ENCODER: u8 = 0x02;
const TYPE_PITCH: u8 = 0x03;

const NS_DECK1: u8 = 0x10;
const NS_DECK2: u8 = 0x30;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticAdapterError {
    UnsupportedId(u8),
    TypeMismatch { expected: u8, actual: u8 },
    ValueOutOfRange,
}

pub fn adapt_profile_event(event: ProfileEvent) -> Result<ControlEvent, SemanticAdapterError> {
    if (NS_DECK1..NS_DECK1 + 0x20).contains(&event.semantic_id) {
        return adapt_deck(event, DeckId::One, event.semantic_id - NS_DECK1);
    }
    if (NS_DECK2..NS_DECK2 + 0x20).contains(&event.semantic_id) {
        return adapt_deck(event, DeckId::Two, event.semantic_id - NS_DECK2);
    }

    match event.semantic_id {
        0x50 => pitch(
            event,
            Some(DeckId::One),
            SemanticControl::ChannelVolume,
            0x3fff,
        ),
        0x51 => pitch(
            event,
            Some(DeckId::Two),
            SemanticControl::ChannelVolume,
            0x3fff,
        ),
        0x52 => pitch(event, None, SemanticControl::Crossfader, 0x3fff),
        0x53 => button(event, Some(DeckId::One), SemanticControl::Pfl),
        0x54 => button(event, Some(DeckId::Two), SemanticControl::Pfl),
        0x55 => pitch(event, Some(DeckId::One), SemanticControl::Trim, 0x3fff),
        0x56 => pitch(event, Some(DeckId::Two), SemanticControl::Trim, 0x3fff),
        0x57 => pitch(event, Some(DeckId::One), SemanticControl::EqHigh, 0x3fff),
        0x58 => pitch(event, Some(DeckId::Two), SemanticControl::EqHigh, 0x3fff),
        0x59 => pitch(event, Some(DeckId::One), SemanticControl::EqMid, 0x3fff),
        0x5a => pitch(event, Some(DeckId::Two), SemanticControl::EqMid, 0x3fff),
        0x5b => pitch(event, Some(DeckId::One), SemanticControl::EqLow, 0x3fff),
        0x5c => pitch(event, Some(DeckId::Two), SemanticControl::EqLow, 0x3fff),
        0x5d => pitch(event, Some(DeckId::One), SemanticControl::Filter, 0x3fff),
        0x5e => pitch(event, Some(DeckId::Two), SemanticControl::Filter, 0x3fff),
        0x5f => pitch(event, None, SemanticControl::HeadphoneMix, 0x3fff),

        0x60 => relative(event, None, SemanticControl::BrowseDelta),
        0x61 => button(event, Some(DeckId::One), SemanticControl::Load),
        0x62 => button(event, Some(DeckId::Two), SemanticControl::Load),
        0x63 => button(event, None, SemanticControl::BrowsePress),
        0x64 => relative(event, None, SemanticControl::ShiftBrowseDelta),
        0x65 => button(event, None, SemanticControl::ShiftBrowsePress),
        0x66 => button(event, Some(DeckId::One), SemanticControl::ShiftLoad),
        0x67 => button(event, Some(DeckId::Two), SemanticControl::ShiftLoad),

        0x71 => button(event, None, SemanticControl::SmartCfx),
        0x72 => button(event, None, SemanticControl::SmartFader),
        0x73 => button(event, None, SemanticControl::BeatFxSelectNext),
        0x74 => button(event, None, SemanticControl::BeatFxSelectPrev),
        0x75 => button(event, None, SemanticControl::BeatFxBeatDec),
        0x76 => button(event, None, SemanticControl::BeatFxBeatInc),
        0x77 => beat_fx_target(event),
        0x78 => pitch(event, None, SemanticControl::BeatFxDepth, 0x007f),
        0x79 => button(event, None, SemanticControl::BeatFxOn),
        0x7a => button(event, None, SemanticControl::BeatFxClear),
        0x7b => pitch(event, None, SemanticControl::MasterVolume, 0x3fff),
        0x7c => button(event, None, SemanticControl::MasterCue),
        0x7d => pitch(event, None, SemanticControl::HeadphoneLevel, 0x3fff),
        0x7e => button(event, None, SemanticControl::SmartCfxShift),
        0x7f => button(event, None, SemanticControl::SmartFaderShift),
        0x83 => button(event, None, SemanticControl::BeatFxBeatDecShift),
        0x84 => button(event, None, SemanticControl::BeatFxBeatIncShift),
        _ => Err(SemanticAdapterError::UnsupportedId(event.semantic_id)),
    }
}

fn adapt_deck(
    event: ProfileEvent,
    deck: DeckId,
    control: u8,
) -> Result<ControlEvent, SemanticAdapterError> {
    let deck = Some(deck);

    match control {
        0 => button(event, deck, SemanticControl::Play),
        1 => button(event, deck, SemanticControl::Cue),
        2 => relative(event, deck, SemanticControl::JogScratch),
        3 => relative(event, deck, SemanticControl::JogBend),
        4 => button(event, deck, SemanticControl::JogTouch),
        5 => pitch(event, deck, SemanticControl::Tempo, 0x3fff),
        6 => button(event, deck, SemanticControl::Shift),
        7 => button(event, deck, SemanticControl::ToStart),
        8 => button(event, deck, SemanticControl::Sync),
        9 => button(event, deck, SemanticControl::TempoRange),
        10 => button(event, deck, SemanticControl::LoopIn),
        11 => button(event, deck, SemanticControl::LoopOut),
        12 => button(event, deck, SemanticControl::ReloopExit),
        13 => button(event, deck, SemanticControl::LoopHalve),
        14 => button(event, deck, SemanticControl::LoopDouble),
        15 => button(event, deck, SemanticControl::BeatJumpBack),
        16 => button(event, deck, SemanticControl::BeatJumpForward),
        17 => button(event, deck, SemanticControl::PadModeHotCue),
        18 => button(event, deck, SemanticControl::PadModeBeatLoop),
        19 => button(event, deck, SemanticControl::PadModeBeatJump),
        20 => button(event, deck, SemanticControl::PadModeKeyShift),
        21 => pad_action(event, deck),
        22 => button(event, deck, SemanticControl::PadModeKeyboard),
        23 => button(event, deck, SemanticControl::PadModePadFx1),
        24 => button(event, deck, SemanticControl::PadModePadFx2),
        25 => button(event, deck, SemanticControl::PadModeSampler),
        26 => relative(event, deck, SemanticControl::JogSearch),
        27 => button(event, deck, SemanticControl::JogSearchTouch),
        28 => deck_ext_action(event, deck),
        29 => relative(event, deck, SemanticControl::LoopSize),
        _ => Err(SemanticAdapterError::UnsupportedId(event.semantic_id)),
    }
}

fn expect_type(event: ProfileEvent, expected: u8) -> Result<(), SemanticAdapterError> {
    if event.semantic_type == expected {
        Ok(())
    } else {
        Err(SemanticAdapterError::TypeMismatch {
            expected,
            actual: event.semantic_type,
        })
    }
}

fn button(
    event: ProfileEvent,
    deck: Option<DeckId>,
    control: SemanticControl,
) -> Result<ControlEvent, SemanticAdapterError> {
    expect_type(event, TYPE_BUTTON)?;
    Ok(ControlEvent {
        deck,
        control,
        value: ControlValue::Pressed(event.value != 0),
    })
}

fn relative(
    event: ProfileEvent,
    deck: Option<DeckId>,
    control: SemanticControl,
) -> Result<ControlEvent, SemanticAdapterError> {
    expect_type(event, TYPE_ENCODER)?;
    Ok(ControlEvent {
        deck,
        control,
        value: ControlValue::Relative(event.value),
    })
}

fn pitch(
    event: ProfileEvent,
    deck: Option<DeckId>,
    control: SemanticControl,
    max: u16,
) -> Result<ControlEvent, SemanticAdapterError> {
    expect_type(event, TYPE_PITCH)?;
    if event.value < 0 || event.value as u16 > max {
        return Err(SemanticAdapterError::ValueOutOfRange);
    }

    Ok(ControlEvent {
        deck,
        control,
        value: ControlValue::Absolute {
            value: event.value as u16,
            max,
        },
    })
}

fn pad_action(
    event: ProfileEvent,
    deck: Option<DeckId>,
) -> Result<ControlEvent, SemanticAdapterError> {
    expect_type(event, TYPE_BUTTON)?;
    if !(0..=0xff).contains(&event.value) {
        return Err(SemanticAdapterError::ValueOutOfRange);
    }

    let raw = event.value as u8;
    let mode = match (raw >> 3) & 0x07 {
        0 => PadMode::HotCue,
        1 => PadMode::BeatLoop,
        2 => PadMode::BeatJump,
        3 => PadMode::KeyShift,
        4 => PadMode::Keyboard,
        5 => PadMode::PadFx1,
        6 => PadMode::PadFx2,
        7 => PadMode::Sampler,
        _ => return Err(SemanticAdapterError::ValueOutOfRange),
    };

    Ok(ControlEvent {
        deck,
        control: SemanticControl::PadAction,
        value: ControlValue::PadAction(PadAction {
            pad: raw & 0x07,
            mode,
            shifted: raw & 0x40 != 0,
            pressed: raw & 0x80 != 0,
        }),
    })
}

fn deck_ext_action(
    event: ProfileEvent,
    deck: Option<DeckId>,
) -> Result<ControlEvent, SemanticAdapterError> {
    expect_type(event, TYPE_BUTTON)?;
    if !(0..=0xff).contains(&event.value) {
        return Err(SemanticAdapterError::ValueOutOfRange);
    }

    let raw = event.value as u8;
    let action = match raw & 0x7f {
        0 => DeckExtAction::Censor,
        1 => DeckExtAction::SyncMaster,
        2 => DeckExtAction::ReloopStop,
        3 => DeckExtAction::LoopAdjustIn,
        4 => DeckExtAction::LoopAdjustOut,
        5 => DeckExtAction::Quantize,
        6 => DeckExtAction::SyncOff,
        _ => return Err(SemanticAdapterError::ValueOutOfRange),
    };

    Ok(ControlEvent {
        deck,
        control: SemanticControl::DeckExtAction,
        value: ControlValue::DeckExtAction(DeckExtActionValue {
            action,
            pressed: raw & 0x80 != 0,
        }),
    })
}

fn beat_fx_target(event: ProfileEvent) -> Result<ControlEvent, SemanticAdapterError> {
    expect_type(event, TYPE_BUTTON)?;
    let target = match event.value {
        0 => BeatFxTarget::ChannelOne,
        1 => BeatFxTarget::ChannelTwo,
        2 => BeatFxTarget::Both,
        _ => return Err(SemanticAdapterError::ValueOutOfRange),
    };

    Ok(ControlEvent {
        deck: None,
        control: SemanticControl::BeatFxTarget,
        value: ControlValue::BeatFxTarget(target),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_pad_action_wire_value() {
        let event = adapt_profile_event(ProfileEvent {
            semantic_type: TYPE_BUTTON,
            semantic_id: NS_DECK1 + 21,
            value: 0x80 | 0x40 | (2 << 3) | 3,
        })
        .unwrap();

        assert_eq!(
            event,
            ControlEvent {
                deck: Some(DeckId::One),
                control: SemanticControl::PadAction,
                value: ControlValue::PadAction(PadAction {
                    pad: 3,
                    mode: PadMode::BeatJump,
                    shifted: true,
                    pressed: true,
                }),
            }
        );
    }

    #[test]
    fn decodes_extended_deck_action() {
        let event = adapt_profile_event(ProfileEvent {
            semantic_type: TYPE_BUTTON,
            semantic_id: NS_DECK2 + 28,
            value: 0x80 | 6,
        })
        .unwrap();

        assert_eq!(
            event,
            ControlEvent {
                deck: Some(DeckId::Two),
                control: SemanticControl::DeckExtAction,
                value: ControlValue::DeckExtAction(DeckExtActionValue {
                    action: DeckExtAction::SyncOff,
                    pressed: true,
                }),
            }
        );
    }

    #[test]
    fn rejects_pitch_values_outside_declared_range() {
        assert_eq!(
            adapt_profile_event(ProfileEvent {
                semantic_type: TYPE_PITCH,
                semantic_id: 0x78,
                value: 128,
            }),
            Err(SemanticAdapterError::ValueOutOfRange)
        );
    }
}
