//! Soak diagnostic: an unattended DJ that keeps both decks playing.
//!
//! Deck 1 walks the catalog's even indices and deck 2 its odd ones. When a
//! deck's track plays out, that deck loads its next track and presses Play
//! through the same product deck state a controller Play goes through, while
//! the other deck keeps playing. A failed load or a Play the engine does not
//! confirm moves on; after `MAX_FAILURES` in a row the deck is left alone.
//! A deck the operator paused is left paused; the controller keeps working.
//!
//! Built only with `soak-autoplay`; the product build has none of this.
#![forbid(unsafe_code)]

use core::sync::atomic::{AtomicU32, Ordering};

/// Catalog tracks, published by the load task once its catalog is ready.
static CATALOG: AtomicU32 = AtomicU32::new(0);

pub(crate) fn publish_catalog(tracks: u32) {
    CATALOG.store(tracks, Ordering::Release);
}

pub(crate) fn catalog() -> u32 {
    CATALOG.load(Ordering::Acquire)
}

/// Failed loads or unconfirmed Plays in a row before a deck is left alone.
pub(crate) const MAX_FAILURES: u8 = 3;
/// Steps a Play may wait for the engine's confirmation (one step per USB
/// client loop turn: a second or more).
pub(crate) const PLAY_WAIT_STEPS: u16 = 375;

/// One deck as the engine sees it at this step.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct View {
    /// The media lease is current; loads can be admitted.
    pub media: bool,
    pub loaded: bool,
    pub playing: bool,
    pub pending_load: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Action {
    None,
    Load(u32),
    Play,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    /// Ask for the next track.
    Idle,
    /// A load is pending.
    Loading,
    /// Loaded: press Play.
    Start,
    /// Play pressed; `seen` once the engine confirmed playback.
    Playing { seen: bool, waited: u16 },
    /// Too many failures in a row.
    Stopped,
}

pub(crate) struct AutoPlay {
    next: [u32; 2],
    phase: [Phase; 2],
    failures: [u8; 2],
    ended: [bool; 2],
    loads: [u32; 2],
}

impl AutoPlay {
    pub(crate) const fn new() -> Self {
        Self {
            next: [0, 1],
            phase: [Phase::Idle; 2],
            failures: [0; 2],
            ended: [false; 2],
            loads: [0; 2],
        }
    }

    /// The engine stopped deck `i` because its track played out.
    pub(crate) fn track_ended(&mut self, i: usize) {
        self.ended[i] = true;
    }

    /// The engine refused the load `step` asked for; try again next block.
    pub(crate) fn load_refused(&mut self, i: usize) {
        self.phase[i] = Phase::Idle;
    }

    /// Loads each deck completed.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) const fn loads(&self) -> [u32; 2] {
        self.loads
    }

    fn fail(&mut self, i: usize, retry: Phase) {
        self.failures[i] = self.failures[i].saturating_add(1);
        self.phase[i] = if self.failures[i] >= MAX_FAILURES {
            Phase::Stopped
        } else {
            retry
        };
    }

    /// The next action for deck `i` (0 or 1), once per USB client loop turn.
    pub(crate) fn step(&mut self, i: usize, view: View, tracks: u32) -> Action {
        let ended = core::mem::take(&mut self.ended[i]);
        match self.phase[i] {
            Phase::Idle => {
                if tracks == 0 || !view.media || view.pending_load || view.playing {
                    return Action::None;
                }
                self.phase[i] = Phase::Loading;
                Action::Load(self.next[i] % tracks)
            }
            Phase::Loading => {
                if view.pending_load {
                    return Action::None;
                }
                if view.loaded {
                    self.failures[i] = 0;
                    self.loads[i] = self.loads[i].wrapping_add(1);
                    self.phase[i] = Phase::Start;
                } else {
                    self.next[i] = self.next[i].wrapping_add(2);
                    self.fail(i, Phase::Idle);
                }
                Action::None
            }
            Phase::Start => {
                self.phase[i] = Phase::Playing {
                    seen: false,
                    waited: 0,
                };
                Action::Play
            }
            Phase::Playing { seen, waited } => {
                if view.playing {
                    self.failures[i] = 0;
                    self.phase[i] = Phase::Playing {
                        seen: true,
                        waited: 0,
                    };
                } else if !view.loaded && !view.pending_load {
                    // The media went away or the deck was unloaded.
                    self.phase[i] = Phase::Idle;
                } else if seen {
                    if ended {
                        self.next[i] = self.next[i].wrapping_add(2);
                        self.phase[i] = Phase::Idle;
                    }
                    // Otherwise the operator paused: leave the deck paused.
                } else if waited >= PLAY_WAIT_STEPS {
                    self.fail(i, Phase::Start);
                } else {
                    self.phase[i] = Phase::Playing {
                        seen,
                        waited: waited + 1,
                    };
                }
                Action::None
            }
            Phase::Stopped => Action::None,
        }
    }
}

#[cfg(test)]
mod tests;
