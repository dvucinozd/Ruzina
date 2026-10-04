use embassy_time::{Delay, Timer};
use esp_hal::sdmmc::{Config, SdHostController, SlotConfig};
use sdio::{BlockDevice, sd::Card};

const CARD_HZ: u32 = 25_000_000;
const RAIL_SETTLE_MS: u64 = 10;
const INIT_RETRY_MS: u64 = 1_000;

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
            match core::future::pending::<core::convert::Infallible>().await {}
        }
        Timer::after_millis(INIT_RETRY_MS).await;
    }
}
