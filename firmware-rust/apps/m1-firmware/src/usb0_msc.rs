use core::cell::RefCell;

use embassy_futures::select::{Either, select};
use embassy_sync::{
    blocking_mutex::{CriticalSectionMutex, raw::CriticalSectionRawMutex},
    channel::Channel,
};
use embassy_usb_driver::host::{DeviceEvent, UsbHostAllocator};
use embassy_usb_host::{
    BusRoute, BusState, EnumerationError,
    class::msc::{BlockCapacity, MscDevice, MscError, MscLun},
    handler::EnumerationInfo,
};
use pajoniiir_media_block::{BlockGeometry, TransferError};
use pajoniiir_media_fs::FileSystemKind;
use pajoniiir_media_io::{AsyncPartitionDevice, OwnedBlockDevice, OwnedWritableBlockDevice};
use pajoniiir_media_partition::{VolumeKind, classify_boot_sector};
use pajoniiir_media_session::{DisconnectResult, MediaHandle, MediaLease, MediaSession};
use pajoniiir_media_usb_msc::{
    UsbMscCapacity, UsbMscCompletionGate, UsbMscDiscoveryPlan, UsbMscDiscoveryPlanError,
    UsbMscHandleSequencer, UsbMscMountAttempt, UsbMscMountCoordinator, UsbMscMountError,
    UsbMscMountSelection, UsbMscMountedMedia, UsbMscRequestKind, UsbMscRequestTicket,
    UsbMscSessionBridge, UsbMscSessionError, usb_address_source,
};
use static_cell::StaticCell;

pub(crate) const USB0_MSC_QUEUE_DEPTH: usize = 4;
const USB0_ENUM_CONFIG_BYTES: usize = 512;

static USB0_BUS_STATE: BusState = BusState::new();

pub(crate) static USB0_MSC_REQUESTS: Channel<
    CriticalSectionRawMutex,
    Usb0MscRequest,
    USB0_MSC_QUEUE_DEPTH,
> = Channel::new();

pub(crate) static USB0_MSC_COMPLETIONS: Channel<
    CriticalSectionRawMutex,
    Usb0MscCompletion,
    USB0_MSC_QUEUE_DEPTH,
> = Channel::new();

pub(crate) static USB0_MSC_OWNER_EVENTS: Channel<
    CriticalSectionRawMutex,
    Usb0MscOwnerEvent,
    USB0_MSC_QUEUE_DEPTH,
> = Channel::new();

pub(crate) static USB0_MSC_FILESYSTEM_EVENTS: Channel<
    CriticalSectionRawMutex,
    Usb0MscFilesystemEvent,
    USB0_MSC_QUEUE_DEPTH,
> = Channel::new();

static USB0_MSC_LIFECYCLE: CriticalSectionMutex<RefCell<Usb0MscLifecycle>> =
    CriticalSectionMutex::new(RefCell::new(Usb0MscLifecycle::new()));

static USB0_MSC_MEDIA_STATE: CriticalSectionMutex<RefCell<Usb0MscMediaState>> =
    CriticalSectionMutex::new(RefCell::new(Usb0MscMediaState::Idle));

static USB0_MSC_SCRATCH_512: StaticCell<[u8; 512]> = StaticCell::new();
static USB0_MSC_SCRATCH_1024: StaticCell<[u8; 1_024]> = StaticCell::new();
static USB0_MSC_SCRATCH_2048: StaticCell<[u8; 2_048]> = StaticCell::new();
static USB0_MSC_SCRATCH_4096: StaticCell<[u8; 4_096]> = StaticCell::new();

static USB0_FS_SCRATCH_512: StaticCell<[u8; 512]> = StaticCell::new();
static USB0_FS_SCRATCH_1024: StaticCell<[u8; 1_024]> = StaticCell::new();
static USB0_FS_SCRATCH_2048: StaticCell<[u8; 2_048]> = StaticCell::new();
static USB0_FS_SCRATCH_4096: StaticCell<[u8; 4_096]> = StaticCell::new();

