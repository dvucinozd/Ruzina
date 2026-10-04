use aligned::{A4, Aligned};
use block_device_driver::BlockDevice as BlockDeviceDriver;
use embassy_time::{Delay, Timer};
use esp_hal::sdmmc::{Config, SdHostController, Slot, SlotConfig};
use pajoniiir_media_block::{BlockGeometry, TransferError};
use pajoniiir_media_io::OwnedBlockDevice;
use sdio::{Addressable, BlockDevice, MmcError, sd::Card};

const CARD_HZ: u32 = 25_000_000;
const RAIL_SETTLE_MS: u64 = 10;
const INIT_RETRY_MS: u64 = 1_000;


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

/// Persistent owner for the M1 microSD SDMMC0 path.
///
/// This binds the board-authoritative ESP32-P4 slot-0 IO-MUX pins, enables the
/// controlled 3V3_SD rail, and retains the initialized SD-card block device.
/// Filesystem mounting and controller-profile I/O remain separate boundaries.
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
        if card.reacquire(CARD_HZ).await.is_ok() {
            let device = SdProfileBlockDevice::new(card);
            let _ = device.geometry();
            let _ = SdProfileBlockBuffer::zeroed();
            match core::future::pending::<core::convert::Infallible>().await {}
        }
        Timer::after_millis(INIT_RETRY_MS).await;
    }
}
