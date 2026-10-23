use aligned::{A4, Aligned};
use block_device_driver::BlockDevice as BlockDeviceDriver;
use embassy_futures::select::{Either, select};
use embassy_time::{Delay, Timer};
use esp_hal::sdmmc::Slot;
use ruzina_controller_profile_storage::{ProfileRegistry, ProfileScanBuffers};
use ruzina_media_block::{BlockGeometry, BlockRange, TransferError};
use ruzina_media_fat32::{
    Fat32AsyncFileSystem, Fat32BootInfo, parse_fat32_boot_sector, parse_fat32_fs_info,
};
use ruzina_media_io::{AsyncPartitionDevice, OwnedBlockDevice};
use ruzina_media_partition::{
    GptEntryStream, PartitionLayout, PartitionScanResult, VolumeKind, parse_gpt_header,
    scan_mbr_or_superfloppy,
};
use ruzina_media_session::{ConnectResult, MediaSession, MediaSourceId};
use sdio::{Addressable, BlockDevice, MmcError, sd::Card};
use static_cell::StaticCell;

use crate::controller_profiles::{
    complete_profile_activation, fail_profile_activation, wait_for_profile_activation_request,
};

const CARD_HZ: u32 = 25_000_000;
const RAIL_SETTLE_MS: u64 = 10;
const RAIL_OFF_MS: u64 = 20;
const CARD_DEBOUNCE_MS: u64 = 50;
const CARD_POLL_MS: u64 = 100;
const INIT_RETRY_MS: u64 = 1_000;
const SD_PROFILE_SOURCE_ID: u32 = 0x5344_0001;

static SD_PROFILE_SCAN_BUFFERS: StaticCell<ProfileScanBuffers> = StaticCell::new();

struct SdProfileBlockBuffer(Aligned<A4, [u8; 512]>);

impl SdProfileBlockBuffer {
    fn zeroed() -> Self {
        Self(Aligned([0u8; 512]))
    }
}

impl AsRef<[u8]> for SdProfileBlockBuffer {
    fn as_ref(&self) -> &[u8] {
        &self.0[..]
    }
}

impl AsMut<[u8]> for SdProfileBlockBuffer {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.0[..]
    }
}

#[derive(Debug)]
enum SdProfileBlockError {
    Transfer(TransferError),
    AddressOverflow,
    Device(MmcError),
}

struct SdProfileBlockDevice<'d> {
    inner: BlockDevice<Card, Slot<'d, 0, esp_hal::Async>, Delay, 512>,
    geometry: BlockGeometry,
}

impl<'d> SdProfileBlockDevice<'d> {
    fn new(inner: BlockDevice<Card, Slot<'d, 0, esp_hal::Async>, Delay, 512>) -> Self {
        Self {
            geometry: BlockGeometry {
                block_size: 512,
                block_count: u64::from(inner.card().block_count()),
            },
            inner,
        }
    }

    fn into_inner(self) -> BlockDevice<Card, Slot<'d, 0, esp_hal::Async>, Delay, 512> {
        self.inner
    }
}

impl OwnedBlockDevice for SdProfileBlockDevice<'_> {
    type Error = SdProfileBlockError;
    type Buffer = SdProfileBlockBuffer;

    fn geometry(&self) -> BlockGeometry {
        self.geometry
    }

    async fn read_blocks_owned(
        &mut self,
        first_block: u64,
        block_count: u32,
        mut buffer: Self::Buffer,
    ) -> Result<Self::Buffer, (Self::Error, Self::Buffer)> {
        if let Err(error) =
            self.geometry
                .validate_transfer(first_block, block_count, buffer.as_ref().len())
        {
            return Err((SdProfileBlockError::Transfer(error), buffer));
        }
        let Ok(first_block) = u32::try_from(first_block) else {
            return Err((SdProfileBlockError::AddressOverflow, buffer));
        };

        match self
            .inner
            .read(first_block, core::slice::from_mut(&mut buffer.0))
            .await
        {
            Ok(()) => Ok(buffer),
            Err(error) => Err((SdProfileBlockError::Device(error), buffer)),
        }
    }
}

