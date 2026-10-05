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
        assert_eq!(SemanticControl::BrowseDelta.domain(), ControlDomain::Library);
        assert_eq!(SemanticControl::JogSearch.domain(), ControlDomain::Deferred);
        assert_eq!(
            SemanticControl::SmartCfxShift.domain(),
            ControlDomain::Deferred
        );
    }

}
