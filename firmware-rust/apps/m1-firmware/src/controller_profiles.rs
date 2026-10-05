use core::cell::RefCell;

use embassy_sync::{
    blocking_mutex::{CriticalSectionMutex, raw::CriticalSectionRawMutex},
    mutex::Mutex,
    signal::Signal,
};
use pajoniiir_controller_profile_storage::{
    MAX_PROFILE_SIZE, ProfileActivationState, ProfileActivationTicket, ProfileRegistry, ProfileSlot,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProfileCatalogMatch {
    pub(crate) index: Option<u8>,
    pub(crate) revision: u32,
    pub(crate) activation_state: ProfileActivationState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProfileActivationRequest {
    pub(crate) ticket: ProfileActivationTicket,
    pub(crate) slot: ProfileSlot,
}

struct ProfileCatalog {
    registry: ProfileRegistry,
    revision: u32,
    activation_revision: u32,
}

impl ProfileCatalog {
    const fn new() -> Self {
        Self {
            registry: ProfileRegistry::new(),
            revision: 0,
            activation_revision: 0,
        }
    }

    fn next_revision(value: &mut u32) -> u32 {
        *value = value.wrapping_add(1);
        if *value == 0 {
            *value = 1;
        }
        *value
    }

    fn replace(&mut self, registry: ProfileRegistry) -> u32 {
        self.registry.apply_rescan(&registry);
        Self::next_revision(&mut self.revision)
    }

    fn match_profile(&self, vid: u16, pid: u16) -> ProfileCatalogMatch {
        ProfileCatalogMatch {
            index: self.registry.match_index(vid, pid),
            revision: self.revision,
            activation_state: self.registry.activation_state(),
        }
    }

    fn begin_activation(&mut self, index: u8) -> Option<ProfileActivationRequest> {
        let ticket = self.registry.begin_activation(index).ok()?;
        let slot = self.registry.profile(index)?;
        Some(ProfileActivationRequest { ticket, slot })
    }

    fn bump_activation_revision(&mut self) -> u32 {
        Self::next_revision(&mut self.activation_revision)
    }
}

struct ActiveProfileBlob {
    bytes: [u8; MAX_PROFILE_SIZE],
    len: usize,
    index: Option<u8>,
    controller_epoch: u32,
    revision: u32,
}

impl ActiveProfileBlob {
    const fn new() -> Self {
        Self {
            bytes: [0u8; MAX_PROFILE_SIZE],
            len: 0,
            index: None,
            controller_epoch: 0,
            revision: 0,
        }
    }

    fn clear(&mut self) {
        self.len = 0;
        self.index = None;
        self.controller_epoch = 0;
    }
}

static PROFILE_CATALOG: CriticalSectionMutex<RefCell<ProfileCatalog>> =
    CriticalSectionMutex::new(RefCell::new(ProfileCatalog::new()));
static PROFILE_CATALOG_CHANGED: Signal<CriticalSectionRawMutex, u32> = Signal::new();
static PROFILE_ACTIVATION_REQUEST: Signal<CriticalSectionRawMutex, ProfileActivationRequest> =
    Signal::new();
static PROFILE_ACTIVATION_CHANGED: Signal<CriticalSectionRawMutex, u32> = Signal::new();
static ACTIVE_PROFILE: Mutex<CriticalSectionRawMutex, ActiveProfileBlob> =
    Mutex::new(ActiveProfileBlob::new());

pub(crate) fn publish_scanned_registry(registry: ProfileRegistry) {
    let revision = PROFILE_CATALOG.lock(|cell| cell.borrow_mut().replace(registry));
    PROFILE_CATALOG_CHANGED.signal(revision);
}

pub(crate) fn bind_controller_descriptor(
    vid: u16,
    pid: u16,
    capabilities: u16,
    epoch: u32,
) -> ProfileCatalogMatch {
    PROFILE_CATALOG.lock(|cell| {
        let mut catalog = cell.borrow_mut();
        let _ = catalog.registry.on_descriptor(vid, pid, capabilities, epoch);
        catalog.match_profile(vid, pid)
    })
}

pub(crate) fn on_controller_disconnect() {
    let revision = PROFILE_CATALOG.lock(|cell| {
        let mut catalog = cell.borrow_mut();
        if !catalog.registry.on_disconnect() {
            return None;
        }
        Some(catalog.bump_activation_revision())
    });
    if let Some(revision) = revision {
        PROFILE_ACTIVATION_CHANGED.signal(revision);
    }
}

pub(crate) fn match_scanned_profile(vid: u16, pid: u16) -> ProfileCatalogMatch {
    PROFILE_CATALOG.lock(|cell| cell.borrow().match_profile(vid, pid))
}

pub(crate) fn request_profile_activation(index: u8) -> bool {
    let started = PROFILE_CATALOG.lock(|cell| {
        let mut catalog = cell.borrow_mut();
        let request = catalog.begin_activation(index)?;
        let revision = catalog.bump_activation_revision();
        Some((request, revision))
    });
    let Some((request, revision)) = started else {
        return false;
    };

    PROFILE_ACTIVATION_REQUEST.signal(request);
    PROFILE_ACTIVATION_CHANGED.signal(revision);
    true
}

pub(crate) async fn wait_for_profile_catalog_change() -> u32 {
    PROFILE_CATALOG_CHANGED.wait().await
}

pub(crate) async fn wait_for_profile_activation_request() -> ProfileActivationRequest {
    PROFILE_ACTIVATION_REQUEST.wait().await
}

pub(crate) async fn wait_for_profile_activation_change() -> u32 {
    PROFILE_ACTIVATION_CHANGED.wait().await
}

pub(crate) async fn complete_profile_activation(
    request: ProfileActivationRequest,
    blob: &[u8],
) -> bool {
    if blob.len() > MAX_PROFILE_SIZE {
        fail_profile_activation(request).await;
        return false;
    }

    let prevalidated = PROFILE_CATALOG.lock(|cell| {
        cell.borrow()
            .registry
            .validate_activation_blob(request.ticket, blob)
            .is_ok()
    });
    if !prevalidated {
        return false;
    }

    let mut active = ACTIVE_PROFILE.lock().await;
    active.clear();
    active.bytes[..blob.len()].copy_from_slice(blob);
    active.len = blob.len();

    let committed = PROFILE_CATALOG.lock(|cell| {
        let mut catalog = cell.borrow_mut();
        if catalog
            .registry
            .validate_activation_blob(request.ticket, blob)
            .is_err()
        {
            return None;
        }
        if catalog.registry.commit_activation(request.ticket).is_err() {
            return None;
        }
        Some(catalog.bump_activation_revision())
    });

    let Some(revision) = committed else {
        active.clear();
        return false;
    };

    active.index = Some(request.ticket.index());
    active.controller_epoch = request.ticket.controller().epoch;
    active.revision = revision;
    PROFILE_ACTIVATION_CHANGED.signal(revision);
    true
}

pub(crate) async fn fail_profile_activation(request: ProfileActivationRequest) {
    let mut active = ACTIVE_PROFILE.lock().await;
    active.clear();

    let revision = PROFILE_CATALOG.lock(|cell| {
        let mut catalog = cell.borrow_mut();
        if catalog.registry.fail_activation(request.ticket).is_err() {
            return None;
        }
        Some(catalog.bump_activation_revision())
    });
    if let Some(revision) = revision {
        PROFILE_ACTIVATION_CHANGED.signal(revision);
    }
}
