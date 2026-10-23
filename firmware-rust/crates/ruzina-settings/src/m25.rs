//! Explicit, one-shot import of M2.5 `app_settings` values (RP-11).
//!
//! Rust never reads or reinterprets M2.5 NVS on its own. A caller that has
//! read the donor's `cdjcfg` keys passes them here once; the mapping applies
//! M2.5's own load normalization, so the result is what the M2.5 deck showed:
//!
//! | M2.5 key | Rust setting | Rule |
//! |---|---|---|
//! | `backlight` | backlight | values above 100 become 100 (M2.5), below 10 become 10 (Rust minimum) |
//! | `time_rem` | time display | nonzero is Remaining (M2.5 stores any u8) |
//! | `cue_mode` | cue mode | 0/1, anything else keeps the default (M2.5) |
//! | `master_trim` | master trim | 0..=2, anything else keeps the default (M2.5) |
//! | `wifi_rem`, `dj_link` | toggles | 0/1, anything else keeps the default (M2.5) |
//! | `audio_out` | — | dropped: a JC4880 speaker/RCA route with no Guition meaning |
//! | `ota_cfg_v2` / legacy `ota_*` | network | imported only if every field passes today's rules |
//!
//! LOAD LOCK and VINYL/CDJ have no M2.5 key and keep their defaults. The
//! import runs only while no Rust settings record exists, so it can never
//! override a later choice; the network record is written first and the
//! settings record last, which also marks the import done. An interrupted
//! import simply runs again.
//!
//! Hot Cue and analysis-cache data are not imported here. Any later import
//! must go through `ruzina-media-identity` (full volume/path identity and
//! the legacy v2 helper), rejecting collisions and stale records; donor
//! structs are never reinterpreted as Rust records.

use crate::{
    BACKLIGHT_MAX_PERCENT, BACKLIGHT_MIN_PERCENT, CueMode, MasterTrim, ProductSettings,
    TimeDisplay,
    network::{NetworkConfig, NetworkConfigError},
    owner::{RecordStatus, SettingsError, SettingsOwner},
    store::SettingsFlash,
};

/// Raw values as read from M2.5 NVS; `None` is an absent key.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct M25Settings<'a> {
    pub audio_out: Option<u8>,
    pub backlight: Option<u8>,
    pub time_remain: Option<u8>,
    pub cue_mode: Option<u8>,
    pub master_trim: Option<u8>,
    pub wifi_remote: Option<u8>,
    pub dj_link: Option<u8>,
    pub network: Option<M25Network<'a>>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct M25Network<'a> {
    pub ssid: &'a str,
    pub password: &'a str,
    pub url: &'a str,
}