static USB0_MSC_FILESYSTEM_STATE: CriticalSectionMutex<RefCell<Usb0MscFilesystemState>> =
    CriticalSectionMutex::new(RefCell::new(Usb0MscFilesystemState::Idle));

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb0MscMediaWorkerError {
    Mount(UsbMscMountError),
    Discovery(Usb0MscDiscoveryClientError),
    UnsupportedBlockSize(u32),
    ScratchUnavailable(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb0MscMediaState {
    Idle,
    Discovering {
        binding: Usb0MscBinding,
    },
    Mounted {
        binding: Usb0MscBinding,
        media: UsbMscMountedMedia,
    },
    Failed {
        binding: Usb0MscBinding,
        error: Usb0MscMediaWorkerError,
    },
    Detached {
        binding: Usb0MscBinding,
        reason: Usb0MscDetachReason,
        result: DisconnectResult,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb0MscFilesystemProbeError {
    UnsupportedBlockSize(u32),
    ScratchUnavailable(u32),
    Io(Usb0MscIoError),
    UnknownVolume,
    KindMismatch {
        expected: FileSystemKind,
        observed: FileSystemKind,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb0MscFilesystemState {
    Idle,
    Probing {
        binding: Usb0MscBinding,
        media: UsbMscMountedMedia,
    },
    VolumeProbed {
        binding: Usb0MscBinding,
        media: UsbMscMountedMedia,
        filesystem: FileSystemKind,
    },
    Failed {
        binding: Usb0MscBinding,
        media: UsbMscMountedMedia,
        error: Usb0MscFilesystemProbeError,
    },
    Detached {
        binding: Usb0MscBinding,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb0MscFilesystemEvent {
    Mounted {
        binding: Usb0MscBinding,
        media: UsbMscMountedMedia,
    },
    Detached {
        binding: Usb0MscBinding,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb0MscDetachReason {
    Disconnected,
    Overcurrent,
    Replaced,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb0MscOwnerEvent {
    Ready {
        binding: Usb0MscBinding,
        probe: Usb0MscProbe,
    },
    Detached {
        binding: Usb0MscBinding,
        reason: Usb0MscDetachReason,
        result: DisconnectResult,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb0MscLifecycleError {
    InvalidDeviceAddress,
    Session(UsbMscSessionError),
}

impl From<UsbMscSessionError> for Usb0MscLifecycleError {
    fn from(error: UsbMscSessionError) -> Self {
        Self::Session(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Usb0MscBinding {
    pub(crate) lease: MediaLease,
    pub(crate) handle: MediaHandle,
    pub(crate) device_address: u8,
}

pub(crate) struct Usb0MscLifecycle {
    bridge: UsbMscSessionBridge,
    handles: UsbMscHandleSequencer,
}

impl Usb0MscLifecycle {
    pub(crate) const fn new() -> Self {
        Self {
            bridge: UsbMscSessionBridge::new(),
            handles: UsbMscHandleSequencer::new(),
        }
    }

    pub(crate) fn on_enumerated(
        &mut self,
        info: &EnumerationInfo,
    ) -> Result<Usb0MscBinding, Usb0MscLifecycleError> {
        let source = usb_address_source(info.device_address)
            .ok_or(Usb0MscLifecycleError::InvalidDeviceAddress)?;
        let handle = self.handles.issue();
        let lease = self.bridge.on_enumerated(source, handle)?;

        Ok(Usb0MscBinding {
            lease,
            handle,
            device_address: info.device_address,
        })
    }

    /// Process HandlerEvent::HandlerDisconnected for the handler that owns
    /// this binding. Old bindings fail closed after a newer enumeration.
    pub(crate) fn on_handler_disconnected(
        &mut self,
        binding: Usb0MscBinding,
    ) -> DisconnectResult {
        self.bridge.on_disconnect(binding.handle)
    }

    pub(crate) fn session(&self) -> &MediaSession {
        self.bridge.session()
    }

    pub(crate) fn bridge_mut(&mut self) -> &mut UsbMscSessionBridge {
        &mut self.bridge
    }
}

impl Default for Usb0MscLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

fn with_lifecycle_mut<R>(f: impl FnOnce(&mut Usb0MscLifecycle) -> R) -> R {
    USB0_MSC_LIFECYCLE.lock(|cell| {
        let mut lifecycle = cell.borrow_mut();
        f(&mut lifecycle)
    })
}

fn with_lifecycle<R>(f: impl FnOnce(&Usb0MscLifecycle) -> R) -> R {
    USB0_MSC_LIFECYCLE.lock(|cell| {
        let lifecycle = cell.borrow();
        f(&lifecycle)
    })
}

fn set_media_state(state: Usb0MscMediaState) {
    USB0_MSC_MEDIA_STATE.lock(|cell| {
        *cell.borrow_mut() = state;
    });
}

pub(crate) fn media_state() -> Usb0MscMediaState {
    USB0_MSC_MEDIA_STATE.lock(|cell| *cell.borrow())
}

fn set_filesystem_state(state: Usb0MscFilesystemState) {
    USB0_MSC_FILESYSTEM_STATE.lock(|cell| {
        *cell.borrow_mut() = state;
    });
}

pub(crate) fn filesystem_state() -> Usb0MscFilesystemState {
    USB0_MSC_FILESYSTEM_STATE.lock(|cell| *cell.borrow())
}

pub(crate) fn issue_current_request(
    lun: u8,
    kind: UsbMscRequestKind,
) -> Result<UsbMscRequestTicket, UsbMscSessionError> {
    with_lifecycle_mut(|lifecycle| lifecycle.bridge_mut().issue(lun, kind))
}

fn issue_bound_request(
    attempt: UsbMscMountAttempt,
    kind: UsbMscRequestKind,
) -> Result<UsbMscRequestTicket, UsbMscSessionError> {
    with_lifecycle_mut(|lifecycle| {
        lifecycle.bridge_mut().issue_for(
            attempt.lease,
            attempt.handle,
            attempt.lun,
            kind,
        )
    })
}

fn prepare_bound_read_request(
    attempt: UsbMscMountAttempt,
    first_block: u64,
    block_count: u32,
    buffer: &'static mut [u8],
) -> Result<Usb0MscRequest, (UsbMscSessionError, &'static mut [u8])> {
    match issue_bound_request(
        attempt,
        UsbMscRequestKind::Read {
            first_block,
            block_count,
        },
    ) {
        Ok(ticket) => Ok(Usb0MscRequest::Read { ticket, buffer }),
        Err(error) => Err((error, buffer)),
    }
}

fn prepare_bound_write_request(
    attempt: UsbMscMountAttempt,
    first_block: u64,
    block_count: u32,
    buffer: &'static mut [u8],
) -> Result<Usb0MscRequest, (UsbMscSessionError, &'static mut [u8])> {
    match issue_bound_request(
        attempt,
        UsbMscRequestKind::Write {
            first_block,
            block_count,
        },
    ) {
        Ok(ticket) => Ok(Usb0MscRequest::Write { ticket, buffer }),
        Err(error) => Err((error, buffer)),
    }
}

fn prepare_bound_flush_request(
    attempt: UsbMscMountAttempt,
) -> Result<Usb0MscRequest, UsbMscSessionError> {
    issue_bound_request(attempt, UsbMscRequestKind::Flush)
        .map(|ticket| Usb0MscRequest::Flush { ticket })
}

pub(crate) fn prepare_read_request(
    lun: u8,
    first_block: u64,
    block_count: u32,
    buffer: &'static mut [u8],
) -> Result<Usb0MscRequest, (UsbMscSessionError, &'static mut [u8])> {
    match issue_current_request(
        lun,
        UsbMscRequestKind::Read {
            first_block,
            block_count,
        },
    ) {
        Ok(ticket) => Ok(Usb0MscRequest::Read { ticket, buffer }),
        Err(error) => Err((error, buffer)),
    }
}

pub(crate) fn prepare_write_request(
    lun: u8,
    first_block: u64,
    block_count: u32,
    buffer: &'static mut [u8],
) -> Result<Usb0MscRequest, (UsbMscSessionError, &'static mut [u8])> {
    match issue_current_request(
        lun,
        UsbMscRequestKind::Write {
            first_block,
            block_count,
        },
    ) {
        Ok(ticket) => Ok(Usb0MscRequest::Write { ticket, buffer }),
        Err(error) => Err((error, buffer)),
    }
}

pub(crate) fn prepare_flush_request(
    lun: u8,
) -> Result<Usb0MscRequest, UsbMscSessionError> {
    issue_current_request(lun, UsbMscRequestKind::Flush)
        .map(|ticket| Usb0MscRequest::Flush { ticket })
}

pub(crate) fn completion_is_current(completion: &Usb0MscCompletion) -> bool {
    with_lifecycle(|lifecycle| completion.is_current(lifecycle.session()))
}

fn shared_on_enumerated(
    info: &EnumerationInfo,
) -> Result<Usb0MscBinding, Usb0MscLifecycleError> {
    with_lifecycle_mut(|lifecycle| lifecycle.on_enumerated(info))
}

fn shared_on_detached(binding: Usb0MscBinding) -> DisconnectResult {
    with_lifecycle_mut(|lifecycle| lifecycle.on_handler_disconnected(binding))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Usb0MscProbe {
    pub(crate) device_address: u8,
    pub(crate) vendor_id: u16,
    pub(crate) product_id: u16,
    pub(crate) num_luns: u8,
    pub(crate) capacity: UsbMscCapacity,
}

#[derive(Debug)]
pub(crate) enum Usb0MscProbeError {
    Enumeration(EnumerationError),
    Lifecycle(Usb0MscLifecycleError),
    Class(MscError),
    Capacity(Usb0MscError),
}

impl From<EnumerationError> for Usb0MscProbeError {
    fn from(error: EnumerationError) -> Self {
        Self::Enumeration(error)
    }
}

impl From<Usb0MscLifecycleError> for Usb0MscProbeError {
    fn from(error: Usb0MscLifecycleError) -> Self {
        Self::Lifecycle(error)
    }
}

impl From<MscError> for Usb0MscProbeError {
    fn from(error: MscError) -> Self {
        Self::Class(error)
    }
}

impl From<Usb0MscError> for Usb0MscProbeError {
    fn from(error: Usb0MscError) -> Self {
        Self::Capacity(error)
    }
}

/// One-shot root-port diagnostic proving the full target API chain.
///
/// This is not the production owner loop. It intentionally tears the device
/// down after probing so the compile gate covers address allocation and cleanup
/// without leaving a stale MediaSession behind.
pub(crate) async fn probe_root_msc_once(
    usb_hs: esp_hal::peripherals::USB_HS<'static>,
    lifecycle: &mut Usb0MscLifecycle,
) -> Result<Usb0MscProbe, Usb0MscProbeError> {
    let usb = esp_hal::usb::otg::Usb::new_hs(usb_hs);
    let driver = esp_hal::usb::otg::embassy_usb_host::Driver::new(usb);
    let (mut controller, bus) = embassy_usb_host::bus(driver, &USB0_BUS_STATE);

    let speed = controller.wait_for_connection().await;
    let mut config = [0u8; USB0_ENUM_CONFIG_BYTES];
    let (info, config_len) = bus
        .enumerate(BusRoute::Direct(speed), &mut config)
        .await?;

    let binding = match lifecycle.on_enumerated(&info) {
        Ok(binding) => binding,
        Err(error) => {
            bus.free_address(info.device_address);
            return Err(error.into());
        }
    };

    let result = async {
        let device = MscDevice::new(&bus, &info, &config[..config_len]).await?;
        let num_luns = device.num_luns();
        let mut lun = device.lun(0)?;
        let capacity = probe_capacity(&mut lun).await?;

        Ok(Usb0MscProbe {
            device_address: info.device_address,
            vendor_id: info.device_desc.vendor_id,
            product_id: info.device_desc.product_id,
            num_luns,
            capacity,
        })
    }
    .await;

    let _ = lifecycle.on_handler_disconnected(binding);
    bus.free_address(info.device_address);
    result
}

/// Persistent root-port MSC owner.
///
/// This task owns the Embassy bus, device and LUN for their full lifetime.
/// MediaSession state is shared only through short critical sections and is
/// never locked across an await. Queued requests retain ownership of their
/// buffers even if a detach wins the USB command/device-event race.
pub(crate) async fn run_root_msc_owner(
    usb_hs: esp_hal::peripherals::USB_HS<'static>,
) -> ! {
    let usb = esp_hal::usb::otg::Usb::new_hs(usb_hs);
    let driver = esp_hal::usb::otg::embassy_usb_host::Driver::new(usb);
    let (mut controller, bus) = embassy_usb_host::bus(driver, &USB0_BUS_STATE);
    let mut pending_speed = None;

    loop {
        let speed = match pending_speed.take() {
            Some(speed) => speed,
            None => loop {
                match controller.wait_for_device_event().await {
                    DeviceEvent::Connected(speed) => break speed,
                    DeviceEvent::Disconnected | DeviceEvent::Overcurrent => {}
                    _ => {}
                }
            },
        };

        let mut config = [0u8; USB0_ENUM_CONFIG_BYTES];
        let (info, config_len) = match bus
            .enumerate(BusRoute::Direct(speed), &mut config)
            .await
        {
            Ok(enumerated) => enumerated,
            Err(_) => continue,
        };

        let binding = match shared_on_enumerated(&info) {
            Ok(binding) => binding,
            Err(_) => {
                bus.free_address(info.device_address);
                continue;
            }
        };

        let device = match MscDevice::new(&bus, &info, &config[..config_len]).await {
            Ok(device) => device,
            Err(_) => {
                let _ = shared_on_detached(binding);
                bus.free_address(info.device_address);
                continue;
            }
        };
        let num_luns = device.num_luns();
        let mut lun = match device.lun(0) {
            Ok(lun) => lun,
            Err(_) => {
                let _ = shared_on_detached(binding);
                drop(device);
                bus.free_address(info.device_address);
                continue;
            }
        };
        let capacity = match probe_capacity(&mut lun).await {
            Ok(capacity) => capacity,
            Err(_) => {
                let _ = shared_on_detached(binding);
                drop(lun);
                drop(device);
                bus.free_address(info.device_address);
                continue;
            }
        };

        let probe = Usb0MscProbe {
            device_address: info.device_address,
            vendor_id: info.device_desc.vendor_id,
            product_id: info.device_desc.product_id,
            num_luns,
            capacity,
        };
        USB0_MSC_OWNER_EVENTS
            .send(Usb0MscOwnerEvent::Ready { binding, probe })
            .await;

        let replacement_speed = 'attached: loop {
            match select(
                USB0_MSC_REQUESTS.receive(),
                controller.wait_for_device_event(),
            )
            .await
            {
                Either::First(mut request) => {
                    match select(
                        execute_request_status(&mut lun, binding.lease, &mut request),
                        controller.wait_for_device_event(),
                    )
                    .await
                    {
                        Either::First(status) => {
                            USB0_MSC_COMPLETIONS
                                .send(complete_with_status(request, status))
                                .await;
                        }
                        Either::Second(event) => {
                            let (status, reason, next_speed) = match event {
                                DeviceEvent::Disconnected => (
                                    Usb0MscCompletionStatus::Disconnected,
                                    Usb0MscDetachReason::Disconnected,
                                    None,
                                ),
                                DeviceEvent::Overcurrent => (
                                    Usb0MscCompletionStatus::Overcurrent,
                                    Usb0MscDetachReason::Overcurrent,
                                    None,
                                ),
                                DeviceEvent::Connected(speed) => (
                                    Usb0MscCompletionStatus::Disconnected,
                                    Usb0MscDetachReason::Replaced,
                                    Some(speed),
                                ),
                                _ => (
                                    Usb0MscCompletionStatus::HostError,
                                    Usb0MscDetachReason::Disconnected,
                                    None,
                                ),
                            };

                            let result = shared_on_detached(binding);
                            USB0_MSC_COMPLETIONS
                                .send(complete_with_status(request, status))
                                .await;
                            USB0_MSC_OWNER_EVENTS
                                .send(Usb0MscOwnerEvent::Detached {
                                    binding,
                                    reason,
                                    result,
                                })
                                .await;
                            break 'attached next_speed;
                        }
                    }
                }
                Either::Second(event) => {
                    let (reason, next_speed) = match event {
                        DeviceEvent::Disconnected => (Usb0MscDetachReason::Disconnected, None),
                        DeviceEvent::Overcurrent => (Usb0MscDetachReason::Overcurrent, None),
                        DeviceEvent::Connected(speed) => {
                            (Usb0MscDetachReason::Replaced, Some(speed))
                        }
                        _ => continue,
                    };

                    let result = shared_on_detached(binding);
                    USB0_MSC_OWNER_EVENTS
                        .send(Usb0MscOwnerEvent::Detached {
                            binding,
                            reason,
                            result,
                        })
                        .await;
                    break 'attached next_speed;
                }
            }
        };

        drop(lun);
        drop(device);
        bus.free_address(info.device_address);
        pending_speed = replacement_speed;
    }
}

#[derive(Debug)]
pub(crate) enum Usb0MscError {
    Host(MscError),
    InvalidCapacity,
}

impl From<MscError> for Usb0MscError {
    fn from(error: MscError) -> Self {
        Self::Host(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb0MscCompletionStatus {
    Success,
    StaleLease,
    WrongLun,
    InvalidTransfer,
    HostError,
    Disconnected,
    Overcurrent,
}

#[derive(Debug)]
pub(crate) enum Usb0MscRequest {
    Read {
        ticket: UsbMscRequestTicket,
        buffer: &'static mut [u8],
    },
    Write {
        ticket: UsbMscRequestTicket,
        buffer: &'static mut [u8],
    },
    Flush {
        ticket: UsbMscRequestTicket,
    },
}

impl Usb0MscRequest {
    pub(crate) const fn ticket(&self) -> UsbMscRequestTicket {
        match self {
            Self::Read { ticket, .. }
            | Self::Write { ticket, .. }
            | Self::Flush { ticket } => *ticket,
        }
    }
}

#[derive(Debug)]
pub(crate) enum Usb0MscCompletion {
    Read {
        ticket: UsbMscRequestTicket,
        buffer: &'static mut [u8],
        status: Usb0MscCompletionStatus,
    },
    Write {
        ticket: UsbMscRequestTicket,
        buffer: &'static mut [u8],
        status: Usb0MscCompletionStatus,
    },
    Flush {
        ticket: UsbMscRequestTicket,
        status: Usb0MscCompletionStatus,
    },
}

#[derive(Debug)]
pub(crate) enum Usb0MscSubmitResult {
    Completed(Usb0MscCompletion),
    QueueFull(Usb0MscRequest),
    PendingCompletion(Usb0MscRequest),
    UnexpectedCompletion {
        expected: UsbMscRequestTicket,
        completion: Usb0MscCompletion,
    },
}

/// Single-media-worker client for the USB0 owner request/completion channels.
///
/// Exactly one long-lived media worker should own this value. submit() uses a
/// nonblocking channel send so an unsent request is returned with its static
/// buffer intact. Once accepted by the owner, the ticket stays in pending even
/// if the receive future is cancelled; the same worker can resume with
/// receive_pending_completion() before issuing another operation.
pub(crate) struct Usb0MscClient {
    pending: Option<UsbMscRequestTicket>,
}

impl Usb0MscClient {
    pub(crate) const fn new() -> Self {
        Self { pending: None }
    }

    pub(crate) const fn pending_ticket(&self) -> Option<UsbMscRequestTicket> {
        self.pending
    }

    pub(crate) const fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub(crate) async fn submit(
        &mut self,
        request: Usb0MscRequest,
    ) -> Usb0MscSubmitResult {
        if self.pending.is_some() {
            return Usb0MscSubmitResult::PendingCompletion(request);
        }

        let ticket = request.ticket();
        match USB0_MSC_REQUESTS.sender().try_send(request) {
            Ok(()) => self.pending = Some(ticket),
            Err(embassy_sync::channel::TrySendError::Full(request)) => {
                return Usb0MscSubmitResult::QueueFull(request);
            }
        }

        self.receive_pending_completion()
            .await
            .expect("pending ticket is set before awaiting completion")
    }

    pub(crate) async fn receive_pending_completion(
        &mut self,
    ) -> Option<Usb0MscSubmitResult> {
        let expected = self.pending?;
        let completion = USB0_MSC_COMPLETIONS.receive().await;

        if completion.ticket() != expected {
            return Some(Usb0MscSubmitResult::UnexpectedCompletion {
                expected,
                completion,
            });
        }

        self.pending = None;
        Some(Usb0MscSubmitResult::Completed(completion))
    }
}

impl Default for Usb0MscClient {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb0MscIoError {
    Transfer(TransferError),
    Session(UsbMscSessionError),
    QueueFull,
    PendingCompletion,
    Completion(Usb0MscCompletionStatus),
    StaleCompletion,
}

pub(crate) struct Usb0MscAsyncBlockDevice {
    attempt: UsbMscMountAttempt,
    client: Usb0MscClient,
}

impl Usb0MscAsyncBlockDevice {
    pub(crate) const fn new(media: UsbMscMountedMedia) -> Self {
        Self {
            attempt: media.attempt,
            client: Usb0MscClient::new(),
        }
    }

    pub(crate) const fn attempt(&self) -> UsbMscMountAttempt {
        self.attempt
    }

    fn completion_is_bound_current(&self, ticket: UsbMscRequestTicket) -> bool {
        ticket.lease == self.attempt.lease
            && ticket.lun == self.attempt.lun
            && with_lifecycle(|lifecycle| {
                lifecycle.session().validate(self.attempt.lease)
                    && lifecycle.session().handle() == Some(self.attempt.handle)
                    && UsbMscCompletionGate::accepts(lifecycle.session(), ticket)
            })
    }

    fn invariant_reset() -> ! {
        esp_hal::system::software_reset()
    }
}

impl OwnedBlockDevice for Usb0MscAsyncBlockDevice {
    type Error = Usb0MscIoError;
    type Buffer = &'static mut [u8];

    fn geometry(&self) -> BlockGeometry {
        self.attempt.geometry
    }

    async fn read_blocks_owned(
        &mut self,
        first_block: u64,
        block_count: u32,
        buffer: Self::Buffer,
    ) -> Result<Self::Buffer, (Self::Error, Self::Buffer)> {
        if let Err(error) =
            self.geometry()
                .validate_transfer(first_block, block_count, buffer.len())
        {
            return Err((Usb0MscIoError::Transfer(error), buffer));
        }

        let request =
            match prepare_bound_read_request(self.attempt, first_block, block_count, buffer) {
                Ok(request) => request,
                Err((error, buffer)) => {
                    return Err((Usb0MscIoError::Session(error), buffer));
                }
            };

        match self.client.submit(request).await {
            Usb0MscSubmitResult::Completed(Usb0MscCompletion::Read {
                ticket,
                buffer,
                status,
            }) => {
                if status != Usb0MscCompletionStatus::Success {
                    return Err((Usb0MscIoError::Completion(status), buffer));
                }
                if !self.completion_is_bound_current(ticket) {
                    return Err((Usb0MscIoError::StaleCompletion, buffer));
                }
                Ok(buffer)
            }
            Usb0MscSubmitResult::QueueFull(Usb0MscRequest::Read { buffer, .. }) => {
                Err((Usb0MscIoError::QueueFull, buffer))
            }
            Usb0MscSubmitResult::PendingCompletion(Usb0MscRequest::Read { buffer, .. }) => {
                Err((Usb0MscIoError::PendingCompletion, buffer))
            }
            Usb0MscSubmitResult::Completed(_)
            | Usb0MscSubmitResult::QueueFull(_)
            | Usb0MscSubmitResult::PendingCompletion(_)
            | Usb0MscSubmitResult::UnexpectedCompletion { .. } => Self::invariant_reset(),
        }
    }
}

impl OwnedWritableBlockDevice for Usb0MscAsyncBlockDevice {
    async fn write_blocks_owned(
        &mut self,
        first_block: u64,
        block_count: u32,
        buffer: Self::Buffer,
    ) -> Result<Self::Buffer, (Self::Error, Self::Buffer)> {
        if let Err(error) =
            self.geometry()
                .validate_transfer(first_block, block_count, buffer.len())
        {
            return Err((Usb0MscIoError::Transfer(error), buffer));
        }

        let request =
            match prepare_bound_write_request(self.attempt, first_block, block_count, buffer) {
                Ok(request) => request,
                Err((error, buffer)) => {
                    return Err((Usb0MscIoError::Session(error), buffer));
                }
            };

        match self.client.submit(request).await {
            Usb0MscSubmitResult::Completed(Usb0MscCompletion::Write {
                ticket,
                buffer,
                status,
            }) => {
                if status != Usb0MscCompletionStatus::Success {
                    return Err((Usb0MscIoError::Completion(status), buffer));
                }
                if !self.completion_is_bound_current(ticket) {
                    return Err((Usb0MscIoError::StaleCompletion, buffer));
                }
                Ok(buffer)
            }
            Usb0MscSubmitResult::QueueFull(Usb0MscRequest::Write { buffer, .. }) => {
                Err((Usb0MscIoError::QueueFull, buffer))
            }
            Usb0MscSubmitResult::PendingCompletion(Usb0MscRequest::Write { buffer, .. }) => {
                Err((Usb0MscIoError::PendingCompletion, buffer))
            }
            Usb0MscSubmitResult::Completed(_)
            | Usb0MscSubmitResult::QueueFull(_)
            | Usb0MscSubmitResult::PendingCompletion(_)
            | Usb0MscSubmitResult::UnexpectedCompletion { .. } => Self::invariant_reset(),
        }
    }

    async fn flush_owned(&mut self) -> Result<(), Self::Error> {
        let request =
            prepare_bound_flush_request(self.attempt).map_err(Usb0MscIoError::Session)?;

        match self.client.submit(request).await {
            Usb0MscSubmitResult::Completed(Usb0MscCompletion::Flush { ticket, status }) => {
                if status != Usb0MscCompletionStatus::Success {
                    return Err(Usb0MscIoError::Completion(status));
                }
                if !self.completion_is_bound_current(ticket) {
                    return Err(Usb0MscIoError::StaleCompletion);
                }
                Ok(())
            }
            Usb0MscSubmitResult::QueueFull(Usb0MscRequest::Flush { .. }) => {
                Err(Usb0MscIoError::QueueFull)
            }
            Usb0MscSubmitResult::PendingCompletion(Usb0MscRequest::Flush { .. }) => {
                Err(Usb0MscIoError::PendingCompletion)
            }
            Usb0MscSubmitResult::Completed(_)
            | Usb0MscSubmitResult::QueueFull(_)
            | Usb0MscSubmitResult::PendingCompletion(_)
            | Usb0MscSubmitResult::UnexpectedCompletion { .. } => Self::invariant_reset(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb0MscDiscoveryClientError {
    ScratchSize { expected: usize, actual: usize },
    MissingScratch,
    MissingPendingLba,
    Plan(UsbMscDiscoveryPlanError),
    Session(UsbMscSessionError),
    QueueFull,
    PendingCompletion,
    UnexpectedCompletion,
    WrongCompletionKind,
    Completion(Usb0MscCompletionStatus),
    StaleCompletion,
}

impl From<UsbMscDiscoveryPlanError> for Usb0MscDiscoveryClientError {
    fn from(error: UsbMscDiscoveryPlanError) -> Self {
        Self::Plan(error)
    }
}

/// Resumable async partition-discovery client.
///
/// The worker owns exactly one logical-block scratch buffer. While a read is in
/// flight the buffer is owned by the USB0 owner task and `scratch` is None.
/// If this future is cancelled, `client.pending` and `pending_lba` remain in
/// the worker; the next step resumes by receiving that completion instead of
/// issuing a duplicate SCSI read.
pub(crate) struct Usb0MscDiscoveryWorker {
    client: Usb0MscClient,
    plan: UsbMscDiscoveryPlan,
    scratch: Option<&'static mut [u8]>,
    pending_lba: Option<u64>,
}

impl Usb0MscDiscoveryWorker {
    pub(crate) fn new(
        attempt: UsbMscMountAttempt,
        scratch: &'static mut [u8],
    ) -> Result<Self, (Usb0MscDiscoveryClientError, &'static mut [u8])> {
        let expected = attempt.geometry.block_size as usize;
        if scratch.len() != expected {
            let actual = scratch.len();
            return Err((
                Usb0MscDiscoveryClientError::ScratchSize { expected, actual },
                scratch,
            ));
        }

        let plan = match UsbMscDiscoveryPlan::new(attempt) {
            Ok(plan) => plan,
            Err(error) => {
                return Err((Usb0MscDiscoveryClientError::Plan(error), scratch));
            }
        };

        Ok(Self {
            client: Usb0MscClient::new(),
            plan,
            scratch: Some(scratch),
            pending_lba: None,
        })
    }

    pub(crate) const fn has_pending(&self) -> bool {
        self.client.has_pending()
    }

    pub(crate) const fn pending_lba(&self) -> Option<u64> {
        self.pending_lba
    }

    pub(crate) fn scratch(&self) -> Option<&[u8]> {
        self.scratch.as_deref()
    }

    pub(crate) fn into_scratch(self) -> Option<&'static mut [u8]> {
        self.scratch
    }

    pub(crate) async fn step(
        &mut self,
        coordinator: &UsbMscMountCoordinator,
    ) -> Result<Option<UsbMscMountSelection>, Usb0MscDiscoveryClientError> {
        let result = if self.client.has_pending() {
            self.client
                .receive_pending_completion()
                .await
                .ok_or(Usb0MscDiscoveryClientError::MissingPendingLba)?
        } else {
            let Some(lba) = self.plan.next_lba()? else {
                return Ok(self.plan.selection());
            };
            let scratch = self
                .scratch
                .take()
                .ok_or(Usb0MscDiscoveryClientError::MissingScratch)?;
            let request = match prepare_read_request(self.plan.attempt().lun, lba, 1, scratch) {
                Ok(request) => request,
                Err((error, scratch)) => {
                    self.scratch = Some(scratch);
                    return Err(Usb0MscDiscoveryClientError::Session(error));
                }
            };
            self.pending_lba = Some(lba);
            self.client.submit(request).await
        };

        self.handle_submit_result(coordinator, result)
    }

    pub(crate) async fn run_to_selection(
        &mut self,
        coordinator: &UsbMscMountCoordinator,
    ) -> Result<UsbMscMountSelection, Usb0MscDiscoveryClientError> {
        loop {
            if let Some(selection) = self.step(coordinator).await? {
                return Ok(selection);
            }
        }
    }

    fn handle_submit_result(
        &mut self,
        coordinator: &UsbMscMountCoordinator,
        result: Usb0MscSubmitResult,
    ) -> Result<Option<UsbMscMountSelection>, Usb0MscDiscoveryClientError> {
        match result {
            Usb0MscSubmitResult::Completed(Usb0MscCompletion::Read {
                ticket,
                buffer,
                status,
            }) => {
                let lba = self
                    .pending_lba
                    .take()
                    .ok_or(Usb0MscDiscoveryClientError::MissingPendingLba)?;
                self.scratch = Some(buffer);

                if status != Usb0MscCompletionStatus::Success {
                    return Err(Usb0MscDiscoveryClientError::Completion(status));
                }
                if !with_lifecycle(|lifecycle| {
                    UsbMscCompletionGate::accepts(lifecycle.session(), ticket)
                }) {
                    return Err(Usb0MscDiscoveryClientError::StaleCompletion);
                }

                let block = self
                    .scratch
                    .as_deref()
                    .ok_or(Usb0MscDiscoveryClientError::MissingScratch)?;
                self.plan
                    .ingest_block(coordinator, lba, block)
                    .map_err(Usb0MscDiscoveryClientError::Plan)
            }
            Usb0MscSubmitResult::Completed(_) => {
                self.pending_lba = None;
                Err(Usb0MscDiscoveryClientError::WrongCompletionKind)
            }
            Usb0MscSubmitResult::QueueFull(request) => {
                self.pending_lba = None;
                self.restore_unsent_read(request)?;
                Err(Usb0MscDiscoveryClientError::QueueFull)
            }
            Usb0MscSubmitResult::PendingCompletion(request) => {
                self.restore_unsent_read(request)?;
                Err(Usb0MscDiscoveryClientError::PendingCompletion)
            }
            Usb0MscSubmitResult::UnexpectedCompletion { .. } => {
                Err(Usb0MscDiscoveryClientError::UnexpectedCompletion)
            }
        }
    }

    fn restore_unsent_read(
        &mut self,
        request: Usb0MscRequest,
    ) -> Result<(), Usb0MscDiscoveryClientError> {
        match request {
            Usb0MscRequest::Read { buffer, .. } => {
                self.scratch = Some(buffer);
                Ok(())
            }
            _ => Err(Usb0MscDiscoveryClientError::WrongCompletionKind),
        }
    }
}

pub(crate) fn begin_current_mount(
    coordinator: &mut UsbMscMountCoordinator,
    binding: Usb0MscBinding,
    lun: u8,
    capacity: UsbMscCapacity,
) -> Result<UsbMscMountAttempt, UsbMscMountError> {
    with_lifecycle(|lifecycle| {
        coordinator.begin(
            lifecycle.session(),
            binding.lease,
            binding.handle,
            lun,
            capacity,
        )
    })
}

pub(crate) fn commit_current_mount(
    coordinator: &mut UsbMscMountCoordinator,
    attempt: UsbMscMountAttempt,
    selection: UsbMscMountSelection,
) -> Result<UsbMscMountedMedia, UsbMscMountError> {
    with_lifecycle_mut(|lifecycle| {
        coordinator.commit(
            lifecycle.bridge_mut().session_mut(),
            attempt,
            selection,
        )
    })
}

pub(crate) fn detach_mount(
    coordinator: &mut UsbMscMountCoordinator,
    binding: Usb0MscBinding,
) -> bool {
    coordinator.on_detached(binding.handle)
}

struct Usb0MscScratchPool {
    sector_512: Option<&'static mut [u8]>,
    sector_1024: Option<&'static mut [u8]>,
    sector_2048: Option<&'static mut [u8]>,
    sector_4096: Option<&'static mut [u8]>,
}

impl Usb0MscScratchPool {
    fn new() -> Self {
        Self {
            sector_512: Some(&mut USB0_MSC_SCRATCH_512.init([0; 512])[..]),
            sector_1024: Some(&mut USB0_MSC_SCRATCH_1024.init([0; 1_024])[..]),
            sector_2048: Some(&mut USB0_MSC_SCRATCH_2048.init([0; 2_048])[..]),
            sector_4096: Some(&mut USB0_MSC_SCRATCH_4096.init([0; 4_096])[..]),
        }
    }

    fn take(
        &mut self,
        block_size: u32,
    ) -> Result<&'static mut [u8], Usb0MscMediaWorkerError> {
        let slot = match block_size {
            512 => &mut self.sector_512,
            1_024 => &mut self.sector_1024,
            2_048 => &mut self.sector_2048,
            4_096 => &mut self.sector_4096,
            other => return Err(Usb0MscMediaWorkerError::UnsupportedBlockSize(other)),
        };
        slot.take()
            .ok_or(Usb0MscMediaWorkerError::ScratchUnavailable(block_size))
    }

    fn restore(
        &mut self,
        scratch: &'static mut [u8],
    ) -> Result<(), Usb0MscMediaWorkerError> {
        let block_size = scratch.len() as u32;
        let slot = match block_size {
            512 => &mut self.sector_512,
            1_024 => &mut self.sector_1024,
            2_048 => &mut self.sector_2048,
            4_096 => &mut self.sector_4096,
            other => return Err(Usb0MscMediaWorkerError::UnsupportedBlockSize(other)),
        };
        if slot.is_some() {
            return Err(Usb0MscMediaWorkerError::ScratchUnavailable(block_size));
        }
        *slot = Some(scratch);
        Ok(())
    }
}

struct Usb0FilesystemScratchPool {
    sector_512: Option<&'static mut [u8]>,
    sector_1024: Option<&'static mut [u8]>,
    sector_2048: Option<&'static mut [u8]>,
    sector_4096: Option<&'static mut [u8]>,
}

impl Usb0FilesystemScratchPool {
    fn new() -> Self {
        Self {
            sector_512: Some(&mut USB0_FS_SCRATCH_512.init([0; 512])[..]),
            sector_1024: Some(&mut USB0_FS_SCRATCH_1024.init([0; 1_024])[..]),
            sector_2048: Some(&mut USB0_FS_SCRATCH_2048.init([0; 2_048])[..]),
            sector_4096: Some(&mut USB0_FS_SCRATCH_4096.init([0; 4_096])[..]),
        }
    }

    fn take(
        &mut self,
        block_size: u32,
    ) -> Result<&'static mut [u8], Usb0MscFilesystemProbeError> {
        let slot = match block_size {
            512 => &mut self.sector_512,
            1_024 => &mut self.sector_1024,
            2_048 => &mut self.sector_2048,
            4_096 => &mut self.sector_4096,
            other => return Err(Usb0MscFilesystemProbeError::UnsupportedBlockSize(other)),
        };
        slot.take()
            .ok_or(Usb0MscFilesystemProbeError::ScratchUnavailable(block_size))
    }

    fn restore(
        &mut self,
        scratch: &'static mut [u8],
    ) -> Result<(), Usb0MscFilesystemProbeError> {
        let block_size = scratch.len() as u32;
        let slot = match block_size {
            512 => &mut self.sector_512,
            1_024 => &mut self.sector_1024,
            2_048 => &mut self.sector_2048,
            4_096 => &mut self.sector_4096,
            other => return Err(Usb0MscFilesystemProbeError::UnsupportedBlockSize(other)),
        };
        if slot.is_some() {
            return Err(Usb0MscFilesystemProbeError::ScratchUnavailable(block_size));
        }
        *slot = Some(scratch);
        Ok(())
    }
}

fn filesystem_kind_from_boot(block: &[u8]) -> Option<FileSystemKind> {
    match classify_boot_sector(block) {
        VolumeKind::Fat => Some(FileSystemKind::Fat32),
        VolumeKind::ExFat => Some(FileSystemKind::ExFat),
        VolumeKind::Unknown => None,
    }
}

async fn probe_mounted_filesystem(
    media: UsbMscMountedMedia,
    scratch_pool: &mut Usb0FilesystemScratchPool,
) -> Result<
    (
        FileSystemKind,
        AsyncPartitionDevice<Usb0MscAsyncBlockDevice>,
    ),
    Usb0MscFilesystemProbeError,
> {
    let block_size = media.attempt.geometry.block_size;
    let scratch = scratch_pool.take(block_size)?;
    let raw = Usb0MscAsyncBlockDevice::new(media);
    let mut partition = AsyncPartitionDevice::new(raw, media.selection.range);

    let scratch = match partition.read_blocks_owned(0, 1, scratch).await {
        Ok(scratch) => scratch,
        Err((error, scratch)) => {
            if scratch_pool.restore(scratch).is_err() {
                esp_hal::system::software_reset();
            }
            return Err(Usb0MscFilesystemProbeError::Io(error));
        }
    };

    let observed = filesystem_kind_from_boot(scratch)
        .ok_or(Usb0MscFilesystemProbeError::UnknownVolume);
    if scratch_pool.restore(scratch).is_err() {
        esp_hal::system::software_reset();
    }
    let observed = observed?;
    if observed != media.selection.filesystem {
        return Err(Usb0MscFilesystemProbeError::KindMismatch {
            expected: media.selection.filesystem,
            observed,
        });
    }

    Ok((observed, partition))
}

pub(crate) async fn run_usb0_filesystem_worker() -> ! {
    let mut scratch_pool = Usb0FilesystemScratchPool::new();
    let mut active:
        Option<(UsbMscMountedMedia, AsyncPartitionDevice<Usb0MscAsyncBlockDevice>)> = None;

    loop {
        match USB0_MSC_FILESYSTEM_EVENTS.receive().await {
            Usb0MscFilesystemEvent::Mounted { binding, media } => {
                set_filesystem_state(Usb0MscFilesystemState::Probing { binding, media });
                match probe_mounted_filesystem(media, &mut scratch_pool).await {
                    Ok((filesystem, partition)) => {
                        active = Some((media, partition));
                        set_filesystem_state(Usb0MscFilesystemState::VolumeProbed {
                            binding,
                            media,
                            filesystem,
                        });
                    }
                    Err(error) => {
                        active = None;
                        set_filesystem_state(Usb0MscFilesystemState::Failed {
                            binding,
                            media,
                            error,
                        });
                    }
                }
            }
            Usb0MscFilesystemEvent::Detached { binding } => {
                if active
                    .as_ref()
                    .is_some_and(|(media, _)| media.attempt.handle == binding.handle)
                {
                    active = None;
                }
                set_filesystem_state(Usb0MscFilesystemState::Detached { binding });
            }
        }
    }
}

/// Persistent USB0 media worker.
///
/// This task consumes owner lifecycle events, performs partition discovery only
/// through the bounded request/completion queue, and commits a generation-safe
/// mount descriptor. The descriptor is then handed to the separate filesystem
/// worker, which owns the async partition adapter. FAT32/exFAT file operations
/// are still intentionally not mounted through the synchronous backends here.
pub(crate) async fn run_usb0_media_worker() -> ! {
    let mut coordinator = UsbMscMountCoordinator::new();
    let mut scratch_pool = Usb0MscScratchPool::new();

    loop {
        match USB0_MSC_OWNER_EVENTS.receive().await {
            Usb0MscOwnerEvent::Ready { binding, probe } => {
                set_media_state(Usb0MscMediaState::Discovering { binding });

                let attempt = match begin_current_mount(
                    &mut coordinator,
                    binding,
                    0,
                    probe.capacity,
                ) {
                    Ok(attempt) => attempt,
                    Err(error) => {
                        set_media_state(Usb0MscMediaState::Failed {
                            binding,
                            error: Usb0MscMediaWorkerError::Mount(error),
                        });
                        continue;
                    }
                };

                let scratch = match scratch_pool.take(attempt.geometry.block_size) {
                    Ok(scratch) => scratch,
                    Err(error) => {
                        let _ = coordinator.abort(attempt);
                        set_media_state(Usb0MscMediaState::Failed { binding, error });
                        continue;
                    }
                };

                let mut discovery = match Usb0MscDiscoveryWorker::new(attempt, scratch) {
                    Ok(discovery) => discovery,
                    Err((error, scratch)) => {
                        if scratch_pool.restore(scratch).is_err() {
                            esp_hal::system::software_reset();
                        }
                        let _ = coordinator.abort(attempt);
                        set_media_state(Usb0MscMediaState::Failed {
                            binding,
                            error: Usb0MscMediaWorkerError::Discovery(error),
                        });
                        continue;
                    }
                };

                let selection = discovery.run_to_selection(&coordinator).await;
                let Some(scratch) = discovery.into_scratch() else {
                    esp_hal::system::software_reset();
                };
                if scratch_pool.restore(scratch).is_err() {
                    esp_hal::system::software_reset();
                }

                match selection {
                    Ok(selection) => match commit_current_mount(
                        &mut coordinator,
                        attempt,
                        selection,
                    ) {
                        Ok(media) => {
                            set_media_state(Usb0MscMediaState::Mounted { binding, media });
                            USB0_MSC_FILESYSTEM_EVENTS
                                .send(Usb0MscFilesystemEvent::Mounted { binding, media })
                                .await;
                        }
                        Err(error) => {
                            let _ = coordinator.abort(attempt);
                            set_media_state(Usb0MscMediaState::Failed {
                                binding,
                                error: Usb0MscMediaWorkerError::Mount(error),
                            });
                        }
                    },
                    Err(error) => {
                        let _ = coordinator.abort(attempt);
                        set_media_state(Usb0MscMediaState::Failed {
                            binding,
                            error: Usb0MscMediaWorkerError::Discovery(error),
                        });
                    }
                }
            }
            Usb0MscOwnerEvent::Detached {
                binding,
                reason,
                result,
            } => {
                let _ = detach_mount(&mut coordinator, binding);
                USB0_MSC_FILESYSTEM_EVENTS
                    .send(Usb0MscFilesystemEvent::Detached { binding })
                    .await;
                set_media_state(Usb0MscMediaState::Detached {
                    binding,
                    reason,
                    result,
                });
            }
        }
    }
}

impl Usb0MscCompletion {
    pub(crate) const fn ticket(&self) -> UsbMscRequestTicket {
        match self {
            Self::Read { ticket, .. }
            | Self::Write { ticket, .. }
            | Self::Flush { ticket, .. } => *ticket,
        }
    }

    pub(crate) const fn status(&self) -> Usb0MscCompletionStatus {
        match self {
            Self::Read { status, .. }
            | Self::Write { status, .. }
            | Self::Flush { status, .. } => *status,
        }
    }

    pub(crate) fn is_current(&self, session: &MediaSession) -> bool {
        self.status() == Usb0MscCompletionStatus::Success
            && UsbMscCompletionGate::accepts(session, self.ticket())
    }
}

pub(crate) fn capacity_from_embassy(capacity: BlockCapacity) -> Option<UsbMscCapacity> {
    UsbMscCapacity::from_block_count(capacity.block_size, capacity.block_count)
}

pub(crate) async fn probe_capacity<'dev, 'd, A>(
    lun: &mut MscLun<'dev, 'd, A>,
) -> Result<UsbMscCapacity, Usb0MscError>
where
    A: UsbHostAllocator<'d>,
{
    let capacity = lun.capacity().await?;
    capacity_from_embassy(capacity).ok_or(Usb0MscError::InvalidCapacity)
}

pub(crate) async fn test_unit_ready<'dev, 'd, A>(
    lun: &mut MscLun<'dev, 'd, A>,
) -> Result<bool, MscError>
where
    A: UsbHostAllocator<'d>,
{
    lun.test_unit_ready().await
}

pub(crate) async fn read_blocks<'dev, 'd, A>(
    lun: &mut MscLun<'dev, 'd, A>,
    lba: u64,
    output: &mut [u8],
) -> Result<(), MscError>
where
    A: UsbHostAllocator<'d>,
{
    lun.read_blocks(lba, output).await
}

pub(crate) async fn write_blocks<'dev, 'd, A>(
    lun: &mut MscLun<'dev, 'd, A>,
    lba: u64,
    input: &[u8],
) -> Result<(), MscError>
where
    A: UsbHostAllocator<'d>,
{
    lun.write_blocks(lba, input).await
}

pub(crate) async fn flush<'dev, 'd, A>(
    lun: &mut MscLun<'dev, 'd, A>,
) -> Result<(), MscError>
where
    A: UsbHostAllocator<'d>,
{
    lun.flush().await
}

/// Execute one bounded queued request on the task that owns the Embassy MSC LUN.
///
/// The dispatch lease is checked before I/O. The returned completion still
/// carries the original request ticket and must be revalidated against the
/// current MediaSession after the await.
pub(crate) async fn owner_step<'dev, 'd, A>(
    lun: &mut MscLun<'dev, 'd, A>,
    lease_at_dispatch: MediaLease,
) where
    A: UsbHostAllocator<'d>,
{
    let request = USB0_MSC_REQUESTS.receive().await;
    let completion = execute_request(lun, lease_at_dispatch, request).await;
    USB0_MSC_COMPLETIONS.send(completion).await;
}

pub(crate) async fn execute_request<'dev, 'd, A>(
    lun: &mut MscLun<'dev, 'd, A>,
    lease_at_dispatch: MediaLease,
    mut request: Usb0MscRequest,
) -> Usb0MscCompletion
where
    A: UsbHostAllocator<'d>,
{
    let status = execute_request_status(lun, lease_at_dispatch, &mut request).await;
    complete_with_status(request, status)
}

async fn execute_request_status<'dev, 'd, A>(
    lun: &mut MscLun<'dev, 'd, A>,
    lease_at_dispatch: MediaLease,
    request: &mut Usb0MscRequest,
) -> Usb0MscCompletionStatus
where
    A: UsbHostAllocator<'d>,
{
    let ticket = request.ticket();
    if ticket.lease != lease_at_dispatch {
        return Usb0MscCompletionStatus::StaleLease;
    }
    if ticket.lun != lun.lun() {
        return Usb0MscCompletionStatus::WrongLun;
    }

    match request {
        Usb0MscRequest::Read { ticket, buffer } => {
            let ticket = *ticket;
            if !buffer_matches_ticket(lun, ticket, buffer.len(), true) {
                return Usb0MscCompletionStatus::InvalidTransfer;
            }
            match lun
                .read_blocks(ticket.first_block().unwrap(), &mut **buffer)
                .await
            {
                Ok(()) => Usb0MscCompletionStatus::Success,
                Err(_) => Usb0MscCompletionStatus::HostError,
            }
        }
        Usb0MscRequest::Write { ticket, buffer } => {
            let ticket = *ticket;
            if !buffer_matches_ticket(lun, ticket, buffer.len(), false) {
                return Usb0MscCompletionStatus::InvalidTransfer;
            }
            match lun
                .write_blocks(ticket.first_block().unwrap(), &**buffer)
                .await
            {
                Ok(()) => Usb0MscCompletionStatus::Success,
                Err(_) => Usb0MscCompletionStatus::HostError,
            }
        }
        Usb0MscRequest::Flush { ticket } => {
            if ticket.kind != UsbMscRequestKind::Flush {
                return Usb0MscCompletionStatus::InvalidTransfer;
            }
            match lun.flush().await {
                Ok(()) => Usb0MscCompletionStatus::Success,
                Err(_) => Usb0MscCompletionStatus::HostError,
            }
        }
    }
}

fn buffer_matches_ticket<'dev, 'd, A>(
    lun: &MscLun<'dev, 'd, A>,
    ticket: UsbMscRequestTicket,
    buffer_len: usize,
    read: bool,
) -> bool
where
    A: UsbHostAllocator<'d>,
{
    let expected_kind = match ticket.kind {
        UsbMscRequestKind::Read { block_count, .. } if read => Some(block_count),
        UsbMscRequestKind::Write { block_count, .. } if !read => Some(block_count),
        _ => None,
    };
    let Some(block_count) = expected_kind else {
        return false;
    };
    if block_count == 0 {
        return false;
    }
    let Some(capacity) = lun.cached_capacity() else {
        return false;
    };
    (capacity.block_size as usize)
        .checked_mul(block_count as usize)
        == Some(buffer_len)
}

fn complete_with_status(
    request: Usb0MscRequest,
    status: Usb0MscCompletionStatus,
) -> Usb0MscCompletion {
    match request {
        Usb0MscRequest::Read { ticket, buffer } => Usb0MscCompletion::Read {
            ticket,
            buffer,
            status,
        },
        Usb0MscRequest::Write { ticket, buffer } => Usb0MscCompletion::Write {
            ticket,
            buffer,
            status,
        },
        Usb0MscRequest::Flush { ticket } => Usb0MscCompletion::Flush { ticket, status },
    }
}
