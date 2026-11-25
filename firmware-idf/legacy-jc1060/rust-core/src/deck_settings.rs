//! The deck's own settings, kept on flash (RP-11): LOAD LOCK, each deck's jog as VINYL
//! or CDJ, the screen's brightness, the MAIN trim and the screensaver's wait.
//!
//! - **Boot:** the record is read before the screen and USB audio start. The sector the
//!   saves move on to, once theirs is full, is erased then: an erase stalls the USB
//!   audio fill and the screen for tens of milliseconds.
//! - **Change:** the Settings page changes the value here at once. The USB client task
//!   applies it to the engine and the mix on its next turn, and the screen's brightness
//!   follows at once.
//! - **Save:** the app main task saves a change, in `ruzina_legacy_settings_service`
//!   (`idf_media_bridge.rs`), which `main/probe.c` calls from its heartbeat loop. It saves
//!   once the change has been quiet for [`SAVE_QUIET_MS`] and no deck plays, as Hot Cue
//!   saves wait: flash stalls everything running from flash and PSRAM, the USB audio fill
//!   included. A save usually appends a record after the last one and erases nothing. A
//!   save that has to erase a sector (once saves have filled one), and erasing the next
//!   spare ahead, first wait until USB audio has stood still, then erase.
//!
//! Settings this board does not offer (time display, cue mode, DJ Link) keep what the
//! record holds.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use ruzina_settings::{
    BACKLIGHT_MIN_PERCENT, CueMode, JogMode, MasterTrim, ProductSettings, Screensaver, TimeDisplay,
};

/// The board's out-of-the-box settings: the shared defaults, with the screen as bright
/// as it has always started (25 %) and the deck's own Wi-Fi on.
pub(crate) const BOARD_DEFAULT: ProductSettings = ProductSettings {
    backlight_percent: 25,
    wifi_remote: true,
    ..ProductSettings::DEFAULT
};

/// The brightest the Settings page goes. The board still runs from the PC's USB port,
/// and a saved brightness its supply cannot hold would come back at every boot.
pub(crate) const BACKLIGHT_MAX_PERCENT: u8 = 50;
/// One step of the brightness buttons.
#[cfg(any(test, feature = "slint-ui"))]
pub(crate) const BACKLIGHT_STEP_PERCENT: u8 = 5;
/// How long a change must stay quiet before it is saved, so that stepping the
/// brightness is one save, not one per tap.
pub(crate) const SAVE_QUIET_MS: u32 = 2_000;

/// The settings in force, packed (see [`pack`]).
static CURRENT: AtomicU32 = AtomicU32::new(pack(&BOARD_DEFAULT));
/// A change waits to be saved, and when it was made (milliseconds since boot).
static OWED: AtomicBool = AtomicBool::new(false);
static CHANGED_MS: AtomicU32 = AtomicU32::new(0);

/// The settings in force.
pub(crate) fn current() -> ProductSettings {
    unpack(CURRENT.load(Ordering::Acquire))
}

/// The settings in force, packed: a cheap value to compare with what was applied.
pub(crate) fn current_packed() -> u32 {
    CURRENT.load(Ordering::Acquire)
}

/// Publishes the settings read at boot; nothing is owed.
pub(crate) fn publish_loaded(settings: &ProductSettings) {
    CURRENT.store(pack(&sanitized(*settings)), Ordering::Release);
}

/// Changes the settings in force, from the Settings page (the UI task) or the Web Remote's
/// SETUP page (the web task), and owes a save. Returns the new settings. Two writers: the
/// edit is applied to the settings as they are when it lands, so neither undoes the
/// other's change.
#[cfg(any(test, feature = "slint-ui"))]
pub(crate) fn change(now_ms: u32, edit: impl Fn(&mut ProductSettings)) -> ProductSettings {
    let mut before = CURRENT.load(Ordering::Acquire);
    loop {
        let mut settings = unpack(before);
        edit(&mut settings);
        let settings = sanitized(settings);
        let packed = pack(&settings);
        if packed == before {
            return settings;
        }
        match CURRENT.compare_exchange(before, packed, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => {
                CHANGED_MS.store(now_ms, Ordering::Relaxed);
                OWED.store(true, Ordering::Release);
                return settings;
            }
            Err(now) => before = now,
        }
    }
}