impl core::fmt::Debug for M25Network<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("M25Network")
            .field("ssid", &self.ssid)
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum M25NetworkImport {
    Absent,
    Imported,
    Rejected(NetworkConfigError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum M25Import {
    /// A Rust settings record already exists; nothing was written.
    AlreadyMigrated,
    Imported {
        network: M25NetworkImport,
        /// `audio_out` was present and deliberately not carried over.
        audio_route_dropped: bool,
    },
}

/// The settings M2.5 itself would have loaded from these values.
pub fn map_m25_settings(legacy: &M25Settings<'_>) -> ProductSettings {
    let defaults = ProductSettings::DEFAULT;
    let flag = |value: Option<u8>, default: bool| match value {
        Some(0) => false,
        Some(1) => true,
        _ => default,
    };
    ProductSettings {
        backlight_percent: legacy
            .backlight
            .map_or(defaults.backlight_percent, |value| {
                value.clamp(BACKLIGHT_MIN_PERCENT, BACKLIGHT_MAX_PERCENT)
            }),
        time_display: match legacy.time_remain {
            Some(0) | None => TimeDisplay::Elapsed,
            Some(_) => TimeDisplay::Remaining,
        },
        cue_mode: match legacy.cue_mode {
            Some(1) => CueMode::SplitMono,
            Some(0) => CueMode::StereoMaster,
            _ => defaults.cue_mode,
        },
        master_trim: match legacy.master_trim {
            Some(0) => MasterTrim::ZeroDb,
            Some(1) => MasterTrim::Minus3Db,
            Some(2) => MasterTrim::Minus6Db,
            _ => defaults.master_trim,
        },
        wifi_remote: flag(legacy.wifi_remote, defaults.wifi_remote),
        dj_link: flag(legacy.dj_link, defaults.dj_link),
        ..defaults
    }
}

impl<F: SettingsFlash> SettingsOwner<F> {
    /// One-shot M2.5 import; see the module documentation for the policy.
    pub fn import_m25(
        &mut self,
        legacy: &M25Settings<'_>,
    ) -> Result<M25Import, SettingsError<F::Error>> {
        match self.status().0 {
            RecordStatus::Defaults | RecordStatus::Rejected => {}
            RecordStatus::Stored { .. } => return Ok(M25Import::AlreadyMigrated),
            RecordStatus::NewerPreserved { .. } => return Err(SettingsError::NewerRecordPreserved),
        }
        let network = match legacy.network {
            None => M25NetworkImport::Absent,
            Some(network) if network.ssid.is_empty() && network.url.is_empty() => {
                M25NetworkImport::Absent
            }
            Some(network) => {
                match NetworkConfig::new(network.ssid, network.password, network.url) {
                    Ok(config) => {
                        self.commit_network(config)?;
                        M25NetworkImport::Imported
                    }
                    Err(error) => M25NetworkImport::Rejected(error),
                }
            }
        };
        self.commit_settings(map_m25_settings(legacy))?;
        Ok(M25Import::Imported {
            network,
            audio_route_dropped: legacy.audio_out.is_some(),
        })
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::store::fake::FakeFlash;
    use crate::{JogMode, network::NetworkField};
    use std::format;

    fn legacy() -> M25Settings<'static> {
        M25Settings {
            audio_out: Some(0),
            backlight: Some(130),
            time_remain: Some(7),
            cue_mode: Some(1),
            master_trim: Some(9),
            wifi_remote: Some(1),
            dj_link: Some(2),
            network: Some(M25Network {
                ssid: "Club",
                password: "club-pass-1",
                url: "https://ota.example/m1",
            }),
        }
    }

    #[test]
    fn mapping_reproduces_m25_load_normalization() {
        let mapped = map_m25_settings(&legacy());
        assert_eq!(mapped.backlight_percent, 100);
        assert_eq!(mapped.time_display, TimeDisplay::Remaining);
        assert_eq!(mapped.cue_mode, CueMode::SplitMono);
        assert_eq!(mapped.master_trim, MasterTrim::ZeroDb);
        assert!(mapped.wifi_remote);
        assert!(!mapped.dj_link);
        assert!(mapped.load_lock);
        assert_eq!(mapped.jog_modes, [JogMode::Vinyl; 2]);
        assert_eq!(
            map_m25_settings(&M25Settings::default()),
            ProductSettings::DEFAULT
        );
        let dim = M25Settings {
            backlight: Some(2),
            ..M25Settings::default()
        };
        assert_eq!(map_m25_settings(&dim).backlight_percent, 10);
        assert!(!format!("{:?}", legacy()).contains("club-pass"));
    }

    #[test]
    fn import_is_one_shot_idempotent_and_survives_interruption() {
        // A cut after the network record (erase + 20 + 259 + 4 = 284 bytes)
        // but before the settings record commits.
        let (mut owner, _) = SettingsOwner::load(FakeFlash::new()).unwrap();
        owner.store_budget(300);
        assert!(owner.import_m25(&legacy()).is_err());
        let (mut owner, report) = owner.reboot();
        assert_eq!(report.settings, RecordStatus::Defaults);
        assert!(owner.network().is_configured());

        // Rerunning completes it with the same result.
        assert_eq!(
            owner.import_m25(&legacy()),
            Ok(M25Import::Imported {
                network: M25NetworkImport::Imported,
                audio_route_dropped: true,
            })
        );
        assert_eq!(owner.settings(), map_m25_settings(&legacy()));
        assert_eq!(owner.network().password_for_association(), "club-pass-1");

        // Later choices are never overridden by another import.
        owner
            .apply(ProductSettings {
                load_lock: false,
                ..owner.settings()
            })
            .unwrap();
        assert_eq!(owner.import_m25(&legacy()), Ok(M25Import::AlreadyMigrated));
        let (owner, _) = owner.reboot();
        assert!(!owner.settings().load_lock);
    }

    #[test]
    fn invalid_legacy_network_is_reported_not_partially_imported() {
        let (mut owner, _) = SettingsOwner::load(FakeFlash::new()).unwrap();
        let weak = M25Settings {
            network: Some(M25Network {
                ssid: "Club",
                password: "short",
                url: "http://ota.example",
            }),
            ..M25Settings::default()
        };
        assert_eq!(
            owner.import_m25(&weak),
            Ok(M25Import::Imported {
                network: M25NetworkImport::Rejected(NetworkConfigError::TooShort(
                    NetworkField::Password
                )),
                audio_route_dropped: false,
            })
        );
        assert!(!owner.network().is_configured());
        assert_eq!(owner.status().1, RecordStatus::Defaults);
    }
}
