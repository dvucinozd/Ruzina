//! Board selection owns every transfer of GPIO/peripheral handles.
use embassy_executor::Spawner;
use esp_hal::peripherals::Peripherals;

#[cfg(any(
    all(feature = "board-legacy-m1", feature = "board-jc1060"),
    all(feature = "board-legacy-m1", feature = "board-jc8012"),
    all(feature = "board-jc1060", feature = "board-jc8012"),
))]
compile_error!("select at most one board profile");

#[cfg(all(
    not(feature = "board-legacy-m1"),
    any(
        feature = "usb0-hardware-bringup",
        feature = "usb1-controller-bringup",
        feature = "hotcue-flash-storage",
        feature = "p4-psram-storage",
        feature = "sdmmc-profile-storage-bringup",
    )
))]
compile_error!(
    "legacy hardware features require board-legacy-m1; Guition peripheral bindings are not implemented"
);

#[cfg(feature = "board-jc1060")]
mod jc1060;
#[cfg(feature = "board-jc8012")]
mod jc8012;
#[cfg(feature = "board-legacy-m1")]
mod legacy_m1;

pub(crate) fn start(spawner: Spawner, peripherals: Peripherals) {
    #[cfg(feature = "board-legacy-m1")]
    legacy_m1::start(spawner, peripherals);
    #[cfg(feature = "board-jc1060")]
    jc1060::start(spawner, peripherals);
    #[cfg(feature = "board-jc8012")]
    jc8012::start(spawner, peripherals);
    #[cfg(not(any(
        feature = "board-legacy-m1",
        feature = "board-jc1060",
        feature = "board-jc8012"
    )))]
    start_idle(spawner, peripherals);
}

/// Scheduler-only skeleton. It does not take display, USB, SD, codec, flash or
/// PSRAM ownership. The board identity does not imply a working driver.
#[cfg(not(feature = "board-legacy-m1"))]
fn start_idle(_spawner: Spawner, peripherals: Peripherals) {
    let timg0 = esp_hal::timer::timg::TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);
}
