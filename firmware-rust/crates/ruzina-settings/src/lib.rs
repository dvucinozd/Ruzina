//! Versioned durable product settings (RP-11, M2.5 `app_settings`).
//!
//! Settings and the service-network configuration are separate records, each
//! kept in its own pair of flash sectors and replaced atomically (see
//! [`store`]). The [`owner`] publishes a RAM value only after its record is
//! durably committed. Records are Rust formats: M2.5 NVS values enter only
//! through the explicit, one-shot [`m25`] import, never by reinterpretation.
#![no_std]
#![forbid(unsafe_code)]

pub mod m25;
pub mod network;
pub mod owner;
pub mod store;

/// Current settings record format. A newer stored format is preserved, not
/// reinterpreted (see [`owner::LoadReport`]).
pub const SETTINGS_FORMAT_VERSION: u8 = 1;
pub const SETTINGS_PAYLOAD_BYTES: usize = 16;
pub const BACKLIGHT_MIN_PERCENT: u8 = 10;
pub const BACKLIGHT_MAX_PERCENT: u8 = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeDisplay {
    Elapsed,
    Remaining,
}

/// M2.5 `cue_mode`: headphone cue on its own output or split mono (left
/// master, right cue) on one stereo pair.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CueMode {
    StereoMaster,
    SplitMono,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MasterTrim {
    ZeroDb,
    Minus3Db,
    Minus6Db,
}

/// Per-deck platter behavior (RP-03).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JogMode {
    Vinyl,
    Cdj,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProductSettings {
    pub backlight_percent: u8,
    pub time_display: TimeDisplay,
    pub cue_mode: CueMode,
    pub master_trim: MasterTrim,
    pub wifi_remote: bool,
    pub dj_link: bool,
    /// RP-04 LOAD LOCK; on by default.
    pub load_lock: bool,
    pub jog_modes: [JogMode; 2],
}

impl ProductSettings {
    /// Out-of-the-box values, matching M2.5 where it has the setting.
    pub const DEFAULT: Self = Self {
        backlight_percent: 80,
        time_display: TimeDisplay::Elapsed,
        cue_mode: CueMode::StereoMaster,
        master_trim: MasterTrim::ZeroDb,
        wifi_remote: false,
        dj_link: false,
        load_lock: true,
        jog_modes: [JogMode::Vinyl; 2],
    };

    pub fn validate(&self) -> Result<(), SettingsCodecError> {
        if !(BACKLIGHT_MIN_PERCENT..=BACKLIGHT_MAX_PERCENT).contains(&self.backlight_percent) {
            return Err(SettingsCodecError::OutOfRange);
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<[u8; SETTINGS_PAYLOAD_BYTES], SettingsCodecError> {
        self.validate()?;
        let mut out = [0u8; SETTINGS_PAYLOAD_BYTES];
        out[0] = self.backlight_percent;
        out[1] = match self.time_display {
            TimeDisplay::Elapsed => 0,
            TimeDisplay::Remaining => 1,
        };
        out[2] = match self.cue_mode {
            CueMode::StereoMaster => 0,
            CueMode::SplitMono => 1,
        };
        out[3] = match self.master_trim {
            MasterTrim::ZeroDb => 0,
            MasterTrim::Minus3Db => 1,
            MasterTrim::Minus6Db => 2,
        };
        out[4] = u8::from(self.wifi_remote);
        out[5] = u8::from(self.dj_link);
        out[6] = u8::from(self.load_lock);
        for (index, mode) in self.jog_modes.iter().enumerate() {
            out[7 + index] = match mode {
                JogMode::Vinyl => 0,
                JogMode::Cdj => 1,
            };
        }
        Ok(out)
    }

    /// Strict: any unknown value or nonzero reserved byte rejects the whole
    /// record rather than being clamped into something the user never chose.
    pub fn decode(payload: &[u8]) -> Result<Self, SettingsCodecError> {
        let payload: &[u8; SETTINGS_PAYLOAD_BYTES] =
            payload.try_into().map_err(|_| SettingsCodecError::Length)?;
        if payload[9..].iter().any(|byte| *byte != 0) {
            return Err(SettingsCodecError::Reserved);
        }
        let flag = |byte: u8| match byte {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(SettingsCodecError::OutOfRange),
        };
        let jog = |byte: u8| match byte {
            0 => Ok(JogMode::Vinyl),
            1 => Ok(JogMode::Cdj),
            _ => Err(SettingsCodecError::OutOfRange),
        };
        let settings = Self {
            backlight_percent: payload[0],
            time_display: if flag(payload[1])? {
                TimeDisplay::Remaining
            } else {
                TimeDisplay::Elapsed
            },
            cue_mode: if flag(payload[2])? {
                CueMode::SplitMono
            } else {
                CueMode::StereoMaster
            },
            master_trim: match payload[3] {
                0 => MasterTrim::ZeroDb,
                1 => MasterTrim::Minus3Db,
                2 => MasterTrim::Minus6Db,
                _ => return Err(SettingsCodecError::OutOfRange),
            },
            wifi_remote: flag(payload[4])?,
            dj_link: flag(payload[5])?,
            load_lock: flag(payload[6])?,
            jog_modes: [jog(payload[7])?, jog(payload[8])?],
        };
        settings.validate()?;
        Ok(settings)
    }
}

impl Default for ProductSettings {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsCodecError {
    Length,
    OutOfRange,
    Reserved,
}

pub(crate) fn crc32(parts: &[&[u8]]) -> u32 {
    let mut crc = u32::MAX;
    for part in parts {
        for byte in *part {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
            }
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_reject_unknown_values() {
        let settings = ProductSettings {
            backlight_percent: 35,
            time_display: TimeDisplay::Remaining,
            cue_mode: CueMode::SplitMono,
            master_trim: MasterTrim::Minus6Db,
            wifi_remote: true,
            dj_link: false,
            load_lock: false,
            jog_modes: [JogMode::Cdj, JogMode::Vinyl],
        };
        let bytes = settings.encode().unwrap();
        assert_eq!(ProductSettings::decode(&bytes), Ok(settings));
        assert_eq!(
            ProductSettings::decode(&ProductSettings::DEFAULT.encode().unwrap()),
            Ok(ProductSettings::DEFAULT)
        );

        for (at, value, error) in [
            (0, 9, SettingsCodecError::OutOfRange),
            (0, 101, SettingsCodecError::OutOfRange),
            (1, 2, SettingsCodecError::OutOfRange),
            (3, 3, SettingsCodecError::OutOfRange),
            (6, 0xff, SettingsCodecError::OutOfRange),
            (8, 2, SettingsCodecError::OutOfRange),
            (9, 1, SettingsCodecError::Reserved),
            (15, 1, SettingsCodecError::Reserved),
        ] {
            let mut bad = bytes;
            bad[at] = value;
            assert_eq!(ProductSettings::decode(&bad), Err(error), "byte {at}");
        }
        assert_eq!(
            ProductSettings::decode(&bytes[..15]),
            Err(SettingsCodecError::Length)
        );
        let dim = ProductSettings {
            backlight_percent: 5,
            ..settings
        };
        assert_eq!(dim.encode(), Err(SettingsCodecError::OutOfRange));
    }
}
