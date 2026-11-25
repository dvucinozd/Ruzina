//! The deck's settings as the Web Remote's SETUP page reads and changes them: the same
//! settings the deck's own Settings page offers, through the same store, and what that
//! page shows about the deck.
//!
//! `GET /api/settings` answers with [`SettingsView`] (`json::write_settings`).
//! `POST /api/settings?name=…&value=…` changes one setting ([`parse_change`]) and answers
//! with the settings as they then are.

use crate::parse::{parse_bounded, parse_u32, query_value};
use ruzina_core::DeckId;

/// A deck's jog: VINYL scratches on the platter top, CDJ bends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Jog {
    Vinyl,
    Cdj,
}

impl Jog {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Vinyl => "vinyl",
            Self::Cdj => "cdj",
        }
    }
}

/// Whether the deck's media can be read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Media {
    /// Mounted, its catalog read.
    Ready,
    /// Being read: mounted, the catalog not yet in.
    Reading,
    /// Pulled out.
    Removed,
}

impl Media {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Reading => "reading",
            Self::Removed => "removed",
        }
    }
}

/// What became of a new password for the deck's own network.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PasswordOutcome {
    /// Taken: the deck saves it and restarts its network with it.
    Accepted,
    /// Not while a deck plays: saving stalls the audio.
    Busy,
    /// Not a WPA2 password (8 to 63 printable characters).
    Invalid,
    /// The deck has no network it can change.
    Unavailable,
}

/// The password a `POST /api/network` body `{"password":"…"}` carries: the field alone,
/// with no escapes, at most 64 bytes. The deck checks the rest.
pub fn parse_password(body: &str) -> Option<&str> {
    crate::parse::json_string(body, "password", 64).filter(|password| !password.is_empty())
}

/// What the deck tells about itself, as its Settings page does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SettingsInfo<'a> {
    /// The deck's own network's name.
    pub network: &'a str,
    /// The firmware's version and build time.
    pub firmware: &'a str,
    pub built: &'a str,
    /// The attached controller's name, or empty for none.
    pub controller: &'a str,
    pub tracks: u32,
    pub media: Media,
    pub uptime_s: u32,
}

/// A controller profile the deck carries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControllerProfile<'a> {
    pub name: &'a str,
    pub vid: u16,
    pub pid: u16,
    /// Tried on this deck's hardware.
    pub tested: bool,
    /// The attached controller's.
    pub active: bool,
}

/// The settings in force and the deck's information.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SettingsView<'a> {
    pub load_lock: bool,
    pub jog: [Jog; 2],
    pub brightness_percent: u8,
    /// The range and step the deck allows for the brightness.
    pub brightness_min: u8,
    pub brightness_max: u8,
    pub brightness_step: u8,
    /// The MAIN trim, in dB below 0: 0, 3 or 6.
    pub trim_db: u8,
    /// The screensaver's wait in minutes: 0 (off), 2, 5 or 10.
    pub screensaver_min: u8,
    pub info: SettingsInfo<'a>,
    /// The controller profiles the deck carries, chosen by VID:PID when one attaches.
    pub profiles: &'a [ControllerProfile<'a>],
}

/// One setting changed from the page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingChange {
    LoadLock(bool),
    Jog(DeckId, Jog),
    /// A brightness in percent; the deck keeps it within its range.
    Brightness(u8),
    TrimDb(u8),
    ScreensaverMin(u8),
}

/// Why a change was refused, as the page's text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeError {
    UnknownSetting,
    Value,
    Deck,
}

impl ChangeError {
    pub const fn text(self) -> &'static str {
        match self {
            Self::UnknownSetting => "Unknown setting",
            Self::Value => "Invalid setting value",
            Self::Deck => "Deck must be 1 or 2",
        }
    }
}

