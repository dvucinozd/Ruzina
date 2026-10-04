use crate::{ParseError, Profile};

pub const MAX_CONTROLLER_PROFILES: usize = 16;
pub const MAX_PROFILE_SIZE: usize = 16_384;
pub const MAX_PROFILE_ID_BYTES: usize = 39;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProfileId {
    bytes: [u8; MAX_PROFILE_ID_BYTES],
    len: u8,
}

impl ProfileId {
    pub fn parse(value: &str) -> Result<Self, ProfileIdError> {
        let bytes = value.as_bytes();
        if bytes.is_empty() || bytes.len() > MAX_PROFILE_ID_BYTES {
            return Err(ProfileIdError::Length);
        }
        if !bytes
            .iter()
            .copied()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(ProfileIdError::Character);
        }

        let mut output = [0u8; MAX_PROFILE_ID_BYTES];
        output[..bytes.len()].copy_from_slice(bytes);
        Ok(Self {
            bytes: output,
            len: bytes.len() as u8,
        })
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..usize::from(self.len)])
            .expect("ProfileId stores validated ASCII only")
    }

    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileIdError {
    Length,
    Character,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProfileMeta {
    pub vid: u16,
    pub pid: u16,
    pub input_count: u16,
    pub output_count: u16,
    pub size: usize,
}

impl ProfileMeta {
    pub fn parse(blob: &[u8]) -> Result<Self, ProfileMetaError> {
        if blob.len() > MAX_PROFILE_SIZE {
            return Err(ProfileMetaError::TooLarge);
        }

        let profile = Profile::parse(blob).map_err(ProfileMetaError::Profile)?;
        Ok(Self {
            vid: profile.vid(),
            pid: profile.pid(),
            input_count: profile.input_count() as u16,
            output_count: profile.output_count() as u16,
            size: blob.len(),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileMetaError {
    TooLarge,
    Profile(ParseError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProfileSlot {
    pub id: ProfileId,
    pub meta: ProfileMeta,
}

impl ProfileSlot {
    pub fn from_blob(id: &str, blob: &[u8]) -> Result<Self, ProfileRegistryError> {
        Ok(Self {
            id: ProfileId::parse(id).map_err(ProfileRegistryError::InvalidId)?,
            meta: ProfileMeta::parse(blob).map_err(ProfileRegistryError::InvalidProfile)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileActivationState {
    Idle,
    Matched,
    Activating,
    Active,
    Failed,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProfileActivationTicket {
    index: u8,
    controller: ConnectedController,
    expected_meta: ProfileMeta,
}

impl ProfileActivationTicket {
    pub const fn index(self) -> u8 {
        self.index
    }

    pub const fn controller(self) -> ConnectedController {
        self.controller
    }

    pub const fn expected_meta(self) -> ProfileMeta {
        self.expected_meta
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConnectedController {
    pub vid: u16,
    pub pid: u16,
    pub capabilities: u16,
    pub epoch: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescriptorApply {
    Matched(u8),
    Active(u8),
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescriptorError {
    StaleOrConflictingEpoch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileActivationError {
    Registry(ProfileRegistryError),
    NoConnectedController,
    NotMatched,
    StaleBinding,
    InvalidProfile(ProfileMetaError),
    MetadataChanged {
        expected: ProfileMeta,
        actual: ProfileMeta,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileRegistryError {
    Full,
    InvalidId(ProfileIdError),
    InvalidProfile(ProfileMetaError),
    InvalidIndex,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProfileRegistry {
    profiles: [Option<ProfileSlot>; MAX_CONTROLLER_PROFILES],
    count: u8,
    matched_index: Option<u8>,
    active_index: Option<u8>,
    activation_state: ProfileActivationState,
    connected: Option<ConnectedController>,
}

impl Default for ProfileRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ProfileRegistry {
    pub const fn new() -> Self {
        Self {
            profiles: [None; MAX_CONTROLLER_PROFILES],
            count: 0,
            matched_index: None,
            active_index: None,
            activation_state: ProfileActivationState::Idle,
            connected: None,
        }
    }

    pub const fn len(&self) -> usize {
        self.count as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub const fn matched_index(&self) -> Option<u8> {
        self.matched_index
    }

    pub const fn active_index(&self) -> Option<u8> {
        self.active_index
    }

    pub const fn activation_state(&self) -> ProfileActivationState {
        self.activation_state
    }

    pub const fn connected(&self) -> Option<ConnectedController> {
        self.connected
    }

    pub fn profile(&self, index: u8) -> Option<ProfileSlot> {
        self.profiles.get(usize::from(index)).copied().flatten()
    }

    pub fn push_blob(&mut self, id: &str, blob: &[u8]) -> Result<u8, ProfileRegistryError> {
        let slot = ProfileSlot::from_blob(id, blob)?;
        self.push_slot(slot)
    }

    pub fn push_slot(&mut self, slot: ProfileSlot) -> Result<u8, ProfileRegistryError> {
        let index = usize::from(self.count);
        if index >= self.profiles.len() {
            return Err(ProfileRegistryError::Full);
        }
        self.profiles[index] = Some(slot);
        self.count = self.count.saturating_add(1);
        Ok(index as u8)
    }

    pub fn match_index(&self, vid: u16, pid: u16) -> Option<u8> {
        for index in 0..usize::from(self.count) {
            let Some(slot) = self.profiles[index] else {
                continue;
            };
            if slot.meta.vid == vid && slot.meta.pid == pid {
                return Some(index as u8);
            }
        }
        None
    }

    pub fn descriptor_is_fresh(&self, vid: u16, pid: u16, epoch: u32) -> bool {
        if epoch == 0 {
            return false;
        }

        let Some(connected) = self.connected else {
            return true;
        };

        if connected.epoch == epoch {
            return connected.vid == vid && connected.pid == pid;
        }

        (epoch.wrapping_sub(connected.epoch) as i32) > 0
    }

    pub fn on_descriptor(
        &mut self,
        vid: u16,
        pid: u16,
        capabilities: u16,
        epoch: u32,
    ) -> Result<DescriptorApply, DescriptorError> {
        if !self.descriptor_is_fresh(vid, pid, epoch) {
            return Err(DescriptorError::StaleOrConflictingEpoch);
        }

        self.connected = Some(ConnectedController {
            vid,
            pid,
            capabilities,
            epoch,
        });

        let matched = self.match_index(vid, pid);
        self.matched_index = matched;

        match matched {
            Some(index)
                if self.active_index == Some(index)
                    && self.activation_state == ProfileActivationState::Active =>
            {
                Ok(DescriptorApply::Active(index))
            }
            Some(index) => {
                self.active_index = None;
                self.activation_state = ProfileActivationState::Matched;
                Ok(DescriptorApply::Matched(index))
            }
            None => {
                self.active_index = None;
                self.activation_state = ProfileActivationState::Unsupported;
                Ok(DescriptorApply::Unsupported)
            }
        }
    }

    pub fn on_disconnect(&mut self) -> bool {
        let was_present = self.connected.is_some();
        self.connected = None;
        self.matched_index = None;
        self.active_index = None;
        self.activation_state = ProfileActivationState::Idle;
        was_present
    }

    pub fn begin_activation(
        &mut self,
        index: u8,
    ) -> Result<ProfileActivationTicket, ProfileActivationError> {
        self.require_index(index)
            .map_err(ProfileActivationError::Registry)?;
        let controller = self
            .connected
            .ok_or(ProfileActivationError::NoConnectedController)?;
        if self.matched_index != Some(index) {
            return Err(ProfileActivationError::NotMatched);
        }

        let slot = self
            .profile(index)
            .ok_or(ProfileActivationError::Registry(
                ProfileRegistryError::InvalidIndex,
            ))?;
        if slot.meta.vid != controller.vid || slot.meta.pid != controller.pid {
            return Err(ProfileActivationError::NotMatched);
        }

        self.active_index = None;
        self.activation_state = ProfileActivationState::Activating;

        Ok(ProfileActivationTicket {
            index,
            controller,
            expected_meta: slot.meta,
        })
    }

    pub fn activation_ticket_is_current(&self, ticket: ProfileActivationTicket) -> bool {
        self.connected == Some(ticket.controller)
            && self.matched_index == Some(ticket.index)
            && self.activation_state == ProfileActivationState::Activating
            && self.profile(ticket.index).map(|slot| slot.meta) == Some(ticket.expected_meta)
    }

    pub fn validate_activation_blob<'a>(
        &self,
        ticket: ProfileActivationTicket,
        blob: &'a [u8],
    ) -> Result<Profile<'a>, ProfileActivationError> {
        if !self.activation_ticket_is_current(ticket) {
            return Err(ProfileActivationError::StaleBinding);
        }

        let actual = ProfileMeta::parse(blob).map_err(ProfileActivationError::InvalidProfile)?;
        if actual != ticket.expected_meta {
            return Err(ProfileActivationError::MetadataChanged {
                expected: ticket.expected_meta,
                actual,
            });
        }

        Profile::parse(blob).map_err(|error| {
            ProfileActivationError::InvalidProfile(ProfileMetaError::Profile(error))
        })
    }

    pub fn commit_activation(
        &mut self,
        ticket: ProfileActivationTicket,
    ) -> Result<(), ProfileActivationError> {
        if !self.activation_ticket_is_current(ticket) {
            return Err(ProfileActivationError::StaleBinding);
        }
        self.active_index = Some(ticket.index);
        self.activation_state = ProfileActivationState::Active;
        Ok(())
    }

    pub fn fail_activation(
        &mut self,
        ticket: ProfileActivationTicket,
    ) -> Result<(), ProfileActivationError> {
        if !self.activation_ticket_is_current(ticket) {
            return Err(ProfileActivationError::StaleBinding);
        }
        self.active_index = None;
        self.activation_state = ProfileActivationState::Failed;
        Ok(())
    }

    pub fn mark_activation_started(&mut self, index: u8) -> Result<(), ProfileRegistryError> {
        self.require_index(index)?;
        self.matched_index = Some(index);
        self.active_index = None;
        self.activation_state = ProfileActivationState::Activating;
        Ok(())
    }

    pub fn mark_activation_active(&mut self, index: u8) -> Result<(), ProfileRegistryError> {
        self.require_index(index)?;
        self.matched_index = Some(index);
        self.active_index = Some(index);
        self.activation_state = ProfileActivationState::Active;
        Ok(())
    }

    pub fn mark_activation_failed(&mut self, index: u8) -> Result<(), ProfileRegistryError> {
        self.require_index(index)?;
        self.matched_index = Some(index);
        self.active_index = None;
        self.activation_state = ProfileActivationState::Failed;
        Ok(())
    }

    pub fn apply_rescan(&mut self, scanned: &Self) {
        let connected = self.connected;

        self.profiles = scanned.profiles;
        self.count = scanned.count;
        self.matched_index = None;
        self.active_index = None;
        self.activation_state = ProfileActivationState::Idle;
        self.connected = connected;

        if let Some(connected) = connected {
            self.matched_index = self.match_index(connected.vid, connected.pid);
            self.activation_state = if self.matched_index.is_some() {
                ProfileActivationState::Matched
            } else {
                ProfileActivationState::Unsupported
            };
        }
    }

    fn require_index(&self, index: u8) -> Result<(), ProfileRegistryError> {
        if usize::from(index) >= usize::from(self.count)
            || self.profiles[usize::from(index)].is_none()
        {
            Err(ProfileRegistryError::InvalidIndex)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HEADER_SIZE, INPUT_ENTRY_SIZE, OUTPUT_ENTRY_SIZE, crc32};

    fn profile_blob(
        vid: u16,
        pid: u16,
    ) -> [u8; HEADER_SIZE + INPUT_ENTRY_SIZE + OUTPUT_ENTRY_SIZE] {
        let mut bytes = [0u8; HEADER_SIZE + INPUT_ENTRY_SIZE + OUTPUT_ENTRY_SIZE];
        bytes[0..4].copy_from_slice(b"S3CP");
        bytes[4..6].copy_from_slice(&2u16.to_le_bytes());
        bytes[6..8].copy_from_slice(&(HEADER_SIZE as u16).to_le_bytes());
        let profile_size = bytes.len() as u32;
        bytes[8..12].copy_from_slice(&profile_size.to_le_bytes());
        bytes[16..18].copy_from_slice(&vid.to_le_bytes());
        bytes[18..20].copy_from_slice(&pid.to_le_bytes());
        bytes[24..26].copy_from_slice(&1u16.to_le_bytes());
        bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
        bytes[29] = 2;

        let input = HEADER_SIZE;
        bytes[input] = 0x90;
        bytes[input + 1] = 0x0b;
        bytes[input + 2] = 0;
        bytes[input + 3] = 0xff;
        bytes[input + 4] = 1;
        bytes[input + 5] = 0x10;

        let output = HEADER_SIZE + INPUT_ENTRY_SIZE;
        bytes[output] = 1;
        bytes[output + 1] = 0;
        bytes[output + 2] = 0;
        bytes[output + 3] = 0x90;
        bytes[output + 4] = 0x0b;
        bytes[output + 6] = 0x7f;
        bytes[output + 7] = 0x7f;

        let checksum = crc32(&bytes[16..]);
        bytes[12..16].copy_from_slice(&checksum.to_le_bytes());
        bytes
    }

    #[test]
    fn profile_id_matches_released_directory_policy() {
        assert_eq!(
            ProfileId::parse("pioneer_ddj_flx4").unwrap().as_str(),
            "pioneer_ddj_flx4"
        );
        assert_eq!(
            ProfileId::parse("Hercules-500").unwrap().as_str(),
            "Hercules-500"
        );
        assert_eq!(
            ProfileId::parse("../escape"),
            Err(ProfileIdError::Character)
        );
        assert_eq!(ProfileId::parse(""), Err(ProfileIdError::Length));
        assert_eq!(
            ProfileId::parse("1234567890123456789012345678901234567890"),
            Err(ProfileIdError::Length)
        );
    }

    #[test]
    fn registry_matches_exact_vid_pid_and_tracks_activation() {
        let flx4 = profile_blob(0x2b73, 0x0045);
        let hercules = profile_blob(0x06f8, 0xe020);
        let mut registry = ProfileRegistry::new();

        let flx4_index = registry.push_blob("pioneer_ddj_flx4", &flx4).unwrap();
        let hercules_index = registry
            .push_blob("hercules_djcontrol_inpulse_500", &hercules)
            .unwrap();

        assert_eq!(flx4_index, 0);
        assert_eq!(hercules_index, 1);
        assert_eq!(
            registry.on_descriptor(0x2b73, 0x0045, 0x0003, 1),
            Ok(DescriptorApply::Matched(flx4_index))
        );
        assert_eq!(registry.matched_index(), Some(flx4_index));
        assert_eq!(registry.active_index(), None);
        assert_eq!(registry.activation_state(), ProfileActivationState::Matched);

        registry.mark_activation_started(flx4_index).unwrap();
        assert_eq!(
            registry.activation_state(),
            ProfileActivationState::Activating
        );
        registry.mark_activation_active(flx4_index).unwrap();
        assert_eq!(registry.active_index(), Some(flx4_index));
        assert_eq!(
            registry.on_descriptor(0x2b73, 0x0045, 0x0003, 1),
            Ok(DescriptorApply::Active(flx4_index))
        );
    }

    #[test]
    fn unsupported_controller_is_explicit_and_disconnect_preserves_inventory() {
        let flx4 = profile_blob(0x2b73, 0x0045);
        let mut registry = ProfileRegistry::new();
        registry.push_blob("pioneer_ddj_flx4", &flx4).unwrap();

        assert_eq!(
            registry.on_descriptor(0xffff, 0x0001, 0, 10),
            Ok(DescriptorApply::Unsupported)
        );
        assert_eq!(
            registry.activation_state(),
            ProfileActivationState::Unsupported
        );
        assert!(registry.on_disconnect());
        assert!(!registry.on_disconnect());
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.activation_state(), ProfileActivationState::Idle);
    }

    #[test]
    fn stale_or_conflicting_descriptor_cannot_replace_live_binding() {
        let flx4 = profile_blob(0x2b73, 0x0045);
        let mut registry = ProfileRegistry::new();
        registry.push_blob("pioneer_ddj_flx4", &flx4).unwrap();

        registry.on_descriptor(0x2b73, 0x0045, 1, 100).unwrap();
        assert_eq!(
            registry.on_descriptor(0x2b73, 0x0046, 1, 100),
            Err(DescriptorError::StaleOrConflictingEpoch)
        );
        assert_eq!(
            registry.on_descriptor(0x2b73, 0x0045, 1, 99),
            Err(DescriptorError::StaleOrConflictingEpoch)
        );
        assert_eq!(registry.connected().unwrap().epoch, 100);
    }

    #[test]
    fn serial_epoch_wrap_is_treated_as_fresh() {
        let flx4 = profile_blob(0x2b73, 0x0045);
        let mut registry = ProfileRegistry::new();
        registry.push_blob("pioneer_ddj_flx4", &flx4).unwrap();

        registry.on_descriptor(0x2b73, 0x0045, 1, u32::MAX).unwrap();
        assert!(registry.descriptor_is_fresh(0x2b73, 0x0045, 1));
        assert_eq!(
            registry.on_descriptor(0x2b73, 0x0045, 1, 1),
            Ok(DescriptorApply::Matched(0))
        );
    }

    #[test]
    fn rescan_preserves_descriptor_but_requires_reactivation() {
        let flx4 = profile_blob(0x2b73, 0x0045);
        let hercules = profile_blob(0x06f8, 0xe020);

        let mut registry = ProfileRegistry::new();
        registry.push_blob("pioneer_ddj_flx4", &flx4).unwrap();
        registry.on_descriptor(0x2b73, 0x0045, 3, 7).unwrap();
        registry.mark_activation_active(0).unwrap();

        let mut scanned = ProfileRegistry::new();
        scanned
            .push_blob("hercules_djcontrol_inpulse_500", &hercules)
            .unwrap();
        scanned.push_blob("pioneer_ddj_flx4", &flx4).unwrap();

        registry.apply_rescan(&scanned);

        assert_eq!(registry.connected().unwrap().epoch, 7);
        assert_eq!(registry.matched_index(), Some(1));
        assert_eq!(registry.active_index(), None);
        assert_eq!(registry.activation_state(), ProfileActivationState::Matched);
    }

    #[test]
    fn activation_ticket_revalidates_blob_before_commit() {
        let flx4 = profile_blob(0x2b73, 0x0045);
        let hercules = profile_blob(0x06f8, 0xe020);
        let mut registry = ProfileRegistry::new();
        registry.push_blob("pioneer_ddj_flx4", &flx4).unwrap();
        registry.on_descriptor(0x2b73, 0x0045, 3, 17).unwrap();

        let ticket = registry.begin_activation(0).unwrap();
        assert_eq!(ticket.controller().epoch, 17);
        assert_eq!(ticket.expected_meta().vid, 0x2b73);
        assert_eq!(
            registry
                .validate_activation_blob(ticket, &hercules)
                .unwrap_err(),
            ProfileActivationError::MetadataChanged {
                expected: ProfileMeta::parse(&flx4).unwrap(),
                actual: ProfileMeta::parse(&hercules).unwrap(),
            }
        );

        let profile = registry.validate_activation_blob(ticket, &flx4).unwrap();
        assert_eq!(profile.vid(), 0x2b73);
        assert_eq!(profile.pid(), 0x0045);
        registry.commit_activation(ticket).unwrap();
        assert_eq!(registry.active_index(), Some(0));
        assert_eq!(registry.activation_state(), ProfileActivationState::Active);
    }

    #[test]
    fn activation_ticket_rejects_disconnect_reconnect_during_storage_read() {
        let flx4 = profile_blob(0x2b73, 0x0045);
        let mut registry = ProfileRegistry::new();
        registry.push_blob("pioneer_ddj_flx4", &flx4).unwrap();
        registry.on_descriptor(0x2b73, 0x0045, 3, 20).unwrap();

        let ticket = registry.begin_activation(0).unwrap();
        assert!(registry.on_disconnect());
        registry.on_descriptor(0x2b73, 0x0045, 3, 21).unwrap();

        assert_eq!(
            registry
                .validate_activation_blob(ticket, &flx4)
                .unwrap_err(),
            ProfileActivationError::StaleBinding
        );
        assert_eq!(
            registry.commit_activation(ticket),
            Err(ProfileActivationError::StaleBinding)
        );
        assert_eq!(registry.active_index(), None);
    }

    #[test]
    fn activation_ticket_rejects_registry_rescan_during_storage_read() {
        let flx4 = profile_blob(0x2b73, 0x0045);
        let mut registry = ProfileRegistry::new();
        registry.push_blob("pioneer_ddj_flx4", &flx4).unwrap();
        registry.on_descriptor(0x2b73, 0x0045, 3, 30).unwrap();

        let ticket = registry.begin_activation(0).unwrap();

        let mut scanned = ProfileRegistry::new();
        scanned.push_blob("pioneer_ddj_flx4", &flx4).unwrap();
        registry.apply_rescan(&scanned);

        assert_eq!(registry.activation_state(), ProfileActivationState::Matched);
        assert_eq!(
            registry
                .validate_activation_blob(ticket, &flx4)
                .unwrap_err(),
            ProfileActivationError::StaleBinding
        );
    }

    #[test]
    fn activation_failure_is_committed_only_for_current_ticket() {
        let flx4 = profile_blob(0x2b73, 0x0045);
        let mut registry = ProfileRegistry::new();
        registry.push_blob("pioneer_ddj_flx4", &flx4).unwrap();
        registry.on_descriptor(0x2b73, 0x0045, 3, 40).unwrap();

        let ticket = registry.begin_activation(0).unwrap();
        registry.fail_activation(ticket).unwrap();
        assert_eq!(registry.active_index(), None);
        assert_eq!(registry.activation_state(), ProfileActivationState::Failed);
    }

    #[test]
    fn profile_meta_enforces_released_size_ceiling() {
        let oversized = [0u8; MAX_PROFILE_SIZE + 1];
        assert_eq!(
            ProfileMeta::parse(&oversized),
            Err(ProfileMetaError::TooLarge)
        );
    }
}
