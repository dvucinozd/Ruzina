//! The boot screen and the screensaver, as the donor firmware (Pajoniiir) has them: the
//! word RUZINA, its letters lit in turn, for the first seconds after boot and again once
//! the deck has stood idle for the wait set on the Settings page.
//!
//! The timing is the shared [`IdleScreen`]: nothing touched or pressed for the wait, and
//! no deck playing. A playing deck holds it off and counts as activity, so it does not
//! show the moment a long track ends.
//!
//! Three things wake it, from two tasks:
//! - **Touch**, on the screen task: the screensaver covers the whole screen and takes the
//!   tap, so a waking tap presses nothing below it.
//! - **The controller**, on the USB client task ([`controller`]): a button, pad, jog or
//!   browse turn only wakes the screen. Faders and knobs always act, so the mix stays
//!   where the controller's faders are.
//! - **The Web Remote**, on the USB client task ([`remote`]): it wakes the screen and acts,
//!   since the page shows what it does.
//!
//! The screen task sees the other task's activity once a frame ([`Saver::tick`]); until
//! then the screensaver stays up, and a second press in that frame only wakes it too.
use core::sync::atomic::{AtomicBool, Ordering};
use ruzina_controller_core::{ControlEvent, ControlValue};
#[cfg(any(test, feature = "slint-ui"))]
use ruzina_ui_model::idle::{IdleAction, IdleScreen, InputSource};

/// How long the boot screen shows, from the screen's first frame.
#[cfg(any(test, feature = "slint-ui"))]
pub(crate) const BOOT_MS: u32 = 3_000;

/// Activity from the USB client task since the screen task last looked.
static ACTIVITY: AtomicBool = AtomicBool::new(false);
/// The screensaver shows: the screen task's word, for the USB client task.
static SHOWN: AtomicBool = AtomicBool::new(false);
/// A firmware update is being written (`deck_ota`), from the web task.
#[cfg(any(feature = "web-remote", all(feature = "slint-ui", not(test))))]
static UPDATING: AtomicBool = AtomicBool::new(false);

/// From the web task: a firmware update starts writing its slot, or ends unfinished. While
/// it is written the screen shows UPDATING FIRMWARE over everything and, once that is
/// drawn, draws nothing more: writing flash stalls the screen's tasks, and frames drawn
/// across the stalls flickered (s211).
#[cfg(feature = "web-remote")]
pub(crate) fn set_updating(on: bool) {
    UPDATING.store(on, Ordering::Release);
}

/// A firmware update is being written.
#[cfg(any(feature = "web-remote", all(feature = "slint-ui", not(test))))]
pub(crate) fn updating() -> bool {
    UPDATING.load(Ordering::Acquire)
}

/// What the screen shows over the deck.
#[cfg(any(test, feature = "slint-ui"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Cover {
    None,
    Boot,
    Screensaver,
    /// A firmware update is being written (`updating`); only the screen shows it.
    #[cfg_attr(test, allow(dead_code))]
    Updating,
}

/// A controller event, on the USB client task: whether it goes on to the deck. While the
/// screensaver shows, a press or a turn only wakes it; a release, a fader or a knob goes on.
pub(crate) fn controller(event: &ControlEvent) -> bool {
    ACTIVITY.store(true, Ordering::Release);
    !(SHOWN.load(Ordering::Acquire) && wakes_only(event.value))
}

/// A Web Remote tap, control, load or seek, on the USB client task.
#[cfg(any(test, feature = "web-remote"))]
pub(crate) fn remote() {
    ACTIVITY.store(true, Ordering::Release);
}

/// The controls a sleeping screen keeps: a press, a pad or a turn wakes it and goes no
/// further, while a fader or knob sets where the controller's own sits and a release ends
/// what came before.
fn wakes_only(value: ControlValue) -> bool {
    match value {
        ControlValue::Absolute { .. } => false,
        ControlValue::Pressed(pressed) => pressed,
        ControlValue::PadAction(pad) => pad.pressed,
        ControlValue::DeckExtAction(action) => action.pressed,
        ControlValue::Relative(_) | ControlValue::BeatFxTarget(_) => true,
    }
}

/// The screen task's side: the boot screen, then the screensaver's timing.
#[cfg(any(test, feature = "slint-ui"))]
pub(crate) struct Saver {
    idle: IdleScreen,
    timeout_ms: u32,
    /// When the screen began, while the boot screen shows.
    booting: Option<u32>,
}

#[cfg(any(test, feature = "slint-ui"))]
impl Saver {
    /// At the screen's first frame, `now_ms`: the boot screen shows for [`BOOT_MS`].
    pub(crate) fn new(now_ms: u32, timeout_ms: u32) -> Self {
        SHOWN.store(false, Ordering::Release);
        Self {
            idle: IdleScreen::new(timeout_ms, now_ms),
            timeout_ms,
            booting: Some(now_ms),
        }
    }

    /// One frame: the screen touched, a deck playing, and the wait the settings hold.
    pub(crate) fn tick(
        &mut self,
        now_ms: u32,
        touched: bool,
        playing: bool,
        timeout_ms: u32,
    ) -> Cover {
        if let Some(started_ms) = self.booting {
            if now_ms.wrapping_sub(started_ms) < BOOT_MS {
                return Cover::Boot;
            }
            // The wait counts from when the deck first shows.
            self.booting = None;
            self.idle.set_timeout(timeout_ms, now_ms);
            self.timeout_ms = timeout_ms;
            ACTIVITY.store(false, Ordering::Release);
        }
        if timeout_ms != self.timeout_ms {
            self.idle.set_timeout(timeout_ms, now_ms);
            self.timeout_ms = timeout_ms;
        }
        if ACTIVITY.swap(false, Ordering::AcqRel) || touched {
            self.idle.input(InputSource::Physical, now_ms);
        }
        match self.idle.tick(now_ms, playing, false) {
            IdleAction::Show => SHOWN.store(true, Ordering::Release),
            IdleAction::Hide => SHOWN.store(false, Ordering::Release),
            IdleAction::None => {}
        }
        if self.idle.shown() {
            Cover::Screensaver
        } else {
            Cover::None
        }
    }
}

#[cfg(test)]
mod tests;