/// The settings to save now, once the last change has been quiet long enough. Taking
/// them clears the debt; a change after this owes a new save.
pub(crate) fn take_owed(now_ms: u32) -> Option<ProductSettings> {
    if !OWED.load(Ordering::Acquire)
        || now_ms.wrapping_sub(CHANGED_MS.load(Ordering::Relaxed)) < SAVE_QUIET_MS
    {
        return None;
    }
    OWED.swap(false, Ordering::AcqRel).then(current)
}

/// A save taken by [`take_owed`] had to wait: the settings in force are owed again,
/// and the next turn may save them at once.
pub(crate) fn owe_again() {
    OWED.store(true, Ordering::Release);
}

/// The brightness one step up or down, within the page's range.
#[cfg(any(test, feature = "slint-ui"))]
pub(crate) fn step_backlight(percent: u8, up: bool) -> u8 {
    let next = if up {
        percent.saturating_add(BACKLIGHT_STEP_PERCENT)
    } else {
        percent.saturating_sub(BACKLIGHT_STEP_PERCENT)
    };
    next.clamp(BACKLIGHT_MIN_PERCENT, BACKLIGHT_MAX_PERCENT)
}

/// The MAIN gain for a trim.
pub(crate) fn trim_gain(trim: MasterTrim) -> f32 {
    match trim {
        MasterTrim::ZeroDb => 1.0,
        MasterTrim::Minus3Db => 0.707_945_8,
        MasterTrim::Minus6Db => 0.501_187_2,
    }
}

/// A stored brightness above this board's range comes back within it.
fn sanitized(mut settings: ProductSettings) -> ProductSettings {
    settings.backlight_percent = settings
        .backlight_percent
        .clamp(BACKLIGHT_MIN_PERCENT, BACKLIGHT_MAX_PERCENT);
    settings
}

/// Bits 0-6 the brightness; then one bit each for the time display, the cue mode, two
/// for the trim, then Wi-Fi, DJ Link, LOAD LOCK and each deck's CDJ jog, then two for the
/// screensaver.
const fn pack(settings: &ProductSettings) -> u32 {
    let trim = match settings.master_trim {
        MasterTrim::ZeroDb => 0,
        MasterTrim::Minus3Db => 1,
        MasterTrim::Minus6Db => 2,
    };
    (settings.backlight_percent as u32 & 0x7f)
        | (matches!(settings.time_display, TimeDisplay::Remaining) as u32) << 7
        | (matches!(settings.cue_mode, CueMode::SplitMono) as u32) << 8
        | trim << 9
        | (settings.wifi_remote as u32) << 11
        | (settings.dj_link as u32) << 12
        | (settings.load_lock as u32) << 13
        | (matches!(settings.jog_modes[0], JogMode::Cdj) as u32) << 14
        | (matches!(settings.jog_modes[1], JogMode::Cdj) as u32) << 15
        | (match settings.screensaver {
            Screensaver::Minutes2 => 0,
            Screensaver::Off => 1,
            Screensaver::Minutes5 => 2,
            Screensaver::Minutes10 => 3,
        }) << 16
}

fn unpack(packed: u32) -> ProductSettings {
    let bit = |n: u32| packed >> n & 1 != 0;
    let jog = |n: u32| if bit(n) { JogMode::Cdj } else { JogMode::Vinyl };
    ProductSettings {
        backlight_percent: (packed & 0x7f) as u8,
        time_display: if bit(7) {
            TimeDisplay::Remaining
        } else {
            TimeDisplay::Elapsed
        },
        cue_mode: if bit(8) {
            CueMode::SplitMono
        } else {
            CueMode::StereoMaster
        },
        master_trim: match packed >> 9 & 3 {
            1 => MasterTrim::Minus3Db,
            2 => MasterTrim::Minus6Db,
            _ => MasterTrim::ZeroDb,
        },
        wifi_remote: bit(11),
        dj_link: bit(12),
        load_lock: bit(13),
        jog_modes: [jog(14), jog(15)],
        screensaver: match packed >> 16 & 3 {
            1 => Screensaver::Off,
            2 => Screensaver::Minutes5,
            3 => Screensaver::Minutes10,
            _ => Screensaver::Minutes2,
        },
    }
}

#[cfg(test)]
pub(crate) fn reset() {
    CURRENT.store(pack(&BOARD_DEFAULT), Ordering::Release);
    OWED.store(false, Ordering::Release);
    CHANGED_MS.store(0, Ordering::Release);
}

#[cfg(test)]
mod tests;
