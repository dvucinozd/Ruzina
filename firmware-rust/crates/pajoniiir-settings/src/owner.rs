//! The settings owner: one durable value per record, published to readers
//! only after its write committed (M2.5 "publish RAM after durable NVS").

use crate::{
    BACKLIGHT_MAX_PERCENT, BACKLIGHT_MIN_PERCENT, ProductSettings, SETTINGS_FORMAT_VERSION,
    SETTINGS_PAYLOAD_BYTES, SettingsCodecError,
    network::{
        NETWORK_FORMAT_VERSION, NETWORK_PAYLOAD_BYTES, NetworkConfig, NetworkConfigError,
        PasswordUpdate,
    },
    store::{Loaded, RecordKind, RecordStore, SettingsFlash, StoreError},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordStatus {
    /// Defaults in RAM; nothing stored yet.
    Defaults,
    Stored {
        sequence: u32,
    },
    /// A newer firmware's record: kept byte-for-byte, defaults in RAM, and
    /// ordinary writes refused until an explicit reset or clear.
    NewerPreserved {
        version: u8,
    },
    /// A committed record of this format that fails validation: defaults in
    /// RAM; the next write replaces it.
    Rejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoadReport {
    pub settings: RecordStatus,
    pub network: RecordStatus,
    /// A damaged slot was ignored; the previous commit (if any) is in use.
    pub corrupt_slot: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsError<E> {
    Store(StoreError<E>),
    Invalid(SettingsCodecError),
    Network(NetworkConfigError),
    /// The stored record is newer than this firmware; see [`RecordStatus`].
    NewerRecordPreserved,
    /// `PasswordUpdate::Keep` with no stored passphrase to keep.
    NoPasswordToKeep,
}

impl<E> From<StoreError<E>> for SettingsError<E> {
    fn from(error: StoreError<E>) -> Self {
        Self::Store(error)
    }
}

pub struct SettingsOwner<F> {
    store: RecordStore<F>,
    settings: ProductSettings,
    network: NetworkConfig,
    settings_status: RecordStatus,
    network_status: RecordStatus,
    live_backlight: Option<u8>,
}

impl<F: SettingsFlash> SettingsOwner<F> {
    /// Reads both records. Storage I/O errors are returned; content problems
    /// fall back to defaults and are reported, never repaired silently.
    pub fn load(flash: F) -> Result<(Self, LoadReport), StoreError<F::Error>> {
        let mut store = RecordStore::new(flash);
        let mut buffer = [0u8; NETWORK_PAYLOAD_BYTES];
        let settings_load = store.load(
            RecordKind::Settings,
            SETTINGS_FORMAT_VERSION,
            &mut buffer[..SETTINGS_PAYLOAD_BYTES],
        )?;
        let (settings, settings_status) = match settings_load.loaded {
            Loaded::Missing => (ProductSettings::DEFAULT, RecordStatus::Defaults),
            Loaded::Newer { version, .. } => (
                ProductSettings::DEFAULT,
                RecordStatus::NewerPreserved { version },
            ),
            Loaded::Record { len, sequence, .. } => match ProductSettings::decode(&buffer[..len]) {
                Ok(settings) => (settings, RecordStatus::Stored { sequence }),
                Err(_) => (ProductSettings::DEFAULT, RecordStatus::Rejected),
            },
        };
        let network_load = store.load(RecordKind::Network, NETWORK_FORMAT_VERSION, &mut buffer)?;
        let (network, network_status) = match network_load.loaded {
            Loaded::Missing => (NetworkConfig::NONE, RecordStatus::Defaults),
            Loaded::Newer { version, .. } => (
                NetworkConfig::NONE,
                RecordStatus::NewerPreserved { version },
            ),
            Loaded::Record { len, sequence, .. } => match NetworkConfig::decode(&buffer[..len]) {
                Ok(network) => (network, RecordStatus::Stored { sequence }),
                Err(_) => (NetworkConfig::NONE, RecordStatus::Rejected),
            },
        };
        let report = LoadReport {
            settings: settings_status,
            network: network_status,
            corrupt_slot: settings_load.corrupt_slot || network_load.corrupt_slot,
        };
        Ok((
            Self {
                store,
                settings,
                network,
                settings_status,
                network_status,
                live_backlight: None,
            },
            report,
        ))
    }

    /// The durable settings.
    pub const fn settings(&self) -> ProductSettings {
        self.settings
    }

    /// The backlight to drive now: a slider value not yet persisted wins.
    pub fn backlight_percent(&self) -> u8 {
        self.live_backlight
            .unwrap_or(self.settings.backlight_percent)
    }

    pub const fn network(&self) -> &NetworkConfig {
        &self.network
    }

    pub const fn status(&self) -> (RecordStatus, RecordStatus) {
        (self.settings_status, self.network_status)
    }

    pub fn into_flash(self) -> F {
        self.store.into_flash()
    }

    /// Commits `next` and publishes it. Returns false when nothing changed.
    /// On any error the published value is unchanged.
    pub fn apply(&mut self, next: ProductSettings) -> Result<bool, SettingsError<F::Error>> {
        if matches!(self.settings_status, RecordStatus::NewerPreserved { .. }) {
            return Err(SettingsError::NewerRecordPreserved);
        }
        if next == self.settings && matches!(self.settings_status, RecordStatus::Stored { .. }) {
            return Ok(false);
        }
        self.commit_settings(next)?;
        Ok(true)
    }

    /// Continuous backlight control: applied at once, persisted later by
    /// [`persist_backlight`](Self::persist_backlight) once the slider is
    /// quiet, so a drag is not one flash write per sample.
    pub fn set_backlight_live(&mut self, percent: u8) -> u8 {
        let percent = percent.clamp(BACKLIGHT_MIN_PERCENT, BACKLIGHT_MAX_PERCENT);
        self.live_backlight = Some(percent);
        percent
    }

    /// Persists a pending live backlight. On failure it stays pending.
    pub fn persist_backlight(&mut self) -> Result<bool, SettingsError<F::Error>> {
        let Some(percent) = self.live_backlight else {
            return Ok(false);
        };
        let next = ProductSettings {
            backlight_percent: percent,
            ..self.settings
        };
        let changed = self.apply(next)?;
        self.live_backlight = None;
        Ok(changed)
    }

    /// Explicit return to defaults; the only way to replace a newer record.
    pub fn reset_settings(&mut self) -> Result<(), SettingsError<F::Error>> {
        self.live_backlight = None;
        self.commit_settings(ProductSettings::DEFAULT)
    }

    /// Replaces the service network. `PasswordUpdate::Keep` preserves the
    /// stored passphrase so only the SSID or URL changes.
    pub fn set_network(
        &mut self,
        ssid: &str,
        password: PasswordUpdate<'_>,
        url: &str,
    ) -> Result<(), SettingsError<F::Error>> {
        if matches!(self.network_status, RecordStatus::NewerPreserved { .. }) {
            return Err(SettingsError::NewerRecordPreserved);
        }
        let current = self.network;
        let password = match password {
            PasswordUpdate::Keep if current.has_password() => current.password_for_association(),
            PasswordUpdate::Keep => return Err(SettingsError::NoPasswordToKeep),
            PasswordUpdate::Open => "",
            PasswordUpdate::Set(password) => password,
        };
        let next = NetworkConfig::new(ssid, password, url).map_err(SettingsError::Network)?;
        self.commit_network(next)
    }

    /// CLEAR WI-FI: commits an empty record (a tombstone), so no older
    /// configuration can reappear from the other slot after a power cut.
    pub fn clear_network(&mut self) -> Result<(), SettingsError<F::Error>> {
        self.commit_network(NetworkConfig::NONE)
    }

    pub(crate) fn commit_settings(
        &mut self,
        next: ProductSettings,
    ) -> Result<(), SettingsError<F::Error>> {
        let payload = next.encode().map_err(SettingsError::Invalid)?;
        let sequence = self
            .store
            .write(RecordKind::Settings, SETTINGS_FORMAT_VERSION, &payload)?;
        self.settings = next;
        self.settings_status = RecordStatus::Stored { sequence };
        Ok(())
    }

    pub(crate) fn commit_network(
        &mut self,
        next: NetworkConfig,
    ) -> Result<(), SettingsError<F::Error>> {
        let sequence =
            self.store
                .write(RecordKind::Network, NETWORK_FORMAT_VERSION, &next.encode())?;
        self.network = next;
        self.network_status = RecordStatus::Stored { sequence };
        Ok(())
    }
}

#[cfg(test)]
impl SettingsOwner<crate::store::fake::FakeFlash> {
    /// Power-cut injection: the next writes stop after `bytes`.
    pub(crate) fn store_budget(&mut self, bytes: usize) {
        self.store.flash_mut().budget = Some(bytes);
    }

    pub(crate) fn reboot(self) -> (Self, LoadReport) {
        let mut flash = self.into_flash();
        flash.budget = None;
        Self::load(flash).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::fake::FakeFlash;
    use crate::{JogMode, TimeDisplay};

    fn fresh() -> SettingsOwner<FakeFlash> {
        SettingsOwner::load(FakeFlash::new()).unwrap().0
    }

    fn reboot(owner: SettingsOwner<FakeFlash>) -> (SettingsOwner<FakeFlash>, LoadReport) {
        owner.reboot()
    }

    #[test]
    fn values_publish_only_after_a_durable_commit_and_survive_reboot() {
        let mut owner = fresh();
        assert_eq!(owner.status().0, RecordStatus::Defaults);
        let next = ProductSettings {
            time_display: TimeDisplay::Remaining,
            load_lock: false,
            jog_modes: [JogMode::Cdj, JogMode::Vinyl],
            ..ProductSettings::DEFAULT
        };
        assert_eq!(owner.apply(next), Ok(true));
        assert_eq!(owner.apply(next), Ok(false));

        // A power cut during the next write publishes nothing.
        let attempt = ProductSettings {
            dj_link: true,
            ..next
        };
        owner.store.flash_mut().budget = Some(10);
        assert!(matches!(owner.apply(attempt), Err(SettingsError::Store(_))));
        assert_eq!(owner.settings(), next);

        let (owner, report) = reboot(owner);
        assert_eq!(owner.settings(), next);
        assert!(matches!(report.settings, RecordStatus::Stored { .. }));
        assert!(!report.corrupt_slot);
    }

    #[test]
    fn backlight_applies_live_and_persists_once_settled() {
        let mut owner = fresh();
        for percent in [20, 40, 5, 130, 64] {
            owner.set_backlight_live(percent);
        }
        assert_eq!(owner.backlight_percent(), 64);
        assert_eq!(owner.settings().backlight_percent, 80);
        owner.set_backlight_live(3);
        assert_eq!(owner.backlight_percent(), 10);

        owner.store.flash_mut().budget = Some(0);
        assert!(owner.persist_backlight().is_err());
        assert_eq!(
            owner.backlight_percent(),
            10,
            "still pending after a failure"
        );
        owner.store.flash_mut().budget = None;
        assert_eq!(owner.persist_backlight(), Ok(true));
        assert_eq!(owner.persist_backlight(), Ok(false));
        let (owner, _) = reboot(owner);
        assert_eq!(owner.settings().backlight_percent, 10);
    }

    #[test]
    fn network_keeps_or_clears_the_passphrase_and_a_tombstone_wins() {
        let mut owner = fresh();
        assert_eq!(
            owner.set_network("Deck", PasswordUpdate::Keep, "https://ota.example"),
            Err(SettingsError::NoPasswordToKeep)
        );
        owner
            .set_network(
                "Deck",
                PasswordUpdate::Set("first-pass"),
                "https://ota.example",
            )
            .unwrap();
        owner
            .set_network("Deck 2", PasswordUpdate::Keep, "https://ota.example/m1")
            .unwrap();
        assert_eq!(owner.network().password_for_association(), "first-pass");
        assert_eq!(owner.network().ssid(), "Deck 2");
        assert_eq!(
            owner.set_network("Deck", PasswordUpdate::Set("short"), "https://h"),
            Err(SettingsError::Network(NetworkConfigError::TooShort(
                crate::network::NetworkField::Password
            )))
        );
        assert_eq!(owner.network().ssid(), "Deck 2");

        owner.clear_network().unwrap();
        let (owner, report) = reboot(owner);
        assert!(!owner.network().is_configured());
        assert!(matches!(report.network, RecordStatus::Stored { .. }));
        // Settings were never written.
        assert_eq!(report.settings, RecordStatus::Defaults);
    }

    #[test]
    fn newer_and_invalid_records_are_reported_and_handled_explicitly() {
        let mut store = RecordStore::new(FakeFlash::new());
        store.write(RecordKind::Settings, 2, &[0x42; 24]).unwrap();
        let mut bad = ProductSettings::DEFAULT.encode().unwrap();
        bad[3] = 7;
        store.write(RecordKind::Network, 1, &[0xee; 32]).unwrap();
        let (mut owner, report) = SettingsOwner::load(store.into_flash()).unwrap();
        assert_eq!(report.settings, RecordStatus::NewerPreserved { version: 2 });
        assert_eq!(report.network, RecordStatus::Rejected);
        assert_eq!(owner.settings(), ProductSettings::DEFAULT);
        assert_eq!(
            owner.apply(ProductSettings {
                dj_link: true,
                ..ProductSettings::DEFAULT
            }),
            Err(SettingsError::NewerRecordPreserved)
        );
        owner.set_backlight_live(50);
        assert_eq!(
            owner.persist_backlight(),
            Err(SettingsError::NewerRecordPreserved)
        );
        // A rejected record of this format is simply replaced by the next write.
        owner
            .set_network("Deck", PasswordUpdate::Open, "https://h")
            .unwrap();

        // The newer record survived untouched until the explicit reset.
        let (mut owner, report) = reboot(owner);
        assert_eq!(report.settings, RecordStatus::NewerPreserved { version: 2 });
        owner.reset_settings().unwrap();
        let (owner, report) = reboot(owner);
        assert!(matches!(report.settings, RecordStatus::Stored { .. }));
        assert_eq!(owner.settings(), ProductSettings::DEFAULT);

        // A CRC-valid record with an impossible value is rejected, not clamped.
        let mut store = RecordStore::new(FakeFlash::new());
        store.write(RecordKind::Settings, 1, &bad).unwrap();
        let (_, report) = SettingsOwner::load(store.into_flash()).unwrap();
        assert_eq!(report.settings, RecordStatus::Rejected);
    }
}
