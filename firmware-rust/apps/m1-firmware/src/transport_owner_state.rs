//! Pure transport transitions shared by the P4 owner and composed host tests.
use pajoniiir_audio_runtime::pcm_stream::{
    PcmBlockDisposition, PcmStreamBlock, PcmStreamProgress, PcmTransportCursor,
};
use pajoniiir_controller_session::DeckId;
use pajoniiir_deck::DeckEffect;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TransportDeckOwnerState {
    pub(crate) playing: bool,
    pub(crate) position_ms: u32,
    pub(crate) pitch_centipercent: i16,
    pub(crate) loop_region: Option<(u32, u32)>,
    pub(crate) pcm_cursor: PcmTransportCursor,
    pub(crate) stream_faults: u32,
}

impl TransportDeckOwnerState {
    pub(crate) const fn new() -> Self {
        Self {
            playing: false,
            position_ms: 0,
            pitch_centipercent: 0,
            loop_region: None,
            pcm_cursor: PcmTransportCursor::new(),
            stream_faults: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TransportOwnerState {
    pub(crate) decks: [TransportDeckOwnerState; 2],
    pub(crate) accepted_commands: u32,
    source_epochs: [u32; 2],
}

impl TransportOwnerState {
    pub(crate) const fn new() -> Self {
        Self {
            decks: [TransportDeckOwnerState::new(); 2],
            accepted_commands: 0,
            source_epochs: [1; 2],
        }
    }

    pub(crate) fn sync_source_epochs(&mut self, epochs: [u32; 2]) {
        for (index, epoch) in epochs.into_iter().enumerate() {
            if self.source_epochs[index] != epoch {
                let faults = self.decks[index].stream_faults;
                self.decks[index] = TransportDeckOwnerState::new();
                self.decks[index].stream_faults = faults;
                self.source_epochs[index] = epoch;
            }
        }
    }

    pub(crate) fn accepts(&self, command: TransportCommand) -> bool {
        self.source_epochs[deck_index(command.deck())] == command.source_epoch
    }

    pub(crate) fn fail_seek(&mut self, command: TransportCommand) -> Option<TransportCompletion> {
        if !self.accepts(command) {
            return None;
        }
        let deck = command.deck();
        let state = self.deck_mut(deck);
        state.playing = false;
        state.stream_faults = state.stream_faults.saturating_add(1);
        Some(TransportCompletion {
            source_epoch: command.source_epoch,
            outcome: TransportOutcome::SourceFailed { deck },
        })
    }

    pub(crate) fn apply(&mut self, command: TransportCommand) -> Option<TransportCompletion> {
        if !self.accepts(command) {
            return None;
        }
        let effect = command.effect;
        let completion = match effect {
            DeckEffect::PlaybackRequest {
                deck,
                playing,
                request_id,
            } => {
                let state = self.deck_mut(deck);
                state.playing = playing;
                if playing {
                    state.pcm_cursor.begin_playback();
                }
                Some(TransportOutcome::Playback {
                    deck,
                    request_id,
                    succeeded: true,
                })
            }
            DeckEffect::Pause { deck } => {
                self.deck_mut(deck).playing = false;
                None
            }
            DeckEffect::Seek { deck, position_ms } => {
                self.deck_mut(deck).position_ms = position_ms;
                None
            }
            DeckEffect::SetPitchCentipercent { deck, value } => {
                self.deck_mut(deck).pitch_centipercent = value;
                None
            }
            DeckEffect::SetLoop {
                deck,
                start_ms,
                end_ms,
            } => {
                self.deck_mut(deck).loop_region = Some((start_ms, end_ms));
                None
            }
            DeckEffect::ClearLoop { deck } => {
                self.deck_mut(deck).loop_region = None;
                None
            }
            _ => return None,
        };
        self.accepted_commands = self.accepted_commands.saturating_add(1);
        completion.map(|outcome| TransportCompletion {
            source_epoch: command.source_epoch,
            outcome,
        })
    }

    pub(crate) fn bind_seek(
        &mut self,
        deck: DeckId,
        position_ms: u32,
        generation: u32,
        frame: u64,
        sample_rate_hz: u32,
    ) {
        let state = self.deck_mut(deck);
        state.position_ms = position_ms;
        state
            .pcm_cursor
            .bind_seek(generation, frame, sample_rate_hz);
        self.accepted_commands = self.accepted_commands.saturating_add(1);
    }

    pub(crate) fn consume_pcm_block(
        &mut self,
        deck: DeckId,
        block: &PcmStreamBlock,
    ) -> Option<TransportCompletion> {
        let source_epoch = self.source_epochs[deck_index(deck)];
        let state = self.deck_mut(deck);
        let outcome = match state.pcm_cursor.accept(block) {
            PcmBlockDisposition::Accepted {
                position_ms, eof, ..
            } => {
                state.position_ms = position_ms;
                if eof {
                    state.playing = false;
                    Some(TransportOutcome::StreamEnded {
                        deck,
                        generation: block.generation,
                        position_ms,
                        faulted: false,
                    })
                } else {
                    None
                }
            }
            PcmBlockDisposition::Stale => None,
            PcmBlockDisposition::Fault { position_ms } => {
                state.position_ms = position_ms;
                state.playing = false;
                state.stream_faults = state.stream_faults.saturating_add(1);
                Some(TransportOutcome::StreamEnded {
                    deck,
                    generation: block.generation,
                    position_ms,
                    faulted: true,
                })
            }
        };
        outcome.map(|outcome| TransportCompletion {
            source_epoch,
            outcome,
        })
    }

    pub(crate) fn consume_render_progress(
        &mut self,
        deck: DeckId,
        progress: PcmStreamProgress,
        frames: u16,
        active_generation: u32,
    ) -> Option<TransportCompletion> {
        let source_epoch = self.source_epochs[deck_index(deck)];
        let state = self.deck_mut(deck);
        let outcome =
            match state
                .pcm_cursor
                .accept_active_progress(progress, frames, active_generation)
            {
                PcmBlockDisposition::Accepted {
                    position_ms, eof, ..
                } => {
                    state.position_ms = position_ms;
                    if eof {
                        state.playing = false;
                        Some(TransportOutcome::StreamEnded {
                            deck,
                            generation: progress.generation,
                            position_ms,
                            faulted: false,
                        })
                    } else {
                        None
                    }
                }
                PcmBlockDisposition::Stale => None,
                PcmBlockDisposition::Fault { position_ms } => {
                    state.position_ms = position_ms;
                    state.playing = false;
                    state.stream_faults = state.stream_faults.saturating_add(1);
                    Some(TransportOutcome::StreamEnded {
                        deck,
                        generation: progress.generation,
                        position_ms,
                        faulted: true,
                    })
                }
            };
        outcome.map(|outcome| TransportCompletion {
            source_epoch,
            outcome,
        })
    }

    fn deck_mut(&mut self, deck: DeckId) -> &mut TransportDeckOwnerState {
        &mut self.decks[deck_index(deck)]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TransportCommand {
    pub(crate) effect: DeckEffect,
    pub(crate) source_epoch: u32,
}
impl TransportCommand {
    pub(crate) fn deck(self) -> DeckId {
        self.effect
            .source_deck()
            .expect("transport effect has a deck")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TransportCompletion {
    pub(crate) source_epoch: u32,
    pub(crate) outcome: TransportOutcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransportOutcome {
    Playback {
        deck: DeckId,
        request_id: u32,
        succeeded: bool,
    },
    StreamEnded {
        deck: DeckId,
        generation: u32,
        position_ms: u32,
        faulted: bool,
    },
    SourceFailed {
        deck: DeckId,
    },
}

impl TransportOutcome {
    pub(crate) const fn deck(self) -> DeckId {
        match self {
            Self::Playback { deck, .. }
            | Self::StreamEnded { deck, .. }
            | Self::SourceFailed { deck } => deck,
        }
    }
}

const fn deck_index(deck: DeckId) -> usize {
    match deck {
        DeckId::One => 0,
        DeckId::Two => 1,
    }
}
