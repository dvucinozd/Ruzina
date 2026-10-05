use aligned::{A4, Aligned};
use block_device_driver::BlockDevice as BlockDeviceDriver;
use embassy_time::{Delay, Timer};
use esp_hal::sdmmc::{Config, SdHostController, Slot, SlotConfig};
use pajoniiir_controller_profile_storage::ProfileScanBuffers;
use pajoniiir_media_block::{BlockGeometry, BlockRange, TransferError};
use pajoniiir_media_fat32::{
    Fat32AsyncFileSystem, parse_fat32_boot_sector, parse_fat32_fs_info,
};
use pajoniiir_media_io::{AsyncPartitionDevice, OwnedBlockDevice};
use pajoniiir_media_partition::{
    PartitionLayout, PartitionScanResult, VolumeKind, scan_mbr_or_superfloppy,
};
use pajoniiir_media_session::{ConnectResult, MediaSession, MediaSourceId};
use sdio::{Addressable, BlockDevice, MmcError, sd::Card};
use static_cell::StaticCell;

const CARD_HZ: u32 = 25_000_000;
const RAIL_SETTLE_MS: u64 = 10;
const INIT_RETRY_MS: u64 = 1_000;
const OWNER_HEALTH_TICK_MS: u64 = 60_000;
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

async fn park_profile_storage() -> core::convert::Infallible {
    core::future::pending().await
}

/// Persistent owner for the M1 microSD SDMMC0 path.
///
/// This binds the board-authoritative ESP32-P4 slot-0 IO-MUX pins, enables the
/// controlled 3V3_SD rail, initializes the SD card, mounts a FAT32
/// superfloppy/MBR partition through Pajoniiir's native async filesystem, and
/// scans the released /controllers/<id>/profile.s3bin layout without heap
/// allocation.
///
/// GPT discovery, physical card removal/reinsert, rail power-cycle recovery and
/// profile activation in the USB1 controller session remain explicit follow-on
/// hardware/software gates.
pub(crate) async fn run_profile_storage_owner(
    sdhost: esp_hal::peripherals::SDHOST<'static>,
    d0: esp_hal::peripherals::GPIO39<'static>,
    d1: esp_hal::peripherals::GPIO40<'static>,
    d2: esp_hal::peripherals::GPIO41<'static>,
    d3: esp_hal::peripherals::GPIO42<'static>,
    clk: esp_hal::peripherals::GPIO43<'static>,
    cmd: esp_hal::peripherals::GPIO44<'static>,
    power_enable: esp_hal::peripherals::GPIO45<'static>,
    card_detect: esp_hal::peripherals::GPIO46<'static>,
) -> ! {
    let controller =
        SdHostController::new(sdhost, Config::default()).expect("SDMMC controller config is valid");
    let slot = controller
        .slot::<0>(SlotConfig::default())
        .expect("SDMMC slot 0 must be available")
        .with_clk(clk)
        .with_cmd(cmd)
        .with_data0(d0)
        .with_data1(d1)
        .with_data2(d2)
        .with_data3(d3)
        .with_power_enable(power_enable)
        .with_card_detect(card_detect)
        .into_async();

    Timer::after_millis(RAIL_SETTLE_MS).await;

    let mut card: BlockDevice<Card, _, _, 512> = BlockDevice::new_uninit_sd_card(slot, Delay);
    loop {
        if card.reacquire(CARD_HZ).await.is_err() {
            Timer::after_millis(INIT_RETRY_MS).await;
            continue;
        }

        let mut device = SdProfileBlockDevice::new(card);
        let geometry = device.geometry();
        let scratch = SdProfileBlockBuffer::zeroed();

        let scratch = match device.read_blocks_owned(0, 1, scratch).await {
            Ok(scratch) => scratch,
            Err(_) => match park_profile_storage().await {},
        };

        let mut layout = PartitionLayout::new();
        if scan_mbr_or_superfloppy(scratch.as_ref(), &mut layout) != PartitionScanResult::Ok {
            match park_profile_storage().await {}
        }

        let Some(candidate) = layout
            .candidates()
            .iter()
            .copied()
            .find(|candidate| candidate.kind == VolumeKind::Fat)
        else {
            match park_profile_storage().await {}
        };

        let Some(range) = candidate_range(candidate.first_lba, candidate.sector_count, geometry)
        else {
            match park_profile_storage().await {}
        };

        let mut partition = AsyncPartitionDevice::new(device, range);
        let scratch = match partition.read_blocks_owned(0, 1, scratch).await {
            Ok(scratch) => scratch,
            Err(_) => match park_profile_storage().await {},
        };
        let boot = match parse_fat32_boot_sector(scratch.as_ref(), partition.geometry()) {
            Ok(boot) => boot,
            Err(_) => match park_profile_storage().await {},
        };

        let scratch = match partition
            .read_blocks_owned(u64::from(boot.fs_info_sector), 1, scratch)
            .await
        {
            Ok(scratch) => scratch,
            Err(_) => match park_profile_storage().await {},
        };
        if parse_fat32_fs_info(scratch.as_ref(), &boot).is_err() {
            match park_profile_storage().await {}
        }

        let source = MediaSourceId::new(SD_PROFILE_SOURCE_ID)
            .expect("M1 microSD profile-storage source id is non-zero");
        let mut session = MediaSession::new();
        let lease = match session.on_connect(source) {
            ConnectResult::Accepted(lease)
            | ConnectResult::Duplicate(lease)
            | ConnectResult::IgnoredSecondary(lease) => lease,
        };

        let mut filesystem = match Fat32AsyncFileSystem::new(partition, scratch, boot, lease) {
            Ok(filesystem) => filesystem,
            Err(_) => match park_profile_storage().await {},
        };

        let buffers = SD_PROFILE_SCAN_BUFFERS.init(ProfileScanBuffers::new());
        let report = match buffers.scan(&mut filesystem).await {
            Ok(report) => report,
            Err(_) => match park_profile_storage().await {},
        };
        crate::controller_profiles::publish_scanned_registry(report.registry);

        // Retain the filesystem/card owner and immutable scan report. USB1 can
        // consume the bounded registry snapshot immediately; loading and
        // validating the matched profile blob remains the next activation gate.
        loop {
            Timer::after_millis(OWNER_HEALTH_TICK_MS).await;
            let _ = filesystem.boot_info();
            let _ = report.registry.len();
            let _ = report.stats.entries_seen;
        }
    }
}
