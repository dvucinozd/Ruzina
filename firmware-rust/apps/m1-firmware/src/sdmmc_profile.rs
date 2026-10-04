use esp_hal::sdmmc::{Config, SdHostController, SlotConfig};

/// Persistent compile-gate owner for the M1 microSD SDMMC0 path.
///
/// This binds the board-authoritative ESP32-P4 slot-0 IO-MUX pins and the
/// controlled 3V3_SD rail. It deliberately stops before card initialization,
/// filesystem mounting, profile scanning, or transactional profile updates.
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
    let _slot = controller
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

    match core::future::pending::<core::convert::Infallible>().await {}
}
