//! Idle screensaver timing and wake handling (M2.5 `ui_idle` and the
//! `deck_core` activity callback).
//!
//! Pure timing: the caller feeds a wrapping millisecond clock and gets back an
//! edge (show, hide or nothing); rendering stays with the UI. Playback and
//! recording inhibit the screensaver and count as activity, so the countdown
//! restarts when they stop instead of blanking the screen at once. A physical
//! control that wakes the screensaver is spent on waking and never reaches a
//! deck; an authenticated Web mutation wakes it and still applies.

/// M2.5 default: two minutes.
pub const IDLE_DEFAULT_TIMEOUT_MS: u32 = 2 * 60 * 1000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdleAction {
    None,
    Show,
    Hide,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputSource {
    /// Touch or a controller event.
    Physical,
    /// An authenticated Web Remote mutation.
    Remote,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputDisposition {
    /// Deliver the input normally.
    Deliver,
    /// The input only woke the screensaver; drop it.
    ConsumedByWake,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdleScreen {
    /// 0 disables the screensaver.
    timeout_ms: u32,
    last_activity_ms: u32,
    shown: bool,
}

impl IdleScreen {
    pub const fn new(timeout_ms: u32, now_ms: u32) -> Self {
        Self {
            timeout_ms,
            last_activity_ms: now_ms,
            shown: false,
        }
    }

    /// Restarts the countdown, so raising the timeout cannot blank at once.
    pub fn set_timeout(&mut self, timeout_ms: u32, now_ms: u32) {
        self.timeout_ms = timeout_ms;
        self.last_activity_ms = now_ms;
    }

    pub const fn shown(&self) -> bool {
        self.shown
    }

    /// Notes input and decides whether it reaches the deck. Hiding happens on
    /// the next [`IdleScreen::tick`].
    pub fn input(&mut self, source: InputSource, now_ms: u32) -> InputDisposition {
        let waking = self.shown;
        self.last_activity_ms = now_ms;
        if waking && source == InputSource::Physical {
            InputDisposition::ConsumedByWake
        } else {
            InputDisposition::Deliver
        }
    }

    pub fn tick(&mut self, now_ms: u32, playing: bool, recording: bool) -> IdleAction {
        let inhibited = playing || recording;
        if inhibited {
            self.last_activity_ms = now_ms;
        }
        if self.timeout_ms == 0 || inhibited {
            return self.set_shown(false);
        }
        let elapsed = now_ms.wrapping_sub(self.last_activity_ms);
        self.set_shown(elapsed >= self.timeout_ms)
    }

    fn set_shown(&mut self, shown: bool) -> IdleAction {
        if shown == self.shown {
            return IdleAction::None;
        }
        self.shown = shown;
        if shown {
            IdleAction::Show
        } else {
            IdleAction::Hide
        }
    }

    /// Milliseconds until the screensaver appears; `None` if it cannot.
    pub fn remaining_ms(&self, now_ms: u32, playing: bool, recording: bool) -> Option<u32> {
        if self.timeout_ms == 0 || playing || recording {
            return None;
        }
        if self.shown {
            return Some(0);
        }
        Some(
            self.timeout_ms
                .saturating_sub(now_ms.wrapping_sub(self.last_activity_ms)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: u32 = IDLE_DEFAULT_TIMEOUT_MS;

    #[test]
    fn shows_after_the_timeout_and_hides_on_activity() {
        let mut idle = IdleScreen::new(T, 0);
        assert_eq!(idle.tick(T - 1, false, false), IdleAction::None);
        assert_eq!(idle.remaining_ms(T - 1, false, false), Some(1));
        assert_eq!(idle.tick(T, false, false), IdleAction::Show);
        assert_eq!(idle.tick(T + 5, false, false), IdleAction::None);
        assert_eq!(
            idle.input(InputSource::Physical, T + 10),
            InputDisposition::ConsumedByWake
        );
        assert_eq!(idle.tick(T + 11, false, false), IdleAction::Hide);
        assert_eq!(
            idle.input(InputSource::Physical, T + 12),
            InputDisposition::Deliver,
            "awake again"
        );
    }

    #[test]
    fn playback_and_recording_inhibit_and_restart_the_countdown() {
        let mut idle = IdleScreen::new(T, 0);
        assert_eq!(
            idle.tick(10 * T, true, false),
            IdleAction::None,
            "a long track never blanks"
        );
        assert_eq!(idle.remaining_ms(10 * T, true, false), None);
        assert_eq!(
            idle.tick(10 * T + 1, false, false),
            IdleAction::None,
            "stopping is activity"
        );
        assert_eq!(idle.tick(11 * T, false, false), IdleAction::Show);
        assert_eq!(
            idle.tick(11 * T + 1, false, true),
            IdleAction::Hide,
            "recording hides it"
        );
    }

    #[test]
    fn remote_mutations_wake_without_being_consumed() {
        let mut idle = IdleScreen::new(T, 0);
        idle.tick(T, false, false);
        assert!(idle.shown());
        assert_eq!(
            idle.input(InputSource::Remote, T + 1),
            InputDisposition::Deliver
        );
        assert_eq!(idle.tick(T + 2, false, false), IdleAction::Hide);
    }

    #[test]
    fn zero_disables_and_a_new_timeout_restarts_the_count() {
        let mut idle = IdleScreen::new(T, 0);
        idle.tick(T, false, false);
        idle.set_timeout(0, T + 1);
        assert_eq!(idle.tick(T + 2, false, false), IdleAction::Hide);
        assert_eq!(idle.tick(100 * T, false, false), IdleAction::None);
        assert_eq!(idle.remaining_ms(100 * T, false, false), None);
        idle.set_timeout(5 * T, 100 * T);
        assert_eq!(
            idle.tick(100 * T + T, false, false),
            IdleAction::None,
            "not back-dated"
        );
    }

    #[test]
    fn the_wrapping_millisecond_clock_needs_no_special_case() {
        let start = u32::MAX - 1_000;
        let mut idle = IdleScreen::new(T, start);
        assert_eq!(
            idle.tick(start.wrapping_add(T - 1), false, false),
            IdleAction::None
        );
        assert_eq!(
            idle.tick(start.wrapping_add(T), false, false),
            IdleAction::Show
        );
    }
}
