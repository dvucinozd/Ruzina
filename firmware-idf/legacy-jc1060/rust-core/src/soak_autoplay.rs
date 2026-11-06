//! Soak diagnostic: an unattended DJ that keeps both decks playing.
//!
//! Deck 1 walks the catalog's even indices and deck 2 its odd ones. When a
//! deck's track plays out, that deck loads its next track and presses Play
//! through the same product deck state a controller Play goes through, while
//! the other deck keeps playing. A failed load or a Play the engine does not
//! confirm moves on; after `MAX_FAILURES` in a row the deck is left alone.
//! A deck the operator paused is left paused; the controller keeps working.
//!
//! `Exerciser` then plays the DJ: every few seconds one gesture on a playing
//! deck, as a controller would send it, so a long run covers scratch, cues, loops,
//! effects, EQ, seeks and the flash journal together, not just playback.
//!
//! Built only with `soak-autoplay`; the product build has none of this.
#![forbid(unsafe_code)]

use core::sync::atomic::{AtomicU32, Ordering};
use ruzina_controller_core::{
    ControlEvent, ControlValue, DeckId, PadAction, PadMode, SemanticControl,
};

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
                } else if ended {
                    // The track played out, even if it ended before any
                    // step saw it playing (a short track): load the next.
                    self.failures[i] = 0;
                    self.next[i] = self.next[i].wrapping_add(2);
                    self.phase[i] = Phase::Idle;
                } else if !view.loaded && !view.pending_load {
                    // The media went away or the deck was unloaded.
                    self.phase[i] = Phase::Idle;
                } else if seen {
                    // The operator paused: leave the deck paused.
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

/// One thing a DJ does; the number is its code in the log (event 20).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum Gesture {
    Scratch,
    SetOrJumpCue,
    DeleteCue,
    Loop,
    BeatJump,
    PadFx1,
    PadFx2,
    BeatFx,
    Eq,
    Cue,
    Sync,
    /// Pauses every playing deck for a moment: the flash journal's window.
    PauseAll,
}

pub(crate) const GESTURES: [Gesture; 12] = [
    Gesture::Scratch,
    Gesture::SetOrJumpCue,
    Gesture::DeleteCue,
    Gesture::Loop,
    Gesture::BeatJump,
    Gesture::PadFx1,
    Gesture::PadFx2,
    Gesture::BeatFx,
    Gesture::Eq,
    Gesture::Cue,
    Gesture::Sync,
    Gesture::PauseAll,
];

const CONTROL_MAX: u16 = 16_383;
const CONTROL_CENTER: u16 = 8_192;
const JOG_TICKS: i16 = 8;
const SCRATCH_STEPS: u16 = 90;
const EQ_STEPS: u16 = 48;

#[derive(Clone, Copy)]
struct Running {
    gesture: Gesture,
    deck: DeckId,
    pad: u8,
    step: u16,
    /// Decks a Pause All stopped, to start again.
    paused: [bool; 2],
}

pub(crate) struct Exerciser {
    seed: u32,
    at_ms: u32,
    running: Option<Running>,
    started: Option<u32>,
}

fn button(deck: DeckId, control: SemanticControl, pressed: bool) -> Option<ControlEvent> {
    Some(ControlEvent {
        deck: Some(deck),
        control,
        value: ControlValue::Pressed(pressed),
    })
}

fn system(control: SemanticControl, pressed: bool) -> Option<ControlEvent> {
    Some(ControlEvent {
        deck: None,
        control,
        value: ControlValue::Pressed(pressed),
    })
}

fn pad(deck: DeckId, mode: PadMode, pad: u8, shifted: bool, pressed: bool) -> Option<ControlEvent> {
    Some(ControlEvent {
        deck: Some(deck),
        control: SemanticControl::PadAction,
        value: ControlValue::PadAction(PadAction {
            pad,
            mode,
            shifted,
            pressed,
        }),
    })
}

