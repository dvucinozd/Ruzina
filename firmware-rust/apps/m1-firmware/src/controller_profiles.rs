use core::cell::RefCell;

use embassy_sync::{
    blocking_mutex::{CriticalSectionMutex, raw::CriticalSectionRawMutex},
    mutex::Mutex,
    signal::Signal,
};
use pajoniiir_controller_profile_storage::{
    MAX_INIT_SYSEX, MAX_PROFILE_SIZE, Profile, ProfileActivationState, ProfileActivationTicket,
    ProfileRegistry, ProfileSlot,
};
use pajoniiir_controller_session::{
    ControllerSession, LedUpdate, SessionDeliverySink, SessionDispatchOutcome, UsbMidiBatchError,
    UsbMidiBatchReport,
};
use pajoniiir_controller_usb::UsbMidiEvent;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActiveProfileMidiError {
    NotActive,
    StaleController,
    InvalidActiveProfile,
    Batch(UsbMidiBatchError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActiveProfileLedError {
    NotActive,
    StaleController,
    InvalidActiveProfile,
    InvalidMidiOutput,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProfileInitializationError {
    NotInitializing,
    StaleController,
    InvalidActiveProfile,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ProductDispatchReport {
    pub(crate) accepted: u32,
    pub(crate) backpressured: bool,
    pub(crate) idle: bool,
    pub(crate) budget_exhausted: bool,
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
    ticket: Option<ProfileActivationTicket>,
}

impl ActiveProfileBlob {
    const fn new() -> Self {
        Self {
            bytes: [0u8; MAX_PROFILE_SIZE],
            len: 0,
            index: None,
            controller_epoch: 0,
            revision: 0,
            ticket: None,
        }
    }

    fn clear(&mut self) {
        self.len = 0;
        self.index = None;
        self.controller_epoch = 0;
        self.ticket = None;
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
        fail_profile_activation(request).await;
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
    active.ticket = Some(request.ticket);
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

/// Copy the published profile's initial SysEx for the controller binding that
/// is still waiting for it. MIDI input and LED output stay closed meanwhile.
pub(crate) async fn pending_profile_initialization(
    controller_epoch: u32,
    output: &mut [u8; MAX_INIT_SYSEX],
) -> Result<(ProfileActivationTicket, usize), ProfileInitializationError> {
    let active = ACTIVE_PROFILE.lock().await;
    let ticket = active
        .ticket
        .ok_or(ProfileInitializationError::NotInitializing)?;
    if !profile_initialization_is_current(ticket, controller_epoch)
        || active.controller_epoch != controller_epoch
        || active.index != Some(ticket.index())
    {
        return Err(ProfileInitializationError::StaleController);
    }

    let profile = Profile::parse(&active.bytes[..active.len])
        .map_err(|_| ProfileInitializationError::InvalidActiveProfile)?;
    let sysex = profile.init_sysex();
    output[..sysex.len()].copy_from_slice(sysex);
    Ok((ticket, sysex.len()))
}

pub(crate) fn profile_initialization_is_current(
    ticket: ProfileActivationTicket,
    controller_epoch: u32,
) -> bool {
    ticket.controller().epoch == controller_epoch
        && PROFILE_CATALOG.lock(|cell| {
            cell.borrow()
                .registry
                .initialization_ticket_is_current(ticket)
        })
}

/// Open MIDI/LED activation after every SysEx packet was accepted by USB
/// output, or close the activation as FAILED. Successful enqueue does not
/// prove that the controller accepted the command.
pub(crate) async fn finish_profile_initialization(
    ticket: ProfileActivationTicket,
    enqueued: bool,
) -> bool {
    // Same lock order as activation commit: blob owner, then catalog. A stale
    // ticket must not clear or reopen a newer activation's published blob.
    let mut active = ACTIVE_PROFILE.lock().await;
    let revision = PROFILE_CATALOG.lock(|cell| {
        let mut catalog = cell.borrow_mut();
        if enqueued {
            catalog.registry.complete_initialization(ticket).ok()?;
        } else {
            catalog.registry.fail_activation(ticket).ok()?;
        }
        Some(catalog.bump_activation_revision())
    });
    let Some(revision) = revision else {
        return false;
    };
    if enqueued {
        active.revision = revision;
    } else if active.ticket == Some(ticket) {
        active.clear();
    }
    PROFILE_ACTIVATION_CHANGED.signal(revision);
    enqueued
}

/// Profile-owned FILTER_ALWAYS for the current, fully active binding only.
/// Disconnect, initialization, failure or a profile without it yields false.
pub(crate) async fn active_channel_filter_always(controller_epoch: u32) -> bool {
    let active = ACTIVE_PROFILE.lock().await;
    let current = PROFILE_CATALOG.lock(|cell| {
        let catalog = cell.borrow();
        catalog.registry.activation_state() == ProfileActivationState::Active
            && catalog.registry.connected().map(|controller| controller.epoch)
                == Some(controller_epoch)
            && active.controller_epoch == controller_epoch
            && active.index == catalog.registry.active_index()
    });
    current
        && Profile::parse(&active.bytes[..active.len])
            .is_ok_and(|profile| profile.channel_filter_always())
}

pub(crate) async fn process_active_midi_transfer(
    controller_epoch: u32,
    session: &mut ControllerSession,
    bytes: &[u8],
) -> Result<UsbMidiBatchReport, ActiveProfileMidiError> {
    let active = ACTIVE_PROFILE.lock().await;
    let (state, active_index, connected_epoch) = PROFILE_CATALOG.lock(|cell| {
        let catalog = cell.borrow();
        (
            catalog.registry.activation_state(),
            catalog.registry.active_index(),
            catalog.registry.connected().map(|controller| controller.epoch),
        )
    });

    if state != ProfileActivationState::Active {
        return Err(ActiveProfileMidiError::NotActive);
    }
    if active.controller_epoch != controller_epoch
        || connected_epoch != Some(controller_epoch)
        || active.index != active_index
    {
        return Err(ActiveProfileMidiError::StaleController);
    }

    let profile = Profile::parse(&active.bytes[..active.len])
        .map_err(|_| ActiveProfileMidiError::InvalidActiveProfile)?;
    session
        .process_usb_midi_transfer(&profile, bytes)
        .map_err(ActiveProfileMidiError::Batch)
}


pub(crate) async fn map_active_led_update(
    controller_epoch: u32,
    update: LedUpdate,
) -> Result<Option<UsbMidiEvent>, ActiveProfileLedError> {
    let active = ACTIVE_PROFILE.lock().await;
    let (state, active_index, connected_epoch) = PROFILE_CATALOG.lock(|cell| {
        let catalog = cell.borrow();
        (
            catalog.registry.activation_state(),
            catalog.registry.active_index(),
            catalog.registry.connected().map(|controller| controller.epoch),
        )
    });

    if state != ProfileActivationState::Active {
        return Err(ActiveProfileLedError::NotActive);
    }
    if active.controller_epoch != controller_epoch
        || connected_epoch != Some(controller_epoch)
        || active.index != active_index
    {
        return Err(ActiveProfileLedError::StaleController);
    }

    let profile = Profile::parse(&active.bytes[..active.len])
        .map_err(|_| ActiveProfileLedError::InvalidActiveProfile)?;
    let Some([status, data1, data2]) = profile.map_led_update(update) else {
        return Ok(None);
    };

    let cin = status >> 4;
    if !(0x08..=0x0e).contains(&cin) || data1 > 0x7f || data2 > 0x7f {
        return Err(ActiveProfileLedError::InvalidMidiOutput);
    }

    Ok(Some(UsbMidiEvent {
        cable: 0,
        cin,
        bytes: [status, data1, data2],
        len: if matches!(cin, 0x0c | 0x0d) { 2 } else { 3 },
    }))
}

pub(crate) async fn dispatch_session_deliveries<S>(
    controller_epoch: u32,
    session: &mut ControllerSession,
    sink: &mut S,
    budget: usize,
) -> ProductDispatchReport
where
    S: SessionDeliverySink,
{
    let active = ACTIVE_PROFILE.lock().await;
    let active_is_current = PROFILE_CATALOG.lock(|cell| {
        let catalog = cell.borrow();
        catalog.registry.activation_state() == ProfileActivationState::Active
            && catalog.registry.connected().map(|controller| controller.epoch)
                == Some(controller_epoch)
            && active.controller_epoch == controller_epoch
            && active.index == catalog.registry.active_index()
    });
    let profile = if active_is_current {
        Profile::parse(&active.bytes[..active.len]).ok()
    } else {
        None
    };

    let mut report = ProductDispatchReport::default();
    for _ in 0..budget {
        match session.dispatch_next(profile.as_ref(), sink) {
            SessionDispatchOutcome::Idle => {
                report.idle = true;
                return report;
            }
            SessionDispatchOutcome::Accepted(_) => {
                report.accepted = report.accepted.saturating_add(1);
            }
            SessionDispatchOutcome::Backpressured(_) => {
                report.backpressured = true;
                return report;
            }
        }
    }

    report.budget_exhausted = true;
    report
}
