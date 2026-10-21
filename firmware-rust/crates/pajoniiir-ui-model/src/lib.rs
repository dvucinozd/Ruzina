#![no_std]
#![forbid(unsafe_code)]

mod mix_fx;
pub use mix_fx::*;

use pajoniiir_controller_core::{
    ControlEvent, ControlValue, DeckExtAction, DeckExtActionValue, PadAction, PadMode,
    SemanticControl,
};
use pajoniiir_core::DeckId;
use pajoniiir_deck::DeckState;
use pajoniiir_hot_cues::{HotCueBank, HotCueSlot};

/// Stable UI adapter action codes. All deck mutations enter the same semantic
/// layer as physical controllers; adapters must gate unloaded decks separately.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeckAction {
    Play,
    Cue,
    Sync,
    JumpBack,
    JumpForward,
    LoopIn,
    LoopOut,
    LoopExit,
    Quantize,
    TempoRange,
}

impl DeckAction {
    pub const fn from_code(code: i32) -> Option<Self> {
        Some(match code {
            0 => Self::Play,
            1 => Self::Cue,
            2 => Self::Sync,
            3 => Self::JumpBack,
            4 => Self::JumpForward,
            5 => Self::LoopIn,
            6 => Self::LoopOut,
            7 => Self::LoopExit,
            8 => Self::Quantize,
            9 => Self::TempoRange,
            _ => return None,
        })
    }
    pub const fn event(self, deck: DeckId, pressed: bool) -> ControlEvent {
        let control = match self {
            Self::Play => SemanticControl::Play,
            Self::Cue => SemanticControl::Cue,
            Self::Sync => SemanticControl::Sync,
            Self::JumpBack => SemanticControl::BeatJumpBack,
            Self::JumpForward => SemanticControl::BeatJumpForward,
            Self::LoopIn => SemanticControl::LoopIn,
            Self::LoopOut => SemanticControl::LoopOut,
            Self::LoopExit => SemanticControl::ReloopExit,
            Self::Quantize => SemanticControl::DeckExtAction,
            Self::TempoRange => SemanticControl::TempoRange,
        };
        ControlEvent {
            deck: Some(deck),
            control,
            value: if matches!(self, Self::Quantize) {
                ControlValue::DeckExtAction(DeckExtActionValue {
                    action: DeckExtAction::Quantize,
                    pressed,
                })
            } else {
                ControlValue::Pressed(pressed)
            },
        }
    }
}

pub const fn cue_event(deck: DeckId, slot: u8, clear: bool, pressed: bool) -> Option<ControlEvent> {
    if slot >= 8 {
        return None;
    }
    Some(ControlEvent {
        deck: Some(deck),
        control: SemanticControl::PadAction,
        value: ControlValue::PadAction(PadAction {
            pad: slot,
            mode: PadMode::HotCue,
            shifted: clear,
            pressed,
        }),
    })
}

/// Small authoritative projection, including effective (locally merged) cues.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeckPresentation {
    pub transport: DeckSnapshot,
    pub quantize: bool,
    pub tempo_range_percent: u16,
    pub loop_region: Option<(u32, u32)>,
    pub cues: [Option<HotCueSlot>; 8],
}

pub fn present_deck(
    deck: DeckId,
    state: &DeckState,
    loaded: bool,
    duration_ms: u32,
    bank: Option<HotCueBank>,
) -> DeckPresentation {
    let bpm = (u64::from(state.base_bpm_x100)
        * 10
        * (10_000i64 + i64::from(state.pitch_centipercent)).max(0) as u64
        / 10_000)
        .min(u64::from(u32::MAX)) as u32;
    DeckPresentation {
        transport: DeckSnapshot {
            deck,
            loaded,
            playing: loaded && state.playing,
            bpm_milli: if loaded { bpm } else { 0 },
            position_ms: if loaded {
                u64::from(state.position_ms)
            } else {
                0
            },
            duration_ms: u64::from(duration_ms),
            pitch_centi_percent: i32::from(state.pitch_centipercent),
            sync: loaded && state.sync_enabled,
            master_tempo: state.master_tempo,
        },
        quantize: state.quantize_enabled,
        tempo_range_percent: state.tempo_range_percent,
        loop_region: if loaded {
            state.loop_state.active.map(|r| (r.start_ms, r.end_ms))
        } else {
            None
        },
        cues: core::array::from_fn(|index| {
            bank.filter(|_| loaded)
                .and_then(|bank| bank.slot(index as u8))
        }),
    }
}

/// Immutable deck snapshot published to UI/web adapters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeckSnapshot {
    pub deck: DeckId,
    pub loaded: bool,
    pub playing: bool,
    pub bpm_milli: u32,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub pitch_centi_percent: i32,
    pub sync: bool,
    pub master_tempo: bool,
}

impl DeckSnapshot {
    pub const fn empty(deck: DeckId) -> Self {
        Self {
            deck,
            loaded: false,
            playing: false,
            bpm_milli: 0,
            position_ms: 0,
            duration_ms: 0,
            pitch_centi_percent: 0,
            sync: false,
            master_tempo: false,
        }
    }
}

/// Product UI snapshot.
///
/// Large waveform data is referenced separately so normal state publication
/// stays small and bounded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiSnapshot {
    pub deck_one: DeckSnapshot,
    pub deck_two: DeckSnapshot,
    pub crossfader_milli: i16,
    pub master_milli: u16,
}

impl Default for UiSnapshot {
    fn default() -> Self {
        Self {
            deck_one: DeckSnapshot::empty(DeckId::One),
            deck_two: DeckSnapshot::empty(DeckId::Two),
            crossfader_milli: 0,
            master_milli: 1000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_snapshot_is_safe_and_stopped() {
        let state = UiSnapshot::default();
        assert!(!state.deck_one.playing);
        assert!(!state.deck_two.playing);
        assert_eq!(state.master_milli, 1000);
    }

    #[test]
    fn presentation_reads_effective_cues_and_real_pitch_instead_of_ui_toggles() {
        let mut state = DeckState::new();
        state.base_bpm_x100 = 12_800;
        state.pitch_centipercent = -625;
        state.playing = true;
        state.sync_enabled = true;
        let mut bank = HotCueBank::empty(pajoniiir_core::MediaTrackId([1; 16]));
        bank.set_single(2, 1234);
        let p = present_deck(DeckId::Two, &state, true, 14_000, Some(bank));
        assert_eq!(p.transport.bpm_milli, 120_000);
        assert!(p.transport.playing && p.transport.sync);
        assert_eq!(p.cues[2].unwrap().pos_ms, 1234);
        let empty = present_deck(DeckId::Two, &state, false, 0, Some(bank));
        assert!(!empty.transport.playing && !empty.transport.sync);
        assert_eq!(empty.transport.bpm_milli, 0);
        assert!(empty.cues.iter().all(Option::is_none));
    }

    #[test]
    fn ui_action_release_is_not_a_new_button_press() {
        let mut state = pajoniiir_deck::DeckProductState::new();
        let release = DeckAction::Play.event(DeckId::One, false);
        assert!(state.handle_control(release).is_empty());
        let press = DeckAction::Play.event(DeckId::One, true);
        assert_eq!(state.handle_control(press).len(), 1);
        assert!(
            state
                .handle_control(DeckAction::Quantize.event(DeckId::Two, false))
                .is_empty()
        );
        assert!(!state.deck(DeckId::Two).quantize_enabled);
        state.handle_control(DeckAction::Quantize.event(DeckId::Two, true));
        assert!(state.deck(DeckId::Two).quantize_enabled);
        assert!(!state.deck(DeckId::One).quantize_enabled);
    }
}