/// The gesture's event and the wait after it; `None` when it is over. A press
/// and its release take two steps, as on the controller.
fn script(run: &Running) -> Option<(Option<ControlEvent>, u32)> {
    use SemanticControl as C;
    let (deck, n, number) = (run.deck, run.step, run.pad);
    let click = |control, first: u16| {
        if n == first {
            (button(deck, control, true), 30)
        } else {
            (button(deck, control, false), 30)
        }
    };
    let hold_pad = |mode: PadMode, select: SemanticControl| match n {
        0 | 1 => Some(click(select, 0)),
        2 => Some((pad(deck, mode, number, false, true), 1_500)),
        3 => Some((pad(deck, mode, number, false, false), 300)),
        4 | 5 => Some(click(C::PadModeHotCue, 4)),
        _ => None,
    };
    match run.gesture {
        Gesture::Scratch => match n {
            0 => Some((button(deck, C::JogTouch, true), 20)),
            n if n <= SCRATCH_STEPS => {
                let ticks = if (n / 30) % 2 == 0 {
                    -JOG_TICKS
                } else {
                    JOG_TICKS
                };
                Some((
                    Some(ControlEvent {
                        deck: Some(deck),
                        control: C::JogScratch,
                        value: ControlValue::Relative(ticks),
                    }),
                    5,
                ))
            }
            n if n == SCRATCH_STEPS + 1 => Some((button(deck, C::JogTouch, false), 20)),
            _ => None,
        },
        Gesture::SetOrJumpCue => match n {
            0 => Some((pad(deck, PadMode::HotCue, number, false, true), 30)),
            1 => Some((pad(deck, PadMode::HotCue, number, false, false), 30)),
            _ => None,
        },
        Gesture::DeleteCue => match n {
            0 => Some((pad(deck, PadMode::HotCue, number, true, true), 30)),
            1 => Some((pad(deck, PadMode::HotCue, number, true, false), 30)),
            _ => None,
        },
        Gesture::Loop => match n {
            0 | 1 => Some(click(C::ReloopExit, 0)),
            2 => Some((None, 3_000)),
            3 | 4 => Some(click(C::ReloopExit, 3)),
            _ => None,
        },
        Gesture::BeatJump => match n {
            0 | 1 => Some(click(C::PadModeBeatJump, 0)),
            2 => Some((pad(deck, PadMode::BeatJump, number, false, true), 30)),
            3 => Some((pad(deck, PadMode::BeatJump, number, false, false), 30)),
            4 | 5 => Some(click(C::PadModeHotCue, 4)),
            _ => None,
        },
        Gesture::PadFx1 => hold_pad(PadMode::PadFx1, C::PadModePadFx1),
        Gesture::PadFx2 => hold_pad(PadMode::PadFx2, C::PadModePadFx2),
        Gesture::BeatFx => match n {
            0 => Some((system(C::BeatFxSelectNext, true), 30)),
            1 => Some((system(C::BeatFxSelectNext, false), 30)),
            2 => Some((system(C::BeatFxOn, true), 30)),
            3 => Some((system(C::BeatFxOn, false), 5_000)),
            4 => Some((system(C::BeatFxOn, true), 30)),
            5 => Some((system(C::BeatFxOn, false), 30)),
            _ => None,
        },
        Gesture::Eq => {
            if n >= EQ_STEPS + 4 {
                return None;
            }
            let control = [C::EqHigh, C::EqMid, C::EqLow, C::Filter][usize::from(n % 4)];
            // Up and back, ending at the center for all four.
            let distance = n.min(EQ_STEPS.saturating_sub(n)).min(EQ_STEPS / 2);
            let value = CONTROL_CENTER + distance * 300;
            Some((
                Some(ControlEvent {
                    deck: Some(deck),
                    control,
                    value: ControlValue::Absolute {
                        value: value.min(CONTROL_MAX),
                        max: CONTROL_MAX,
                    },
                }),
                50,
            ))
        }
        Gesture::Cue => match n {
            0 => Some((button(deck, C::Cue, true), 100)),
            1 => Some((button(deck, C::Cue, false), 500)),
            2 | 3 => Some(click(C::Play, 2)),
            _ => None,
        },
        Gesture::Sync => match n {
            0 => Some((button(deck, C::Sync, true), 30)),
            1 => Some((button(deck, C::Sync, false), 4_000)),
            2 | 3 => Some(click(C::Sync, 2)),
            _ => None,
        },
        Gesture::PauseAll => {
            let play = |deck, slot: usize, pressed, wait| {
                let event = if run.paused[slot] {
                    button(deck, C::Play, pressed)
                } else {
                    None
                };
                Some((event, wait))
            };
            match n {
                0 => play(DeckId::One, 0, true, 30),
                1 => play(DeckId::One, 0, false, 30),
                2 => play(DeckId::Two, 1, true, 30),
                3 => play(DeckId::Two, 1, false, 1_500),
                4 => play(DeckId::One, 0, true, 30),
                5 => play(DeckId::One, 0, false, 30),
                6 => play(DeckId::Two, 1, true, 30),
                7 => play(DeckId::Two, 1, false, 30),
                _ => None,
            }
        }
    }
}

impl Exerciser {
    pub(crate) const fn new() -> Self {
        Self {
            seed: 0x2f6e_2b1d,
            at_ms: 0,
            running: None,
            started: None,
        }
    }

    fn random(&mut self) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed
    }

    /// The gesture that just started, for the log: `gesture << 8 | deck`.
    pub(crate) fn take_started(&mut self) -> Option<u32> {
        self.started.take()
    }

    /// The next event, if one is due at `now_ms`. `playing` chooses the deck;
    /// with nothing playing it waits, so the load path stays the autoplay's.
    pub(crate) fn next(&mut self, now_ms: u32, playing: [bool; 2]) -> Option<ControlEvent> {
        if (now_ms.wrapping_sub(self.at_ms) as i32) < 0 {
            return None;
        }
        if self.running.is_none() {
            if !playing[0] && !playing[1] {
                self.at_ms = now_ms.wrapping_add(500);
                return None;
            }
            let gesture = GESTURES[self.random() as usize % GESTURES.len()];
            let slot = self.random() as usize % 2;
            let slot = if playing[slot] { slot } else { 1 - slot };
            let deck = if slot == 0 { DeckId::One } else { DeckId::Two };
            // The presets are on pads 1 to 4, and four cues keep the journal busy.
            let pad = (self.random() % 4) as u8;
            self.started = Some((gesture as u32) << 8 | slot as u32);
            self.running = Some(Running {
                gesture,
                deck,
                pad,
                step: 0,
                paused: playing,
            });
        }
        let run = self.running.as_mut()?;
        let Some((event, wait)) = script(run) else {
            self.running = None;
            let gap = 1_000 + self.random() % 3_000;
            self.at_ms = now_ms.wrapping_add(gap);
            return None;
        };
        run.step += 1;
        self.at_ms = now_ms.wrapping_add(wait);
        event
    }
}
