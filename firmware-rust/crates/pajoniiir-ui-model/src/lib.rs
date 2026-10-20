#![no_std]
#![forbid(unsafe_code)]

use pajoniiir_core::DeckId;

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
}
