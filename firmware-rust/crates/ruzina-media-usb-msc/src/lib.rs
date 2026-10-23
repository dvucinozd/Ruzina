#![no_std]
#![forbid(unsafe_code)]

use core::fmt;
use core::num::NonZeroU32;

use ruzina_media_block::{
    BlockDevice, BlockGeometry, BlockRange, TransferError, WritableBlockDevice,
};
use ruzina_media_fs::FileSystemKind;
use ruzina_media_partition::{
    GptEntryStream, PartitionCandidate, PartitionLayout, PartitionScanResult, VolumeKind,
    classify_boot_sector, parse_gpt_header, scan_mbr_or_superfloppy,
};
use ruzina_media_session::{MediaHandle, MediaLease, MediaSession, MediaSourceId};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct UsbMscRequestId(NonZeroU32);

impl UsbMscRequestId {
    pub const fn new(value: u32) -> Option<Self> {
        match NonZeroU32::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsbMscRequestKind {
    Read { first_block: u64, block_count: u32 },
    Write { first_block: u64, block_count: u32 },
    Flush,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsbMscRequestTicket {
    pub id: UsbMscRequestId,
    pub lease: MediaLease,
    pub lun: u8,
    pub kind: UsbMscRequestKind,
}

impl UsbMscRequestTicket {
    pub const fn first_block(self) -> Option<u64> {
        match self.kind {
            UsbMscRequestKind::Read { first_block, .. }
            | UsbMscRequestKind::Write { first_block, .. } => Some(first_block),
            UsbMscRequestKind::Flush => None,
        }
    }

    pub const fn block_count(self) -> Option<u32> {
        match self.kind {
            UsbMscRequestKind::Read { block_count, .. }
            | UsbMscRequestKind::Write { block_count, .. } => Some(block_count),
            UsbMscRequestKind::Flush => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsbMscRequestSequencer {
    next: u32,
}

impl UsbMscRequestSequencer {
    pub const fn new() -> Self {
        Self { next: 1 }
    }

    pub fn issue(
        &mut self,
        lease: MediaLease,
        lun: u8,
        kind: UsbMscRequestKind,
    ) -> UsbMscRequestTicket {
        let id = UsbMscRequestId::new(self.next).expect("request sequence never emits zero");
        self.next = self.next.wrapping_add(1);
        if self.next == 0 {
            self.next = 1;
        }
        UsbMscRequestTicket {
            id,
            lease,
            lun,
            kind,
        }
    }
}

impl Default for UsbMscRequestSequencer {
    fn default() -> Self {
        Self::new()
    }
}

/// Gate applied after an async USB command completes.
///
/// Dispatch-time validation is necessary but insufficient because disconnect
/// and re-enumeration may advance MediaSession while the owner task is awaiting
/// the USB command. A completion is publishable only while its original lease
/// is still the session's current generation/source lease.
pub struct UsbMscCompletionGate;

impl UsbMscCompletionGate {
    pub const fn accepts(session: &MediaSession, ticket: UsbMscRequestTicket) -> bool {
        session.validate(ticket.lease)
    }
}

pub const fn usb_address_source(device_address: u8) -> Option<MediaSourceId> {
    MediaSourceId::new(device_address as u32)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsbMscHandleSequencer {
    next: u64,
}

impl UsbMscHandleSequencer {
    pub const fn new() -> Self {
        Self { next: 1 }
    }

    pub fn issue(&mut self) -> MediaHandle {
        let handle = MediaHandle::new(self.next).expect("handle sequence never emits zero");
        self.next = self.next.wrapping_add(1);
        if self.next == 0 {
            self.next = 1;
        }
        handle
    }
}

impl Default for UsbMscHandleSequencer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsbMscSessionError {
    NoActiveMedia,
    SecondaryDevice,
    HandleRejected,
    StaleLease,
    WrongOwner,
}

pub struct UsbMscSessionBridge {
    session: MediaSession,
    requests: UsbMscRequestSequencer,
}

impl UsbMscSessionBridge {
    pub const fn new() -> Self {
        Self {
            session: MediaSession::new(),
            requests: UsbMscRequestSequencer::new(),
        }
    }

    pub fn on_enumerated(
        &mut self,
        source: ruzina_media_session::MediaSourceId,
        handle: ruzina_media_session::MediaHandle,
    ) -> Result<MediaLease, UsbMscSessionError> {
        let lease = match self.session.on_connect(source) {
            ruzina_media_session::ConnectResult::Accepted(lease)
            | ruzina_media_session::ConnectResult::Duplicate(lease) => lease,
            ruzina_media_session::ConnectResult::IgnoredSecondary(_) => {
                return Err(UsbMscSessionError::SecondaryDevice);
            }
        };

        if !self.session.bind_handle(lease, handle) {
            return Err(UsbMscSessionError::HandleRejected);
        }
        Ok(lease)
    }

    pub fn on_disconnect(
        &mut self,
        handle: ruzina_media_session::MediaHandle,
    ) -> ruzina_media_session::DisconnectResult {
        self.session.on_disconnect(Some(handle))
    }

    pub fn issue(
        &mut self,
        lun: u8,
        kind: UsbMscRequestKind,
    ) -> Result<UsbMscRequestTicket, UsbMscSessionError> {
        let Some(lease) = self.session.lease() else {
            return Err(UsbMscSessionError::NoActiveMedia);
        };
        let Some(handle) = self.session.handle() else {
            return Err(UsbMscSessionError::NoActiveMedia);
        };
        self.issue_for(lease, handle, lun, kind)
    }

    pub fn issue_for(
        &mut self,
        lease: MediaLease,
        handle: MediaHandle,
        lun: u8,
        kind: UsbMscRequestKind,
    ) -> Result<UsbMscRequestTicket, UsbMscSessionError> {
        if !self.session.validate(lease) {
            return Err(UsbMscSessionError::StaleLease);
        }
        if self.session.handle() != Some(handle) {
            return Err(UsbMscSessionError::WrongOwner);
        }
        Ok(self.requests.issue(lease, lun, kind))
    }

    pub const fn session(&self) -> &MediaSession {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut MediaSession {
        &mut self.session
    }

    pub const fn lease(&self) -> Option<MediaLease> {
        self.session.lease()
    }

    pub const fn accepts_completion(&self, ticket: UsbMscRequestTicket) -> bool {
        UsbMscCompletionGate::accepts(&self.session, ticket)
    }
}

impl Default for UsbMscSessionBridge {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsbMscMountAttempt {
    pub lease: MediaLease,
    pub handle: MediaHandle,
    pub lun: u8,
    pub geometry: BlockGeometry,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsbMscMountSelection {
    pub range: BlockRange,
    pub filesystem: FileSystemKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsbMscMountedMedia {
    pub attempt: UsbMscMountAttempt,
    pub selection: UsbMscMountSelection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsbMscMountError {
    StaleLease,
    WrongOwner,
    InvalidGeometry,
    NoActiveAttempt,
    StaleAttempt,
    UnsupportedFilesystem,
    PartitionOutOfRange,
    SessionRejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsbMscMountCoordinator {
    active: Option<UsbMscMountAttempt>,
    mounted: Option<UsbMscMountedMedia>,
}

impl UsbMscMountCoordinator {
    pub const fn new() -> Self {
        Self {
            active: None,
            mounted: None,
        }
    }

    pub fn begin(
        &mut self,
        session: &MediaSession,
        lease: MediaLease,
        handle: MediaHandle,
        lun: u8,
        capacity: UsbMscCapacity,
    ) -> Result<UsbMscMountAttempt, UsbMscMountError> {
        if !session.validate(lease) {
            return Err(UsbMscMountError::StaleLease);
        }
        if session.handle() != Some(handle) {
            return Err(UsbMscMountError::WrongOwner);
        }

        let geometry = capacity.geometry();
        if !geometry.is_valid() {
            return Err(UsbMscMountError::InvalidGeometry);
        }

        let attempt = UsbMscMountAttempt {
            lease,
            handle,
            lun,
            geometry,
        };
        self.active = Some(attempt);
        self.mounted = None;
        Ok(attempt)
    }

    pub fn select_partition(
        &self,
        attempt: UsbMscMountAttempt,
        candidate: PartitionCandidate,
        boot_sector: &[u8],
    ) -> Result<UsbMscMountSelection, UsbMscMountError> {
        if self.active != Some(attempt) {
            return Err(UsbMscMountError::StaleAttempt);
        }

        let filesystem = match candidate.kind {
            VolumeKind::Fat => FileSystemKind::Fat32,
            VolumeKind::ExFat => FileSystemKind::ExFat,
            VolumeKind::Unknown => match classify_boot_sector(boot_sector) {
                VolumeKind::Fat => FileSystemKind::Fat32,
                VolumeKind::ExFat => FileSystemKind::ExFat,
                VolumeKind::Unknown => return Err(UsbMscMountError::UnsupportedFilesystem),
            },
        };

        if candidate.first_lba >= attempt.geometry.block_count {
            return Err(UsbMscMountError::PartitionOutOfRange);
        }

        let available = attempt.geometry.block_count - candidate.first_lba;
        let block_count = candidate
            .sector_count
            .map(core::num::NonZeroU64::get)
            .unwrap_or(available);
        let Some(nonzero_count) = core::num::NonZeroU64::new(block_count) else {
            return Err(UsbMscMountError::PartitionOutOfRange);
        };
        let Some(range) = BlockRange::new(candidate.first_lba, nonzero_count, attempt.geometry)
        else {
            return Err(UsbMscMountError::PartitionOutOfRange);
        };

        Ok(UsbMscMountSelection { range, filesystem })
    }

    pub fn commit(
        &mut self,
        session: &mut MediaSession,
        attempt: UsbMscMountAttempt,
        selection: UsbMscMountSelection,
    ) -> Result<UsbMscMountedMedia, UsbMscMountError> {
        let Some(active) = self.active else {
            return Err(UsbMscMountError::NoActiveAttempt);
        };
        if active != attempt {
            return Err(UsbMscMountError::StaleAttempt);
        }
        if !session.validate(attempt.lease) {
            return Err(UsbMscMountError::StaleLease);
        }
        if session.handle() != Some(attempt.handle) {
            return Err(UsbMscMountError::WrongOwner);
        }
        if !session.commit_mounted(attempt.lease) {
            return Err(UsbMscMountError::SessionRejected);
        }

        let mounted = UsbMscMountedMedia { attempt, selection };
        self.active = None;
        self.mounted = Some(mounted);
        Ok(mounted)
    }

    pub fn abort(&mut self, attempt: UsbMscMountAttempt) -> bool {
        if self.active != Some(attempt) {
            return false;
        }
        self.active = None;
        true
    }

    pub fn on_detached(&mut self, handle: MediaHandle) -> bool {
        let owns_active = self.active.is_some_and(|attempt| attempt.handle == handle);
        let owns_mounted = self
            .mounted
            .is_some_and(|mounted| mounted.attempt.handle == handle);
        if !owns_active && !owns_mounted {
            return false;
        }

        self.active = None;
        self.mounted = None;
        true
    }

    pub const fn active(&self) -> Option<UsbMscMountAttempt> {
        self.active
    }

    pub const fn mounted(&self) -> Option<UsbMscMountedMedia> {
        self.mounted
    }
}

impl Default for UsbMscMountCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsbMscDiscoveryPlanError {
    UnsupportedBlockSize(u32),
    BlockTooSmall { required: usize, actual: usize },
    UnexpectedLba { expected: u64, actual: u64 },
    AlreadySelected,
    InvalidPartitionTable,
    GptTableOutOfRange,
    NoSupportedFilesystem,
    Mount(UsbMscMountError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UsbMscDiscoveryStage {
    Root,
    GptHeader,
    GptEntries { next_lba: u64 },
    Candidate { index: usize },
    Selected(UsbMscMountSelection),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsbMscDiscoveryPlan {
    attempt: UsbMscMountAttempt,
    layout: PartitionLayout,
    gpt_stream: Option<GptEntryStream>,
    stage: UsbMscDiscoveryStage,
}

impl UsbMscDiscoveryPlan {
    pub fn new(attempt: UsbMscMountAttempt) -> Result<Self, UsbMscDiscoveryPlanError> {
        if attempt.geometry.block_size < ruzina_media_partition::MIN_SECTOR_SIZE as u32 {
            return Err(UsbMscDiscoveryPlanError::UnsupportedBlockSize(
                attempt.geometry.block_size,
            ));
        }

        Ok(Self {
            attempt,
            layout: PartitionLayout::new(),
            gpt_stream: None,
            stage: UsbMscDiscoveryStage::Root,
        })
    }

    pub const fn attempt(&self) -> UsbMscMountAttempt {
        self.attempt
    }

    pub fn layout(&self) -> &PartitionLayout {
        &self.layout
    }

    pub fn selection(&self) -> Option<UsbMscMountSelection> {
        match &self.stage {
            UsbMscDiscoveryStage::Selected(selection) => Some(*selection),
            _ => None,
        }
    }

    pub fn next_lba(&mut self) -> Result<Option<u64>, UsbMscDiscoveryPlanError> {
        loop {
            match &self.stage {
                UsbMscDiscoveryStage::Root => return Ok(Some(0)),
                UsbMscDiscoveryStage::GptHeader => return Ok(Some(1)),
                UsbMscDiscoveryStage::GptEntries { next_lba } => {
                    if *next_lba >= self.attempt.geometry.block_count {
                        return Err(UsbMscDiscoveryPlanError::GptTableOutOfRange);
                    }
                    return Ok(Some(*next_lba));
                }
                UsbMscDiscoveryStage::Candidate { index } => {
                    let index = *index;
                    let Some(candidate) = self.layout.candidates().get(index).copied() else {
                        return Err(UsbMscDiscoveryPlanError::NoSupportedFilesystem);
                    };
                    if candidate.first_lba >= self.attempt.geometry.block_count {
                        self.stage = UsbMscDiscoveryStage::Candidate { index: index + 1 };
                        continue;
                    }
                    return Ok(Some(candidate.first_lba));
                }
                UsbMscDiscoveryStage::Selected(_) => return Ok(None),
            }
        }
    }

    pub fn ingest_block(
        &mut self,
        coordinator: &UsbMscMountCoordinator,
        lba: u64,
        block: &[u8],
    ) -> Result<Option<UsbMscMountSelection>, UsbMscDiscoveryPlanError> {
        let required = self.attempt.geometry.block_size as usize;
        if block.len() < required {
            return Err(UsbMscDiscoveryPlanError::BlockTooSmall {
                required,
                actual: block.len(),
            });
        }
        let block = &block[..required];

        let Some(expected) = self.next_lba()? else {
            return Err(UsbMscDiscoveryPlanError::AlreadySelected);
        };
        if lba != expected {
            return Err(UsbMscDiscoveryPlanError::UnexpectedLba {
                expected,
                actual: lba,
            });
        }

        match &mut self.stage {
            UsbMscDiscoveryStage::Root => match scan_mbr_or_superfloppy(block, &mut self.layout) {
                PartitionScanResult::Ok => {
                    self.stage = UsbMscDiscoveryStage::Candidate { index: 0 };
                }
                PartitionScanResult::NeedsGpt => {
                    if self.attempt.geometry.block_count <= 1 {
                        return Err(UsbMscDiscoveryPlanError::GptTableOutOfRange);
                    }
                    self.layout.clear();
                    self.stage = UsbMscDiscoveryStage::GptHeader;
                }
                PartitionScanResult::Invalid => {
                    return Err(UsbMscDiscoveryPlanError::InvalidPartitionTable);
                }
                PartitionScanResult::NoCandidate => {
                    return Err(UsbMscDiscoveryPlanError::NoSupportedFilesystem);
                }
            },
            UsbMscDiscoveryStage::GptHeader => {
                let info = parse_gpt_header(block)
                    .ok_or(UsbMscDiscoveryPlanError::InvalidPartitionTable)?;
                if info.entries_lba >= self.attempt.geometry.block_count {
                    return Err(UsbMscDiscoveryPlanError::GptTableOutOfRange);
                }
                let stream = GptEntryStream::new(info)
                    .ok_or(UsbMscDiscoveryPlanError::InvalidPartitionTable)?;
                self.layout.clear();
                self.gpt_stream = Some(stream);
                self.stage = UsbMscDiscoveryStage::GptEntries {
                    next_lba: info.entries_lba,
                };
            }
            UsbMscDiscoveryStage::GptEntries { next_lba } => {
                let stream = self
                    .gpt_stream
                    .as_mut()
                    .ok_or(UsbMscDiscoveryPlanError::InvalidPartitionTable)?;
                let _ = stream.push(block, &mut self.layout);
                if stream.is_complete() {
                    self.gpt_stream = None;
                    self.stage = UsbMscDiscoveryStage::Candidate { index: 0 };
                } else {
                    self.stage = UsbMscDiscoveryStage::GptEntries {
                        next_lba: next_lba
                            .checked_add(1)
                            .ok_or(UsbMscDiscoveryPlanError::GptTableOutOfRange)?,
                    };
                }
            }
            UsbMscDiscoveryStage::Candidate { index } => {
                let candidate = self.layout.candidates()[*index];
                let boot_kind = classify_boot_sector(block);
                if boot_kind == VolumeKind::Unknown {
                    *index += 1;
                    return Ok(None);
                }

                let verified = PartitionCandidate {
                    first_lba: candidate.first_lba,
                    sector_count: candidate.sector_count,
                    kind: boot_kind,
                };

                match coordinator.select_partition(self.attempt, verified, block) {
                    Ok(selection) => {
                        self.stage = UsbMscDiscoveryStage::Selected(selection);
                        return Ok(Some(selection));
                    }
                    Err(
                        UsbMscMountError::UnsupportedFilesystem
                        | UsbMscMountError::PartitionOutOfRange,
                    ) => {
                        *index += 1;
                    }
                    Err(error) => return Err(UsbMscDiscoveryPlanError::Mount(error)),
                }
            }
            UsbMscDiscoveryStage::Selected(_) => {
                return Err(UsbMscDiscoveryPlanError::AlreadySelected);
            }
        }

        Ok(None)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum UsbMscDiscoveryError<E> {
    GeometryMismatch,
    ScratchTooSmall { required: usize, actual: usize },
    Read(E),
    Plan(UsbMscDiscoveryPlanError),
}

impl<E> From<UsbMscDiscoveryPlanError> for UsbMscDiscoveryError<E> {
    fn from(error: UsbMscDiscoveryPlanError) -> Self {
        Self::Plan(error)
    }
}

pub fn discover_mount_selection<D: BlockDevice>(
    device: &mut D,
    coordinator: &UsbMscMountCoordinator,
    attempt: UsbMscMountAttempt,
    scratch: &mut [u8],
) -> Result<UsbMscMountSelection, UsbMscDiscoveryError<D::Error>> {
    let geometry = device.geometry();
    if geometry != attempt.geometry {
        return Err(UsbMscDiscoveryError::GeometryMismatch);
    }

    let block_size = geometry.block_size as usize;
    if scratch.len() < block_size {
        return Err(UsbMscDiscoveryError::ScratchTooSmall {
            required: block_size,
            actual: scratch.len(),
        });
    }

    let mut plan = UsbMscDiscoveryPlan::new(attempt)?;
    let block = &mut scratch[..block_size];

    loop {
        let Some(lba) = plan.next_lba()? else {
            return plan
                .selection()
                .ok_or(UsbMscDiscoveryPlanError::NoSupportedFilesystem.into());
        };
        device
            .read_blocks(lba, 1, block)
            .map_err(UsbMscDiscoveryError::Read)?;
        if let Some(selection) = plan.ingest_block(coordinator, lba, block)? {
            return Ok(selection);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsbMscCapacity {
    pub block_size: u32,
    pub block_count: u64,
}

impl UsbMscCapacity {
    pub const fn from_block_count(block_size: u32, block_count: u64) -> Option<Self> {
        if block_size == 0 || block_count == 0 {
            None
        } else {
            Some(Self {
                block_size,
                block_count,
            })
        }
    }

    /// Convert SCSI READ CAPACITY's last logical block address into a count.
    pub const fn from_last_lba(block_size: u32, last_lba: u64) -> Option<Self> {
        match last_lba.checked_add(1) {
            Some(block_count) => Self::from_block_count(block_size, block_count),
            None => None,
        }
    }

    pub const fn geometry(self) -> BlockGeometry {
        BlockGeometry {
            block_size: self.block_size,
            block_count: self.block_count,
        }
    }
}

/// Completed USB mass-storage command transport.
///
/// The ESP32-P4 binding may be driven by an async USB host task. This trait is
/// intentionally the synchronous completion boundary consumed by the media
/// worker; USB endpoint/DMA ownership must not leak into filesystem code.
pub trait UsbMscTransport {
    type Error;

    fn read_blocks(
        &mut self,
        lun: u8,
        first_block: u64,
        block_count: u32,
        output: &mut [u8],
    ) -> Result<(), Self::Error>;
}

pub trait WritableUsbMscTransport: UsbMscTransport {
    fn write_blocks(
        &mut self,
        lun: u8,
        first_block: u64,
        block_count: u32,
        input: &[u8],
    ) -> Result<(), Self::Error>;

    fn synchronize_cache(&mut self, lun: u8) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsbMscConfigError {
    InvalidGeometry,
}

impl fmt::Display for UsbMscConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGeometry => formatter.write_str("invalid USB MSC block geometry"),
        }
    }
}

impl core::error::Error for UsbMscConfigError {}

#[derive(Debug, Eq, PartialEq)]
pub enum UsbMscError<E> {
    Disconnected,
    TooManyBlocks { requested: u32, maximum: u32 },
    Transfer(TransferError),
    Backend(E),
}

impl<E: fmt::Display> fmt::Display for UsbMscError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disconnected => formatter.write_str("USB mass-storage device disconnected"),
            Self::TooManyBlocks { requested, maximum } => {
                write!(
                    formatter,
                    "USB MSC transfer requests {requested} blocks; maximum is {maximum}"
                )
            }
            Self::Transfer(error) => {
                write!(formatter, "USB MSC transfer contract error: {error:?}")
            }
            Self::Backend(error) => write!(formatter, "USB MSC transport error: {error}"),
        }
    }
}

impl<E> core::error::Error for UsbMscError<E> where E: core::error::Error + 'static {}

/// Generation-local USB mass-storage block adapter.
///
/// Once `disconnect` is called this adapter is terminal. Re-enumeration must
/// construct a new adapter so stale filesystem owners cannot silently become
/// valid against a different device that reused the same USB address.
pub struct UsbMscBlockDevice<T> {
    transport: T,
    lun: u8,
    geometry: BlockGeometry,
    max_blocks_per_transfer: NonZeroU32,
    connected: bool,
}

impl<T> UsbMscBlockDevice<T> {
    pub fn new(
        transport: T,
        lun: u8,
        capacity: UsbMscCapacity,
        max_blocks_per_transfer: NonZeroU32,
    ) -> Result<Self, UsbMscConfigError> {
        let geometry = capacity.geometry();
        if !geometry.is_valid() {
            return Err(UsbMscConfigError::InvalidGeometry);
        }

        Ok(Self {
            transport,
            lun,
            geometry,
            max_blocks_per_transfer,
            connected: true,
        })
    }

    pub const fn lun(&self) -> u8 {
        self.lun
    }

    pub const fn is_connected(&self) -> bool {
        self.connected
    }

    pub const fn max_blocks_per_transfer(&self) -> NonZeroU32 {
        self.max_blocks_per_transfer
    }

    pub fn disconnect(&mut self) {
        self.connected = false;
    }

    pub const fn transport(&self) -> &T {
        &self.transport
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    pub fn into_transport(self) -> T {
        self.transport
    }

    fn validate_transfer<E>(
        &self,
        first_block: u64,
        block_count: u32,
        buffer_len: usize,
    ) -> Result<(), UsbMscError<E>> {
        if !self.connected {
            return Err(UsbMscError::Disconnected);
        }
        if block_count > self.max_blocks_per_transfer.get() {
            return Err(UsbMscError::TooManyBlocks {
                requested: block_count,
                maximum: self.max_blocks_per_transfer.get(),
            });
        }

        self.geometry
            .validate_transfer(first_block, block_count, buffer_len)
            .map_err(UsbMscError::Transfer)
    }
}

impl<T: UsbMscTransport> BlockDevice for UsbMscBlockDevice<T> {
    type Error = UsbMscError<T::Error>;

    fn geometry(&self) -> BlockGeometry {
        self.geometry
    }

    fn read_blocks(
        &mut self,
        first_block: u64,
        block_count: u32,
        output: &mut [u8],
    ) -> Result<(), Self::Error> {
        self.validate_transfer(first_block, block_count, output.len())?;
        self.transport
            .read_blocks(self.lun, first_block, block_count, output)
            .map_err(UsbMscError::Backend)
    }
}

impl<T: WritableUsbMscTransport> WritableBlockDevice for UsbMscBlockDevice<T> {
    fn write_blocks(
        &mut self,
        first_block: u64,
        block_count: u32,
        input: &[u8],
    ) -> Result<(), Self::Error> {
        self.validate_transfer(first_block, block_count, input.len())?;
        self.transport
            .write_blocks(self.lun, first_block, block_count, input)
            .map_err(UsbMscError::Backend)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        if !self.connected {
            return Err(UsbMscError::Disconnected);
        }
        self.transport
            .synchronize_cache(self.lun)
            .map_err(UsbMscError::Backend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mount_session(
        source_value: u32,
        handle_value: u64,
    ) -> (MediaSession, MediaLease, MediaHandle) {
        let mut session = MediaSession::new();
        let source = MediaSourceId::new(source_value).unwrap();
        let lease = match session.on_connect(source) {
            ruzina_media_session::ConnectResult::Accepted(lease) => lease,
            other => panic!("unexpected connect result: {other:?}"),
        };
        let handle = MediaHandle::new(handle_value).unwrap();
        assert!(session.bind_handle(lease, handle));
        (session, lease, handle)
    }

    fn fat32_boot_sector() -> [u8; 512] {
        let mut sector = [0u8; 512];
        sector[0] = 0xeb;
        sector[82..87].copy_from_slice(b"FAT32");
        sector[510] = 0x55;
        sector[511] = 0xaa;
        sector
    }

    fn exfat_boot_sector() -> [u8; 512] {
        let mut sector = [0u8; 512];
        sector[0] = 0xeb;
        sector[3..11].copy_from_slice(b"EXFAT   ");
        sector[510] = 0x55;
        sector[511] = 0xaa;
        sector
    }

    struct MemoryBlockDevice {
        bytes: [u8; 512 * 64],
        geometry: BlockGeometry,
        read_calls: u32,
    }

    impl MemoryBlockDevice {
        fn new(block_size: u32, block_count: u64) -> Self {
            assert!((block_size as u64) * block_count <= (512 * 64) as u64);
            Self {
                bytes: [0u8; 512 * 64],
                geometry: BlockGeometry {
                    block_size,
                    block_count,
                },
                read_calls: 0,
            }
        }

        fn block_mut(&mut self, lba: u64) -> &mut [u8] {
            let size = self.geometry.block_size as usize;
            let start = lba as usize * size;
            &mut self.bytes[start..start + size]
        }

        fn byte_range_mut(&mut self, start: usize, len: usize) -> &mut [u8] {
            &mut self.bytes[start..start + len]
        }
    }

    impl BlockDevice for MemoryBlockDevice {
        type Error = TestError;

        fn geometry(&self) -> BlockGeometry {
            self.geometry
        }

        fn read_blocks(
            &mut self,
            first_block: u64,
            block_count: u32,
            output: &mut [u8],
        ) -> Result<(), Self::Error> {
            self.geometry
                .validate_transfer(first_block, block_count, output.len())
                .map_err(|_| TestError::Failed)?;
            self.read_calls += 1;
            let size = self.geometry.block_size as usize;
            let start = first_block as usize * size;
            output.copy_from_slice(&self.bytes[start..start + output.len()]);
            Ok(())
        }
    }

    fn write_mbr_partition(
        block: &mut [u8],
        partition_type: u8,
        first_lba: u32,
        sector_count: u32,
    ) {
        block[510] = 0x55;
        block[511] = 0xaa;
        block[446 + 4] = partition_type;
        block[446 + 8..446 + 12].copy_from_slice(&first_lba.to_le_bytes());
        block[446 + 12..446 + 16].copy_from_slice(&sector_count.to_le_bytes());
    }

    fn write_protective_mbr(block: &mut [u8]) {
        write_mbr_partition(block, 0xee, 1, u32::MAX);
    }

    fn write_gpt_header(block: &mut [u8], entries_lba: u64, entry_count: u32, entry_size: u32) {
        block[..8].copy_from_slice(b"EFI PART");
        block[12..16].copy_from_slice(&92u32.to_le_bytes());
        block[72..80].copy_from_slice(&entries_lba.to_le_bytes());
        block[80..84].copy_from_slice(&entry_count.to_le_bytes());
        block[84..88].copy_from_slice(&entry_size.to_le_bytes());
    }

    fn crc32(input: &[u8]) -> u32 {
        let mut crc = u32::MAX;
        for &byte in input {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                let mask = 0u32.wrapping_sub(crc & 1);
                crc = (crc >> 1) ^ (0xedb8_8320 & mask);
            }
        }
        !crc
    }

    fn finalize_gpt_integrity(
        device: &mut MemoryBlockDevice,
        header_lba: u64,
        entries_lba: u64,
        entry_count: u32,
        entry_size: u32,
    ) {
        let block_size = device.geometry.block_size as usize;
        let table_start = entries_lba as usize * block_size;
        let table_len = entry_count as usize * entry_size as usize;
        let entries_crc = crc32(&device.bytes[table_start..table_start + table_len]);

        let header = device.block_mut(header_lba);
        header[88..92].copy_from_slice(&entries_crc.to_le_bytes());
        header[16..20].fill(0);
        let header_crc = crc32(&header[..92]);
        header[16..20].copy_from_slice(&header_crc.to_le_bytes());
    }

    fn write_gpt_basic_data_entry(entry: &mut [u8], first_lba: u64, last_lba: u64) {
        const BASIC_DATA_GUID: [u8; 16] = [
            0xa2, 0xa0, 0xd0, 0xeb, 0xe5, 0xb9, 0x33, 0x44, 0x87, 0xc0, 0x68, 0xb6, 0xb7, 0x26,
            0x99, 0xc7,
        ];
        entry[..16].copy_from_slice(&BASIC_DATA_GUID);
        entry[32..40].copy_from_slice(&first_lba.to_le_bytes());
        entry[40..48].copy_from_slice(&last_lba.to_le_bytes());
    }

    #[test]
    fn discovery_plan_drives_async_friendly_gpt_read_sequence() {
        let (session, lease, handle) = mount_session(25, 250);
        let capacity = UsbMscCapacity::from_block_count(512, 64).unwrap();
        let mut coordinator = UsbMscMountCoordinator::new();
        let attempt = coordinator
            .begin(&session, lease, handle, 0, capacity)
            .unwrap();

        let mut image = MemoryBlockDevice::new(512, 64);
        write_protective_mbr(image.block_mut(0));
        write_gpt_header(image.block_mut(1), 2, 4, 136);
        let entry_start = 2 * 512 + 3 * 136;
        write_gpt_basic_data_entry(image.byte_range_mut(entry_start, 136), 20, 31);
        finalize_gpt_integrity(&mut image, 1, 2, 4, 136);
        image.block_mut(20).copy_from_slice(&fat32_boot_sector());

        let mut plan = UsbMscDiscoveryPlan::new(attempt).unwrap();
        let mut observed = [u64::MAX; 8];
        let mut observed_len = 0usize;
        let mut block = [0u8; 512];
        let selection = loop {
            let lba = plan.next_lba().unwrap().unwrap();
            observed[observed_len] = lba;
            observed_len += 1;
            block.copy_from_slice(image.block_mut(lba));
            if let Some(selection) = plan.ingest_block(&coordinator, lba, &block).unwrap() {
                break selection;
            }
        };

        assert_eq!(&observed[..observed_len], &[0, 1, 2, 3, 20]);
        assert_eq!(selection.filesystem, FileSystemKind::Fat32);
        assert_eq!(selection.range.first_block(), 20);
        assert_eq!(selection.range.block_count().get(), 12);
        assert_eq!(plan.selection(), Some(selection));
        assert_eq!(plan.next_lba().unwrap(), None);
    }

    #[test]
    fn discovery_plan_rejects_out_of_order_and_short_blocks() {
        let (session, lease, handle) = mount_session(26, 260);
        let capacity = UsbMscCapacity::from_block_count(512, 64).unwrap();
        let mut coordinator = UsbMscMountCoordinator::new();
        let attempt = coordinator
            .begin(&session, lease, handle, 0, capacity)
            .unwrap();
        let mut plan = UsbMscDiscoveryPlan::new(attempt).unwrap();

        assert_eq!(
            plan.ingest_block(&coordinator, 1, &[0u8; 512]),
            Err(UsbMscDiscoveryPlanError::UnexpectedLba {
                expected: 0,
                actual: 1,
            })
        );
        assert_eq!(
            plan.ingest_block(&coordinator, 0, &[0u8; 511]),
            Err(UsbMscDiscoveryPlanError::BlockTooSmall {
                required: 512,
                actual: 511,
            })
        );
    }

    #[test]
    fn discovery_mounts_4k_superfloppy_exfat_with_one_block_scratch() {
        let (session, lease, handle) = mount_session(21, 210);
        let capacity = UsbMscCapacity::from_block_count(4_096, 8).unwrap();
        let mut coordinator = UsbMscMountCoordinator::new();
        let attempt = coordinator
            .begin(&session, lease, handle, 0, capacity)
            .unwrap();

        let mut device = MemoryBlockDevice::new(4_096, 8);
        device.block_mut(0)[..512].copy_from_slice(&exfat_boot_sector());
        let mut scratch = [0u8; 4_096];

        let selection =
            discover_mount_selection(&mut device, &coordinator, attempt, &mut scratch).unwrap();
        assert_eq!(selection.filesystem, FileSystemKind::ExFat);
        assert_eq!(selection.range.first_block(), 0);
        assert_eq!(selection.range.block_count().get(), 8);
        assert_eq!(device.read_calls, 2);
    }

    #[test]
    fn discovery_mounts_mbr_fat32_candidate_after_boot_validation() {
        let (session, lease, handle) = mount_session(22, 220);
        let capacity = UsbMscCapacity::from_block_count(512, 64).unwrap();
        let mut coordinator = UsbMscMountCoordinator::new();
        let attempt = coordinator
            .begin(&session, lease, handle, 0, capacity)
            .unwrap();

        let mut device = MemoryBlockDevice::new(512, 64);
        write_mbr_partition(device.block_mut(0), 0x0c, 8, 24);
        device.block_mut(8).copy_from_slice(&fat32_boot_sector());
        let mut scratch = [0u8; 512];

        let selection =
            discover_mount_selection(&mut device, &coordinator, attempt, &mut scratch).unwrap();
        assert_eq!(selection.filesystem, FileSystemKind::Fat32);
        assert_eq!(selection.range.first_block(), 8);
        assert_eq!(selection.range.block_count().get(), 24);
        assert_eq!(device.read_calls, 2);
    }

    #[test]
    fn discovery_streams_gpt_entry_that_crosses_block_boundary() {
        let (session, lease, handle) = mount_session(23, 230);
        let capacity = UsbMscCapacity::from_block_count(512, 64).unwrap();
        let mut coordinator = UsbMscMountCoordinator::new();
        let attempt = coordinator
            .begin(&session, lease, handle, 0, capacity)
            .unwrap();

        let mut device = MemoryBlockDevice::new(512, 64);
        write_protective_mbr(device.block_mut(0));
        write_gpt_header(device.block_mut(1), 2, 4, 136);

        let entry_start = 2 * 512 + 3 * 136;
        write_gpt_basic_data_entry(device.byte_range_mut(entry_start, 136), 20, 31);
        finalize_gpt_integrity(&mut device, 1, 2, 4, 136);
        device.block_mut(20).copy_from_slice(&fat32_boot_sector());

        let mut scratch = [0u8; 512];
        let selection =
            discover_mount_selection(&mut device, &coordinator, attempt, &mut scratch).unwrap();

        assert_eq!(selection.filesystem, FileSystemKind::Fat32);
        assert_eq!(selection.range.first_block(), 20);
        assert_eq!(selection.range.block_count().get(), 12);
        assert!(device.read_calls >= 5);
    }

    #[test]
    fn discovery_rejects_small_scratch_without_issuing_io() {
        let (session, lease, handle) = mount_session(24, 240);
        let capacity = UsbMscCapacity::from_block_count(4_096, 8).unwrap();
        let mut coordinator = UsbMscMountCoordinator::new();
        let attempt = coordinator
            .begin(&session, lease, handle, 0, capacity)
            .unwrap();
        let mut device = MemoryBlockDevice::new(4_096, 8);
        let mut scratch = [0u8; 512];

        assert_eq!(
            discover_mount_selection(&mut device, &coordinator, attempt, &mut scratch),
            Err(UsbMscDiscoveryError::ScratchTooSmall {
                required: 4_096,
                actual: 512,
            })
        );
        assert_eq!(device.read_calls, 0);
    }

    #[test]
    fn mount_attempt_requires_current_bound_generation() {
        let (mut session, lease, handle) = mount_session(3, 10);
        let capacity = UsbMscCapacity::from_block_count(512, 4_096).unwrap();
        let mut coordinator = UsbMscMountCoordinator::new();

        let attempt = coordinator
            .begin(&session, lease, handle, 0, capacity)
            .unwrap();
        assert_eq!(coordinator.active(), Some(attempt));

        assert_eq!(
            session.on_disconnect(Some(handle)),
            ruzina_media_session::DisconnectResult::Accepted
        );
        let fresh = match session.on_connect(MediaSourceId::new(3).unwrap()) {
            ruzina_media_session::ConnectResult::Accepted(lease) => lease,
            other => panic!("unexpected reconnect result: {other:?}"),
        };
        let fresh_handle = MediaHandle::new(11).unwrap();
        assert!(session.bind_handle(fresh, fresh_handle));

        assert_eq!(
            coordinator.commit(
                &mut session,
                attempt,
                UsbMscMountSelection {
                    range: BlockRange::from_start(0, capacity.geometry()).unwrap(),
                    filesystem: FileSystemKind::Fat32,
                },
            ),
            Err(UsbMscMountError::StaleLease)
        );
        assert!(!session.is_mounted());
    }

    #[test]
    fn abort_clears_only_the_matching_active_mount_attempt() {
        let (session, lease, handle) = mount_session(31, 310);
        let capacity = UsbMscCapacity::from_block_count(512, 4_096).unwrap();
        let mut coordinator = UsbMscMountCoordinator::new();
        let attempt = coordinator
            .begin(&session, lease, handle, 0, capacity)
            .unwrap();
        let foreign = UsbMscMountAttempt { lun: 1, ..attempt };

        assert!(!coordinator.abort(foreign));
        assert_eq!(coordinator.active(), Some(attempt));
        assert!(coordinator.abort(attempt));
        assert_eq!(coordinator.active(), None);
        assert_eq!(coordinator.mounted(), None);
    }

    #[test]
    fn unknown_gpt_candidate_is_classified_from_partition_boot_sector() {
        let (session, lease, handle) = mount_session(4, 20);
        let capacity = UsbMscCapacity::from_block_count(512, 10_000).unwrap();
        let mut coordinator = UsbMscMountCoordinator::new();
        let attempt = coordinator
            .begin(&session, lease, handle, 0, capacity)
            .unwrap();

        let selection = coordinator
            .select_partition(
                attempt,
                PartitionCandidate {
                    first_lba: 2_048,
                    sector_count: core::num::NonZeroU64::new(4_096),
                    kind: VolumeKind::Unknown,
                },
                &fat32_boot_sector(),
            )
            .unwrap();

        assert_eq!(selection.filesystem, FileSystemKind::Fat32);
        assert_eq!(selection.range.first_block(), 2_048);
        assert_eq!(selection.range.block_count().get(), 4_096);
    }

    #[test]
    fn superfloppy_exfat_selection_uses_device_remainder() {
        let (session, lease, handle) = mount_session(5, 30);
        let capacity = UsbMscCapacity::from_block_count(4_096, 2_000).unwrap();
        let mut coordinator = UsbMscMountCoordinator::new();
        let attempt = coordinator
            .begin(&session, lease, handle, 0, capacity)
            .unwrap();

        let selection = coordinator
            .select_partition(
                attempt,
                PartitionCandidate {
                    first_lba: 0,
                    sector_count: None,
                    kind: VolumeKind::Unknown,
                },
                &exfat_boot_sector(),
            )
            .unwrap();

        assert_eq!(selection.filesystem, FileSystemKind::ExFat);
        assert_eq!(selection.range.first_block(), 0);
        assert_eq!(selection.range.block_count().get(), 2_000);
    }

    #[test]
    fn selection_rejects_partition_outside_reported_capacity() {
        let (session, lease, handle) = mount_session(6, 40);
        let capacity = UsbMscCapacity::from_block_count(512, 1_000).unwrap();
        let mut coordinator = UsbMscMountCoordinator::new();
        let attempt = coordinator
            .begin(&session, lease, handle, 0, capacity)
            .unwrap();

        assert_eq!(
            coordinator.select_partition(
                attempt,
                PartitionCandidate {
                    first_lba: 900,
                    sector_count: core::num::NonZeroU64::new(200),
                    kind: VolumeKind::Fat,
                },
                &fat32_boot_sector(),
            ),
            Err(UsbMscMountError::PartitionOutOfRange)
        );
    }

    #[test]
    fn commit_marks_only_current_owner_mounted() {
        let (mut session, lease, handle) = mount_session(7, 50);
        let capacity = UsbMscCapacity::from_block_count(512, 8_000).unwrap();
        let mut coordinator = UsbMscMountCoordinator::new();
        let attempt = coordinator
            .begin(&session, lease, handle, 0, capacity)
            .unwrap();
        let selection = coordinator
            .select_partition(
                attempt,
                PartitionCandidate {
                    first_lba: 2_048,
                    sector_count: core::num::NonZeroU64::new(2_000),
                    kind: VolumeKind::Fat,
                },
                &fat32_boot_sector(),
            )
            .unwrap();

        let mounted = coordinator
            .commit(&mut session, attempt, selection)
            .unwrap();
        assert_eq!(coordinator.mounted(), Some(mounted));
        assert!(session.is_mounted());

        assert!(!coordinator.on_detached(MediaHandle::new(99).unwrap()));
        assert_eq!(coordinator.mounted(), Some(mounted));
        assert!(coordinator.on_detached(handle));
        assert_eq!(coordinator.mounted(), None);
    }

    fn lease(session: &mut MediaSession, source: u32) -> MediaLease {
        let source = ruzina_media_session::MediaSourceId::new(source).unwrap();
        match session.on_connect(source) {
            ruzina_media_session::ConnectResult::Accepted(lease) => lease,
            other => panic!("unexpected connect result: {other:?}"),
        }
    }

    #[test]
    fn request_ids_skip_zero_across_wrap() {
        let mut session = MediaSession::new();
        let lease = lease(&mut session, 1);
        let mut ids = UsbMscRequestSequencer { next: u32::MAX };

        let last = ids.issue(lease, 0, UsbMscRequestKind::Flush);
        let wrapped = ids.issue(lease, 0, UsbMscRequestKind::Flush);

        assert_eq!(last.id.get(), u32::MAX);
        assert_eq!(wrapped.id.get(), 1);
    }

    #[test]
    fn reconnect_same_source_rejects_old_completion_ticket() {
        let mut session = MediaSession::new();
        let old_lease = lease(&mut session, 7);
        let mut ids = UsbMscRequestSequencer::new();
        let old_ticket = ids.issue(
            old_lease,
            0,
            UsbMscRequestKind::Read {
                first_block: 42,
                block_count: 2,
            },
        );
        assert!(UsbMscCompletionGate::accepts(&session, old_ticket));

        assert_eq!(
            session.on_disconnect(None),
            ruzina_media_session::DisconnectResult::Accepted
        );
        let fresh_lease = lease(&mut session, 7);
        let fresh_ticket = ids.issue(
            fresh_lease,
            0,
            UsbMscRequestKind::Read {
                first_block: 42,
                block_count: 2,
            },
        );

        assert_ne!(old_lease.generation, fresh_lease.generation);
        assert!(!UsbMscCompletionGate::accepts(&session, old_ticket));
        assert!(UsbMscCompletionGate::accepts(&session, fresh_ticket));
        assert_ne!(old_ticket.id, fresh_ticket.id);
    }

    #[test]
    fn ticket_preserves_u64_lba_block_count_lun_and_operation() {
        let mut session = MediaSession::new();
        let lease = lease(&mut session, 3);
        let mut ids = UsbMscRequestSequencer::new();
        let lba = u32::MAX as u64 + 77;
        let ticket = ids.issue(
            lease,
            4,
            UsbMscRequestKind::Write {
                first_block: lba,
                block_count: 8,
            },
        );

        assert_eq!(ticket.lun, 4);
        assert_eq!(ticket.first_block(), Some(lba));
        assert_eq!(ticket.block_count(), Some(8));
        assert!(matches!(ticket.kind, UsbMscRequestKind::Write { .. }));
    }

    #[test]
    fn usb_device_address_is_session_local_source_and_zero_is_rejected() {
        assert_eq!(usb_address_source(0), None);
        assert_eq!(usb_address_source(1).unwrap().get(), 1);
        assert_eq!(usb_address_source(127).unwrap().get(), 127);
    }

    #[test]
    fn media_handle_sequence_skips_zero_and_does_not_reuse_usb_address() {
        let mut handles = UsbMscHandleSequencer { next: u64::MAX };
        let last = handles.issue();
        let wrapped = handles.issue();

        assert_eq!(last.get(), u64::MAX);
        assert_eq!(wrapped.get(), 1);

        let source = usb_address_source(5).unwrap();
        assert_ne!(last.get(), source.get() as u64);
    }

    fn media_source(value: u32) -> ruzina_media_session::MediaSourceId {
        ruzina_media_session::MediaSourceId::new(value).unwrap()
    }

    fn media_handle(value: u64) -> ruzina_media_session::MediaHandle {
        ruzina_media_session::MediaHandle::new(value).unwrap()
    }

    #[test]
    fn enumeration_binds_owner_and_admits_requests_only_while_connected() {
        let mut bridge = UsbMscSessionBridge::new();
        assert_eq!(
            bridge.issue(
                0,
                UsbMscRequestKind::Read {
                    first_block: 0,
                    block_count: 1,
                },
            ),
            Err(UsbMscSessionError::NoActiveMedia)
        );

        let lease = bridge
            .on_enumerated(media_source(4), media_handle(11))
            .unwrap();
        let ticket = bridge
            .issue(
                0,
                UsbMscRequestKind::Read {
                    first_block: 0,
                    block_count: 1,
                },
            )
            .unwrap();

        assert_eq!(ticket.lease, lease);
        assert!(bridge.accepts_completion(ticket));
        assert_eq!(
            bridge.on_disconnect(media_handle(11)),
            ruzina_media_session::DisconnectResult::Accepted
        );
        assert!(!bridge.accepts_completion(ticket));
        assert_eq!(
            bridge.issue(0, UsbMscRequestKind::Flush),
            Err(UsbMscSessionError::NoActiveMedia)
        );
    }

    #[test]
    fn bound_issue_rejects_stale_generation_after_reenumeration() {
        let mut bridge = UsbMscSessionBridge::new();
        let old_handle = media_handle(11);
        let old_lease = bridge.on_enumerated(media_source(4), old_handle).unwrap();

        assert_eq!(
            bridge.on_disconnect(old_handle),
            ruzina_media_session::DisconnectResult::Accepted
        );
        let fresh_handle = media_handle(12);
        let fresh_lease = bridge.on_enumerated(media_source(4), fresh_handle).unwrap();

        assert_eq!(
            bridge.issue_for(old_lease, old_handle, 0, UsbMscRequestKind::Flush),
            Err(UsbMscSessionError::StaleLease)
        );
        let fresh = bridge
            .issue_for(fresh_lease, fresh_handle, 0, UsbMscRequestKind::Flush)
            .unwrap();
        assert_eq!(fresh.lease, fresh_lease);
    }

    #[test]
    fn bound_issue_rejects_wrong_owner_without_issuing_for_current_media() {
        let mut bridge = UsbMscSessionBridge::new();
        let owner = media_handle(11);
        let lease = bridge.on_enumerated(media_source(4), owner).unwrap();

        assert_eq!(
            bridge.issue_for(
                lease,
                media_handle(99),
                0,
                UsbMscRequestKind::Read {
                    first_block: 7,
                    block_count: 1,
                },
            ),
            Err(UsbMscSessionError::WrongOwner)
        );

        let ticket = bridge
            .issue_for(
                lease,
                owner,
                0,
                UsbMscRequestKind::Read {
                    first_block: 7,
                    block_count: 1,
                },
            )
            .unwrap();
        assert_eq!(ticket.lease, lease);
        assert_eq!(ticket.first_block(), Some(7));
    }

    #[test]
    fn same_source_reenumeration_never_revalidates_old_ticket() {
        let mut bridge = UsbMscSessionBridge::new();
        let old_lease = bridge
            .on_enumerated(media_source(4), media_handle(11))
            .unwrap();
        let old_ticket = bridge
            .issue(
                0,
                UsbMscRequestKind::Read {
                    first_block: 7,
                    block_count: 2,
                },
            )
            .unwrap();

        assert_eq!(
            bridge.on_disconnect(media_handle(11)),
            ruzina_media_session::DisconnectResult::Accepted
        );
        let fresh_lease = bridge
            .on_enumerated(media_source(4), media_handle(12))
            .unwrap();
        let fresh_ticket = bridge
            .issue(
                0,
                UsbMscRequestKind::Read {
                    first_block: 7,
                    block_count: 2,
                },
            )
            .unwrap();

        assert_ne!(old_lease.generation, fresh_lease.generation);
        assert!(!bridge.accepts_completion(old_ticket));
        assert!(bridge.accepts_completion(fresh_ticket));
    }

    #[test]
    fn foreign_disconnect_and_secondary_enumeration_do_not_steal_owner() {
        let mut bridge = UsbMscSessionBridge::new();
        let primary = bridge
            .on_enumerated(media_source(4), media_handle(11))
            .unwrap();
        assert_eq!(
            bridge.on_enumerated(media_source(9), media_handle(99)),
            Err(UsbMscSessionError::SecondaryDevice)
        );
        assert_eq!(
            bridge.on_disconnect(media_handle(99)),
            ruzina_media_session::DisconnectResult::IgnoredForeign
        );

        let ticket = bridge.issue(0, UsbMscRequestKind::Flush).unwrap();
        assert_eq!(ticket.lease, primary);
        assert!(bridge.accepts_completion(ticket));
    }

    #[test]
    fn delayed_old_handle_disconnect_cannot_drop_fresh_generation() {
        let mut bridge = UsbMscSessionBridge::new();
        let old_handle = media_handle(11);
        let fresh_handle = media_handle(12);

        bridge.on_enumerated(media_source(4), old_handle).unwrap();
        assert_eq!(
            bridge.on_disconnect(old_handle),
            ruzina_media_session::DisconnectResult::Accepted
        );
        let fresh_lease = bridge.on_enumerated(media_source(4), fresh_handle).unwrap();
        let fresh_ticket = bridge.issue(0, UsbMscRequestKind::Flush).unwrap();

        assert_eq!(
            bridge.on_disconnect(old_handle),
            ruzina_media_session::DisconnectResult::IgnoredForeign
        );
        assert_eq!(bridge.lease(), Some(fresh_lease));
        assert!(bridge.accepts_completion(fresh_ticket));
    }

    #[test]
    fn duplicate_enumeration_cannot_replace_bound_handle() {
        let mut bridge = UsbMscSessionBridge::new();
        let lease = bridge
            .on_enumerated(media_source(4), media_handle(11))
            .unwrap();

        assert_eq!(
            bridge.on_enumerated(media_source(4), media_handle(12)),
            Err(UsbMscSessionError::HandleRejected)
        );
        assert_eq!(bridge.lease(), Some(lease));
        assert_eq!(bridge.session().handle(), Some(media_handle(11)));
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TestError {
        Failed,
    }

    impl fmt::Display for TestError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("test transport failed")
        }
    }

    impl core::error::Error for TestError {}

    #[derive(Default)]
    struct TestTransport {
        read_calls: u32,
        write_calls: u32,
        flush_calls: u32,
        last_lun: u8,
        last_lba: u64,
        last_blocks: u32,
        fail: bool,
    }

    impl UsbMscTransport for TestTransport {
        type Error = TestError;

        fn read_blocks(
            &mut self,
            lun: u8,
            first_block: u64,
            block_count: u32,
            output: &mut [u8],
        ) -> Result<(), Self::Error> {
            if self.fail {
                return Err(TestError::Failed);
            }
            self.read_calls += 1;
            self.last_lun = lun;
            self.last_lba = first_block;
            self.last_blocks = block_count;
            output.fill(0xa5);
            Ok(())
        }
    }

    impl WritableUsbMscTransport for TestTransport {
        fn write_blocks(
            &mut self,
            lun: u8,
            first_block: u64,
            block_count: u32,
            _input: &[u8],
        ) -> Result<(), Self::Error> {
            if self.fail {
                return Err(TestError::Failed);
            }
            self.write_calls += 1;
            self.last_lun = lun;
            self.last_lba = first_block;
            self.last_blocks = block_count;
            Ok(())
        }

        fn synchronize_cache(&mut self, lun: u8) -> Result<(), Self::Error> {
            if self.fail {
                return Err(TestError::Failed);
            }
            self.flush_calls += 1;
            self.last_lun = lun;
            Ok(())
        }
    }

    fn device(max_blocks: u32) -> UsbMscBlockDevice<TestTransport> {
        UsbMscBlockDevice::new(
            TestTransport::default(),
            2,
            UsbMscCapacity::from_block_count(512, 1_000).unwrap(),
            NonZeroU32::new(max_blocks).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn capacity_converts_last_lba_without_overflow() {
        assert_eq!(
            UsbMscCapacity::from_last_lba(512, 999),
            Some(UsbMscCapacity {
                block_size: 512,
                block_count: 1_000,
            })
        );
        assert_eq!(UsbMscCapacity::from_last_lba(512, u64::MAX), None);
        assert_eq!(UsbMscCapacity::from_block_count(0, 1), None);
    }

    #[test]
    fn read_is_bounded_and_preserves_lun_and_u64_lba() {
        let lba = u32::MAX as u64 + 32;
        let mut device = UsbMscBlockDevice::new(
            TestTransport::default(),
            2,
            UsbMscCapacity::from_block_count(512, lba + 2).unwrap(),
            NonZeroU32::new(8).unwrap(),
        )
        .unwrap();
        let mut output = [0u8; 1_024];

        device.read_blocks(lba, 2, &mut output).unwrap();

        assert!(output.iter().all(|byte| *byte == 0xa5));
        assert_eq!(device.transport().read_calls, 1);
        assert_eq!(device.transport().last_lun, 2);
        assert_eq!(device.transport().last_lba, lba);
        assert_eq!(device.transport().last_blocks, 2);
    }

    #[test]
    fn invalid_buffer_and_transfer_window_fail_before_transport() {
        let mut device = device(2);

        assert_eq!(
            device.read_blocks(0, 3, &mut [0u8; 1_536]),
            Err(UsbMscError::TooManyBlocks {
                requested: 3,
                maximum: 2,
            })
        );
        assert_eq!(
            device.read_blocks(0, 1, &mut [0u8; 511]),
            Err(UsbMscError::Transfer(TransferError::InvalidBufferLength))
        );
        assert_eq!(device.transport().read_calls, 0);
    }

    #[test]
    fn disconnect_is_terminal_and_fail_closed() {
        let mut device = device(4);
        device.disconnect();

        assert!(!device.is_connected());
        assert_eq!(
            device.read_blocks(0, 1, &mut [0u8; 512]),
            Err(UsbMscError::Disconnected)
        );
        assert_eq!(device.flush(), Err(UsbMscError::Disconnected));
        assert_eq!(device.transport().read_calls, 0);
        assert_eq!(device.transport().flush_calls, 0);
    }

    #[test]
    fn write_and_flush_delegate_only_after_contract_validation() {
        let mut device = device(4);
        let input = [0x5au8; 1_024];

        device.write_blocks(7, 2, &input).unwrap();
        device.flush().unwrap();

        assert_eq!(device.transport().write_calls, 1);
        assert_eq!(device.transport().last_lba, 7);
        assert_eq!(device.transport().last_blocks, 2);
        assert_eq!(device.transport().flush_calls, 1);
    }

    #[test]
    fn backend_errors_are_not_misreported_as_disconnects() {
        let transport = TestTransport {
            fail: true,
            ..TestTransport::default()
        };
        let mut device = UsbMscBlockDevice::new(
            transport,
            0,
            UsbMscCapacity::from_block_count(4_096, 16).unwrap(),
            NonZeroU32::new(1).unwrap(),
        )
        .unwrap();

        assert_eq!(
            device.read_blocks(0, 1, &mut [0u8; 4_096]),
            Err(UsbMscError::Backend(TestError::Failed))
        );
    }
}
