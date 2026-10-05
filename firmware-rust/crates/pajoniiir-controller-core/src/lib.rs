#![no_std]
#![forbid(unsafe_code)]

pub use pajoniiir_core::DeckId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PadMode {
    HotCue,
    BeatLoop,
    BeatJump,
    KeyShift,
    Keyboard,
    PadFx1,
    PadFx2,
    Sampler,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PadAction {
    pub pad: u8,
    pub mode: PadMode,
    pub shifted: bool,
    pub pressed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeckExtAction {
    Censor,
    SyncMaster,
    ReloopStop,
    LoopAdjustIn,
    LoopAdjustOut,
    Quantize,
    SyncOff,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeckExtActionValue {
    pub action: DeckExtAction,
    pub pressed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BeatFxTarget {
    ChannelOne,
    ChannelTwo,
    Both,
}

/// Controller-independent product action.
///
/// Raw USB MIDI bytes and controller-specific note/CC values must be translated
/// into this semantic layer before they can affect product state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticControl {
    Play,
    Cue,
    JogScratch,
    JogBend,
    JogTouch,
    Tempo,
    Shift,
    ToStart,
    Sync,
    TempoRange,
    LoopIn,
    LoopOut,
    ReloopExit,
    LoopHalve,
    LoopDouble,
    BeatJumpBack,
    BeatJumpForward,
    PadModeHotCue,
    PadModeBeatLoop,
    PadModeBeatJump,
    PadModeKeyShift,
    PadAction,
    PadModeKeyboard,
    PadModePadFx1,
    PadModePadFx2,
    PadModeSampler,
    JogSearch,
    JogSearchTouch,
    DeckExtAction,
    LoopSize,
    ChannelVolume,
    Crossfader,
    Trim,
    EqHigh,
    EqMid,
    EqLow,
    Filter,
    Pfl,
    HeadphoneMix,
    BrowseDelta,
    Load,
    BrowsePress,
    ShiftBrowseDelta,
    ShiftBrowsePress,
    ShiftLoad,
    SmartCfx,
    SmartFader,
    BeatFxSelectNext,
    BeatFxSelectPrev,
    BeatFxBeatDec,
    BeatFxBeatInc,
    BeatFxTarget,
    BeatFxDepth,
    BeatFxOn,
    BeatFxClear,
    MasterVolume,
    MasterCue,
    HeadphoneLevel,
    SmartCfxShift,
    SmartFaderShift,
    BeatFxBeatDecShift,
    BeatFxBeatIncShift,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlDomain {
    Deck,
    Mixer,
    Library,
    Deferred,
}

impl SemanticControl {
    pub const fn domain(self) -> ControlDomain {
        match self {
            Self::Play
            | Self::Cue
            | Self::JogScratch
            | Self::JogBend
            | Self::JogTouch
            | Self::Tempo
            | Self::Shift
            | Self::ToStart
            | Self::Sync
            | Self::TempoRange
            | Self::LoopIn
            | Self::LoopOut
            | Self::ReloopExit
            | Self::LoopHalve
            | Self::LoopDouble
            | Self::BeatJumpBack
            | Self::BeatJumpForward
            | Self::PadModeHotCue
            | Self::PadModeBeatLoop
            | Self::PadModeBeatJump
            | Self::PadAction
            | Self::PadModePadFx1
            | Self::PadModePadFx2
            | Self::DeckExtAction
            | Self::LoopSize
            | Self::BeatFxSelectNext
            | Self::BeatFxSelectPrev
            | Self::BeatFxBeatDec
            | Self::BeatFxBeatInc
            | Self::BeatFxTarget
            | Self::BeatFxDepth
            | Self::BeatFxOn
            | Self::BeatFxClear
            | Self::BeatFxBeatDecShift
            | Self::BeatFxBeatIncShift => ControlDomain::Deck,

            Self::ChannelVolume
            | Self::Crossfader
            | Self::Trim
            | Self::EqHigh
            | Self::EqMid
            | Self::EqLow
            | Self::Filter
            | Self::Pfl
            | Self::HeadphoneMix
            | Self::SmartCfx
            | Self::SmartFader
            | Self::MasterVolume
            | Self::MasterCue
            | Self::HeadphoneLevel => ControlDomain::Mixer,

            Self::BrowseDelta
            | Self::Load
            | Self::BrowsePress
            | Self::ShiftBrowseDelta
            | Self::ShiftBrowsePress
            | Self::ShiftLoad => ControlDomain::Library,

            Self::PadModeKeyShift
            | Self::PadModeKeyboard
            | Self::PadModeSampler
            | Self::JogSearch
            | Self::JogSearchTouch
            | Self::SmartCfxShift
            | Self::SmartFaderShift => ControlDomain::Deferred,
        }
    }
}

/// Exact controller-independent payload.
///
/// Absolute controls keep their integer range instead of being prematurely
/// converted to floating point, which preserves deterministic replay and host
/// parity with the released controller-profile runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlValue {
    Pressed(bool),
    Relative(i16),
    Absolute { value: u16, max: u16 },
    PadAction(PadAction),
    DeckExtAction(DeckExtActionValue),
    BeatFxTarget(BeatFxTarget),
}

/// A controller-independent event entering the product scheduler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControlEvent {
    pub deck: Option<DeckId>,
    pub control: SemanticControl,
    pub value: ControlValue,
}

/// Semantic LED state before controller-specific output mapping.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedState {
    Off,
    On,
    Blink,
    Value(u8),
}

impl LedState {
    pub const fn wire_value(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::On => 1,
            Self::Blink => 2,
            Self::Value(value) => value & 0x7f,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct SemanticLedId(u8);

impl SemanticLedId {
    pub const CUE: Self = Self(0);
    pub const PLAY: Self = Self(1);
    pub const PFL: Self = Self(4);
    pub const VU_METER: Self = Self(5);
    pub const PAD_MODE_HOT_CUE: Self = Self(6);
    pub const PAD_MODE_KEYBOARD: Self = Self(7);
    pub const PAD_MODE_PAD_FX1: Self = Self(8);
    pub const PAD_MODE_PAD_FX2: Self = Self(9);
    pub const PAD_MODE_BEAT_JUMP: Self = Self(10);
    pub const PAD_MODE_BEAT_LOOP: Self = Self(11);
    pub const PAD_MODE_SAMPLER: Self = Self(12);
    pub const PAD_MODE_KEY_SHIFT: Self = Self(13);
    pub const SYNC: Self = Self(14);
    pub const LOOP_IN: Self = Self(15);
    pub const LOOP_OUT: Self = Self(16);
    pub const SMART_CFX: Self = Self(41);
    pub const SMART_FADER: Self = Self(42);
    pub const BEAT_FX_ON: Self = Self(43);
    pub const MASTER_CUE: Self = Self(52);
    pub const CENSOR: Self = Self(53);
    pub const CUE_SHIFT: Self = Self(54);
    pub const LOOP_ADJUST_IN: Self = Self(55);
    pub const LOOP_ADJUST_OUT: Self = Self(56);
    pub const TRACK_LOAD_DECK1: Self = Self(57);
    pub const TRACK_LOAD_DECK2: Self = Self(58);

    pub const BEAT_LOOP_PAD_BASE: u8 = 17;
    pub const PAD_FX1_PAD_BASE: u8 = 25;
    pub const PAD_FX2_PAD_BASE: u8 = 33;
    pub const HOT_CUE_PAD_BASE: u8 = 44;
    pub const BEAT_JUMP_PAD_BASE: u8 = 59;
    pub const BEAT_JUMP_SHIFT_HELPER_BASE: u8 = 67;
    pub const MAX_ID: u8 = 68;

    pub const fn from_raw(value: u8) -> Option<Self> {
        match value {
            0 | 1 | 4..=68 => Some(Self(value)),
            _ => None,
        }
    }

    pub const fn as_u8(self) -> u8 {
        self.0
    }

    pub const fn beat_loop_pad(index: u8) -> Option<Self> {
        Self::bank(Self::BEAT_LOOP_PAD_BASE, 8, index)
    }

    pub const fn pad_fx1_pad(index: u8) -> Option<Self> {
        Self::bank(Self::PAD_FX1_PAD_BASE, 8, index)
    }

    pub const fn pad_fx2_pad(index: u8) -> Option<Self> {
        Self::bank(Self::PAD_FX2_PAD_BASE, 8, index)
    }

    pub const fn hot_cue_pad(index: u8) -> Option<Self> {
        Self::bank(Self::HOT_CUE_PAD_BASE, 8, index)
    }

    pub const fn beat_jump_pad(index: u8) -> Option<Self> {
        Self::bank(Self::BEAT_JUMP_PAD_BASE, 8, index)
    }

    pub const fn beat_jump_shift_helper(index: u8) -> Option<Self> {
        Self::bank(Self::BEAT_JUMP_SHIFT_HELPER_BASE, 2, index)
    }

    const fn bank(base: u8, count: u8, index: u8) -> Option<Self> {
        if index < count {
            Some(Self(base + index))
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LedUpdate {
    pub led: SemanticLedId,
    pub deck: Option<DeckId>,
    pub state: LedState,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_event_does_not_encode_midi_transport() {
        let event = ControlEvent {
            deck: Some(DeckId::One),
            control: SemanticControl::Play,
            value: ControlValue::Pressed(true),
        };

        assert_eq!(event.control, SemanticControl::Play);
        assert_eq!(event.deck, Some(DeckId::One));
    }

    #[test]
    fn pad_action_is_semantic_not_wire_packed() {
        let value = ControlValue::PadAction(PadAction {
            pad: 3,
            mode: PadMode::BeatJump,
            shifted: true,
            pressed: true,
        });

        assert_eq!(
            value,
            ControlValue::PadAction(PadAction {
                pad: 3,
                mode: PadMode::BeatJump,
                shifted: true,
                pressed: true,
            })
        );
    }

    #[test]
    fn control_domains_do_not_silently_route_unimplemented_controls() {
        assert_eq!(SemanticControl::Play.domain(), ControlDomain::Deck);
        assert_eq!(SemanticControl::BeatFxOn.domain(), ControlDomain::Deck);
        assert_eq!(SemanticControl::Crossfader.domain(), ControlDomain::Mixer);
        assert_eq!(
            SemanticControl::BrowseDelta.domain(),
            ControlDomain::Library
        );
        assert_eq!(SemanticControl::JogSearch.domain(), ControlDomain::Deferred);
        assert_eq!(
            SemanticControl::SmartCfxShift.domain(),
            ControlDomain::Deferred
        );
    }

    #[test]
    fn released_semantic_led_ids_and_banks_are_stable() {
        assert_eq!(SemanticLedId::PLAY.as_u8(), 1);
        assert_eq!(SemanticLedId::SYNC.as_u8(), 14);
        assert_eq!(SemanticLedId::BEAT_FX_ON.as_u8(), 43);
        assert_eq!(SemanticLedId::MASTER_CUE.as_u8(), 52);
        assert_eq!(SemanticLedId::hot_cue_pad(7).unwrap().as_u8(), 51);
        assert_eq!(SemanticLedId::beat_jump_pad(7).unwrap().as_u8(), 66);
        assert_eq!(
            SemanticLedId::beat_jump_shift_helper(1).unwrap().as_u8(),
            68
        );
        assert_eq!(SemanticLedId::hot_cue_pad(8), None);
        assert_eq!(SemanticLedId::from_raw(2), None);
        assert_eq!(SemanticLedId::from_raw(68), Some(SemanticLedId(68)));
    }

    #[test]
    fn led_state_wire_encoding_preserves_value_semantics() {
        assert_eq!(LedState::Off.wire_value(), 0);
        assert_eq!(LedState::On.wire_value(), 1);
        assert_eq!(LedState::Blink.wire_value(), 2);
        assert_eq!(LedState::Value(0xff).wire_value(), 0x7f);
    }

}
