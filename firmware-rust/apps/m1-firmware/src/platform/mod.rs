//! Only a selected Guition BSP may acquire physical board resources.
use embassy_executor::Spawner;
use esp_hal::peripherals::Peripherals;

#[cfg(all(feature = "board-jc1060", feature = "board-jc8012"))]
compile_error!("select at most one Guition board profile");
#[cfg(all(
    feature = "integration-check",
    any(feature = "board-jc1060", feature = "board-jc8012")
))]
compile_error!("integration-check cannot be combined with a Guition board selector");
#[cfg(all(
    not(feature = "integration-check"),
    any(
        feature = "usb0-hardware-bringup",
        feature = "usb1-controller-bringup",
        feature = "hotcue-flash-storage",
        feature = "p4-psram-storage",
        feature = "sdmmc-profile-storage-bringup",
        feature = "runtime-health"
    )
))]
compile_error!(
    "product owner features require integration-check; Guition runtime bindings are pending"
);
#[cfg(all(
    feature = "runtime-health",
    not(any(feature = "usb0-hardware-bringup", feature = "usb1-controller-bringup"))
))]
compile_error!("runtime-health requires an explicitly enabled USB owner");
#[cfg(all(
    feature = "board-diagnostics",
    not(any(feature = "board-jc1060", feature = "board-jc8012"))
))]
compile_error!("board-diagnostics requires a Guition board selector");
#[cfg(all(
    any(feature = "guition-flash-probe", feature = "guition-psram-test"),
    not(any(feature = "board-jc1060", feature = "board-jc8012"))
))]
compile_error!("Guition memory probes require a Guition board selector");
#[cfg(all(feature = "guition-psram-test", debug_assertions))]
compile_error!("guition-psram-test requires a release build");

#[cfg(all(
    feature = "board-diagnostics",
    any(feature = "board-jc1060", feature = "board-jc8012")
))]
mod diagnostics;
#[cfg(feature = "board-jc1060")]
mod jc1060;
#[cfg(feature = "board-jc8012")]
mod jc8012;
#[cfg(all(
    any(feature = "guition-flash-probe", feature = "guition-psram-test"),
    any(feature = "board-jc1060", feature = "board-jc8012")
))]
mod memory;

pub(crate) fn start(spawner: Spawner, peripherals: Peripherals) {
    #[cfg(feature = "board-jc1060")]
    jc1060::start(spawner, peripherals);
    #[cfg(feature = "board-jc8012")]
    jc8012::start(spawner, peripherals);
    #[cfg(not(any(feature = "board-jc1060", feature = "board-jc8012")))]
    {
        // Integration configurations compile owner modules but never acquire
        // USB, SD, flash or PSRAM, register heap regions, or spawn those owners.
        let _ = spawner;
        let timg0 = esp_hal::timer::timg::TimerGroup::new(peripherals.TIMG0);
        esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);
    }
}

#[cfg(any(feature = "board-jc1060", feature = "board-jc8012"))]
fn start_guition(
    spawner: Spawner,
    peripherals: Peripherals,
    profile: &'static ruzina_board_profile::BoardProfile,
) {
    #[cfg(feature = "board-diagnostics")]
    let memory_report = ruzina_board_profile::memory::MemoryReport {
        #[cfg(feature = "guition-flash-probe")]
        flash: memory::flash_probe(peripherals.FLASH),
        #[cfg(not(feature = "guition-flash-probe"))]
        flash: ruzina_board_profile::memory::FlashProbe::NotRequested,
        #[cfg(feature = "guition-psram-test")]
        psram: memory::psram_test(peripherals.PSRAM, profile.psram_bytes),
        #[cfg(not(feature = "guition-psram-test"))]
        psram: ruzina_board_profile::memory::PsramProbe::NotRequested,
    };
    let timg0 = esp_hal::timer::timg::TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);
    #[cfg(feature = "board-diagnostics")]
    spawner.spawn(
        diagnostics::report_task(peripherals.USB_DEVICE, profile, memory_report)
            .expect("boot diagnostic task slot"),
    );
    #[cfg(not(feature = "board-diagnostics"))]
    let _ = (spawner, profile);
}