/// The change a `POST /api/settings` query names:
/// - `name=load_lock&value=0|1`;
/// - `name=jog&deck=1|2&value=vinyl|cdj`;
/// - `name=brightness&value=1..100`;
/// - `name=trim&value=0|3|6` (dB below 0);
/// - `name=screensaver&value=0|2|5|10` (minutes, 0 off).
pub fn parse_change(query: &str) -> Result<SettingChange, ChangeError> {
    let name = query_value(query, "name").ok_or(ChangeError::UnknownSetting)?;
    let value = query_value(query, "value").ok_or(ChangeError::Value)?;
    let number = |allowed: &[u32]| {
        parse_u32(value)
            .filter(|parsed| allowed.contains(parsed))
            .map(|parsed| parsed as u8)
            .ok_or(ChangeError::Value)
    };
    match name {
        "load_lock" => Ok(SettingChange::LoadLock(number(&[0, 1])? == 1)),
        "jog" => {
            let deck = match query_value(query, "deck").and_then(|deck| parse_bounded(deck, 1, 2)) {
                Some(1) => DeckId::One,
                Some(_) => DeckId::Two,
                None => return Err(ChangeError::Deck),
            };
            let jog = match value {
                "vinyl" => Jog::Vinyl,
                "cdj" => Jog::Cdj,
                _ => return Err(ChangeError::Value),
            };
            Ok(SettingChange::Jog(deck, jog))
        }
        "brightness" => parse_bounded(value, 1, 100)
            .map(|percent| SettingChange::Brightness(percent as u8))
            .ok_or(ChangeError::Value),
        "trim" => Ok(SettingChange::TrimDb(number(&[0, 3, 6])?)),
        "screensaver" => Ok(SettingChange::ScreensaverMin(number(&[0, 2, 5, 10])?)),
        _ => Err(ChangeError::UnknownSetting),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_setting_parses_from_its_query() {
        assert_eq!(
            parse_change("name=load_lock&value=1"),
            Ok(SettingChange::LoadLock(true))
        );
        assert_eq!(
            parse_change("name=jog&deck=2&value=cdj"),
            Ok(SettingChange::Jog(DeckId::Two, Jog::Cdj))
        );
        assert_eq!(
            parse_change("name=brightness&value=35"),
            Ok(SettingChange::Brightness(35))
        );
        assert_eq!(
            parse_change("name=trim&value=6"),
            Ok(SettingChange::TrimDb(6))
        );
        assert_eq!(
            parse_change("name=screensaver&value=0"),
            Ok(SettingChange::ScreensaverMin(0))
        );
    }

    #[test]
    fn anything_else_is_refused() {
        assert_eq!(
            parse_change("name=wifi&value=1"),
            Err(ChangeError::UnknownSetting)
        );
        assert_eq!(parse_change("value=1"), Err(ChangeError::UnknownSetting));
        assert_eq!(
            parse_change("name=load_lock&value=2"),
            Err(ChangeError::Value)
        );
        assert_eq!(parse_change("name=load_lock"), Err(ChangeError::Value));
        assert_eq!(
            parse_change("name=jog&deck=3&value=cdj"),
            Err(ChangeError::Deck)
        );
        assert_eq!(parse_change("name=jog&value=cdj"), Err(ChangeError::Deck));
        assert_eq!(
            parse_change("name=jog&deck=1&value=CDJ"),
            Err(ChangeError::Value)
        );
        assert_eq!(
            parse_change("name=brightness&value=0"),
            Err(ChangeError::Value)
        );
        assert_eq!(
            parse_change("name=brightness&value=101"),
            Err(ChangeError::Value)
        );
        assert_eq!(
            parse_change("name=brightness&value=035"),
            Err(ChangeError::Value)
        );
        assert_eq!(parse_change("name=trim&value=2"), Err(ChangeError::Value));
        assert_eq!(
            parse_change("name=screensaver&value=3"),
            Err(ChangeError::Value)
        );
    }

    #[test]
    fn a_password_comes_from_its_json_field_only() {
        assert_eq!(
            parse_password("{\"password\":\"club night 42\"}"),
            Some("club night 42")
        );
        assert_eq!(parse_password("{\"password\":\"\"}"), None);
        assert_eq!(parse_password("{\"password\":\"a\\\"b\"}"), None);
        assert_eq!(parse_password("{\"pass\":\"secret123\"}"), None);
        assert_eq!(parse_password("password=secret123"), None);
    }
}