fn candidate_range(
    first_block: u64,
    block_count: Option<core::num::NonZeroU64>,
    geometry: BlockGeometry,
) -> Option<BlockRange> {
    match block_count {
        Some(block_count) => BlockRange::new(first_block, block_count, geometry),
        None => BlockRange::from_start(first_block, geometry),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SdProfileDiscoveryError {
    Read,
    InvalidPartitionLayout,
    InvalidGptHeader,
    InvalidGptTable,
    NoFat32Volume,
}

async fn read_profile_sector(
    device: &mut SdProfileBlockDevice<'_>,
    lba: u64,
    buffer: SdProfileBlockBuffer,
) -> Result<SdProfileBlockBuffer, (SdProfileDiscoveryError, SdProfileBlockBuffer)> {
    match device.read_blocks_owned(lba, 1, buffer).await {
        Ok(buffer) => Ok(buffer),
        Err((_error, buffer)) => Err((SdProfileDiscoveryError::Read, buffer)),
    }
}

async fn discover_fat32_volume(
    device: &mut SdProfileBlockDevice<'_>,
    mut scratch: SdProfileBlockBuffer,
) -> Result<
    (BlockRange, SdProfileBlockBuffer, Fat32BootInfo),
    (SdProfileDiscoveryError, SdProfileBlockBuffer),
> {
    let geometry = device.geometry();
    scratch = read_profile_sector(device, 0, scratch).await?;

    let mut layout = PartitionLayout::new();
    match scan_mbr_or_superfloppy(scratch.as_ref(), &mut layout) {
        PartitionScanResult::Ok => {}
        PartitionScanResult::NeedsGpt => {
            scratch = read_profile_sector(device, 1, scratch).await?;
            let Some(info) = parse_gpt_header(scratch.as_ref()) else {
                return Err((SdProfileDiscoveryError::InvalidGptHeader, scratch));
            };
            let Some(mut stream) = GptEntryStream::new(info) else {
                return Err((SdProfileDiscoveryError::InvalidGptHeader, scratch));
            };

            layout.clear();
            let mut lba = info.entries_lba;
            while !stream.is_complete() {
                scratch = read_profile_sector(device, lba, scratch).await?;
                let scanned = stream.push(scratch.as_ref(), &mut layout);
                if scanned == 0 {
                    return Err((SdProfileDiscoveryError::InvalidGptTable, scratch));
                }
                let Some(next_lba) = lba.checked_add(1) else {
                    return Err((SdProfileDiscoveryError::InvalidGptTable, scratch));
                };
                lba = next_lba;
            }

            if !stream.is_valid_complete() {
                return Err((SdProfileDiscoveryError::InvalidGptTable, scratch));
            }
        }
        PartitionScanResult::Invalid | PartitionScanResult::NoCandidate => {
            return Err((SdProfileDiscoveryError::InvalidPartitionLayout, scratch));
        }
    }

    for index in 0..layout.count() {
        let candidate = layout.candidates()[index];
        if candidate.kind == VolumeKind::ExFat {
            continue;
        }

        let Some(range) = candidate_range(candidate.first_lba, candidate.sector_count, geometry)
        else {
            continue;
        };

        scratch = read_profile_sector(device, candidate.first_lba, scratch).await?;
        if let Ok(boot) =
            parse_fat32_boot_sector(scratch.as_ref(), range.geometry(geometry.block_size))
        {
            return Ok((range, scratch, boot));
        }
    }

    Err((SdProfileDiscoveryError::NoFat32Volume, scratch))
}

/// BSP-supplied power and presence observation. No GPIO or detect polarity is
/// assumed. A board without card detect needs an explicit media lifecycle policy.
pub(crate) trait ProfileStorageControl {
    fn card_present(&self) -> bool;
    fn set_power(&mut self, enabled: bool);
}

fn card_is_present(control: &impl ProfileStorageControl) -> bool {
    control.card_present()
}

async fn wait_for_card_inserted(card_detect: &impl ProfileStorageControl) {
    loop {
        while !card_is_present(card_detect) {
            Timer::after_millis(CARD_POLL_MS).await;
        }
        Timer::after_millis(CARD_DEBOUNCE_MS).await;
        if card_is_present(card_detect) {
            return;
        }
    }
}

async fn wait_for_card_removed(card_detect: &impl ProfileStorageControl) {
    while card_is_present(card_detect) {
        Timer::after_millis(CARD_POLL_MS).await;
    }
    Timer::after_millis(CARD_DEBOUNCE_MS).await;
}

async fn cycle_sd_rail(power: &mut impl ProfileStorageControl) {
    power.set_power(false);
    Timer::after_millis(RAIL_OFF_MS).await;
    power.set_power(true);
    Timer::after_millis(RAIL_SETTLE_MS).await;
}

/// FAT32 profile owner with separately provided SD slot, rail and presence.
/// No board resource acquisition or pin assignment occurs in this module.
pub(crate) async fn run_profile_storage_owner(
    slot: Slot<'static, 0, esp_hal::Async>,
    mut control: impl ProfileStorageControl,
) -> ! {
    let buffers = SD_PROFILE_SCAN_BUFFERS.init(ProfileScanBuffers::new());
    let source = MediaSourceId::new(SD_PROFILE_SOURCE_ID)
        .expect("M1 microSD profile-storage source id is non-zero");
    let mut media_session = MediaSession::new();
    let mut card: BlockDevice<Card, _, _, 512> = BlockDevice::new_uninit_sd_card(slot, Delay);

    loop {
        wait_for_card_inserted(&control).await;
        cycle_sd_rail(&mut control).await;

        if card.reacquire(CARD_HZ).await.is_err() {
            control.set_power(false);
            Timer::after_millis(INIT_RETRY_MS).await;
            continue;
        }

        let mut device = SdProfileBlockDevice::new(card);
        let scratch = SdProfileBlockBuffer::zeroed();
        let (range, scratch, boot) = match discover_fat32_volume(&mut device, scratch).await {
            Ok(discovered) => discovered,
            Err((error, _scratch)) => {
                card = device.into_inner();
                control.set_power(false);
                match error {
                    SdProfileDiscoveryError::Read => {
                        Timer::after_millis(INIT_RETRY_MS).await;
                    }
                    SdProfileDiscoveryError::InvalidPartitionLayout
                    | SdProfileDiscoveryError::InvalidGptHeader
                    | SdProfileDiscoveryError::InvalidGptTable
                    | SdProfileDiscoveryError::NoFat32Volume => {
                        wait_for_card_removed(&control).await;
                    }
                }
                continue;
            }
        };

        let mut partition = AsyncPartitionDevice::new(device, range);
        let scratch = match partition
            .read_blocks_owned(u64::from(boot.fs_info_sector), 1, scratch)
            .await
        {
            Ok(scratch) => scratch,
            Err((_error, _scratch)) => {
                card = partition.into_inner().into_inner();
                control.set_power(false);
                Timer::after_millis(INIT_RETRY_MS).await;
                continue;
            }
        };
        if parse_fat32_fs_info(scratch.as_ref(), &boot).is_err() {
            card = partition.into_inner().into_inner();
            control.set_power(false);
            wait_for_card_removed(&control).await;
            continue;
        }

        let lease = match media_session.on_connect(source) {
            ConnectResult::Accepted(lease)
            | ConnectResult::Duplicate(lease)
            | ConnectResult::IgnoredSecondary(lease) => lease,
        };

        let mut filesystem = match Fat32AsyncFileSystem::new(partition, scratch, boot, lease) {
            Ok(filesystem) => filesystem,
            Err((_error, partition, _scratch)) => {
                let _ = media_session.on_disconnect(None);
                card = partition.into_inner().into_inner();
                control.set_power(false);
                Timer::after_millis(INIT_RETRY_MS).await;
                continue;
            }
        };

        let report = match buffers.scan(&mut filesystem).await {
            Ok(report) => report,
            Err(_) => {
                let _ = media_session.on_disconnect(None);
                let (partition, _scratch) = filesystem.into_parts();
                card = partition.into_inner().into_inner();
                control.set_power(false);
                Timer::after_millis(INIT_RETRY_MS).await;
                continue;
            }
        };
        crate::controller_profiles::publish_scanned_registry(report.registry);

        // Keep filesystem ownership in this task. Removal invalidates the
        // registry before the filesystem/card stack is dismantled, so USB1
        // cannot keep using an active profile from media that no longer exists.
        card = loop {
            match select(
                wait_for_profile_activation_request(),
                Timer::after_millis(CARD_POLL_MS),
            )
            .await
            {
                Either::First(request) => {
                    match buffers.load_profile(&mut filesystem, request.slot).await {
                        Ok(blob) => {
                            let _ = complete_profile_activation(request, blob).await;
                        }
                        Err(_) => fail_profile_activation(request).await,
                    }
                }
                Either::Second(_) => {
                    let _ = filesystem.boot_info();
                    let _ = report.stats.entries_seen;
                }
            }

            if !card_is_present(&control) {
                crate::controller_profiles::publish_scanned_registry(ProfileRegistry::new());
                let _ = media_session.on_disconnect(None);
                let (partition, _scratch) = filesystem.into_parts();
                break partition.into_inner().into_inner();
            }
        };

        control.set_power(false);
        Timer::after_millis(RAIL_OFF_MS).await;
    }
}
